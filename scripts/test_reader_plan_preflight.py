#!/usr/bin/env python3
import copy
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest

HELPER = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'skills/codeclew/scripts/reader_plan_preflight.py'))
INSPECT = HELPER['inspect']


class ReaderPlanPreflightTests(unittest.TestCase):
    def scoped(self):
        question = 'Explain the fields, copied values and derived result.'
        pin = {'service': 'sample', 'sourceId': 'retained-source', 'revision': 'saved-revision',
               'file': 'src/Example.java', 'startLine': 4, 'endLine': 7, 'textDigest': 'sha256:' + 'f' * 64}
        inventory = {'schema': 'codeclew-reader-scope/1.0', 'question': question,
                     'questionDigest': HELPER['sha'](question.encode()), 'subject': 'service:sample',
                     'snapshot': 'retained-snapshot', 'work': 'initial-work', 'candidates': [
                         {'id': 'attribute', 'kind': 'field', 'declaration': 'exact:field', 'sources': [pin]},
                         {'id': 'formula', 'kind': 'mapping', 'declaration': 'exact:mapper', 'sources': [pin]}]}
        raw = json.dumps(inventory).encode()
        scope = HELPER['bind_scope'](raw, HELPER['sha'](raw))
        plan = {'schema': 'codeclew-data-model-plan/1.0', 'models': [{'id': 'record', 'fields': [
            {'id': 'value', 'readableName': {'text': 'Selected value'}}]}]}
        docs = {'narrative': {'operations': [{'explanation': [{'text': 'The result uses a sourced formula.'}]}]}}
        coverage = {'schema': 'codeclew-reader-coverage/1.0', 'scopeDigest': scope['digest'], 'dispositions': [
            {'candidate': 'attribute', 'kind': 'canonical', 'targets': [{'document': 'plan', 'pointer': '/models/0/fields/0'}]},
            {'candidate': 'formula', 'kind': 'derived', 'targets': [{'document': 'narrative', 'pointer': '/operations/0/explanation/0/text'}]}]}
        return scope, plan, coverage, docs

    def test_earlier_candidate_inventory_survives_narrow_final_plan_and_prose_targets(self):
        scope, plan, coverage, docs = self.scoped()
        result = INSPECT(plan, reader_scope=scope, coverage=coverage, documents=docs)
        self.assertEqual(result['scopeClosure'], 'TARGETS_PRESENT_MEANING_UNASSESSED')
        self.assertIn('cannot prove initial inventory completeness', result['limits'])
        self.assertEqual(result['readerScopeDigest'], scope['digest'])
        coverage['dispositions'].pop()
        plan['models'] = []
        result = INSPECT(plan, reader_scope=scope, coverage=coverage, documents=docs)
        self.assertEqual({f['code'] for f in result['findings']},
                         {'CANDIDATE_DISPOSITION_MISSING', 'CANDIDATE_TARGET_MISSING'})
        self.assertEqual(result['scopeClosure'], 'INCOMPLETE')

    def test_scope_question_inventory_and_coverage_cannot_silently_rebind(self):
        scope, plan, coverage, docs = self.scoped()
        narrowed = copy.deepcopy(scope['inventory']); narrowed['candidates'].pop()
        with self.assertRaisesRegex(ValueError, 'SCOPE_BINDING_MISMATCH'):
            HELPER['bind_scope'](json.dumps(narrowed).encode(), scope['digest'])
        narrowed = copy.deepcopy(scope['inventory']); narrowed['question'] = 'Only explain the main path.'
        raw = json.dumps(narrowed).encode()
        with self.assertRaisesRegex(ValueError, 'INVALID_READER_SCOPE'):
            HELPER['bind_scope'](raw, HELPER['sha'](raw))
        coverage['scopeDigest'] = 'another-scope'
        with self.assertRaisesRegex(ValueError, 'COVERAGE_SCOPE_MISMATCH'):
            INSPECT(plan, reader_scope=scope, coverage=coverage, documents=docs)

    def test_scope_closure_checks_exact_targets_and_explicit_dispositions_not_semantics(self):
        scope, plan, coverage, docs = self.scoped()
        row = coverage['dispositions'][1]
        row.update(kind='excluded', targets=[], reason=None)
        result = INSPECT(plan, reader_scope=scope, coverage=coverage, documents=docs)
        self.assertEqual(result['findings'][0]['code'], 'CANDIDATE_DISPOSITION_INCOMPLETE')
        row['reason'] = 'Proposed exclusion for independent relevance review.'
        result = INSPECT(plan, reader_scope=scope, coverage=coverage, documents=docs)
        self.assertEqual(result['scopeClosure'], 'TARGETS_PRESENT_MEANING_UNASSESSED')
        row.update(kind='copy', targets=[{'document': 'narrative', 'pointer': '/operations/01'}])
        result = INSPECT(plan, reader_scope=scope, coverage=coverage, documents=docs)
        self.assertEqual(result['findings'][0]['code'], 'CANDIDATE_TARGET_MISSING')
        with self.assertRaises(ValueError):
            HELPER['target_at']({'~': 'value'}, '/~')
        coverage['dispositions'].append(copy.deepcopy(coverage['dispositions'][0]))
        self.assertIn('UNKNOWN_OR_DUPLICATE_CANDIDATE_DISPOSITION',
                      {f['code'] for f in INSPECT(plan, reader_scope=scope, coverage=coverage, documents=docs)['findings']})

    def test_cli_uses_the_earlier_binding_and_reports_a_dropped_candidate(self):
        scope, plan, coverage, docs = self.scoped()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            raw = json.dumps(scope['inventory']).encode()
            digest = HELPER['sha'](raw)
            coverage['scopeDigest'] = digest
            (root / 'scope.json').write_bytes(raw)
            (root / 'plan.json').write_text(json.dumps(plan))
            (root / 'coverage.json').write_text(json.dumps(coverage))
            (root / 'narrative.json').write_text(json.dumps(docs['narrative']))
            script = Path(__file__).resolve().parents[1] / 'skills/codeclew/scripts/reader_plan_preflight.py'
            command = [sys.executable, '-I', '-S', str(script), '--plan', str(root / 'plan.json'),
                       '--reader-scope', str(root / 'scope.json'), '--reader-scope-digest', digest,
                       '--coverage', str(root / 'coverage.json'), '--document', 'narrative=' + str(root / 'narrative.json')]
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)['scopeClosure'], 'TARGETS_PRESENT_MEANING_UNASSESSED')
            coverage['dispositions'].pop()
            (root / 'coverage.json').write_text(json.dumps(coverage))
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertEqual(json.loads(result.stdout)['findings'][0]['code'], 'CANDIDATE_DISPOSITION_MISSING')

    def test_reports_missing_labels_and_each_requested_payload_owner_without_mutation(self):
        plan = {'schema': 'codeclew-data-model-plan/1.0', 'models': [
            {'fields': [{'path': 'request.value'}, {'readableName': {'text': ' '}}]}],
            'payloadViews': [{'subject': 'service:other', 'operation': 'apply'}]}
        before = copy.deepcopy(plan)
        result = INSPECT(plan, ['service:sample/apply', 'service:sample/fail'])
        self.assertEqual(result['status'], 'NEEDS_AUTHOR_REPAIR')
        self.assertEqual([f['code'] for f in result['findings']],
                         ['BUSINESS_FIELD_LABEL_MISSING'] * 2 + ['REQUESTED_PAYLOAD_OWNER_MISSING'] * 2)
        self.assertEqual(plan, before)

    def test_explicit_partial_or_unknown_payload_is_not_upgraded_to_a_wire_schema(self):
        plan = {'schema': 'codeclew-data-model-plan/1.0',
                'models': [{'fields': [{'readableName': {'text': 'Measured value'}}]}],
                'payloadViews': [{'subject': 'scenario:example', 'operation': 'apply',
                                  'coverage': {'status': 'unknown'}, 'fields': []}]}
        before = copy.deepcopy(plan)
        result = INSPECT(plan, ['scenario:example/apply'])
        self.assertEqual(result['status'], 'READY_FOR_NATIVE_AND_READER_REVIEW')
        self.assertIn('does not validate evidence', result['limits'])
        self.assertEqual(plan, before)
        for owner in ['sample/apply', 'service:sample', 'service:sample/']:
            with self.assertRaises(ValueError):
                INSPECT(plan, [owner])

    def test_caller_selected_inventory_navigation_and_nested_links_are_exact(self):
        plan = {'schema': 'codeclew-data-model-plan/1.0', 'models': [
            {'id': 'request', 'fields': [{'id': 'body', 'readableName': {'text': 'Input'},
                                         'representations': [{'typeLabel': 'Record'}]}]},
            {'id': 'record', 'entity': 'entity:record', 'fields': []}]}
        result = INSPECT(plan, service='service:sample', expected_fields=['request/body', 'history/year'],
                         nested_fields=['request/body=record'])
        self.assertEqual([f['code'] for f in result['findings']], [
            'SERVICE_MODEL_NAVIGATION_MISSING', 'REQUESTED_CANONICAL_FIELD_MISSING', 'REQUESTED_NESTED_MODEL_LINK_MISSING'])
        plan['models'][0]['processLinks'] = [{'subject': 'service:sample', 'anchor': 'section-entities'}]
        plan['models'][0]['fields'][0]['representations'][0]['relatedModel'] = 'record'
        self.assertEqual(INSPECT(plan, service='service:sample', expected_fields=['request/body'],
                                 nested_fields=['request/body=record'])['findings'], [])

    def test_representation_can_link_to_operation_or_payload_without_domain_placement(self):
        plan = {'schema': 'codeclew-data-model-plan/1.0', 'models': [
            {'id': 'request', 'fields': [], 'processLinks': [
                {'subject': 'service:sample', 'anchor': 'apply'}]}]}
        before = copy.deepcopy(plan)
        self.assertEqual(INSPECT(plan, service='service:sample')['findings'], [])
        self.assertEqual(plan, before)
        plan['models'][0]['processLinks'] = []
        plan['payloadViews'] = [{'subject': 'service:sample', 'operation': 'apply',
                                 'fields': [{'model': 'request', 'field': 'value'}]}]
        self.assertEqual(INSPECT(plan, service='service:sample')['findings'], [])
        plan['payloadViews'][0]['fields'][0]['model'] = 'other'
        self.assertEqual(INSPECT(plan, service='service:sample')['findings'][0]['code'],
                         'SERVICE_MODEL_NAVIGATION_MISSING')
        plan['payloadViews'][0]['fields'][0]['model'] = 'request'
        plan['payloadViews'][0]['subject'] = 'service:other'
        self.assertTrue(INSPECT(plan, service='service:sample')['findings'])


if __name__ == '__main__':
    unittest.main()
