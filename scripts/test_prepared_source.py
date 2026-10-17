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

    def test_native_bytes_are_bounded_without_becoming_captured_semantic_source(self):
        original = packet()
        original['items'].append({'kind': 'SOURCE_NATIVE', 'id': 'native-file', 'record': {
            'text': 'public record Request(string Id);', 'textDigest': 'engine-native-digest',
            'authority': 'REGISTERED_NATIVE_BYTES_ONLY_NO_SEMANTIC_ANALYSIS', 'bindingService': 'worker'}})
        view = json.loads(helper.project(original, Path('/local/packet.json'), 'digest', source_ids=['native-file']))
        self.assertEqual(view['sources'][0]['kind'], 'SOURCE_NATIVE')
        self.assertEqual(view['sources'][0]['record']['authority'], 'REGISTERED_NATIVE_BYTES_ONLY_NO_SEMANTIC_ANALYSIS')
        self.assertEqual(view['evidence']['unresolved'], original['unresolved'])
        self.assertEqual(view['sources'][0]['text'], original['items'][-1]['record']['text'])

    def test_changed_window_citations_survive_without_selected_source_rows(self):
        original = packet()
        window = {'representation': 'CHARACTER_WINDOW_PROJECTION_NOT_EXACT_FULL_SOURCE',
                  'before': '<title>Weather</title>', 'after': '<title>Weather demo</title>',
                  'beforeCitation': {'id': 'title-old', 'revision': 'old', 'file': 'index.html',
                                     'startLine': 2, 'endLine': 8, 'url': 'https://example.test/old#L2-L8'},
                  'afterCitation': {'id': 'title-new', 'revision': 'new', 'file': 'index.html',
                                    'startLine': 3, 'endLine': 9, 'url': 'https://example.test/new#L3-L9'}}
        reason = {'reason': 'REGISTERED_SOURCE_SCOPE_FILE_CHANGED', 'file': 'index.html',
                  'changedWindow': window}
        original['comparison'] = {'status': 'CHANGED', 'reasonExamples': [reason]}
        original['items'] = [{'kind': 'EVIDENCE_CHANGE', 'id': 'change', 'record': reason}]
        view = json.loads(helper.project(original, Path('/local/packet.json'), 'digest'))
        self.assertEqual(view['sources'], [])
        self.assertEqual(view['handles'][0]['change']['changedWindow'], window)
        # Mandatory comparison reasons remain available even with no optional handles.
        minimal = json.loads(helper.project(original, Path('/local/packet.json'), 'digest', max_handles=0))
        self.assertEqual(minimal['evidence']['comparison']['reasonExamples'][0]['changedWindow'], window)

    def test_excessive_limits_fail_closed_instead_of_truncating_them(self):
        original = packet()
        original['unresolved'] = {'large': ['MISSING_INPUT'] * 2000}
        with self.assertRaisesRegex(helper.ViewError, 'METADATA_EXCEEDS'):
            helper.project(original, Path('/local/packet.json'), 'digest', max_bytes=2048)

    def test_fetch_uses_one_exact_command_and_keeps_private_raw_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture = root / 'fixture.json'
            supplied = packet()
            supplied['evidenceSet'] = {'schema': 'codeclew-prepared-source-evidence/1.0', 'reference': {'digest': 'engine-owned'}}
            raw = (json.dumps(supplied, indent=2) + '\n').encode()
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
            self.assertEqual(invocations, [['docs', 'evidence', 'retain-source', '--root', str(root / 'docs'),
                '--service', 'worker', '--symbol', 'exact:dispatch',
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

    def test_compare_is_one_engine_call_preserves_typed_unknown_and_never_uses_git(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = packet()
            original['comparison'] = {'status': 'UNKNOWN', 'reasonExamples': [{'reason': 'DEPENDENCY_UNAVAILABLE'}],
                                      'scope': 'REGISTERED_INPUTS_ONLY_NOT_RUNTIME_BEHAVIOR'}
            original['items'] = []
            fixture = root / 'response.json'
            fixture.write_text(json.dumps(original))
            calls = root / 'calls.jsonl'
            launcher = root / 'candidate-clew'
            launcher.write_text('#!' + sys.executable + '\nimport json,sys\nfrom pathlib import Path\n' +
                f'with Path({str(calls)!r}).open("a") as f: f.write(json.dumps(sys.argv[1:])+"\\n")\n' +
                f'sys.stdout.buffer.write(Path({str(fixture)!r}).read_bytes())\n')
            launcher.chmod(0o700)
            previous = root / 'previous.json'
            previous.write_text('{"evidenceSet":"opaque"}')
            output = root / 'current.json'
            command = [sys.executable, '-I', '-S', str(HELPER), 'compare', '--clew', str(launcher),
                       '--root', str(root / 'docs'), '--input', str(previous), '--snapshot', 'target', '--output', str(output)]
            result = subprocess.run(command, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stdout)
            view = json.loads(result.stdout)
            self.assertEqual(view['evidence']['comparison'], original['comparison'])
            self.assertEqual(json.loads(calls.read_text()), ['docs', 'evidence', 'compare-source', '--root', str(root / 'docs'),
                             '--input', str(previous.resolve()), '--snapshot', 'target'])
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)
            self.assertEqual(previous.read_text(), '{"evidenceSet":"opaque"}')
            # A superficially valid context without an engine verdict is not a comparison.
            fixture.write_text(json.dumps(packet()))
            command[-1] = str(root / 'wrong.json')
            wrong = subprocess.run(command, capture_output=True)
            self.assertEqual(wrong.returncode, 2)
            self.assertIn('ENGINE_COMPARE_RESULT_MISSING', json.loads(wrong.stdout)['reason'])
            self.assertFalse(json.loads(wrong.stdout)['evidenceDelivered'])

    def test_historical_drill_is_one_exact_command_and_requires_immutable_admission(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = packet()
            original['evidenceValidation'] = {'mode': 'PINNED_SOURCE_SNAPSHOT_CLOSURE',
                                               'liveCheckoutRechecked': False}
            fixture = root / 'response.json'
            fixture.write_text(json.dumps(original))
            calls = root / 'calls.jsonl'
            launcher = root / 'candidate-clew'
            launcher.write_text('#!' + sys.executable + '\nimport json,sys\nfrom pathlib import Path\n' +
                f'with Path({str(calls)!r}).open("a") as f: f.write(json.dumps(sys.argv[1:])+"\\n")\n' +
                f'sys.stdout.buffer.write(Path({str(fixture)!r}).read_bytes())\n')
            launcher.chmod(0o700)
            output = root / 'historical.json'
            command = [sys.executable, '-I', '-S', str(HELPER), 'drill', '--clew', str(launcher),
                       '--root', str(root / 'docs'), '--snapshot', 'old-snapshot', '--service', 'worker',
                       '--source', 'body-1', '--output', str(output)]
            result = subprocess.run(command, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stdout)
            view = json.loads(result.stdout)
            self.assertEqual(view['sources'][0]['record']['revision'], 'rev-1')
            self.assertEqual(view['sources'][0]['text'], original['items'][1]['record']['text'])
            self.assertEqual(json.loads(calls.read_text()), ['docs', 'context', '--root', str(root / 'docs'),
                             '--service', 'worker', '--snapshot', 'old-snapshot', '--limit', '20', '--source', 'body-1'])
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)
            original.pop('evidenceValidation')
            fixture.write_text(json.dumps(original))
            for index, validation in enumerate([None, [], {}, 'not-admitted']):
                original['evidenceValidation'] = validation
                fixture.write_text(json.dumps(original))
                command[-1] = str(root / f'unadmitted-{index}.json')
                rejected = subprocess.run(command, capture_output=True)
                self.assertEqual(rejected.returncode, 2)
                self.assertIn('PINNED_SOURCE_ADMISSION_MISSING', json.loads(rejected.stdout)['reason'])
                self.assertFalse(json.loads(rejected.stdout)['evidenceDelivered'])
                self.assertEqual(rejected.stderr, b'')

    def test_maintenance_shows_only_changed_source_and_retains_values_limits_and_citations(self):
        original = packet('new λ🙂 body')
        original['comparison'] = {'status': 'CHANGED', 'selectedSourceBytes': 'CHANGED',
            'baselineBinding': {'snapshot': 'old', 'revision': 'old-rev'},
            'targetBinding': {'snapshot': 'new', 'revision': 'new-rev'},
            'reasonExamples': [{'reason': 'duplicate header'}]}
        before = {'id': 'body-1', 'revision': 'old-rev', 'file': 'src/worker.py', 'startLine': 3, 'endLine': 4}
        after = {**before, 'revision': 'new-rev'}
        window = {'before': 'old value', 'after': 'new value', 'beforeCitation': before, 'afterCitation': after}
        original['items'] += [{'kind': 'EVIDENCE_CHANGE', 'id': 'change-0', 'record': {
            'reason': 'SELECTED_SOURCE_BYTES_CHANGED', 'source': 'body-1', 'changedWindow': window}},
            {'kind': 'SOURCE_BINDING_UPDATE', 'id': 'binding-0', 'record': {
                'source': 'body-1', 'before': before, 'after': after}},
            {'kind': 'SOURCE_NATIVE', 'id': 'unchanged-native', 'record': {
                'text': 'UNCHANGED_BODY_MUST_NOT_REPLAY', 'authority': 'NATIVE_BYTES_ONLY',
                'membershipCoverage': 'REGISTERED_ONLY'}}]
        result = helper.maintenance(original, Path('/packet.json'), 'digest')
        view = json.loads(result)
        self.assertEqual(view['status'], 'CHANGED')
        self.assertNotIn('reasonExamples', view['evidence']['comparison'])
        self.assertEqual(view['changes'][0]['record']['changedWindow'], window)
        self.assertEqual(view['citationUpdates'][0]['binding']['after'], after)
        pointer = view['citationUpdates'][0]['binding']['completeBeforeAfter']['pointer']
        binding = json.loads(helper.local_record(original, Path('/packet.json'), 'digest', pointer, 12000))
        self.assertEqual(binding['item']['record']['before'], before)
        with self.assertRaisesRegex(helper.ViewError, 'INVALID_LOCAL_PACKET_POINTER'):
            helper.local_record(original, Path('/packet.json'), 'digest', '/items/01', 12000)
        self.assertEqual(view['sources'][0]['text'], 'new λ🙂 body')
        self.assertNotIn('UNCHANGED_BODY_MUST_NOT_REPLAY', result)
        self.assertIn('NATIVE_BYTES_ONLY', result)
        self.assertEqual(view['evidence']['unresolved'], original['unresolved'])
        metadata = json.loads(helper.maintenance(original, Path('/packet.json'), 'digest', max_sources=0))
        self.assertEqual(metadata['sources'], [])
        self.assertEqual(metadata['changes'], view['changes'])
        self.assertEqual(metadata['projection']['sourceBodies'], 'METADATA_ONLY')
        self.assertLessEqual(len(result.encode()), 12000)

    def test_maintenance_unchanged_unknown_and_local_record_pages_are_explicit(self):
        original = packet('unchanged body')
        original['comparison'] = {'status': 'UNCHANGED', 'selectedSourceBytes': 'UNCHANGED'}
        unchanged = json.loads(helper.maintenance(original, Path('/packet.json'), 'digest'))
        self.assertEqual(unchanged['sources'], [])
        original['comparison']['status'] = 'UNKNOWN'
        original['targetNativeInputLimits'] = [{'reason': 'NATIVE_INPUT_UNAVAILABLE', 'input': 'receiver.cs'}]
        unknown = json.loads(helper.maintenance(original, Path('/packet.json'), 'digest'))
        self.assertEqual(unknown['status'], 'UNKNOWN')
        self.assertEqual(unknown['evidence']['targetNativeInputLimits'], original['targetNativeInputLimits'])
        original['items'] = [{'kind': 'EVIDENCE_CHANGE', 'id': f'change-{i}', 'record': {
            'reason': 'CALLEE_CHANGED', 'beforeValues': 'before ' * 90, 'afterValues': 'after ' * 90}}
            for i in range(8)]
        seen, offset = [], 0
        while True:
            view = json.loads(helper.maintenance(original, Path('/packet.json'), 'digest', max_bytes=3000, record_offset=offset))
            self.assertLessEqual(len(helper.encode(view).encode()), 3000)
            seen.extend(c['id'] for c in view['changes'])
            if view['continuation'] is None: break
            self.assertEqual(view['continuation']['kind'], 'LOCAL_MAINTENANCE_RECORDS')
            self.assertGreater(view['continuation']['recordOffset'], offset)
            offset = view['continuation']['recordOffset']
        self.assertEqual(seen, [f'change-{i}' for i in range(8)])
        with self.assertRaisesRegex(helper.ViewError, 'EXCEEDS_OUTPUT_BUDGET'):
            helper.maintenance(original, Path('/packet.json'), 'digest', max_bytes=1024)

    def test_zero_sources_is_a_supported_metadata_only_cli_view(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'packet.json'
            supplied = packet()
            supplied['items'].append({'kind': 'SOURCE_NATIVE', 'id': 'native', 'record': {'text': 'DO_NOT_SHOW', 'authority': 'NATIVE_ONLY'}})
            path.write_text(json.dumps(supplied))
            for option in [('--max-sources', '0'), ('--metadata-only',)]:
                run = subprocess.run([sys.executable, '-I', '-S', str(HELPER), 'read', '--input', str(path), *option], capture_output=True)
                self.assertEqual(run.returncode, 0, run.stderr)
                view = json.loads(run.stdout)
                self.assertEqual(view['sources'], [])
                self.assertEqual(view['evidence']['unresolved'], packet()['unresolved'])
                self.assertIn('NATIVE_ONLY', run.stdout.decode())
                self.assertNotIn('DO_NOT_SHOW', run.stdout.decode())

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
