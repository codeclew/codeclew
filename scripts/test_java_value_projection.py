#!/usr/bin/env python3
"""Exercise the actual javac producer with coincident and ordinary AST positions."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent


class JavaValueProjectionTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temp.cleanup)
        cls.root = Path(cls.temp.name)
        home = os.environ.get('JAVA_HOME')
        cls.java = str(Path(home) / 'bin/java') if home else 'java'
        javac = str(Path(home) / 'bin/javac') if home else 'javac'
        analyzer = cls.root / 'CodeclewJavaAnalyzer.java'
        shutil.copyfile(ROOT / 'crates/clew/src/java_analyzer.java', analyzer)
        probe = ROOT / 'scripts/fixtures/java-value-projection/SourcePositionsProbe.java'
        subprocess.run([javac, '-d', str(cls.root), str(analyzer), str(probe)],
                       check=True, capture_output=True, text=True)
        large = ''.join(f'Object x{i}="value";' for i in range(70))
        (cls.root / 'Values.java').write_text('''class Values {
            Object copy(Object input, Object other) { return input; }
            boolean canEqual(Object other) { return other instanceof Values; }
            Object mutate(Object input) { input=null; return input; }
            Object large() { ''' + large + ''' return "result"; }
        }''')

    def read(self, mode):
        result = subprocess.run([self.java, '-cp', str(self.root), 'SourcePositionsProbe',
                                 str(self.root), mode], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return {row['name']: row['valueProjection'] for row in
                map(json.loads, result.stdout.splitlines()) if 'valueProjection' in row}

    def assert_unsupported(self, projection):
        self.assertNotIn('parameterDefinitions', projection)
        self.assertNotIn('calls', projection)
        self.assertEqual(len(projection['nodes']), 1)
        self.assertEqual(projection['nodes'][0]['id'], 'result')
        self.assertEqual(projection['nodes'][0]['operation'], {
            'kind': 'UNKNOWN', 'boundary': 'JAVA_VALUE_BODY_OR_BUDGET_UNSUPPORTED'})

    def test_generated_expression_cannot_overwrite_parameter(self):
        self.assert_unsupported(self.read('expression')['canEqual'])

    def test_duplicate_parameter_ranges_fail_closed(self):
        self.assert_unsupported(self.read('parameters')['copy'])

    def test_empty_parameter_range_fail_closed(self):
        self.assert_unsupported(self.read('empty')['copy'])

    def test_ordinary_parameters_keep_exact_definition_bindings(self):
        projections = self.read('ordinary')
        for name in ('copy', 'canEqual'):
            projection = projections[name]
            nodes = {node['id']: node for node in projection['nodes']}
            parameters = projection['parameterDefinitions']
            self.assertEqual(len(parameters), 2 if name == 'copy' else 1)
            self.assertEqual(len(set(parameters)), len(parameters))
            for identity in parameters:
                self.assertEqual(nodes[identity]['operation'], {'kind': 'INPUT'})
                self.assertEqual(identity, f"v{nodes[identity]['start']}_{nodes[identity]['end']}")
            if name == 'copy':
                result = nodes['result']['operation']['value']
                self.assertEqual(nodes[result]['operation'], {'kind': 'COPY', 'value': parameters[0]})

    def test_unsupported_body_and_budget_still_fail_closed(self):
        projections = self.read('ordinary')
        for name in ('mutate', 'large'):
            self.assert_unsupported(projections[name])


if __name__ == '__main__':
    unittest.main()
