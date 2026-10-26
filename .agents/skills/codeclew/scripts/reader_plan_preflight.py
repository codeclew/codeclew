#!/usr/bin/env python3
"""Check requested reader affordances without authoring or semantic approval."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def sha(raw):
    return 'sha256:' + hashlib.sha256(raw).hexdigest()


def bind_scope(raw, expected_digest):
    """Bind an earlier bounded inventory; never derive one from the final plan."""
    if len(raw) > 256 * 1024 or not expected_digest or sha(raw) != expected_digest:
        raise ValueError('READER_SCOPE_BINDING_MISMATCH')
    value = json.loads(raw)
    question = value.get('question')
    candidates = value.get('candidates')
    if (value.get('schema') != 'codeclew-reader-scope/1.0' or not isinstance(question, str)
            or not question.strip() or len(question.encode('utf-8')) > 32768
            or value.get('questionDigest') != sha(question.encode('utf-8'))
            or not all(isinstance(value.get(k), str) and value[k].strip() for k in ('subject', 'snapshot', 'work'))
            or not isinstance(candidates, list) or not 1 <= len(candidates) <= 256):
        raise ValueError('INVALID_READER_SCOPE')
    ids = set()
    for candidate in candidates:
        if (not isinstance(candidate, dict)
                or not all(isinstance(candidate.get(k), str) and 0 < len(candidate[k].strip()) <= 1024
                           for k in ('id', 'kind', 'declaration'))
                or candidate['id'] in ids or not isinstance(candidate.get('sources'), list)
                or not 1 <= len(candidate['sources']) <= 8):
            raise ValueError('INVALID_READER_SCOPE_CANDIDATE')
        ids.add(candidate['id'])
        for source in candidate['sources']:
            if (not isinstance(source, dict)
                    or not all(isinstance(source.get(k), str) and source[k].strip()
                               for k in ('service', 'sourceId', 'revision', 'file', 'textDigest'))
                    or type(source.get('startLine')) is not int or type(source.get('endLine')) is not int
                    or not 1 <= source['startLine'] <= source['endLine']
                    or not source['textDigest'].startswith('sha256:') or len(source['textDigest']) != 71):
                raise ValueError('INVALID_READER_SCOPE_SOURCE_PIN')
    return {'digest': expected_digest, 'inventory': value}


def target_at(document, pointer):
    if not isinstance(pointer, str) or not pointer.startswith('/') or len(pointer) > 2048:
        raise ValueError('Scope target needs an exact non-root JSON pointer')
    for raw in pointer[1:].split('/'):
        # Decode RFC6901 without accepting malformed escapes or array guesses.
        if any(not part or part[0] not in '01' for part in raw.split('~')[1:]):
            raise ValueError('Invalid scope target pointer escape')
        key = raw.replace('~1', '/').replace('~0', '~')
        if isinstance(document, list):
            if not key.isascii() or not key.isdigit() or str(int(key)) != key:
                raise ValueError('Invalid scope target array index')
            document = document[int(key)]
        else:
            document = document[key]
    return document


def scope_findings(scope, coverage, documents):
    if not isinstance(coverage, dict) or coverage.get('schema') != 'codeclew-reader-coverage/1.0':
        raise ValueError('Scope check requires reader-coverage/1.0 dispositions')
    if coverage.get('scopeDigest') != scope['digest']:
        raise ValueError('READER_COVERAGE_SCOPE_MISMATCH')
    rows = coverage.get('dispositions')
    if not isinstance(rows, list) or len(rows) > 256:
        raise ValueError('INVALID_READER_COVERAGE')
    candidates = {row['id'] for row in scope['inventory']['candidates']}
    seen, findings = set(), []
    for row in rows:
        if not isinstance(row, dict) or not isinstance(row.get('candidate'), str):
            raise ValueError('INVALID_READER_DISPOSITION')
        candidate = row['candidate']
        if candidate not in candidates or candidate in seen:
            findings.append({'code': 'UNKNOWN_OR_DUPLICATE_CANDIDATE_DISPOSITION', 'candidate': candidate})
            continue
        seen.add(candidate)
        kind, targets = row.get('kind'), row.get('targets')
        if (kind not in ('canonical', 'copy', 'history', 'derived', 'unresolved', 'excluded')
                or not isinstance(targets, list) or len(targets) > 16
                or (kind != 'excluded' and not targets)
                or (kind in ('unresolved', 'excluded') and
                    (not isinstance(row.get('reason'), str) or not row['reason'].strip()))):
            findings.append({'code': 'CANDIDATE_DISPOSITION_INCOMPLETE', 'candidate': candidate})
            continue
        for target in targets:
            try:
                value = target_at(documents[target['document']], target['pointer'])
                if value is None or value in ('', [], {}):
                    raise ValueError('Empty target')
            except (KeyError, IndexError, TypeError, ValueError):
                findings.append({'code': 'CANDIDATE_TARGET_MISSING', 'candidate': candidate, 'target': target})
    findings.extend({'code': 'CANDIDATE_DISPOSITION_MISSING', 'candidate': candidate}
                    for candidate in sorted(candidates - seen))
    return findings


def inspect(plan, payload_owners=(), service=None, expected_fields=(), nested_fields=(),
            reader_scope=None, coverage=None, documents=None):
    if not isinstance(plan, dict) or plan.get('schema') != 'codeclew-data-model-plan/1.0':
        raise ValueError('Expected a data-model-plan/1.0 object')
    findings = []
    if reader_scope is not None:
        findings.extend(scope_findings(reader_scope, coverage, {**(documents or {}), 'plan': plan}))
    models = {model.get('id'): model for model in plan.get('models', [])}
    if service is not None and (not service.startswith('service:') or not service[8:]):
        raise ValueError('Service must be service:ID')
    for mi, model in enumerate(plan.get('models', [])):
        linked = any(link.get('subject') == service and link.get('anchor')
                     for link in model.get('processLinks', []))
        payload_linked = any(view.get('subject') == service and view.get('operation') and any(
            reference.get('model') == model.get('id')
            for reference in view.get('fields', [])) for view in plan.get('payloadViews', []))
        if service and not model.get('entity') and not linked and not payload_linked:
            findings.append({'code': 'SERVICE_MODEL_NAVIGATION_MISSING', 'path': f'/models/{mi}/processLinks',
                             'action': 'Bind the representation to its relevant service operation/process link or payload owner. A domain dictionary association instead needs its exact retained entity-service relation checked by native export and rendered review; do not place every DTO in the domain dictionary.'})
        for fi, field in enumerate(model.get('fields', [])):
            name = field.get('readableName')
            if not isinstance(name, dict) or not isinstance(name.get('text'), str) or not name['text'].strip():
                findings.append({'code': 'BUSINESS_FIELD_LABEL_MISSING',
                                 'path': f'/models/{mi}/fields/{fi}/readableName',
                                 'action': 'Author an evidence-bound readableName in the requested language; keep exact code paths in representations and originalName.'})
    views = plan.get('payloadViews', [])
    def selected_field(reference):
        model_id, sep, field_id = reference.rpartition('/')
        if not sep or not model_id or not field_id:
            raise ValueError('Field reference must be MODEL/FIELD')
        return next((field for field in models.get(model_id, {}).get('fields', []) if field.get('id') == field_id), None)

    for reference in expected_fields:
        if selected_field(reference) is None:
            findings.append({'code': 'REQUESTED_CANONICAL_FIELD_MISSING', 'field': reference,
                             'action': 'Account for the caller-selected attribute inventory, including copied, historical, derived and unresolved fields; do not infer aliases by spelling.'})
    for reference in nested_fields:
        field_ref, sep, target = reference.partition('=')
        if not sep or not target:
            raise ValueError('Nested field must be MODEL/FIELD=TARGET_MODEL')
        field = selected_field(field_ref)
        if target not in models or field is None or not any(
                representation.get('relatedModel') == target for representation in field.get('representations', [])):
            findings.append({'code': 'REQUESTED_NESTED_MODEL_LINK_MISSING', 'field': field_ref, 'target': target,
                             'action': 'Bind the exact evidence-supported representation.relatedModel to the existing definition; matching type names alone are insufficient.'})
    for owner in payload_owners:
        subject, sep, operation = owner.rpartition('/')
        if not sep or not subject.startswith(('service:', 'scenario:')) or not operation:
            raise ValueError('Payload owner must be service:ID/OPERATION or scenario:ID/OPERATION')
        if not any(view.get('subject') == subject and view.get('operation') == operation for view in views):
            findings.append({'code': 'REQUESTED_PAYLOAD_OWNER_MISSING', 'owner': owner,
                             'action': 'Author source-supported payloadViews using exact canonical representations. Use unknown coverage only when the selected definition is unavailable; never synthesize a wire schema.'})
    result = {'schema': 'codeclew-reader-plan-preflight/1.0',
            'status': 'NEEDS_AUTHOR_REPAIR' if findings else 'READY_FOR_NATIVE_AND_READER_REVIEW',
            'findings': findings,
            'limits': 'Checks label and caller-selected navigation/inventory presence only; does not validate evidence, language, business meaning, entity-service relations, exhaustive field completeness or runtime behavior. Native validation and independent rendered review remain required.'}
    if reader_scope is not None:
        result['readerScopeDigest'] = reader_scope['digest']
        result['questionDigest'] = reader_scope['inventory']['questionDigest']
        result['scopeClosure'] = 'INCOMPLETE' if findings else 'TARGETS_PRESENT_MEANING_UNASSESSED'
        result['limits'] += ' Scope closure checks the earlier pinned candidate dispositions and exact targets only; it cannot prove initial inventory completeness, mapping correctness or justified exclusions.'
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', required=True, type=Path)
    parser.add_argument('--payload-owner', action='append', default=[],
                        help='Exact requested subject/operation; repeat for every in-scope boundary')
    parser.add_argument('--service', help='Requested service; representation models need relevant operation/process or payload navigation')
    parser.add_argument('--expected-field', action='append', default=[], help='Exact caller-selected canonical MODEL/FIELD; repeat for the requested inventory')
    parser.add_argument('--nested-field', action='append', default=[], help='Evidence-supported MODEL/FIELD=TARGET_MODEL navigation requirement')
    parser.add_argument('--reader-scope', type=Path, help='Bounded original question/candidate inventory saved before authoring')
    parser.add_argument('--reader-scope-digest', help='Exact earlier scope file digest retained in the dispatch/handoff')
    parser.add_argument('--coverage', type=Path, help='Candidate dispositions/targets bound to that scope digest')
    parser.add_argument('--document', action='append', default=[], help='NAME=JSON_FILE for exact prose targets; plan is already available')
    args = parser.parse_args()
    try:
        if bool(args.reader_scope) != bool(args.reader_scope_digest) or bool(args.reader_scope) != bool(args.coverage):
            raise ValueError('Use --reader-scope, --reader-scope-digest and --coverage together')
        documents = {}
        for entry in args.document:
            name, sep, path = entry.partition('=')
            if not sep or not name or name == 'plan' or name in documents or len(documents) >= 8:
                raise ValueError('Document must be a unique NAME=JSON_FILE; plan is reserved')
            raw = Path(path).read_bytes()
            if len(raw) > 4 * 1024 * 1024:
                raise ValueError('Reader document exceeds4 MiB')
            documents[name] = json.loads(raw)
        scope = bind_scope(args.reader_scope.read_bytes(), args.reader_scope_digest) if args.reader_scope else None
        coverage = json.loads(args.coverage.read_text(encoding='utf-8')) if args.coverage else None
        result = inspect(json.loads(args.plan.read_text(encoding='utf-8')), args.payload_owner,
                         args.service, args.expected_field, args.nested_field, scope, coverage, documents)
    except (OSError, ValueError, TypeError, AttributeError) as error:
        parser.exit(2, f'{error}\n')
    print(json.dumps(result, ensure_ascii=False))
    return 2 if result['findings'] else 0


if __name__ == '__main__':
    raise SystemExit(main())
