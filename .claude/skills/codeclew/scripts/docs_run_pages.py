"""Plan isolated documentation authors and assemble their owned reader pages."""
from __future__ import annotations

from collections import defaultdict
import hashlib
import json
from itertools import chain, zip_longest
from pathlib import Path
import re
import runpy

MODE = 'ISOLATED_PAGES'
MAX_PACKET_BYTES = 192 * 1024
MAX_RESPONSE_BYTES = 128 * 1024
MAX_OVERVIEW_FACTS = 64
MAX_GUIDE_RANGES = 8
MAX_ROUTE_FACTS = 24
MAX_ROUTE_EVIDENCE_BYTES = 48 * 1024
ROUTE_BRIDGES = frozenset((
    'http-peer', 'message-peer', 'typed-message-peer', 'storage-reader', 'message-storage-reader',
    'storage-poll-activation', 'storage-message-continuation', 'local-response-independent-poll',
    'further-outbox-poll-activation', 'further-subscriber-outbox', 'further-subscriber-readback',
))
REACHED = runpy.run_path(str(Path(__file__).with_name('docs_run_reached.py')))
FACT_CONTEXT = runpy.run_path(str(Path(__file__).with_name('docs_run_fact_context.py')))
RESPONSE = runpy.run_path(str(Path(__file__).with_name('docs_run_page_response.py')))
OUTCOMES = runpy.run_path(str(Path(__file__).with_name('docs_run_page_outcomes.py')))
PROCESS = runpy.run_path(str(Path(__file__).with_name('docs_run_process_map.py')),
                         init_globals={'need': need, 'encode': encode})
expand_fact_context = FACT_CONTEXT['expand_fact_context']
LABELS = {
    'en': {'contracts': 'Data and contracts', 'process': 'Process map',
           'type': 'Type',
           'caller': 'Initiator', 'field': 'Field', 'meaning': 'Meaning', 'local': 'Service contracts',
           'external': 'External contracts', 'unsupported': 'The retained source does not establish the complete route.',
           'operations': 'Documented operations', 'relatedOperations': 'Related operations',
           'entryKinds': {'http': 'HTTP', 'kafka': 'Kafka', 'scheduled': 'Scheduled', 'background': 'Background',
                          'manual': 'Manual', 'other': 'Other entry points'},
           'noOperations': 'This documentation has no retained operation starts.',
           'localSequence': 'See the operation sequence',
           'routeOutcomes': {'normal': 'Normal outcome', 'error': 'Error outcome', 'handoff': 'Handoff',
                             'unknown': 'Unclassified outcome'},
           'operationOutcomes': {'title': 'Selected conditions and outcomes', 'condition': 'Path condition',
                                 'boundary': 'Evidence boundary'},
           'processRoutes': {'routes': 'Retained routes',
                             'readerOutcomes': 'What can happen',
                             'outcomeIndex': {'success': 'Successful outcomes', 'refusal': 'Refusals and early exits',
                                              'failure': 'Failures', 'handoff': 'Asynchronous handoffs',
                                              'outbound': 'Outbound boundaries'},
                             'routeDetail': 'Route details',
                             'handoffTo': '{message} → {operation}',
                             'moreOutcomes': 'More outcomes ({count})',
                             'intro': 'Open a route to see its evidenced connections and follow the operation that owns each decision.',
                             'operations': 'Participating operations',
                             'serviceRoutes': '{service} routes',
                             'externalBoundary': 'See the originating operation for the retained boundary.',
                             'externalRequest': 'Retained HTTP request {endpoint}. The destination and deployment are unresolved.',
                             'externalRequestUnknown': 'An exact outgoing endpoint could not be bound to this checked route.',
                             'externalRequestLimit': 'Receiving service and deployment are unresolved',
                             'unresolvedDestination': 'Unresolved destination',
                             'ownerConditions': 'See the participating operations for the conditions of each local path.',
                             'operationPath': 'Operation path',
                             'observedRoutes': 'Observed route variants ({count})',
                             'routeReference': 'reference {reference}',
                             'behavior': '{operation} behavior',
                             'sequence': '{operation} sequence',
                             'alternatives': 'Conditions and outcomes',
                             'conditional': 'The connections shown depend on the cited source conditions.',
                             'disabled': 'Automatic subscription startup is disabled in the retained source.',
                             'possible': 'The retained declaration gives a possible destination for this connection.',
                             'omitted': 'Some retained route records could not be matched to their operations or sources ({count}).',
                             'boundary': 'The complete end-to-end outcome and recipient remain open.',
                             'retainedPredicate': 'Retained path condition {condition}',
                             'independentResponse': 'The local response can complete independently of the related polling and delivery.',
                             'callerError': 'The caller can fail while decoding the response after the peer takes its commit path.',
                             'historical': 'This route was checked and promoted for the same retained source. The diagram shows its '
                                           'recorded connections.',
                             'historicalSteps': {'call': 'Call {operation}', 'await': 'Wait for {operation}',
                                                 'async': 'Handoff to {operation}', 'return': 'Return to {operation}'},
                             'declarations': 'Declared interactions',
                             'declarationScope': 'These relationships were declared in the retained documentation. Their execution is '
                                                 'unverified.',
                             'effects': {'unresolved': 'Retained connection',
                                         'outcome': 'Operation outcome',
                                         'exit-return': 'Local return',
                                         'exit-throw': 'Local exception',
                                         'write': 'Write path',
                                         'no-write': 'Path without a write',
                                         'error': 'Caller decoding failure',
                                         'independent-response': 'Independent local response'},
                             'protocols': {'http': {'scope': 'The retained HTTP request matches a documented operation. Delivery depends '
                                                             'on deployment and routing.',
                                                    'label': 'HTTP request · {qualifier}',
                                                    'condition': 'If the request reaches the matching operation'},
                                           'message': {'scope': 'The retained publication and subscription match. Continuation depends on '
                                                                'activation and delivery.',
                                                       'label': 'Publication · {qualifier}',
                                                       'condition': 'If delivered to the active subscription'},
                                           'storage': {'scope': 'The operations name the same storage relation. The read depends on '
                                                                'sharing the database and key and seeing the write.',
                                                       'label': 'Related read · {qualifier}',
                                                       'condition': 'Independent start, if it reads the same database and key'},
                                           'poll': {'scope': 'The independently registered poller can select the row if the write becomes '
                                                             'visible for the same database and key.',
                                                    'label': 'Independent poll · {qualifier}',
                                                    'condition': 'Independent trigger, if the poll selects the visible row'}}}},

}


def stable_id(text):
    return hashlib.sha256(text.encode()).hexdigest()[:16]


def route_id(route):
    return 'route-' + stable_id(route['origin'] + '\n' + route['title'])


def discovery_slots(plan, service):
    slots = (slot for slot in plan['slots'] if slot['kind'] == 'DISCOVERY' and slot['service'] == service)
    return {'F' + str(index): slot for index, slot in enumerate(slots, 1)}


def draft_target(page, anchor=None):
    return page.removesuffix('.md') + '.html' + ('#' + anchor if anchor else '')


def page_link(title, page, anchor=None):
    label = title.replace('\\', '\\\\').replace('[', '\\[').replace(']', '\\]')
    return f'[{label}]({draft_target(page, anchor)})'


def service_evidence(scope, owner, services, snapshot):
    home = Path(scope['sourceIndex']).parent
    ready = load(home / 'requirements.json')
    manifest_paths = [scope['manifest']] if scope['manifest'] else []
    facts = scope.get('preparedFacts', {})
    qualified = facts.get('citations', {})
    manifest_paths.extend(row['manifest'] for row in qualified.values() if 'manifest' in row)
    sources = {}
    for manifest in dict.fromkeys(manifest_paths):
        path = home / manifest
        _, packet, _, entries = citation_load(path)
        by_id = {entry['source']['id']: packet['texts'][entry['textKey']] for entry in packet['outcomes']}
        for handle, value in entries.items():
            source = value['source']
            service = source['service']
            text = by_id[source['id']]
            if service != owner:
                proof = qualified.get(handle, {})
                # Another service's manifest can carry additional ranges beyond
                # the exact handles selected by this page's prepared facts.
                if 'manifest' not in proof or home / proof['manifest'] != path:
                    continue
                need(service in services and all(proof.get(key) == source.get(key) for key in (
                    'service', 'revision', 'file', 'startLine', 'endLine'))
                    and packet['scope']['snapshot'] == snapshot
                    and peer_membership(packet, value, proof, text, services[service]),
                    'PAGE_PEER_SOURCE_MEMBERSHIP_CHANGED', file=source['file'])
            need(source['revision'] == services[service]['revision'], 'PAGE_SOURCE_REVISION_CHANGED', file=source['file'])
            sources[handle] = {**source, 'text': text}
    return_path = home / 'literal-returns.json'
    return {'ready': ready, 'sources': sources, 'facts': facts.get('entries', {}),
            'witnessCatalog': facts.get('witnessCatalog', ()),
            'returns': load(return_path) if return_path.exists() else [], 'coverage': scope['coverage'],
            'deferred': scope['deferredResourceFiles']}


def peer_membership(packet, entry, proof, text, scope):
    if packet.get('kind') != 'NATIVE':
        return packet['work'] == scope['work']
    # citation_load re-admits the engine evidence set. Match the reloaded
    # binding and the exact bytes against the handle composed for this page.
    source = entry['source']
    native = entry.get('nativeBinding', {})
    ticket = native.get('evidenceSet', {})
    return (native.get('kind') == 'SOURCE_NATIVE'
            and ticket.get('service') == packet['scope'].get('service') == source['service'] == source.get('bindingService')
            and ticket.get('snapshot') == packet['scope']['snapshot']
            and native.get('binding', {}).get('revision') == source['revision'] == scope['revision']
            and proof.get('textDigest') == source.get('textDigest') == 'sha256:' + hashlib.sha256(text.encode()).hexdigest())


def range_key(source):
    return REACHED['location'](source)


def source_views(scoped, sources, ordinary, catalogs=None, cache=None):
    selected = defaultdict(dict)
    catalogs = catalogs or {}
    cache = {} if cache is None else cache
    for identity, face in scoped.items():
        catalog = catalogs.get(identity, ())
        for group in ('facts', 'boundaries'):
            for fact in face.get(group, ()):
                witnesses = defaultdict(list)
                proofs = chain(fact.get('_sourceWitnesses', ()), (catalog[i] for i in fact.get('_sourceWitnessIds', ())))
                for proof in proofs:
                    witnesses[proof['cite']].append(proof)
                for handle in fact['sources']:
                    source = sources[handle]
                    slices = []
                    if handle not in ordinary:
                        for proof in witnesses[handle]:
                            key = handle, tuple(sorted(proof.items()))
                            if key not in cache:
                                cache[key] = REACHED['excerpt'](source, proof)
                            slices.append(cache[key])
                    for row in slices if slices and all(slices) else [source]:
                        selected[handle][range_key(row)] = row
    return selected


def source_payload(handles, sources, views=None, *, spans=None):
    """Share exact excerpts once while keeping the original citation bindings."""
    aliases, citations, regions, by_scope, by_range = {}, {}, {}, defaultdict(list), {}
    views = views or {}
    selected = {}
    for handle in handles:
        need(handle in sources, 'PAGE_CITATION_SOURCE_UNAVAILABLE', cite=handle)
        selected[handle] = list(views[handle].values()) if handle in views else [sources[handle]]
    unique = {range_key(source): source for rows in selected.values() for source in rows}
    for identity, source in sorted(unique.items(), key=lambda row: -len(row[1]['text'].encode())):
        display = {**source, 'startByte': source.get('startByte', (source.get('occurrence') or {}).get('startByte'))}
        scope = REACHED['key'](source)
        covered = next((ref for ref, row in by_scope[scope] if REACHED['relative_slice'](row, display) is not None), None)
        if covered is None:
            covered = 'V' + str(len(regions) + 1)
            regions[covered] = {field: source[field] for field in ('service', 'file', 'startLine', 'endLine', 'text')}
            by_scope[scope].append((covered, display))
        by_range[identity] = covered
    for handle, rows in selected.items():
        alias = 'S' + stable_id(handle)
        aliases[alias] = handle
        locations = list(dict.fromkeys(by_range[range_key(row)] for row in rows))
        if spans is not None:
            spans[alias] = list(dict.fromkeys((by_range[range_key(row)],
                                              max(row['startLine'], sources[handle]['startLine']),
                                              min(row['endLine'], sources[handle]['endLine'])) for row in rows))
        citations[alias] = {'region': locations[0], 'authority': sources[handle].get('authority'),
                            **({'regions': locations} if len(locations) > 1 else {})}
    return aliases, citations, regions



def entry_index(inventory):
    return [{'id': row['handlerId'], 'service': row['service'], 'page': row['page'],
             'title': (row['method'] + ' ' + row['path'] if row['method']
                       else row.get('entrySymbol') or row['entrypoint']),
             'kind': start_kind(row),
             'method': row['method'], 'path': row['path']}
            for row in inventory['requirements'] if row['kind'] == 'ENTRY']


def peer_declarations(inventory, entries):
    """Select only exact, unique retained peer declarations; keep their authority visible."""
    native = {row['handlerId']: row for row in inventory['requirements'] if row['kind'] == 'ENTRY'}

    def select(endpoint, transport=None):
        selector = endpoint.get('selector') or {}
        symbol = {selector['owner'] + separator + selector['name'] for separator in ('.', '#')} if (
            isinstance(selector.get('owner'), str) and isinstance(selector.get('name'), str)) else set()
        method, path = (transport or {}).get('method'), (transport or {}).get('path')
        candidates = [entry for entry in entries if entry['service'] == endpoint.get('service')
                      and (not symbol or native[entry['id']].get('entrySymbol') in symbol)
                      and (not method or entry['method'] == method) and (not path or entry['path'] == path)]
        if (symbol or (method and path)) and len(candidates) == 1:
            return candidates[0]['id']
        return None

    result = {}
    for scope in inventory['services'].values():
        for row in scope.get('peerDeclarations', ()):
            record = row.get('record', {})
            if record.get('kind') != 'DECLARED_INTERACTION' or row.get('kind') != 'DEPENDENCY':
                continue
            definition = record['normalized']
            identity = definition['id']
            if identity in result:
                continue
            transport = definition['transport']
            result[identity] = {'id': identity, 'title': definition['title'], 'authority': 'RETAINED_DECLARATION_NOT_EXECUTED',
                                'fromService': definition['from']['service'], 'toService': definition['to']['service'],
                                'origin': select(definition['from']), 'peer': select(definition['to'], transport),
                                'transport': {key: transport[key] for key in ('kind', 'method', 'path', 'topic') if key in transport},
                                **({'contractReference': definition['contractReference']} if definition.get('contractReference') else {})}
    return list(result.values())



def limit_facts(scoped, maximum):
    """Sample fixed-page or byte-limited facts across requirement scopes and boundaries."""
    count = sum(len(face.get(group, ())) for face in scoped.values() for group in ('facts', 'boundaries'))
    if maximum is None or count <= maximum:
        return scoped, 0
    kept = {identity: {**face, 'facts': [], 'boundaries': []} for identity, face in scoped.items()}
    remaining = maximum
    groups = [(kept[identity][group], iter(face.get(group, ())))
              for identity, face in scoped.items() for group in ('facts', 'boundaries') if face.get(group)]
    while groups and remaining:
        active = []
        for target, queue in groups:
            if (fact := next(queue, None)) is not None:
                target.append(fact)
                remaining -= 1
                active.append((target, queue))
            if remaining == 0:
                break
        groups = active
    return kept, count - maximum


def scoped_records(scoped):
    groups = []
    for identity, row in scoped.items():
        ranked = [(identity, group, fact) for group in ('facts', 'boundaries') for fact in row.get(group, ())]
        groups.append(sorted(ranked, key=lambda item: item[2].get('_rank', 0)))
    return [item for batch in zip_longest(*groups) for item in batch if item is not None]


def selected_records(scoped, records):
    selected = {identity: {key: item for key, item in row.items() if key not in ('facts', 'boundaries')}
                for identity, row in scoped.items()}
    for identity, group, fact in records:
        selected[identity].setdefault(group, []).append(fact)
    return selected


def admit_in_order(candidates, fits):
    """Try batches first, then smaller groups without losing later affordable evidence."""
    selected = []
    pending = [candidates] if candidates else []
    while pending:
        group = pending.pop()
        if fits(selected + group):
            selected.extend(group)
        elif len(group) > 1:
            middle = len(group) // 2
            pending.extend((group[middle:], group[:middle]))
    return selected


def scope_facts(scope, include):
    return {**scope, **{key: [fact for fact in scope.get(key, ()) if include(fact)]
                        for key in ('facts', 'boundaries')}}


def fact_key(fact):
    # An equal sentence is insufficient when it came from another occurrence.
    lines = {handle: sorted(ranges) for handle, ranges in fact.get('lines', {}).items()}
    witnesses = sorted(tuple(sorted(row.items())) for row in fact.get('_sourceWitnesses', ()))
    catalog_ids = sorted(fact.get('_sourceWitnessIds', ()))
    if not (lines or witnesses or catalog_ids):
        return None
    return (fact['kind'], fact['text'], fact.get('condition'), fact.get('uncertainty'),
            fact.get('status'), fact.get('authority'), tuple(sorted(fact['sources'])),
            json.dumps([lines, witnesses, catalog_ids], sort_keys=True))


def operation_evidence(requirements, indexes):
    by_source, behavior = defaultdict(set), defaultdict(set)
    for row in requirements:
        if row['kind'] != 'ENTRY':
            continue
        index = indexes[row['service']]
        for handle in index['ready'].get(row['id'], {}).get('evidenceCandidates', ()):
            by_source[handle].add(row['handlerId'])
        scope = index['facts'].get(row['id'], {})
        for key in ('facts', 'boundaries'):
            behavior[row['service']].update(identity for fact in scope.get(key, ()) if (identity := fact_key(fact)))
    return by_source, behavior


def guide_sources(owned, index, service):
    """Select service evidence outside the source owned by operations and discovery."""
    available = index['sources']
    local = [row for row in owned if row['kind'] != 'SERVICE']
    owned_handles = {handle for row in local
                     for handle in index['ready'].get(row['id'], {}).get('evidenceCandidates', ())}
    for row in local:
        scope = index['facts'].get(row['id'], {})
        for fact in chain(scope.get('facts', ()), scope.get('boundaries', ())):
            owned_handles.update(fact['sources'])
    owned_ranges = defaultdict(list)
    for handle in owned_handles & available.keys():
        source = available[handle]
        owned_ranges[REACHED['key'](source)].append(source)
    distinct = [handle for handle, source in available.items()
                if source['service'] == service and handle not in owned_handles and not any(
                    REACHED['relative_slice'](outer, source) is not None
                    for outer in owned_ranges.get(REACHED['key'](source), ()))]
    return sorted(distinct, key=lambda handle: (len(available[handle]['text'].encode()), handle))[:MAX_GUIDE_RANGES]


def route_evidence(requirements, indexes, all_sources, by_source, declarations, *, prepared):
    """Keep evidence of relationships between operations, including same-service relationships."""
    operations = [row for row in requirements if row['kind'] == 'ENTRY']
    if len(operations) < 2:
        return {}, {}, {}
    scopes, ranges, linked, covered = {}, defaultdict(list), defaultdict(set), set()
    for row in operations:
        service, identity = row['service'], row['id']
        index = indexes[service]
        if scope := index['facts'].get(identity):
            structural, fallback = {}, {}
            for fact in chain(scope.get('facts', ()), scope.get('boundaries', ())):
                if fact.get('authority') != 'RETAINED_SOURCE_EDGE':
                    continue
                handles = fact['sources']
                peers = {ref for handle in handles for ref in by_source.get(handle, ())} - {row['handlerId']}
                services = {all_sources[handle]['service'] for handle in handles} - {service}
                if not peers and not services:
                    continue
                linked[identity].update(peers)
                signature = frozenset(peers), frozenset(services)
                kind = fact['kind']
                bridge = kind in ROUTE_BRIDGES or kind.removeprefix('http-') in ROUTE_BRIDGES
                if bridge:
                    structural.setdefault(signature, []).append(fact)
                else:
                    # A projector can qualify a relationship without a separate bridge proposition.
                    size = sum(len((fact.get(field) or '').encode()) for field in ('text', 'condition', 'uncertainty'))
                    if signature not in fallback or size < fallback[signature][0]:
                        fallback[signature] = size, fact
            selected = {id(fact) for group in structural.values() for fact in group}
            selected.update(id(fact) for signature, (_, fact) in fallback.items() if signature not in structural)
            if selected:
                scopes[identity] = scope_facts(scope, lambda fact: id(fact) in selected)
                covered.update(frozenset((row['handlerId'], peer)) for peer in linked[identity])
    declared = {ref for row in declarations if row['origin'] and row['peer'] and row['origin'] != row['peer']
                and frozenset((row['origin'], row['peer'])) not in covered for ref in (row['origin'], row['peer'])}
    for row in operations:
        if not prepared or row['handlerId'] in declared:
            service = row['service']
            ready = indexes[service]['ready'].get(row['id'], {}).get('evidenceCandidates', ())
            ranges[service].extend(ready)
    return scopes, ranges, linked


def page_jobs(job, inventory, prepared, directory, maximum_pages, instructions):
    need(prepared is not None, 'ISOLATED_PAGE_SOURCE_UNAVAILABLE', blocked=True)
    directory.mkdir(mode=0o700)
    indexes = {service: service_evidence(scope, service, inventory['services'], inventory['snapshot'])
               for service, scope in prepared['services'].items()}
    entries = entry_index(inventory)
    peer_links = peer_declarations(inventory, entries)
    requirements = inventory['requirements']
    all_sources = {handle: source for row in indexes.values() for handle, source in row['sources'].items()}
    by_source, owned_behavior = operation_evidence(requirements, indexes)
    route_facts, route_ranges, route_links = route_evidence(requirements, indexes, all_sources, by_source, peer_links,
                                                            prepared=job.get('evidenceMode') == 'PREPARED_FACTS'
                                                            or any(index['facts'] for index in indexes.values()))
    routed = len(job['services']) > 1
    required_pages = [row['page'] for row in requirements if row['kind'] != 'DISCOVERY']
    taken = set(required_pages)
    need(len(taken) == len(required_pages), 'PAGE_STARTER_DESTINATION_COLLISION', blocked=True)

    def fixed_name(base):
        name = base + '.md'
        if name in taken:
            name = base + '-' + stable_id(base) + '.md'
        need(name not in taken, 'PAGE_DESTINATION_COLLISION', page=name)
        taken.add(name)
        return name

    contracts_pages = {service: fixed_name(service + '-contracts') for service in job['services']}
    process_page = fixed_name('process-map')
    need(len(taken) <= maximum_pages, 'REQUIRED_PAGE_LIMIT', requiredPages=len(taken), limit=maximum_pages)
    slots = []
    base_schema = load(Path(__file__).resolve().parent.parent / 'schemas/docs-run-page-author.schema.json')

    route_outcomes = defaultdict(list)
    for row in prepared.get('routeRecords', ()):
        choice = row['selection']
        if (row['status'] == 'SOURCE_QUALIFIED_CANDIDATE' and choice and choice.get('httpStatus')
                and choice.get('condition') and len(route_outcomes[choice['owner']]) < 4):
            route_outcomes[choice['owner']].append({**choice, 'kind': 'http-outcome', 'text': '',
                                              '_httpStatus': choice['httpStatus']})

    def add(kind, page, service, required, available, context):
        slot_instructions = RESPONSE['instructions_for_slot'](instructions, kind)
        key = kind.lower() + '-' + stable_id(page + ':' + (required['id'] if required else ''))
        mandatory, supplemental, scoped_facts, returns, witness_catalogs = [], [], {}, {}, {}
        for owner in available:
            index = indexes[owner]
            owned = [row for row in requirements if row['service'] == owner]
            evidence_requirements = [required] if required else owned
            if kind == 'GUIDE':
                supplemental.extend(guide_sources(owned, index, owner))
                evidence_requirements = [row for row in owned if row['kind'] == 'SERVICE']
            if kind == 'PROCESS_MAP':
                supplemental.extend(route_ranges.get(owner, ()))
            for row in evidence_requirements:
                ready = index['ready'].get(row['id'], {})
                if kind not in ('GUIDE', 'PROCESS_MAP'):
                    group = mandatory if required else supplemental
                    group.extend(ready.get('evidenceCandidates', []))
                facts = (route_facts if kind == 'PROCESS_MAP' else index['facts']).get(row['id'])
                if facts:
                    if kind == 'GUIDE':
                        positive = [fact for fact in facts.get('facts', ())
                                    if fact.get('status') in (None, 'SUPPORTED', 'SOURCE_QUALIFIED_CANDIDATE')
                                    and all(all_sources[handle]['service'] == owner for handle in fact['sources'])
                                    and fact_key(fact) not in owned_behavior[owner]]
                        facts = {'facts': positive} if positive else {}
                    if kind == 'DISCOVERY':
                        facts = scope_facts(facts, lambda fact: fact_key(fact) not in owned_behavior[owner])
                    if kind == 'CONTRACTS':
                        facts = scope_facts(facts, lambda fact: fact.get('authority', '').startswith('DECLARED_'))
                    scoped_facts[row['id']] = facts
                    witness_catalogs[row['id']] = index.get('witnessCatalog', ())
            if kind == 'OPERATION':
                returns.update({'R' + str(i): value for i, value in enumerate(
                    (value for value in index['returns'] if value['page'] == page), 1)})
                mandatory.extend(row['cite'] for row in returns.values())
            if kind == 'CONTRACTS':
                supplemental.extend(handle for handle, source in index['sources'].items() if source['service'] == owner)

        overview_limit = MAX_ROUTE_FACTS if kind == 'PROCESS_MAP' else (
            MAX_OVERVIEW_FACTS if kind in ('GUIDE', 'CONTRACTS') else None)
        scoped_facts, omitted_overview = limit_facts(scoped_facts, overview_limit)
        fact_groups = [fact['sources'] for scoped in scoped_facts.values() for category in ('facts', 'boundaries')
                       for fact in scoped.get(category, ())]
        fact_handles = {handle for group in fact_groups for handle in group}
        boundary_facts = [row for row in prepared.get('routeBoundaries', ())
                          if required and kind == 'OPERATION' and row['owner'] == required['id']
                          and row.get('_conditionCases')]
        local_checks = scoped_facts.get(required['id'], {}).get('facts', ()) if required and kind == 'OPERATION' else ()
        route_checks = route_outcomes.get(required['id'], ()) if required and kind == 'OPERATION' else ()
        supplemental.extend(handle for row in chain(boundary_facts[:4], route_checks) for handle in row['sources']
                            if handle in all_sources and len(all_sources[handle]['text'].encode()) <= 8192)

        optional = set(supplemental) | fact_handles
        mandatory = list(dict.fromkeys(mandatory))
        supplemental = list(dict.fromkeys(supplemental))

        def serialize(selected, views, active, omitted_facts):
            spans = {}
            aliases, citations, sources = source_payload(selected, all_sources, views, spans=spans)
            reverse = {handle: alias for alias, handle in aliases.items()}
            facts = {identity: {} for identity in active}
            prepared_omissions = {identity: row['_preparationOmissions'] for identity, row in scoped_facts.items()
                                  if '_preparationOmissions' in row}
            add_prepared_facts({identity: (identity, target) for identity, target in facts.items()}, active, reverse)
            cited_services = dict.fromkeys([*available, *(all_sources[handle]['service'] for handle in selected)])
            packet = {'slot': key, 'type': kind, 'page': page, 'request': job['request'], 'language': job['language'],
                      'service': service, 'context': context, 'citations': citations, 'sourceRanges': sources,
                      'facts': {identity: value for identity, value in facts.items() if value},
                      'returns': {ref: {'condition': row['predicate'], 'cite': reverse[row['cite']]}
                                  for ref, row in returns.items()},
                      'coverage': {owner: {'retained': indexes[owner]['coverage']['coverage'],
                                           'limits': indexes[owner]['coverage'].get('boundaries', []),
                                           'deferredResources': indexes[owner]['deferred'],
                                           'peerDeclarations': inventory['services'][owner].get('peerDeclarationCoverage', 'UNAVAILABLE')}
                                   for owner in cited_services},
                      'omittedOptionalRanges': len(optional - set(selected)),
                      'omittedPreparedFacts': omitted_facts,
                      'preparationOmissions': prepared_omissions}
            if required:
                packet['requirement'] = {key: required[key] for key in (
                    'id', 'kind', 'file', 'entryKind', 'entrySymbol', 'method', 'path', 'authority', 'boundaries') if key in required}
            if kind == 'OPERATION' and (job.get('evidenceMode') == 'PREPARED_FACTS' or local_checks or boundary_facts):
                visible = {id(fact) for fact in chain(boundary_facts[:4], route_checks,
                                                     active.get(required['id'], {}).get('facts', ()))}

                def compact_proof(handles):
                    refs = [reverse[handle] for handle in handles]
                    for host in handles:
                        outer = spans[reverse[host]]
                        if all(handle == host or (
                                REACHED['relative_slice'](all_sources[host], all_sources[handle]) is not None
                                and all(any(loc == view and low <= first <= last <= high for loc, low, high in outer)
                                        for view, first, last in spans[reverse[handle]])) for handle in handles):
                            return [reverse[host]]
                    return refs

                paths, coverage = RESPONSE['CONDITION_PATHS']['select'](
                    chain(boundary_facts, local_checks, route_checks), reverse, visible=visible, compact=compact_proof,
                    language=job['language'] if job.get('evidenceMode') == 'PREPARED_FACTS' else None)
                packet['conditionPathCoverage'] = coverage
                if paths:
                    packet['conditionPaths'] = paths
            prompt = FACT_CONTEXT['author_prompt'](slot_instructions, packet)
            return prompt, aliases, spans, {ref: (row['startLine'], row['endLine']) for ref, row in sources.items()}

        if kind == 'OPERATION':
            candidates = scoped_records(scoped_facts)
            cache = {}

            def render(records, additional):
                active = selected_records(scoped_facts, records)
                fact_handles = (handle for _, _, fact in records for handle in fact['sources'])
                handles = list(dict.fromkeys(chain(mandatory, fact_handles, additional)))
                views = source_views(active, all_sources, set(mandatory) | set(additional), witness_catalogs, cache)
                return serialize(handles, views, active, len(candidates) - len(records))

            # Measure exactly what the author gets. The normal case uses one serialization.
            prompt, aliases, spans, regions = render(candidates, supplemental)
            if len(prompt.encode()) > MAX_PACKET_BYTES:
                prompt, aliases, spans, regions = render([], [])

                def fits(records, additional):
                    nonlocal prompt, aliases, spans, regions
                    trial, trial_aliases, trial_spans, trial_regions = render(records, additional)
                    if len(trial.encode()) > MAX_PACKET_BYTES:
                        return False
                    prompt, aliases, spans, regions = trial, trial_aliases, trial_spans, trial_regions
                    return True

                accepted = admit_in_order(candidates, lambda records: fits(records, []))
                admit_in_order(supplemental, lambda additional: fits(accepted, additional))
            need(aliases, 'PAGE_SLOT_SOURCE_UNAVAILABLE', page=page, kind=kind, blocked=True)
        else:
            views = source_views(scoped_facts, all_sources, set(mandatory) | set(supplemental), witness_catalogs)
            handles = mandatory.copy()
            remaining = MAX_PACKET_BYTES - len(slot_instructions.encode()) - 16 * 1024
            if kind == 'PROCESS_MAP':
                remaining = min(remaining, MAX_ROUTE_EVIDENCE_BYTES)
            ranges = {key: row for handle in handles for key, row in
                      views.get(handle, {range_key(all_sources[handle]): all_sources[handle]}).items()}
            included = set(ranges)
            bytes_used = sum(len(row['text'].encode()) for row in ranges.values())
            selected_handles = set(handles)
            supplemental_groups = ((handle,) for handle in supplemental)
            for group in chain(fact_groups, supplemental_groups):
                new_handles = list(dict.fromkeys(handle for handle in group if handle not in selected_handles))
                excerpts = {key: row for handle in new_handles for key, row in
                            views.get(handle, {range_key(all_sources[handle]): all_sources[handle]}).items() if key not in included}
                size = sum(len(row['text'].encode()) + 256 for row in excerpts.values()) + 128 * len(new_handles)
                if bytes_used + size <= remaining:
                    handles.extend(new_handles)
                    selected_handles.update(new_handles)
                    included.update(excerpts)
                    bytes_used += size
            if kind in ('GUIDE', 'PROCESS_MAP') and not handles:
                return
            need(handles, 'PAGE_SLOT_SOURCE_UNAVAILABLE', page=page, kind=kind, blocked=True)

            def render(selected, maximum_facts=None):
                visible = set(selected)
                records = scoped_records(scoped_facts)
                admitted = [row for row in records if visible.issuperset(row[2]['sources'])]
                active = selected_records(scoped_facts, admitted)
                active, omitted_bytes = limit_facts(active, maximum_facts)
                frozen = serialize(selected, views, active, omitted_overview + len(records) - len(admitted) + omitted_bytes)
                return *frozen, len(admitted) - omitted_bytes

            required_count = max(1, len(mandatory))
            prompt, aliases, spans, regions, visible_count = render(handles)
            maximum_facts = None
            while len(prompt.encode()) > MAX_PACKET_BYTES and visible_count > 16:
                maximum_facts = max(16, visible_count - max(1, visible_count // 8))
                prompt, aliases, spans, regions, visible_count = render(handles, maximum_facts)
            while len(prompt.encode()) > MAX_PACKET_BYTES and len(handles) > required_count:
                handles = handles[:max(required_count, len(handles) - max(1, len(handles) // 8))]
                prompt, aliases, spans, regions, visible_count = render(handles, maximum_facts)
            while len(prompt.encode()) > MAX_PACKET_BYTES and visible_count:
                maximum_facts = visible_count - max(1, visible_count // 8)
                prompt, aliases, spans, regions, visible_count = render(handles, maximum_facts)
        need(len(prompt.encode()) <= MAX_PACKET_BYTES, 'PAGE_SLOT_PACKET_LIMIT', page=page, kind=kind,
             bytes=len(prompt.encode()), limit=MAX_PACKET_BYTES, blocked=True,
             nextAction='Use a smaller retained subject or document this source boundary explicitly in the workspace workflow.')
        path = directory / (key + '.txt')
        path.write_text(prompt, encoding='utf-8')
        schema = directory / key / 'docs-run-page-author.schema.json'
        slot = {'key': key, 'kind': kind, 'page': page, 'service': service,
                'prompt': str(path.relative_to(directory.parent)), 'schema': str(schema.relative_to(directory.parent)),
                'requirement': required['id'] if required else None, 'citations': aliases, 'citationSpans': spans,
                'sourceRegions': regions, 'returns': {ref: {'id': row['id'], 'page': row['page']} for ref, row in returns.items()}}
        if kind == 'OPERATION':
            related = {link['peer'] for link in context['declaredInteractions']}
            related.update(route_links.get(required['id'], ()))
            admitted = set(aliases.values())
            start_handles = indexes[service]['ready'].get(required['id'], {}).get('evidenceCandidates', ())
            registrations = set(required.get('registrationSourceIds', ()))
            start_handles = [handle for handle in start_handles if handle in admitted]
            if start_handles:
                slot['startCitation'] = next((handle for handle in start_handles if all_sources[handle].get('id') in registrations),
                                             start_handles[0])
            refs = [entry['id'] for entry in entries if entry['id'] in related and entry['id'] != required['handlerId']]
            if refs:
                slot['relatedOperations'] = refs
            packet = json.loads(prompt.rsplit('\n\n', 1)[-1])
            if paths := packet.get('conditionPaths'):
                slot['conditionPaths'] = paths
            if coverage := packet.get('conditionPathCoverage'):
                slot['conditionPathCoverage'] = coverage
            local_status = {}
            for fact in scoped_facts.get(required['id'], {}).get('facts', ()):
                status = fact.get('_httpStatus')
                if (type(status) is int and 200 <= status < 600 and set(fact['sources']) <= admitted
                        and fact.get('status') in (None, 'SUPPORTED', 'SOURCE_QUALIFIED_CANDIDATE')):
                    condition = fact.get('condition')
                    local_status.setdefault((status, condition), {
                        'httpStatus': status, 'condition': condition, 'sources': fact['sources'], 'effect': 'outcome'})
            if local_status:
                slot['mapOutcomes'] = list(local_status.values())[:16]
            if job['language'] == 'en':
                packet = expand_fact_context(packet)
                source_bytes = {ref: len(all_sources[handle]['text'].encode()) for ref, handle in aliases.items()}
                if outcomes := OUTCOMES['select'](packet, required['id'], source_bytes):
                    slot['sourceOutcomes'] = outcomes
        if kind == 'PROCESS_MAP':
            slot['operationIds'] = [entry['id'] for entry in context['operations']]
            slot['transitionRoutes'] = True
        if kind in ('GUIDE', 'PROCESS_MAP'):
            slot['ownedContentOnly'] = True
        schema.parent.mkdir(mode=0o700, parents=True)
        schema.write_bytes(encode(RESPONSE['schema_for_slot'](base_schema, slot)))
        slots.append(slot)

    for service in job['services']:
        overview = service + '.md'
        local = [entry for entry in entries if entry['service'] == service]
        relations = [row for row in peer_links if service in (row['fromService'], row['toService'])]
        owner = [service]
        add('GUIDE', overview, service, None, owner, {})
        add('CONTRACTS', contracts_pages[service], service, None, owner,
            {'operations': local, 'declaredInteractions': relations})
        for row in requirements:
            if row['service'] != service or row['kind'] == 'SERVICE':
                continue
            kind = 'OPERATION' if row['kind'] == 'ENTRY' else 'DISCOVERY'
            local_entry = next((entry for entry in local if entry['page'] == row['page']), None)
            context = ({'operation': local_entry, 'contractsPage': contracts_pages[service], 'processPage': process_page,
                        'declaredInteractions': [link for link in relations if link['origin'] == row.get('handlerId')]}
                       if kind == 'OPERATION' else {'knownStarts': local})
            add(kind, row['page'], service, row, owner, context)
    if 'routeRecords' not in prepared and (route_facts or any(route_ranges.values())):
        add('PROCESS_MAP', process_page, None, None, job['services'],
            {'operations': entries, 'multiService': routed, 'declaredInteractions': peer_links})
    used_handles = {handle for slot in slots for handle in slot['citations'].values()}
    processes = {}
    if 'routeRecords' in prepared:
        records, omitted = PROCESS['select_records'](prepared['routeRecords'], requirements, entries, all_sources)
        processes = {'routeRecords': records, 'routeRecordOmissions': omitted, 'routeNarrativeLayout': 'anchored',
                     'routePresentation': 'owner-scoped-v2', 'processOutcomes': True,
                     'routeBoundaries': PROCESS['select_boundaries'](prepared.get('routeBoundaries', ()), requirements, all_sources),
                     'routeDeclarations': [row for row in peer_links if row['origin'] and row['peer']]}
        used_handles.update(all_sources)
    return {**processes, 'mode': MODE, 'reason': 'EXPLICIT_ISOLATED_PAGE_AUTHORING', 'slots': slots, 'operationOwnership': True,
            'navigationOverview': True,
            'contracts': contracts_pages, 'processPage': process_page, 'entries': entries, 'maximumPages': maximum_pages,
            'routed': routed, 'citationOwners': {handle: source['service'] for handle, source in all_sources.items()
                                               if handle in used_handles}}


def bounded_line(value, maximum=160):
    return isinstance(value, str) and bool(value.strip()) and '\n' not in value and '\r' not in value and len(value) <= maximum


def claim_handles(claim, slot):
    closed(claim, ('heading', 'text', 'cites', 'assessment', 'returns'))
    need(isinstance(claim['text'], str) and claim['text'].strip() and len(claim['text']) <= 20000
         and 'clew-sequence' not in claim['text'] and type(claim['heading']) is str
         and (not claim['heading'] or bounded_line(claim['heading'])), 'PAGE_CLAIM_INVALID')
    need(claim['assessment'] in ('SUPPORTED', 'EXPLICIT_UNKNOWN'), 'PAGE_CLAIM_ASSESSMENT_REQUIRED')
    need(isinstance(claim['returns'], list) and all(ref in slot['returns'] for ref in claim['returns']), 'PAGE_RETURN_REF_UNKNOWN')
    return citation_handles(claim['cites'], slot)


def citation_handles(aliases, slot):
    need(isinstance(aliases, list) and aliases and len(set(aliases)) == len(aliases)
         and all(alias in slot['citations'] for alias in aliases), 'PAGE_CITATION_OUTSIDE_PACKET')
    return [slot['citations'][alias] for alias in aliases]


def validate_response(slot, value):
    return RESPONSE['review_response'](slot, value, need)



def heading(name, text, anchor=None):
    return {'page': name, 'fragments': [{'text': '# ' + text + '\n\n' + (
        '<a id="' + anchor + '"></a>\n\n' if anchor else '')}]}


def append_claim(page, review, text, evidence, assessment='SUPPORTED'):
    accounts = review['claims'].setdefault(page['page'], {})
    identity = 'claim-' + str(len(accounts) + 1)
    accounts[identity] = assessment
    page['fragments'].append({'id': identity, 'text': text.strip() + '\n\n', 'evidence': evidence})


def append_claims(page, review, claims, slot):
    for claim in claims:
        if claim['heading']:
            page['fragments'].append({'text': '## ' + claim['heading'] + '\n\n'})
        append_claim(page, review, claim['text'], claim_handles(claim, slot), claim['assessment'])


def table_cell(text):
    return text.replace('\\', '\\\\').replace('|', '\\|').replace('<', '&lt;').replace('>', '&gt;')


def add_contracts(original, review, nav, slot, response, labels):
    page = heading(slot['page'], labels['contracts'])
    original['pages'].append(page)
    nav['pages'].append(slot['page'])
    nav['data'].append({'title': labels['contracts'], 'target': slot['page']})
    append_claims(page, review, response['claims'], slot)
    for scope, label in (('LOCAL', labels['local']), ('EXTERNAL', labels['external'])):
        rows = [row for row in response['contracts'] if row['scope'] == scope]
        if not rows:
            continue
        page['fragments'].append({'text': '## ' + label + '\n\n'})
        for row in rows:
            page['fragments'].append({'text': '### ' + row['name'] + '\n\n'})
            header = ' | '.join(labels[key] for key in ('field', 'type', 'meaning'))
            data = ['| ' + ' | '.join(table_cell(field[key]) for key in ('name', 'type', 'meaning')) + ' |'
                    for field in row['fields']]
            append_claim(page, review, '\n'.join(['| ' + header + ' |', '| --- | --- | --- |', *data]),
                         citation_handles(row['cites'], slot))


def start_titles(navigation):
    return {row['target'].partition('#')[0]: row['title']
            for service in navigation['services'] for row in service['starts']}


def append_service_starts(page, nav, labels, kinds):
    by_kind = defaultdict(list)
    for start in nav['starts']:
        name = start['target'].partition('#')[0]
        by_kind[start['kind']].append('- ' + page_link(start['title'], name, 'behavior'))
    if by_kind:
        groups = ['### ' + title + '\n\n' + '\n'.join(by_kind[kind])
                  for kind, title in kinds.items() if kind in by_kind]
        page['fragments'].append({'text': '\n## ' + labels['operations'] + '\n\n' + '\n\n'.join(groups) + '\n\n'})


def add_processes(original, review, navigation, plan, slot, response, job, labels, cite_services,
                  operation_steps=None, start_sources=None, operation_values=None, historical_routes=()):
    """Own the map and canonical non-service route destinations from one fixed slot."""
    ownership = plan.get('operationOwnership')
    process_page = plan['processPage']
    page = heading(process_page, labels['process'])
    original['pages'].append(page)
    claims = [] if slot and slot.get('ownedContentOnly') else response['claims']
    append_claims(page, review, claims, slot)
    titles = start_titles(navigation) if ownership else {}
    operations = {entry['id']: {**entry, 'title': titles.get(entry['page'], entry['title'])}
                  for entry in plan['entries']}
    starts = [start for service in navigation['services'] for start in service['starts']]
    start_fragment = len(page['fragments'])
    operation_links = [page_link(start['title'], start['target'].partition('#')[0], 'behavior') if ownership
                       else page_link(start['title'], *start['target'].split('#', 1)) for start in starts]
    if operation_links:
        title = '## ' + labels['operations'] + '\n\n' if ownership else ''
        body = '\n'.join('- ' + link for link in operation_links)
        if start_sources and not claims:
            page['fragments'].append({'text': title})
            evidence = list(dict.fromkeys(start_sources[start['target'].partition('#')[0]] for start in starts))
            append_claim(page, review, body, evidence)
        else:
            page['fragments'].append({'text': title + body + '\n\n'})
    elif start_sources and not claims:
        append_claim(page, review, labels['noOperations'], [next(iter(start_sources.values()))], 'EXPLICIT_UNKNOWN')
    docset = 'retained'
    version = 'r-' + stable_id(job['snapshot'])

    def target(name, anchor=None):
        result = {'docset': docset, 'page': draft_target(name)}
        if anchor:
            result['anchor'] = anchor
        return result

    catalog = {'schema': 'codeclew-cross-docset-routes/1.1', 'revision': version,
               'catalog': target(process_page), 'externalActors': [], 'routes': [],
               'docsets': [{'id': docset, 'title': navigation['home']['title'], 'version': version,
                            'baseURL': 'https://retained-docs.invalid/',
                            'services': [{'id': s['id'], 'title': s['title'], 'overview': target(s['overview'])}
                                         for s in navigation['services']]}]}
    routes = catalog['routes']
    if 'routeRecords' in plan:
        later = []
        if plan.get('routePresentation') == 'owner-scoped-v2':
            later = page['fragments'][start_fragment:]
            del page['fragments'][start_fragment:]
        PROCESS['render'](original, review, page, catalog, target, operations, operation_values, plan,
                          historical_routes, labels, heading, page_link, append_claim)
        page['fragments'].extend(later)
        return catalog
    taken = {row['page'] for row in original['pages']}
    for row in response['routes']:
        origin = operations.get(row['origin'])
        refs = [row['origin'], *(peer['entry'] for peer in row['participants'])]
        need(origin is not None and len(set(refs)) == len(refs) and all(ref in operations for ref in refs),
             'PAGE_ROUTE_ENTRY_OUTSIDE_PACKET', route=row['title'])
        need((row['terminal'] in refs and row['boundary'] is not None) or (
            row['terminal'] is None and row['boundary'] is None), 'PAGE_ROUTE_TERMINAL_INVALID', route=row['title'])
        cited = citation_handles(row['cites'], slot)
        need({operations[ref]['service'] for ref in refs} <= {cite_services[handle] for handle in cited},
             'PAGE_ROUTE_PARTICIPANT_EVIDENCE_REQUIRED', route=row['title'])
        identity = route_id(row)
        name = identity + '.md'
        need(name not in taken, 'PAGE_DESTINATION_COLLISION', page=name)
        taken.add(name)
        narrative = heading(name, row['title'], 'scope')
        original['pages'].append(narrative)
        page['fragments'].append({'text': '- ' + page_link(row['title'], name) + '\n\n'})
        append_claim(narrative, review, row['summary'], cited)
        narrative['fragments'].append({'text': '<a id="branch"></a>\n\n'})
        steps = route_steps(row, operations, operation_steps or {}, labels) if ownership else row['steps']
        numbered = ['1. ' + page_link(origin['title'], origin['page'], 'behavior' if ownership else 'handler')]
        for index, step in enumerate(steps, 2):
            label = page_link(step['label'], *step['target'].split('#', 1)) if 'target' in step else step['label']
            numbered.append(f"{index}. {label}" + (f" ({step['condition']})" if step['condition'] else ''))
        append_claim(narrative, review, row['condition'] + '\n\n' + '\n'.join(numbered), cited)
        narrative['fragments'].append({'text': '<a id="outcome"></a>\n\n'})
        append_claim(narrative, review, row['outcome'], cited,
                     'EXPLICIT_UNKNOWN' if row['result'] == 'unknown' else 'SUPPORTED')
        narrative['fragments'].append({'text': '- ' + page_link(labels['process'], process_page) + '\n' + ''.join(
            '- ' + page_link(operations[ref]['title'], operations[ref]['page'], 'behavior' if ownership else 'handler') + '\n'
            for ref in refs)})
        roles = {row['origin']: origin['title'], **{peer['entry']: peer['role'] for peer in row['participants']}}
        occurrences = [{'id': ref, 'actor': {'kind': 'service', 'docset': docset, 'service': operations[ref]['service']},
                        'role': roles[ref], 'target': target(operations[ref]['page'], 'handler')} for ref in refs]
        diagram = {'title': row['title'],
                   'participants': [{'id': 'caller', 'title': labels['caller']},
                                    *({'id': ref, 'title': operations[ref]['title'],
                                       'target': operations[ref]['page'] + '#handler'} for ref in refs)],
                   'steps': [{'from': 'caller', 'to': row['origin'], 'label': origin['title'],
                              'kind': 'call' if origin['kind'] == 'http' else 'async'},
                             *({key: item for key, item in step.items() if item is not None} for step in steps)]}
        narrative['fragments'].append({'text': '\n\n```clew-sequence\n' + encode(diagram).decode() + '```\n'})
        terminal = row['terminal']
        end = ({'status': 'known', 'ownerOccurrence': terminal, 'boundary': row['boundary'],
                'recipient': {'kind': 'unknown', 'reason': row['recipient']}} if terminal else
               {'status': 'unknown', 'reason': row['recipient']})
        outcome_title = row['outcome']
        if not ownership:
            outcome_title = outcome_title[:160]
        elif len(outcome_title.encode('utf-16-le')) > 320:
            outcome_title = labels['routeOutcomes'][row['result']]
        routes.append({'id': identity, 'version': version, 'title': row['title'], 'narrative': target(name),
                       'origin': {'status': 'known', 'occurrence': row['origin'], 'kind': origin['kind'],
                                  'title': origin['title'], 'summary': row['summary']},
                       'occurrences': occurrences,
                       'branches': [{'id': 'branch', 'title': row['title'], 'condition': row['condition'],
                                     'target': target(name, 'branch')}],
                       'outcomes': [{'id': 'outcome', 'branch': 'branch', 'kind': row['result'], 'title': outcome_title,
                                     'target': target(name, 'outcome'), 'end': end,
                                     'internal': {'coverage': 'partial', 'occurrences': [ref for ref in refs
                                                  if ref not in (row['origin'], terminal)]}}],
                       'coverage': {'outcomeInventory': 'partial', 'scope': row['summary'], 'reference': target(name, 'scope')}})
    if not routes and not plan.get('navigationOverview') and (not ownership or len(operations) > 1):
        page['fragments'].append({'text': labels['unsupported'] + '\n\n'})
    return catalog


def route_steps(row, operations, operation_steps, labels):
    """Replace repeated local notes with links while retaining transitions and unique context."""
    steps = []
    for step in row['steps']:
        actor = step['from']
        repeats = actor == step['to'] and step['kind'] == 'note' and any(
                local['from'] == local['to'] == 'handler' and local['kind'] == 'note'
                and local['label'] == step['label'] and local['condition'] == step['condition']
                for local in operation_steps.get(actor, ()))
        if repeats or step['kind'] == 'reference':
            step = {**step, 'kind': 'note', 'label': labels['localSequence'],
                    'condition': None if repeats else step['condition'], 'target': operations[actor]['page'] + '#sequence'}
            if steps and steps[-1] == step:
                continue
        steps.append(step)
    return steps


def operation_anchors(page):
    page['fragments'].insert(1, {'text': '<a id="behavior"></a>\n\n'})
    sequence = next(index for index, part in enumerate(page['fragments']) if part['text'].startswith('```clew-sequence\n'))
    page['fragments'].insert(sequence, {'text': '<a id="sequence"></a>\n\n'})


def assemble_pages(job, inventory, plan, values, starter, navigation, inline_assemble, cite_services,
                   historical_routes=()):
    """Compose accepted outputs in the starter order, regardless of author completion order."""
    slots = {slot['key']: slot for slot in plan['slots']}
    need(values.keys() == slots.keys(), 'PAGE_SLOT_RESPONSE_MISSING')
    original = {'pages': []}
    review = {'schema': COMPACT_REVIEW, 'requirements': {}, 'claims': {}, 'returnClaims': {},
              'unresolved': list(dict.fromkeys(limit for key in slots for limit in values[key]['limitations'])),
              'visual': {'status': 'UNAVAILABLE', 'reason': 'Isolated authors work from retained source packets.'}}
    labels, kinds = LABELS['en'], LABELS['en']['entryKinds']
    if job['language'] == 'ru':
        locale = load(Path(job['renderer']) / 'assets/ru.json')
        labels = locale['documentationPages']
        kinds = {kind: locale['kind_' + kind] for kind in kinds}
    navigation_overview = plan.get('navigationOverview')
    entries_by_id = {row['id']: row for row in plan['entries']}
    titles = start_titles(navigation)
    start_sources = {}
    for nav in navigation['services']:
        service = nav['id']
        owned = [slot for slot in slots.values() if slot['service'] == service]
        guide = next((slot for slot in owned if slot['kind'] == 'GUIDE'), None)
        contracts = next(slot for slot in owned if slot['kind'] == 'CONTRACTS')
        contract_response = values[contracts['key']]
        guides_links = '- ' + page_link(labels['contracts'], contracts['page']) + '\n- ' + page_link(
            labels['process'], plan['processPage'])
        authored = values[guide['key']] if guide else None
        uses_cited_links = navigation_overview and (not authored or not authored['claims'])
        if navigation_overview:
            guide_claims = authored['claims'] if authored else []
            if uses_cited_links:
                alias = contract_response['claims'][0]['cites'][0]
                guide_claims = [{'heading': '', 'text': guides_links, 'cites': [alias],
                                 'assessment': 'SUPPORTED', 'returns': []}]
            overview_value = {'title': nav['title'], 'claims': guide_claims, 'disposition': 'EXPLAINED'}
        else:
            overview_value = {key: authored[key] for key in ('title', 'claims', 'disposition')}
        operations = [slot for slot in owned if slot['kind'] == 'OPERATION']
        discovery = discovery_slots(plan, service)
        entries = {slot['key']: slot['requirement'] for slot in operations}
        files = {ref: slot['requirement'] for ref, slot in discovery.items()}
        selected = {'entries': entries, 'files': files,
                    'citations': {ref: handle for slot in owned for ref, handle in slot['citations'].items()},
                    'returns': {}}
        responses = []
        for slot in operations:
            row = values[slot['key']]
            claims = []
            for claim in row['claims']:
                refs = {ref: slot['key'] + '-' + ref for ref in claim['returns']}
                selected['returns'].update({full: slot['returns'][ref] for ref, full in refs.items()})
                claims.append({**claim, 'returns': list(refs.values())})
            responses.append({'ref': slot['key'], 'claims': claims,
                              **{key: row[key] for key in ('disposition', 'participants', 'steps')}})
        required = {row['page'] for row in inventory['requirements'] if row['service'] == service}
        scope = {'services': {service: inventory['services'][service]},
                 'requirements': [row for row in inventory['requirements'] if row['service'] == service]}
        value = {'status': 'DOCUMENTED', 'reason': '', 'reviewed': True,
                 'overview': overview_value,
                 'entries': responses,
                 'discovery': [{'ref': ref, **{key: values[slot['key']][key]
                                              for key in ('disposition', 'discovery', 'starts')}}
                               for ref, slot in discovery.items()],
                 'unresolved': [], 'visual': review['visual']}
        page_set = {'pages': [
            {**page, 'fragments': page['fragments'][:1]} if page['page'] == nav['overview'] else page
            for page in starter['pages'] if page['page'] in required]}
        local_navigation = {'services': [nav], 'home': {'title': nav['title']}}
        artifacts = inline_assemble(value, selected, scope, page_set, local_navigation)
        original['pages'].extend(artifacts['original.json']['pages'])
        local = artifacts['self-review.json']
        for group in ('requirements', 'claims', 'returnClaims'):
            review[group].update(local[group])
        overview = next(page for page in original['pages'] if page['page'] == nav['overview'])
        add_contracts(original, review, nav, contracts, contract_response, labels)
        links = {'text': '\n' + guides_links + '\n\n'}
        if not uses_cited_links:
            overview['fragments'].append(links)
        if navigation_overview:
            append_service_starts(overview, nav, labels, kinds)
        by_page = {slot['page']: slot for slot in operations}
        starts = {start['target'].partition('#')[0] for start in nav['starts']}
        for page in artifacts['original.json']['pages']:
            if slot := by_page.get(page['page']):
                if job['language'] == 'en':
                    OUTCOMES['append'](page, review, slot, values[slot['key']]['claims'],
                                       labels['operationOutcomes'], citation_handles)
                if related := slot.get('relatedOperations'):
                    refs = [entries_by_id[ref] for ref in related]
                    page['fragments'].append({'text': '\n## ' + labels['relatedOperations'] + '\n\n' + '\n'.join(
                        '- ' + page_link(titles.get(ref['page'], ref['title']), ref['page'], 'behavior')
                        for ref in refs) + '\n\n'})
                page['fragments'].append(links)
            if plan.get('operationOwnership') and page['page'] in starts:
                operation_anchors(page)
                if slot:
                    source = slot.get('startCitation') or next(iter(slot['citations'].values()))
                else:
                    source = next(part['evidence'][0] for part in page['fragments'] if 'evidence' in part)
                start_sources[page['page']] = source
    navigation['home']['title'] = navigation['services'][0]['title']
    process = next((slot for slot in slots.values() if slot['kind'] == 'PROCESS_MAP'), None)
    response = values[process['key']] if process else {'claims': [], 'routes': []}
    if plan.get('operationOwnership') and not start_sources:
        kind = 'CONTRACTS' if navigation_overview else 'GUIDE'
        guide = next(slot for slot in slots.values() if slot['kind'] == kind)
        start_sources[guide['page']] = next(iter(guide['citations'].values()))
    by_page = {slot['page']: values[slot['key']]['steps'] for slot in slots.values() if slot['kind'] == 'OPERATION'}
    operation_steps = {row['id']: by_page[row['page']] for row in plan['entries'] if row['page'] in by_page}
    slots_by_page = {slot['page']: slot for slot in slots.values() if slot['kind'] == 'OPERATION'}
    operation_values = {entry['id']: values[slot['key']] for entry in plan['entries']
                        if (slot := slots_by_page.get(entry['page']))}
    catalog = add_processes(original, review, navigation, plan, process, response, job, labels, cite_services,
                            operation_steps, start_sources, operation_values, historical_routes)
    need(len(original['pages']) <= plan['maximumPages'], 'PAGE_LIMIT_AFTER_DISCOVERY', page=plan['processPage'],
         count=len(original['pages']), limit=plan['maximumPages'])
    return {'original.json': original, 'navigation.json': navigation, 'self-review.json': review, 'routes.json': catalog}
