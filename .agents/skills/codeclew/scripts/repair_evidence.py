#!/usr/bin/env python3
"""Append exactly grounded RETURN evidence; native-check the full saved candidate."""
from __future__ import annotations
import argparse
import copy
import hashlib
import json
from pathlib import Path
import runpy
import subprocess

TRANSPORT = runpy.run_path(str(Path(__file__).with_name('collect_proposal.py')))
collect = TRANSPORT['collect']
Incomplete = TRANSPORT['Incomplete']


class Blocked(ValueError):
    pass


def load(raw):
    return json.loads(raw, object_pairs_hook=TRANSPORT['unique_object'],
                      parse_constant=TRANSPORT['reject_constant'])


def digest(raw):
    return 'sha256:' + hashlib.sha256(raw).hexdigest()


def record_digest(record):
    return digest(json.dumps(record, sort_keys=True, ensure_ascii=False,
                             separators=(',', ':')).encode())


def save(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def keys(value, required, label):
    if not isinstance(value, dict) or set(value) != set(required):
        raise Blocked('INVALID_' + label + '_FIELDS')


def target(base, edit):
    operation = edit['operation']
    if type(operation) is not int or operation < 0:
        raise Blocked('INVALID_OPERATION_INDEX')
    try:
        node = base['operations'][operation]
        if not isinstance(node, dict):
            raise Blocked('OPERATION_NOT_OBJECT')
        path = edit['stepPath']
        if not isinstance(path, list) or not 1 <= len(path) <= 16:
            raise Blocked('INVALID_STEP_PATH')
        for index, part in enumerate(path):
            keys(part, ('list', 'index'), 'STEP_PATH')
            if part['list'] not in (('steps',) if index == 0 else ('children', 'otherwise')):
                raise Blocked('UNSUPPORTED_STEP_PATH')
            if type(part['index']) is not int or part['index'] < 0:
                raise Blocked('INVALID_STEP_INDEX')
            node = node[part['list']][part['index']]
            if not isinstance(node, dict):
                raise Blocked('TARGET_STEP_NOT_OBJECT')
        return node
    except (KeyError, IndexError, TypeError) as error:
        raise Blocked('MISSING_EXACT_TARGET') from error


def validate_intent(base, raw, intent):
    keys(intent, ('schema', 'work', 'snapshot', 'baseDigest', 'targets'), 'INTENT')
    if intent['schema'] != 'codeclew-evidence-only-repair-intent/1.0':
        raise Blocked('UNSUPPORTED_INTENT_SCHEMA')
    if not isinstance(base, dict) or base.get('schema') != 'codeclew-documentation-proposal/1.0':
        raise Blocked('UNSUPPORTED_PROPOSAL_SCHEMA')
    if intent['baseDigest'] != digest(raw):
        raise Blocked('BASE_DIGEST_CHANGED')
    if not all(isinstance(intent[name], str) and intent[name] for name in ('work', 'snapshot')):
        raise Blocked('MISSING_WORK_IDENTITY')
    if not isinstance(intent['targets'], list) or not 1 <= len(intent['targets']) <= 8:
        raise Blocked('TARGET_COUNT_OUTSIDE_BOUND')
    seen = set()
    for edit in intent['targets']:
        keys(edit, ('operation', 'stepPath', 'expectedEvidence', 'expectedText', 'reference',
                    'recordId', 'recordDigest', 'mapping'), 'TARGET')
        if edit['mapping'] != 'VERBATIM_FLOW_RETURN':
            raise Blocked('NEEDS_CONTEXT_SEMANTIC_MAPPING')
        node = target(base, edit)
        pointer = json.dumps([edit['operation'], edit['stepPath']], sort_keys=True)
        if pointer in seen:
            raise Blocked('DUPLICATE_TARGET')
        seen.add(pointer)
        evidence = edit['expectedEvidence']
        if not isinstance(evidence, list) or not evidence or any(not isinstance(v, str) or not v for v in evidence):
            raise Blocked('INVALID_EXPECTED_EVIDENCE')
        if node.get('kind') != 'return' or not isinstance(node.get('meaning'), dict):
            raise Blocked('NEEDS_CONTEXT_RETURN_TARGET')
        if node['meaning'].get('text') != edit['expectedText'] or node['meaning'].get('evidence') != evidence:
            raise Blocked('EXACT_TARGET_CHANGED')
        if not all(isinstance(edit[name], str) and edit[name] for name in ('reference', 'recordId', 'recordDigest', 'expectedText')):
            raise Blocked('MISSING_EVIDENCE_IDENTITY')
        if edit['reference'] in evidence:
            raise Blocked('REFERENCE_ALREADY_IN_BASE_NO_STALE_REBASE')


def materialize(base, intent, rows):
    by_reference = {row.get('reference'): row for row in rows if row.get('reference')}
    candidate = copy.deepcopy(base)
    bindings = []
    for edit in intent['targets']:
        row = by_reference.get(edit['reference'])
        if not row or row.get('kind') != 'DEPENDENCY' or row.get('id') != edit['recordId']:
            raise Blocked('EVIDENCE_REFERENCE_IDENTITY_CHANGED')
        record = row.get('record')
        if not isinstance(record, dict) or record_digest(record) != edit['recordDigest']:
            raise Blocked('EVIDENCE_RECORD_CHANGED')
        normalized = record.get('normalized', {})
        if not isinstance(normalized, dict):
            raise Blocked('INVALID_FLOW_RECORD')
        if record.get('kind') != 'FLOW' or normalized.get('kind') != 'RETURN':
            raise Blocked('NEEDS_CONTEXT_RETURN_FLOW')
        if normalized.get('authority') not in ('SYNTAX', 'COMPILER'):
            raise Blocked('UNSUPPORTED_FLOW_AUTHORITY')
        if normalized.get('text') != edit['expectedText']:
            raise Blocked('NEEDS_CONTEXT_NONVERBATIM_MEANING')
        target(candidate, edit)['meaning']['evidence'].append(edit['reference'])
        bindings.append({'reference': edit['reference'], 'recordId': row['id'],
                         'recordDigest': edit['recordDigest'], 'authority': normalized['authority'],
                         'sourceReferences': row.get('sourceReferences', []),
                         'operation': edit['operation'], 'stepPath': edit['stepPath']})
    return candidate, bindings


def native_submit(launcher, root, work, input_path, output, label):
    return TRANSPORT['submit_and_collect'](launcher, root, work, input_path, output, label)


def repair(launcher, root, base_path, intent_path, output):
    # A fresh directory is mandatory. Existing or stale results are never
    # accepted merely because the requested reference appears in a file.
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    result = {'schema': 'codeclew-evidence-only-repair-result/1.0', 'status': 'BLOCKED',
              'modelCalls': 0, 'publication': 'NOT_PERFORMED', 'meaningReview': 'NOT_PERFORMED',
              'scope': 'VERBATIM_FLOW_RETURN_EVIDENCE_ONLY',
              'limitation': 'No semantic approval, automatic rebase, source recapture or native/host atomicity guarantee.'}
    try:
        raw = base_path.read_bytes()
        intent_raw = intent_path.read_bytes()
        (output / 'base.original.json').write_bytes(raw)
        (output / 'intent.original.json').write_bytes(intent_raw)
        base, intent = load(raw), load(intent_raw)
        validate_intent(base, raw, intent)
        references = list(dict.fromkeys(edit['reference'] for edit in intent['targets']))
        evidence = collect(launcher, root, intent['work'], output / 'evidence-collection',
                           kind='work-evidence', references=references)
        if evidence['status'] != 'COMPLETE':
            raise Blocked('INCOMPLETE_NATIVE_EVIDENCE_' + str(evidence['reason']))
        if evidence['identity']['snapshot'] != intent['snapshot']:
            raise Blocked('WORK_SNAPSHOT_CHANGED')
        if evidence['identity'].get('authority') != 'IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED':
            raise Blocked('UNSUPPORTED_WORK_AUTHORITY')
        rows = load((output / 'evidence-collection/items.json').read_bytes())
        result['contextReads'] = [{'flowReference': row['reference'],
            'selection': {'references': row.get('sourceReferences', [])},
            'when': 'Inspect existing delivered source first; use this supported Work selection only if its body remains unread.'}
            for row in rows if row.get('reference') in references and row.get('sourceReferences')]
        candidate, bindings = materialize(base, intent, rows)
        baseline = native_submit(launcher, root, intent['work'], output / 'base.original.json', output, 'baseline-submit')
        diagnostics = load((output / 'baseline-submit-collection/diagnostics.json').read_bytes())
        if baseline['nativeStatus'] != 'NEEDS_REPAIR' or not any(
                row.get('record', {}).get('code') == 'STRUCTURE_OR_COVERAGE_INVALID'
                and any('flowId=' + binding['recordId'] + ',' in row['record'].get('nextAction', '') for binding in bindings)
                for row in diagnostics):
            raise Blocked('NEEDS_CONTEXT_SUPPORTED_NATIVE_FLOW_DIAGNOSTIC')
        save(output / 'candidate.json', candidate)
        checked = native_submit(launcher, root, intent['work'], output / 'candidate.json', output, 'candidate-submit')
        result.update({'status': 'NATIVE_CANDIDATE_RECORDED', 'work': intent['work'], 'snapshot': intent['snapshot'],
                       'baseDigest': digest(raw), 'intentDigest': digest(intent_raw),
                       'candidateDigest': digest((output / 'candidate.json').read_bytes()),
                       'bindings': bindings, 'nativeStatus': checked['nativeStatus'],
                       'nativeMeaningReview': checked['meaningReview'], 'wholeCandidateChecked': True,
                       'nextAction': 'Inspect all native diagnostics and obtain independent meaning review; this helper never publishes.'})
    except (Blocked, Incomplete, OSError, ValueError, TypeError, KeyError, RecursionError) as error:
        result['reason'] = str(error)
        result['nextAction'] = 'Inspect the saved native archive. For nonverbatim or ambiguous mapping, read the exact FLOW sourceReferences and enclosing branch/receiver/downstream context through supported Work reads, then use the author/reviewer route.'
    save(output / 'result.json', result)
    return result


def anchored(root, path):
    root = root.resolve(strict=True)
    selected = (root / path).resolve()
    if not root.is_dir() or not selected.is_relative_to(root):
        raise Blocked('INPUT_PATH_OUTSIDE_ASSIGNED_ROOT')
    return selected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ('launcher', 'root', 'input-root', 'base', 'intent', 'output'):
        parser.add_argument('--' + flag, type=Path, required=True)
    args = parser.parse_args()
    try:
        result = repair(args.launcher.absolute(), args.root.absolute(), anchored(args.input_root, args.base),
                        anchored(args.input_root, args.intent), anchored(args.input_root, args.output))
    except (OSError, Blocked) as error:
        result = {'status': 'BLOCKED', 'reason': str(error), 'publication': 'NOT_PERFORMED'}
    print(json.dumps({k: v for k, v in result.items() if k != 'bindings'}, separators=(',', ':')))
    return 0 if result.get('nativeStatus', '').startswith('READY_') and result['status'] == 'NATIVE_CANDIDATE_RECORDED' else 2


if __name__ == '__main__':
    raise SystemExit(main())
