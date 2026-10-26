#!/usr/bin/env python3
import copy
from pathlib import Path
import runpy
import unittest

INSPECT = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'skills/codeclew/scripts/reader_plan_preflight.py'))['inspect']


class ReaderPlanPreflightTests(unittest.TestCase):
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


if __name__ == '__main__':
    unittest.main()
