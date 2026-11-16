#!/usr/bin/env python3
"""Run fixed documentation gates over one frozen retained scope.

READY qualifies this finite mechanical profile, never semantic truth or runtime
behavior. Same-UID filesystem access is not a security boundary: an embedding
publisher must protect this runner, its frozen inputs and accepted outputs from
its author and must call promote rather than trust an author-provided report.
"""
from __future__ import annotations
import argparse
import fcntl
import hashlib
import json
import os
import re
from pathlib import Path
import runpy
import shutil
import signal
import subprocess
import sys
import time
import uuid

HERE = Path(__file__).resolve().parent
SKILL = HERE.parent
E = runpy.run_path(str(HERE / 'reader_export_check.py'))
M = E['M']
T = runpy.run_path(str(HERE / 'work_transport.py'))
U = runpy.run_path(str(HERE / 'source_union.py'))
W = runpy.run_path(str(HERE / 'docs_workspace.py'))
S = runpy.run_path(str(HERE / 'docs_successor.py'))
P = runpy.run_path(str(HERE / 'docs_topics.py'))
H = runpy.run_path(str(HERE / 'docs_show.py'))
SCHEMA = 'codeclew-docs-run-job/1.0'
REVIEW = 'codeclew-docs-run-review/1.0'
COMPACT_REVIEW = 'codeclew-docs-run-review/1.1'
CHECKS = ('inputs', 'author', 'output', 'provenance', 'entrySequences', 'selfReview', 'finalize', 'outputPins')


class Stop(Exception):
    def __init__(self, code, *, blocked=False, **details):
        self.code, self.blocked, self.details = code, blocked, details


def need(condition, code, **details):
    if not condition:
        raise Stop(code, **details)


def sha(raw):
    return 'sha256:' + hashlib.sha256(raw).hexdigest()


def encode(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + '\n').encode()


def digest(value):
    return sha(encode(value))


def _sum_usage(attempts):
    """Primary metric excludes cached input; retain all raw reported usage separately."""
    primary, lower_bound, missing, invalid = 0, 0, [], []
    fields = ('input_tokens', 'cached_input_tokens', 'output_tokens', 'reasoning_output_tokens')
    totals = dict.fromkeys(fields, 0)
    for number, attempt in enumerate(attempts):
        usage = attempt.get('usage')
        if usage is not None and not isinstance(usage, dict):
            invalid.append({'attempt': number, 'field': 'usage'})
        usage = usage if isinstance(usage, dict) else {}
        valid = {}
        for key in fields:
            value = usage.get(key)
            valid[key] = type(value) is int and value >= 0
            if key in usage and not valid[key]:
                invalid.append({'attempt': number, 'field': key})
            if not valid[key]:
                totals[key] = None
            elif totals[key] is not None:
                totals[key] += value
        if valid['input_tokens'] and valid['cached_input_tokens'] and usage['cached_input_tokens'] > usage['input_tokens']:
            invalid.append({'attempt': number, 'field': 'cached_input_tokens>input_tokens'})
        if valid['output_tokens'] and valid['reasoning_output_tokens'] and usage['reasoning_output_tokens'] > usage['output_tokens']:
            invalid.append({'attempt': number, 'field': 'reasoning_output_tokens>output_tokens'})
        required = ('input_tokens', 'cached_input_tokens', 'output_tokens')
        unavailable = [key for key in required if key not in usage]
        if unavailable:
            missing.append({'attempt': number, 'fields': unavailable})
        if valid['output_tokens']:
            lower_bound += usage['output_tokens']
        if valid['input_tokens'] and valid['cached_input_tokens'] and usage['cached_input_tokens'] <= usage['input_tokens']:
            lower_bound += usage['input_tokens'] - usage['cached_input_tokens']
        if all(valid[key] for key in required) and usage['cached_input_tokens'] <= usage['input_tokens']:
            primary += usage['input_tokens'] - usage['cached_input_tokens'] + usage['output_tokens']
    status = 'INVALID' if invalid else 'UNAVAILABLE' if missing else 'MEASURED'
    raw, output = totals['input_tokens'], totals['output_tokens']
    return {'metric': 'UNCACHED_INPUT_PLUS_OUTPUT', 'status': status,
            'budgetTokens': primary if status == 'MEASURED' else None,
            'knownBudgetTokensLowerBound': lower_bound, 'rawTotals': totals,
            'totalInputOutputFootprint': raw + output if raw is not None and output is not None else None,
            'missing': missing, 'invalid': invalid, 'monetaryCost': 'UNAVAILABLE'}


def token_accounting(attempts):
    """Codex terminal counters are cumulative within a thread, including resume.

    Preserve reports verbatim, derive each turn's delta, and sum distinct threads
    only once. A historical single report needs no subtraction. Ambiguous identity
    or missing/regressing counters cannot establish an exact within-budget result.
    """
    latest, bounds, missing, invalid, turns, reported = {}, {}, [], [], [], []
    unidentified = []
    for number, attempt in enumerate(attempts):
        usage = attempt.get('usage')
        checked = _sum_usage([{'usage': usage}])
        missing += [{**row, 'attempt': number} for row in checked['missing']]
        invalid += [{**row, 'attempt': number} for row in checked['invalid']]
        thread = attempt.get('threadId')
        reported.append({'attempt': number, 'threadId': thread, 'usage': usage})
        if not isinstance(thread, str) or not thread:
            if len(attempts) == 1:
                thread = 'historical-single-report'
            else:
                missing.append({'attempt': number, 'fields': ['threadId']})
                unidentified.append(checked['knownBudgetTokensLowerBound'])
                turns.append({'attempt': number, 'threadId': None, 'usage': None, 'budgetTokens': None})
                continue
        current = usage if isinstance(usage, dict) else {}
        previous = latest.get(thread)
        delta = {}
        for key in ('input_tokens', 'cached_input_tokens', 'output_tokens', 'reasoning_output_tokens'):
            value = current.get(key)
            if type(value) is not int or value < 0:continue
            prior = 0 if previous is None else previous.get(key)
            if type(prior) is not int or prior < 0:continue
            if value < prior:
                invalid.append({'attempt': number, 'field': key, 'code': 'CUMULATIVE_COUNTER_REGRESSED'})
                continue
            delta[key] = value - prior
        incremental = _sum_usage([{'usage': delta}])
        invalid += [{**row, 'attempt': number, 'scope': 'turnDelta'} for row in incremental['invalid']]
        latest[thread] = current
        bounds[thread] = max(bounds.get(thread, 0), checked['knownBudgetTokensLowerBound'])
        turns.append({'attempt': number, 'threadId': thread, 'usage': delta,
                      'budgetTokens': incremental['budgetTokens'] if checked['status'] == 'MEASURED' else None})
    aggregate = _sum_usage([{'usage': usage} for usage in latest.values()])
    status = 'INVALID' if invalid else 'UNAVAILABLE' if missing else 'MEASURED'
    aggregate.update(status=status, missing=missing, invalid=invalid,
                     budgetTokens=aggregate['budgetTokens'] if status == 'MEASURED' else None,
                     knownBudgetTokensLowerBound=max(sum(bounds.values()), max(unidentified, default=0)),
                     counterScope='THREAD_CUMULATIVE', reportedUsage=reported, attemptUsage=turns)
    if unidentified:
        aggregate['rawTotals'] = dict.fromkeys(aggregate['rawTotals'])
        aggregate['totalInputOutputFootprint'] = None
    return aggregate


def budget_tokens(accounting):
    return accounting['budgetTokens'] if accounting['budgetTokens'] is not None else accounting['knownBudgetTokensLowerBound']


def scoped_budget_tokens(state, accounting):
    baseline = state.get('successor', {}).get('inheritedModelTokens', 0)
    used = budget_tokens(accounting)
    need(used >= baseline, 'SUCCESSOR_ACCOUNTING_REGRESSION')
    return used - baseline


def load(path, maximum=32 * 1024 * 1024):
    path = Path(path)
    need(path.is_file() and not path.is_symlink() and path.stat().st_size <= maximum, 'INVALID_INPUT_FILE', path=str(path))
    return json.loads(path.read_bytes(), object_pairs_hook=T['unique_object'], parse_constant=T['reject_constant'])


def save(path, value):
    path = Path(path)
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = path.with_name('.' + path.name + '.' + uuid.uuid4().hex)
    temporary.write_bytes(encode(value))
    os.replace(temporary, path)


def closed(value, required, optional=()):
    need(isinstance(value, dict) and set(required) <= value.keys()
         and value.keys() <= set(required) | set(optional), 'INVALID_JOB_FIELDS')


def tree(root, *, skill=False):
    root = Path(root)
    need(root.is_dir() and not root.is_symlink(), 'INVALID_DIRECTORY', path=str(root))
    rows = {}
    for path in sorted(root.rglob('*')):
        relative = path.relative_to(root)
        if skill and any(p in {'node_modules', '__pycache__', '.git'} for p in relative.parts):
            continue
        need(not path.is_symlink(), 'SYMLINK_NOT_ALLOWED', path=str(relative))
        if path.is_file():
            need(len(rows) < 4096 and path.stat().st_size <= 32 * 1024 * 1024, 'ARTIFACT_LIMIT')
            rows[relative.as_posix()] = sha(path.read_bytes())
    need(sum((root / name).stat().st_size for name in rows) <= 128 * 1024 * 1024, 'ARTIFACT_LIMIT')
    return rows


def command(argv, directory, label, *, timeout=120):
    directory = Path(directory)
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    started = time.monotonic()
    receipt = {'argv': list(map(str, argv)), 'exitCode': None}
    try:
        process = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, start_new_session=True)
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except BaseException:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.communicate()
            raise
        receipt['exitCode'] = process.returncode
    except (OSError, subprocess.TimeoutExpired) as error:
        stdout, stderr = b'', str(error).encode()
        receipt['error'] = type(error).__name__
    receipt['seconds'] = time.monotonic() - started
    (directory / (label + '.stdout')).write_bytes(stdout)
    (directory / (label + '.stderr')).write_bytes(stderr)
    save(directory / (label + '.command.json'), receipt)
    try:
        value = json.loads(stdout) if stdout.strip().startswith((b'{', b'[')) else stdout.decode().strip()
    except (ValueError, UnicodeError):
        value = None
    if receipt['exitCode'] != 0:
        code = value.get('code', value.get('stageStatus', 'COMMAND_FAILED')) if isinstance(value, dict) else 'COMMAND_FAILED'
        raise Stop(code, blocked=receipt['exitCode'] is None, stage=label, exitCode=receipt['exitCode'], response=value)
    return value


def validate_job(job):
    closed(job, ('schema', 'request', 'clew', 'root', 'snapshot', 'services', 'language', 'renderer', 'node', 'author', 'budget'),
           ('components',))
    need(job['schema'] == SCHEMA and isinstance(job['request'], str) and job['request'].strip(), 'INVALID_JOB')
    need(job['language'] in ('en', 'ru'), 'INVALID_LANGUAGE')
    need(isinstance(job['services'], list) and 1 <= len(job['services']) <= 16
         and len(set(job['services'])) == len(job['services'])
         and all(isinstance(s, str) and re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]{0,63}', s) for s in job['services']), 'INVALID_SERVICES')
    need(isinstance(job['snapshot'], str) and job['snapshot'].startswith('sha256:'), 'EXACT_SNAPSHOT_REQUIRED')
    for key in ('clew', 'root', 'renderer', 'node'):
        need(isinstance(job[key], str) and Path(job[key]).is_absolute(), 'ABSOLUTE_LOCATOR_REQUIRED', field=key)
    closed(job['author'], ('executable', 'model', 'reasoningEffort'), ('stateRoot',))
    if 'stateRoot' in job['author']:
        need(Path(job['author']['stateRoot']).is_absolute(), 'ABSOLUTE_LOCATOR_REQUIRED', field='author.stateRoot')
    need(Path(job['author']['executable']).is_absolute() and job['author']['model']
         and job['author']['reasoningEffort'] in ('none', 'low', 'medium', 'high', 'xhigh', 'max'), 'INVALID_AUTHOR')
    closed(job['budget'], ('wallSeconds', 'maxModelTokens', 'maxRepairs'))
    for key, lower, upper in (('wallSeconds', 1, 7200), ('maxRepairs', 0, 2)):
        need(type(job['budget'][key]) is int and lower <= job['budget'][key] <= upper, 'INVALID_BUDGET', field=key)
    limit = job['budget']['maxModelTokens']
    need(limit is None or type(limit) is int and limit > 0, 'INVALID_BUDGET', field='maxModelTokens')
    if 'components' in job:
        closed(job['components'], ('path', 'digest'))
        need(Path(job['components']['path']).is_absolute(), 'ABSOLUTE_LOCATOR_REQUIRED')
    return job


def renderer_pins(job, receipts):
    actual, expected = tree(job['renderer'], skill=True), tree(SKILL / 'examples/markdown-renderer', skill=True)
    need(actual == expected, 'RENDERER_PACKAGE_DIFFERS_FROM_SKILL')
    probe = command([job['node'], str(HERE / 'docs_run_sequences.mjs'), job['renderer'], '--probe'], receipts, 'renderer-probe')
    return {'files': actual, 'parser': probe}


def requirements(service, rows, initial):
    coverage = [r for r in initial if r['kind'] == 'COVERAGE']
    need(len(coverage) == 1 and coverage[0].get('revision'), 'SERVICE_REVISION_REQUIRED', service=service)
    result, seen, page_names = [], set(), set()
    for row in rows:
        if row['kind'] != 'THREAD_ENTRY' or row['record'].get('classification') != 'DISCOVERED_TRIGGER':
            continue
        record = row['record']; entry = record.get('entrypoint')
        need(isinstance(entry, dict), 'INVALID_ENTRY_KIND', field='entrypoint')
        W['entry_kind']({'id': entry.get('id'), 'entryKind': entry.get('kind'),
                         'method': 'deferred', 'path': 'deferred'}, need)
        trigger = entry.get('trigger', {})
        routes = trigger.get('sourceRoutes') or [dict(method=m, path=p) for m in trigger.get('methods', []) for p in trigger.get('paths', [])]
        if entry['kind'] != 'HTTP_ENDPOINT':
            routes = [{}]
        need(routes, 'ENTRY_TRIGGER_IDENTITY_UNAVAILABLE', entrypoint=entry['id'])
        for route in routes:
            identity = service + ':' + entry['id'] + ':' + route.get('method', '') + ':' + route.get('path', '')
            if identity in seen:
                continue
            seen.add(identity)
            need(record.get('sourceIds', entry.get('sourceIds')), 'REQUIRED_SOURCE_COVERAGE_UNAVAILABLE', blocked=True, entrypoint=entry['id'])
            key = hashlib.sha256(identity.encode()).hexdigest()[:16]
            name = route.get('method', '') + '-' + route.get('path', trigger.get('name', entry['kind']))
            name = re.sub(r'(?<=[a-z0-9])(?=[A-Z])', '-', name)
            slug = re.sub(r'[^a-z0-9]+', '-', name.lower()).strip('-')
            page_name = service + '-' + (slug or 'entry') + '.md'
            if page_name in page_names:
                page_name = service + '-' + (slug or 'entry') + '-' + key[:8] + '.md'
            page_names.add(page_name)
            result.append({'id': identity, 'kind': 'ENTRY', 'service': service, 'entrypoint': entry['id'],
                           'entryKind': entry['kind'], 'page': page_name,
                           'handlerId': 'entry-' + key, 'method': route.get('method'), 'path': route.get('path'),
                           'sourceIds': record.get('sourceIds', entry.get('sourceIds', [])),
                           'sourceReferences': row.get('sourceReferences', []),
                           'rootDeclarations': record.get('rootDeclarations', []),
                           'authority': trigger.get('authority'), 'boundaries': entry.get('boundaries', [])})
    need(result, 'ENTRY_INVENTORY_EMPTY', service=service)
    for row in rows:
        if row['kind'] == 'THREAD_FILE' and row['record']['input'].get('coverage') != 'FILE_ONLY':
            file = row['record']['path']
            result.append({'id': service + ':discover:' + file, 'kind': 'DISCOVERY', 'service': service,
                           'page': service + '.md', 'file': file,
                           'fileFingerprint': row['record']['input']['digest'], 'sourceIds': []})
    result.append({'id': service + ':service-scope', 'kind': 'SERVICE', 'service': service,
                   'page': service + '.md', 'sourceIds': [],
                   'topics': ['contracts', 'data fields and domain distinctions', 'side effects', 'failures', 'configuration', 'unclassified source entry surfaces']})
    return result, coverage[0]


def source_projection(packet, entries):
    """One verbatim region per compatible occurrence; every original cite stays bound."""
    pairs, texts = [], {}
    by_source = {value['source']['id']: (handle, value) for handle, value in entries.items()}
    for outcome in packet['outcomes']:
        source = outcome['source']; handle, entry = by_source[source['id']]
        raw = packet['texts'][outcome['textKey']]
        need(sha(raw.encode()) == source['textDigest'], 'PREPARED_SOURCE_TEXT_CHANGED')
        texts[outcome['textKey']] = raw
        pairs.append(({'cite': handle}, outcome))
    projected, pool, metrics = U['compact'](pairs, texts, packet['scope'], include_reused=True)
    regions, bindings = {}, {}
    for row, (_, outcome) in zip(projected, pairs):
        handle, key = row['cite'], row['text']; source = outcome['source']
        region = regions.setdefault(key, {'file': source['file'], 'text': pool[key], 'citations': []})
        region['citations'].append(handle)
        bindings[handle] = {'source': source, 'sourceRecordDigest': outcome['sourceRecordDigest'],
                            'region': key, 'byteSlice': row['byteSlice']}
        raw = pool[key].encode()[row['byteSlice']['startByte']:row['byteSlice']['endByte']]
        need(raw == packet['texts'][outcome['textKey']].encode(), 'PREPARED_CITATION_RECONSTRUCTION_CHANGED')
    return regions, bindings, metrics


def prepare_sources(job, inventory, directory):
    def expired(*_):
        raise Stop('SOURCE_PREPARATION_WALL_LIMIT', blocked=True)
    previous = signal.signal(signal.SIGALRM, expired)
    timer = signal.setitimer(signal.ITIMER_REAL, 300)
    try:
        need(timer == (0.0, 0.0), 'SOURCE_PREPARATION_NESTED_TIMER_UNSUPPORTED')
        return _prepare_sources(job, inventory, directory)
    finally:
        signal.setitimer(signal.ITIMER_REAL, *timer)
        signal.signal(signal.SIGALRM, previous)


def compiler_source_pairs(choices):
    pairs = {}
    for row in choices:
        need(len(row['sourceIds']) == len(row['sourceReferences']), 'PREPARED_SOURCE_BINDING_AMBIGUOUS', id=row['id'])
        for identity, reference in zip(row['sourceIds'], row['sourceReferences']):
            need(identity not in pairs or pairs[identity] == reference, 'PREPARED_SOURCE_REFERENCE_CHANGED')
            pairs[identity] = reference
    return pairs


def _prepare_sources(job, inventory, directory):
    """Prepare supported retained reads serially, without an author or new capture."""
    started = time.monotonic(); directory.mkdir(mode=0o700)
    proof_root = directory.parent / 'source-proof'; proof_root.mkdir(mode=0o700)
    bounds = {'wallSeconds': 300, 'files': 128, 'visibleBytes': 8 * 1024 * 1024}
    services, all_bindings, count, visible_bytes = {}, {}, 0, 0
    M['configure_engine'](job['clew'], job['root'])

    def invoke(arguments, output, label):
        remaining = bounds['wallSeconds'] - (time.monotonic() - started)
        need(remaining > 0, 'SOURCE_PREPARATION_WALL_LIMIT')
        result = command(arguments, output, label, timeout=remaining)
        tree(directory); tree(proof_root)  # Keep both presentation and proof closures bounded.
        return result

    for service, scope in inventory['services'].items():
        home = directory / service; home.mkdir(mode=0o700)
        proof_home = proof_root / service; proof_home.mkdir(mode=0o700)
        resource_count = None
        base = [sys.executable, '-I', '-S', str(HERE / 'evidence_packet.py')]
        common = ['--clew', job['clew'], '--root', job['root'], '--service', service, '--snapshot', job['snapshot']]
        source_scope = any(r['kind'] == 'THREAD_SCOPE' for r in scope['sourceInventory'])
        action = 'discover-files' if source_scope else 'discover'
        invoke(base + [action, *common, '--work', scope['work'], '--output-dir', str(proof_home / 'discovery')], proof_home, 'discover')
        discovery = load(proof_home / 'discovery/packet.json')
        need(discovery.get('queryComplete'), 'PREPARED_DISCOVERY_INCOMPLETE', service=service)
        choices = discovery['choices']
        if source_scope:
            selection = [{'file': row['file']} for row in choices]
            count += len(selection)
            need(count <= bounds['files'], 'SOURCE_PREPARATION_FILE_LIMIT')
            save(proof_home / 'selection.json', selection)
            arguments = ['read-files', *common, '--packet', str(proof_home / 'discovery/packet.json')]
        else:
            pairs = compiler_source_pairs(choices)
            save(proof_home / 'resources-selection.json', {'query': {'kind': 'RETAINED_RESOURCE', 'projection': 'NAVIGATION'}})
            resources = invoke([sys.executable, '-I', '-S', str(HERE / 'work_session.py'), 'read',
                                '--launcher', job['clew'], '--root', job['root'], '--work', scope['work'],
                                '--input-root', str(proof_home), '--input', 'resources-selection.json',
                                '--output', 'resources', '--max-pages', '128'], proof_home, 'resources')
            need(resources['status'] == 'COMPLETE', 'PREPARED_RESOURCES_INCOMPLETE')
            resource_rows = load(proof_home / 'resources/read-collection/items.json')
            resource_count = len(resource_rows)
            for row in resource_rows:
                need(row['kind'] == 'SOURCE_NAVIGATION', 'PREPARED_RESOURCE_KIND_CHANGED')
                identity, reference = row['id'], row['record']['fullRecordReference']
                need(identity not in pairs or pairs[identity] == reference, 'PREPARED_SOURCE_REFERENCE_CHANGED')
                pairs[identity] = reference
            selection = [{'sourceId': identity, 'sourceReference': reference} for identity, reference in sorted(pairs.items())]
            save(proof_home / 'selection.json', selection)
            arguments = ['read-sources', *common, '--work', scope['work']]
        need(selection, 'PREPARED_SOURCE_SELECTION_EMPTY', service=service)
        invoke(base + [*arguments, '--selection', str(proof_home / 'selection.json'), '--source-union',
                       '--output-dir', str(proof_home / 'read')], proof_home, 'read')
        manifest = proof_home / 'read/citations.json'
        _, packet, _, entries = M['C']['load'](manifest)
        need(packet['scope']['snapshot'] == job['snapshot'] and packet['work'] == scope['work'], 'PREPARED_SOURCE_SCOPE_CHANGED')
        for handle, entry in entries.items():
            source = entry['source']
            need(source['service'] == service and source['revision'] == scope['revision'], 'PREPARED_SOURCE_REVISION_CHANGED')
            need(handle not in all_bindings, 'DUPLICATE_CITATION_HANDLE')
        regions, bindings, metrics = source_projection(packet, entries)
        if not source_scope:
            count += len({entry['source']['file'] for entry in entries.values()})
            need(count <= bounds['files'], 'SOURCE_PREPARATION_FILE_LIMIT')
        all_bindings.update({handle: {**value, 'manifest': str(manifest)} for handle, value in bindings.items()})
        views = home / 'views'; views.mkdir()
        view_index = {}
        for key, region in regions.items():
            raw = region['text'].encode()
            full = [h for h in region['citations'] if bindings[h]['byteSlice'] == {'startByte': 0, 'endByte': len(raw)}]
            lines = ['# ' + region['file'], 'Cite: ' + (full[0] if full else ', '.join(region['citations'])), '']
            fence = '`' * max(3, max((len(m[0]) + 1 for m in re.finditer(r'`+', region['text'])), default=3))
            lines += [fence, region['text'], fence, '']
            view = '\n'.join(lines); visible_bytes += len(view.encode())
            need(visible_bytes <= bounds['visibleBytes'], 'SOURCE_PREPARATION_VISIBLE_LIMIT')
            (views / (key + '.md')).write_text(view, encoding='utf-8')
            view_index[key] = {'file': region['file'], 'view': 'views/' + key + '.md', 'codeBytes': len(raw),
                               'coveringCitations': full, 'originalCitations': region['citations']}
        save(proof_home / 'citation-index.json', bindings)
        source_ids = {value['source']['id'] for value in bindings.values()}
        requirements_index, proofs, ready = {}, {}, {}
        source_texts = {o['source']['id']: packet['texts'][o['textKey']] for o in packet['outcomes']}
        source_entries = {h: {**entry, 'readText': source_texts[entry['source']['id']]}
                          for h, entry in entries.items()}
        for requirement in inventory['requirements']:
            if requirement['service'] != service:
                continue
            handles = [h for h, b in bindings.items() if (b['source']['id'] in requirement['sourceIds'] if requirement['kind'] == 'ENTRY'
                       else b['source']['file'] == requirement.get('file') if requirement['kind'] == 'DISCOVERY' else True)]
            need(handles, 'PREPARED_REQUIREMENT_SOURCE_MISSING', requirement=requirement['id'])
            requirements_index[requirement['id']] = handles
            if requirement['kind'] == 'ENTRY':
                need(set(requirement['sourceIds']) <= source_ids, 'PREPARED_ENTRY_SOURCE_MEMBERSHIP_MISSING', requirement=requirement['id'])
            if requirement['kind'] == 'DISCOVERY':
                proofs[requirement['id']] = full_file_proof(requirement, handles, source_entries, job['snapshot'])
            suggested = handles
            if requirement['kind'] == 'DISCOVERY':
                proof = proofs[requirement['id']]
                complete = [h for h in handles if bindings[h]['source'].get('occurrence', {}).get('startByte') == 0
                            and bindings[h]['source']['occurrence'].get('endByte') == proof['bytes']
                            and bindings[h]['source']['occurrence'].get('blob') == proof['gitBlob']
                            and bindings[h]['source']['textDigest'] == proof['textDigest']]
                if complete:
                    suggested = complete[:1]
            if requirement['kind'] != 'SERVICE':
                ready[requirement['id']] = {'page': requirement['page'], 'kind': requirement['kind'],
                    'evidenceCandidates': suggested,
                    'views': list(dict.fromkeys(view_index[bindings[h]['region']]['view'] for h in suggested)),
                    'meaningReviewed': False}
        save(home / 'requirements.json', ready)
        save(home / 'views.json', {k: {field: value for field, value in row.items() if field != 'originalCitations'} for k, row in view_index.items()})
        services[service] = {'work': scope['work'], 'revision': scope['revision'], 'coverage': scope['coverage'],
                             'sourceIndex': str(home / 'index.md'), 'views': view_index, 'manifest': str(manifest),
                             'citationIndex': str(proof_home / 'citation-index.json'), 'requirements': requirements_index,
                             'discoveryProofs': proofs, 'records': len(bindings), 'regions': len(regions), 'union': metrics,
                             'retainedResourceRecords': resource_count,
                             'inventoryKind': 'SOURCE_FILES' if source_scope else 'COMPILER_SYMBOLS_AND_RETAINED_RESOURCES',
                             'wholeServiceOrRuntimeCompletenessClaimed': False}
        lookup = ['# ' + service + ' source index', '', 'Open the individual source views needed for your explanation. Cite supplied handles directly.',
                  'Ready requirement IDs, views and cite handles: requirements.json.',
                  'Capture coverage: ' + scope['coverage']['coverage'] + '. No runtime proof.', '']
        if metrics['fallbacks']:
            lookup += ['Some original fragments lack compatible exact coordinates; they remain separate, including overlapping text.', '']
        if resource_count is not None:
            lookup += ['Compiler Work retained-resource query returned ' + str(resource_count) + ' records. Files outside retained inputs remain unknown.', '']
        lookup += ['## Required entry and discovery reads', '']
        for requirement in inventory['requirements']:
            if requirement['id'] in ready:
                row = ready[requirement['id']]
                lookup.append('- ' + requirement.get('file', requirement['page']) + ': ' + ', '.join(row['views']) + '; cite ' + ', '.join(row['evidenceCandidates']))
        lookup += ['', '## All retained source regions', '']
        for region in view_index.values():
            lookup.append('- ' + region['view'] + ': ' + region['file'] + ' (' + str(region['codeBytes']) + ' bytes of code)')
        lookup += ['', 'Open large resource bodies when needed; all remain available, with no scope filter or truncation.',
                   'Read code views and cite their supplied handles. Raw citation/proof sidecars are for targeted diagnostics, not routine reading.',
                   'These mappings are input navigation, not semantic self-review or registration proof.']
        (home / 'index.md').write_text('\n'.join(lookup) + '\n', encoding='utf-8')
    need(time.monotonic() - started <= bounds['wallSeconds'], 'SOURCE_PREPARATION_WALL_LIMIT')
    (directory / 'index.md').write_text('# Prepared retained sources\n\n' + '\n'.join(
        '- ' + service + ': ' + service + '/index.md; individual code views in ' + service + '/views/'
        for service in services) + '\n\nNative inventory and required self-review are unchanged. '
        'Use these prepared sources before acquiring additional retained evidence. '
        'Raw proof packages and metadata are sidecars; no old authored pages are included.\n', encoding='utf-8')
    visible_bytes = sum(p.stat().st_size for p in directory.rglob('*.md'))
    need(visible_bytes <= bounds['visibleBytes'], 'SOURCE_PREPARATION_VISIBLE_LIMIT')
    tree(directory)
    result = {'status': 'PREPARED_RETAINED_SOURCES', 'bounds': bounds, 'services': services,
              'snapshot': job['snapshot'], 'requirements': inventory['requirements'],
              'seconds': time.monotonic() - started, 'visibleBytes': visible_bytes, 'sourceFiles': count,
              'sourceBindings': len(all_bindings), 'semanticTruthAssessed': False}
    save(proof_root / 'index.json', result)
    save(directory / 'index.json', {
        'services': {service: {'index': service + '/index.md', 'requirements': service + '/requirements.json'}
                     for service in services},
        'scope': 'Retained source only; read the service indexes and individual code views. Proof metadata is not narrative context.'})
    return result


def author_inventory(inventory):
    """Task addresses and coverage, without native proof bookkeeping.

    The complete immutable inventory remains the authority for every gate.
    Source text, cite bindings and requirement identities are unchanged.
    """
    fields = ('id', 'kind', 'service', 'page', 'handlerId', 'method', 'path',
              'entryKind', 'file', 'topics', 'authority', 'boundaries')
    return {'schema': 'codeclew-docs-run-author-inventory/1.0', 'snapshot': inventory['snapshot'],
            'scope': inventory['scope'],
            'services': {service: {'revision': scope['revision'], 'coverage': scope['coverage']}
                         for service, scope in inventory['services'].items()},
            'requirements': [{key: row[key] for key in fields if key in row}
                             for row in inventory['requirements']]}


def navigation_starter(inventory):
    """Derive page ownership and entry addresses; author supplies reader labels."""
    services = []
    for service in inventory['services']:
        rows = [r for r in inventory['requirements'] if r['service'] == service]
        overview = next(r['page'] for r in rows if r['kind'] == 'SERVICE')
        starts = []
        for row in rows:
            if row['kind'] != 'ENTRY':
                continue
            label = (row['method'] + ' ' + row['path']) if row.get('method') else row['entryKind']
            kind = W['entry_kind'](row, need)
            starts.append({'title': label, 'target': row['page'] + '#handler', 'kind': kind,
                           'summary': W['START_SUMMARY']})
        services.append({'id': service, 'title': service, 'overview': overview,
                         'pages': list(dict.fromkeys(r['page'] for r in rows)), 'starts': starts, 'data': []})
    return {'schema': 'codeclew-markdown-navigation/1',
            'home': {'title': 'Overview', 'target': services[0]['overview']}, 'services': services, 'routes': []}


def prepare(job_path, run):
    job = validate_job(load(job_path)); run = Path(run).absolute()
    need(not run.exists(), 'RUN_ALREADY_EXISTS')
    run.mkdir(mode=0o700, parents=True)
    inputs = run / 'inputs'; inputs.mkdir(mode=0o700)
    save(inputs / 'job.json', job)
    version = command([job['clew'], '--version'], inputs / 'receipts', 'clew-version')
    capabilities = command([job['clew'], 'capabilities'], inputs / 'receipts', 'clew-capabilities')
    need(isinstance(capabilities, dict) and capabilities.get('runtimeMode') == 'RELEASE', 'SUPPORTED_RELEASE_RUNTIME_REQUIRED', blocked=True)
    renderer = renderer_pins(job, inputs / 'receipts')
    scopes, expected = {}, []
    request = {'schema': 'codeclew-documentation-work-request/1.0', 'audience': 'Service documentation',
               'documentationLanguage': job['language'], 'maxItems': 100, 'maxBytes': 49152}
    # --language is the supported CLI selector; keep the request schema portable.
    request.pop('documentationLanguage')
    save(inputs / 'request.json', request)
    for service in job['services']:
        directory = inputs / service; directory.mkdir(mode=0o700)
        page = command([job['clew'], 'docs', 'work', 'prepare', '--root', job['root'], '--subject', 'service:' + service,
                        '--snapshot', job['snapshot'], '--language', job['language'], '--input', str(inputs / 'request.json')], directory, 'prepare')
        need(page.get('snapshot') == job['snapshot'], 'SNAPSHOT_MISMATCH')
        work = page['work']
        initial = T['collect'](job['clew'], job['root'], work, directory / 'initial')
        selected = T['collect'](job['clew'], job['root'], work, directory / 'threads',
                                kind='work-selection', selection={'query': {'kind': 'THREAD_INVENTORY'}})
        for collection in (initial, selected):
            need(collection['status'] == 'COMPLETE' and not collection['omittedRecords'], 'INCOMPLETE_REQUIRED_INVENTORY', service=service)
            need(collection['identity']['snapshot'] == job['snapshot'], 'SNAPSHOT_MISMATCH')
        initial_rows = load(directory / 'initial/items.json')
        rows = load(directory / 'threads/items.json')
        members, coverage = requirements(service, rows, initial_rows)
        expected.extend(members)
        scopes[service] = {'work': work, 'revision': coverage['revision'],
                           'contextDigest': selected['identity']['contextDigest'], 'inputDigest': selected['identity']['inputDigest'],
                           'coverage': coverage, 'sourceInventory': [r for r in rows if r['kind'] in ('THREAD_SCOPE', 'THREAD_FILE')]}
    inventory = {'schema': 'codeclew-docs-run-inventory/1.0', 'snapshot': job['snapshot'], 'services': scopes,
                 'requirements': expected, 'scope': 'Retained captured source; unclassified surfaces and runtime registration remain explicit limits.'}
    save(inputs / 'inventory.json', inventory)
    prepare_sources(job, inventory, inputs / 'author-source')
    workspace = run / 'workspace'; workspace.mkdir(mode=0o700)
    (workspace / 'draft').mkdir(mode=0o700)
    W['install'](inventory, run, globals())
    author_files = W['install_author_entry'](run, SKILL, sys.executable)
    pins = {'jobDigest': digest(job), 'inventoryDigest': digest(inventory), 'skillFiles': tree(SKILL, skill=True),
            'renderer': renderer, 'clewVersion': version, 'clewCapabilities': capabilities,
            'authorExecutableDigest': sha(Path(job['author']['executable']).read_bytes()),
            'nodeExecutableDigest': sha(Path(job['node']).read_bytes()),
            'authorSkillsConfig': W['author_skill_config'](run),
            'authorWorkspaceFiles': {name: sha((workspace / name).read_bytes()) for name in author_files}}
    if 'components' in job:
        pins['componentTicketDigest'] = sha(Path(job['components']['path']).read_bytes())
    pins['inputFiles'] = tree(inputs)
    save(run / 'pins.json', pins)
    author_input = author_inventory(inventory)
    save(workspace / 'inventory.json', author_input)
    save(workspace / 'draft/navigation.json', navigation_starter(inventory))
    pins['authorInventoryDigest'] = digest(author_input)
    prompt = (SKILL / 'templates/docs-run-prompt.txt').read_text().format(
        request=job['request'], language=job['language'], skill=str(SKILL), clew=job['clew'], root=job['root'], run=str(run),
        snapshot=job['snapshot'], inventory=str(workspace / 'inventory.json'), draft=str(workspace / 'draft'),
        prepared_sources=str(inputs / 'author-source/index.md'),
        model=job['author']['model'], repairs=job['budget']['maxRepairs'])
    (run / 'prompt.txt').write_text(prompt, encoding='utf-8')
    pins['promptDigest'] = sha(prompt.encode()); save(run / 'pins.json', pins)
    save(run / 'state.json', {'schema': 'codeclew-docs-run-state/1.0', 'status': 'PREPARED', 'attempts': [],
                              'modelTokens': 0, 'usageAvailable': True, 'authorStartedAt': None})
    return {'status': 'PREPARED', 'run': str(run), 'requirements': len(expected), 'inputDigest': digest(pins)}


def frozen_inputs(run, *, skill=SKILL, prepared_origin=None):
    """Read immutable input pins without native commands or receipt writes."""
    job = validate_job(load(run / 'inputs/job.json')); pins = load(run / 'pins.json')
    need(digest(job) == pins['jobDigest'] and digest(load(run / 'inputs/inventory.json')) == pins['inventoryDigest']
         and tree(run / 'inputs') == pins['inputFiles'] and tree(skill, skill=True) == pins['skillFiles']
         and sha((run / 'prompt.txt').read_bytes()) == pins['promptDigest']
         and sha(Path(job['author']['executable']).read_bytes()) == pins['authorExecutableDigest']
         and sha(Path(job['node']).read_bytes()) == pins['nodeExecutableDigest']
         and digest(load(run / 'workspace/inventory.json')) == pins.get('authorInventoryDigest', pins['inventoryDigest']), 'FROZEN_INPUTS_CHANGED')
    need(tree(job['renderer'], skill=True) == pins['renderer']['files'], 'RENDERER_CHANGED')
    for name, expected in pins.get('authorWorkspaceFiles', {}).items():
        path = run / 'workspace' / name
        need(path.is_file() and not path.is_symlink() and sha(path.read_bytes()) == expected,
             'PREPARED_AUTHOR_ENTRY_CHANGED', file=name)
    if 'authorSkillsConfig' in pins:
        need(pins['authorSkillsConfig'] == W['author_skill_config'](prepared_origin or run),
             'PREPARED_AUTHOR_ENTRY_CHANGED', file='authorSkillsConfig')
    if 'components' in job:
        need(sha(Path(job['components']['path']).read_bytes()) == pins['componentTicketDigest'], 'COMPONENT_TICKET_CHANGED')
    return job, load(run / 'inputs/inventory.json'), pins


def inputs_unchanged(run, *, skill=SKILL):
    job, inventory, pins = frozen_inputs(run, skill=skill)
    current_version = command([job['clew'], '--version'], run / 'runtime-checks', uuid.uuid4().hex)
    need(current_version == pins['clewVersion'], 'ADMITTED_RUNTIME_CHANGED', blocked=True)
    parser = subprocess.run([job['node'], str(HERE / 'docs_run_sequences.mjs'), job['renderer'], '--probe'], capture_output=True)
    need(parser.returncode == 0 and json.loads(parser.stdout) == pins['renderer']['parser'], 'RENDERER_PARSER_CHANGED')
    return job, inventory, pins


def recover_accounting(run, previous_skill):
    """Trusted caller migration after the old summed cumulative counters stop.

    Only a completed unit's proven false budget stop is recoverable. Preserve
    raw history and every original limit; no author invocation occurs here.
    """
    run = Path(run).absolute()
    job, _, old_pins = inputs_unchanged(run, skill=previous_skill)
    old_state = load(run / 'state.json')
    need(old_state['status'] == 'AUTHOR_BUDGET_EXCEEDED', 'ACCOUNTING_RECOVERY_NOT_APPLICABLE')
    attempts = old_state['attempts']
    need(attempts and all(a.get('status') == 'COMPLETED' and a.get('exitCode') == 0 for a in attempts),
         'COMPLETED_AUTHOR_REQUIRED')
    need(tree(run / 'workspace/draft') == attempts[-1]['draftFiles'], 'ACCEPTED_DRAFT_CHANGED')
    updated = json.loads(json.dumps(old_state))
    thread = old_state.get('threadId')
    need(isinstance(thread, str) and thread, 'AUTHOR_THREAD_REQUIRED')
    logs = []
    for number, attempt in enumerate(updated['attempts']):
        path = run / 'attempts' / str(number) / 'stdout.jsonl'
        need(path.is_file() and not path.is_symlink() and path.stat().st_size <= 128 * 1024 * 1024,
             'AUTHOR_HISTORY_REQUIRED')
        starts, completed = [], []
        for line in path.read_text().splitlines():
            try:event = json.loads(line)
            except ValueError:continue
            if event.get('type') == 'thread.started':starts.append(event.get('thread_id'))
            if event.get('type') == 'turn.completed':completed.append(event.get('usage'))
        need(starts == [thread] and completed == [attempt.get('usage')], 'AUTHOR_HISTORY_MISMATCH', attempt=number)
        argv = attempt['argv']
        need(('resume' not in argv if number == 0 else
              'resume' in argv and argv[argv.index('resume') + 1] == thread), 'AUTHOR_RESUME_CHAIN_MISMATCH')
        need(attempt.get('threadId', thread) == thread, 'AUTHOR_THREAD_CHANGED')
        attempt['threadId'] = thread
        logs.append({'attempt': number, 'stdoutDigest': sha(path.read_bytes())})
    old_usage = _sum_usage(attempts)
    corrected = token_accounting(updated['attempts'])
    need(old_usage['status'] == corrected['status'] == 'MEASURED', 'AUTHOR_USAGE_UNAVAILABLE')
    maximum = job['budget']['maxModelTokens']
    need(type(maximum) is int and old_usage['budgetTokens'] == old_state['modelTokens'] > maximum
         and corrected['budgetTokens'] < maximum, 'FALSE_BUDGET_STOP_NOT_PROVEN')
    need(type(old_state.get('authorStartedAt')) in (int, float)
         and 0 <= time.time() - old_state['authorStartedAt'] < job['budget']['wallSeconds'], 'AUTHOR_WALL_BUDGET_EXHAUSTED')
    need(sum(a.get('kind') == 'repair' for a in attempts) <= job['budget']['maxRepairs'], 'AUTHOR_REPAIR_BUDGET_EXHAUSTED')
    units = load(run / 'inputs/scaffold.json')['units']; cursor = old_state.get('unitCursor', 0)
    need(type(cursor) is int and 0 <= cursor < len(units) and attempts[-1].get('kind') == 'unit'
         and old_state.get('completedUnit') == attempts[-1].get('unit') == units[cursor]['id'], 'COMPLETED_UNIT_REQUIRED')
    folder = run / 'accounting-recoveries' / uuid.uuid4().hex; folder.mkdir(mode=0o700, parents=True)
    for name in ('state.json', 'pins.json'):
        (folder / ('original-' + name)).write_bytes((run / name).read_bytes())
    new_pins = {**old_pins, 'skillFiles': tree(SKILL, skill=True)}
    updated.update(status='AUTHOR_COMPLETED', modelTokens=corrected['budgetTokens'], usageAvailable=True,
                   tokenAccounting=corrected, accountingRecovery=str(folder.relative_to(run)))
    receipt = {'status': 'PREPARED', 'counterScope': 'THREAD_CUMULATIVE', 'threadId': thread, 'logs': logs,
               'oldUsage': old_usage, 'correctedUsage': corrected, 'originalStartedAt': old_state['authorStartedAt'],
               'originalBudget': job['budget'], 'originalPinsDigest': digest(old_pins), 'successorPinsDigest': digest(new_pins),
               'previousSkill': str(Path(previous_skill).absolute()), 'successorSkill': str(SKILL),
               'modelCalls': 0, 'ready': False, 'semanticTruthAssessed': False}
    save(folder / 'receipt.json', receipt)
    save(run / 'pins.json', new_pins); save(run / 'state.json', updated)
    receipt['status'] = 'ACCOUNTING_RECOVERED'; save(folder / 'receipt.json', receipt)
    return {**receipt, 'receipt': str(folder / 'receipt.json')}


def review_for_run(run):
    _, inventory, _ = frozen_inputs(run)
    result = review_template(inventory, load(run / 'workspace/draft/original.json'))
    if (run / 'inputs/scaffold.json').exists():
        for row in inventory['requirements']:
            if row['kind'] != 'ENTRY':result['requirements'][row['id']]['section'] = 'clew-scope'
    original = load(run / 'workspace/draft/original.json')
    reviewed_path = run / 'workspace/draft/reviewed-pages.json'
    previous_path = run / 'workspace/draft/self-review.json'
    if reviewed_path.exists() and previous_path.exists():
        reviewed, previous = load(reviewed_path), load(previous_path)
        if previous.get('schema') == COMPACT_REVIEW:
            for page in original['pages']:
                if reviewed.get(page['page']) != digest(page):continue
                for requirement in inventory['requirements']:
                    if requirement['page'] == page['page'] and requirement['id'] in previous['requirements']:
                        result['requirements'][requirement['id']] = {**result['requirements'][requirement['id']],
                                                                   **previous['requirements'][requirement['id']]}
                if page['page'] in result['claims']:
                    result['claims'][page['page']] = previous['claims'].get(page['page'], result['claims'][page['page']])
            result['unresolved'], result['visual'] = previous['unresolved'], previous['visual']
    for account in W['registered_accounts'](run, globals()):
        row = result['requirements'][account['requirement']]
        dispositions = {c['id']: c.get('disposition') for c in row.get('discoveredEntries') or []}
        row['discoveredEntries'] = [{**c, 'disposition': dispositions.get(c['id'])} for c in account['discoveredEntries']]
    for requirement in inventory['requirements']:
        path = run / 'workspace/notes' / (requirement['page'] + '.json')
        if requirement['kind'] == 'SERVICE' and path.exists():
            result['requirements'][requirement['id']]['sourceNotes'] = load(path)['fragments'][0]
    return result


def check_source_membership(job, inventory, entry, checks, cache):
    """Reload identity without requiring a large SOURCE body on stdout.

    Complete saved body/part proof is still checked by C.load. SOURCE navigation
    supplies only an independently reloaded Work record digest and metadata; it
    is never treated as a new body read or source evidence.
    """
    source=entry['source'];service=source['service']
    keys=('id','service','revision','file','textDigest','startLine','endLine','occurrence','authority','evidenceDigest','textOrigin')
    source_scope=any(r['kind']=='THREAD_SCOPE' for r in inventory['services'][service].get('sourceInventory',[]))
    if source_scope:
        work=entry.get('work');need(work,'SOURCE_WORK_IDENTITY_REQUIRED')
        key=(work,source['file'])
        if key not in cache:
            directory=Path(checks)/('source-navigation-'+str(len(cache)))
            result=T['collect'](job['clew'],job['root'],work,directory,kind='work-selection',
                selection={'query':{'kind':'SOURCE','projection':'NAVIGATION','file':source['file']}})
            need(result['status']=='COMPLETE' and not result['omittedRecords']
                 and result['identity']['snapshot']==job['snapshot'],'SOURCE_MEMBERSHIP_NAVIGATION_INCOMPLETE')
            rows=load(directory/'items.json')
            need(all(r['kind']=='SOURCE_NAVIGATION' for r in rows),'SOURCE_MEMBERSHIP_NAVIGATION_CHANGED')
            need(len({r['id'] for r in rows})==len(rows),'SOURCE_MEMBERSHIP_NAVIGATION_AMBIGUOUS')
            cache[key]={r['id']:r['record'] for r in rows}
        row=cache[key].get(source['id'])
        need(row is not None and row.get('authority')=='NAVIGATION_ONLY' and row.get('textDelivered') is False
             and row.get('recordDigest')==entry['sourceRecordDigest']
             and all(row.get('source',{}).get(k)==source.get(k) for k in keys),
             'PROVENANCE_MISMATCH',source=source['id'])
        return
    # Compiler Work without SOURCE_SCOPE keeps its own supported context route.
    # Never retry unsupported file inventory or silently substitute syntax.
    page=command([job['clew'],'docs','context','--root',job['root'],'--service',service,
        '--snapshot',job['snapshot'],'--source',source['id'],'--format','compact','--limit','100'],
        checks,'source-'+str(len(cache)))
    records=[r['record'] for r in page.get('items',[]) if r.get('kind')=='SOURCE' and r.get('id')==source['id']]
    need(page.get('selectedSnapshot',page.get('snapshot'))==job['snapshot'] and len(records)==1
         and all(records[0].get(k)==source.get(k) for k in keys),'PROVENANCE_MISMATCH',
         source=source['id'],omitted=page.get('omitted',[]))
    cache[(service,source['id'])]=True


def cite_entries(job, inventory, draft, checks, prepared=None):
    M['configure_engine'](job['clew'], job['root'])
    manifests = sorted((draft / 'reads').rglob('citations.json'))
    if prepared is not None:
        manifests += sorted(prepared.rglob('citations.json'))
        proof = prepared.parent / 'source-proof'
        if proof.exists():manifests += sorted(proof.rglob('citations.json'))
    need(manifests, 'SOURCE_READS_REQUIRED')
    entries = {}
    for path in manifests:
        manifest, packet, _, selected = M['C']['load'](path)
        for handle, entry in selected.items():
            source, scope = entry.get('source', {}), entry.get('scope', packet.get('scope', {}))
            service = source.get('service')
            need(service in inventory['services'] and scope.get('snapshot') == job['snapshot']
                 and source.get('revision') == inventory['services'][service]['revision'], 'PROVENANCE_MISMATCH', handle=handle)
            need(handle not in entries, 'DUPLICATE_CITATION_HANDLE')
            matches = [o for o in packet['outcomes'] if o['source']['id'] == source['id']]
            need(len(matches) == 1 and matches[0]['textKey'] in packet['texts'], 'CITATION_TEXT_UNAVAILABLE')
            entries[handle] = {**entry, 'readText': packet['texts'][matches[0]['textKey']]}
    # Native source membership is reloaded independently of author-written proof.
    membership_cache={}
    for entry in {(v['source']['service'], v['source']['id']): v for v in entries.values()}.values():
        if entry.get('nativeBinding', {}).get('kind') == 'SOURCE_NATIVE':
            continue  # C.load already reloaded the exact registered native bytes.
        check_source_membership(job,inventory,entry,checks,membership_cache)
    return manifests, entries


def full_file_proof(requirement, handles, entries, snapshot):
    selected, texts = [], {}
    for handle in handles:
        entry = entries[handle]; source = entry['source']
        if source['service'] != requirement['service'] or source['file'] != requirement['file']:
            continue
        text = entry['readText']; key = sha(text.encode())
        texts[key] = text
        selected.append(({'cite': handle}, {'source': source, 'textKey': key}))
    need(selected, 'DISCOVERY_FULL_FILE_READ_REQUIRED', requirement=requirement['id'])
    projected, pool, metadata = U['compact'](selected, texts, {'snapshot': snapshot}, include_reused=True)
    need(not metadata['fallbacks'], 'DISCOVERY_EXACT_BYTE_OCCURRENCES_REQUIRED', requirement=requirement['id'])
    groups = {}
    for row, (_, outcome) in zip(projected, selected):
        occurrence = outcome['source']['occurrence']
        raw = pool[row['text']].encode()
        start = occurrence['startByte'] - row['byteSlice']['startByte']
        identity = (occurrence['snapshot'], occurrence['blob'])
        groups.setdefault(identity, {})[(start, len(raw))] = raw
    for (_, blob), regions in groups.items():
        if not isinstance(blob, str) or len(blob) not in (40, 64) or any(c not in '0123456789abcdef' for c in blob):
            continue
        content, cursor = bytearray(), 0
        for (start, size), raw in sorted(regions.items()):
            if start != cursor:
                break
            content.extend(raw); cursor += size
        else:
            header = ('blob ' + str(len(content)) + '\0').encode()
            actual = hashlib.new('sha1' if len(blob) == 40 else 'sha256', header + content).hexdigest()
            if actual == blob:
                return {'status': 'COMPLETE_RETAINED_GIT_BLOB', 'gitBlob': blob, 'bytes': len(content),
                        'textDigest': sha(content), 'sourceHandles': list(handles),
                        'scopeFingerprint': requirement['fileFingerprint'], 'semanticTruthAssessed': False}
    raise Stop('DISCOVERY_FULL_FILE_READ_REQUIRED', requirement=requirement['id'],
               reason='Agreeing retained byte ranges do not reconstruct their complete native Git blob. Syntax fingerprints are not text digests.')


def review_template(inventory, original):
    requirements = {}
    for requirement in inventory['requirements']:
        row = {'disposition': None, 'section': 'handler' if requirement['kind'] == 'ENTRY' else None}
        if requirement['kind'] == 'ENTRY':
            row['diagram'] = 0
        if requirement['kind'] == 'DISCOVERY':
            row.update(discoveryDisposition=None, discoveredEntries=None)
        if requirement['kind'] == 'SERVICE':
            row['topics'] = {topic: {'disposition': None, 'page': None, 'claim': None} for topic in requirement['topics']}
            row['sourceNotes'] = None
        requirements[requirement['id']] = row
    claims = {}
    for page in original['pages']:
        fragments = {fragment['id']: None for fragment in page['fragments'] if 'id' in fragment}
        need(len(fragments) == sum('id' in fragment for fragment in page['fragments']), 'DUPLICATE_REVIEW_CLAIM')
        if fragments:
            need(page['page'] not in claims, 'DUPLICATE_REVIEW_PAGE')
            claims[page['page']] = fragments
    return {'schema': COMPACT_REVIEW, 'requirements': requirements, 'claims': claims,
            'unresolved': None, 'visual': {'status': None}}


def local_section(value):
    if isinstance(value, str) and '#' in value:
        need(value.startswith('#') and value.count('#') == 1 and len(value) > 1,
             'SELF_REVIEW_SECTION_MUST_BE_LOCAL_ANCHOR')
        return value[1:]
    return value


def resolve_review(review, inventory, entries, original, defaults=None):
    """Derive duplicate identifiers/evidence, never an author's meaning decision."""
    closed(review, ('schema', 'requirements', 'claims', 'unresolved', 'visual'))
    if review['schema'] == REVIEW:
        result = json.loads(json.dumps(review))
    else:
        need(review['schema'] == COMPACT_REVIEW, 'INVALID_SELF_REVIEW')
        template = review_template(inventory, original)
        need(isinstance(review['requirements'], dict)
             and review['requirements'].keys() == template['requirements'].keys(), 'REQUIREMENT_ACCOUNTING_MISMATCH')
        need(isinstance(review['claims'], dict) and review['claims'].keys() == template['claims'].keys(), 'CLAIM_SELF_REVIEW_MISMATCH')
        annotated = {(page['page'], fragment['id']): fragment for page in original['pages']
                     for fragment in page['fragments'] if 'id' in fragment}
        claims = []
        for page, fragments in template['claims'].items():
            assessments = review['claims'][page]
            need(isinstance(assessments, dict) and assessments.keys() == fragments.keys(), 'CLAIM_SELF_REVIEW_MISMATCH')
            for identity in fragments:
                need(assessments[identity] in ('SUPPORTED', 'EXPLICIT_UNKNOWN'), 'CLAIM_ASSESSMENT_REQUIRED')
                claims.append({'page': page, 'claim': identity, 'evidence': annotated[(page, identity)]['evidence'],
                               'disposition': assessments[identity]})
        requirements = []
        for requirement in inventory['requirements']:
            row = review['requirements'][requirement['id']]
            closed(row, ('disposition',), ('section', 'diagram', 'evidence', 'discoveryDisposition', 'discoveredEntries', 'topics', 'sourceNotes'))
            need(row['disposition'] in ('EXPLAINED', 'EXPLICIT_UNKNOWN'), 'REQUIREMENT_ASSESSMENT_REQUIRED')
            page_handles = list(dict.fromkeys(h for (page, _), fragment in annotated.items()
                                             if page == requirement['page'] for h in fragment['evidence']))
            if requirement['kind'] == 'ENTRY':
                candidates = [h for h in page_handles if h in entries and entries[h]['source']['id'] in requirement['sourceIds']]
                if not candidates:
                    candidates = (defaults or {}).get(requirement['id'], [h for h, e in entries.items() if e['source']['id'] in requirement['sourceIds']])
            elif requirement['kind'] == 'DISCOVERY':
                candidates = (defaults or {}).get(requirement['id'], [h for h, e in entries.items()
                    if e['source']['service'] == requirement['service'] and e['source']['file'] == requirement['file']])
            else:
                candidates = page_handles
            requirements.append({'requirement': requirement['id'], 'page': requirement['page'],
                                 'section': 'handler' if requirement['kind'] == 'ENTRY' else None,
                                 **({'diagram': 0} if requirement['kind'] == 'ENTRY' else {}),
                                 'evidence': candidates, **row})
        result = {'schema': REVIEW, 'requirements': requirements, 'claims': claims,
                  'unresolved': review['unresolved'], 'visual': review['visual']}
    for account in result['requirements']:
        account['section'] = local_section(account.get('section'))
        for discovered in account.get('discoveredEntries') or []:
            discovered['section'] = local_section(discovered.get('section'))
    return result


def review_accounts(review, inventory, pages, entries, original):
    closed(review, ('schema', 'requirements', 'claims', 'unresolved', 'visual'))
    need(review['schema'] == REVIEW and isinstance(review['unresolved'], list), 'INVALID_SELF_REVIEW')
    accounts = review['requirements']
    need(isinstance(accounts, list) and len(accounts) == len(inventory['requirements'])
         and {a.get('requirement') for a in accounts} == {r['id'] for r in inventory['requirements']}, 'REQUIREMENT_ACCOUNTING_MISMATCH')
    by_id = {a['requirement']: a for a in accounts}
    W['validate_discovery_review'](inventory, accounts, globals())
    effective, additional, discovery_proofs = list(inventory['requirements']), [], {}
    for requirement in inventory['requirements']:
        account = by_id[requirement['id']]
        need(account.get('page') == requirement['page'] and account['page'] in pages
             and isinstance(account.get('section'), str) and account['section']
             and account.get('disposition') in ('EXPLAINED', 'EXPLICIT_UNKNOWN')
             and isinstance(account.get('evidence'), list) and account['evidence']
             and all(h in entries for h in account['evidence']), 'INVALID_REQUIREMENT_ACCOUNT', requirement=requirement['id'])
        if requirement['kind'] == 'DISCOVERY':
            evidence = [entries[h]['source'] for h in account['evidence']]
            discovery_proofs[requirement['id']] = full_file_proof(requirement, account['evidence'], entries, inventory['snapshot'])
            found = account.get('discoveredEntries')
            local_ids = set()
            for candidate in found:
                local = candidate.get('id')
                need(isinstance(local, str) and local and all(c.isascii() and (c.isalnum() or c in '-_') for c in local)
                     and local not in local_ids, 'INVALID_DISCOVERED_ENTRY_ID')
                local_ids.add(local)
                need(candidate.get('kind') in ('HTTP', 'KAFKA', 'SCHEDULED', 'OTHER'), 'INVALID_DISCOVERED_ENTRY_KIND')
                member = W['discovered_entry'](requirement, candidate, globals())
                identity = member['id']
                member['sourceIds'] = [source['id'] for source in evidence]
                if candidate['kind'] == 'HTTP':
                    need(isinstance(member['method'], str) and member['method'] and isinstance(member['path'], str)
                         and member['path'], 'DISCOVERED_HTTP_IDENTITY_REQUIRED')
                need(member['page'] in pages and isinstance(candidate.get('section'), str) and candidate['section']
                     and type(candidate.get('diagram')) is int and candidate['diagram'] >= 0
                     and candidate.get('disposition') in ('EXPLAINED', 'EXPLICIT_UNKNOWN')
                     and candidate.get('evidence') and all(h in entries for h in candidate['evidence'])
                     and any(entries[h]['source']['file'] == requirement['file'] for h in candidate['evidence']),
                     'INVALID_DISCOVERED_ENTRY_ACCOUNT')
                additional.append({**candidate, 'requirement': identity})
                effective.append(member)
        if requirement['kind'] == 'ENTRY':
            need(type(account.get('diagram')) is int and account['diagram'] >= 0, 'REQUIRED_SEQUENCE_MISSING', requirement=requirement['id'])
            need(set(requirement['sourceIds']) & {entries[h]['source']['id'] for h in account['evidence']},
                 'ENTRY_SOURCE_BINDING_MISMATCH', requirement=requirement['id'])
    canonical, _ = W['entry_registry'](inventory, accounts, globals())
    need({r['id'] for r in canonical} == {r['id'] for r in effective if r['kind'] == 'ENTRY'}, 'ENTRY_REGISTRY_MISMATCH')
    W['validate_discovered_sources'](inventory, additional, entries, globals())
    by_identity = {r['id']: r for r in canonical}
    effective = [{**r, 'target': by_identity[r['id']]['target']} if r['kind'] == 'ENTRY' else r for r in effective]
    claims = {(p['page'], f['id']): f for p in original['pages'] for f in p['fragments'] if 'id' in f}
    assessed = review['claims']
    need(isinstance(assessed, list) and len(assessed) == len(claims)
         and {(a.get('page'), a.get('claim')) for a in assessed} == claims.keys(), 'CLAIM_SELF_REVIEW_MISMATCH')
    for account in assessed:
        claim = claims[(account['page'], account['claim'])]
        need(account.get('disposition') in ('SUPPORTED', 'EXPLICIT_UNKNOWN')
             and account.get('evidence') == claim.get('evidence')
             and all(h in entries for h in account['evidence']), 'INVALID_CLAIM_SELF_REVIEW')
    need(isinstance(review['visual'], dict) and review['visual'].get('status') in ('REVIEWED', 'UNAVAILABLE'), 'VISUAL_STAGE_MUST_BE_RECORDED')
    return accounts + additional, effective, discovery_proofs


def verify(run, *, debug=False):
    run = Path(run).absolute(); checks = {name: {'status': 'NOT_RUN'} for name in CHECKS}
    directory = run / 'checks' / uuid.uuid4().hex; directory.mkdir(mode=0o700, parents=True)
    stage = 'inputs'; result = {'schema': 'codeclew-docs-run-report/1.0', 'status': 'FAILED', 'checks': checks,
        'semanticTruthAssessed': False, 'runtimeVerified': False, 'publicationPerformed': False,
        'semanticReviewRecorded': False, 'independentVisualAcceptanceRequired': True}
    try:
        job, inventory, pins = inputs_unchanged(run)
        result['inputDigest'] = digest(pins)
        accounting = load(run / 'state.json')
        checks[stage] = {'status': 'PASS'}
        stage = 'author'; draft = run / 'workspace/draft'
        tokens = token_accounting(accounting.get('attempts', []))
        need(('successor' in accounting) == (run / 'inputs/successor.json').exists(), 'SUCCESSOR_STATE_MISMATCH')
        base = S['validate'](run, accounting, globals()) if (run / 'inputs/successor.json').exists() else 0
        need(tokens['status'] != 'INVALID', 'AUTHOR_USAGE_INVALID', accounting=tokens)
        limit = job['budget']['maxModelTokens']
        need(limit is None or scoped_budget_tokens(accounting, tokens) <= limit, 'AUTHOR_TOKEN_BUDGET_EXCEEDED')
        need(tokens['status'] == 'MEASURED', 'AUTHOR_USAGE_UNAVAILABLE', blocked=True, accounting=tokens)
        last = accounting.get('attempts', [])[-1:]
        need(accounting.get('status') == 'AUTHOR_COMPLETED' and len(accounting['attempts']) > base
             and (not base or accounting['attempts'][base].get('kind') == 'successor-review')
             and last and last[0].get('status') == 'COMPLETED'
             and last[0].get('exitCode') == 0 and last[0].get('draftFiles') == tree(draft), 'AUTHOR_COMPLETION_REQUIRED')
        checks[stage] = {'status': 'PASS', 'usageAvailable': tokens['status'] == 'MEASURED',
                         'reportedModelTokens': tokens['budgetTokens'], 'tokenAccounting': tokens}
        if base:checks[stage].update(budgetScope='NEW_ATTEMPTS_ONLY', inheritedModelTokens=accounting['successor']['inheritedModelTokens'],
                                    continuationModelTokens=scoped_budget_tokens(accounting, tokens), totalModelTokens=tokens['budgetTokens'])
        stage = 'output'; before = tree(draft)
        need(before and 'original.json' in before, 'MISSING_REQUIRED_OUTPUT')
        original = load(draft / 'original.json')
        need(not W['unfinished'](original), 'UNFINISHED_SCAFFOLD', pages=W['unfinished'](original))
        W['validate_start_labels'](run, globals())
        W['validate_fragment_addresses'](original, globals(), job=job)
        if 'components' in job:
            M['configure_engine'](job['clew'], job['root'], job['components']['path'], job['components']['digest'])
        pages, _, _ = M['compose_pages'](original)
        need(all(r['page'] in pages and pages[r['page']].strip() for r in inventory['requirements']), 'MISSING_REQUIRED_PAGE')
        W['route_preflight'](run, job, globals())
        checks[stage] = {'status': 'PASS', 'pages': len(pages)}
        stage = 'provenance'; manifests, entries = cite_entries(job, inventory, draft, directory / 'sources', run / 'inputs/author-source')
        checks[stage] = {'status': 'PASS', 'handles': len(entries)}
        stage = 'selfReview'; review = load(draft / 'self-review.json')
        W['local_registry'](run, inventory, globals())
        defaults = {}
        for path in (run / 'inputs/author-source').glob('*/requirements.json'):
            defaults.update({key: value['evidenceCandidates'] for key, value in load(path).items()})
        review = resolve_review(review, inventory, entries, original, defaults)
        registered = W['registered_accounts'](run, globals())
        for account in registered:
            actual = next(r for r in review['requirements'] if r['requirement'] == account['requirement'])
            need(W['declaration_identity'](actual.get('discoveredEntries') or []) == W['declaration_identity'](account['discoveredEntries']),
                 'ENTRY_REGISTRATION_REVIEW_MISMATCH', requirement=account['requirement'])
        reviewed = draft / 'reviewed-pages.json'
        if reviewed.exists():
            hashes = load(reviewed)
            need(all(hashes.get(p['page']) == digest(p) for p in original['pages']), 'PAGE_REVIEW_STALE',
                 pages=[p['page'] for p in original['pages'] if hashes.get(p['page']) != digest(p)],
                 recovery='Review only the listed pages using ./docs review-page; unchanged page reviews remain valid.')
        save(directory / 'resolved-self-review.json', review)
        accounts, effective, discovery_proofs = review_accounts(review, inventory, pages, entries, original)
        P['validate'](inventory, review['requirements'], original, load(draft / 'navigation.json'), entries, globals())
        checks[stage] = {'status': 'PASS', 'discoveryCoverage': discovery_proofs}; result['semanticReviewRecorded'] = True
        result['authorVisualReview'] = review['visual']
        stage = 'entrySequences'; request = directory / 'sequences.json'
        save(request, {'pages': {k: v.decode() for k, v in pages.items()}, 'requirements': effective, 'accounts': accounts,
                       'processPages': W['process_pages'](run, globals())})
        sequence = command([job['node'], str(HERE / 'docs_run_sequences.mjs'), job['renderer'], str(request)], directory, 'sequences')
        need(sequence.get('status') == 'ENTRY_SEQUENCES_CHECKED', 'ENTRY_SEQUENCES_NOT_CHECKED')
        checks[stage] = {'status': 'PASS', 'result': sequence}
        stage = 'finalize'
        navigation = load(draft / 'navigation.json')
        service_navigation = {s['id']: s for s in navigation['services']}
        need(set(service_navigation) == set(job['services']) and len(service_navigation) == len(navigation['services']), 'NAVIGATION_SERVICE_SCOPE_MISMATCH')
        for requirement in effective:
            if requirement['kind'] == 'ENTRY':
                kind = {'HTTP_ENDPOINT': 'http', 'KAFKA_LISTENER': 'kafka', 'SCHEDULED_JOB': 'scheduled'}.get(requirement['entryKind'], 'other')
                need(any(start['target'] == requirement['page'] + '#handler' and start['kind'] == kind
                         for start in service_navigation[requirement['service']]['starts']),
                     'REQUIRED_START_NAVIGATION_MISSING', requirement=requirement['id'])
        compose = directory / 'compose'
        M['compose'](draft / 'original.json', compose)
        renderer = directory / 'renderer.json'
        argv = [job['node'], str(Path(job['renderer']) / 'render.mjs'), '{input}', '{output}', 'Service documentation',
                '--theme', 'codeclew', '--lang', job['language'], '--navigation', str(draft / 'navigation.json')]
        routes_present = (draft / 'routes.json').exists()
        if routes_present or len(job['services']) > 1:
            registry = load(draft / 'routes.json')
            local = registry.get('catalog', {}).get('docset')
            docsets = [d for d in registry['docsets'] if d['id'] == local]
            need(len(docsets) == 1 and {s['id'] for s in docsets[0]['services']} == set(job['services']), 'LOCAL_PROCESS_CATALOG_REQUIRED')
            argv += ['--docset-routes', str(draft / 'routes.json'), '--docset', local, '--docset-version', docsets[0]['version']]
        save(renderer, argv)
        args = [sys.executable, '-I', '-S', str(HERE / 'reader_export_check.py'), 'finalize', '--compose-root', str(compose),
                '--clew', job['clew'], '--root', job['root'], '--renderer-command', str(renderer), '--source-footers',
                '--require-sequences', '--output-dir', str(directory / 'final')]
        if len(job['services']) > 1:
            args += ['--require-process-search']
        for manifest in manifests:
            args += ['--manifest', str(manifest)]
        if 'components' in job:
            args += ['--components', job['components']['path'], '--component-set-digest', job['components']['digest']]
        finalized = command(args, directory, 'finalize', timeout=300)
        need(finalized.get('status') == 'CHECKED_PAGES_FINALIZED', 'FINALIZATION_FAILED')
        checked_html = {p.name: p.read_bytes() for p in (directory / 'final/html').glob('*.html')}
        documents = {}
        for name, raw in checked_html.items():
            parsed = E['Links'](); parsed.feed(raw.decode()); parsed.close(); documents[name] = parsed
        for account in accounts:
            need(account['section'] in documents[str(Path(account['page']).with_suffix('.html'))].targets,
                 'SELF_REVIEW_SECTION_MISSING', requirement=account['requirement'])
        checks[stage] = {'status': 'PASS', 'result': finalized['exportStatus']}
        stage = 'outputPins'; inputs_unchanged(run)
        need(tree(draft) == before, 'DRAFT_CHANGED_DURING_CHECK')
        checks[stage] = {'status': 'PASS'}
        result.update(status='READY', draftDigest=digest(before), draftFiles=before,
                      output=str((directory / 'final').relative_to(run)), outputFiles=tree(directory / 'final'))
        result['outputDigest'] = digest(result['outputFiles'])
        result['deliverableFiles'] = tree(directory / 'final/html')
    except (Stop, E['Stop'], OSError, ValueError, TypeError, KeyError, IndexError) as error:
        code = getattr(error, 'code', getattr(error, 'status', 'INVALID_RUN_INPUT'))
        checks[stage] = {'status': 'BLOCKED' if getattr(error, 'blocked', False) else 'FAIL',
                         'code': code, 'details': getattr(error, 'details', str(error))}
        result['status'] = 'BLOCKED' if getattr(error, 'blocked', False) else 'FAILED'
    if debug:
        save(directory / 'debug.json', {'observableInputsOnly': True, 'checks': checks, 'semanticTruthAssessed': False})
    save(directory / 'report.json', result); save(run / 'report.json', result)
    return result


def promote(run, destination=None):
    run = Path(run).absolute(); accepted = load(run / 'report.json')
    need(accepted.get('status') == 'READY' and all(accepted['checks'][c]['status'] == 'PASS' for c in CHECKS), 'RUN_NOT_READY')
    need(tree(run / 'workspace/draft') == accepted['draftFiles']
         and tree(run / accepted['output']) == accepted['outputFiles'], 'ACCEPTED_OUTPUT_CHANGED')
    # Re-run the fixed gates; an author-created report is not an admission token.
    current = verify(run)
    need(current['status'] == 'READY' and current['draftDigest'] == accepted['draftDigest'], 'PROMOTION_RECHECK_FAILED')
    need(current['deliverableFiles'] == accepted['deliverableFiles'], 'PROMOTION_OUTPUT_CHANGED')
    ready = run / 'ready'; ready.mkdir(mode=0o700, exist_ok=True)
    key = accepted['outputDigest'].split(':')[1]
    output = ready / key
    if not output.exists():
        stage = ready / ('.staging-' + uuid.uuid4().hex)
        shutil.copytree(run / accepted['output'], stage)
        need(tree(stage) == accepted['outputFiles'], 'PROMOTION_COPY_CHANGED')
        os.replace(stage, output)
    save(ready / 'current.json', {'output': key, 'outputDigest': accepted['outputDigest'],
                                  'reportDigest': digest(current), 'publicationPerformed': False})
    if destination is not None:
        destination = Path(destination).absolute(); need(not destination.exists(), 'OUTPUT_ALREADY_EXISTS')
        stage = destination.with_name('.' + destination.name + '.' + uuid.uuid4().hex)
        shutil.copytree(output, stage)
        need(tree(stage) == accepted['outputFiles'], 'PROMOTION_COPY_CHANGED')
        os.replace(stage, destination)
    return {'status': 'READY_PROMOTED', 'outputDigest': accepted['outputDigest'], 'ready': str(output), 'publicationPerformed': False}


def repair_facts(report):
    """Keep complete receipts on disk; send only actionable failed checks."""
    failed = {name: row for name, row in report.get('checks', {}).items()
              if isinstance(row, dict) and row.get('status') in ('FAIL', 'FAILED', 'BLOCKED')}
    return {'checks': failed, 'status': report.get('status'),
            **({'code': report['code']} if report.get('code') else {}),
            'fullReport': 'report.json', 'usageAndAttemptHistory': 'state.json'}


def author(run, *, unit=None, repair=False):
    run = Path(run).absolute(); job, inventory, pins = inputs_unchanged(run)
    state = load(run / 'state.json'); attempts = state['attempts']
    need(('successor' in state) == (run / 'inputs/successor.json').exists(), 'SUCCESSOR_STATE_MISMATCH')
    base = S['validate'](run, state, globals()) if (run / 'inputs/successor.json').exists() else 0
    tokens = token_accounting(attempts)
    need(tokens['status'] != 'INVALID', 'AUTHOR_USAGE_INVALID', accounting=tokens)
    limit = job['budget']['maxModelTokens']
    need(limit is None or scoped_budget_tokens(state, tokens) < limit, 'AUTHOR_TOKEN_BUDGET_EXHAUSTED')
    need(tokens['status'] == 'MEASURED', 'AUTHOR_USAGE_UNAVAILABLE', blocked=True, accounting=tokens)
    if state['authorStartedAt'] is None:
        state['authorStartedAt'] = time.time()
    remaining = job['budget']['wallSeconds'] - (time.time() - state['authorStartedAt'])
    first_successor = bool(base and len(attempts) == base)
    kind = 'successor-review' if first_successor else 'repair' if repair or attempts and unit is None else 'unit' if unit else 'full'
    repair_count = sum(a.get('kind', 'full' if i == 0 else 'repair') == 'repair' for i, a in enumerate(attempts[base:]))
    need(remaining > 0 and (kind != 'repair' or repair_count < job['budget']['maxRepairs']), 'AUTHOR_BUDGET_EXHAUSTED')
    folder = run / 'attempts' / str(len(attempts)); folder.mkdir(mode=0o700, parents=True)
    if first_successor:
        prompt = encode({'instruction': 'Perform one standalone full-docset review of the full original question under the updated generic prepared-author contract. This is not a service-unit replay or an enum-only repair. First save your own compact source-block consequence notes per service through review-page --text-file, before using existing prose/arrows as review cues. Compare that source-derived outline to existing text and diagrams, then every visible claim back to source; reconcile operation/path/completion facts, skipped/failure/default/malformed paths, readable overview/linked phases and all independent known starts. Close each SERVICE topic with a real reader fragment or evidence-bounded absence using review-page. Preserve unchanged pages and valid assessments where justified by your review. No evaluator answers or second author are supplied. Run the supported checks, record explicit review judgments, finish-review, then end this turn; the caller runs the mandatory full final gate.',
                         'request': job['request'], 'contract': str(SKILL / 'references/docs-run-author.md'),
                         'workspace': str(run / 'workspace'), 'inventory': str(run / 'workspace/inventory.json'),
                         'sourceIndex': str(run / 'inputs/author-source/index.md'),
                         'predecessorMachineFailure': repair_facts(load(run / 'history/report.json')),
                         'callerRevisionRequest': load(run / 'inputs/successor.json').get('callerDecision'),
                         'inheritedModelTokens': state['successor']['inheritedModelTokens'],
                         'continuationAllowance': job['budget'], 'budgetScope': 'NEW_ATTEMPTS_ONLY',
                         'historyReceipt': str(run / 'inputs/successor.json'), 'sameAuthorThread': state['threadId']})
    elif not attempts:
        prompt = (run / 'prompt.txt').read_bytes()
    elif unit and not repair:
        prompt = encode({'instruction': 'Continue the same frozen request and source scope. Complete only this related-page unit, save local page files, run the supported early check and repair them, then end this turn. Do not rebuild previous units. Record local semantic assessments with review-page before leaving each unit. The complete-docset unit finishes shared full-process diagrams and necessary cross-service semantic reconciliation, not a rewrite or rereview of unchanged accepted unit content. No scope may be dropped.',
                         'unit': unit, 'budgetPlan': state.get('budgetPlan'), 'remainingSeconds': int(remaining)})
    else:
        failures = load(run / 'report.json') if (run / 'report.json').exists() else {'status': 'INTERRUPTED'}
        prompt = encode({'instruction': 'Repair only the affected fragments named below and repeat semantic self-review for changed content and its consequential dependencies. Preserve unchanged pages, valid local review and source reads; do not replay the whole docset. Acquire only needed missing sources. Only machine failures follow, not a semantic answer key.',
                         'failures': repair_facts(failures),
                         'remainingSeconds': int(remaining), 'remainingRepairs': job['budget']['maxRepairs'] - repair_count})
    if unit:
        prompt += encode({'assignedUnit': unit, 'budgetPlan': state.get('budgetPlan'),
                          'stopAfterUnit': True, 'fullRequestStillRequired': True,
                          'preparedAuthorContract': str(SKILL / 'references/docs-run-author.md'),
                          'authorCommands': './docs show --page PAGE.md [--fragment ID]; ./docs put --page PAGE.md --fragment CLAIM --text-file FILE --evidence HANDLE; ./docs sequence --page PAGE.md --fragment FLOW --input FILE --evidence HANDLE; ./docs check --page PAGE.md; ./docs review-page --page PAGE.md',
                          'workflow': 'Continue the admitted prepared-author workflow. Do not rediscover the globally installed skill or restart generic source acquisition/export. Full semantic requirements and source reads remain required.'})
    if state.get('accountingRecovery'):
        prompt += encode({'instruction': 'This same logical run recovered a cumulative-counter accounting error. Use the successor helper below for save-pages, check and review-template; keep your existing source reads, pages and context. The request, source scope and original time/token/repair caps are unchanged.',
                          'helper': str(HERE / 'docs_run.py'), 'runDirectory': str(run)})
    (folder / 'prompt.txt').write_bytes(prompt)
    backend = job['author']
    argv = [backend['executable'], 'exec', '--ignore-user-config', '--skip-git-repo-check', '--json',
            '-m', backend['model'], '-c', 'model_reasoning_effort=' + json.dumps(backend['reasoningEffort']),
            '-c', 'approval_policy="never"', '-s', 'workspace-write', '-C', str(run / 'workspace'),
            '--add-dir', job['root'], '--disable', 'multi_agent', '-o', str(folder / 'last-message.txt')]
    if 'authorSkillsConfig' in pins:
        argv += ['-c', pins['authorSkillsConfig']]
    if backend.get('stateRoot'):
        argv += ['--add-dir', backend['stateRoot']]
    if attempts:
        need(state.get('threadId'), 'INTERRUPTED_AUTHOR_SESSION_UNAVAILABLE')
        argv += ['resume', state['threadId']]
    argv += ['-']
    attempt = {'argv': argv, 'kind': kind, 'unit': unit['id'] if unit else None,
               'status': 'RUNNING', 'startedAt': time.time(), 'exitCode': None}
    attempts.append(attempt); state['status'] = 'AUTHOR_RUNNING'; save(run / 'state.json', state)
    usage = None; interrupted = False; thread_changed = False
    expected_thread = state.get('threadId')
    with (folder / 'stdout.jsonl').open('wb') as stdout, (folder / 'stderr.log').open('wb') as stderr:
        process = subprocess.Popen(argv, cwd=run / 'workspace', stdin=subprocess.PIPE,
                                   stdout=stdout, stderr=stderr, start_new_session=True)
        try:
            process.communicate(input=prompt, timeout=remaining)
        except (KeyboardInterrupt, subprocess.TimeoutExpired):
            interrupted = True
            os.killpg(process.pid, signal.SIGTERM)
            try: process.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL); process.communicate()
    need((folder / 'stdout.jsonl').stat().st_size <= 128 * 1024 * 1024, 'AUTHOR_LOG_LIMIT')
    with (folder / 'stdout.jsonl').open() as stream:
        for line in stream:
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if event.get('type') == 'thread.started':
                if expected_thread and event['thread_id'] != expected_thread:thread_changed = True
                state['threadId'] = event['thread_id']
                attempt['threadId'] = event['thread_id']
            if event.get('type') == 'turn.completed':
                usage = event.get('usage')
    attempt['draftFiles'] = tree(run / 'workspace/draft')
    shutil.copytree(run / 'workspace/draft', folder / 'draft')
    attempt.update(status='INTERRUPTED' if interrupted else 'COMPLETED', exitCode=process.returncode, usage=usage)
    if unit and not interrupted and process.returncode == 0:state['completedUnit'] = unit['id']
    tokens = token_accounting(attempts)
    state['tokenAccounting'] = tokens
    state['usageAvailable'] = tokens['status'] == 'MEASURED'
    state['modelTokens'] = tokens['budgetTokens']
    state['status'] = ('INTERRUPTED' if interrupted else 'AUTHOR_FAILED' if process.returncode != 0 or thread_changed else
                       'AUTHOR_USAGE_INVALID' if tokens['status'] == 'INVALID' else
                       'AUTHOR_BUDGET_EXCEEDED' if limit is not None and scoped_budget_tokens(state, tokens) > limit else
                       'AUTHOR_USAGE_UNAVAILABLE' if tokens['status'] != 'MEASURED' else 'AUTHOR_COMPLETED')
    save(run / 'state.json', state); save(folder / 'result.json', attempt)
    need(not thread_changed, 'AUTHOR_THREAD_CHANGED')
    need(not interrupted and process.returncode == 0, 'AUTHOR_NOT_COMPLETED')
    need(tokens['status'] != 'INVALID', 'AUTHOR_USAGE_INVALID', accounting=tokens)
    need(limit is None or scoped_budget_tokens(state, tokens) <= limit, 'AUTHOR_TOKEN_BUDGET_EXCEEDED')
    need(tokens['status'] == 'MEASURED', 'AUTHOR_USAGE_UNAVAILABLE', blocked=True, accounting=tokens)
    # This is reported turn accounting, available after a turn, not a provider
    # billing guarantee or a continuously enforceable in-turn token ceiling.
    return state


def execute(run, *, debug=False):
    run = Path(run).absolute(); state = load(run / 'state.json')
    need(state.get('status') != 'SUCCESSOR_ADMISSION_PENDING', 'SUCCESSOR_NOT_ADMITTED')
    if (run / 'inputs/successor.json').exists():return S['execute'](run, globals(), debug)
    if (run / 'inputs/scaffold.json').exists():return W['run_units'](run, globals(), debug)
    while True:
        if state['status'] != 'AUTHOR_COMPLETED':
            state = author(run)
        result = verify(run, debug=debug)
        if result['status'] == 'READY':
            return promote(run)
        job = load(run / 'inputs/job.json')
        if result['status'] == 'BLOCKED' or result['checks']['inputs']['status'] != 'PASS' or len(state['attempts']) > job['budget']['maxRepairs']:
            return result
        state['status'] = 'REPAIR_REQUIRED'; save(run / 'state.json', state)


def main():
    parser = argparse.ArgumentParser(description=__doc__, epilog=(
        'Normal author commands: show, put, sequence, register-entry, page-info, route, review-page, finish-review, check. '
        'review-page records explicit per-page judgments; finish-review validates the assembled review. '
        'Requirement disposition: EXPLAINED or EXPLICIT_UNKNOWN. Discovery discoveryDisposition: '
        'ADDITIONAL_STARTS for a nonempty declared finding list, or NO_ADDITIONAL_STARTS for an explicitly reviewed empty list. '
        'These are separate fields; an empty registry does not prove no additions. '
        'Use route --input FILE for route metadata repair; keep prose and other routes unchanged.'))
    parser.add_argument('command', choices=('prepare', 'prepare-successor', 'run', 'verify', 'promote', 'review-template', 'show', 'save-pages', 'check', 'recover-accounting', 'register-entry', 'put', 'sequence', 'page-info', 'route', 'review-page', 'finish-review'))
    parser.add_argument('--job', type=Path)
    parser.add_argument('--run-dir', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path)
    parser.add_argument('--debug', action='store_true')
    parser.add_argument('--input', type=Path)
    parser.add_argument('--page', action='append', default=[])
    parser.add_argument('--scaffold', action='store_true')
    parser.add_argument('--previous-skill', type=Path)
    parser.add_argument('--previous-run', type=Path, help='Terminal predecessor for caller-owned prepare-successor; --job budget is its explicit new-attempt allowance.')
    parser.add_argument('--revise-ready', action='store_true', help='Caller requests an isolated revision of native READY after independent quality rejection; preserve the parent status and deliverable.')
    parser.add_argument('--full', action='store_true', help='Print complete local save/check receipt instead of its compact summary.')
    parser.add_argument('--text-file', type=Path)
    parser.add_argument('--fragment')
    parser.add_argument('--evidence', action='append', default=[])
    parser.add_argument('--requirement')
    parser.add_argument('--kind', choices=('HTTP', 'KAFKA', 'SCHEDULED', 'OTHER'))
    parser.add_argument('--title')
    parser.add_argument('--summary', help='page-info: replace the existing start reader summary/title without changing its identity.')
    parser.add_argument('--method')
    parser.add_argument('--path')
    parser.add_argument('--service')
    parser.add_argument('--data', action='store_true', help='Register a Data page; add --fragment ID to target an existing source-backed section.')
    parser.add_argument('--entries', action='store_true', help='Show the compact entry registry; select one with --entry ID.')
    parser.add_argument('--entry')
    parser.add_argument('--routes', action='store_true', help='Show the compact route registry; select one with --route ID.')
    parser.add_argument('--route')
    parser.add_argument('--offset', type=int, default=0, help='show: character offset (default 0).')
    parser.add_argument('--limit', type=int, default=8000, help='show: maximum characters; 0 returns the full remainder (default 8000).')
    args = parser.parse_args()
    edits = ('save-pages', 'check', 'register-entry', 'put', 'sequence', 'page-info', 'route', 'review-page', 'finish-review')
    lock = None
    try:
        if args.command in edits:
            need(args.run_dir.is_dir() and not args.run_dir.is_symlink(), 'RUN_NOT_FOUND')
            lock = os.open(args.run_dir / 'workspace/.edit.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
            fcntl.flock(lock, fcntl.LOCK_EX)
        elif args.command not in ('prepare', 'prepare-successor', 'review-template', 'show'):
            need(args.run_dir.is_dir() and not args.run_dir.is_symlink(), 'RUN_NOT_FOUND')
            lock = os.open(args.run_dir / '.runner.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise Stop('RUN_BUSY', blocked=True)
        if args.command == 'show':
            result = H['show'](args.run_dir.absolute(), args, globals())
            print(result['text'], end='')
            print('\n[Characters ' + str(result['offset']) + ':' + str(result['end']) +
                  ' of ' + str(result['total']) + '; remaining ' + str(result['remaining']) + ']')
            if result['next']:print('Continue: ' + result['next'])
            return 0
        elif args.command == 'prepare-successor':
            need(args.job is not None and args.previous_run is not None and args.previous_skill is not None,
                 'SUCCESSOR_JOB_AND_PREDECESSOR_REQUIRED')
            result = S['prepare'](args.job, args.previous_run, args.previous_skill, args.run_dir, globals(), revise_ready=args.revise_ready)
        elif args.command == 'recover-accounting':
            need(args.previous_skill is not None, 'PREVIOUS_ADMITTED_SKILL_REQUIRED')
            result = recover_accounting(args.run_dir, args.previous_skill)
        elif args.command in ('put', 'sequence', 'register-entry', 'page-info', 'review-page'):
            need(len(args.page) == 1, 'ONE_PAGE_REQUIRED')
            if args.command == 'register-entry':
                prior = next((c for a in W['registered_accounts'](args.run_dir.absolute(), globals()) if a['requirement'] == args.requirement
                              for c in a['discoveredEntries'] if c['page'] == args.page[0]), None)
                if prior:
                    declaration = {**prior, 'evidence': args.evidence}
                    for key in ('kind', 'title', 'method', 'path'):
                        if getattr(args, key) is not None:declaration[key] = getattr(args, key)
                else:
                    need(args.kind and args.title, 'ENTRY_KIND_AND_TITLE_REQUIRED')
                    declaration = {'id': args.page[0].removesuffix('.md').replace('.', '-'),
                        'kind': args.kind, 'page': args.page[0], 'title': args.title,
                        'evidence': args.evidence, 'disposition': None}
                    if args.kind == 'HTTP':declaration.update(method=args.method, path=args.path)
                result = W['register_entry'](args.run_dir.absolute(), args.requirement, declaration, globals())
            elif args.command == 'page-info':
                if args.fragment and not args.data:
                    need(not (args.title or args.service or args.summary), 'SEPARATE_FRAGMENT_ADDRESS_AND_PAGE_METADATA')
                    result = W['address_fragment'](args.run_dir.absolute(), args.page[0], args.fragment, globals())
                else:
                    need(not args.fragment or (args.title and args.service and args.summary is None), 'DATA_FRAGMENT_METADATA_REQUIRED')
                    need(args.title, 'PAGE_TITLE_REQUIRED')
                    result = W['page_info'](args.run_dir.absolute(), args.page[0], args.service, args.title, args.data, globals(), summary=args.summary, fragment=args.fragment)
            elif args.command == 'review-page':
                need(not (args.input and args.text_file), 'SEPARATE_SOURCE_NOTES_AND_ASSESSMENT')
                if args.text_file:
                    result = P['save_notes'](args.run_dir.absolute(), args.page[0], args.text_file.read_text(), args.evidence, globals())
                else:result = W['review_page'](args.run_dir.absolute(), args.page[0], load(args.input) if args.input else None, globals())
                if args.input is None and args.text_file is None:
                    print(encode(result).decode(), end='');return 0
            else:
                path = args.input if args.command == 'sequence' else args.text_file
                need(path is not None and args.fragment, 'FRAGMENT_AND_CONTENT_REQUIRED')
                result = W['put'](args.run_dir.absolute(), args.page[0], args.fragment, path.read_text(),
                                   args.evidence, args.command == 'sequence', globals())
        elif args.command == 'finish-review':
            need(args.input is not None, 'GLOBAL_REVIEW_REQUIRED')
            result = W['finish_review'](args.run_dir.absolute(), load(args.input), globals())
        elif args.command == 'route':
            need(args.input is not None, 'ROUTE_INPUT_REQUIRED')
            result = W['route'](args.run_dir.absolute(), load(args.input), globals())
        elif args.command == 'save-pages':
            need(args.input is not None, 'PAGE_INPUT_REQUIRED')
            result = W['save_pages'](args.run_dir.absolute(), args.input, globals())
        elif args.command == 'check':
            result = W['check'](args.run_dir.absolute(), args.page, args.scaffold, globals())
        elif args.command == 'review-template':
            result = review_for_run(args.run_dir.absolute())
            print(encode(result).decode(), end='')
            return 0
        elif args.command == 'prepare':
            need(args.job is not None, 'JOB_REQUIRED'); result = prepare(args.job, args.run_dir)
        elif args.command == 'run': result = execute(args.run_dir, debug=args.debug)
        elif args.command == 'verify': result = verify(args.run_dir, debug=args.debug)
        else: result = promote(args.run_dir, args.output_dir)
    except (Stop, E['Stop'], M['Stop'], OSError, ValueError, TypeError, KeyError) as error:
        result = {'status': 'BLOCKED' if getattr(error, 'blocked', False) else 'FAILED',
                  'code': getattr(error, 'code', getattr(error, 'status', 'INVALID_RUN_INPUT')), 'details': getattr(error, 'details', str(error)),
                  'publicationPerformed': False}
    finally:
        if lock is not None:os.close(lock)
    if args.command in ('save-pages', 'check', 'put', 'sequence') and not args.full:
        result = W['summary'](result)
    print(encode(result).decode(), end='')
    return 0 if result['status'] in ('PREPARED', 'SUCCESSOR_PREPARED', 'READY', 'READY_PROMOTED', 'PAGES_SAVED', 'UNIT_CHECKED', 'SCAFFOLD_CHECKED', 'ACCOUNTING_RECOVERED', 'ENTRY_REGISTERED', 'PAGE_REGISTERED', 'ROUTE_SAVED', 'PAGE_REVIEW_RECORDED', 'GLOBAL_REVIEW_RECORDED', 'SOURCE_NOTES_SAVED') else 2


if __name__ == '__main__':
    raise SystemExit(main())
