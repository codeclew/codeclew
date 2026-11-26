"""Project retained interactions and checked routes onto accepted operation pages."""
from collections import Counter, defaultdict
from itertools import zip_longest
import hashlib
from html import escape
import re


def identity(value):
    return hashlib.sha256(encode(value)).hexdigest()[:16]


def select_records(records, requirements, entries, sources):
    operations = {entry['id']: entry for entry in entries}
    handles = operation_handles(requirements)
    selected, omitted = {}, Counter()
    for row in records:
        hops = [{**hop, 'from': handles.get(hop['from']), 'to': handles.get(hop['to'])} for hop in row['hops']]
        occurrences = [hops[0]['from'], *(hop['to'] for hop in hops)]
        choice = row['selection']
        selected_owner = handles.get(choice['owner']) if choice else None
        if not set(occurrences) <= operations.keys() or choice and selected_owner not in occurrences:
            omitted['OPERATION_NOT_RETAINED'] += 1
            continue
        if (len(occurrences) != len(set(occurrences))
                or any(left['to'] != right['from'] for left, right in zip(hops, hops[1:]))):
            omitted['UNSUPPORTED_OCCURRENCE_PATH'] += 1
            continue
        if (not set(row['sources']) <= sources.keys()
                or not {operations[ref]['service'] for ref in occurrences}
                <= {sources[handle]['service'] for handle in row['sources']}):
            omitted['SOURCE_MEMBERSHIP_UNAVAILABLE'] += 1
            continue
        selection = {**choice, 'owner': selected_owner} if choice else None
        lead = hops[0]
        proof = (lead.get('sources') or (lead['site'],))[0]
        location = sources.get(proof)
        row = {**row, 'hops': hops, 'selection': selection}
        if location:
            row['location'] = f"{location['file'].rsplit('/', 1)[-1]}:{location['startLine']}"
        selected[identity(row)] = row
    return list(selected.values()), dict(omitted)


def operation_handles(requirements):
    return {row['id']: row['handlerId'] for row in requirements if row['kind'] == 'ENTRY'}


def select_boundaries(records, requirements, sources):
    handles = operation_handles(requirements)
    selected = {}
    for row in records:
        if row['owner'] in handles and set(row['sources']) <= sources.keys():
            bound = {**row, 'owner': handles[row['owner']]}
            selected[identity(bound)] = bound
    return list(selected.values())


def single_condition(selections):
    """Several retained facts do not imply a common execution path."""
    distinct = {(row['owner'], predicate(row['condition'])): row['condition']
                for row in selections if row.get('condition')}
    return next(iter(distinct.values())) if len(distinct) == 1 else None


def scoped_condition(selections, words):
    selected = single_condition(selections)
    if selected:
        return line(selected)
    return words['ownerConditions'] if any(row.get('condition') for row in selections) else words['conditional']


def trim(text, size):
    return text if len(text.encode('utf-16-le')) <= 2 * size else (
        text.encode('utf-16-le')[:2 * (size - 1)].decode('utf-16-le', 'ignore').rstrip() + '…')


def predicate(text):
    if not text:
        return ''
    # Match exact predicates and code spans, including an outer Markdown code span.
    value = text.strip()
    boolean = re.fullmatch(r'`([^`]+)` is (true|false)', value)
    if boolean:
        value = boolean[1] if boolean[2] == 'true' else '!(' + boolean[1] + ')'
    normalized = value.removeprefix('`').removesuffix('`')
    return ' '.join(normalized.split())


def owner_selection(selection, values, slots):
    """Give preference to an exact diagram step, then a cited claim naming the predicate."""
    ref = selection['owner']
    condition = predicate(selection.get('condition'))
    if not condition or ref not in values:
        return None
    value = values[ref]
    candidates = [step for step in value['steps'] if predicate(step['condition']) == condition]
    if candidates:
        kinds = ('return',) if selection.get('effect') in ('outcome', 'independent-response') else ('note', 'return')
        specific = [step for step in candidates if step['kind'] in kinds]
        selected = specific or candidates
        if len(selected) == 1:
            step, = selected
            return {'anchor': 'sequence', 'label': step['label'], 'condition': step['condition'], 'kind': step['kind']}
    claims = []
    for claim in value['claims']:
        cited = {slots[ref]['citations'][alias] for alias in claim['cites']}
        spans = re.findall(r'`([^`\n]+)`', claim['text'])
        if (claim['assessment'] == 'SUPPORTED' and cited.intersection(selection['sources'])
                and condition in map(predicate, spans)):
            claims.append({'anchor': 'behavior', 'label': claim['heading'],
                           'condition': next(span for span in spans if predicate(span) == condition)})
    return claims[0] if len(claims) == 1 else None


def route_path(record):
    return [{key: hop[key] for key in ('from', 'to', 'protocol', 'qualifier', 'site')} for hop in record['hops']]


def route_key(path):
    return 'route-' + identity(path)


def retained_records(plan, history):
    return [*plan.get('routeRecords', ()), *(item['record'] for item in history if 'record' in item)]


def group_routes(records):
    groups = {}
    for row in records:
        path = route_path(row)
        key = route_key(path)
        group = groups.setdefault(key, {'id': key, 'path': path, 'branches': {}})
        conditions = [{'owner': hop['from'], 'condition': hop['condition'],
                       'sources': hop.get('sources', row['sources'])} for hop in row['hops'] if hop['condition']]
        if selection := row['selection']:
            if selection['condition']:
                conditions = [cond for cond in conditions if (
                    cond['owner'], predicate(cond['condition'])) != (
                    selection['owner'], predicate(selection['condition']))]
            conditions.append(selection)
        branch = {**row, 'conditions': conditions}
        signature = [[condition['owner'], predicate(condition['condition'])] for condition in conditions]
        key = identity([row['status'], signature, selection and [selection['effect'], selection['site'], selection.get('alternative')]])
        if key in group['branches']:
            prior = group['branches'][key]
            prior['sources'] = list(dict.fromkeys([*prior['sources'], *branch['sources']]))
        else:
            group['branches'][key] = branch
    return list(groups.values())


def historical_identity(row):
    return identity([row['origin'], row['title'], row['participants'], row['condition'], row['result'],
                     row['terminal'], row['boundary'], row['outcome'], row['steps']])


def historical_groups(records, taken):
    seen = set()
    for item in records:
        if 'route' not in item:
            continue
        row = item['route']
        identity = historical_identity(row)
        if identity in seen:
            continue
        seen.add(identity)
        key = item['id']
        if key in taken:
            key = 'route-' + identity
        taken.add(key)
        yield key, row


def line(text):
    return ' '.join(text.split())


def outcome_index(groups, catalog, operations, values, slots, plan, words):
    """Index distinct effects and their premises without repeating a route narrative."""
    sections = {kind: {} for kind in ('success', 'refusal', 'failure', 'handoff', 'outbound')}

    def link(label, name, anchor):
        href = escape(name.removesuffix('.md').removesuffix('.html') + '.html#' + anchor, quote=True)
        return f'<a href="{href}">{escape(label)}</a>'

    def offer(kind, ref, label, condition=None, route=None, anchor='sequence'):
        entry = operations[ref]
        caption = escape(label) + ' · ' + link(entry['title'], entry['page'], anchor)
        if condition:
            caption += ' · <code>' + escape(trim(line(condition), 160).strip('`')) + '</code>'
        if route:
            caption += ' · ' + link(words['routeDetail'], *route)
        sections[kind].setdefault((ref, label, condition), '<li>' + caption + '</li>')

    def result(ref, selection, route=None):
        status = selection.get('httpStatus', 0)
        effect = selection.get('effect')
        if 200 <= status < 300:
            kind = 'success'
        elif 400 <= status < 500:
            kind = 'refusal'
        elif 500 <= status < 600 or effect in ('error', 'exit-throw'):
            kind = 'failure'
        else:
            return
        own = owner_selection(selection, values, slots)
        if not own and status and not selection['condition']:
            candidates = [step for step in values[ref]['steps'] if step['kind'] == 'return' and not step['condition']
                          and re.search(rf'\bHTTP\s+{status}\b', step['label'])]
            if len(candidates) == 1:
                own = {**candidates[0], 'anchor': 'sequence'}
        prefix = f'HTTP {status}' if status else words['effects'][effect]
        label = prefix + (' · ' + trim(line(own['label']), 130) if own and own['label'] else '')
        offer(kind, ref, label, own['condition'] if own else selection['condition'], route,
              own['anchor'] if own else 'sequence')

    routes = {row['id']: row for row in catalog['routes']}
    for group in groups:
        row = routes[group['id']]
        narrative = row['narrative']['page']
        for branch_id, branch in group['branches'].items():
            if branch['status'] != 'SOURCE_QUALIFIED_CANDIDATE':
                continue
            if selection := branch['selection']:
                result(selection['owner'], selection, (narrative, row['id'] + '-outcome-' + branch_id))
            for hop in branch['hops']:
                if hop['protocol'] == 'message':
                    label = words['handoffTo'].format(operation=operations[hop['to']]['title'], message=hop['qualifier'])
                    offer('handoff', hop['from'], trim(label, 180), hop['condition'], (narrative, row['id']))
    for ref, slot in slots.items():
        for outcome in slot.get('mapOutcomes', ()):
            result(ref, {**outcome, 'owner': ref})
        for path in slot.get('conditionPaths', ()):
            if path.get('skip') and path.get('stepKind') == 'return':
                selection = {'owner': ref, 'condition': path['condition'], 'effect': 'exit-return',
                             'sources': [slot['citations'][alias] for alias in path['cites']]}
                own = owner_selection(selection, values, slots)
                if own:
                    offer('refusal', ref, trim(line(own['label']), 130), own['condition'], anchor=own['anchor'])
    for boundary in plan['routeBoundaries']:
        endpoint = trim(boundary['method'] + ' ' + boundary['endpoint'], 150)
        offer('outbound', boundary['owner'], endpoint + ' · ' + words['externalRequestLimit'], boundary['condition'])

    for kind, links in sections.items():
        if not links:
            continue
        by_owner = defaultdict(list)
        for (ref, _, _), item in links.items():
            by_owner[ref].append(item)
        rows = [link for batch in zip_longest(*by_owner.values()) for link in batch if link]
        yield '### ' + words['outcomeIndex'][kind] + '\n\n<ul>\n' + '\n'.join(rows[:8]) + '\n</ul>\n\n'
        if remaining := rows[8:]:
            yield ('<details>\n<summary>' + words['moreOutcomes'].format(count=len(remaining))
                   + '</summary>\n<ul>\n' + '\n'.join(remaining) + '\n</ul>\n</details>\n\n')


def render(original, review, page, catalog, target, operations, values, plan, history, labels,
           heading, page_link, append_claim):
    """Keep the cross-operation trace and link to the accepted local sequences."""
    words = labels['processRoutes']
    page_paths = {entry['page']: ref for ref, entry in operations.items()}
    slots = {page_paths[slot['page']]: slot for slot in plan['slots'] if slot['kind'] == 'OPERATION'}
    version = catalog['revision']
    groups = group_routes(retained_records(plan, history))
    historic = list(historical_groups(history, {group['id'] for group in groups}))
    anchored = plan.get('routeNarrativeLayout') == 'anchored'
    presentation = plan.get('routePresentation')
    owner_scoped = presentation in ('owner-scoped', 'owner-scoped-v2')
    direct_links = presentation == 'owner-scoped-v2'
    outcome_first = direct_links and plan.get('processOutcomes')
    if anchored:
        catalog['narrativeLayout'] = 'anchored'
    if owner_scoped:
        catalog['compactOverview'] = True
    retained = len(groups) + len(historic)
    need(anchored or len(original['pages']) + retained <= plan['maximumPages'], 'PAGE_LIMIT_AFTER_DISCOVERY', page=page['page'],
         count=len(original['pages']) + retained, limit=plan['maximumPages'],
         retainedRoutePages=len(groups), historicalRoutePages=len(historic))
    if omitted := sum(plan['routeRecordOmissions'].values()):
        page['fragments'].append({'text': words['omitted'].format(count=omitted) + '\n\n'})
    if groups or historic:
        page['fragments'].append({'text': '## ' + words['routes'] + '\n\n' + words['intro'] + '\n\n'})

    landing_pages = {}
    services = {entry['id']: entry for entry in catalog['docsets'][0]['services']}
    route_origins = {group['path'][0]['from'] for group in groups} | {row['origin'] for _, row in historic}
    route_services = {operations[ref]['service'] for ref in route_origins}
    available = plan['maximumPages'] - len(original['pages'])

    def generated_title(path):
        refs = [path[0]['from'], *(hop['to'] for hop in path)]
        title = ' → '.join(trim(operations[ref]['title'], 52) for ref in refs)
        qualifier = path[0]['qualifier']
        if qualifier and qualifier != operations[refs[1]]['title']:
            title += ' · ' + trim(qualifier, 44)
        return trim(title, 160)

    def route_titles():
        descriptions = []
        for group in groups:
            evidence = list(group['branches'].values())
            condition = single_condition([cond for branch in evidence for cond in branch['conditions']])
            location = next((branch['location'] for branch in evidence if branch.get('location')), None)
            descriptions.append((group['id'], generated_title(group['path']), condition or location))
        for key, row in historic:
            selection = {'owner': row['origin'], 'condition': row['condition'], 'sources': row['cites']}
            own = owner_selection(selection, values, slots)
            if not own and row['terminal'] is not None:
                own = owner_selection({**selection, 'owner': row['terminal']}, values, slots)
            descriptions.append((key, row['title'], own['condition'] if own else None))
        if not owner_scoped:
            return {key: title for key, title, _ in descriptions}
        counts = Counter(title for _, title, _ in descriptions)
        names = {}
        for key, title, detail in descriptions:
            if counts[title] > 1 and detail:
                detail = trim(line(detail), 55)
                title = trim(title, 156 - len(detail.encode('utf-16-le')) // 2) + ' · ' + detail
            names[key] = title
        collisions = Counter(names.values())
        return {key: title if collisions[title] == 1 else
                trim(title, 130) + ' · ' + words['routeReference'].format(reference=key[-8:])
                for key, title in names.items()}

    display_titles = route_titles()

    def landing(service):
        # Use stable service pages while there is room. A full docset can still
        # host the same checked sections directly under its process catalog.
        name = 'process-routes-' + identity(service) + '.md'
        if len(route_services) > available:
            name = page['page']
        if name not in landing_pages:
            if name == page['page']:
                landing_pages[name] = page
            else:
                need(not any(item['page'] == name for item in original['pages']), 'PAGE_DESTINATION_COLLISION', page=name)
                label = words['serviceRoutes'].format(service=services[service]['title'])
                landing_pages[name] = heading(name, escape(label, quote=False))
                original['pages'].append(landing_pages[name])
        return landing_pages[name]

    def add_route(key, title, refs, summary, cited):
        refs = list(dict.fromkeys(refs))
        origin = refs[0]
        if anchored:
            narrative = landing(operations[origin]['service'])
            name = narrative['page']
            anchor = key
        else:
            name = key + '.md'
            need(not any(item['page'] == name for item in original['pages']), 'PAGE_DESTINATION_COLLISION', page=name)
            narrative = heading(name, escape(title, quote=False), 'scope')
            original['pages'].append(narrative)
            page['fragments'].append({'text': '- ' + page_link(title, name) + '\n'})
            anchor = None
        if anchored:
            narrative['fragments'].append({'text': '\n<a id="' + key + '"></a>\n\n## ' + escape(title, quote=False) + '\n\n'})
        append_claim(narrative, review, summary, cited)
        level = '### ' if anchored else '## '
        narrative['fragments'].append({'text': level + words['operations'] + '\n\n' + '\n'.join(
            '- ' + page_link(words['behavior'].format(operation=operations[ref]['title']), operations[ref]['page'], 'behavior')
            + ' · ' + page_link(words['sequence'].format(operation=operations[ref]['title']), operations[ref]['page'], 'sequence')
            for ref in refs) + '\n\n'})
        row = {'id': key, 'version': version, 'title': title, 'narrative': target(name, anchor),
               'origin': {'status': 'known', 'occurrence': origin, 'kind': operations[origin]['kind'],
                          'title': trim(operations[origin]['title'], 160), 'summary': trim(summary, 512)},
               'occurrences': [{'id': ref, 'actor': {'kind': 'service', 'docset': 'retained',
                                                     'service': operations[ref]['service']},
                                'role': trim(operations[ref]['title'], 512),
                                'target': target(operations[ref]['page'], 'handler')} for ref in refs],
               'branches': [], 'outcomes': [],
               'coverage': {'outcomeInventory': 'partial', 'scope': trim(summary, 512), 'reference': target(name, anchor or 'scope')}}
        catalog['routes'].append(row)
        return narrative, row

    def steps_fragment(narrative, title, refs, steps, boundary=None):
        participants = [{'id': 'caller', 'title': labels['caller']},
                        *({'id': ref, 'title': trim(operations[ref]['title'], 240),
                           'target': operations[ref]['page'] + '#sequence'} for ref in refs)]
        if boundary:
            participants.append({'id': 'unresolved-destination', 'title': words['unresolvedDestination']})
            steps = [*steps, {'from': refs[0], 'to': 'unresolved-destination', 'kind': 'call',
                              'label': trim(boundary['method'] + ' ' + boundary['endpoint'], 240),
                              'target': operations[refs[0]]['page'] + '#sequence',
                              'condition': words['externalRequestLimit']}]
        diagram = {'title': title,
                   'participants': participants,
                   'steps': [{'from': 'caller', 'to': refs[0], 'label': trim(operations[refs[0]]['title'], 240),
                              'kind': 'call' if operations[refs[0]]['kind'] == 'http' else 'async'},
                             *steps]}
        narrative['fragments'].append({'text': '```clew-sequence\n' + encode(diagram).decode() + '```\n\n'})

    def local_links(selections):
        parts = []
        if owner_scoped and not single_condition(selections):
            refs = dict.fromkeys(select['owner'] for select in selections if select.get('condition'))
            return [page_link(words['behavior'].format(operation=operations[ref]['title']),
                              operations[ref]['page'], 'behavior') for ref in refs]
        for select in selections:
            ref = select['owner']
            entry = operations[ref]
            own = owner_selection(select, values, slots)
            if own:
                label = entry['title'] + (' · ' + line(own['label']) if own['label'] else '')
                parts.append(page_link(trim(label, 260), entry['page'], own['anchor'])
                             + ' · `' + own['condition'].strip('`').replace('`', '\\`') + '`')
            elif select.get('condition'):
                parts.append(words['retainedPredicate'].format(
                    condition='`' + line(select['condition']).strip('`').replace('`', '\\`') + '`')
                    + ' · ' + page_link(words['behavior'].format(operation=entry['title']), entry['page'], 'behavior'))
        return list(dict.fromkeys(parts))

    def add_branch(narrative, row, branch, label, condition, selections, outcome, kind, end, intermediates, cited):
        name = narrative['page']
        prefix = row['id'] + '-' if anchored else ''
        anchor = prefix + 'branch-' + branch
        outcome_anchor = prefix + 'outcome-' + branch
        narrative['fragments'].append({'text': ('#### ' if anchored else '### ') + escape(label, quote=False) + '\n\n<a id="' + anchor + '"></a>\n\n'})
        links = local_links(selections)
        append_claim(narrative, review, escape(condition, quote=False) + ('\n\n' + '\n'.join('- ' + link for link in links) if links else ''),
                     cited)
        narrative['fragments'].append({'text': '<a id="' + outcome_anchor + '"></a>\n\n'})
        append_claim(narrative, review, outcome, cited, 'EXPLICIT_UNKNOWN' if kind == 'unknown' else 'SUPPORTED')
        row['branches'].append({'id': anchor, 'title': trim(label, 160), 'condition': trim(condition, 512),
                                'target': target(name, anchor)})
        row['outcomes'].append({'id': outcome_anchor, 'branch': anchor, 'kind': kind,
                                'title': trim(label, 160), 'target': target(name, outcome_anchor), 'end': end,
                                'internal': {'coverage': 'partial', 'occurrences': intermediates}})

    for group in groups:
        path = group['path']
        refs = [path[0]['from'], *(hop['to'] for hop in path)]
        title = display_titles[group['id']]
        protocols = list(dict.fromkeys(hop['protocol'] for hop in path))
        summary = ' '.join(words['protocols'][protocol]['scope'] for protocol in protocols)
        branches = group['branches']
        statuses = {branch['status'] for branch in branches.values()}
        if statuses == {'DISABLED_IN_SOURCE'}:
            summary += ' ' + words['disabled']
        elif 'POSSIBLE' in statuses:
            summary += ' ' + words['possible']
        cited = list(dict.fromkeys(source for branch in branches.values() for source in branch['sources']))
        narrative, row = add_route(group['id'], title, refs, summary, cited)
        steps = []
        for hop in path:
            protocol = words['protocols'][hop['protocol']]
            entry = operations[hop['to']]
            steps.append({'from': hop['from'], 'to': hop['to'],
                          'label': trim(protocol['label'].format(qualifier=hop['qualifier']).rstrip(' ·'), 240),
                          'kind': {'http': 'call', 'message': 'async', 'storage': 'note', 'poll': 'note'}[hop['protocol']],
                          'target': entry['page'] + '#sequence',
                          **({'independent': True} if hop['protocol'] in ('poll', 'storage') else {}),
                          'condition': words['disabled'] if statuses == {'DISABLED_IN_SOURCE'} else protocol['condition']})
        steps_fragment(narrative, title, refs, steps)
        alternatives = list(branches.items())
        if len(alternatives) > 1:
            alternatives = [(key, branch) for key, branch in alternatives if branch['selection'] or branch['conditions']]
        narrative['fragments'].append({'text': ('### ' if anchored else '## ') + words['alternatives'] + '\n\n'})
        for branch_id, branch in alternatives:
            selection = branch['selection']
            effect = selection['effect'] if selection else 'unresolved'
            label = words['operationPath'] if owner_scoped and effect == 'write' else words['effects'][effect]
            status = selection.get('httpStatus', 0) if selection else 0
            if status:
                label += f' · HTTP {status}'
            condition = (scoped_condition(branch['conditions'], words) if owner_scoped else
                         ' · '.join(line(cond['condition']) for cond in branch['conditions'] if cond.get('condition')))
            if branch['status'] == 'DISABLED_IN_SOURCE':
                condition = words['disabled']
            elif branch['status'] != 'SOURCE_QUALIFIED_CANDIDATE':
                condition = (words['possible'] if owner_scoped else
                             words['conditional'] + (' · ' + condition if condition else ''))
            if selection:
                ref = selection['owner']
                own = owner_selection(selection, values, slots)
                entry = operations[ref]
                outcome = page_link(trim(line(own['label']), 240) if own and own['label'] else
                                    words['behavior'].format(operation=entry['title']), entry['page'],
                                    own['anchor'] if own else 'behavior')
                if effect == 'independent-response':
                    outcome += '\n\n' + words['independentResponse']
                elif effect == 'error':
                    outcome += '\n\n' + words['callerError']
                if own and own['label']:
                    label += ' · ' + trim(line(own['label']), 110)
            else:
                outcome = ''
            boundary, kind = None, 'unknown'
            if effect in ('error', 'exit-throw'):
                boundary, kind = ('return' if status else 'stop'), 'error'
            elif effect == 'exit-return' or status:
                boundary = 'return'
                kind = 'error' if status >= 400 else 'normal' if 200 <= status < 300 else 'unknown'
            terminal = selection['owner'] if boundary else None
            if terminal not in (refs[0], refs[-1]) or effect == 'independent-response':
                # An intermediate local response does not terminate the retained continuation.
                terminal, kind = None, 'unknown'
            if not terminal or kind == 'unknown':
                outcome = (outcome + '\n\n' if outcome else '') + words['boundary']
            end = ({'status': 'known', 'ownerOccurrence': terminal, 'boundary': boundary,
                    'recipient': {'kind': 'unknown', 'reason': words['boundary']}}
                   if terminal else {'status': 'unknown', 'reason': words['boundary']})
            add_branch(narrative, row, branch_id, trim(label, 160), condition or words['conditional'],
                       branch['conditions'], outcome, kind, end, [ref for ref in refs[1:] if ref != terminal],
                       branch['sources'])
        narrative['fragments'].append({'text': '\n' + page_link(labels['process'], page['page']) + '\n'})

    for key, prior in historic:
        refs = [prior['origin'], *(peer['entry'] for peer in prior['participants'])]
        cited = prior['cites']
        title = display_titles[key]
        narrative, row = add_route(key, title, refs, words['historical'], cited)
        def prior_selection(ref, condition):
            selection = {'owner': ref, 'condition': condition, 'sources': cited}
            return selection, owner_selection(selection, values, slots)

        step_choices = []
        steps = []
        for step in prior['steps']:
            origin, peer, kind = (step[field] for field in ('from', 'to', 'kind'))
            if origin not in refs or peer not in refs or origin == peer or kind not in words['historicalSteps']:
                continue
            entry = operations[peer]
            selected, match = prior_selection(origin, step['condition'])
            label = words['historicalSteps'][kind].format(operation=entry['title'])
            steps.append({'from': origin, 'to': peer, 'kind': kind, 'label': label,
                          'target': entry['page'] + '#sequence', **({'condition': match['condition']} if match else {})})
            if match:
                step_choices.append(selected)
        selections = [selection for selection, match in (prior_selection(ref, prior['condition']) for ref in refs) if match]
        selections.extend(step_choices)
        boundary = None
        if owner_scoped:
            condition = scoped_condition(selections, words)
            if not single_condition(selections):
                steps = [{key: value for key, value in step.items() if key != 'condition'} for step in steps]
            if len(refs) == 1:
                candidates = [candidate for candidate in plan['routeBoundaries'] if candidate['owner'] == refs[0]
                              and set(candidate['sources']).intersection(cited)]
                endpoints = {(candidate['method'], candidate['endpoint']) for candidate in candidates}
                if len(endpoints) == 1:
                    boundary = candidates[0]
                    proof = list(dict.fromkeys(handle for candidate in candidates for handle in candidate['sources']))
                    if direct_links:
                        guards = [candidate for candidate in candidates if candidate.get('condition')]
                        if candidate := single_condition(guards):
                            selection = {'owner': refs[0], 'condition': candidate, 'sources': proof}
                            if owner_selection(selection, values, slots):
                                selections.append(selection)
                                condition = scoped_condition(selections, words)
                    append_claim(narrative, review, words['externalRequest'].format(
                        endpoint=escape(boundary['method'] + ' ' + boundary['endpoint'], quote=False)), proof)
                else:
                    append_claim(narrative, review, words['externalRequestUnknown'], cited, 'EXPLICIT_UNKNOWN')
        else:
            condition = ' · '.join(dict.fromkeys(selection['condition'] for selection in selections)) or words['conditional']
        steps_fragment(narrative, title, refs, steps, boundary)
        terminal = prior['terminal']
        outcome = words['boundary']
        if anchored and len(refs) == 1:
            entry = operations[refs[0]]
            outcome = words['externalBoundary'] + '\n\n' + page_link(
                words['behavior'].format(operation=entry['title']), entry['page'], 'behavior')
        terminal_selection = next((select for select in selections if select['owner'] == terminal), None)
        match = owner_selection(terminal_selection, values, slots) if terminal_selection else None
        if terminal:
            entry = operations[terminal]
            outcome = page_link(line(match['label']) if match and match['label'] else
                                words['behavior'].format(operation=entry['title']), entry['page'],
                                match['anchor'] if match else 'behavior')
        outcome += '\n\n' + page_link(labels['process'], page['page'])
        known_return = (match and match.get('kind') == prior['boundary'] == 'return'
                        and (not owner_scoped or single_condition(selections)))
        end = ({'status': 'known', 'ownerOccurrence': terminal, 'boundary': 'return',
                'recipient': {'kind': 'unknown', 'reason': words['boundary']}}
               if known_return else {'status': 'unknown', 'reason': words['boundary']})
        kind = prior['result'] if known_return else 'unknown'
        if terminal and not known_return:
            outcome = words['boundary'] + '\n\n' + outcome
        add_branch(narrative, row, 'retained', labels['routeOutcomes'][kind], condition, selections,
                   outcome, kind, end, [ref for ref in refs[1:] if not known_return or ref != terminal], cited)

    if anchored and catalog['routes']:
        by_origin = {}
        for row in catalog['routes']:
            by_origin.setdefault(row['origin']['occurrence'], []).append(row)
        sections = []
        historical_ids = {key for key, _ in historic}
        for ref, routes in by_origin.items():
            entry = operations[ref]
            start = page_link(entry['title'], entry['page'], 'behavior')
            def route_link(row):
                return '- ' + page_link(row['title'], row['narrative']['page'].removesuffix('.html') + '.md',
                                        row['narrative']['anchor'])
            if not owner_scoped:
                sections.append('### ' + start + '\n\n' + '\n'.join(map(route_link, routes)))
                continue
            sequence = page_link(words['sequence'].format(operation=entry['title']), entry['page'], 'sequence')
            start += ' · ' + sequence
            featured = [route_link(row) for row in routes if row['id'] in historical_ids]
            observed = [route_link(row) for row in routes if row['id'] not in historical_ids]
            body = '\n'.join(featured)
            if observed:
                summary = words['observedRoutes'].format(count=len(observed))
                body += ('\n\n' if body else '') + '<details>\n<summary>' + summary + '</summary>\n\n'
                if direct_links:
                    items = []
                    for row in routes:
                        if row['id'] in historical_ids:
                            continue
                        dest = row['narrative']
                        href = escape(dest['page'] + '#' + dest['anchor'], quote=True)
                        items.append('<li><a href="' + href + '">' + escape(row['title']) + '</a></li>')
                    body += '<ul>' + '\n'.join(items) + '</ul>\n</details>'
                else:
                    body += '\n'.join(observed) + '\n\n</details>'
            sections.append('### ' + start + '\n\n' + body)
        # Insert the overview before any narrative sections on this page.
        position = next(i for i, fragment in enumerate(page['fragments']) if words['intro'] in fragment['text']) + 1
        page['fragments'].insert(position, {'text': '\n\n'.join(sections) + '\n\n'})
    if outcome_first and catalog['routes']:
        page['fragments'].append({'text': '\n<!-- clew-process-catalog -->\n\n'})
    if outcome_first:
        visible = ''.join(outcome_index(groups, catalog, operations, values, slots, plan, words))
        if visible:
            page['fragments'].insert(1, {'text': '## ' + words['readerOutcomes'] + '\n\n' + visible})


    declarations = plan['routeDeclarations']
    if declarations:
        declaration_text = ['\n## ' + words['declarations'] + '\n\n' + words['declarationScope'] + '\n']
        for row in declarations:
            if row['origin'] == row['peer']:
                continue
            parts = [page_link(operations[ref]['title'], operations[ref]['page'], 'behavior')
                     for ref in (row['origin'], row['peer'])]
            declaration_text.append('- ' + row['title'] + ' · ' + ' → '.join(parts))
        page['fragments'].append({'text': '\n'.join(declaration_text) + '\n\n'})
