"""Prepare cited propositions from the same immutable Work used for authoring."""
from __future__ import annotations

from collections import Counter, defaultdict
import hashlib
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
FACT_SCHEMA = 'codeclew-docs-run-prepared-facts/1.0'


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
                               'occurrence': {'snapshot': snapshot, 'blob': blob, 'startByte': 0, 'endByte': len(raw)}}
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
            report = collect(job['clew'], job['root'], scope['work'], output, max_pages=8,
                             timeout=min(15, deadline - time.monotonic()), kind='work-selection',
                             selection={'query': {'kind': 'THREAD_CONTEXT', 'symbolContains': root}})
            identity_scope = report.get('identity') or {}
            contexts[root] = (load(output / 'items.json') if report['status'] == 'COMPLETE'
                              and identity_scope.get('snapshot') == job['snapshot']
                              and identity_scope.get('contextDigest') == scope['contextDigest']
                              and identity_scope.get('inputDigest') == scope['inputDigest'] else None)
        if contexts[root] is None:
            states[identity] = 'RETAINED_THREAD_INCOMPLETE'
    return contexts, states


def acquire_sources(job, service, scope, home, contexts, exact, deadline):
    bound = {ref for row in exact.values() for ref in row['references']}
    wanted = {}
    for rows in contexts.values():
        for row in rows or ():
            for identity, reference in row_bindings(row).items():
                if reference not in bound and len(wanted) < MAX_REFS:
                    wanted.setdefault(identity, reference)
    if not wanted or deadline - time.monotonic() <= 1:
        return {}
    # A refused source may leave a partial packet. Keep its receipts, but do not
    # admit that incomplete manifest into authoring or the page verification gate.
    selection = home / 'source-selection.json'
    save(selection, [{'sourceId': key, 'sourceReference': value} for key, value in sorted(wanted.items())])
    directory = home / 'read'
    argv = [sys.executable, '-I', '-S', str(DIRECTORY / 'evidence_packet.py'), 'read-sources',
            '--clew', job['clew'], '--root', job['root'], '--service', service, '--snapshot', job['snapshot'],
            '--work', scope['work'], '--selection', str(selection), '--output-dir', str(directory),
            '--output-format', 'metadata']
    manifest = directory / 'citations.json'
    try:
        command(argv, home, 'read-sources', timeout=min(45, deadline - time.monotonic()))
        _, packet, _, entries = citation_load(manifest)
        need(packet['scope']['snapshot'] == job['snapshot'] and packet['work'] == scope['work']
             and all(row['source']['service'] == service and row['source']['revision'] == scope['revision']
                     for row in entries.values()), 'PREPARED_FACT_SOURCE_SCOPE_CHANGED')
    except (Stop, CitationStop):
        if manifest.is_file():
            manifest.rename(directory / 'unadmitted-citations.json')
        return {}
    return exact_sources(entries, packet)


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


def pinned_file(repository, revision, file):
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
            return subprocess.run(base + list(args), check=True, capture_output=True,
                                  stdin=subprocess.DEVNULL, timeout=3, env=env).stdout
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
    exact.update(acquire_sources(job, service, scope, service_root, contexts, exact, deadline))
    files = full_files(exact)
    repository = local_repository(job, service, scope, service_root, deadline) if supported else None
    pinned_cache = {}

    def read_pinned(file, revision):
        if revision != scope['revision'] or not repository:
            return None
        if file not in pinned_cache and len(pinned_cache) < 24 and deadline - time.monotonic() >= 3:
            pinned_cache[file] = pinned_file(repository, revision, file)
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
        process, context = operation(requirement, contexts[roots[0]] if roots else (), exact, files)
        projection_limits = []
        context.update(_repository=repository, _revision=scope['revision'], _snapshot=job['snapshot'],
                       _loadPinnedFile=read_pinned, _deadline=deadline, _projectionUnavailable=projection_limits,
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
        propositions, unbound, unavailable = [], 0, []
        local = {site['id'] for site in context['sourceRanges']}
        for filename, allowed in ADAPTERS.items():
            if not languages & allowed:
                continue
            path = DIRECTORY / 'prepared_facts' / filename
            try:
                if filename not in projectors:
                    projectors[filename] = runpy.run_path(str(path))['project']
                raw, extra = projectors[filename](process, context, node=job['node'])
            except (FileNotFoundError, PermissionError, subprocess.SubprocessError):
                unavailable.append(path.stem)
                continue
            accepted, rejected = bound_facts(raw, extra, exact, local, read_pinned)
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
        status = 'PROJECTOR_UNAVAILABLE' if unavailable or limits else 'NO_CITABLE_FACTS'
        if visible:
            status = 'PREPARED' if any('boundary' not in fact['kind'] for fact in visible) else 'BOUNDARIES_ONLY'
        row.update(status=status, propositions=visible, unboundPropositions=unbound, omittedForLimit=limited)
        if unavailable:
            row['unavailableProjectors'] = unavailable
        if limits:
            row['projectionLimits'] = limits
    summary = dict(Counter(row['status'] for row in result.values()))
    save(home / 'prepared-facts.json', {'schema': FACT_SCHEMA, 'snapshot': job['snapshot'], 'service': service,
                                       'entries': result, 'summary': summary})
    propositions = [fact for row in result.values() for fact in row.get('propositions', ())]
    return {'statusCounts': summary, 'facts': sum('boundary' not in fact['kind'] for fact in propositions),
            'boundaries': sum('boundary' in fact['kind'] for fact in propositions), 'citations': cite_index}
