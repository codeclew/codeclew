#!/usr/bin/env python3
"""Synthetic native-export reconciliation controls; no model/semantic approval."""
import copy
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / 'skills/codeclew/scripts/boundary_coverage.py'
CHECK = runpy.run_path(str(SCRIPT))['reconcile']


def fixture():
    expected = {'schema': 'codeclew-boundary-expectations/1.0', 'service': 'orders',
                'snapshot': 'sha256:synthetic-capture/123',
                'sourceScope': 'Synthetic owned BoundaryExample.java and interface contract',
                'coverageGaps': ['Storage durability and SDK transport unknown'], 'boundaries': [],
                'domain': {'requiredConcepts': ['reservation'], 'representationOnly': ['reservation-request']},
                'dataflowViews': ['reservation-lineage']}
    page = {'subject': 'service:orders', 'selectedSnapshot': expected['snapshot'],
            'externalServices': {}, 'declaredRelationships': {'edges': []},
            'operations': [], 'operationExternalCalls': {},
            'entities': [{'normalized': {'entity': {'id': 'reservation'}}}],
            'relatedViews': [{'id': 'reservation-lineage'}]}
    operation = {'id': 'reserve', 'participants': [{'id': 'service-orders', 'service': 'orders'}], 'events': []}
    for identity, transport, response in [('active-version', 'http', 'explicit'),
                                           ('save-reservation', 'logical', 'explicit'),
                                           ('notify', 'kafka', 'none'), ('inference', 'logical', 'unresolved')]:
        destination = identity + '-provider'
        expected['boundaries'].append({'id': identity, 'destination': destination, 'transport': transport,
                                       'operations': [{'id': 'reserve', 'response': response}]})
        page['externalServices'][destination] = {'id': destination}
        page['declaredRelationships']['edges'].append({'id': identity, 'from': 'service:orders',
                                                       'to': 'external-service:' + destination,
                                                       'transport': {'kind': transport}})
        operation['participants'].append({'id': destination, 'externalService': destination})
        operation['events'].append({'id': identity + '-request', 'kind': 'declared-boundary',
                                    'from': 'service-orders', 'to': destination, 'interaction': identity})
        if response == 'explicit':
            operation['events'].append({'id': identity + '-response', 'kind': 'declared-boundary-return',
                                        'from': destination, 'to': 'service-orders', 'interaction': identity})
        page['operationExternalCalls'].setdefault('reserve', []).append({'interactionId': identity})
    page['operations'] = [operation]
    views = [{'subject': 'scenario:reservation-lineage', 'selectedSnapshot': expected['snapshot'],
              'operations': [{'dataflow': {
        'schema': 'codeclew-documentation-dataflow/1.0', 'view': 'reservation-lineage',
        'nodes': [{'id': 'input', 'kind': 'dto'}, {'id': 'business', 'kind': 'domain'}],
        'edges': [{'from': 'input', 'to': 'business', 'kind': 'transform'}]}}]}]
    return expected, page, views


class BoundaryCoverageTests(unittest.TestCase):
    def setUp(self):
        self.expected, self.page, self.views = fixture()

    def codes(self):
        return {row['code'] for row in CHECK(self.expected, self.page, self.views)['issues']}

    def test_positive_owned_closure_preserves_gaps_without_semantic_promotion(self):
        result = CHECK(self.expected, self.page, self.views)
        self.assertEqual(result['status'], 'PASS_MECHANICAL_CLOSURE')
        self.assertEqual(result['semanticReview'], 'REQUIRED_SEPARATELY')
        self.assertEqual(result['coverageGaps'], self.expected['coverageGaps'])

    def test_deleting_identity_registry_and_arrows_together_still_fails_frozen_expectation(self):
        self.page['externalServices'].clear()
        self.page['declaredRelationships']['edges'].clear()
        self.page['operations'][0]['participants'] = self.page['operations'][0]['participants'][:1]
        self.page['operations'][0]['events'].clear()
        self.assertTrue({'MISSING_EXTERNAL_IDENTITY', 'MISSING_OR_MISMATCHED_SERVICE_EGRESS',
                         'MISSING_BOUNDARY_PARTICIPANT', 'REACHABLE_BOUNDARY_WITHOUT_CALL'} <= self.codes())

    def test_self_call_and_missing_response_are_not_boundary_coverage(self):
        events = self.page['operations'][0]['events']
        events[0]['to'] = 'service-orders'
        del events[1]
        self.assertTrue({'WRONG_CALL_ENDPOINTS', 'MISSING_EXPLICIT_RETURN'} <= self.codes())

    def test_return_must_reverse_an_earlier_request_of_same_interaction(self):
        events = self.page['operations'][0]['events']
        events[1]['interaction'] = 'save-reservation'
        self.assertIn('RETURN_WITHOUT_PRIOR_REVERSED_REQUEST', self.codes())

    def test_kafka_business_reply_is_rejected_even_with_valid_endpoints(self):
        self.page['operations'][0]['events'].append({'id': 'fake-consumer-result',
            'kind': 'declared-boundary-return', 'interaction': 'notify',
            'from': 'notify-provider', 'to': 'service-orders'})
        self.assertIn('UNSUPPORTED_SYNCHRONOUS_RETURN', self.codes())

    def test_other_service_triggers_remain_in_egress_inventory(self):
        self.expected['boundaries'].append({'id': 'other-trigger', 'destination': 'other-provider',
                                          'transport': 'logical', 'operations': []})
        self.assertIn('MISSING_OR_MISMATCHED_SERVICE_EGRESS', self.codes())

    def test_reviewed_transport_representation_cannot_substitute_for_domain_concept(self):
        self.page['entities'] = [{'normalized': {'entity': {'id': 'reservation-request'}}}]
        self.assertTrue({'MISSING_REVIEWED_DOMAIN_CONCEPT', 'REPRESENTATION_ACCEPTED_AS_DOMAIN'} <= self.codes())

    def test_renamed_dependency_map_does_not_satisfy_native_dataflow_shape(self):
        self.views[0]['operations'][0] = {'title': 'DataFlow', 'dependencyGraph': {'nodes': [1], 'edges': [2]}}
        self.assertIn('MISSING_TYPED_DATAFLOW_GRAPH', self.codes())

    def test_foreign_native_view_identity_cannot_be_relabelled(self):
        self.views[0]['operations'][0]['dataflow']['view'] = 'other-view'
        self.assertIn('MISSING_TYPED_DATAFLOW_GRAPH', self.codes())

    def test_empty_or_duplicate_inventory_and_foreign_service_are_blocked(self):
        for mutation in ('empty', 'duplicate', 'foreign'):
            expected, page, views = fixture()
            if mutation == 'empty': expected['boundaries'] = []
            elif mutation == 'duplicate': expected['boundaries'].append(copy.deepcopy(expected['boundaries'][0]))
            else: page['subject'] = 'service:foreign'
            with self.assertRaises(ValueError): CHECK(expected, page, views)

    def test_same_service_from_another_capture_is_blocked(self):
        self.page['selectedSnapshot'] = 'sha256:other-capture/124'
        with self.assertRaisesRegex(ValueError, 'SNAPSHOT'):
            CHECK(self.expected, self.page, self.views)

    def test_foreign_or_missing_dataflow_snapshot_is_blocked(self):
        for snapshot in (None, 'sha256:other-capture/124'):
            views = copy.deepcopy(self.views)
            if snapshot is None:
                del views[0]['selectedSnapshot']
            else:
                views[0]['selectedSnapshot'] = snapshot
            with self.subTest(snapshot=snapshot), self.assertRaisesRegex(ValueError, 'VIEW_SNAPSHOT'):
                CHECK(self.expected, self.page, views)

    def test_duplicate_view_subject_is_blocked_in_both_orders(self):
        valid = self.views[0]
        invalid = copy.deepcopy(valid)
        invalid['operations'] = []
        for views in ([valid, valid], [invalid, valid], [valid, invalid]):
            with self.subTest(views=views), self.assertRaisesRegex(ValueError, 'DUPLICATE_VIEW_SUBJECT'):
                CHECK(self.expected, self.page, views)

    def test_extra_selected_view_must_also_be_bound_to_exact_capture(self):
        self.views.append({'subject': 'scenario:unrequested', 'selectedSnapshot': 'sha256:foreign/1'})
        with self.assertRaisesRegex(ValueError, 'VIEW_SNAPSHOT'):
            CHECK(self.expected, self.page, self.views)

    def test_normal_and_debug_run_the_same_checks_and_archive_exact_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, value in [('expected', self.expected), ('page', self.page), ('view', self.views[0])]:
                (root / (name + '.json')).write_text(json.dumps(value))
            for mode in ('normal', 'debug'):
                command = [sys.executable, '-I', '-S', str(SCRIPT), '--expectations', str(root/'expected.json'),
                           '--page', str(root/'page.json'), '--view', str(root/'view.json'), '--output', str(root/mode)]
                if mode == 'debug': command.append('--debug')
                process = subprocess.run(command, capture_output=True)
                self.assertEqual(process.returncode, 0, process.stdout)
                self.assertEqual((root/mode/'input-1.json').read_bytes(), (root/'page.json').read_bytes())
            self.assertEqual((root/'normal/result.json').read_bytes(), (root/'debug/result.json').read_bytes())
            self.assertFalse((root/'normal/debug.json').exists())
            self.assertTrue((root/'debug/debug.json').is_file())


if __name__ == '__main__':
    unittest.main()
