#!/usr/bin/env python3
"""Check Java projections and Work citation binding through the packaged adapter."""
import hashlib
from pathlib import Path
import runpy
import shutil
import subprocess
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parents[1] / 'skills/codeclew/scripts'
JAVA = runpy.run_path(str(SCRIPTS / 'prepared_facts/java.py'))
TRANSPORT = runpy.run_path(str(SCRIPTS / 'docs_run_prepared.py'))
TRANSPORT['bound_facts'].__globals__.update(
    sha=lambda raw: 'sha256:' + hashlib.sha256(raw).hexdigest(), digest=repr,
)
JDK = shutil.which('java')
HAS_JDK = JDK and subprocess.run([JDK, '--source', '21', '-version'], capture_output=True).returncode == 0
PATH = 'src/catalog/Entry.java'
HELPER_PATH = 'src/catalog/Helper.java'
BODY = '''String change(String input, boolean permitted) {
        if (!permitted) return "blocked";
        final String stored = input.trim();
        final Helper helper = new Helper();
        helper.save(stored);
        return stored;
    }'''
FILE = f'package catalog;\nfinal class Entry {{\n    {BODY}\n}}\n'
HELPER = '''package catalog;
final class Helper {
    String latest;
    void save(String value) { this.latest = value; }
}
'''


def digest(raw):
    return 'sha256:' + hashlib.sha256(raw).hexdigest()


def blob(raw):
    return hashlib.sha1(f'blob {len(raw)}\0'.encode() + raw).hexdigest()


def source(file, full, text, *, revision='a' * 40, ref='s1', corrupt_blob=False):
    raw, payload = full.encode(), text.encode()
    start = raw.index(payload)
    end = start + len(payload)
    first = raw[:start].count(b'\n') + 1
    return {'file': file, 'revision': revision, 'service': 'catalog', 'text': text,
            'authority': 'EXACT_SNAPSHOT_TEXT', 'textDigest': digest(payload),
            'references': [ref], 'startByte': start, 'endByte': end, 'startLine': first,
            'endLine': first + payload.removesuffix(b'\n').count(b'\n'),
            'occurrence': {'snapshot': 'sha256:' + 'b' * 64, 'blob': '0' * 40 if corrupt_blob else blob(raw),
                           'startByte': start, 'endByte': end}}


def fixture(*, discovery=False, subranges=False, helper_cited=True, revision='a' * 40, repository=None,
            corrupt_blob=False):
    selected = BODY if subranges else FILE
    exact = {'C1': source(PATH, FILE, selected, revision=revision, corrupt_blob=corrupt_blob)}
    if helper_cited:
        exact['C2'] = source(HELPER_PATH, HELPER, HELPER, revision=revision, ref='s2')
    requirement = {'kind': 'DISCOVERY' if discovery else 'ENTRY', 'file': PATH if discovery else None,
                   'id': 'sample', 'rootDeclarations': [] if discovery else ['root'], 'sourceReferences': ['s1'],
                   'service': 'catalog', 'page': 'Change catalog'}
    units = TRANSPORT['full_files'](exact)
    operation = [{'id': 'BODY:root', 'obligationKind': 'BODY',
                  'record': {'declaration': 'root', 'sourceIds': ['body']}, 'sourceReferences': ['s1']}]
    process, context = TRANSPORT['operation'](requirement, [] if discovery else operation, exact, units)
    context.update(_revision=revision, _snapshot='sha256:' + 'c' * 64 + '/1', _repository=repository)
    rows, ranges = JAVA['project'](process, context, node=None)
    facts, rejected = TRANSPORT['bound_facts'](rows, ranges, exact, {site['id'] for site in context['sourceRanges']})
    return facts, ranges, rejected


@unittest.skipUnless(HAS_JDK, 'JDK 21 or newer required')
class PackagedJava(unittest.TestCase):
    def test_native_body_without_full_blob_still_projects_lexical_guards(self):
        rows, ranges, rejected = fixture(subranges=True)
        paths = [row for row in rows if row['kind'] == 'java-guarded-outcome']
        self.assertTrue(any('`permitted` is false' in row['condition'] and 'bypasses later' in row['text'] for row in paths))
        self.assertTrue(any(row.get('status') == 'SNAPSHOT_SOURCE_UNAVAILABLE' for row in rows))
        self.assertEqual(rejected, 0)
        self.assertEqual(ranges, {})

    def test_discovery_projects_only_covered_declarations_and_callee_witnesses(self):
        rows, ranges, rejected = fixture(discovery=True)
        path = next(row for row in rows if row['kind'] == 'java-guarded-outcome')
        self.assertIn('source declaration `catalog.Entry.change`', path['condition'])
        self.assertEqual(path['sources'], ['C1'])
        mutation = next(row for row in rows if row['kind'] == 'java-callee-path')
        self.assertIn('this.latest', mutation['text'])
        self.assertIn('C2', mutation['sources'])
        self.assertTrue(any(row['file'] == HELPER_PATH for row in ranges.values()))
        self.assertEqual(rejected, 0)

    def test_pinned_calculation_cannot_publish_uncovered_helper(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            for relative, code in ((PATH, FILE), (HELPER_PATH, HELPER)):
                file = repo / relative
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text(code)

            def git(*args):
                return subprocess.check_output(['git', '-C', directory, *args], stderr=subprocess.DEVNULL)

            git('init', '-q')
            git('add', '.')
            git('-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '-qm', 'Fixture')
            revision = git('rev-parse', 'HEAD').decode().strip()
            (repo / PATH).write_text('class Changed {}')
            visible, _, rejected = fixture(discovery=True, helper_cited=False, revision=revision, repository=directory)
            self.assertTrue(any(row['kind'] == 'java-guarded-outcome' for row in visible))
            self.assertTrue(any(row.get('status') == 'SNAPSHOT_SOURCE_UNAVAILABLE' for row in visible))
            self.assertFalse(any('this.latest' in row['text'] for row in visible))
            self.assertEqual(rejected, 0)
            wrong, _, rejected = fixture(discovery=True, subranges=True, helper_cited=False, revision=revision,
                                         repository=directory, corrupt_blob=True)
            self.assertEqual([row.get('status') for row in wrong], ['SNAPSHOT_SOURCE_UNAVAILABLE'])
            self.assertEqual(rejected, 0)


if __name__ == '__main__':
    unittest.main()
