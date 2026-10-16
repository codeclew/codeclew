#!/usr/bin/env python3
"""Behavioral tests for the portable prepared SOURCE packet helper."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
HELPER = ROOT / 'skills/codeclew/scripts/prepared_source.py'
spec = importlib.util.spec_from_file_location('prepared_source', HELPER)
helper = importlib.util.module_from_spec(spec)
sys.dont_write_bytecode = True
spec.loader.exec_module(helper)


def packet(text='def dispatch():\n    enqueue()\n'):
    return {'schema': 'codeclew-docs-page/1.0', 'snapshot': 'snapshot-1',
            'authority': 'PINNED_SNAPSHOT_NOT_REVERIFIED', 'narrativeAuthority': 'AGENT_INFERRED',
            'sourceAuthorities': {'worker': 'CAPTURED_SOURCE'}, 'subject': 'service:worker',
            'inputDigest': 'input-1', 'contextDigest': 'context-1', 'total': 3,
            'unresolved': {'receiver': 'NOT_CAPTURED'}, 'omitted': ['OUTSIDE_SCOPE'],
            'nextCursor': 'cursor-2', 'bindings': {'generated': {'digest': 'generated-1'}},
            'items': [{'kind': 'COVERAGE', 'id': 'worker', 'revision': 'rev-1',
                       'coverage': 'SYNTAX', 'boundaries': ['CALL_TARGETS_UNRESOLVED'],
                       'callAuthority': 'SYNTAX_UNRESOLVED'},
                      {'kind': 'SOURCE', 'id': 'body-1', 'record': {
                          'id': 'body-1', 'service': 'worker', 'authority': 'EXACT_SNAPSHOT_TEXT',
                          'revision': 'rev-1', 'file': 'src/worker.py', 'startLine': 3, 'endLine': 4,
                          'text': text, 'textDigest': hashlib.sha256(text.encode()).hexdigest(),
                          'occurrence': {'blob': 'blob-1', 'snapshot': 'source-snapshot-1'}}},
                      {'kind': 'DEPENDENCY', 'id': 'callee-1', 'record': {
                          'fullRecordDigest': 'callee-digest', 'normalized': {
                              'authority': 'SYNTAX', 'targetStatus': 'UNRESOLVED',
                              'documentation': {'boundaries': ['NOT_RUNTIME_PROOF']},
                              'longCatalogue': 'NOT_FOR_STDOUT' * 10000}}}]}


class PreparedSourceTest(unittest.TestCase):
    def test_projection_keeps_limits_bindings_and_exact_selected_text(self):
        original = packet()
        before = copy.deepcopy(original)
        result = helper.project(original, Path('/local/packet.json'), 'file-digest')
        view = json.loads(result)
        self.assertEqual(original, before)
        self.assertEqual(view['evidence'], {k: v for k, v in original.items() if k != 'items'})
        self.assertEqual(view['coverage'][0], original['items'][0])
        self.assertEqual(view['sources'][0]['text'], original['items'][1]['record']['text'])
        self.assertEqual(view['sources'][0]['record']['occurrence'], original['items'][1]['record']['occurrence'])
        self.assertTrue(view['sources'][0]['textDelivery']['complete'])
        self.assertEqual(view['recordLimits'][0]['items'], ['/items/2'])
        self.assertEqual(view['recordLimits'][0]['limits']['record']['normalized'], {
            'authority': 'SYNTAX', 'targetStatus': 'UNRESOLVED',
            'documentation': {'boundaries': ['NOT_RUNTIME_PROOF']}})
        self.assertNotIn('NOT_FOR_STDOUT', result)
        self.assertLessEqual(len(result.encode()), 12000)

    def test_unicode_budget_and_local_continuation_are_explicit(self):
        text = 'λ🙂' * 20000
        original = packet(text)
        first = helper.project(original, Path('/local/packet.json'), 'digest',
                               source_ids=['body-1'], max_bytes=2400)
        view = json.loads(first)
        row = view['sources'][0]
        self.assertLessEqual(len(first.encode('utf-8')), 2400)
        self.assertFalse(row['textDelivery']['complete'])
        self.assertEqual(row['text'], text[:row['textDelivery']['nextOffset']])
        next_view = json.loads(helper.project(original, Path('/local/packet.json'), 'digest',
                                             source_ids=['body-1'], text_offset=row['textDelivery']['nextOffset']))
        next_row = next_view['sources'][0]
        offset = next_row['textDelivery']['offset']
        self.assertEqual(next_row['text'], text[offset:offset + len(next_row['text'])])
        self.assertEqual(view['evidence']['omitted'], original['omitted'])
        self.assertEqual(view['evidence']['nextCursor'], 'cursor-2')

    def test_aliases_and_optional_handles_do_not_silently_disappear(self):
        original = packet()
        original['items'].append({'kind': 'SOURCE_ALIAS', 'id': 'alias-1',
                                  'authority': 'EXACT_SNAPSHOT_TEXT', 'coveredBy': ['body-1']})
        view = json.loads(helper.project(original, Path('/local/packet.json'), 'digest', max_handles=0))
        self.assertEqual(view['projection']['aliasesNotShown'], 1)
        self.assertEqual(view['projection']['handlesNotShown'], 1)
        self.assertEqual(view['recordLimits'][1], {'items': ['/items/3'],
                                                'limits': {'authority': 'EXACT_SNAPSHOT_TEXT'}})
        with self.assertRaises(helper.ViewError):
            helper.project(original, Path('/local/packet.json'), 'digest', source_ids=['alias-1'])

    def test_excessive_limits_fail_closed_instead_of_truncating_them(self):
        original = packet()
        original['unresolved'] = {'large': ['MISSING_INPUT'] * 2000}
        with self.assertRaisesRegex(helper.ViewError, 'METADATA_EXCEEDS'):
            helper.project(original, Path('/local/packet.json'), 'digest', max_bytes=2048)

    def test_fetch_uses_one_exact_command_and_keeps_private_raw_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture = root / 'fixture.json'
            raw = (json.dumps(packet(), indent=2) + '\n').encode()
            fixture.write_bytes(raw)
            calls = root / 'calls.jsonl'
            launcher = root / 'installed-clew'
            launcher.write_text('#!' + sys.executable + '\n' +
                'import json,sys\nfrom pathlib import Path\n' +
                f'with Path({str(calls)!r}).open("a") as f: f.write(json.dumps(sys.argv[1:])+"\\n")\n' +
                f'sys.stdout.buffer.write(Path({str(fixture)!r}).read_bytes())\n' +
                'sys.stderr.write("PRIVATE_PROGRESS")\n')
            launcher.chmod(0o700)
            output = root / 'context.json'
            command = [sys.executable, '-I', '-S', str(HELPER), 'fetch', '--clew', str(launcher),
                       '--root', str(root / 'docs'), '--snapshot', 'snapshot-1', '--service', 'worker',
                       '--symbol', 'exact:dispatch', '--output', str(output)]
            run = subprocess.run(command, capture_output=True, check=False)
            self.assertEqual(run.returncode, 0, run.stderr)
            view = json.loads(run.stdout)
            self.assertEqual(output.read_bytes(), raw)
            self.assertEqual(view['packet']['sha256'], hashlib.sha256(raw).hexdigest())
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)
            self.assertEqual(output.with_name('context.json.stderr').read_text(), 'PRIVATE_PROGRESS')
            self.assertNotIn(b'PRIVATE_PROGRESS', run.stdout + run.stderr)
            invocations = [json.loads(line) for line in calls.read_text().splitlines()]
            self.assertEqual(invocations, [['docs', 'context', '--root', str(root / 'docs'),
                '--service', 'worker', '--symbol', 'exact:dispatch', '--format', 'compact', '--limit', '40',
                '--snapshot', 'snapshot-1']])
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 2)
            self.assertEqual(output.read_bytes(), raw)
            self.assertEqual(len(calls.read_text().splitlines()), 1)
            # Reusing the saved packet never invokes clew.
            read = subprocess.run([sys.executable, '-I', '-S', str(HELPER), 'read', '--input', str(output)], capture_output=True)
            self.assertEqual(read.returncode, 0)
            self.assertEqual(len(calls.read_text().splitlines()), 1)
            # A failed refresh retains both private streams without retries/dumps.
            launcher.write_text(launcher.read_text() + '\nsys.exit(42)\n')
            failure_command = command[:]
            position = failure_command.index('--snapshot')
            failure_command[position:position + 2] = ['--refresh']
            failure_command[-1] = str(root / 'failed.json')
            failed = subprocess.run(failure_command, capture_output=True)
            self.assertEqual(failed.returncode, 2)
            self.assertFalse(json.loads(failed.stdout)['evidenceDelivered'])
            self.assertEqual((root / 'failed.json').read_bytes(), raw)
            self.assertNotIn(b'PRIVATE_PROGRESS', failed.stdout + failed.stderr)
            self.assertEqual(len(calls.read_text().splitlines()), 2)

    def test_malformed_or_overbudget_packet_never_prints_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'bad.json'
            for raw in ['NOT_JSON_PRIVATE_SOURCE', json.dumps({**packet(), 'unresolved': ['PRIVATE_LIMIT'] * 5000})]:
                path.write_text(raw)
                run = subprocess.run([sys.executable, '-I', '-S', str(HELPER), 'read', '--input', str(path), '--max-bytes', '1024'], capture_output=True)
                self.assertEqual(run.returncode, 2)
                self.assertLessEqual(len(run.stdout), 1024)
                self.assertFalse(json.loads(run.stdout)['evidenceDelivered'])
                self.assertNotIn(b'PRIVATE', run.stdout + run.stderr)


if __name__ == '__main__':
    unittest.main()
