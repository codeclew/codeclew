"""Prepare cited propositions from the same immutable Work used for authoring."""
from __future__ import annotations

from collections import Counter, defaultdict
import hashlib
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
MAX_FILE_BYTES = 128 * 1024
CSHARP_FILE_LIMIT = 12
FACT_SCHEMA = 'codeclew-docs-run-prepared-facts/1.0'
VIEW_SCHEMA = 'codeclew-docs-run-fact-view/1.0'


def exact_sources(entries, packet):
    """Project only source bytes already admitted by the ordinary manifest reader."""
    originals = {row['source']['id']: row for row in packet['outcomes']}
    selected = {}
    for handle, entry in entries.items():
        source = entry['source']
        if source.get('authority') != 'EXACT_SNAPSHOT_TEXT':
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
            'ordering', 'targetStatus', 'accessMode', 'condition', 'eventId', 'kafka') if key in fact}
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
        if not isinstance(row.get('text'), str) or not row['text'].strip() or not row.get('kind') or not row.get('authority'):
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
        values['sources'] = list(dict.fromkeys(bindings.get(ref, ref) for ref in refs))
        if witnesses := [dict(cite=bindings[ref], **{key: lifted[ref][key] for key in (
                'file', 'revision', 'startLine', 'endLine', 'startByte', 'endByte', 'textDigest') if key in lifted[ref]})
                for ref in dict.fromkeys(refs) if ref in bindings]:
            values['witnesses'] = witnesses
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
        for row in rows or ():
            record = row['record']
            fact = record.get('normalized', {})
            if (record.get('service') == scope['coverage']['id']
                    and record.get('symbol') == fact.get('file') == file
                    and fact.get('schema') == 'codeclew-csharp-compiler-fact/1.0'
                    and fact.get('kind') == 'SOURCE_FILE' and fact.get('resolution') == 'SOURCE_MEMBERSHIP_EXACT'
                    and fact.get('sourceStatus') == 'SOURCE_RETAINED' and len(row_bindings(row)) == 1):
                units.append(row)
    return units


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


def acquire_sources(job, service, scope, home, selections, exact, deadline):
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
        return exact_sources(entries, packet), None

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


def author_views(home, entries):
    """Write per-requirement answers and explicit gaps without repeated witness metadata."""
    if not entries:
        return {}, 0
    directory = home / 'fact-views'
    directory.mkdir(mode=0o700)
    views, byte_count = {}, 0
    for identity, row in entries.items():
        name = sha(identity.encode()).split(':', 1)[1][:20] + '.json'
        path = directory / name
        packet = {'schema': VIEW_SCHEMA, 'requirement': identity, **row}
        packet['propositions'] = [{key: value for key, value in fact.items() if key != 'witnesses'}
                                  for fact in row.get('propositions', ())]
        raw = (json.dumps(packet, ensure_ascii=False, separators=(',', ':')) + '\n').encode()
        with path.open('xb') as output:
            output.write(raw)
        byte_count += len(raw)
        relative = str(path.relative_to(home))
        row['authorView'] = relative
        views[identity] = {'label': row.get('file', row['page']), 'path': relative, 'status': row['status']}
    return views, byte_count


def prepare(job, service, scope, requirements, entries, packet, home, deadline):
    """Produce a bounded author packet and explicit per-entry coverage."""
    # Preparation is a treatment. A refusal still leaves the original read kit.
    service_root = home / 'fact-evidence'
    service_root.mkdir(mode=0o700)
    exact = exact_sources(entries, packet)
    extensions = {ext for supported in ADAPTERS.values() for ext in supported}
    supported = [row for row in requirements if
                 Path(row.get('file', '')).suffix.lower() in extensions or any(
                     Path(entry['source']['file']).suffix.lower() in extensions for entry in entries.values()
                     if entry['source']['id'] in row['sourceIds'])]
    contexts, states = collect_contexts(job, scope, service_root, supported, deadline)
    units = csharp_units(job, scope, service_root, exact, deadline)
    exact.update(acquire_sources(job, service, scope, service_root, [units, *contexts.values()], exact, deadline))
    admitted_references = {ref for row in exact.values() for ref in row['references']}
    files = {**full_files(exact), **csharp_files(units, exact)}
    boundaries = scope['coverage'].get('boundaries')
    allow_legacy_pinned = isinstance(boundaries, list) and 'WORKING_TREE_SOURCE' not in boundaries
    repository = local_repository(job, service, scope, service_root, deadline) if supported else None
    pinned_cache = {}

    def read_pinned(file, revision):
        if revision != scope['revision'] or not repository or not allow_legacy_pinned:
            return None
        if file not in pinned_cache and len(pinned_cache) < 24 and deadline - time.monotonic() >= 3:
            pinned_cache[file] = pinned_file(repository, revision, file, deadline)
        return pinned_cache.get(file)

    projectors = {}
    result, cite_index, total_bytes = {}, {}, 0
    for requirement in requirements:
        entry_id = requirement['id']
        row = {'page': requirement['page'], 'kind': requirement['kind']}
        if requirement.get('file'):
            row['file'] = requirement['file']
        result[entry_id] = row
        if states.get(entry_id):
            row['status'] = states[entry_id]
            continue
        if requirement not in supported:
            row['status'] = 'NO_SUPPORTED_ENTRY_SOURCE'
            continue
        if deadline - time.monotonic() <= 1:
            row['status'] = 'PREPARATION_LIMIT'
            continue
        roots = requirement.get('rootDeclarations', [])
        thread = contexts[roots[0]] if roots else ()
        missing = {ref for item in thread for ref in row_bindings(item).values()} - admitted_references
        if missing:
            row['missingSourceReferences'] = len(missing)
        process, context = operation(requirement, thread, exact, files)
        projection_limits = []
        context.update(_repository=repository, _revision=scope['revision'], _snapshot=job['snapshot'],
                       _loadPinnedFile=read_pinned, _deadline=deadline, _projectionUnavailable=projection_limits,
                       _allowLegacyPinned=allow_legacy_pinned,
                       _bindWitness=lambda witness: extra_binding(witness, exact, read_pinned))
        if file := context['file']:
            languages = {Path(file).suffix.lower()} if context['sourceRanges'] else set()
        else:
            root_sources = {ref for fact in process['facts'] if fact['kind'] == 'BODY' and fact['declaration'] == process['root']
                            for ref in fact['sources']}
            languages = {Path(process['sources'][ref]['file']).suffix.lower() for ref in root_sources}
        if not languages:
            row['status'] = 'NO_RETAINED_ROOT_BODY'
            continue
        if languages & {'.ts', '.tsx', '.mts', '.cts', '.js', '.jsx', '.mjs', '.cjs'}:
            row['scopeLimits'] = ['TYPESCRIPT_CROSS_METHOD_RELATIONSHIPS_OPEN']
        propositions, unbound, unavailable, failed = [], 0, [], {}
        local = {site['id'] for site in context['sourceRanges']}
        for filename, allowed in ADAPTERS.items():
            if not languages & allowed:
                continue
            path = DIRECTORY / 'prepared_facts' / filename
            try:
                if filename not in projectors:
                    projectors[filename] = runpy.run_path(str(path))['project']
                raw, extra = projectors[filename](process, context, node=job['node'])
                accepted, rejected = bound_facts(raw, extra, exact, local, read_pinned)
            except (OSError, ImportError, subprocess.SubprocessError):
                unavailable.append(path.stem)
                continue
            except (AttributeError, IndexError, KeyError, RecursionError, TypeError, ValueError) as error:
                failed[path.stem] = type(error).__name__
                continue
            propositions.extend(accepted)
            unbound += rejected
        visible, limited = [], 0
        for fact in propositions:
            size = len(encode(fact))
            if len(visible) >= MAX_FACTS or total_bytes + size > MAX_PACKET_BYTES:
                limited += 1
                continue
            visible.append(fact)
            total_bytes += size
            for handle in fact['sources']:
                cite_index[handle] = {key: exact[handle][key] for key in ('file', 'startLine', 'endLine')}
        limits = sorted({code for code in projection_limits
                         if isinstance(code, str) and re.fullmatch(r'[A-Z][A-Z_0-9]{0,79}', code)})
        status = 'NO_CITABLE_FACTS'
        if failed:
            status = 'PROJECTOR_FAILED'
        elif unavailable or limits:
            status = 'PROJECTOR_UNAVAILABLE'
        if visible:
            status = 'PREPARED' if not all(map(is_boundary, visible)) else 'BOUNDARIES_ONLY'
        row.update(status=status, propositions=visible, unboundPropositions=unbound, omittedForLimit=limited)
        if failed:
            row['failedProjectors'] = failed
        if unavailable:
            row['unavailableProjectors'] = unavailable
        if limits:
            row['projectionLimits'] = limits
    summary = dict(Counter(row['status'] for row in result.values()))
    views, author_bytes = author_views(home, result)
    save(home / 'prepared-facts.json', {'schema': FACT_SCHEMA, 'snapshot': job['snapshot'], 'service': service,
                                       'entries': result, 'summary': summary})
    propositions = [fact for row in result.values() for fact in row.get('propositions', ())]
    return {'statusCounts': summary, 'facts': sum(not is_boundary(fact) for fact in propositions),
            'boundaries': sum(map(is_boundary, propositions)),
            'authorViewBytes': author_bytes, 'authorViews': views, 'citations': cite_index}
