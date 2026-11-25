"""Inline delivery and assembly for a finite, fully retained documentation scope."""
from __future__ import annotations

import json
from pathlib import Path
import re

MAX_SOURCE_BYTES = 12 * 1024
MAX_PROMPT_BYTES = 20 * 1024
MAX_FILES = 4
MAX_ENTRIES = 3
MAX_CITATIONS = 24
MAX_RESPONSE_BYTES = 128 * 1024


def plan(job, inventory, prepared, instructions):
    """Select only scopes for which the author receives all retained file bytes."""
    def workspace(reason):
        return {'mode': 'WORKSPACE', 'reason': reason}
    if len(job['services']) != 1:
        return workspace('MULTIPLE_SERVICES'), None
    if 'components' in job:
        return workspace('DECLARATION_COMPONENTS'), None
    if prepared is None:
        return workspace('INLINE_SOURCE_UNAVAILABLE'), None

    service = job['services'][0]
    scope = prepared['services'][service]
    if scope['inventoryKind'] != 'SOURCE_FILES':
        return workspace('COMPILER_SCOPED_EVIDENCE'), None
    if scope['deferredResourceFiles']:
        return workspace('RETAINED_RESOURCES_ON_REQUEST'), None
    entries = [row for row in inventory['requirements'] if row['kind'] == 'ENTRY']
    discoveries = [row for row in inventory['requirements'] if row['kind'] == 'DISCOVERY']
    if len(entries) > MAX_ENTRIES or any(not entry['method'] for entry in entries):
        return workspace('ENTRY_SCOPE_REQUIRES_WORKSPACE'), None
    proofs = scope['discoveryProofs']
    if (not discoveries or len(discoveries) > MAX_FILES
            or {view['file'] for view in scope['views'].values()} != {row['file'] for row in discoveries}):
        return workspace('FILE_SCOPE_REQUIRES_WORKSPACE'), None
    source_bytes = sum(proof['bytes'] for proof in proofs.values())
    if source_bytes > MAX_SOURCE_BYTES:
        return workspace('INLINE_SOURCE_BYTE_LIMIT'), None

    home = Path(scope['sourceIndex']).parent
    ready = load(home / 'requirements.json')
    selections = [ready[row['id']]['evidenceCandidates'] for row in discoveries + entries]
    handles = list(dict.fromkeys(handle for selection in selections for handle in selection))
    prepared_facts = scope.get('preparedFacts')
    if prepared_facts:
        known_files = {row['file'] for row in discoveries}
        if any(source['file'] not in known_files for source in prepared_facts['citations'].values()):
            return workspace('PREPARED_FACTS_REQUIRE_WORKSPACE_SOURCE'), None
        handles = list(dict.fromkeys([*handles, *sorted(prepared_facts['citations'])]))
    if len(handles) > MAX_CITATIONS:
        return workspace('INLINE_CITATION_LIMIT'), None
    aliases = {'S' + str(i): handle for i, handle in enumerate(handles, 1)}
    handle_to_alias = {handle: alias for alias, handle in aliases.items()}
    bindings = load(scope['citationIndex'])
    packet = load(Path(scope['manifest']).with_name('packet.json'))
    bodies = {row['source']['id']: packet['texts'][row['textKey']] for row in packet['outcomes']}

    files = {}
    for number, requirement in enumerate(discoveries, 1):
        proof = proofs[requirement['id']]
        raw = bytearray(proof['bytes'])
        for handle in proof['sourceHandles']:
            source = bindings[handle]['source']
            occurrence = source.get('occurrence') or {}
            start, end = occurrence.get('startByte'), occurrence.get('endByte')
            if ((occurrence.get('snapshot'), occurrence.get('blob')) != (proof['occurrenceSnapshot'], proof['gitBlob'])
                    or type(start) is not int or type(end) is not int or not 0 <= start <= end <= len(raw)):
                continue
            text = bodies[source['id']].encode()
            if end - start == len(text):
                raw[start:end] = text
        if sha(raw) != proof['textDigest']:
            return workspace('INLINE_FILE_RECONSTRUCTION_UNAVAILABLE'), None
        files['F' + str(number)] = {'id': requirement['id'], 'file': requirement['file'], 'source': raw.decode(),
                                   'cites': [handle_to_alias[h] for h in ready[requirement['id']]['evidenceCandidates']]}

    entry_refs = {'E' + str(i): row for i, row in enumerate(entries, 1)}
    return_refs = {}
    check_path = home / 'literal-returns.json'
    checks = load(check_path) if check_path.exists() else []
    for i, check in enumerate(checks, 1):
        return_refs['R' + str(i)] = check
    citation_locations = {handle: {key: row['source'][key] for key in ('file', 'startLine', 'endLine')}
                          for handle, row in bindings.items()}
    if prepared_facts:
        citation_locations.update({handle: {key: row[key] for key in ('file', 'startLine', 'endLine')}
                                   for handle, row in prepared_facts['citations'].items()})
    packet = {
        'request': job['request'], 'language': job['language'], 'service': service,
        'coverage': scope['coverage']['coverage'], 'limits': scope['coverage'].get('boundaries', []),
        'files': {ref: {key: value for key, value in row.items() if key != 'id'} for ref, row in files.items()},
        'citations': {alias: citation_locations[handle] for alias, handle in aliases.items()},
        'entries': {ref: {'method': row['method'], 'path': row['path'],
                          'cites': [handle_to_alias[h] for h in ready[row['id']]['evidenceCandidates']]}
                    for ref, row in entry_refs.items()},
        'returns': {ref: {'entry': next(alias for alias, entry in entry_refs.items() if entry['page'] == row['page']),
                          'condition': row['predicate'], 'cite': handle_to_alias[row['cite']]}
                    for ref, row in return_refs.items()},
    }
    if prepared_facts:
        targets = {ref: (row['id'], packet[group][ref]) for group, refs in (('entries', entry_refs), ('files', files))
                   for ref, row in refs.items()}
        add_prepared_facts(targets, prepared_facts['entries'], handle_to_alias)
        instructions += ('\n\nUse the facts and boundaries on the corresponding entry or file, including any shared '
                         'through alsoFor. Check retained source, preserve conditions and uncertainty, '
                         'and cite the supplied aliases. Line anchors narrow the location within a citation.')
    prompt = instructions + '\n\n' + json.dumps(packet, ensure_ascii=False, separators=(',', ':')) + '\n'
    prompt_bytes = len(prompt.encode())
    if prompt_bytes > MAX_PROMPT_BYTES:
        return workspace('INLINE_PROMPT_BYTE_LIMIT'), None
    return ({
        'mode': 'INLINE', 'reason': 'COMPLETE_SMALL_RETAINED_SCOPE', 'promptBytes': prompt_bytes,
        'sourceBytes': source_bytes,
        'citations': aliases, 'entries': {ref: row['id'] for ref, row in entry_refs.items()},
        'files': {ref: row['id'] for ref, row in files.items()},
        'returns': {ref: {'id': row['id'], 'page': row['page']} for ref, row in return_refs.items()},
    }, prompt)


def add_prepared_facts(targets, prepared, aliases):
    """Add scoped source annotations to the records already read by the author."""
    seen = {}
    for ref, (identity, target) in targets.items():
        row = prepared.get(identity, {})
        if limits := row.get('factLimits'):
            target['factLimits'] = limits
        for category in ('facts', 'boundaries'):
            for fact in row.get(category, ()):
                key = (category, json.dumps(fact, ensure_ascii=False, sort_keys=True))
                if key in seen:
                    original_ref, value = seen[key]
                    if ref != original_ref and ref not in value.get('alsoFor', ()):
                        value.setdefault('alsoFor', []).append(ref)
                    continue
                value = {**fact, 'sources': [aliases[handle] for handle in fact['sources']]}
                if lines := fact.get('lines'):
                    value['lines'] = {aliases[handle]: ranges for handle, ranges in lines.items()}
                target.setdefault(category, []).append(value)
                seen[key] = ref, value


def referenced(rows, expected, field):
    result = {row[field]: row for row in rows}
    need(len(result) == len(rows) and result.keys() == expected.keys(), 'INLINE_SCOPE_ACCOUNTING_MISMATCH', field=field)
    return result


def assemble(value, selected, inventory, original, navigation):
    """Translate authored content and explicit assessments to the ordinary gates."""
    closed(value, ('status', 'reason', 'reviewed', 'overview', 'entries', 'discovery', 'unresolved', 'visual'))
    if value['status'] == 'NEEDS_WORKSPACE':
        need(isinstance(value['reason'], str) and value['reason'].strip(), 'INLINE_WORKSPACE_REASON_REQUIRED')
        raise Stop('INLINE_AUTHOR_NEEDS_WORKSPACE', reason=value['reason'][:1024])
    need(value['status'] == 'DOCUMENTED' and value['reviewed'] is True, 'INLINE_REVIEW_REQUIRED')
    need(isinstance(value['unresolved'], list) and all(isinstance(row, str) for row in value['unresolved']),
         'INVALID_INLINE_REVIEW')
    visual = value['visual']
    need(isinstance(visual, dict) and visual.get('status') == 'UNAVAILABLE'
         and isinstance(visual.get('reason'), str) and visual['reason'].strip(), 'VISUAL_STAGE_MUST_BE_RECORDED')

    requirements = {row['id']: row for row in inventory['requirements']}
    service = next(iter(inventory['services']))
    overview_page = service + '.md'
    overview = next(row for row in original['pages'] if row['page'] == overview_page)
    pages = {page['page']: page for page in original['pages']}
    nav = navigation['services'][0]
    accounts = {row['id']: {'section': 'service-scope'} for row in requirements.values()}
    assessments, return_bindings = {}, {}

    def disposition(choice):
        need(choice in ('EXPLAINED', 'EXPLICIT_UNKNOWN'), 'INLINE_DISPOSITION_REQUIRED')
        return choice

    def add_claims(page, claims):
        name = page['page']
        need(isinstance(claims, list) and claims, 'INLINE_READER_CLAIMS_REQUIRED', page=name)
        assessments[name] = {}
        result = []
        for i, claim in enumerate(claims, 1):
            closed(claim, ('heading', 'text', 'cites', 'assessment', 'returns'))
            cite_refs = claim['cites']
            need(isinstance(cite_refs, list) and cite_refs and len(set(cite_refs)) == len(cite_refs)
                 and all(ref in selected['citations'] for ref in cite_refs), 'INLINE_CITATION_UNKNOWN', page=name)
            text = claim['text']
            need(isinstance(text, str) and text.strip(), 'INLINE_READER_CLAIMS_REQUIRED', page=name)
            need(claim['assessment'] in ('SUPPORTED', 'EXPLICIT_UNKNOWN'), 'CLAIM_ASSESSMENT_REQUIRED', page=name)
            identity = 'claim-' + str(i)
            heading = claim['heading']
            need(isinstance(heading, str) and len(heading.splitlines()) <= 1, 'INVALID_INLINE_HEADING', page=name)
            if heading.strip():
                page['fragments'].append({'text': '\n## ' + heading.strip() + '\n\n'})
            handles = [selected['citations'][ref] for ref in cite_refs]
            page['fragments'].append({'id': identity, 'text': text.strip('\r\n') + '\n\n', 'evidence': handles})
            assessments[name][identity] = claim['assessment']
            result.extend(handles)
            for ref in claim['returns']:
                check = selected['returns'].get(ref)
                need(check and check['page'] == name and check['id'] not in return_bindings,
                     'INVALID_INLINE_RETURN_BINDING', page=name, check=ref)
                return_bindings[check['id']] = identity
        return list(dict.fromkeys(result))

    def add_trace(page, trace, diagram):
        participants = trace['participants']
        ids = {'caller': diagram['participants'][0]['id'], 'handler': diagram['participants'][1]['id']}
        declared = set()
        for participant in participants:
            key = participant['id']
            need(isinstance(key, str) and re.fullmatch(r'[a-z][a-z0-9-]{0,31}', key) and key not in declared,
                 'INVALID_INLINE_PARTICIPANT', page=page['page'])
            declared.add(key)
            if key in ids:
                continue  # Keep the participant and target already supplied by the entry.
            ids[key] = 'actor-' + key
            diagram['participants'].append({'id': ids[key], 'title': participant['title']})
        for step in trace['steps']:
            need(step['from'] in ids and step['to'] in ids, 'INVALID_INLINE_STEP_PARTICIPANT', page=page['page'])
            diagram['steps'].append({'from': ids[step['from']], 'to': ids[step['to']], 'label': step['label'],
                                     'kind': step['kind'], **({'condition': step['condition']} if step['condition'] else {})})
        page['fragments'].append({'text': '```clew-sequence\n' + encode(diagram).decode() + '```\n'})

    authored_overview = value['overview']
    need(isinstance(authored_overview['title'], str) and authored_overview['title'].strip()
         and len(authored_overview['title'].splitlines()) == 1, 'INVALID_INLINE_TITLE')
    title = authored_overview['title']
    overview['fragments'][0]['text'] = '# ' + title + '\n\n<a id="service-scope"></a>\n\n'
    nav['title'] = navigation['home']['title'] = title
    if len(overview['fragments']) > 1:
        overview['fragments'].append({'text': '\n'})
    service_requirement = next(row for row in requirements.values() if row['kind'] == 'SERVICE')
    accounts[service_requirement['id']]['disposition'] = disposition(authored_overview['disposition'])
    add_claims(overview, authored_overview['claims'])

    for ref, trace in referenced(value['entries'], selected['entries'], 'ref').items():
        requirement = requirements[selected['entries'][ref]]
        page = pages[requirement['page']]
        fragment = page['fragments'].pop()
        diagram = json.loads(fragment['text'].split('\n', 1)[1].rsplit('```', 1)[0])
        add_claims(page, trace['claims'])
        add_trace(page, trace, diagram)
        accounts[requirement['id']].update(section='handler', diagram=0, disposition=disposition(trace['disposition']))

    for file, finding in referenced(value['discovery'], selected['files'], 'ref').items():
        requirement_id = selected['files'][file]
        result = finding['discovery']
        need(result in ('NO_ADDITIONAL_STARTS', 'ADDITIONAL_STARTS') and
             bool(finding['starts']) == (result == 'ADDITIONAL_STARTS'), 'DISCOVERY_DISPOSITION_REQUIRED', file=file)
        discovered = []
        for i, start in enumerate(finding['starts'], 1):
            kind, title = start['kind'], start['title']
            kinds = {'HTTP': 'http', 'KAFKA': 'kafka', 'SCHEDULED': 'scheduled', 'OTHER': 'other'}
            need(kind in kinds and isinstance(title, str) and title.strip() and len(title.splitlines()) == 1,
                 'INVALID_DISCOVERED_ENTRY_ACCOUNT', file=file)
            local = file.lower() + '-' + str(i)
            name = service + '-discovered-' + local + '.md'
            need(name not in pages, 'INLINE_DISCOVERED_PAGE_COLLISION', page=name)
            target = name + '#handler'
            page = {'page': name, 'fragments': [{'text': '# ' + title + '\n\n<a id="handler"></a>\n\n'}]}
            pages[name] = page
            evidence = add_claims(page, start['claims'])
            handler = 'discovered-' + local
            method, path = start['method'], start['path']
            if kind == 'HTTP':
                need(isinstance(method, str) and method and isinstance(path, str) and path,
                     'DISCOVERED_HTTP_IDENTITY_REQUIRED', file=file)
                first = {'label': method + ' ' + path, 'kind': 'call'}
            else:
                first = {'label': start['startLabel'], 'kind': start['startKind']}
            diagram = {'title': title, 'participants': [{'id': 'caller', 'title': start['initiator']},
                       {'id': handler, 'title': title, 'target': target}],
                       'steps': [{'from': 'caller', 'to': handler, 'target': target, **first}]}
            add_trace(page, start, diagram)
            original['pages'].append(page)
            nav['pages'].append(name)
            nav['starts'].append({'title': title, 'summary': title, 'target': target, 'kind': kinds[kind]})
            discovered.append({'id': local, 'kind': kind, 'page': name, 'section': 'handler', 'diagram': 0,
                               'evidence': evidence, 'disposition': disposition(start['disposition']),
                               **({'method': method, 'path': path} if kind == 'HTTP' else {})})
        accounts[requirement_id].update(discoveryDisposition=result, discoveredEntries=discovered,
                                        disposition=disposition(finding['disposition']))

    need(return_bindings.keys() == {row['id'] for row in selected['returns'].values()}, 'LITERAL_RETURN_CLAIMS_REQUIRED',
         checks=list(selected['returns']))
    review = {'schema': COMPACT_REVIEW, 'requirements': accounts, 'claims': assessments,
              'unresolved': value['unresolved'], 'visual': visual, 'returnClaims': return_bindings}
    return {'original.json': original, 'navigation.json': navigation, 'self-review.json': review}
