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


def record(result, scopes, location, text, anchors):
    if scopes and anchors:
        row = {'scopes': sorted(scopes), 'location': location, 'text': text, 'anchors': anchors}
        key = sha(json.dumps(row, sort_keys=True, ensure_ascii=False).encode())
        result[key] = row


def collect(run, inventory, plan):
    statement_rows = {}
    scopes = select_scopes(inventory)
    if plan['mode'] == 'INLINE':
        packet = json.loads((run / 'prompt.txt').read_text().strip().rsplit('\n\n', 1)[1])
        refs = {row['id']: op for op, row in scopes.items()}
        for group in ('entries', 'files'):
            for ref, target in packet[group].items():
                for category in ('facts', 'boundaries'):
                    for i, fact in enumerate(target.get(category, [])):
                        relevant = {refs[value] for alias in (ref, *fact.get('alsoFor', ()))
                                    if (value := plan['entries'].get(alias)) in refs}
                        anchors = []
                        for alias in fact['sources']:
                            require(alias in packet['citations'], 'Inline cite alias absent from author prompt')
                            detail = fact.get('lines', {}).get(alias)
                            handle = plan['citations'][alias]
                            anchors.append(handle + (f' (lines {", ".join(detail)})' if detail else ''))
                        visible = {key: value for key, value in fact.items() if key not in {'sources', 'lines', 'alsoFor'}}
                        record(statement_rows, relevant, f'prompt.txt#{group}/{ref}/{category}/{i}',
                               json.dumps(visible, ensure_ascii=False, sort_keys=True), anchors)
    elif plan['mode'] == 'WORKSPACE':
        source = run / 'inputs/author-source'
        for service in inventory['services']:
            home = source / service
            available = load(home / 'views.json')
            required = load(home / 'requirements.json')
            labelled = {('Page ' + row['page']): op for op, row in scopes.items() if row['service'] == service}
            allowed = {op: set(required[row['id']]['views']) for op, row in scopes.items() if row['service'] == service}
            for view in available.values():
                path = home / view['view']
                for line, labels, text, anchors in visible_view(path):
                    relevant = {op for label in labels if (op := labelled.get(label)) and view['view'] in allowed[op]}
                    record(statement_rows, relevant, f'{path.relative_to(run)}:{line}', text, anchors)
    else:
        raise ValueError('Unknown installed author delivery mode')
    return scopes, statement_rows


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
                require(bool(eligible.intersection(fact['scopes'])), f'Wrong operation scope for {atom} component {part}')
                anchors.update(fact['anchors'])
        if status == 'FULL':
            require(row['route'] in report['scopes'], f'Missing installed entry scope for {atom}')
            full += 1
    return full, anchors


def check_sources(run, skill, repository, snapshot, anchors):
    loader = cache(runpy.run_path(str(skill / 'scripts/author_citations.py'))['load'])
    manifests = {}
    for path in sorted((run / 'inputs/author-source').rglob('citations.json')):
        identity = load(path)['id'].removeprefix('sha256:')
        manifests.setdefault('m' + identity[:12], []).append(path)

    @cache
    def source(handle):
        tag = handle.split(':', 1)[0]
        for path in manifests.get(tag, ()):
            _, packet, _, entries = loader(path)
            if handle in entries:
                entry = entries[handle]
                require(entry['scope']['snapshot'] == snapshot, f'Foreign snapshot for {handle}')
                row = entry['source']
                require(row['revision'] == REVISION and row['authority'] == 'EXACT_SNAPSHOT_TEXT',
                        f'Nonexact or foreign source for {handle}')
                outcome = next(item for item in packet['outcomes'] if item['source']['id'] == row['id'])
                raw = packet['texts'][outcome['textKey']].encode()
                require(sha(raw) == row['textDigest'], f'Digest mismatch for {handle}')
                return row, raw
        raise ValueError(f'No installed retained manifest owns {handle}')

    @cache
    def blob(file):
        return subprocess.check_output(['git', '-C', str(repository), 'cat-file', 'blob', f'{REVISION}:{file}'])

    for anchor in sorted(anchors):
        handle, _, ranges = anchor.partition(' (lines ')
        row, raw = source(handle)
        first, last = row['startLine'], row['endLine']
        full = blob(row['file'])
        require(matches_source(full, raw, first, last, row.get('occurrence')), f'Source span mismatch for {handle}')
        for location in ranges.removesuffix(')').split(', ') if ranges else ():
            a, _, b = location.partition('-')
            require(first <= int(a) <= int(b or a) <= last, f'Witness outside source location for {handle}')
    return len(anchors)


def matches_source(blob, raw, first, last, occurrence):
    if not raw or not 0 < first <= last:
        return False
    if occurrence and type(occurrence.get('startByte')) is int and type(occurrence.get('endByte')) is int:
        start, end = occurrence['startByte'], occurrence['endByte']
        return (0 <= start < end <= len(blob) and blob[start:end] == raw
                and blob.count(b'\n', 0, start) + 1 == first
                and first + raw.removesuffix(b'\n').count(b'\n') == last)
    lines = blob.splitlines(keepends=True)
    if last > len(lines) or first + raw.removesuffix(b'\n').count(b'\n') != last:
        return False
    selected = b''.join(lines[first - 1:last])
    start = selected.find(raw)
    return 0 <= start < len(lines[first - 1]) and selected.find(raw, start + 1) < 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skill', type=Path, required=True, help='Authenticated installed skill directory')
    parser.add_argument('--runtime-key', required=True, help='Runtime key of the authenticated install')
    parser.add_argument('--run-dir', type=Path, required=True)
    parser.add_argument('--job', type=Path, help='Run ordinary installed prepare first using this job')
    parser.add_argument('--review', type=Path, help='Fresh independent semantic review JSON')
    parser.add_argument('--source-repo', type=Path, help='Git objects of pinned public Weather for review verification')
    args = parser.parse_args()
    if args.job:
        subprocess.run([sys.executable, '-I', '-S', str(args.skill / 'scripts/docs_run.py'), 'prepare',
                        '--job', str(args.job), '--run-dir', str(args.run_dir)], check=True, stdout=sys.stderr)
    need_git = args.review is not None
    require(not need_git or args.source_repo is not None, '--source-repo is required with --review')
    inventory = admitted(args.run_dir, args.skill, args.runtime_key)
    plan = load(args.run_dir / 'inputs/author-plan.json')
    scopes, statements = collect(args.run_dir, inventory, plan)
    report = {'rubricSha256': RUBRIC_SHA, 'runDigest': sha((args.run_dir / 'pins.json').read_bytes()),
              'mode': plan['mode'], 'criteria': rubric(),
              'scopes': {op: {key: row[key] for key in ('id', 'page', 'service')} for op, row in scopes.items()},
              'statements': statements}
    if not need_git:
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return 0
    review = load(args.review)
    full, anchors = coverage(report, review)
    count = check_sources(args.run_dir, args.skill, args.source_repo, inventory['snapshot'], anchors)
    false_count = len(review['materialFalseClaims'])
    passed = full == len(report['criteria']) and not false_count
    print(json.dumps({'status': 'PASS' if passed else 'FAIL', 'full': full, 'required': len(report['criteria']),
                      'materialFalseClaims': false_count, 'verifiedAnchors': count,
                      'runDigest': report['runDigest'], 'rubricSha256': RUBRIC_SHA}, indent=2))
    return 0 if passed else 1


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (ValueError, OSError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error)) from error
