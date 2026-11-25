"""Join prepared declaration facts to source-proven operation scopes."""

from collections import defaultdict, deque
import hashlib

MAX_REACHED = 32
EXACT_RESOLUTIONS = {'SAME_SCOPE_UNIQUE', 'SAME_SCOPE_SELECTED_DECLARATION'}
EXACT_AUTHORITIES = {'JAVAC_SELECTED_DECLARATION_NOT_CONTROL_FLOW',
                     'RETAINED_K2_COMPILER_SELECTED_SYMBOL_NOT_RUNTIME_DISPATCH'}
CONDITIONAL = 'If control reaches the cited call or callback and enters this retained declaration'
DISPATCH_OPEN = 'Source selection does not establish runtime dispatch, invocation or completion.'


def key(row):
    return row['service'], row['revision'], row['file']


def location(row):
    occurrence = row.get('occurrence') or {}
    return (*key(row), row['startLine'], row['endLine'], row['textDigest'],
            row.get('startByte', occurrence.get('startByte')), row.get('endByte', occurrence.get('endByte')),
            occurrence.get('snapshot'), occurrence.get('blob'))


def digest(raw):
    return 'sha256:' + hashlib.sha256(raw).hexdigest()


def relative_slice(container, site):
    """Prove exact bytes at their indicated lines, including older line-only captures."""
    if key(container) != key(site):
        return None
    whole, part = container['text'].encode(), site['text'].encode()
    if not part or site['startLine'] < container['startLine'] or site['endLine'] > container['endLine']:
        return None
    expected, observed = container.get('occurrence') or {}, site.get('occurrence') or {}
    if observed and expected and any(expected.get(field) != observed.get(field) for field in ('snapshot', 'blob')):
        return None
    start, offset = container.get('startByte'), site.get('startByte')
    if isinstance(start, int) and isinstance(offset, int):
        lower = offset - start
    else:
        lower = whole.find(part)
        if lower >= 0:
            candidate = lower
            while candidate >= 0 and container['startLine'] + whole.count(b'\n', 0, candidate) < site['startLine']:
                candidate = whole.find(part, candidate + 1)
            lower = candidate
            following = whole.find(part, lower + 1) if lower >= 0 else -1
            if following >= 0 and container['startLine'] + whole.count(b'\n', 0, following) == site['startLine']:
                return None
    upper = lower + len(part)
    if (0 <= lower < upper <= len(whole) and whole[lower:upper] == part
            and container['startLine'] + whole.count(b'\n', 0, lower) == site['startLine']
            and site['startLine'] + part.removesuffix(b'\n').count(b'\n') == site['endLine']):
        return lower, upper
    return None


def witness(container, site, handle):
    if (selected := relative_slice(container, site)) is None:
        return None
    return {'cite': handle, **{field: site[field] for field in ('file', 'revision', 'startLine', 'endLine', 'textDigest')},
            'sliceStart': selected[0], 'sliceEnd': selected[1],
            **{field: site[field] for field in ('startByte', 'endByte') if type(site.get(field)) is int}}


def excerpt(source, proof):
    """Resolve a display slice against the original cited source, never a file locator."""
    if any(source[field] != proof[field] for field in ('file', 'revision')):
        return None
    lower, upper = proof.get('sliceStart'), proof.get('sliceEnd')
    origin = (source.get('occurrence') or {}).get('startByte', source.get('startByte'))
    if lower is None and all(type(value) is int for value in (origin, proof.get('startByte'), proof.get('endByte'))):
        lower, upper = proof['startByte'] - origin, proof['endByte'] - origin
    whole = source['text'].encode()
    if type(lower) is not int or type(upper) is not int or not 0 <= lower < upper <= len(whole):
        return None
    selected = whole[lower:upper]
    if digest(selected) != proof['textDigest']:
        return None
    first = source['startLine'] + whole.count(b'\n', 0, lower)
    last = first + selected.removesuffix(b'\n').count(b'\n')
    if (first, last) != (proof['startLine'], proof['endLine']):
        return None
    try:
        return {**source, 'text': selected.decode(), 'textDigest': proof['textDigest'], 'startLine': first, 'endLine': last,
                'startByte': origin + lower if type(origin) is int else None,
                'endByte': origin + upper if type(origin) is int else None}
    except UnicodeError:
        return None


def body_index(process):
    sources = process['sources']
    candidates = defaultdict(dict)
    for fact in process['facts']:
        owner = fact.get('declaration')
        if not owner or fact['kind'] != 'BODY':
            continue
        for ref in fact['sources']:
            source = sources.get(ref)
            if source is not None:
                candidates[owner][location(source)] = source
    return {owner: next(iter(rows.values())) for owner, rows in candidates.items() if len(rows) == 1}


def compiler_calls(process):
    """Use exact declaration IDs paired with a source-bound call site."""
    rows = process['facts']
    events = defaultdict(list)
    for row in rows:
        if row['kind'] == 'EVENT' and row.get('event') in ('CALL', 'CONSTRUCT') and row.get('resolution') == 'COMPILER_EXACT':
            events[row.get('occurrence')].append(row)
    for row in rows:
        if row['kind'] != 'CALL' or row.get('resolution') not in EXACT_RESOLUTIONS:
            continue
        owner, target = row.get('declaration'), row.get('targetMethodId')
        if not owner or not target:
            continue
        authority = row.get('relationshipAuthority')
        if authority in EXACT_AUTHORITIES:
            refs = row['sources']
        elif authority is None and row.get('eventId'):
            matches = [event for event in events[row['eventId'].removeprefix('EVENT:')]
                       if event.get('target') and event['target'] == row.get('target') and event['declaration'] == owner]
            refs = matches[0]['sources'] if len(matches) == 1 else []
        else:
            continue
        for ref in refs:
            if ref in process['sources']:
                yield owner, target, process['sources'][ref]


def source_calls(process, bodies):
    sources = process['sources']
    # These offsets come from the compiler/syntax registration adapters, not
    # from declaration spelling or from a source name candidate.
    for call, target in process.get('_background', {}).get('calls', ()):
        site = sources.get(call)
        if site is None or target not in bodies:
            continue
        owners = [owner for owner, source in bodies.items() if relative_slice(source, site) is not None]
        if len(owners) == 1:
            yield owners[0], target, site


def reached(process, bodies, bind):
    root = process['root']
    if root not in bodies or not (cite := bind(bodies[root])):
        return {}, 0
    links = defaultdict(dict)
    for owner, target, site in (*compiler_calls(process), *source_calls(process, bodies)):
        if owner not in bodies or target not in bodies or relative_slice(bodies[owner], site) is None:
            continue
        if (site_cite := bind(site)) and (target_cite := bind(bodies[target])):
            identity = target, key(site), site['startLine'], site.get('startByte'), site['textDigest']
            links[owner][identity] = (target, site, site_cite, target_cite)
    selected, queue = {root: (bodies[root], cite, ())}, deque([root])
    limited = set()
    while queue:
        owner = queue.popleft()
        trail = selected[owner][2]
        for target, site, site_cite, target_cite in links[owner].values():
            if target in selected:
                continue
            if len(selected) >= MAX_REACHED:
                limited.add(target)
                continue
            selected[target] = bodies[target], target_cite, (*trail, (site, site_cite))
            queue.append(target)
    return selected, len(limited)


def select(process, discovery, exact, bind):
    """Keep declaration-local evidence only from this operation's reached bodies."""
    bodies = body_index(process)
    targets, limited = reached(process, bodies, bind)
    other = defaultdict(list)
    for owner, body in bodies.items():
        if owner not in targets:
            other[key(body)].append(body)
    result, seen = [], set()
    for declaration, (body, body_cite, trail) in targets.items():
        for fact in discovery.get(key(body), ()):
            proofs = []
            localized = False
            for cite in fact['sources']:
                source = exact[cite]
                supplied = [proof for proof in fact.get('witnesses', ()) if proof['cite'] == cite]
                spans = [excerpt(source, proof) for proof in supplied] if supplied else [source]
                if not spans or any(span is None or relative_slice(body, span) is None
                                    or any(relative_slice(contender, span) is not None for contender in other[key(body)])
                                    for span in spans):
                    break
                proofs.extend(witness(source, span, cite) for span in spans)
                localized |= bool(supplied) or any(span['textDigest'] != body['textDigest'] for span in spans)
            else:
                if not localized or not all(proofs):
                    continue
                identity = fact['kind'], fact['text'], fact.get('condition'), tuple(
                    (proof['file'], proof['startLine'], proof['endLine'], proof['textDigest']) for proof in proofs)
                if identity in seen:
                    continue
                trail_evidence = [witness(exact[cite], site, cite) for site, cite in trail]
                declaration_proof = witness(exact[body_cite], body, body_cite)
                if not declaration_proof or not all(trail_evidence + proofs):
                    continue
                seen.add(identity)
                witnesses = [*trail_evidence, declaration_proof, *proofs]
                condition = fact.get('condition')
                if trail:
                    condition = CONDITIONAL + (f'. Additional condition: {condition}' if condition else '.')
                uncertainty = ' '.join(filter(None, (fact.get('uncertainty'), DISPATCH_OPEN if trail else None)))
                result.append({**fact, 'sources': list(dict.fromkeys(proof['cite'] for proof in witnesses)),
                               'witnesses': witnesses, '_reachedDeclaration': declaration,
                               **({'condition': condition} if condition else {}),
                               **({'uncertainty': uncertainty} if uncertainty else {})})
    return result, limited


def index_discovery(rows, exact):
    indexed = defaultdict(list)
    for row in rows:
        for fact in row:
            for scope in dict.fromkeys(key(exact[cite]) for cite in fact['sources']):
                indexed[scope].append(fact)
    return indexed
