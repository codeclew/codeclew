"""Build and apply one source-bounded reviewer correction per isolated slot."""
from copy import deepcopy
import json


SCHEMA = 'codeclew-docs-page-review-findings/1.0'
CONTRACT = 2
MAX_FINDINGS_BYTES = 32 * 1024
MAX_FINDINGS = 16
FIELDS = {'title': 'title', 'claim': 'claims', 'step': 'steps', 'routeStep': 'steps', 'startStep': 'steps',
          'routeOutcome': 'outcome'}
NESTED_IDS = {'routeStep': 'routeId', 'startStep': 'startId', 'routeOutcome': 'routeId'}


def findings(value):
    need(isinstance(value, dict) and value.keys() == {'schema', 'findings'} and value['schema'] == SCHEMA
         and isinstance(value['findings'], list) and 0 < len(value['findings']) <= MAX_FINDINGS,
         'PAGE_REVIEW_FINDINGS_INVALID')
    rows = {}
    for row in value['findings']:
        need(isinstance(row, dict) and row.keys() == {'slot', 'responseDigest', 'selector', 'issue', 'sourceAliases'},
             'PAGE_REVIEW_FINDING_INVALID')
        slot, issue, selector, aliases = row['slot'], row['issue'], row['selector'], row['sourceAliases']
        need(isinstance(slot, str) and slot and slot not in rows and isinstance(row['responseDigest'], str)
             and isinstance(issue, str) and 0 < len(issue.strip()) <= 600 and len(issue) <= 600
             and all(ord(character) >= 32 for character in issue)
             and isinstance(aliases, list) and 0 < len(aliases) <= 8 and all(isinstance(a, str) for a in aliases)
             and len(set(aliases)) == len(aliases), 'PAGE_REVIEW_FINDING_INVALID')
        need(isinstance(selector, dict) and isinstance(selector.get('field'), str), 'PAGE_REVIEW_SELECTOR_INVALID')
        field = selector['field']
        identity = NESTED_IDS.get(field)
        indexed = field not in ('title', 'routeOutcome')
        selector_keys = {'field', 'index', 'action'} if indexed else {'field'}
        if identity:
            selector_keys.add(identity)
        need(field in FIELDS and selector.keys() == selector_keys
             and (not indexed or type(selector['index']) is int and selector['index'] >= 0
                  and selector['action'] in ('replace', 'insert'))
             and (identity is None or isinstance(selector[identity], str) and 0 < len(selector[identity]) <= 64),
             'PAGE_REVIEW_SELECTOR_INVALID')
        rows[slot] = row
    return rows


def selected_value(plan, slot, response, selector):
    kind = selector['field']
    field = FIELDS[kind]
    if kind in ('routeStep', 'routeOutcome'):
        need(slot['kind'] == 'PROCESS_MAP', 'PAGE_REVIEW_SELECTOR_OUTSIDE_SLOT', slot=slot['key'])
        trace = next((route for route in response['routes'] if route_id(route) == selector['routeId']), None)
        parent = 'route'
    elif kind == 'startStep':
        need(slot['kind'] == 'DISCOVERY', 'PAGE_REVIEW_SELECTOR_OUTSIDE_SLOT', slot=slot['key'])
        refs = discovery_slots(plan, slot['service'])
        file_ref = next(ref for ref, owner in refs.items() if owner['key'] == slot['key'])
        trace = next((start for number, start in enumerate(response['starts'], 1)
                      if discovered_entry_id(file_ref, number) == selector['startId']), None)
        parent = 'start'
    else:
        return field, response, None
    need(trace is not None, 'PAGE_REVIEW_TRACE_OUTSIDE_SLOT', slot=slot['key'])
    return field, trace, parent


def format_failure(failure):
    return (failure is not None and failure['code'] == 'PAGE_RESPONSE_INVALID'
            and failure['details'].get('formatOnly') is True)


def format_prompt(slot, failure):
    issues = [{key: issue[key] for key in ('path', 'code', 'message', 'maxCharacters') if key in issue}
              for issue in failure['details']['issues']]
    return encode({'slot': slot['key'], 'reviewFormatRepair': issues,
                   'instruction': 'Correct the format of the replacement using the same selection, retained excerpts '
                                  'and response schema from this session. Keep all material conditions and source '
                                  'bindings. For length errors, rephrase concisely without dropping conditions. '
                                  'Return null if the correction cannot be expressed accurately in the required format.'})


def replacement_contract(field):
    heading = f'One line, at most {response_contract["HEADING_LENGTH"]} characters.'
    step = f'One nonempty line, at most {response_contract["STEP_TEXT_LENGTH"]} characters.'
    if field == 'steps':
        return {'label': step, 'condition': f'{step} Use null when there is no condition.'}
    if field == 'claims':
        return {'heading': heading + ' Use an empty string when there is no heading.',
                'text': f'Nonempty reader Markdown, at most {response_contract["CLAIM_TEXT_LENGTH"]} characters. '
                        'Keep sequence steps in their trace.'}
    if field == 'outcome':
        return {'outcome': f'One nonempty line, at most {response_contract["ROUTE_PROSE_LENGTH"]} characters.'}
    return {'title': heading + ' Supply a nonempty title.'}


def author_input(run, plan, slot, proof, response, row, *, contract):
    key, selector, aliases = slot['key'], row['selector'], row['sourceAliases']
    need(proof['responseDigest'] == row['responseDigest'], 'PAGE_REVIEW_RESPONSE_MISMATCH', slot=key)
    field, target, parent = selected_value(plan, slot, response, selector)
    schema = load(run / 'inputs' / slot['schema'])
    properties = (schema if parent is None else schema['$defs'][parent])['properties']
    need(field in properties, 'PAGE_REVIEW_SELECTOR_OUTSIDE_SLOT', slot=key)
    property_schema = properties[field]
    current, insert = target[field], selector.get('action') == 'insert'
    if 'index' in selector:
        index = selector['index']
        need(index < len(current) or (insert and index == len(current)), 'PAGE_REVIEW_INDEX_OUTSIDE_SLOT', slot=key)
        current = None if insert else current[index]
        property_schema = property_schema['items']
    packet = decode((run / 'inputs' / slot['prompt']).read_bytes().rsplit(b'\n\n', 1)[-1])
    need(all(alias in slot['citations'] and alias in packet['citations'] for alias in aliases),
         'PAGE_REVIEW_SOURCE_OUTSIDE_SLOT', slot=key)
    if field == 'outcome':
        need(set(aliases) <= set(target['cites']), 'PAGE_REVIEW_SOURCE_OUTSIDE_ROUTE', slot=key)
    citations = {alias: packet['citations'][alias] for alias in aliases}
    ranges = {region for citation in citations.values() for region in citation.get('regions', [citation['region']])}
    need(ranges <= packet['sourceRanges'].keys(), 'PAGE_REVIEW_SOURCE_OUTSIDE_SLOT', slot=key)
    context = {'slot': key, 'type': slot['kind'], 'language': packet['language'], 'selector': selector,
               'issue': row['issue'], 'current': current, 'citations': citations,
               'sourceRanges': {region: packet['sourceRanges'][region] for region in sorted(ranges)}}
    result_schema = {'type': 'object', 'properties': {'replacement': {'anyOf': [property_schema, {'type': 'null'}]}},
                     'required': ['replacement'],
                     'additionalProperties': False}
    if field == 'claims':
        claim = deepcopy(schema['$defs']['claim'])
        claim['properties']['cites']['items'] = {'type': 'string', 'enum': aliases}
        if 'returns' in claim['properties']:
            context['returns'] = {ref: binding for ref, binding in packet['returns'].items() if binding['cite'] in aliases}
        result_schema['$defs'] = {'claim': claim}
    if field == 'steps':
        step = deepcopy(schema['$defs']['step'])
        actors = (['caller', target['origin'], *(actor['entry'] for actor in target['participants'])] if parent == 'route'
                  else ['caller', 'handler', *(actor['id'] for actor in target['participants'])])
        for actor in ('from', 'to'):
            step['properties'][actor] = {'type': 'string', 'enum': actors}
        result_schema['$defs'] = {'step': step}
    if contract == CONTRACT:
        limits = replacement_contract(field)
        context['replacementContract'] = limits
        if 'index' not in selector:
            result_schema['properties']['replacement']['anyOf'][0] = {**property_schema, 'description': limits[field]}
        else:
            replacement = next(iter(result_schema['$defs'].values()))['properties']
            for name, description in limits.items():
                replacement[name]['description'] = description
    instruction = ('Correct the selected value only if the retained source supports the review finding. '
                   'The issue describes what to check. Use the source as evidence, preserve supported conditions, '
                   'and use only the listed citations and existing actors. Return the replacement in the supplied schema. '
                   'If the evidence does not support a correction, return null as the replacement.')
    prompt = (instruction + '\n\n' + json.dumps(context, ensure_ascii=False, separators=(',', ':')) + '\n').encode()
    need(len(prompt) <= 224 * 1024, 'PAGE_REVIEW_CONTEXT_LIMIT', slot=key)
    return prompt, result_schema, contract == CONTRACT


def apply(plan, slot, original, row, answer, validate):
    key = slot['key']
    need(isinstance(answer, dict) and answer.keys() == {'replacement'}, 'PAGE_REVIEW_CORRECTION_INVALID', slot=key)
    revision = answer['replacement']
    need(revision is not None, 'PAGE_REVIEW_CORRECTION_UNSUPPORTED', slot=key)
    selector = row['selector']
    corrected = deepcopy(original)
    field, target, _ = selected_value(plan, slot, corrected, selector)
    if field == 'claims':
        need(isinstance(revision, dict) and isinstance(revision.get('cites'), list)
             and all(alias in row['sourceAliases'] for alias in revision['cites']),
             'PAGE_REVIEW_CORRECTION_OUTSIDE_SOURCE', slot=key)
    if 'index' not in selector:
        target[field] = revision
    elif selector['action'] == 'insert':
        target[field].insert(selector['index'], revision)
    else:
        target[field][selector['index']] = revision
    revised = validate(slot, corrected)
    need(revised != original, 'PAGE_REVIEW_CORRECTION_UNSUPPORTED', slot=key)
    need(len(encode(revised)) <= response_limit, 'PAGE_REVIEW_CORRECTION_OVERSIZE', slot=key)
    return revised
