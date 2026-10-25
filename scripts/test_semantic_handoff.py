#!/usr/bin/env python3
"""Owned deterministic handoff regressions; no real models, private code or publish."""
import copy
import json
from pathlib import Path
import runpy
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
D = runpy.run_path(str(ROOT / 'skills/codeclew/examples/semantic-repair/run_owned.py'))
H = D['H']


class HandoffTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fixture = D['setup'](self.root, 'omission')
        self.launcher = D['fake_launcher'](self.root)

    def write(self, name, value):
        return D['write'](self.root, name, value)

    def read(self, name):
        return json.loads((self.root / name).read_text())

    def invoke(self, action, input_name, output, **fields):
        args = [action, '--input', input_name, '--output', output]
        for key, value in fields.items():
            args += ['--' + key, str(value)]
        return D['invoke'](self.root, *args)

    def prepared(self):
        code, result = self.invoke('prepare', 'request.json', 'handoff')
        self.assertEqual(code, 0, result)
        self.write('first-review.json', D['finding'](self.root, self.fixture))

    def ready(self):
        self.prepared()
        code, result = self.invoke('repair-context', 'handoff', 'repair', review='first-review.json')
        self.assertEqual(code, 0, result)
        raw = (self.root / 'repair/repair-context.json').read_bytes(); context = json.loads(raw)
        self.write('patch.json', {'schema': 'codeclew-semantic-replacements/1.0', 'binding': context['binding'],
            'contextDigest': H['sha'](raw), 'author': context['author'], 'replacements': [{
                'path': self.fixture['path'], 'old': context['targets'][0]['old'], 'new': self.fixture['replacement']}]})

    def apply(self, output='applied'):
        return self.invoke('apply', 'repair', output, patch='patch.json', launcher=self.launcher, root=self.root / 'docs')

    def assert_blocked_before_submit(self, expected):
        code, result = self.apply()
        self.assertEqual(code, 2, result); self.assertIn(expected, result['reason'])
        self.assertEqual(result['nativeCallsAttempted'], 0)
        self.assertFalse((self.root / 'native-calls.jsonl').exists())
        self.assertFalse((self.root / 'repair/apply-claimed.json').exists())

    def test_three_counterexamples_full_lifecycle_and_unchanged_siblings(self):
        for case in ('omission', 'order', 'literal'):
            root = self.root / case; root.mkdir()
            result = D['run'](root, case)
            self.assertEqual(result['status'], 'REVIEWED_BOUNDED_CANDIDATE')
            self.assertEqual(result['nativeMeaningReview'], 'UNASSESSED')
            before = json.loads((root / 'native-before/input.original.json').read_text())
            after = json.loads((root / 'applied/input.original.json').read_text())
            self.assertEqual(before['operations'][1], after['operations'][1])
            if case == 'literal':
                self.assertNotEqual(before['uncertainties'][0], after['uncertainties'][0])
                self.assertEqual(before['uncertainties'][1:], after['uncertainties'][1:])
                self.assertEqual(after['uncertainties'][0], 'AUTHORITY is worker in the retained declaration; runtime use remains unproven.')
            else:
                self.assertEqual(before['uncertainties'], after['uncertainties'])
            self.assertEqual(json.loads((root / 'repair/first-review.original.json').read_text())['verdict'], 'REJECT')
            self.assertEqual(json.loads((root / 'handoff/review-context.json').read_text())['native']['identity']['status'], 'READY_WITH_LIMITATIONS')
            context = json.loads((root / 'repair/repair-context.json').read_text())
            self.assertNotIn('proposal', context)
            self.assertNotIn('Untouched sibling', json.dumps(context))
            self.assertEqual(context['targets'][0]['path'], '/operations/0/steps' if case == 'order' else '/operations/0/summary')
            self.assertTrue(all(e['native']['authority'] == 'EXACT_SNAPSHOT_TEXT' for e in context['evidence']))
            calls = [json.loads(line) for line in (root / 'native-calls.jsonl').read_text().splitlines()]
            self.assertEqual([c[:3] for c in calls], [['docs', 'proposal', 'submit'], ['docs', 'proposal', 'show']])
            self.assertFalse(any('publish' in c or 'prepare' in c for c in calls))

    def test_initial_navigation_exposes_unread_candidate_not_an_unavailable_claim(self):
        self.prepared(); context = self.read('handoff/review-context.json')
        items = context['navigation']['items']
        self.assertTrue(any(i['reference'] == 'd-candidate' and i['sourceReferences'] == ['s-candidate'] for i in items))
        self.assertTrue(any(i['reference'] == 's-candidate' and i['textAvailability'] == 'NOT_YET_ARCHIVED_TEXT' for i in items))
        self.assertEqual(context['navigation']['modelRead'], 'UNATTESTED')
        self.assertIn('proposal', context)
        self.assertEqual(context['binding']['proposalDigest'], H['sha']((self.root / 'native-before/input.original.json').read_bytes()))

    def test_stale_base_work_archive_and_first_review_are_blocked(self):
        for index, name in enumerate(('native-before/input.original.json', 'work/initial.stdout', 'first-review.json')):
            with tempfile.TemporaryDirectory() as directory:
                old_root = self.root; self.root = Path(directory)
                self.fixture = D['setup'](self.root, 'omission'); self.launcher = D['fake_launcher'](self.root)
                self.ready(); (self.root / name).write_bytes((self.root / name).read_bytes() + b' ')
                self.assert_blocked_before_submit('FIRST_REVIEW_CHANGED' if index == 2 else 'STALE_HANDOFF_INPUT')
                self.root = old_root

    def test_foreign_snapshot_and_work_fail_before_review_handoff(self):
        for field in ('snapshot', 'work'):
            value = self.read('reviewer-read/source.stdout'); value[field] = 'foreign'
            self.write('reviewer-read/source.stdout', value)
            self.prepared()
            code, result = self.invoke('repair-context', 'handoff', 'foreign-' + field, review='first-review.json')
            self.assertEqual(code, 2); self.assertIn('IDENTITY_CHANGED', result['reason'])
            self.assertFalse((self.root / 'native-calls.jsonl').exists())
            # Restore initial setup and use a separate stage for the next case.
            if field == 'snapshot':
                (self.root / 'handoff').rename(self.root / 'old-handoff')
                original = D['page']([self.fixture['sources'][1], self.fixture['sources'][2]])
                self.write('reviewer-read/source.stdout', original)

    def test_missing_source_cannot_be_replaced_by_navigation_or_author_prose(self):
        self.prepared(); value = self.read('first-review.json'); value['acquiredPages'] = []
        self.write('first-review.json', value)
        code, result = self.invoke('repair-context', 'handoff', 'repair', review='first-review.json')
        self.assertEqual(code, 2); self.assertIn('NEEDS_EVIDENCE', result['reason'])
        self.assertFalse((self.root / 'native-calls.jsonl').exists())

    def test_ambiguous_same_reference_is_refused(self):
        value = self.read('reviewer-read/source.stdout')
        value['items'].append({'kind': 'DEPENDENCY', 'id': 'ambiguous', 'reference': 's-candidate',
            'record': {'kind': 'SYMBOL', 'id': 'ambiguous', 'symbol': 'Other', 'sourceIds': [], 'normalized': {}}})
        value['total'] += 1; self.write('reviewer-read/source.stdout', value)
        self.prepared()
        code, result = self.invoke('repair-context', 'handoff', 'repair', review='first-review.json')
        self.assertEqual(code, 2); self.assertIn('MISSING_OR_AMBIGUOUS_EVIDENCE', result['reason'])

    def test_guard_rejects_old_value_wrong_binding_disallowed_path_and_dropped_evidence(self):
        self.ready(); original = self.read('patch.json')
        mutations = [
            ('EXACT_OLD_VALUE_CHANGED', lambda p: p['replacements'][0]['old'].update(text='stale')),
            ('PATCH_BINDING_OR_AUTHOR_CHANGED', lambda p: p['binding'].update(snapshot='foreign')),
            ('PATCH_BINDING_OR_AUTHOR_CHANGED', lambda p: p['binding'].update(work='foreign')),
            ('DISALLOWED_OR_MISSING_PATCH', lambda p: p['replacements'][0].update(path='/operations/1/summary')),
            ('DROPPED_SOURCE_REFERENCE', lambda p: p['replacements'][0]['new'].update(evidence=['s-candidate'])),
            ('MISSING_PATCH_EVIDENCE', lambda p: p['replacements'][0]['new']['evidence'].append('s-foreign')),
            ('PATCH_BINDING_OR_AUTHOR_CHANGED', lambda p: p['author'].update(model='gpt-6.1-sol')),
        ]
        for index, (expected, change) in enumerate(mutations):
            patch = copy.deepcopy(original); change(patch); self.write('patch.json', patch)
            code, result = self.apply('bad-' + str(index))
            self.assertEqual(code, 2, result); self.assertIn(expected, result['reason'])
            self.assertEqual(result['nativeCallsAttempted'], 0)
        self.assertFalse((self.root / 'native-calls.jsonl').exists())

    def test_early_insufficient_evidence_is_terminal_and_actionable(self):
        self.prepared(); value = self.read('first-review.json')
        value.update(verdict='NEEDS_EVIDENCE', inspected=[], acquiredPages=[])
        value['issues'][0].update(evidence=[], neededReads=[{'query': {'kind': 'SYMBOL', 'symbolContains': 'Collector'}}])
        self.write('first-review.json', value)
        code, result = self.invoke('repair-context', 'handoff', 'refusal', review='first-review.json')
        self.assertEqual(code, 2); self.assertEqual(result['status'], 'INSUFFICIENT_EVIDENCE')
        self.assertFalse((self.root / 'refusal/repair-context.json').exists())
        self.assertEqual(self.read('refusal/first-review.original.json'), value)
        self.assertFalse(result['automaticRetry'])

    def test_reviewer_separation_exact_digest_and_approval_contradiction(self):
        self.prepared(); original = self.read('first-review.json')
        for index, (field, replacement, reason) in enumerate([
            ('reviewer', self.read('request.json')['author'], 'REVIEWER_IDENTITY_CHANGED'),
            ('contextDigest', 'stale', 'REVIEW_BINDING_MISMATCH'),
            ('assessedOperations', [], 'INITIAL_OPERATION_COVERAGE_INCOMPLETE'),
            ('verdict', 'APPROVE', 'CONTRADICTORY_REVIEW_APPROVAL')]):
            value = copy.deepcopy(original); value[field] = replacement; self.write('first-review.json', value)
            code, result = self.invoke('repair-context', 'handoff', 'bad-review-' + str(index), review='first-review.json')
            self.assertEqual(code, 2); self.assertIn(reason, result['reason'])

    def test_native_assertion_reference_is_preserved_without_requiring_a_claim_list(self):
        self.assertEqual(H['references']({'text': 'Owned', 'evidence': ['s1'], 'checks': [
            {'kind': 'factEquals', 'evidence': 'd1', 'field': 'kind', 'expected': 'RETURN'}]}), {'s1': 1, 'd1': 1})

    def test_one_native_attempt_preserves_failure_and_never_retries(self):
        self.ready(); self.write('native-response.json', {'error': {'message': 'owned failure'}})
        code, result = self.apply(); self.assertEqual(code, 2)
        self.assertEqual(result['nativeCallsAttempted'], 1)
        self.assertTrue((self.root / 'repair/apply-claimed.json').exists())
        original = (self.root / 'applied/submit.stdout').read_bytes()
        code, result = self.apply('again'); self.assertEqual(code, 2)
        self.assertEqual(result['nativeCallsAttempted'], 0)
        self.assertEqual(len((self.root / 'native-calls.jsonl').read_text().splitlines()), 1)
        self.assertEqual((self.root / 'applied/submit.stdout').read_bytes(), original)

    def test_native_ready_without_affected_review_stays_unreviewed_and_native_failure_blocks_acceptance(self):
        self.ready(); self.write('native-response.json', D['native_page']('owned-repaired', status='NEEDS_REPAIR'))
        code, result = self.apply(); self.assertEqual(code, 0, result)
        self.assertEqual(result['meaningStatus'], 'NOT_REVIEWED')
        value = D['finding'](self.root, self.fixture, 'applied/affected-review-context.json', True, 'APPROVE')
        self.write('affected.json', value)
        code, result = self.invoke('finish', 'applied', 'final', review='affected.json')
        self.assertEqual(code, 2); self.assertEqual(result['status'], 'NOT_ACCEPTED')
        self.assertEqual(result['nativeMeaningReview'], 'UNASSESSED')
        self.assertEqual(result['independentMeaningVerdict'], 'APPROVE')

    def test_affected_review_requires_all_paths_sources_and_preserves_reject(self):
        self.ready(); code, result = self.apply(); self.assertEqual(code, 0, result)
        original = D['finding'](self.root, self.fixture, 'applied/affected-review-context.json', True, 'APPROVE')
        for index, (field, value, reason) in enumerate([
            ('assessedPaths', [], 'AFFECTED_TARGET_COVERAGE_INCOMPLETE'),
            ('inspected', original['inspected'][:1], 'AFFECTED_EVIDENCE_COVERAGE_INCOMPLETE')]):
            review = copy.deepcopy(original); review[field] = value; self.write('affected.json', review)
            code, result = self.invoke('finish', 'applied', 'bad-final-' + str(index), review='affected.json')
            self.assertEqual(code, 2); self.assertIn(reason, result['reason'])
        self.write('affected.json', D['finding'](self.root, self.fixture, 'applied/affected-review-context.json', True))
        code, result = self.invoke('finish', 'applied', 'final', review='affected.json')
        self.assertEqual(code, 2); self.assertEqual(result['independentMeaningVerdict'], 'REJECT')
        self.assertEqual(result['status'], 'NOT_ACCEPTED')

    def prepare_affected_approval(self):
        self.ready(); code, result = self.apply(); self.assertEqual(code, 0, result)
        self.write('affected.json', D['finding'](self.root, self.fixture, 'applied/affected-review-context.json', True, 'APPROVE'))

    def rewrite_applied_native(self, identity_changes=None, diagnostics=None):
        # Keep the archive internally consistent, isolating the reviewed-result
        # binding check from ordinary corrupt-page/manifest guards.
        page = self.read('applied/submit-collection/page-0001.stdout')
        page.update(identity_changes or {})
        if diagnostics is not None:
            page.update(items=diagnostics, total=len(diagnostics))
        self.write('applied/submit.stdout', page)
        self.write('applied/submit-collection/page-0001.stdout', page)
        self.write('applied/submit-collection/items.json', page['items'])
        self.write('applied/submit-collection/diagnostics.json', [r for r in page['items'] if r['kind'] == 'DIAGNOSTIC'])
        manifest = self.read('applied/submit-collection/manifest.json')
        manifest.update(identity=D['T']['page_identity'](page, page['proposal']), rows=page['total'])
        self.write('applied/submit-collection/manifest.json', manifest)

    def test_finish_rejects_another_internally_consistent_proposal_in_same_work(self):
        self.prepare_affected_approval()
        reviewed = (self.root / 'applied/affected-review-context.json').read_bytes()
        binding = self.read('applied/origin.json')['binding']
        self.rewrite_applied_native({'proposal': 'owned-other-same-work-proposal'})
        code, result = self.invoke('finish', 'applied', 'mixed-final', review='affected.json')
        self.assertEqual(code, 2, result)
        self.assertEqual(result['status'], 'BLOCKED')
        self.assertIn('FINAL_NATIVE_BINDING_CHANGED', result['reason'])
        self.assertEqual(result['nativeCallsAttempted'], 0)
        self.assertEqual((self.root / 'applied/affected-review-context.json').read_bytes(), reviewed)
        self.assertEqual(self.read('applied/origin.json')['binding'], binding)

    def test_finish_rejects_same_proposal_metadata_and_diagnostics_changed_after_review(self):
        self.prepare_affected_approval()
        names = ['applied/submit.stdout', 'applied/submit-collection/page-0001.stdout',
                 'applied/submit-collection/items.json', 'applied/submit-collection/diagnostics.json',
                 'applied/submit-collection/manifest.json']
        originals = {name: (self.root / name).read_bytes() for name in names}
        cases = [({'status': 'READY'}, None), ({'meaningReview': 'VERIFIED'}, None),
                 ({'readDigest': 'other-read-set'}, None), ({'narrativeDigest': 'other-narrative'}, None),
                 ({}, [{'kind': 'DIAGNOSTIC', 'id': 'new-warning', 'record': {'code': 'NEW_LIMIT', 'severity': 'WARNING'}}])]
        for index, (changes, diagnostics) in enumerate(cases):
            with self.subTest(changes=changes, diagnostics=diagnostics):
                for name, raw in originals.items():
                    (self.root / name).write_bytes(raw)
                self.rewrite_applied_native(changes, diagnostics)
                code, result = self.invoke('finish', 'applied', 'changed-final-' + str(index), review='affected.json')
                self.assertEqual(code, 2, result)
                self.assertEqual(result['status'], 'BLOCKED')
                self.assertIn('REVIEWED_NATIVE_RESULT_CHANGED', result['reason'])
                self.assertEqual(result['nativeCallsAttempted'], 0)

    def test_root_uncertainty_replacement_rejects_wrong_missing_index_and_nonstring(self):
        # Use a fresh literal exercise to address both the operation and one root
        # string, preserving the unrelated runtime uncertainty at index one.
        root = self.root / 'literal-root'; root.mkdir()
        fixture = D['setup'](root, 'literal')
        code, result = D['invoke'](root, 'prepare', '--input', 'request.json', '--output', 'handoff')
        self.assertEqual(code, 0, result)
        D['write'](root, 'first-review.json', D['finding'](root, fixture))
        code, result = D['invoke'](root, 'repair-context', '--input', 'handoff', '--review', 'first-review.json', '--output', 'repair')
        self.assertEqual(code, 0, result)
        raw = (root / 'repair/repair-context.json').read_bytes(); context = json.loads(raw)
        replacements = {fixture['path']: fixture['replacement'], '/uncertainties/0': fixture['additionalTargets'][0]['new']}
        original = {'schema': 'codeclew-semantic-replacements/1.0', 'binding': context['binding'], 'contextDigest': H['sha'](raw),
                    'author': context['author'], 'replacements': [{'path': t['path'], 'old': t['old'], 'new': replacements[t['path']]} for t in context['targets']]}
        launcher = D['fake_launcher'](root)
        for index, (change, reason) in enumerate([
                ({'path': '/uncertainties/1'}, 'DISALLOWED_OR_MISSING_PATCH'),
                ({'path': '/uncertainties/99'}, 'DISALLOWED_OR_MISSING_PATCH'),
                ({'new': ['not a string']}, 'UNCERTAINTY_REPLACEMENT_MUST_BE_STRING'),
                ({'old': 'stale'}, 'EXACT_OLD_VALUE_CHANGED')]):
            patch = copy.deepcopy(original); patch['replacements'][1].update(change)
            D['write'](root, 'patch.json', patch)
            code, result = D['invoke'](root, 'apply', '--input', 'repair', '--patch', 'patch.json', '--output', 'bad-' + str(index), '--launcher', launcher, '--root', root / 'docs')
            self.assertEqual(code, 2, result); self.assertIn(reason, result['reason'])
            self.assertEqual(result['nativeCallsAttempted'], 0)
        self.assertFalse((root / 'native-calls.jsonl').exists())
        request = json.loads((root / 'request.json').read_text())
        request['allowedPaths'] = ['/uncertainties/99']
        D['write'](root, 'missing-request.json', request)
        code, result = D['invoke'](root, 'prepare', '--input', 'missing-request.json', '--output', 'missing')
        self.assertEqual(code, 2); self.assertIn('MISSING_EXACT_TARGET', result['reason'])
        for path in ['/uncertainties', '/uncertainties/-', '/uncertainties/0/text']:
            with self.assertRaisesRegex(ValueError, 'DISALLOWED_PATCH_PATH'):
                H['pointer'](fixture['base'], path)

    def test_changed_newly_acquired_source_is_blocked_before_submit(self):
        self.ready(); value = self.read('reviewer-read/source.stdout')
        value['items'][0]['record']['text'] += '// changed\n'; self.write('reviewer-read/source.stdout', value)
        self.assert_blocked_before_submit('ACQUIRED_PAGE_CHANGED')

    def test_untracked_evidence_bad_manifest_overlapping_targets_and_existing_output(self):
        request = self.read('request.json'); request['allowedPaths'].append('/operations/0/summary/text')
        self.write('request.json', request); self.prepared()
        value = self.read('first-review.json'); value['issues'].append({**value['issues'][0], 'path': '/operations/0/summary/text'})
        self.write('first-review.json', value)
        code, result = self.invoke('repair-context', 'handoff', 'repair', review='first-review.json')
        self.assertEqual(code, 2); self.assertIn('OVERLAPPING_TARGETS', result['reason'])
        output = self.root / 'existing'; output.mkdir(); (output / 'keep').write_text('KEEP')
        code, result = self.invoke('prepare', 'request.json', 'existing')
        self.assertEqual(code, 2); self.assertEqual(list(output.iterdir()), [output / 'keep'])

    def test_manifest_diagnostic_rows_are_complete_and_not_an_approval(self):
        n = D['native_page'](diagnostics=[{'kind': 'DIAGNOSTIC', 'id': 'native-gap', 'record': {'code': 'OWNED_GAP', 'severity': 'WARNING'}}])
        self.write('native-before/submit.stdout', n); self.write('native-before/submit-collection/page-0001.stdout', n)
        self.write('native-before/submit-collection/items.json', n['items'])
        manifest = self.read('native-before/submit-collection/manifest.json'); manifest['identity'] = D['T']['page_identity'](n, n['proposal'])
        self.write('native-before/submit-collection/manifest.json', manifest)
        self.prepared(); context = self.read('handoff/review-context.json')
        self.assertEqual(context['native']['diagnostics'], n['items'])
        self.assertEqual(self.read('handoff/result.json')['meaningStatus'], 'NOT_REVIEWED')
        # Altered manifest cannot make incomplete rows complete.
        manifest['identity']['total'] = 2; self.write('native-before/submit-collection/manifest.json', manifest)
        code, result = self.invoke('prepare', 'request.json', 'bad-native')
        self.assertEqual(code, 2); self.assertIn('NATIVE_MANIFEST_CHANGED', result['reason'])

    def test_repair_context_delivers_the_native_step_text_utf8_budget(self):
        self.ready(); context = self.read('repair/repair-context.json')
        limit = context['nativeConstraints']['stepMeaningText']
        self.assertEqual(limit['maxUtf8Bytes'], 1024)
        self.assertEqual(limit['unit'], 'UTF8_BYTES')
        self.assertIn('children', limit['appliesTo'])
        self.assertIn('otherwise', limit['appliesTo'])

    def test_assembled_step_text_utf8_boundaries_are_checked_before_dispatch(self):
        cases = [('ascii-at', 'a' * 1024, 1024, 'children'), ('ascii-over', 'a' * 1025, 1025, 'otherwise'),
                 ('unicode-at', 'é' * 512, 1024, 'otherwise'), ('unicode-over', 'é' * 512 + 'x', 1025, 'children'),
                 ('emoji-at', '🧵' * 256, 1024, 'children'), ('emoji-over', '🧵' * 256 + 'x', 1025, 'otherwise')]
        original_root = self.root
        for name, text, actual, branch in cases:
            with self.subTest(case=name):
                self.root = original_root / name; self.root.mkdir()
                self.fixture = D['setup'](self.root, 'omission'); self.launcher = D['fake_launcher'](self.root)
                request = self.read('request.json'); request['allowedPaths'] = ['/operations/0/steps']; self.write('request.json', request)
                self.fixture['path'] = '/operations/0/steps'
                self.fixture['replacement'] = copy.deepcopy(self.fixture['base']['operations'][0]['steps'])
                self.ready(); patch = self.read('patch.json')
                claim = lambda value: {'text': value, 'evidence': ['s-main']}
                # Preserve the existing two reference occurrences while placing
                # the exact text at a recursive children/otherwise address.
                step = {'kind': 'alt', 'meaning': claim('Owned guard'), branch: [{'kind': 'note', 'meaning': claim(text)}]}
                patch['replacements'][0]['new'] = [step]; self.write('patch.json', patch)
                code, result = self.apply()
                path = '/operations/0/steps/0/' + branch + '/0/meaning/text'
                if actual > 1024:
                    self.assertEqual(code, 2, result)
                    self.assertEqual(result['status'], 'BLOCKED')
                    self.assertIn('STEP_TEXT_UTF8_LIMIT_EXCEEDED', result['reason'])
                    self.assertIn(path, result['reason']); self.assertIn('actualBytes=1025', result['reason'])
                    self.assertIn('limitBytes=1024', result['reason'])
                    self.assertEqual(result['nativeCallsAttempted'], 0)
                    self.assertFalse((self.root / 'native-calls.jsonl').exists())
                    self.assertFalse((self.root / 'repair/apply-claimed.json').exists())
                    report = self.read('applied/local-preflight.json')
                    self.assertEqual(report['issues'], [{'code': 'STEP_TEXT_UTF8_LIMIT_EXCEEDED', 'path': path,
                                                        'actualBytes': actual, 'limitBytes': 1024, 'unit': 'UTF8_BYTES'}])
                    self.assertEqual(result['localPreflight'], 'local-preflight.json')
                    self.assertEqual(self.read('applied/input.original.json')['operations'][0]['steps'], [step])
                    self.assertNotIn('duplicate', result['reason'])
                else:
                    self.assertEqual(code, 0, result)
                    self.assertEqual(result['status'], 'AWAITING_AFFECTED_REVIEW')
                    self.assertEqual(self.read('applied/local-preflight.json')['status'], 'PASS_KNOWN_STEP_TEXT_LIMIT')
                    self.assertEqual(len((self.root / 'native-calls.jsonl').read_text().splitlines()), 2)
        self.root = original_root

    def test_preflight_checks_the_full_assembled_candidate_and_all_oversized_paths(self):
        base = self.read('native-before/input.original.json')
        base['operations'][1]['steps'] = [
            {'kind': 'note', 'meaning': {'text': 'x' * 1025, 'evidence': ['s-main']}},
            {'kind': 'note', 'meaning': {'text': 'é' * 513, 'evidence': ['s-main']}}]
        self.write('native-before/input.original.json', base)
        self.ready(); self.assert_blocked_before_submit('STEP_TEXT_UTF8_LIMIT_EXCEEDED')
        issues = self.read('applied/local-preflight.json')['issues']
        self.assertEqual([issue['path'] for issue in issues], ['/operations/1/steps/0/meaning/text', '/operations/1/steps/1/meaning/text'])
        self.assertEqual([issue['actualBytes'] for issue in issues], [1025, 1026])
        self.assertEqual(self.read('applied/input.original.json')['operations'][1], base['operations'][1])


if __name__ == '__main__':
    unittest.main()
