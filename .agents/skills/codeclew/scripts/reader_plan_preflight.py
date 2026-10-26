#!/usr/bin/env python3
"""Check requested reader affordances without authoring or semantic approval."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


def inspect(plan, payload_owners=(), service=None, expected_fields=(), nested_fields=()):
    if not isinstance(plan, dict) or plan.get('schema') != 'codeclew-data-model-plan/1.0':
        raise ValueError('Expected a data-model-plan/1.0 object')
    findings = []
    models = {model.get('id'): model for model in plan.get('models', [])}
    if service is not None and (not service.startswith('service:') or not service[8:]):
        raise ValueError('Service must be service:ID')
    for mi, model in enumerate(plan.get('models', [])):
        if service and not model.get('entity') and not any(
                link.get('subject') == service and link.get('anchor') == 'section-entities'
                for link in model.get('processLinks', [])):
            findings.append({'code': 'SERVICE_MODEL_NAVIGATION_MISSING', 'path': f'/models/{mi}/processLinks',
                             'action': 'Bind the requested service entities section explicitly; an entity association instead needs its exact retained service relation checked by native export and rendered review.'})
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
    return {'schema': 'codeclew-reader-plan-preflight/1.0',
            'status': 'NEEDS_AUTHOR_REPAIR' if findings else 'READY_FOR_NATIVE_AND_READER_REVIEW',
            'findings': findings,
            'limits': 'Checks label and caller-selected navigation/inventory presence only; does not validate evidence, language, business meaning, entity-service relations, exhaustive field completeness or runtime behavior. Native validation and independent rendered review remain required.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', required=True, type=Path)
    parser.add_argument('--payload-owner', action='append', default=[],
                        help='Exact requested subject/operation; repeat for every in-scope boundary')
    parser.add_argument('--service', help='Requested service landing; unbound models need explicit service section links')
    parser.add_argument('--expected-field', action='append', default=[], help='Exact caller-selected canonical MODEL/FIELD; repeat for the requested inventory')
    parser.add_argument('--nested-field', action='append', default=[], help='Evidence-supported MODEL/FIELD=TARGET_MODEL navigation requirement')
    args = parser.parse_args()
    try:
        result = inspect(json.loads(args.plan.read_text(encoding='utf-8')), args.payload_owner,
                         args.service, args.expected_field, args.nested_field)
    except (OSError, ValueError, TypeError, AttributeError) as error:
        parser.exit(2, f'{error}\n')
    print(json.dumps(result, ensure_ascii=False))
    return 2 if result['findings'] else 0


if __name__ == '__main__':
    raise SystemExit(main())
