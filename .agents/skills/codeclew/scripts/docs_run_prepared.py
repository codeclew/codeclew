"""Prepare cited propositions from the same immutable Work used for authoring."""
from __future__ import annotations

from collections import Counter, defaultdict, deque
import hashlib
from itertools import chain, repeat
import json
import os
from pathlib import Path, PurePosixPath
import re
import runpy
import subprocess
import sys
import time


DIRECTORY = Path(__file__).resolve().parent
ADAPTERS = {
    'java.py': {'.java'},
    'managed_languages.py': {'.cs', '.kt', '.kts', '.ts', '.tsx', '.mts', '.cts', '.js', '.jsx', '.mjs', '.cjs'},
}
MAX_ROOTS = 32
MAX_REFS = 384
MAX_FACTS = 128
MAX_PACKET_BYTES = 256 * 1024
MAX_DISCOVERY_FACT_BYTES = 40 * 1024
MAX_SOURCE_VIEW_PREFIX_BYTES = 6 * 1024
MAX_FILE_BYTES = 128 * 1024
CSHARP_FILE_LIMIT = 12
FACT_SCHEMA = 'codeclew-docs-run-prepared-facts/1.0'
REACHED = runpy.run_path(str(DIRECTORY / 'docs_run_reached.py'))


def exact_sources(entries, packet, *, authorities=("EXACT_SNAPSHOT_TEXT",)):
    """Project only source bytes already admitted by the ordinary manifest reader."""
    originals = {row['source']['id']: row for row in packet['outcomes']}
    selected = {}
    for handle, entry in entries.items():
        source = entry['source']
        if source.get('authority') not in authorities:
            continue
        original = originals[source['id']]
        reference = original.get('sourceBinding', {}).get('reference')
        if not reference:
            reference = original.get('declaration', {}).get('selectedSourceReference')
        raw = packet['texts'][original['textKey']]
        if not reference or source.get('textDigest') != sha(raw.encode()):
            continue
        occurrence = source.get('occurrence') or {}
        selected[handle] = {**source, 'text': raw, 'references': [reference],
                            'startByte': occurrence.get('startByte'), 'endByte': occurrence.get('endByte')}
    return selected


def full_files(exact):
    """Reconstruct only bounded complete Git blobs from compatible Work occurrences."""
    groups = defaultdict(list)
    for row in exact.values():
        occurrence = row.get('occurrence') or {}
        blob, snapshot = occurrence.get('blob'), occurrence.get('snapshot')
        start, end = row.get('startByte'), row.get('endByte')
        if (isinstance(blob, str) and len(blob) in (40, 64) and snapshot
                and not occurrence.get('generatedOrigin') and type(start) is type(end) is int
                and 0 <= start < end <= MAX_FILE_BYTES):
            raw = row['text'].encode()
            if len(raw) == end - start:
                groups[(row['file'], row['revision'], snapshot, blob)].append((start, raw))
    files, ambiguous = {}, set()
    for (file, revision, snapshot, blob), spans in groups.items():
        raw = bytearray()
        for start, fragment in sorted(spans):
            if start > len(raw):
                break
            overlap = min(len(raw) - start, len(fragment))
            if raw[start:start + overlap] != fragment[:overlap]:
                break
            raw.extend(fragment[overlap:])
        else:
            git = hashlib.new('sha1' if len(blob) == 40 else 'sha256')
            git.update(('blob ' + str(len(raw)) + '\0').encode())
            git.update(raw)
            if git.hexdigest() == blob:
                if file in files:
                    ambiguous.add(file)
                files[file] = {'file': file, 'revision': revision, 'text': raw.decode(), 'textDigest': sha(raw),
                               'occurrence': {'snapshot': snapshot, 'blob': blob, 'startByte': 0, 'endByte': len(raw)},
                               'authority': 'RECONSTRUCTED_GIT_BLOB'}
    return {file: source for file, source in files.items() if file not in ambiguous}


def row_bindings(row):
    ids, refs = row['record'].get('sourceIds', []), row.get('sourceReferences', [])
    return dict(zip(ids, refs)) if len(ids) == len(refs) else {}


def operation(requirement, rows, exact, files):
    """Translate Work rows without promoting candidate targets to selected calls."""
    lookup = {ref: (handle, row) for handle, row in exact.items() for ref in row['references']}
    root = requirement['rootDeclarations'][0] if requirement['kind'] == 'ENTRY' else None
    entry_references = requirement.get('sourceReferences', [])
    all_refs, facts = set(entry_references), []
    file = requirement.get('file')
    if file:
        all_refs.update(ref for ref, (_, source) in lookup.items() if source['file'] == file)
    for row in rows:
        record = row['record']
        obligation = row['obligationKind']
        fact = record.get('event') or record.get('edge') or record.get('effect') or {}
        references = list(row_bindings(row).values())
        all_refs.update(references)
        sources = [ref for ref in references if ref in lookup]
        prepared = {key: fact[key] for key in (
            'ordinal', 'parentOrdinal', 'target', 'targetMethodId', 'relationshipAuthority', 'resolution',
            'ordering', 'targetStatus', 'accessMode', 'condition', 'eventId', 'http', 'kafka') if key in fact}
        prepared.update(kind=obligation, occurrence=row['id'].removeprefix('EVENT:'),
                        declaration=record.get('declaration') or fact.get('fromMethodId'),
                        event=fact.get('kind'), sources=sources)
        if obligation == 'BODY':
            prepared.update(_bodyStatus=record.get('bodyStatus'), _bodyAuthority=record.get('authority'),
                            _bodySourceIds=record.get('sourceIds', []))
        facts.append(prepared)
    occurrences = {ref: lookup[ref][1] for ref in sorted(all_refs & lookup.keys())}
    site_handles = sorted({lookup[ref][0] for ref in occurrences})
    site_fields = ('file', 'revision', 'startLine', 'endLine', 'text', 'textDigest', 'authority', 'startByte', 'endByte')
    sites = [{'id': handle, **{key: exact[handle].get(key) for key in site_fields},
              'references': [ref for ref in exact[handle]['references'] if ref in occurrences]}
             for handle in site_handles]
    entry_refs = [ref for ref in entry_references if ref in occurrences]
    process = {'id': requirement.get('entrypoint', requirement['id']), 'root': root, 'service': requirement['service'],
               'title': requirement['page'], 'entryKind': requirement.get('entryKind'),
               'entrySymbol': requirement.get('entrySymbol'), 'trigger': requirement.get('trigger'),
               'routes': [{'method': requirement['method'], 'path': requirement['path']}] if requirement.get('method') else [],
               'entrySources': entry_refs, 'facts': facts, 'sources': occurrences}
    context = {'operation': process['id'], 'sourceRanges': sites, '_sourceOccurrences': occurrences,
               '_nativeFiles': files, '_citationSources': exact,
               'file': file, 'entrySources': [lookup[ref][0] for ref in entry_refs]}
    return process, context


def extra_binding(witness, exact, read_pinned=None):
    """Bind a narrower witness only when its literal and coordinates occur in a retained cite."""
    raw = witness.get('text')
    if not isinstance(raw, str) or not raw or witness.get('textDigest') != sha(raw.encode()):
        return None
    start, end = witness.get('startByte'), witness.get('endByte')
    first, last = witness.get('startLine'), witness.get('endLine')
    if type(first) is not int or type(last) is not int or not 0 < first <= last:
        return None
    payload = raw.encode()
    matches = []
    for handle, row in exact.items():
        if (witness.get('file'), witness.get('revision')) != (row['file'], row['revision']):
            continue
        if occurrence := witness.get('occurrence'):
            retained = row.get('occurrence') or {}
            if any(occurrence.get(key) != retained.get(key) for key in ('snapshot', 'blob')):
                continue
        text = row['text'].encode()
        if (first, last, payload) == (row['startLine'], row['endLine'], text):
            matches.append((len(text), handle))
            continue
        origin = row.get('startByte')
        if type(start) is not int or type(end) is not int or end - start != len(payload):
            continue
        if type(origin) is not int and read_pinned:
            origin = locate_in_file(row, read_pinned)
        if type(origin) is not int:
            continue
        lower, upper = start - origin, end - origin
        if (0 <= lower < upper <= len(text) and text[lower:upper] == payload
                and row['startLine'] + text.count(b'\n', 0, lower) == first
                and first + payload.removesuffix(b'\n').count(b'\n') == last):
            matches.append((len(text), handle))
    return min(matches)[1] if matches else None


def locate_in_file(row, read_pinned):
    text = read_pinned(row['file'], row['revision'])
    if text is None:
        return None
    raw, fragment = text.encode(), row['text'].encode()
    line = row['startLine']
    start = 0
    for _ in range(line - 1):
        start = raw.find(b'\n', start) + 1
        if not start:
            return None
    end = raw.find(b'\n', start)
    end = len(raw) if end < 0 else end
    offset = raw.find(fragment, start)
    if start <= offset <= end and not start <= raw.find(fragment, offset + 1) <= end:
        return offset
    return None


def bound_facts(rows, lifted, exact, local, read_pinned=None):
    """Keep each proposition only if every citation remains usable."""
    bindings = {ref: handle for ref, source in lifted.items() if (handle := extra_binding(source, exact, read_pinned))}
    accepted, rejected, seen = [], 0, set()
    for row in rows:
        refs = row.get('sources', [])
        if not refs or any(ref not in local and ref not in bindings for ref in refs):
            rejected += 1
            continue
        if any(not isinstance(row.get(key), str) or not row[key].strip() for key in ('text', 'kind', 'authority')):
            rejected += 1
            continue
        # These fields are the accepted human-facing assertion and its scope.
        # Adapter-specific replay graphs and offsets stay outside the author packet.
        values = {key: row[key] for key in ('kind', 'authority', 'status', 'text', 'condition', 'uncertainty')
                  if row.get(key) is not None}
        for key in ('condition', 'uncertainty'):
            if isinstance(values.get(key), (list, tuple)):
                details = [value.get('text') if isinstance(value, dict) else value for value in values[key]]
                values[key] = ' '.join(value for value in details if isinstance(value, str) and value)
        if any(not isinstance(value, str) for value in values.values()):
            rejected += 1
            continue
        values['sources'] = list(dict.fromkeys(bindings.get(ref, ref) for ref in refs))
        witnesses = []
        repetition_sites = row.get('_repetitionSites', ())
        loop_identity = {}
        for ref in dict.fromkeys(refs):
            if ref not in bindings:
                continue
            handle = bindings[ref]
            source, site = exact[handle], lifted[ref]
            proof = REACHED['witness'](source, {**site, 'service': source['service']}, handle)
            if proof and all(type(proof.get(key)) is int for key in ('startByte', 'endByte')):
                if ref == row.get('_writeSite'):
                    values['_writeSite'] = proof
                if ref in repetition_sites:
                    origin = source.get('occurrence') or {}
                    loop_identity[ref] = (source['service'], source['file'], source['revision'],
                                          proof['startByte'], proof['endByte'], proof['textDigest'],
                                          origin.get('snapshot'), origin.get('blob'))
            witnesses.append(proof or dict(cite=handle, **{key: site[key] for key in (
                'file', 'revision', 'startLine', 'endLine', 'startByte', 'endByte', 'textDigest') if key in site}))
        if repetition_sites:
            if not all(ref in loop_identity for ref in repetition_sites):
                rejected += 1
                continue
            values['_repetitionKey'] = tuple(loop_identity[ref] for ref in repetition_sites)
        if witnesses:
            values['witnesses'] = witnesses
        for private in ('_edgeCause', '_edgePhase', '_writeFocus', '_pollPublication', '_httpStatus', '_writeBinding',
                        '_httpCompletion'):
            if private in row:
                values[private] = row[private]
        if edge := row.get('_edge'):
            site = bindings.get(edge['site'], edge['site'])
            if site in values['sources']:
                values['_edge'] = {**edge, 'site': site}
        key = digest(values)
        if key not in seen:
            accepted.append(values)
            seen.add(key)
    return accepted, rejected


def work_rows(job, scope, output, query, deadline):
    report = collect(job['clew'], job['root'], scope['work'], output, max_pages=8,
                     timeout=15, deadline=deadline, kind='work-selection', selection={'query': query})
    identity_scope = report.get('identity') or {}
    if (report['status'] == 'COMPLETE' and identity_scope.get('snapshot') == job['snapshot']
            and identity_scope.get('contextDigest') == scope['contextDigest']
            and identity_scope.get('inputDigest') == scope['inputDigest']):
        return load(output / 'items.json')
    return None


def csharp_units(job, scope, home, exact, deadline):
    units = []
    paths = sorted({source['file'] for source in exact.values() if source['file'].lower().endswith('.cs')})
    for number, file in enumerate(paths[:CSHARP_FILE_LIMIT], 1):
        if deadline - time.monotonic() <= 1:
            break
        rows = work_rows(job, scope, home / ('csharp-unit-' + str(number)),
                         {'kind': 'SOURCE_FILE', 'symbolContains': file}, deadline)
        units.extend(row for row in rows or () if csharp_unit_row(row, scope) and row['record']['symbol'] == file)
    return units


def csharp_unit_row(row, scope):
    record = row['record']
    fact = record.get('normalized', {})
    return (record.get('service') == scope['coverage']['id']
            and record.get('symbol') == fact.get('file')
            and fact.get('schema') == 'codeclew-csharp-compiler-fact/1.0'
            and fact.get('kind') == 'SOURCE_FILE' and fact.get('resolution') == 'SOURCE_MEMBERSHIP_EXACT'
            and len(row_bindings(row)) == 1)


def csharp_files(units, exact):
    """Admit original compiler inputs with independent typed CAS and raw byte checks."""
    sources = {ref: source for source in exact.values() for ref in source['references']}
    files, ambiguous = {}, set()
    for unit in units:
        fact = unit['record']['normalized']
        ref, = row_bindings(unit).values()
        source = sources.get(ref)
        if source is None:
            continue
        file, raw = fact['file'], source['text'].encode()
        occurrence = source.get('occurrence') or {}
        pin = fact.get('snapshotSource') or {}
        blob = pin.get('blob') or {}
        domain = b'codeclew-cas/v2\0codeclew-repository-input-blob/2.0\0'
        if not (0 < len(raw) <= MAX_FILE_BYTES and source['file'] == file and source['startLine'] == 1
                and fact.get('sourceContentDigest') == source['textDigest'] == sha(raw)
                and fact.get('sourceRange') == {'byteStart': 0, 'byteEnd': len(raw)}
                and blob.get('schema') == 'codeclew-cas-object/2.0'
                and blob.get('objectSchema') == 'codeclew-repository-input-blob/2.0'
                and blob.get('size') == len(raw) and blob.get('digest') == sha(domain + raw)
                and occurrence.get('blob') == blob['digest'] and occurrence.get('snapshot') == pin.get('snapshot')
                and occurrence.get('startByte') == 0 and occurrence.get('endByte') == len(raw)
                and not occurrence.get('generatedOrigin')):
            continue
        if file in files and files[file]['textDigest'] != source['textDigest']:
            ambiguous.add(file)
        files[file] = {key: source[key] for key in ('file', 'revision', 'text', 'textDigest', 'occurrence')}
        files[file]['authority'] = 'RETAINED_CSHARP_COMPILATION_UNIT'
    return {file: source for file, source in files.items() if file not in ambiguous}


def collect_contexts(job, scope, home, requirements, deadline):
    contexts, states = {}, {}
    for requirement in requirements:
        if requirement['kind'] != 'ENTRY':
            continue
        identity = requirement['id']
        roots = requirement['rootDeclarations']
        if len(roots) != 1:
            states[identity] = 'ENTRY_ROOT_UNAVAILABLE'
            continue
        root, = roots
        if root not in contexts:
            if len(contexts) >= MAX_ROOTS or deadline - time.monotonic() <= 1:
                states[identity] = 'PREPARATION_LIMIT'
                continue
            output = home / ('thread-' + str(len(contexts) + 1).zfill(2))
            contexts[root] = work_rows(job, scope, output,
                                       {'kind': 'THREAD_CONTEXT', 'symbolContains': root}, deadline)
        if contexts[root] is None:
            states[identity] = 'RETAINED_THREAD_INCOMPLETE'
    return contexts, states


def acquire_sources(job, service, scope, home, selections, exact, deadline, *, authorities=("EXACT_SNAPSHOT_TEXT",)):
    bound = {ref for row in exact.values() for ref in row['references']}
    wanted = {}
    for rows in selections:
        for row in rows or ():
            for identity, reference in row_bindings(row).items():
                if reference not in bound:
                    wanted.setdefault(identity, reference)

    def read_selected(selected, label):
        selection = home / (label + '-selection.json')
        save(selection, selected)
        directory = home / label
        manifest = directory / 'citations.json'
        argv = [sys.executable, '-I', '-S', str(DIRECTORY / 'evidence_packet.py'), 'read-sources',
                '--clew', job['clew'], '--root', job['root'], '--service', service, '--snapshot', job['snapshot'],
                '--work', scope['work'], '--selection', str(selection), '--output-dir', str(directory),
                '--output-format', 'metadata']
        try:
            command(argv, home, label, timeout=min(45, deadline - time.monotonic()))
            _, packet, _, entries = citation_load(manifest)
            need(packet['scope']['snapshot'] == job['snapshot'] and packet['work'] == scope['work']
                 and all(row['source']['service'] == service and row['source']['revision'] == scope['revision']
                         for row in entries.values()), 'PREPARED_FACT_SOURCE_SCOPE_CHANGED')
        except (Stop, CitationStop, OSError, ValueError):
            if manifest.is_file():
                manifest.rename(directory / 'unadmitted-citations.json')
            return {}, directory / 'packet.json'
        return {handle: {**source, 'manifest': manifest} for handle, source in exact_sources(entries, packet, authorities=authorities).items()}, None

    selected = [{'sourceId': key, 'sourceReference': value} for key, value in list(wanted.items())[:MAX_REFS]]
    if not selected or deadline - time.monotonic() <= 1:
        return {}
    acquired, incomplete = read_selected(selected, 'read-sources')
    if incomplete is None or not incomplete.is_file() or deadline - time.monotonic() <= 1:
        return acquired
    try:
        diagnostic = load(incomplete)
    except (OSError, Stop, ValueError):
        return acquired
    # Re-admit successful references through the native proof path. A partial
    # packet remains unadmitted and supplies only a bounded recovery selection.
    supplied = {item['sourceReference']: item for item in selected}
    proven = {}
    for row in diagnostic.get('outcomes', ()):
        selection = row.get('selection', {})
        reference = selection.get('sourceReference')
        if row.get('status') == 'RETAINED_SOURCE_RETURNED' and supplied.get(reference) == selection:
            proven[reference] = selection
    if not proven or len(proven) == len(selected):
        return acquired
    recovered, _ = read_selected(list(proven.values()), 'read-sources-recovered')
    return recovered


def local_repository(job, service, scope, home, deadline):
    # The Git locator is read from the CLI's registered, origin-checked binding.
    if deadline - time.monotonic() <= 1 or re.fullmatch(r'[0-9a-f]{40}|[0-9a-f]{64}', scope['revision']) is None:
        return None
    try:
        response = command([job['clew'], 'docs', 'service', 'show', '--root', job['root'], '--id', service,
                            '--local-binding'], home, 'local-binding', timeout=min(5, deadline - time.monotonic()))
    except Stop:
        return None
    row, binding = response.get('record', {}), response.get('localBinding', {})
    if row.get('id') != service or not row.get('repositoryId') or binding.get('repositoryId') != row['repositoryId']:
        return None
    locator = binding.get('repository')
    return locator if isinstance(locator, str) and Path(locator).is_absolute() and Path(locator).is_dir() else None


def pinned_file(repository, revision, file, deadline):
    if not repository or re.fullmatch(r'[0-9a-f]{40}|[0-9a-f]{64}', revision) is None:
        return None
    if not isinstance(file, str) or len(file) > 512 or any(ord(c) < 32 or c in ':\\' for c in file):
        return None
    path = PurePosixPath(file)
    if path.is_absolute() or '..' in path.parts or path.as_posix() != file:
        return None
    env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
    env.update(GIT_NO_LAZY_FETCH='1', GIT_TERMINAL_PROMPT='0')
    env['GIT_ALLOW_PROTOCOL'] = ''
    base = ['git', '--no-replace-objects', '-C', repository, 'cat-file']
    try:
        def git(*args):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise subprocess.TimeoutExpired(base, 0)
            return subprocess.run(base + list(args), check=True, capture_output=True,
                                  stdin=subprocess.DEVNULL, timeout=min(3, remaining), env=env).stdout
        if git('-t', revision) != b'commit\n':
            return None
        address = revision + ':' + file
        size = git('-s', address).strip()
        if not size.isdigit() or int(size) > MAX_FILE_BYTES:
            return None
        raw = git('blob', address)
        if len(raw) == int(size):
            return raw.decode()
    except (OSError, UnicodeError, subprocess.SubprocessError):
        return None
    return None


def is_boundary(fact):
    kind = fact['kind'].lower().replace('_', '-')
    return 'boundary' in kind or kind.endswith(('scope-limit', 'coverage-limit'))


def author_entries(entries, citations, witness_catalog=None):
    """Keep reader-facing fields and narrower line anchors without replay metadata."""
    result = {}
    for identity, row in entries.items():
        facts = defaultdict(list)
        for rank, fact in enumerate(row.get('propositions', ())):
            value = {key: item for key, item in fact.items() if key != 'witnesses'}
            lines = defaultdict(list)
            for witness in fact.get('witnesses', ()):
                handle = witness['cite']
                site = line_range(witness)
                if site != line_range(citations[handle]) and site not in lines[handle]:
                    lines[handle].append(site)
            if lines:
                value['lines'] = dict(lines)
            if witness_catalog is not None:
                value['_rank'] = rank
            if witnesses := fact.get('witnesses'):
                if witness_catalog is None:
                    value['_sourceWitnesses'] = witnesses
                else:
                    value['_sourceWitnessIds'] = [witness_catalog.setdefault(tuple(sorted(proof.items())), len(witness_catalog))
                                                  for proof in witnesses]
            facts['boundaries' if is_boundary(fact) else 'facts'].append(value)
        limits = row.get('scopeLimits', []) + row.get('projectionLimits', [])
        if limits:
            facts['factLimits'] = limits
        omitted, declarations, unbound = (row.get(key, 0) for key in (
            'omittedForLimit', 'omittedReachedDeclarations', 'unboundPropositions'))
        if omitted or declarations or unbound:
            facts['_preparationOmissions'] = {'facts': omitted, 'declarations': declarations, 'unbound': unbound,
                                             'reasons': row.get('factOmissions', {})}
        if facts:
            result[identity] = dict(facts)
    return result


def line_range(source):
    first, last = source['startLine'], source['endLine']
    return str(first) if first == last else f'{first}-{last}'


def single_line(value):
    return ' '.join(value.splitlines())


def workspace_sections(groups, citations, file, heading):
    if not any(groups.values()):
        return b''
    headings = {'facts': 'Facts', 'boundaries': 'Boundaries', 'factLimits': 'Coverage limits',
                '_preparationOmissions': 'Preparation omissions'}
    lines, sources = [heading, ''], {}
    for (labels, category), items in groups.items():
        if not items:
            continue
        label = f"{headings[category]} for {', '.join(labels)}"
        if category == 'factLimits':
            lines += [label, *('- ' + ', '.join(limits) for limits in items), '']
            continue
        if category == '_preparationOmissions':
            lines.append(label)
            for counts in items:
                for reason, count in counts['reasons'].items():
                    lines.append(f"- {count} facts exceeded the {reason.lower().replace('_', ' ')} limit.")
                if counts['facts'] and not counts['reasons']:
                    lines.append(f"- {counts['facts']} facts exceeded preparation limits.")
                if counts['declarations']:
                    lines.append(f"- {counts['declarations']} reached declarations exceeded the traversal limit.")
                if counts['unbound']:
                    lines.append(f"- {counts['unbound']} facts lacked usable source citations.")
            lines.append('')
            continue
        authority = items[0]['authority']
        common_authority = len(items) > 1 and all(fact['authority'] == authority for fact in items)
        lines.append(f'{label} [{authority}]' if common_authority else label)
        for fact in items:
            scope = [fact['kind']]
            if not common_authority:
                scope.append(fact['authority'])
            if status := fact.get('status'):
                scope.append(status)
            details = [single_line(fact['text'])]
            for key in ('condition', 'uncertainty'):
                if fact.get(key):
                    details.append(f'{key.title()} {single_line(fact[key])}')
            anchors = []
            for handle in fact['sources']:
                sources[handle] = citations[handle]
                ranges = fact.get('lines', {}).get(handle)
                anchors.append(handle if not ranges else f"{handle} (lines {', '.join(ranges)})")
            lines.append(f"- [{', '.join(scope)}] {' | '.join(details)} | Cite {', '.join(anchors)}")
        lines.append('')
    if sources:
        lines.append('Source anchors (file lines)')
        for handle, source in sources.items():
            qualified = ('[' + source['service'] + '@' + source['revision'] + '] ') if 'service' in source else ''
            prefix = qualified + single_line(source['file']) + ':' if qualified or source['file'] != file else ''
            proof = f" ({source['manifest']})" if 'manifest' in source else ''
            lines.append(f"- {handle} {prefix}{line_range(source)}{proof}")
        lines.append('')
    return ('\n'.join(lines) + '\n').encode()


def split_workspace_sections(groups, citations, file):
    """Keep an early sample across page, fact and boundary groups on shared views."""
    heading = '## Prepared source evidence'
    additional_heading = '## Additional prepared source evidence'
    full = workspace_sections(groups, citations, file, heading)
    if len(full) <= MAX_SOURCE_VIEW_PREFIX_BYTES:
        return full, b'', sum(map(len, groups.values())), 0

    def guidance(count):
        return (f'{count} additional prepared items follow the retained source under '
                f'"{additional_heading[3:]}".\n\n').encode()

    limit = MAX_SOURCE_VIEW_PREFIX_BYTES - len(guidance(sum(map(len, groups.values()))))
    first, rest = defaultdict(list), defaultdict(list)
    for offset in range(max(map(len, groups.values()))):
        for key, items in groups.items():
            if offset >= len(items):
                continue
            item = items[offset]
            first[key].append(item)
            if len(workspace_sections(first, citations, file, heading)) > limit:
                first[key].pop()
                rest[key].append(item)
    front_count, rest_count = sum(map(len, first.values())), sum(map(len, rest.values()))
    front = workspace_sections(first, citations, file, heading) + guidance(rest_count)
    return front, workspace_sections(rest, citations, file, additional_heading), front_count, rest_count


def workspace_views(home, required, rows, citations, view_files, budget):
    """Annotate one existing required view per scope and preserve verbatim source bytes."""
    views = defaultdict(dict)
    for identity, row in rows.items():
        view, label = required[identity]
        for category in ('facts', 'boundaries', 'factLimits', '_preparationOmissions'):
            values = row.get(category, ())
            if values and category in {'factLimits', '_preparationOmissions'}:
                values = (values,)
            for value in values:
                key = (category, digest(value))
                labels, _ = views[view].setdefault(key, ([], value))
                labels.append(label)

    summary = {'authorViewBytes': 0, 'maxAuthorPrefixBytes': 0, 'frontAuthorItems': 0,
               'additionalAuthorItems': 0, 'omittedAuthorViews': 0}
    for view, rows in views.items():
        groups = defaultdict(list)
        for (category, _), (labels, value) in rows.items():
            groups[tuple(dict.fromkeys(labels)), category].append(value)
        front, rest, first_count, rest_count = split_workspace_sections(groups, citations, view_files[view])
        added = len(front) + len(rest) + bool(rest)
        if summary['authorViewBytes'] + added > budget:
            summary['omittedAuthorViews'] += 1
            continue
        path = home / view
        header, separator, body = path.read_bytes().partition(b'\n\n')
        path.write_bytes(header + separator + front + body + (b'\n' + rest if rest else b''))
        summary['authorViewBytes'] += added
        summary['maxAuthorPrefixBytes'] = max(summary['maxAuthorPrefixBytes'], len(front))
        summary['frontAuthorItems'] += first_count
        summary['additionalAuthorItems'] += rest_count
    return summary

def csharp_background_units(job, scope, home, exact, deadline):
    extractor = (scope['coverage'].get('extractor') or '').lower()
    if 'csharp' not in extractor and not any(source['file'].lower().endswith('.cs') for source in exact.values()):
        return []
    rows = work_rows(job, scope, home / 'csharp-compilation-units', {'kind': 'SOURCE_FILE'}, deadline)
    return [row for row in rows or () if csharp_unit_row(row, scope)]


def background_units(units, exact):
    """Group verified compiler units by their original retained compilation."""
    groups = defaultdict(list)
    for row in units:
        record = row['record']
        fact = record.get('normalized', {})
        if fact.get('kind') == 'SOURCE_FILE' and fact.get('scope') and fact.get('sourceStatus') != 'EMPTY_SOURCE_FILE':
            groups[fact['scope']].append(row)
    for rows in groups.values():
        paths = {row['record']['normalized'].get('file') for row in rows}
        if len(paths) > 48 or any(row['record']['normalized'].get('sourceStatus') != 'SOURCE_RETAINED' for row in rows):
            continue
        files = csharp_files(rows, exact)
        if files.keys() == paths and sum(len(row['text'].encode()) for row in files.values()) <= 2 * 1024 * 1024:
            yield files


BACKGROUND_KINDS = {
    'HOSTED_SERVICE': ('HOSTED_BACKGROUND_WORKER', 'ROSLYN_FRAMEWORK_REGISTRATION'),
    'INTERVAL': ('SCHEDULED_JOB', 'TIMER_RETAINED_SOURCE'),
    'EACH_MESSAGE': ('KAFKA_LISTENER', 'KAFKAJS_RETAINED_SOURCE'),
    'JAVA_TYPED_DISPATCH': ('JAVA_TYPED_DISPATCH', 'RETAINED_JAVA_DISPATCHER_WIRING'),
}


def background_operation(service, found, files, exact):
    """Build a source-only root and rebind every range to retained Work."""
    sources, handles = {}, {}
    kind, authority = BACKGROUND_KINDS[found['kind']]

    def bind(place):
        file = place['file']
        if file not in files:
            return None
        unit = files[file]
        first, end = place['start'], place['end']
        raw = unit['text'].encode()
        if not 0 <= first < end <= len(raw):
            return None
        ref = 'range-' + hashlib.sha256(encode((file, first, end))).hexdigest()[:20]
        if ref not in handles:
            part = raw[first:end]
            try:
                text = part.decode()
            except UnicodeError:
                return None
            line = raw.count(b'\n', 0, first) + 1
            row = {**unit, 'id': ref, 'service': service, 'text': text, 'textDigest': sha(part),
                   'startByte': first, 'endByte': end, 'startLine': line,
                   'endLine': line + part.removesuffix(b'\n').count(b'\n'), 'references': [ref],
                   'authority': 'EXACT_SNAPSHOT_TEXT',
                   'occurrence': {**unit['occurrence'], 'startByte': first, 'endByte': end}}
            if handle := extra_binding(row, exact):
                # A textual match alone cannot join different snapshots or source objects.
                origin = exact[handle].get('occurrence') or {}
                if any(origin.get(key) != unit['occurrence'].get(key) for key in ('snapshot', 'blob')):
                    return None
                handles[ref] = handle
                sources[ref] = row
        return ref if ref in handles else None

    registration, callback = bind(found['registration']), bind(found['callback'])
    if not registration or not callback:
        return None
    name = found['name']
    identity_parts = (service, kind, registration, callback)
    if kind == 'JAVA_TYPED_DISPATCH':
        identity_parts += (found['channel'], found['eventType'])
    identity = hashlib.sha256(encode(identity_parts)).hexdigest()[:16]
    trigger = {'name': name, 'kind': found['kind'], 'authority': authority, 'registration': registration,
               'callback': callback}
    subscriptions = [bind(row) for row in found.get('subscriptions', ())]
    topic_sources = [bind(row) for row in found.get('topicSources', ())]
    proven_topics = found.get('topics') and subscriptions and all([*subscriptions, *topic_sources])
    if proven_topics:
        trigger['configuration'] = {'topics': found['topics']}
        trigger['subscriptions'] = subscriptions
        trigger['subscriptionSources'] = list(dict.fromkeys(topic_sources))
    if found.get('topicCoverageOpen') or found.get('topics') and not proven_topics:
        trigger['topicCoverageOpen'] = True
    if found.get('sdkImports'):
        trigger['assumesSdkImports'] = True
    refs = [registration, callback]
    if proven_topics:
        refs.extend(dict.fromkeys([*subscriptions, *topic_sources]))
    if kind == 'JAVA_TYPED_DISPATCH':
        assembly = [bind(row) for row in found['assemblySources']]
        if not assembly or not all(assembly):
            return None
        trigger['configuration'] = {'adapter': found['adapter'], 'channel': found['channel'],
                                    'eventType': found['eventType']}
        if 'subscriberId' in found:
            trigger['configuration']['subscriberId'] = found['subscriberId']
        trigger['assemblySources'] = list(dict.fromkeys(assembly))
        refs = list(dict.fromkeys([*refs, *assembly]))
    origins = list(dict.fromkeys(handles[ref] for ref in refs))
    page = service + '-background-' + identity + '.md'
    requirement = {'id': service + ':background:' + identity, 'kind': 'ENTRY', 'service': service,
                   'page': page, 'entrypoint': identity, 'entrySymbol': name, 'entryKind': kind,
                   'sourceIds': [exact[handle]['id'] for handle in origins],
                   'sourceReferences': [ref for handle in origins for ref in exact[handle]['references']],
                   'rootDeclarations': [callback], 'handlerId': 'entry-' + identity,
                   'method': None, 'path': None, 'authority': authority,
                   'trigger': trigger, 'boundaries': ['RUNTIME_ACTIVATION_NOT_ESTABLISHED']}
    helpers = [ref for row in found.get('helpers', ()) if (ref := bind(row))]
    bodies = list(dict.fromkeys([callback, *helpers]))
    calls, offsets, publications = [], [], []
    for row in found.get('calls', ()):
        if (site := bind(row['site'])) and (target := bind(row['target'])) and target in bodies:
            calls.append((site, target))
    for row in found.get('offsets', ()):
        references = [bind(row[key]) for key in ('use', 'declaration', 'consumer', 'call')]
        if all(references):
            offsets.append(references)
    for row in found.get('publications', ()):
        members = [bind(row[key]) for key in ('call', 'producer', 'client')]
        if all(members):
            publications.append(members)
    process = {'id': identity, 'root': callback, 'service': service, 'title': name,
               'entrySymbol': name, 'entryKind': kind, 'trigger': trigger, 'routes': [],
               'entrySources': refs, 'sources': sources,
               'facts': [{'kind': 'BODY', 'declaration': ref, 'sources': [ref]} for ref in bodies],
               '_background': {'bodies': bodies, 'calls': calls,
                               **({'compilationFiles': sorted(files)} if kind == 'HOSTED_BACKGROUND_WORKER' else {}),
                               'offsets': [row[:3] for row in offsets], 'publications': publications,
                               'anchored': list(dict.fromkeys([first for first, _ in calls] +
                                                            [row[0] for row in (*offsets, *publications)]))}}
    uncertainty = ('The source candidate assumes the declared dispatcher, bean and handler bindings. '
                   'Runtime activation, transport and wire bindings, delivery and execution order remain open.'
                   if kind == 'JAVA_TYPED_DISPATCH' else
                   'Source selects this callback. Activation, execution timing and delivery depend on runtime bindings.'
                   + (' Framework SDK imports are assumed.' if found.get('sdkImports') else ''))
    facts = [dict(kind='background-registration', authority=authority, sources=refs,
                  text=background_registration(found, trigger), uncertainty=uncertainty)]
    if trigger.get('topicCoverageOpen'):
        facts.append(dict(kind='background-topic-boundary', authority=authority, sources=[registration],
                          text='Additional subscription topics and activation remain open.'))
    if found.get('manualOffsets'):
        facts.append(dict(kind='background-offset-policy', authority=authority, sources=[registration],
                          text='The KafkaJS run options declare `autoCommit: false`.',
                          uncertainty='Offset advancement and redelivery depend on the broker and reached commit calls.'))
    return requirement, process, sources, origins, facts


def background_registration(found, trigger):
    name = trigger['name']
    if found['kind'] == 'HOSTED_SERVICE':
        return f'`AddHostedService` registers the type whose `BackgroundService` override is `{name}`.'
    if found['kind'] == 'INTERVAL':
        return f'`setInterval` selects the cited callback and supplies `{found["interval"]}` milliseconds as its repeat interval.'
    if found['kind'] == 'JAVA_TYPED_DISPATCH':
        if 'subscriberId' in found:
            return (f'The retained Spring bean wiring and `@EventuateDomainEventHandler` select `{name}` '
                    f'for `{found["eventType"]}` on channel `{found["channel"]}`.')
        return (f'The retained dispatcher bean uses the handler table that registers `{name}` '
                f'for `{found["eventType"]}` on channel `{found["channel"]}`.')
    topics = trigger.get('configuration', {}).get('topics', ())
    suffix = ' The same consumer has a preceding awaited subscription to ' + ', '.join(repr(topic) for topic in topics) + '.' if topics else ''
    return f'KafkaJS `consumer.run` selects `{name}` as `eachMessage`.' + suffix


def background_entries(service, requirements, units, exact, files, projector, job, deadline, page_slots, view_budget):
    typescript = {file: source for file, source in files.items()
                  if Path(file).suffix.lower() in projector['TYPESCRIPT']}
    selections = []
    if typescript and sum(len(row['text'].encode()) for row in typescript.values()) <= 2 * 1024 * 1024:
        selections.append(('typescript', typescript))
    selections.extend(('csharp', scope) for scope in background_units(units, exact))
    java = {file: source for file, source in files.items() if Path(file).suffix.lower() == '.java'}
    if java:
        selections.append(('java', java))
    entries, remaining = {}, min(page_slots, MAX_ROOTS)
    for family, complete in selections:
        if remaining <= 0 or deadline - time.monotonic() <= 1:
            break
        if family == 'java':
            dispatch = runpy.run_path(str(DIRECTORY / 'prepared_facts/clew_java_dispatch.py'))
            found = dispatch['discover'](complete, deadline=min(deadline, time.monotonic() + 12))
        else:
            found = projector['discover_background'](complete, node=job['node'], deadline=deadline, family=family)
        for candidate in found or ():
            if candidate.get('kind') not in BACKGROUND_KINDS:
                continue
            result = background_operation(service, candidate, complete, exact)
            if result is None:
                continue
            requirement, process, witnesses, origins, facts = result
            # Reserve the code bytes plus the formatter's headings, cites and line ranges.
            size = sum(len(witnesses[ref]['text'].encode()) + 1024 for ref in process['entrySources']) + 2048
            if size > view_budget:
                continue
            identity = requirement['id']
            if identity in entries:
                continue
            entries[identity] = (process, witnesses, origins, facts)
            requirements.append(requirement)
            remaining -= 1
            view_budget -= size
            if remaining <= 0:
                break
    return entries


def replay_reached_writes(context, propositions, exact, read_pinned):
    """Select only rebound write sites and replay them in the original operation."""
    replays = context.pop('_deferredWriteFocus', ())
    if not replays:
        return [], 0
    candidates = []
    for fact in propositions:
        proof, edge = fact.get('_writeSite'), fact.get('_edge')
        if not proof or not edge or not edge.get('occurrence'):
            continue
        if (site := REACHED['excerpt'](exact[proof['cite']], proof)) and all(
                site.get(key) == proof[key] for key in ('startByte', 'endByte')):
            candidates.append((site, edge['occurrence']))
    accepted, rejected = [], 0
    local = {site['id'] for site in context['sourceRanges'] if site['id'] in exact}
    for replay in replays if candidates else ():
        rows, extra = replay(candidates)
        bound, unbound = bound_facts(rows, extra, exact, local, read_pinned)
        accepted.extend(bound)
        rejected += unbound
    return accepted, rejected


def prepare(job, service, scope, requirements, entries, packet, home, deadline, *,
            page_slots=0, view_budget=0, edge_records=None, base_manifest=None, contract_resources=()):
    """Produce a bounded author packet and explicit per-entry coverage."""
    # Preparation is a treatment. A refusal still leaves the original read kit.
    service_root = home / 'fact-evidence'
    service_root.mkdir(mode=0o700)
    exact = exact_sources(entries, packet, authorities=('EXACT_SNAPSHOT_TEXT', 'DECLARED_OPENAPI'))
    edge_module = runpy.run_path(str(DIRECTORY / 'prepared_facts/clew_service_edges.py')) if edge_records is not None else None
    extensions = {ext for supported in ADAPTERS.values() for ext in supported}
    supported = [row for row in requirements if
                 Path(row.get('file', '')).suffix.lower() in extensions or any(
                     Path(entry['source']['file']).suffix.lower() in extensions for entry in entries.values()
                     if entry['source']['id'] in row['sourceIds'])]
    contexts, states = collect_contexts(job, scope, service_root, supported, deadline)
    all_units = csharp_background_units(job, scope, service_root, exact, deadline) if page_slots else []
    units = all_units or csharp_units(job, scope, service_root, exact, deadline)
    exact.update(acquire_sources(job, service, scope, service_root, [units, *contexts.values()], exact, deadline))
    admitted_references = {ref for row in exact.values() for ref in row['references']}
    files = {**full_files(exact), **csharp_files(units, exact)}
    boundaries = scope['coverage'].get('boundaries')
    allow_legacy_pinned = isinstance(boundaries, list) and 'WORKING_TREE_SOURCE' not in boundaries
    repository = local_repository(job, service, scope, service_root, deadline) if supported else None
    contract_facts, contract_sources = {}, {}
    if edge_records is not None:
        openapi = runpy.run_path(str(DIRECTORY / 'prepared_facts/clew_prepared_openapi.py'))
        contract_preparation = runpy.run_path(str(DIRECTORY / 'docs_run_contract_sources.py'), init_globals={
            'save': save, 'command': command, 'citation_load': citation_load, 'CitationStop': CitationStop, 'Stop': Stop,
            'openapi': openapi, 'work_rows': work_rows, 'acquire_sources': acquire_sources,
        })
        contract_facts, contract_sources = contract_preparation['prepare'](
            job, service, scope, requirements, exact, contract_resources, repository if allow_legacy_pinned else None,
            service_root, min(deadline, time.monotonic() + 20))
    pinned_cache = {}

    def read_pinned(file, revision):
        if revision != scope['revision'] or not repository or not allow_legacy_pinned:
            return None
        if file not in pinned_cache and len(pinned_cache) < 24 and deadline - time.monotonic() >= 3:
            pinned_cache[file] = pinned_file(repository, revision, file, deadline)
        return pinned_cache.get(file)

    projectors, background = {}, {}
    background_extensions = {'.cs', '.java', '.ts', '.tsx', '.mts', '.cts', '.js', '.jsx', '.mjs', '.cjs'}
    if page_slots and any(Path(file).suffix.lower() in background_extensions for file in files):
        path = DIRECTORY / 'prepared_facts' / 'managed_languages.py'
        managed = runpy.run_path(str(path))
        projectors[path.name] = managed['project']
        if 'discover_background' in managed:
            background = background_entries(service, requirements, all_units, exact, files, managed, job, deadline,
                                            page_slots, view_budget)
    discovered = {}
    result, cite_index, projections = {}, {}, {}
    ordered = sorted(requirements, key=lambda row: row['kind'] != 'ENTRY') if background else requirements
    for requirement in ordered:
        entry_id = requirement['id']
        row = {'page': requirement['page'], 'kind': requirement['kind']}
        if requirement.get('file'):
            row['file'] = requirement['file']
        result[entry_id] = row
        if states.get(entry_id):
            row['status'] = states[entry_id]
            continue
        synthetic = background.get(entry_id)
        if requirement not in supported and not synthetic:
            row['status'] = 'NO_SUPPORTED_ENTRY_SOURCE'
            continue
        if not synthetic and deadline - time.monotonic() <= 1:
            row['status'] = 'PREPARATION_LIMIT'
            continue
        witnesses, registration_facts = {}, []
        if synthetic:
            process, witnesses, origins, registration_facts = synthetic
            context = {'operation': process['id'], 'sourceRanges': list(witnesses.values()),
                       '_sourceOccurrences': witnesses, '_nativeFiles': files, '_citationSources': exact,
                       'file': None, 'entrySources': process['entrySources']}
            discovered[entry_id] = {'sources': {ref: witnesses[ref] for ref in process['entrySources']},
                                    'citations': {handle: exact[handle] for handle in origins}}
        else:
            roots = requirement.get('rootDeclarations', [])
            thread = contexts[roots[0]] if roots else ()
            missing = {ref for item in thread for ref in row_bindings(item).values()} - admitted_references
            if missing:
                row['missingSourceReferences'] = len(missing)
            process, context = operation(requirement, thread, exact, files)
        projection_limits = []
        context.update(_repository=repository, _revision=scope['revision'], _snapshot=job['snapshot'],
                       _loadPinnedFile=read_pinned, _deadline=deadline, _projectionUnavailable=projection_limits,
                       _allowLegacyPinned=allow_legacy_pinned, _sourceCallSelections=[], _deferredWriteFocus=[],
                       _bindWitness=lambda witness: extra_binding(witness, exact, read_pinned))
        if file := context['file']:
            languages = {Path(file).suffix.lower()} if context['sourceRanges'] else set()
        else:
            root_sources = {ref for fact in process['facts'] if fact['kind'] == 'BODY' and fact['declaration'] == process['root']
                            for ref in fact['sources']}
            languages = {Path(process['sources'][ref]['file']).suffix.lower() for ref in root_sources}
        if not languages:
            if edge_module and requirement['kind'] == 'ENTRY':
                edge_records[entry_id] = edge_module['record'](requirement, process, context, [], exact)
            row['status'] = 'NO_RETAINED_ROOT_BODY'
            continue
        if not synthetic and languages & {'.ts', '.tsx', '.mts', '.cts', '.js', '.jsx', '.mjs', '.cjs'}:
            row['scopeLimits'] = ['TYPESCRIPT_CROSS_METHOD_RELATIONSHIPS_OPEN']
        local = {site['id'] for site in context['sourceRanges']} - witnesses.keys()
        propositions, unbound = bound_facts(registration_facts, witnesses, exact, local, read_pinned)
        unavailable, failed = [], {}
        for filename, allowed in ADAPTERS.items():
            if not languages & allowed:
                continue
            path = DIRECTORY / 'prepared_facts' / filename
            try:
                if filename not in projectors:
                    projectors[filename] = runpy.run_path(str(path))['project']
                raw, extra = projectors[filename](process, context, node=job['node'])
                accepted, rejected = bound_facts(raw, {**witnesses, **extra}, exact, local, read_pinned)
            except (OSError, ImportError, subprocess.SubprocessError):
                unavailable.append(path.stem)
                continue
            except (AttributeError, IndexError, KeyError, RecursionError, TypeError, ValueError) as error:
                failed[path.stem] = type(error).__name__
                continue
            propositions.extend(accepted)
            unbound += rejected
        projections[entry_id] = requirement, process, context, propositions, unbound, projection_limits, failed, unavailable
    local_candidates = {}
    indexed = REACHED['index_discovery']((item[3] for item in projections.values() if item[0]['kind'] == 'DISCOVERY'), exact)
    for entry_id, (requirement, process, context, propositions, unbound, projection_limits, failed, unavailable) in projections.items():
        row = result[entry_id]
        if requirement['kind'] == 'ENTRY':
            additional, limited_reach = REACHED['select'](process, indexed, exact, context['_bindWitness'],
                                                         context['_sourceCallSelections'])
            direct = {(fact['kind'], fact['text'], fact.get('condition')) for fact in propositions}
            direct_loops = {fact['_repetitionKey'] for fact in propositions if '_repetitionKey' in fact}
            propositions.extend(fact for fact in additional if fact.get('_repetitionKey') not in direct_loops
                                and ((fact.get('_writeSite') and fact.get('_edge'))
                                     or (fact['kind'], fact['text'], fact.get('condition')) not in direct))
            try:
                focused, rejected = replay_reached_writes(context, propositions, exact, read_pinned)
            except (AttributeError, IndexError, KeyError, RecursionError, TypeError, ValueError) as error:
                failed['managed_languages'] = type(error).__name__
            else:
                propositions.extend(focused)
                unbound += rejected
            if limited_reach:
                row['omittedReachedDeclarations'] = limited_reach
                projection_limits.append('REACHED_DECLARATION_LIMIT')
            if edge_module:
                edge_records[entry_id] = edge_module['record'](requirement, process, context, propositions, exact)
        local_candidates[entry_id] = propositions
        for internal in propositions:
            for handle in internal['sources']:
                if handle in cite_index:
                    continue
                source = exact[handle]
                cite_index[handle] = {key: source[key] for key in ('file', 'startLine', 'endLine')}
                if 'manifest' in source:
                    cite_index[handle]['manifest'] = source['manifest'].relative_to(home).as_posix()
        limits = sorted({code for code in projection_limits
                         if isinstance(code, str) and re.fullmatch(r'[A-Z][A-Z_0-9]{0,79}', code)})
        status = 'NO_CITABLE_FACTS'
        if failed:
            status = 'PROJECTOR_FAILED'
        elif unavailable or limits:
            status = 'PROJECTOR_UNAVAILABLE'
        row.update(status=status, unboundPropositions=unbound)
        if failed:
            row['failedProjectors'] = failed
        if unavailable:
            row['unavailableProjectors'] = unavailable
        if limits:
            row['projectionLimits'] = limits
    selection = {'entries': result, 'citations': cite_index, 'localCandidates': local_candidates,
                 'isolated': job.get('authorMode') == 'ISOLATED_PAGES', 'discovered': discovered}
    if edge_module:
        edge_citations = {handle: {key: source[key] for key in ('file', 'service', 'revision', 'startLine', 'endLine')}
                          | {'manifest': source.get('manifest', base_manifest), 'textDigest': source.get('textDigest')}
                          for handle, source in (exact | contract_sources).items()}
        return {**selection, 'edgeCitations': edge_citations, 'contractFacts': contract_facts}
    merge_edges(selection, {}, {}, home)
    return {**finish(job, service, home, result, selection['citations']), 'discovered': discovered}


def interleave(groups):
    active = deque(iter(group) for group in groups)
    while active:
        group = active.popleft()
        if (item := next(group, None)) is not None:
            yield item
            active.append(group)


def local_group(fact):
    if fact.get('_reachedDeclaration'):
        return 'reached-declaration'
    if fact.get('_writeFocus'):
        return 'selected-write'
    if fact['kind'].replace('_', '-').lower().endswith('-effect-inventory'):
        return 'effect-inventory'
    return 'local'


def fact_event(internal):
    return ((internal.get('_edge') or {}).get('occurrence')
            or (internal.get('_writeFocus') or {}).get('occurrence')
            or internal.get('_edgeCause') or internal.get('_reachedDeclaration')
            or tuple(sorted(internal['sources'])))


def ranked_facts(local, peers, *, operation=False):
    """Group exact evidence and lead with representatives of supported local behavior."""
    groups, unique = {}, set()
    ordered = []
    for origin, rows in (('peer', peers), (None, local)):
        for internal in rows:
            fact = {key: value for key, value in internal.items() if not key.startswith('_')}
            proof = sorted({tuple(sorted(row.items())) for row in fact.get('witnesses', ())})
            key = {**fact, 'sources': sorted(fact['sources']), 'witnesses': proof}
            identity = hashlib.sha256(encode(key)).digest()
            if identity in unique:
                continue
            unique.add(identity)
            channel = groups.setdefault(origin or local_group(internal), {})
            kind = channel.setdefault(fact['kind'], {})
            branch = kind.setdefault(fact.get('condition') or '', {})
            # Stable site and branch grouping can prioritize representatives without merging outcomes.
            branch.setdefault(fact_event(internal), []).append(item := (fact, 0 if operation else len(encode(fact)) + 1))
            ordered.append(item)

    ranked = {group: interleave(interleave(interleave(branch.values()) for branch in kind.values())
                                for kind in kinds.values()) for group, kinds in groups.items()}
    if not operation:
        return ordered, interleave(ranked.values())
    # Local boundary, receipt and continuation kinds get up to one representative
    # before bulk write projections and composed paths compete for room.
    primary = ['local', 'reached-declaration']
    # Sample every local kind, even if a single reached write produced many elaborations.
    primary_rows = [interleave(interleave(branch.values()) for branch in kind.values())
                    for group in primary for kind in groups.get(group, {}).values()]
    representatives = [next(queue) for queue in primary_rows]
    rest = [queue for group, queue in ranked.items() if group not in primary]
    return ordered, chain(representatives, interleave([*rest, *primary_rows]))


def allocate_facts(selection, peers):
    """Allocate after edge qualification, with independent caps for isolated scopes."""
    rows = selection['entries']
    local = selection.pop('localCandidates', {identity: row.get('propositions', ()) for identity, row in rows.items()})
    isolated = selection.get('isolated', False)
    stats = {identity: {'bytes': 0, 'omitted': Counter(), 'propositions': []} for identity in rows}
    isolated_operations = {identity for identity, row in rows.items() if isolated and row.get('kind') == 'ENTRY'}
    ordered = {identity: ranked_facts(local.get(identity, ()), peers.get(identity, ()),
                                      operation=identity in isolated_operations) for identity in rows}
    sizes = {identity: sum(size for _, size in natural) for identity, (natural, _) in ordered.items()}
    shared_limit = not isolated and sum(sizes.values()) > MAX_PACKET_BYTES
    candidates = []
    for identity in rows:
        natural, ranked = ordered[identity]
        if identity in isolated_operations:
            # Only the real operation prompt charges these candidates and their shared citations.
            stats[identity]['propositions'] = [fact for fact, _ in ranked]
            continue
        # Preserve the projector's presentation order when there is no competition for space.
        if len(natural) <= MAX_FACTS and not shared_limit and (not isolated or sizes[identity] <= MAX_DISCOVERY_FACT_BYTES):
            ranked = natural
        candidates.append(zip(repeat(identity), ranked))
    total = 0
    for identity, (fact, size) in interleave(candidates):
        result = stats[identity]
        if len(result['propositions']) >= MAX_FACTS:
            result['omitted']['ENTRY_FACT_LIMIT'] += 1
        elif isolated and result['bytes'] + size > MAX_DISCOVERY_FACT_BYTES:
            result['omitted']['DISCOVERY_FACT_BYTES'] += 1
        elif not isolated and total + size > MAX_PACKET_BYTES:
            result['omitted']['WORKSPACE_FACT_BYTES'] += 1
        else:
            result['propositions'].append(fact)
            result['bytes'] += size
            total += size
    for identity, result in stats.items():
        row = rows[identity]
        if result['propositions'] or result['omitted'] or identity in local:
            facts = result['propositions']
            row.update(propositions=facts, omittedForLimit=sum(result['omitted'].values()))
            if result['omitted']:
                row['factOmissions'] = dict(result['omitted'])
            if facts:
                row['status'] = 'BOUNDARIES_ONLY' if all(map(is_boundary, facts)) else 'PREPARED'
            elif result['omitted'] and row.get('status') == 'NO_CITABLE_FACTS':
                row['status'] = 'PREPARATION_LIMIT'
    selection['bytes'] = total


def merge_edges(selection, edges, sources, home):
    result, citations = selection['entries'], selection['citations']
    eligible = defaultdict(list)
    edge_handles = set()
    for identity, row in result.items():
        for edge in edges.get(identity, ()):
            if any(handle not in sources or sources[handle]['manifest'] is None for handle in edge['sources']):
                row['unboundPropositions'] = row.get('unboundPropositions', 0) + 1
                continue
            eligible[identity].append(edge)
            edge_handles.update(edge['sources'])
    allocate_facts(selection, eligible)
    chosen = {handle for row in result.values() for fact in row.get('propositions', ()) for handle in fact['sources']}
    for handle in sorted(chosen & edge_handles):
        source = sources[handle]
        citations[handle] = {**source, 'manifest': os.path.relpath(source['manifest'], home)}
    selection['citations'] = {handle: value for handle, value in citations.items() if handle in chosen}


def finish(job, service, home, result, cite_index):
    summary = dict(Counter(row['status'] for row in result.values()))
    save(home / 'prepared-facts.json', {'schema': FACT_SCHEMA, 'snapshot': job['snapshot'], 'service': service,
                                       'entries': result, 'summary': summary})
    propositions = [fact for row in result.values() for fact in row.get('propositions', ())]
    catalog = {} if job.get('authorMode') == 'ISOLATED_PAGES' else None
    authors = author_entries(result, cite_index, catalog)
    return {'statusCounts': summary, 'facts': sum(not is_boundary(fact) for fact in propositions),
            'boundaries': sum(map(is_boundary, propositions)),
            'authorEntries': authors, 'citations': cite_index,
            **({'witnessCatalog': [dict(proof) for proof in catalog]} if catalog else {})}
