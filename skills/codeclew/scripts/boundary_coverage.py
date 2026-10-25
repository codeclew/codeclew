#!/usr/bin/env python3
"""Reconcile a separately reviewed inventory with native exports; no semantic approval."""
import argparse
import hashlib
import json
from pathlib import Path


def load(path):
    raw = path.read_bytes()
    if len(raw) > 64 * 1024 * 1024:
        raise ValueError('INPUT_BYTE_LIMIT')
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('DUPLICATE_JSON_KEY: ' + key)
            result[key] = value
        return result
    return raw, json.loads(raw, object_pairs_hook=unique,
                           parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))


def reconcile(expected, page, views=()):
    if not isinstance(expected, dict) or not isinstance(page, dict):
        raise ValueError('EXPECTED_JSON_OBJECTS')
    if expected.get('schema') != 'codeclew-boundary-expectations/1.0':
        raise ValueError('EXPECTED_REVIEWED_BOUNDARY_EXPECTATIONS')
    service = expected['service']
    if not expected.get('sourceScope') or not isinstance(expected.get('coverageGaps'), list):
        raise ValueError('SOURCE_SCOPE_AND_EXPLICIT_GAPS_REQUIRED')
    if page.get('subject') != 'service:' + service:
        raise ValueError('FOREIGN_SERVICE_PAGE')
    if not expected.get('snapshot') or page.get('selectedSnapshot') != expected['snapshot']:
        raise ValueError('FOREIGN_OR_UNBOUND_SNAPSHOT')
    issues = []
    def check(condition, code, identity):
        if not condition:
            issues.append({'code': code, 'id': identity})
    def indexed(rows, name):
        result = {}
        for row in rows:
            identity = row['id']
            if identity in result:
                raise ValueError('DUPLICATE_' + name + ': ' + identity)
            result[identity] = row
        return result
    boundaries = indexed(expected['boundaries'], 'EXPECTED_BOUNDARY')
    if not boundaries:
        raise ValueError('EMPTY_INVENTORY_CANNOT_ESTABLISH_COMPLETENESS')
    graph = page.get('declaredRelationships') or {}
    edges = indexed(graph.get('edges', []), 'DECLARED_EDGE')
    operations = indexed(page.get('operations', []), 'OPERATION')
    for identity, requirement in boundaries.items():
        destination, transport = requirement['destination'], requirement['transport']
        if not destination or transport not in ('http', 'kafka', 'logical'):
            raise ValueError('EXPLICIT_LOGICAL_DESTINATION_AND_TRANSPORT_REQUIRED: ' + identity)
        check(destination in page.get('externalServices', {}), 'MISSING_EXTERNAL_IDENTITY', identity)
        edge = edges.get(identity, {})
        check(edge.get('from') == 'service:' + service
              and edge.get('to') == 'external-service:' + destination
              and edge.get('transport', {}).get('kind') == transport,
              'MISSING_OR_MISMATCHED_SERVICE_EGRESS', identity)
        for selected in requirement['operations']:
            opid, response = selected['id'], selected['response']
            if response not in ('explicit', 'unresolved', 'none'):
                raise ValueError('RESPONSE_MODE_REQUIRED: ' + identity)
            operation = operations.get(opid)
            check(operation is not None, 'MISSING_OPERATION', opid)
            if operation is None:
                continue
            actors = indexed(operation.get('participants', []), 'PARTICIPANT')
            local = {key for key, actor in actors.items() if actor.get('service') == service}
            remote = {key for key, actor in actors.items() if actor.get('externalService') == destination}
            check(bool(remote), 'MISSING_BOUNDARY_PARTICIPANT', identity + ':' + opid)
            calls, returns = [], []
            for index, event in enumerate(operation.get('events', [])):
                if event.get('interaction') != identity:
                    continue
                if event.get('kind') == 'declared-boundary':
                    check(event.get('from') in local and event.get('to') in remote,
                          'WRONG_CALL_ENDPOINTS', event['id'])
                    if event.get('from') in local and event.get('to') in remote:
                        calls.append((index, event))
                elif event.get('kind') == 'declared-boundary-return':
                    returns.append((index, event))
            check(bool(calls), 'REACHABLE_BOUNDARY_WITHOUT_CALL', identity + ':' + opid)
            if response == 'explicit':
                check(bool(returns), 'MISSING_EXPLICIT_RETURN', identity + ':' + opid)
            if response == 'none' or transport == 'kafka':
                check(not returns, 'UNSUPPORTED_SYNCHRONOUS_RETURN', identity + ':' + opid)
            for index, event in returns:
                check(any(before < index and call.get('from') == event.get('to')
                          and call.get('to') == event.get('from') for before, call in calls),
                      'RETURN_WITHOUT_PRIOR_REVERSED_REQUEST', event['id'])
            projected = page.get('operationExternalCalls', {}).get(opid, [])
            check(any(row.get('interactionId') == identity for row in projected),
                  'MISSING_OPERATION_EXTERNAL_PROJECTION', identity + ':' + opid)
    domain = expected.get('domain', {})
    entities = {row.get('normalized', {}).get('entity', {}).get('id') for row in page.get('entities', [])}
    for identity in domain.get('requiredConcepts', []):
        check(identity in entities, 'MISSING_REVIEWED_DOMAIN_CONCEPT', identity)
    for identity in domain.get('representationOnly', []):
        check(identity not in entities, 'REPRESENTATION_ACCEPTED_AS_DOMAIN', identity)
    selected_views = {}
    for row in views:
        subject = row.get('subject')
        if not isinstance(subject, str) or not subject.startswith('scenario:') or len(subject) <= 9:
            raise ValueError('INVALID_VIEW_SUBJECT')
        if subject in selected_views:
            raise ValueError('DUPLICATE_VIEW_SUBJECT: ' + subject)
        selected_views[subject] = row
    for subject, selected in selected_views.items():
        if selected.get('selectedSnapshot') != expected['snapshot']:
            raise ValueError('FOREIGN_OR_UNBOUND_VIEW_SNAPSHOT: ' + subject)
    for identity in expected.get('dataflowViews', []):
        check(any(row.get('id') == identity for row in page.get('relatedViews', [])),
              'DATAFLOW_NOT_LINKED_FROM_SERVICE', identity)
        selected = selected_views.get('scenario:' + identity, {})
        check(any(isinstance(op.get('dataflow'), dict)
                  and op['dataflow'].get('schema') == 'codeclew-documentation-dataflow/1.0'
                  and op['dataflow'].get('view') == identity and op['dataflow'].get('nodes')
                  and op['dataflow'].get('edges') for op in selected.get('operations', [])),
              'MISSING_TYPED_DATAFLOW_GRAPH', identity)
    return {'schema': 'codeclew-boundary-coverage-result/1.0',
            'status': 'FAIL_MECHANICAL_CLOSURE' if issues else 'PASS_MECHANICAL_CLOSURE',
            'service': service, 'issues': issues, 'coverageGaps': expected['coverageGaps'],
            'semanticReview': 'REQUIRED_SEPARATELY',
            'limitations': ['Inventory must be independently reviewed against full selected source, not derived only from the candidate.',
                            'Endpoint/ID consistency does not verify guards, source reachability, multiplicity, awaitedness, remote response attribution, durability or domain meaning.',
                            'Unresolved responses need precise visible prose and source gaps checked by an independent reader.']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--expectations', type=Path, required=True)
    parser.add_argument('--page', type=Path, required=True)
    parser.add_argument('--view', type=Path, action='append', default=[])
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--debug', action='store_true')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    try:
        inputs = [load(path) for path in [args.expectations, args.page, *args.view]]
        result = reconcile(inputs[0][1], inputs[1][1], [value for _, value in inputs[2:]])
        result['inputDigests'] = ['sha256:' + hashlib.sha256(raw).hexdigest() for raw, _ in inputs]
        for index, (raw, _) in enumerate(inputs):
            (args.output / f'input-{index}.json').write_bytes(raw)
        if args.debug:
            (args.output / 'debug.json').write_text(json.dumps({
                'mode': 'OBSERVABLE_INPUTS_AND_CHECKS_ONLY', 'decisionsChanged': False,
                'criteria': 'Frozen reviewed inventory → registry, service egress, actors, arrows, pairing, reviewed domain IDs and native typed DataFlow',
                'semanticAuthority': 'NONE'}, indent=2) + '\n')
    except (OSError, ValueError, TypeError, KeyError, AttributeError) as error:
        result = {'status': 'BLOCKED', 'reason': str(error), 'semanticReview': 'NOT_PERFORMED'}
    (args.output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'status': result['status'], 'issues': len(result.get('issues', [])),
                      'reason': result.get('reason')}))
    return 0 if result['status'] == 'PASS_MECHANICAL_CLOSURE' else 2


if __name__ == '__main__':
    raise SystemExit(main())
