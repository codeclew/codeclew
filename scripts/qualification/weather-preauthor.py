#!/usr/bin/env python3
"""Collect and verify the Weather semantic regression on an installed author kit."""

import argparse
from functools import cache
import hashlib
import json
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
REVISION = 'c9a4283cb5bbe2409b95561aca4e751a71255ff9'
RUBRIC_SHA = 'sha256:3d9f22966411dde050d16819e1d173a5b8cff570654a2f77bf84675c0a00b059'
# Historical operation IDs identify rubric rows. Installed native IDs and page names can vary.
SELECTORS = {
    'operation-a7d8675103bc': ('dashboard', 'POST', '/api/places'),
    'operation-2989c68b73c0': ('dashboard', 'POST', '/api/places/{id}/refresh'),
    'operation-45be3587fb5b': ('places', 'POST', '/places'),
    'operation-50094b895b93': ('places', 'POST', '/places/{id}/refresh'),
    'operation-ff2e6db7f082': ('places', None, 'ExecuteAsync'),
    'operation-a5765007228f': ('forecasts', None, 'consume'),
    'operation-d84844588e8d': ('forecasts', None, 'refreshExpired'),
    'operation-602619abba89': ('forecasts', None, 'publishPending'),
    'operation-62ac6ca924e6': ('dashboard', None, 'consumeMessage'),
}
SECTION = re.compile(r'^## (?:Prepared|Additional prepared) source evidence$')
FACT_GROUP = re.compile(r'^(?:Facts|Boundaries) for (.+?)(?: \[[^\]]+\])?$')
ANCHOR = re.compile(r'[\w:-]+(?: \(lines \d+(?:-\d+)?(?:, \d+(?:-\d+)?)*\))?')
FENCE = re.compile(r'^[ ]{0,3}(`{3,}|~{3,})')


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(raw):
    return 'sha256:' + hashlib.sha256(raw).hexdigest()


def load(path):
    return json.loads(path.read_bytes())


def rows_by(rows, key):
    index = {row[key]: row for row in rows}
    require(len(rows) == len(index), f'Duplicate {key}')
    return index


def rubric():
    raw = (HERE / 'weather-preauthor-rubric.json').read_bytes()
    require(sha(raw) == RUBRIC_SHA, 'The original 45-atom rubric changed')
    rows = rows_by(json.loads(raw), 'id')
    components = (HERE / 'weather-preauthor-components.tsv').read_text().splitlines()
    pairs = [line.split('\t', 1) for line in components]
    require(len(pairs) == len(rows) == 45 and {key for key, _ in pairs} == rows.keys(),
            'Each frozen atom needs its own component list')
    for key, value in pairs:
        rows[key]['components'] = dict(enumerate(value.split(' | '), start=1))
    return rows


def admitted(run, skill, expected_runtime):
    state = load(run / 'state.json')
    require(state['status'] == 'PREPARED' and state['authorStartedAt'] is None and not state['attempts']
            and state['modelTokens'] == 0, 'Requires a complete prepare with zero author invocations')
    pins = load(run / 'pins.json')
    caps = pins['clewCapabilities']
    require(caps['runtimeMode'] == 'RELEASE' and caps['runtimeKey'] == expected_runtime,
            'Prepared kit does not match the admitted installed release runtime')
    runner = runpy.run_path(str(skill / 'scripts/docs_run.py'))
    require(runner['SKILL'].resolve() == skill.resolve(), 'Installed skill location changed')
    # The packaged validator recomputes prompt, inventory, renderer and complete input/skill closure.
    runner['frozen_inputs'](run)
    job = load(run / 'inputs/job.json')
    require(job['evidenceMode'] == 'PREPARED_FACTS', 'Prepared-facts job mode required')
    manifest = load(run / 'inputs/inventory.json')
    require(set(manifest['services']) == {'dashboard', 'forecasts', 'places'}
            and all(row['revision'] == REVISION for row in manifest['services'].values()),
            'Installed run must retain all three services at the pinned Weather revision')
    return manifest


def select_scopes(inventory):
    selected = {}
    for op, (service, method, target) in SELECTORS.items():
        scoped = [row for row in inventory['requirements'] if row['kind'] == 'ENTRY' and row['service'] == service]
        if method:
            scoped = [row for row in scoped if (row['method'], row['path']) == (method, target)]
        else:
            pattern = re.compile(r'(?<![\w])' + re.escape(target) + r'(?![\w])')
            scoped = [row for row in scoped if not row.get('method') and any(
                pattern.search(str(field)) for field in (
                    row.get('entrySymbol'), (row.get('trigger') or {}).get('name'), *row.get('rootDeclarations', ())))]
        require(len(scoped) <= 1, f'Ambiguous installed entry scope for {op}')
        if len(scoped) == 1:
            selected[op] = scoped[0]
    return selected


def visible_view(path):
    """Read the cited proposition rows in authored-facing Markdown, outside all code fences."""
    section = False
    labels = []
    fence = None
    for number, line in enumerate(path.read_text().splitlines(), start=1):
        delimiter = FENCE.match(line)
        if fence:
            if delimiter and delimiter[1][0] == fence[0] and len(delimiter[1]) >= len(fence):
                fence = None
            continue
        if delimiter:
            fence = delimiter[1]
            section = False
            continue
        if line.startswith('## '):
            section = SECTION.fullmatch(line) is not None
            labels = []
        if not section:
            continue
        match = FACT_GROUP.fullmatch(line)
        if match:
            labels = match[1].split(', ')
            continue
        if not line.startswith('- ['):
            if line:
                labels = []
            continue
        assertion, separator, anchors = line.rpartition(' | Cite ')
        if not separator or not labels:
            continue
        require(bool(re.fullmatch(ANCHOR.pattern + r'(?:, ' + ANCHOR.pattern + ')*', anchors)),
                f'Invalid cited prepared statement in {path.name}:{number}')
        yield number, labels, assertion, [match[0] for match in ANCHOR.finditer(anchors)]


def record(result, scopes, location, text, anchors, files=(), locations=None, operation_packet=None):
    if (scopes or files) and anchors:
        row = {'scopes': sorted(scopes), 'files': sorted(files), 'location': location,
               'text': text, 'anchors': anchors, 'locations': locations or {}}
        if operation_packet is not None:
            row['operationPacket'] = operation_packet
        key = sha(json.dumps(row, sort_keys=True, ensure_ascii=False).encode())
        result[key] = row


def prompt_packet(path):
    return json.loads(path.read_text().strip().rsplit('\n\n', 1)[1])


def structured_records(result, path, targets, alias_map, citations, refs, files, source_ranges=None, operation_packet=None):
    for identity, target in targets.items():
        for category in ('facts', 'boundaries'):
            for i, fact in enumerate(target.get(category, [])):
                owners = {identity, *fact.get('alsoFor', ())}
                relevant = {refs[owner] for owner in owners if owner in refs}
                local_files = {files[owner] for owner in owners if owner in files}
                anchors, locations = [], {}
                for alias in fact['sources']:
                    cite = citations[alias]
                    detail = fact.get('lines', {}).get(alias)
                    handle = alias_map[alias]
                    anchors.append(handle + (f' (lines {", ".join(detail)})' if detail else ''))
                    if source_ranges is not None:
                        location = source_ranges[cite['region']]
                        locations[handle] = {key: location[key] for key in ('service', 'file', 'startLine', 'endLine')}
                        locations[handle]['bodySha256'] = sha(location['text'].encode())
                        locations[handle]['authority'] = cite['authority']
                visible = {key: value for key, value in fact.items() if key not in {'sources', 'lines', 'alsoFor'}}
                record(result, relevant, f'{path}#{identity}/{category}/{i}',
                       json.dumps(visible, ensure_ascii=False, sort_keys=True), anchors, local_files, locations,
                       operation_packet)


def collect(run, inventory, plan):
    statement_rows = {}
    scopes = select_scopes(inventory)
    refs = {row['id']: op for op, row in scopes.items()}
    files = {row['id']: row['service'] + ':' + row['file'] for row in inventory['requirements']
             if row['kind'] == 'DISCOVERY'}
    if plan['mode'] == 'INLINE':
        packet = prompt_packet(run / 'prompt.txt')
        targets = {**packet['entries'], **packet['files']}
        inline_refs = {alias: refs[identity] for alias, identity in plan['entries'].items() if identity in refs}
        inline_files = {alias: files[identity] for alias, identity in plan.get('files', {}).items() if identity in files}
        structured_records(statement_rows, 'prompt.txt', targets, plan['citations'],
                           packet['citations'], inline_refs, inline_files)
    elif plan['mode'] == 'ISOLATED_PAGES':
        for slot in plan['slots']:
            path = run / 'inputs' / slot['prompt']
            packet = prompt_packet(path)
            require((packet['slot'], packet['page'], packet['type'], packet.get('requirement', {}).get('id')) ==
                    (slot['key'], slot['page'], slot['kind'], slot['requirement']),
                    'Isolated author packet identity differs from its frozen slot')
            operation_packet = refs.get(slot['requirement']) if slot['kind'] == 'OPERATION' else None
            structured_records(statement_rows, path.relative_to(run), packet['facts'], slot['citations'],
                               packet['citations'], refs, files, packet['sourceRanges'], operation_packet)
    elif plan['mode'] == 'WORKSPACE':
        source = run / 'inputs/author-source'
        for service in inventory['services']:
            home = source / service
            available = load(home / 'views.json')
            required = load(home / 'requirements.json')
            labelled = {('Page ' + row['page']): op for op, row in scopes.items() if row['service'] == service}
            allowed = {op: set(required[row['id']]['views']) for op, row in scopes.items() if row['service'] == service}
            file_views = {('Discovery file ' + row['file']): (files[row['id']], set(required[row['id']]['views']))
                          for row in inventory['requirements'] if row['kind'] == 'DISCOVERY' and row['service'] == service}
            for view in available.values():
                path = home / view['view']
                for line, labels, text, anchors in visible_view(path):
                    relevant = {op for label in labels if (op := labelled.get(label)) and view['view'] in allowed[op]}
                    matching_files = {file for label in labels if label in file_views
                                      for file, views in [file_views[label]] if view['view'] in views}
                    record(statement_rows, relevant, f'{path.relative_to(run)}:{line}', text, anchors, matching_files)
    else:
        raise ValueError('Unknown installed author delivery mode')
    return scopes, statement_rows


def linked_scope(key, answer, evidence, facts, eligible):
    """A file-level fact needs an explicit visible link from an eligible entry claim."""
    link = answer.get('scopeLinks', {}).get(key, {})
    via = link.get('via')
    symbol = link.get('symbol')
    if not facts[key].get('files') or via not in evidence or via not in facts or not isinstance(symbol, str):
        return False
    if not re.fullmatch(r'[A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)*', symbol):
        return False
    token = r'(?<![\w$])' + re.escape(symbol) + r'(?![\w$])'
    return (bool(eligible.intersection(facts[via]['scopes']))
            and bool(re.search(token, facts[via]['text']) and re.search(token, facts[key]['text'])))


def visible_in_operation(fact, scope):
    if fact.get('operationPacket') != scope:
        return False
    locations = fact.get('locations', {})
    for anchor in fact['anchors']:
        handle = anchor.split(' (lines ', 1)[0]
        if not re.fullmatch(r'sha256:[0-9a-f]{64}', locations.get(handle, {}).get('bodySha256', '')):
            return False
    return True


def coverage(report, review):
    require(report['runDigest'] == review['runDigest'] and report['rubricSha256'] == review['rubricSha256'],
            'Review targets a changed kit or rubric')
    require(isinstance(review['reviewer'], str) and bool(review['reviewer'].strip())
            and isinstance(review['materialFalseClaims'], list), 'Independent review identity and contradiction scan required')
    scores = review['criteria']
    require(scores.keys() == report['criteria'].keys(), 'Independent review must score every frozen atom')
    facts = report['statements']
    full, anchors = 0, set()
    for atom, row in report['criteria'].items():
        answer = scores[atom]
        status, parts = answer['status'], answer['witnesses']
        require(status in ('FULL', 'PARTIAL', 'ABSENT', 'CONTRADICTED'), f'Invalid grade for {atom}')
        components = {str(key) for key in row['components']}
        require(parts.keys() <= components, f'Unknown component of {atom}')
        if status == 'FULL':
            require(parts.keys() == components, f'Missing component of {atom}')
        require(status == 'FULL' or str(answer.get('reason', '')).strip(), f'Explain the gap for {atom}')
        eligible = {row['route'], *row['eligiblePeers']}
        for part, evidence in parts.items():
            require(isinstance(evidence, list) and evidence, f'Missing witnesses for {atom} component {part}')
            for key in evidence:
                require(key in facts, f'Stale or unavailable prepared statement for {atom} component {part}')
                fact = facts[key]
                require(report.get('mode') != 'ISOLATED_PAGES' or visible_in_operation(fact, row['route']),
                        f'Witness outside its isolated operation prompt or lacks visible source for {atom} component {part}')
                require(bool(eligible.intersection(fact['scopes'])) or linked_scope(key, answer, evidence, facts, eligible),
                        f'Wrong operation scope for {atom} component {part}')
                anchors.update(fact['anchors'])
        if status == 'FULL':
            require(row['route'] in report['scopes'], f'Missing installed entry scope for {atom}')
            full += 1
    return full, anchors


def check_sources(run, skill, repository, snapshot, anchors, statements=()):
    loader = cache(runpy.run_path(str(skill / 'scripts/author_citations.py'))['load'])
    manifests = {}
    for path in sorted((run / 'inputs/author-source').rglob('citations.json')):
        identity = load(path)['id'].removeprefix('sha256:')
        manifests.setdefault('m' + identity[:12], []).append(path)

    @cache
    def blob(file):
        return subprocess.check_output(['git', '-C', str(repository), 'cat-file', 'blob', f'{REVISION}:{file}'])

    @cache
    def source(handle):
        tag = handle.split(':', 1)[0]
        for path in manifests.get(tag, ()):
            manifest, packet, _, entries = loader(path)
            if handle in entries:
                entry = entries[handle]
                row = entry['source']
                require(entry['scope']['snapshot'] == snapshot and entry['scope']['service'] == row['service'],
                        f'Foreign snapshot or service for {handle}')
                require(entry['manifestId'] == manifest['id'] and row['revision'] == REVISION
                        and row['authority'] in {'EXACT_SNAPSHOT_TEXT', 'DECLARED_OPENAPI'},
                        f'Nonexact or foreign source for {handle}')
                outcome = next(item for item in packet['outcomes'] if item['source']['id'] == row['id'])
                raw = packet['texts'][outcome['textKey']].encode()
                require(outcome['source'] == row and sha(raw) == row['textDigest'], f'Digest mismatch for {handle}')
                full = blob(row['file'])
                occurrence = row.get('occurrence')
                if occurrence and 'blob' in occurrence:
                    identity = occurrence['blob']
                    # Compiler captures can name a retained CAS blob rather
                    # than a Git object. The manifest re-admits that identity.
                    require(isinstance(identity, str) and (re.fullmatch(r'sha256:[0-9a-f]{64}', identity)
                                                          or git_blob_matches(full, identity)),
                            f'Git blob identity changed for {handle}')
                span = source_span(full, raw, row['startLine'], row['endLine'], occurrence)
                require(span is not None, f'Source span mismatch for {handle}')
                return row, full, span
        raise ValueError(f'No installed retained manifest owns {handle}')

    slots = None

    @cache
    def delivered(path, handle):
        nonlocal slots
        if slots is None:
            plan = load(run / 'inputs/author-plan.json')
            slots = {'inputs/' + slot['prompt']: slot for slot in plan['slots']}
        require(path in slots, f'Isolated citation outside a frozen author packet for {handle}')
        packet, aliases = packet_citations(path)
        require(handle in aliases, f'Isolated citation not delivered to its packet for {handle}')
        alias = packet['citations'][aliases[handle]]
        row, full, span = source(handle)
        require(alias['authority'] == row['authority'], f'Isolated citation authority differs for {handle}')
        primary = alias['region']
        region_ids = alias.get('regions', [primary])
        require(isinstance(region_ids, list) and primary in region_ids, f'Isolated citation lacks its primary range for {handle}')
        ranges, location = [], None
        for identity in region_ids:
            region = packet['sourceRanges'][identity]
            require((region['service'], region['file']) == (row['service'], row['file']),
                    f'Isolated citation source identity differs for {handle}')
            first, last, raw = region['startLine'], region['endLine'], region['text'].encode()
            displayed = source_span(full, raw, first, last)
            require(displayed is not None, f'Isolated citation bytes differ from Git for {handle}')
            # Shared views can contain more than the original cite. Only their
            # byte intersection with that cite can support one of its anchors.
            start, end = max(span[0], displayed[0]), min(span[1], displayed[1])
            require(start < end, f'Isolated citation range outside retained source for {handle}')
            start_line = full.count(b'\n', 0, start) + 1
            ranges.append((start_line, start_line + full[start:end].removesuffix(b'\n').count(b'\n')))
            if identity == primary:
                location = {key: region[key] for key in ('service', 'file', 'startLine', 'endLine')}
                location.update(authority=alias['authority'], bodySha256=sha(raw))
        return ranges, location

    @cache
    def packet_citations(path):
        packet = prompt_packet(run / path)
        return packet, {handle: alias for alias, handle in slots[path]['citations'].items()}

    def anchor_source(anchor):
        require(bool(ANCHOR.fullmatch(anchor)), 'Invalid retained witness anchor')
        handle, _, lines = anchor.partition(' (lines ')
        row, _, _ = source(handle)
        intervals = []
        for interval in lines.removesuffix(')').split(', ') if lines else ():
            first, _, last = interval.partition('-')
            intervals.append((int(first), int(last or first)))
        require(all(row['startLine'] <= first <= last <= row['endLine'] for first, last in intervals),
                f'Witness outside retained source for {handle}')
        return handle, row, intervals

    for anchor in sorted(anchors):
        anchor_source(anchor)
    for statement in statements:
        authorities = set()
        locations = statement['locations']
        for anchor in statement['anchors']:
            handle, row, intervals = anchor_source(anchor)
            authorities.add(row['authority'])
            if locations:
                path = statement['location'].partition('#')[0]
                ranges, primary = delivered(path, handle)
                require(locations.get(handle) == primary, f'Isolated packet citation digest differs for {handle}')
                require(all(any(start <= first <= last <= end for start, end in ranges)
                            for first, last in intervals), f'Witness lines absent from its isolated packet for {handle}')
        if authorities == {'DECLARED_OPENAPI'}:
            require(openapi_declaration(statement['text']),
                    'Declared OpenAPI without implementation evidence cannot support an implementation statement')
    return len(anchors)


def openapi_declaration(text):
    kinds = {'openapi-declared-response', 'http-peer-declared-response'}
    if text.startswith('- ['):
        return text[3:].partition(']')[0].split(',')[0] in kinds
    fact = json.loads(text)
    return fact.get('authority') == 'DECLARED_OPENAPI' and fact.get('kind') in kinds


def git_blob_matches(blob, identity):
    match = re.fullmatch(r'(?:[0-9a-f]{40}|[0-9a-f]{64})', identity)
    if match is None:
        return False
    content = hashlib.new('sha1' if len(identity) == 40 else 'sha256')
    content.update(f'blob {len(blob)}\0'.encode())
    content.update(blob)
    return content.hexdigest() == identity


def source_span(blob, raw, first, last, occurrence=None):
    if not raw or type(first) is not int or type(last) is not int or not 0 < first <= last:
        return None
    if occurrence and type(occurrence.get('startByte')) is int and type(occurrence.get('endByte')) is int:
        start, end = occurrence['startByte'], occurrence['endByte']
        if (0 <= start < end <= len(blob) and blob[start:end] == raw
                and blob.count(b'\n', 0, start) + 1 == first
                and first + raw.removesuffix(b'\n').count(b'\n') == last):
            return start, end
        return None
    lines = blob.splitlines(keepends=True)
    if last > len(lines) or first + raw.removesuffix(b'\n').count(b'\n') != last:
        return None
    selected = b''.join(lines[first - 1:last])
    start = selected.find(raw)
    if 0 <= start < len(lines[first - 1]) and selected.find(raw, start + 1) < 0:
        start += sum(map(len, lines[:first - 1]))
        return start, start + len(raw)
    return None


def matches_source(blob, raw, first, last, occurrence):
    return source_span(blob, raw, first, last, occurrence) is not None


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(',', ':')).encode()


def release_report(payload, run, inventory, witness_catalog, review_path, decision):
    pins = load(run / 'pins.json')
    verifier = {file: sha((HERE / file).read_bytes()) for file in (
        'weather-preauthor.py', 'weather-release-payload.py', 'weather-preauthor-rubric.json',
        'weather-preauthor-components.tsv')}
    return {'schema': 'codeclew-installed-weather-release-qualification/1.0', **decision,
            'payload': payload, 'weatherRevision': REVISION, 'snapshot': inventory['snapshot'],
            'authorMode': witness_catalog['mode'],
            'evidence': {'frozenInputFilesDigest': sha(canonical(pins['inputFiles'])),
                         'inventoryDigest': pins['inventoryDigest'], 'promptDigest': pins['promptDigest'],
                         'reviewDigest': sha(review_path.read_bytes()), 'witnessCatalogDigest': sha(canonical(witness_catalog)),
                         'validatorFilesDigest': sha(canonical(verifier))}}


def validate_release_report(path, computed):
    require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 64 * 1024,
            'Qualification report is unavailable or oversized')
    require(computed['status'] == 'PASS' and computed['full'] == computed['required'] == 45
            and computed['materialFalseClaims'] == 0 and computed['verifiedAnchors'] > 0,
            'Installed Weather qualification requires 45/45 full atoms and zero material false claims')
    require(canonical(load(path)) == canonical(computed), 'Qualification report differs from exact archive or frozen evidence')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skill', type=Path, required=True, help='Authenticated installed skill directory')
    parser.add_argument('--runtime-key', required=True, help='Runtime key of the authenticated install')
    parser.add_argument('--run-dir', type=Path, required=True)
    parser.add_argument('--job', type=Path, help='Run ordinary installed prepare first using this job')
    parser.add_argument('--review', type=Path, help='Fresh independent semantic review JSON')
    parser.add_argument('--source-repo', type=Path, help='Git objects of pinned public Weather for review verification')
    parser.add_argument('--release-archive', type=Path, help='Produced official core archive to qualify or match')
    parser.add_argument('--release-commit', help='Exact peeled release tag commit expected in the archive')
    parser.add_argument('--verify-release-report', type=Path, help='Match a previous report by rechecking frozen evidence')
    args = parser.parse_args()
    require(bool(args.release_archive) == bool(args.release_commit), 'An archive and expected release commit are required together')
    require(not args.verify_release_report or args.release_archive and args.review and not args.job,
            'Release verification requires the target archive, frozen run, and independent review')
    need_git = args.review is not None
    require(not need_git or args.source_repo is not None, '--source-repo is required with --review')
    payload_module, payload = None, None
    if args.release_archive:
        payload_module = runpy.run_path(str(HERE / 'weather-release-payload.py'))
        payload, skill = payload_module['read_archive'](args.release_archive, args.release_commit)
        payload_module['check_install'](payload, skill, load(args.job or args.run_dir / 'inputs/job.json'), args.skill)
        require(args.runtime_key == payload['runtimeKey'], 'Requested runtime differs from the target release archive')
        if args.review:
            payload_module['digest_file'](args.review, 2 * 1024 * 1024)
    if args.job:
        subprocess.run([sys.executable, '-I', '-S', str(args.skill / 'scripts/docs_run.py'), 'prepare',
                        '--job', str(args.job), '--run-dir', str(args.run_dir)], check=True, stdout=sys.stderr)
    inventory = admitted(args.run_dir, args.skill, args.runtime_key)
    if payload_module:
        payload_module['check_pins'](payload, load(args.run_dir / 'pins.json'))
    plan = load(args.run_dir / 'inputs/author-plan.json')
    scopes, statements = collect(args.run_dir, inventory, plan)
    report = {'rubricSha256': RUBRIC_SHA, 'runDigest': sha((args.run_dir / 'pins.json').read_bytes()),
              'mode': plan['mode'], 'criteria': rubric(),
              'scopes': {op: {key: row[key] for key in ('id', 'page', 'service')} for op, row in scopes.items()},
              'statements': statements}
    if not need_git:
        if payload:
            report['targetPayload'] = payload
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return 0
    review = load(args.review)
    full, anchors = coverage(report, review)
    count = check_sources(args.run_dir, args.skill, args.source_repo, inventory['snapshot'], anchors, statements.values())
    false_count = len(review['materialFalseClaims'])
    passed = full == len(report['criteria']) and not false_count
    decision = {'status': 'PASS' if passed else 'FAIL', 'full': full, 'required': len(report['criteria']),
                'materialFalseClaims': false_count, 'verifiedAnchors': count,
                'runDigest': report['runDigest'], 'rubricSha256': RUBRIC_SHA}
    if payload:
        decision = release_report(payload, args.run_dir, inventory, report, args.review, decision)
    if args.verify_release_report:
        validate_release_report(args.verify_release_report, decision)
    print(json.dumps(decision, indent=2))
    return 0 if passed else 1


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (ValueError, OSError, KeyError, TypeError, subprocess.CalledProcessError, tarfile.TarError) as error:
        raise SystemExit(str(error)) from error
