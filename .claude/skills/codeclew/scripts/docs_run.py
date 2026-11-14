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


def token_accounting(attempts):
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


def budget_tokens(accounting):
    return accounting['budgetTokens'] if accounting['budgetTokens'] is not None else accounting['knownBudgetTokensLowerBound']


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
    for key, lower, upper in (('wallSeconds', 1, 3600), ('maxModelTokens', 1, 250000), ('maxRepairs', 0, 2)):
        need(type(job['budget'][key]) is int and lower <= job['budget'][key] <= upper, 'INVALID_BUDGET', field=key)
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
        record = row['record']; entry = record['entrypoint']; trigger = entry.get('trigger', {})
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
    bounds = {'wallSeconds': 300, 'files': 128, 'visibleBytes': 8 * 1024 * 1024}
    services, all_bindings, count, visible_bytes = {}, {}, 0, 0
    M['configure_engine'](job['clew'], job['root'])

    def invoke(arguments, output, label):
        remaining = bounds['wallSeconds'] - (time.monotonic() - started)
        need(remaining > 0, 'SOURCE_PREPARATION_WALL_LIMIT')
        result = command(arguments, output, label, timeout=remaining)
        tree(directory)  # Existing closure byte/file limits apply after each bounded helper.
        return result

    for service, scope in inventory['services'].items():
        home = directory / service; home.mkdir(mode=0o700)
        resource_count = None
        base = [sys.executable, '-I', '-S', str(HERE / 'evidence_packet.py')]
        common = ['--clew', job['clew'], '--root', job['root'], '--service', service, '--snapshot', job['snapshot']]
        source_scope = any(r['kind'] == 'THREAD_SCOPE' for r in scope['sourceInventory'])
        action = 'discover-files' if source_scope else 'discover'
        invoke(base + [action, *common, '--work', scope['work'], '--output-dir', str(home / 'discovery')], home, 'discover')
        discovery = load(home / 'discovery/packet.json')
        need(discovery.get('queryComplete'), 'PREPARED_DISCOVERY_INCOMPLETE', service=service)
        choices = discovery['choices']
        if source_scope:
            selection = [{'file': row['file']} for row in choices]
            count += len(selection)
            need(count <= bounds['files'], 'SOURCE_PREPARATION_FILE_LIMIT')
            save(home / 'selection.json', selection)
            arguments = ['read-files', *common, '--packet', str(home / 'discovery/packet.json')]
        else:
            pairs = compiler_source_pairs(choices)
            save(home / 'resources-selection.json', {'query': {'kind': 'RETAINED_RESOURCE', 'projection': 'NAVIGATION'}})
            resources = invoke([sys.executable, '-I', '-S', str(HERE / 'work_session.py'), 'read',
                                '--launcher', job['clew'], '--root', job['root'], '--work', scope['work'],
                                '--input-root', str(home), '--input', 'resources-selection.json',
                                '--output', 'resources', '--max-pages', '128'], home, 'resources')
            need(resources['status'] == 'COMPLETE', 'PREPARED_RESOURCES_INCOMPLETE')
            resource_rows = load(home / 'resources/read-collection/items.json')
            resource_count = len(resource_rows)
            for row in resource_rows:
                need(row['kind'] == 'SOURCE_NAVIGATION', 'PREPARED_RESOURCE_KIND_CHANGED')
                identity, reference = row['id'], row['record']['fullRecordReference']
                need(identity not in pairs or pairs[identity] == reference, 'PREPARED_SOURCE_REFERENCE_CHANGED')
                pairs[identity] = reference
            selection = [{'sourceId': identity, 'sourceReference': reference} for identity, reference in sorted(pairs.items())]
            save(home / 'selection.json', selection)
            arguments = ['read-sources', *common, '--work', scope['work']]
        need(selection, 'PREPARED_SOURCE_SELECTION_EMPTY', service=service)
        invoke(base + [*arguments, '--selection', str(home / 'selection.json'), '--source-union',
                       '--output-dir', str(home / 'read')], home, 'read')
        manifest = home / 'read/citations.json'
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
            lines = ['# ' + region['file'], 'Cite: ' + (full[0] if full else 'see citation-index.json for original fragments'), '']
            fence = '`' * max(3, max((len(m[0]) + 1 for m in re.finditer(r'`+', region['text'])), default=3))
            lines += [fence, region['text'], fence, '']
            view = '\n'.join(lines); visible_bytes += len(view.encode())
            need(visible_bytes <= bounds['visibleBytes'], 'SOURCE_PREPARATION_VISIBLE_LIMIT')
            (views / (key + '.md')).write_text(view, encoding='utf-8')
            view_index[key] = {'file': region['file'], 'view': 'views/' + key + '.md', 'codeBytes': len(raw),
                               'coveringCitations': full, 'originalCitations': region['citations']}
        save(home / 'citation-index.json', bindings)
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
        save(home / 'views.json', view_index)
        services[service] = {'work': scope['work'], 'revision': scope['revision'], 'coverage': scope['coverage'],
                             'sourceIndex': str(home / 'index.md'), 'views': view_index, 'manifest': str(manifest),
                             'citationIndex': str(home / 'citation-index.json'), 'requirements': requirements_index,
                             'discoveryProofs': proofs, 'records': len(bindings), 'regions': len(regions), 'union': metrics,
                             'retainedResourceRecords': resource_count,
                             'inventoryKind': 'SOURCE_FILES' if source_scope else 'COMPILER_SYMBOLS_AND_RETAINED_RESOURCES',
                             'wholeServiceOrRuntimeCompletenessClaimed': False}
        lookup = ['# ' + service + ' source index', '', 'Open the individual source views needed for your explanation. Cite supplied handles directly.',
                  'Full original bindings and exact slice reconstruction: citation-index.json.',
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
                   'Requirements and ready evidence candidates: requirements.json. All original handles and full-file proofs: ../index.json.',
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
    save(directory / 'index.json', result)
    return result


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
    pins = {'jobDigest': digest(job), 'inventoryDigest': digest(inventory), 'skillFiles': tree(SKILL, skill=True),
            'renderer': renderer, 'clewVersion': version, 'clewCapabilities': capabilities,
            'authorExecutableDigest': sha(Path(job['author']['executable']).read_bytes()),
            'nodeExecutableDigest': sha(Path(job['node']).read_bytes())}
    if 'components' in job:
        pins['componentTicketDigest'] = sha(Path(job['components']['path']).read_bytes())
    pins['inputFiles'] = tree(inputs)
    save(run / 'pins.json', pins)
    workspace = run / 'workspace'; workspace.mkdir(mode=0o700)
    (workspace / 'draft').mkdir(mode=0o700)
    save(workspace / 'inventory.json', inventory)
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


def inputs_unchanged(run):
    job = validate_job(load(run / 'inputs/job.json')); pins = load(run / 'pins.json')
    need(digest(job) == pins['jobDigest'] and digest(load(run / 'inputs/inventory.json')) == pins['inventoryDigest']
         and tree(run / 'inputs') == pins['inputFiles'] and tree(SKILL, skill=True) == pins['skillFiles']
         and sha((run / 'prompt.txt').read_bytes()) == pins['promptDigest']
         and sha(Path(job['author']['executable']).read_bytes()) == pins['authorExecutableDigest']
         and sha(Path(job['node']).read_bytes()) == pins['nodeExecutableDigest']
         and digest(load(run / 'workspace/inventory.json')) == pins['inventoryDigest'], 'FROZEN_INPUTS_CHANGED')
    need(tree(job['renderer'], skill=True) == pins['renderer']['files'], 'RENDERER_CHANGED')
    current_version = command([job['clew'], '--version'], run / 'runtime-checks', uuid.uuid4().hex)
    need(current_version == pins['clewVersion'], 'ADMITTED_RUNTIME_CHANGED', blocked=True)
    parser = subprocess.run([job['node'], str(HERE / 'docs_run_sequences.mjs'), job['renderer'], '--probe'], capture_output=True)
    need(parser.returncode == 0 and json.loads(parser.stdout) == pins['renderer']['parser'], 'RENDERER_PARSER_CHANGED')
    if 'components' in job:
        need(sha(Path(job['components']['path']).read_bytes()) == pins['componentTicketDigest'], 'COMPONENT_TICKET_CHANGED')
    return job, load(run / 'inputs/inventory.json'), pins


def cite_entries(job, inventory, draft, checks, prepared=None):
    M['configure_engine'](job['clew'], job['root'])
    manifests = sorted((draft / 'reads').rglob('citations.json'))
    if prepared is not None:
        manifests += sorted(prepared.rglob('citations.json'))
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
    for index, entry in enumerate({(v['source']['service'], v['source']['id']): v for v in entries.values()}.values()):
        if entry.get('nativeBinding', {}).get('kind') == 'SOURCE_NATIVE':
            continue  # C.load already reloaded the exact registered native bytes.
        source = entry['source']
        page = command([job['clew'], 'docs', 'context', '--root', job['root'], '--service', source['service'],
                        '--snapshot', job['snapshot'], '--source', source['id'], '--format', 'compact', '--limit', '100'], checks, 'source-' + str(index))
        records = [r['record'] for r in page.get('items', []) if r.get('kind') == 'SOURCE' and r.get('id') == source['id']]
        need(page.get('selectedSnapshot', page.get('snapshot')) == job['snapshot'] and len(records) == 1
             and all(records[0].get(key) == source.get(key) for key in ('id', 'service', 'revision', 'file', 'textDigest', 'startLine', 'endLine', 'occurrence', 'authority', 'evidenceDigest', 'textOrigin')),
             'PROVENANCE_MISMATCH', source=source['id'])
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
            closed(row, ('disposition',), ('section', 'diagram', 'evidence', 'discoveryDisposition', 'discoveredEntries'))
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
            need(isinstance(found, list) and account.get('discoveryDisposition') in ('ADDITIONAL_STARTS', 'NO_ADDITIONAL_STARTS')
                 and bool(found) == (account['discoveryDisposition'] == 'ADDITIONAL_STARTS'), 'DISCOVERY_DISPOSITION_REQUIRED')
            local_ids = set()
            for candidate in found:
                local = candidate.get('id')
                need(isinstance(local, str) and local and all(c.isascii() and (c.isalnum() or c in '-_') for c in local)
                     and local not in local_ids, 'INVALID_DISCOVERED_ENTRY_ID')
                local_ids.add(local)
                identity = requirement['id'] + ':discovered:' + local
                need(candidate.get('kind') in ('HTTP', 'KAFKA', 'SCHEDULED', 'OTHER'), 'INVALID_DISCOVERED_ENTRY_KIND')
                member = {'id': identity, 'kind': 'ENTRY', 'service': requirement['service'],
                          'page': candidate.get('page'), 'handlerId': 'discovered-' + local,
                          'sourceIds': [source['id'] for source in evidence],
                          'method': candidate.get('method'), 'path': candidate.get('path'),
                          'authority': 'AUTHOR_DISCOVERED_SYNTAX_NOT_COMPILER_BINDING',
                          'entryKind': {'HTTP': 'HTTP_ENDPOINT', 'KAFKA': 'KAFKA_LISTENER', 'SCHEDULED': 'SCHEDULED_JOB', 'OTHER': 'OTHER'}[candidate['kind']]}
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
        need(tokens['status'] != 'INVALID', 'AUTHOR_USAGE_INVALID', accounting=tokens)
        need(budget_tokens(tokens) <= job['budget']['maxModelTokens'], 'AUTHOR_TOKEN_BUDGET_EXCEEDED')
        need(tokens['status'] == 'MEASURED', 'AUTHOR_USAGE_UNAVAILABLE', blocked=True, accounting=tokens)
        last = accounting.get('attempts', [])[-1:]
        need(accounting.get('status') == 'AUTHOR_COMPLETED' and last and last[0].get('status') == 'COMPLETED'
             and last[0].get('exitCode') == 0 and last[0].get('draftFiles') == tree(draft), 'AUTHOR_COMPLETION_REQUIRED')
        checks[stage] = {'status': 'PASS', 'usageAvailable': tokens['status'] == 'MEASURED',
                         'reportedModelTokens': tokens['budgetTokens'], 'tokenAccounting': tokens}
        stage = 'output'; before = tree(draft)
        need(before and 'original.json' in before, 'MISSING_REQUIRED_OUTPUT')
        original = load(draft / 'original.json')
        if 'components' in job:
            M['configure_engine'](job['clew'], job['root'], job['components']['path'], job['components']['digest'])
        pages, _, _ = M['compose_pages'](original)
        need(all(r['page'] in pages and pages[r['page']].strip() for r in inventory['requirements']), 'MISSING_REQUIRED_PAGE')
        checks[stage] = {'status': 'PASS', 'pages': len(pages)}
        stage = 'provenance'; manifests, entries = cite_entries(job, inventory, draft, directory / 'sources', run / 'inputs/author-source')
        checks[stage] = {'status': 'PASS', 'handles': len(entries)}
        stage = 'selfReview'; review = load(draft / 'self-review.json')
        defaults = {}
        for path in (run / 'inputs/author-source').glob('*/requirements.json'):
            defaults.update({key: value['evidenceCandidates'] for key, value in load(path).items()})
        review = resolve_review(review, inventory, entries, original, defaults)
        save(directory / 'resolved-self-review.json', review)
        accounts, effective, discovery_proofs = review_accounts(review, inventory, pages, entries, original)
        checks[stage] = {'status': 'PASS', 'discoveryCoverage': discovery_proofs}; result['semanticReviewRecorded'] = True
        result['authorVisualReview'] = review['visual']
        stage = 'entrySequences'; request = directory / 'sequences.json'
        save(request, {'pages': {k: v.decode() for k, v in pages.items()}, 'requirements': effective, 'accounts': accounts})
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
        argv = [job['node'], str(Path(job['renderer']) / 'render.mjs'), '{input}', '{output}', job['request'],
                '--theme', 'codeclew', '--lang', job['language'], '--navigation', str(draft / 'navigation.json')]
        if len(job['services']) > 1:
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


def author(run):
    run = Path(run).absolute(); job, inventory, _ = inputs_unchanged(run)
    state = load(run / 'state.json'); attempts = state['attempts']
    tokens = token_accounting(attempts)
    need(tokens['status'] != 'INVALID', 'AUTHOR_USAGE_INVALID', accounting=tokens)
    need(budget_tokens(tokens) < job['budget']['maxModelTokens'], 'AUTHOR_TOKEN_BUDGET_EXHAUSTED')
    need(tokens['status'] == 'MEASURED', 'AUTHOR_USAGE_UNAVAILABLE', blocked=True, accounting=tokens)
    if state['authorStartedAt'] is None:
        state['authorStartedAt'] = time.time()
    remaining = job['budget']['wallSeconds'] - (time.time() - state['authorStartedAt'])
    need(remaining > 0 and len(attempts) <= job['budget']['maxRepairs'], 'AUTHOR_BUDGET_EXHAUSTED')
    folder = run / 'attempts' / str(len(attempts)); folder.mkdir(mode=0o700, parents=True)
    if not attempts:
        prompt = (run / 'prompt.txt').read_bytes()
    else:
        failures = load(run / 'report.json') if (run / 'report.json').exists() else {'status': 'INTERRUPTED'}
        prompt = encode({'instruction': 'Repair the draft and repeat your bidirectional self-review. Preserve scope and existing unchanged reads; acquire only needed missing sources. Only machine failures follow, not a semantic answer key.',
                         'failures': failures.get('checks', failures),
                         'remainingSeconds': int(remaining), 'remainingRepairs': job['budget']['maxRepairs'] - len(attempts)})
    (folder / 'prompt.txt').write_bytes(prompt)
    backend = job['author']
    argv = [backend['executable'], 'exec', '--ignore-user-config', '--skip-git-repo-check', '--json',
            '-m', backend['model'], '-c', 'model_reasoning_effort=' + json.dumps(backend['reasoningEffort']),
            '-c', 'approval_policy="never"', '-s', 'workspace-write', '-C', str(run / 'workspace'),
            '--add-dir', job['root'], '--disable', 'multi_agent', '-o', str(folder / 'last-message.txt')]
    if backend.get('stateRoot'):
        argv += ['--add-dir', backend['stateRoot']]
    if attempts:
        need(state.get('threadId'), 'INTERRUPTED_AUTHOR_SESSION_UNAVAILABLE')
        argv += ['resume', state['threadId']]
    argv += ['-']
    attempt = {'argv': argv, 'status': 'RUNNING', 'startedAt': time.time(), 'exitCode': None}
    attempts.append(attempt); state['status'] = 'AUTHOR_RUNNING'; save(run / 'state.json', state)
    usage = None; interrupted = False
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
                state['threadId'] = event['thread_id']
            if event.get('type') == 'turn.completed':
                usage = event.get('usage')
    attempt['draftFiles'] = tree(run / 'workspace/draft')
    shutil.copytree(run / 'workspace/draft', folder / 'draft')
    attempt.update(status='INTERRUPTED' if interrupted else 'COMPLETED', exitCode=process.returncode, usage=usage)
    tokens = token_accounting(attempts)
    state['tokenAccounting'] = tokens
    state['usageAvailable'] = tokens['status'] == 'MEASURED'
    state['modelTokens'] = tokens['budgetTokens']
    state['status'] = ('INTERRUPTED' if interrupted else 'AUTHOR_FAILED' if process.returncode != 0 else
                       'AUTHOR_USAGE_INVALID' if tokens['status'] == 'INVALID' else
                       'AUTHOR_BUDGET_EXCEEDED' if budget_tokens(tokens) > job['budget']['maxModelTokens'] else
                       'AUTHOR_USAGE_UNAVAILABLE' if tokens['status'] != 'MEASURED' else 'AUTHOR_COMPLETED')
    save(run / 'state.json', state); save(folder / 'result.json', attempt)
    need(not interrupted and process.returncode == 0, 'AUTHOR_NOT_COMPLETED')
    need(tokens['status'] != 'INVALID', 'AUTHOR_USAGE_INVALID', accounting=tokens)
    need(budget_tokens(tokens) <= job['budget']['maxModelTokens'], 'AUTHOR_TOKEN_BUDGET_EXCEEDED')
    need(tokens['status'] == 'MEASURED', 'AUTHOR_USAGE_UNAVAILABLE', blocked=True, accounting=tokens)
    # This is reported turn accounting, available after a turn, not a provider
    # billing guarantee or a continuously enforceable in-turn token ceiling.
    return state


def execute(run, *, debug=False):
    run = Path(run).absolute(); state = load(run / 'state.json')
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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('prepare', 'run', 'verify', 'promote', 'review-template'))
    parser.add_argument('--job', type=Path)
    parser.add_argument('--run-dir', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path)
    parser.add_argument('--debug', action='store_true')
    args = parser.parse_args()
    lock = None
    try:
        if args.command not in ('prepare', 'review-template'):
            need(args.run_dir.is_dir() and not args.run_dir.is_symlink(), 'RUN_NOT_FOUND')
            lock = os.open(args.run_dir / '.runner.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise Stop('RUN_BUSY', blocked=True)
        if args.command == 'review-template':
            _, inventory, _ = inputs_unchanged(args.run_dir.absolute())
            result = review_template(inventory, load(args.run_dir / 'workspace/draft/original.json'))
            print(encode(result).decode(), end='')
            return 0
        elif args.command == 'prepare':
            need(args.job is not None, 'JOB_REQUIRED'); result = prepare(args.job, args.run_dir)
        elif args.command == 'run': result = execute(args.run_dir, debug=args.debug)
        elif args.command == 'verify': result = verify(args.run_dir, debug=args.debug)
        else: result = promote(args.run_dir, args.output_dir)
    except (Stop, OSError, ValueError, TypeError, KeyError) as error:
        result = {'status': 'BLOCKED' if getattr(error, 'blocked', False) else 'FAILED',
                  'code': getattr(error, 'code', 'INVALID_RUN_INPUT'), 'details': getattr(error, 'details', str(error)),
                  'publicationPerformed': False}
    finally:
        if lock is not None:os.close(lock)
    print(encode(result).decode(), end='')
    return 0 if result['status'] in ('PREPARED', 'READY', 'READY_PROMOTED') else 2


if __name__ == '__main__':
    raise SystemExit(main())
