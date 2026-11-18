#!/usr/bin/env python3
"""Exercise javac reference identities and binary provenance at module boundaries."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent


class JavaDependencyTargetsTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temp.cleanup)
        cls.root = Path(cls.temp.name)
        home = os.environ.get('JAVA_HOME')
        cls.java = str(Path(home) / 'bin/java') if home else 'java'
        cls.javac = str(Path(home) / 'bin/javac') if home else 'javac'
        analyzer = cls.root / 'CodeclewJavaAnalyzer.java'
        shutil.copyfile(ROOT / 'crates/clew/src/java_analyzer.java', analyzer)
        subprocess.run([cls.javac, '-d', str(cls.root), str(analyzer)],
                       check=True, capture_output=True, text=True)

    def analyze(self, source, classpath=(), extra_sources=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'Refs.java').write_text(source)
            for name, body in (extra_sources or {}).items():
                (root / name).write_text(body)
            (root / 'sources').write_text('Refs.java\n' + ''.join(
                name + '\n' for name in (extra_sources or {})))
            (root / 'classpath').write_text(''.join(str(p) + '\n' for p in classpath))
            (root / 'empty').mkdir()
            result = subprocess.run([
                self.java, '-cp', str(self.root), 'CodeclewJavaAnalyzer', str(root),
                str(root / 'sources'), str(root / 'classpath'), '17', '', '', '', '',
                'CLOSED_NO_AP', str(root / 'empty')], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            return list(map(json.loads, result.stdout.splitlines()))

    def test_array_references_preserve_sites_without_forged_class_origin(self):
        source = '''import java.util.function.*;
class Array {}
class Refs {
    IntFunction<String[]> strings() { return String[]::new; }
    IntFunction<int[]> ints() { return int[]::new; }
    Function<String[], Object> cloneArray() { return String[]::clone; }
    Supplier<Array> local() { return Array::new; }
    IntFunction<Array[]> localArray() { return Array[]::new; }
    Function<String, Integer> parse() { return Integer::valueOf; }
    Function<String, String> trim() { return String::trim; }
    Supplier<StringBuilder> builder() { return StringBuilder::new; }
}
'''
        facts = self.analyze(source)
        refs = {source.encode()[f['byteStart']:f['byteEnd']].decode(): f for f in facts
                if f['kind'] == 'RELATION' and f['relationKind'] == 'REFERENCES'}
        expected = {
            'String[]::new': 'method:array:[Ljava/lang/String;#<init>(I)',
            'int[]::new': 'method:array:[I#<init>(I)',
            'String[]::clone': 'method:array:[Ljava/lang/String;#clone()',
            'Array[]::new': 'method:array:[LArray;#<init>(I)',
            'Array::new': 'method:class:Array#<init>()V',
            'Integer::valueOf': 'method:class:java.lang.Integer#valueOf(',
            'String::trim': 'method:class:java.lang.String#trim()',
            'StringBuilder::new': 'method:class:java.lang.StringBuilder#<init>()V',
        }
        self.assertEqual(set(refs), set(expected))
        for expression, prefix in expected.items():
            fact = refs[expression]
            self.assertTrue(fact['targetIdentity'].startswith(prefix), fact)
            self.assertEqual(fact['resolution'], 'COMPILER_EXACT')
            self.assertEqual(fact['file'], 'Refs.java')
            self.assertEqual(fact['startLine'], source[:fact['start']].count('\n') + 1)
        boundaries = [f for f in facts if f['kind'] == 'BOUNDARY']
        unknown = [f for f in boundaries
                   if f['code'] == 'JAVA_DEPENDENCY_MODULE_ORIGIN_UNAVAILABLE']
        self.assertEqual({source[f['start']:f['end']] for f in unknown},
                         {'String[]::new', 'int[]::new', 'String[]::clone', 'Array[]::new'})
        self.assertTrue(all(f['resolution'] == 'UNKNOWN' and f['requiredChecks'] for f in unknown))
        self.assertEqual(len(boundaries), 4)
        self.assertFalse(any(f['kind'] == 'DEPENDENCY_TARGET' for f in facts))

    def test_unnamed_classpath_targets_keep_first_admitted_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'Array.java'
            source.write_text('''public class Array {
                public Array() {}
                public static Array make(String text) { return new Array(); }
                public String value() { return "first"; }
            }''')
            paths = [root / 'first', root / 'second']
            for path in paths:
                path.mkdir()
                subprocess.run([self.javac, '--release', '17', '-d', str(path), str(source)],
                               check=True, capture_output=True, text=True)
            facts = self.analyze('''import java.util.function.*;
class Refs {
    IntFunction<String[]> array() { return String[]::new; }
    Function<String, Array> factory() { return Array::make; }
    Function<Array, String> value() { return Array::value; }
    Supplier<Array> build() { return Array::new; }
}''', paths)
            targets = [f for f in facts if f['kind'] == 'DEPENDENCY_TARGET']
            self.assertEqual({f['symbolIdentity'] for f in targets}, {
                'class:Array', 'method:class:Array#<init>()V',
                'method:class:Array#make(Ljava/lang/String;)LArray;',
                'method:class:Array#value()Ljava/lang/String;'})
            for target in targets:
                self.assertEqual(target['compilerModule'], 'module:unnamed')
                self.assertEqual(target['ownerIdentity'], 'class:Array')
                self.assertEqual(target['binaryOrigin'], {'classpathIndex': 0, 'classEntry': 'Array.class'})
                self.assertEqual(target['sourceStatus'], 'SOURCE_NOT_ATTACHED')
            refs = [f for f in facts if f['kind'] == 'RELATION' and f['relationKind'] == 'REFERENCES']
            self.assertEqual(len(refs), 4)
            self.assertEqual(sum(f['targetIdentity'].startswith('method:array:') for f in refs), 1)

    def test_missing_compiler_target_still_reports_error_boundary(self):
        facts = self.analyze('''import java.util.function.*;
class Refs { Supplier<Object> missing() { return Missing::new; } }''')
        self.assertTrue(facts)
        self.assertTrue(all(f['kind'] == 'BOUNDARY' and f['resolution'] == 'UNKNOWN' for f in facts))
        self.assertFalse(any(f['kind'] == 'DEPENDENCY_TARGET' for f in facts))

    def test_repeated_source_calls_keep_each_site_overload_and_synthetic_constructor(self):
        source = '''class Refs {
    static void run() { sink(); sink(); sink(1); new Refs(); Runnable r = Refs::sink; }
    static void sink() {}
    static void sink(int value) {}
}'''
        facts = self.analyze(source)
        relations = [f for f in facts if f['kind'] == 'RELATION']
        calls = [f for f in relations if f['relationKind'] == 'CALLS']
        self.assertEqual(len(calls), 3)
        self.assertEqual(sum(f['targetIdentity'] == 'method:class:Refs#sink()V'
                             for f in calls), 2)
        self.assertEqual(sum(f['targetIdentity'] == 'method:class:Refs#sink(I)V'
                             for f in calls), 1)
        self.assertEqual(len({f['byteStart'] for f in calls}), 3)
        self.assertTrue(any(f['relationKind'] == 'CONSTRUCTS' and
                            f['targetIdentity'] == 'method:class:Refs#<init>()V'
                            for f in relations))
        self.assertTrue(any(f['relationKind'] == 'REFERENCES' and
                            f['targetIdentity'] == 'method:class:Refs#sink()V'
                            for f in relations))
        self.assertFalse(any(f['kind'] == 'DEPENDENCY_TARGET' for f in facts))

    def test_variable_symbols_keep_cross_unit_shadowing_record_and_byte_ranges(self):
        sources = {
            'Refs.java': '''class Refs {
    int value;
    int first(int input) {
        int value = input;
        value++;
        return value + Later.value + this.value;
    }
    int second(int input) {
        int value = input;
        String label = "😀";
        System.out.println(label);
        int café = value;
        return café;
    }
}''',
            'Later.java': '''class Later {
    static int value;
    static int read(int input) { int value = input; return value; }
}''',
            'Entry.java': '''record Entry(int value) {
    Entry { if (value < 0) value = 0; }
    int read() { return value; }
}''',
        }
        facts = self.analyze(sources['Refs.java'], extra_sources={
            name: body for name, body in sources.items() if name != 'Refs.java'})
        variables = [f for f in facts if f['kind'] in
                     ('VARIABLE_DECLARATION', 'VARIABLE_ACCESS')]
        locals = [f for f in variables if f['kind'] == 'VARIABLE_DECLARATION'
                  and f['name'] == 'value' and f['variableKind'] == 'LOCAL_VARIABLE']
        self.assertEqual(len(locals), 3)
        self.assertEqual(len({f['variableIdentity'] for f in locals}), 3)
        for declaration in locals:
            accesses = [f for f in variables if f['kind'] == 'VARIABLE_ACCESS'
                        and f['variableIdentity'] == declaration['variableIdentity']]
            self.assertTrue(accesses)
            self.assertTrue(all(f['declarationStatus'] == 'SOURCE_RETAINED' for f in accesses))
        self.assertTrue(any(f.get('variableIdentity') == 'field:class:Later#value:I'
                            and f.get('declarationStatus') == 'SOURCE_RETAINED'
                            and f['file'] == 'Refs.java' for f in variables))
        self.assertTrue(any(f['name'] == 'out' and f.get('declarationStatus') ==
                            'DECLARATION_SOURCE_UNAVAILABLE' for f in variables))
        self.assertTrue(any(f.get('declarationStatus') ==
                            'IMPLICIT_RECORD_COMPONENT_RETAINED' for f in variables))
        self.assertTrue(any(f['name'] == 'café' for f in variables))
        for fact in variables:
            text = sources[fact['file']]
            utf16 = text.encode('utf-16-le')
            for offset, byte_offset in (('start', 'byteStart'), ('end', 'byteEnd')):
                expected = len(utf16[:fact[offset] * 2].decode('utf-16-le').encode())
                self.assertEqual(fact[byte_offset], expected, fact)


if __name__ == '__main__':
    unittest.main()
