"""Admit completed isolated pages from explicitly trusted local release runs."""
from __future__ import annotations

import base64
from collections import Counter, defaultdict
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat

LEDGER = 'page-reuse.json'
PENDING = '.page-reuse-pending.json'
SCHEMA = 'codeclew-docs-page-reuse/1.0'
MODE = 'ISOLATED_PAGES'
MAX_ORIGINS = 4
LOG_LIMIT = 2 * 1024 * 1024
RESPONSE_LIMIT = 128 * 1024
DIGEST = re.compile(r'sha256:[0-9a-f]{64}')
COMMIT = re.compile(r'[0-9a-f]{40}')
OFFICIAL = 'codeclew-release/2.0'
LOCAL = 'codeclew-local-release-candidate/1.0'
SLOT_KEY = re.compile(r'[a-z][a-z0-9_-]{0,127}')
SLOT_FIELDS = ('key', 'kind', 'page', 'service', 'requirement', 'citations', 'citationSpans', 'sourceRegions', 'returns')
SCOPE_FIELDS = ('revision', 'contextDigest', 'inputDigest', 'coverage', 'sourceInventory')
ROUTING_ERROR = 'workspace routing discovery failed'
ROUTING_FAILURE = {'code': 'PAGE_AUTHOR_SERVICE_UNAVAILABLE', 'details': {'backendMessage': ROUTING_ERROR}}


def exact_file(root, relative, maximum):
    rel = PurePosixPath(relative)
    need(not rel.is_absolute() and rel.parts and '..' not in rel.parts, 'REUSE_UNSAFE_PATH')
    path = root
    for part in rel.parts:
        path = path / part
        need(not path.is_symlink(), 'REUSE_UNSAFE_PATH')
    need(path.is_file() and path.stat().st_size <= maximum, 'REUSE_FILE_UNAVAILABLE')
    return path


def slot_files(run, slot):
    return {field: exact_file(run / 'inputs', slot[field], LOG_LIMIT).read_bytes() for field in ('prompt', 'schema')}


def slot_scope(slot):
    return {field: slot[field] for field in SLOT_FIELDS}


@contextmanager
def read_only_run(path):
    root = path.absolute()
    need(root.is_dir() and not root.is_symlink(), 'REUSE_RUN_UNAVAILABLE')
    guard = exact_file(root, '.runner.lock', 4096)
    fd = os.open(guard, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
        except BlockingIOError:
            raise Stop('REUSE_SOURCE_BUSY') from None
        yield root
    finally:
        os.close(fd)


def skill_digest(root, names):
    result = hashlib.sha256()
    for name in sorted(names):
        file = root / name
        result.update(name.encode() + b'\0' + (b'493' if file.stat().st_mode & 0o111 else b'420') + b'\0')
        result.update(file.read_bytes() + b'\0')
    return 'sha256:' + result.hexdigest()


def canonical(value):
    # Release seed, source and runtime seals use compact JSON.
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'), sort_keys=True).encode()


def sealed_json(package, relative, *, maximum=LOG_LIMIT):
    file = exact_file(package, relative, maximum)
    metadata = file.stat()
    raw = file.read_bytes()
    value = decode(raw)
    need(isinstance(value, dict) and metadata.st_uid == os.geteuid()
         and stat.S_IMODE(metadata.st_mode) == 0o400 and raw == canonical(value) + b'\n',
         'REUSE_RELEASE_KIT_CHANGED')
    return value


def official_release_proof(package, release, caps, kit, pinned_files):
    revision, source_tree = release['sourceRevision'], release['sourceTree']
    need(release['status'] == 'PILOT_READY' and release['profile'] == 'core'
         and COMMIT.fullmatch(revision) and COMMIT.fullmatch(source_tree), 'REUSE_RELEASE_KIT_CHANGED')
    epoch = f'seed/release-N-{revision}'
    seed_path = epoch + '/seed.json'
    seeds = [path.relative_to(package).as_posix() for path in (package / 'seed').glob('release-N-*/seed.json')]
    need(seeds == [seed_path], 'REUSE_RELEASE_KIT_CHANGED')
    seed = sealed_json(package, seed_path)
    key = caps['runtimeKey']
    seed_digest = seed.get('seedDigest')
    unsigned = {name: value for name, value in seed.items() if name != 'seedDigest'}
    need(seed['schema'] == 'codeclew-trusted-release-seed/2.0' and seed['mode'] == 'RELEASE'
         and seed['sourceRevision'] == revision and seed['sourceTree'] == source_tree
         and seed['runtimeKey'] == key and seed['manifestDigest'] == caps['runtimeManifestDigest']
         and seed['stateEpoch'] == sha((key + '\0' + revision).encode())
         and seed_digest == sha(canonical(unsigned)), 'REUSE_RELEASE_KIT_CHANGED')

    source = sealed_json(package, 'source/release-source.json', maximum=32 * 1024 * 1024)
    source_digest = seed['sourcePayloadDigest']
    need(source['schema'] == 'codeclew-release-source/1.0'
         and source['sourceRevision'] == revision and source['sourceTree'] == source_tree
         and source['manifestDigest'] == source_digest == sha(canonical({**source, 'manifestDigest': ''})),
         'REUSE_RELEASE_KIT_CHANGED')
    prefix = 'skills/codeclew/'
    rows = [row for row in source['files'] if row['path'].startswith(prefix)]
    sealed = {row['path'].removeprefix(prefix): (row['sha256'], row['size'], row['mode']) for row in rows}
    observed = {name: (digest, (kit / name).stat().st_size, 0o111 if (kit / name).stat().st_mode & 0o111 else 0)
                for name, digest in pinned_files.items()}
    need(len(sealed) == len(rows) and sealed == observed, 'REUSE_RELEASE_KIT_CHANGED')

    capsule = epoch + '/parallel-state/v2/runtimes/' + key.removeprefix('sha256:')
    runtime = sealed_json(package, capsule + '/runtime.json')
    need(runtime['schema'] == 'codeclew-runtime-capsule/4.0' and runtime['mode'] == 'RELEASE'
         and runtime['runtimeKey'] == key
         and runtime['manifestDigest'] == seed['manifestDigest'] == sha(canonical({**runtime, 'manifestDigest': ''}))
         and seed['artifactHashes'] == {name: row['sha256'] for name, row in runtime['artifacts'].items()}
         and seed['workerTreeHashes'] == {name: row['treeHash'] for name, row in runtime['workers'].items()}
         and exact_file(package, capsule + '/READY', LOG_LIMIT).read_text() == key + '\n', 'REUSE_RELEASE_KIT_CHANGED')
    return {'packageSchema': OFFICIAL, 'seedDigest': seed_digest, 'sourcePayloadDigest': source_digest}


def release_proof(launcher, pins):
    caps = pins['clewCapabilities']
    package = launcher.resolve(strict=True).parents[1]
    release = load(exact_file(package, 'release.json', LOG_LIMIT))
    kit = package / 'source/skills/codeclew'
    pinned_files = pins['skillFiles']
    need(caps['runtimeMode'] == 'RELEASE' and DIGEST.fullmatch(caps['runtimeKey'])
         and DIGEST.fullmatch(caps['runtimeManifestDigest'])
         and not (package / 'source').is_symlink() and not kit.parent.is_symlink()
         and tree(kit, skill=True) == pinned_files
         and skill_digest(kit, pinned_files) == caps['agentContract']['skillDigest'], 'REUSE_RELEASE_KIT_CHANGED')
    if release.get('schema') == OFFICIAL:
        return official_release_proof(package, release, caps, kit, pinned_files)
    need(release.get('schema') == LOCAL and release['runtimeKey'] == caps['runtimeKey']
         and release['runtimeManifestDigest'] == caps['runtimeManifestDigest'], 'REUSE_RELEASE_KIT_CHANGED')
    return {'packageSchema': LOCAL}


def run_scope(job, inventory):
    return {
        'snapshot': job['snapshot'], 'sourceSnapshot': inventory['snapshot'], 'services': job['services'],
        'mode': job.get('authorMode'), 'evidenceMode': job.get('evidenceMode'),
        'language': job['language'], 'request': job['request'],
        'sourceScopes': {service: {key: inventory['services'][service].get(key) for key in SCOPE_FIELDS}
                         for service in job['services']},
        'author': {key: job['author'].get(key) for key in ('model', 'reasoningEffort', 'stateRoot')},
    }


def usage_summary(accounting):
    return {key: accounting[key] for key in ('status', 'counterScope', 'budgetTokens',
                                              'knownBudgetTokensLowerBound', 'monetaryCost')}


def history(run, target_job, target_inventory, target_pins):
    pins = load(exact_file(run, 'pins.json', LOG_LIMIT))
    job = load(exact_file(run, 'inputs/job.json', LOG_LIMIT))
    inventory = load(exact_file(run, 'inputs/inventory.json', 32 * 1024 * 1024))
    need(job.get('authorMode') == MODE and run_scope(job, inventory) == run_scope(target_job, target_inventory),
         'REUSE_RUN_SCOPE_MISMATCH')
    need(digest(job) == pins['jobDigest'] and digest(inventory) == pins['inventoryDigest']
         and tree(run / 'inputs') == pins['inputFiles']
         and sha(exact_file(run, 'prompt.txt', LOG_LIMIT).read_bytes()) == pins['promptDigest'],
         'REUSE_SOURCE_INPUTS_CHANGED')
    need(pins['authorExecutableDigest'] == target_pins['authorExecutableDigest'], 'REUSE_AUTHOR_CHANGED')
    caps = pins['clewCapabilities']
    package_proof = release_proof(Path(job['clew']), pins)
    state_file = exact_file(run, 'state.json', 32 * 1024 * 1024)
    seal = sha(state_file.read_bytes())
    state = load(state_file)
    need(state.get('schema') == 'codeclew-docs-run-state/1.0' and isinstance(state.get('attempts'), list)
         and len(state['attempts']) <= 512, 'REUSE_HISTORY_INVALID')
    old_plan = load(exact_file(run, 'inputs/author-plan.json', 32 * 1024 * 1024))
    need(old_plan['mode'] == MODE, 'REUSE_RUN_SCOPE_MISMATCH')
    rows = defaultdict(list)
    owners = defaultdict(set)
    attributable = [row for row in state['attempts'] if not row.get('notLaunched')]
    account_indexes = defaultdict(list)
    for index, attempt in enumerate(attributable):
        rows[attempt['slot']].append(attempt)
        account_indexes[attempt['slot']].append(index)
        if thread := attempt.get('threadId'):
            owners[thread].add(attempt['slot'])
    historical = token_accounting(attributable)
    usage_matches = (canonical(state.get('tokenAccounting')) == canonical(historical)
                     and state.get('modelTokens') == historical['budgetTokens']
                     and state.get('usageAvailable') is (historical['status'] == 'MEASURED'))
    receipt = {
        'run': str(run), 'pinsDigest': digest(pins), 'stateDigest': seal, 'runtimeKey': caps['runtimeKey'],
        'skillDigest': caps['agentContract']['skillDigest'], 'author': run_scope(job, inventory)['author'],
        'releaseKit': package_proof,
        'recordedAttempts': len(state['attempts']),
        'reportedUsage': usage_summary(historical),
    }
    return job, {slot['key']: slot for slot in old_plan['slots']}, rows, owners, receipt, (usage_matches, account_indexes)


def expected_argv(run, slot, number, backend, resume, flags, *, reviewed=False):
    folder = run / 'page-attempts' / slot['key'] / str(number)
    schema = folder / 'docs-run-page-author.schema.json' if reviewed else run / 'inputs' / slot['schema']
    return [backend['executable'], 'exec', '--ignore-user-config', '--skip-git-repo-check', '--json',
            '-m', backend['model'], '-c', 'model_reasoning_effort=' + json.dumps(backend['reasoningEffort']),
            '-c', 'approval_policy="never"', '-C', str(folder / 'context'), '--disable', 'multi_agent',
            '-o', str(folder / 'last-message.txt'), '-s', 'read-only', *flags(schema),
            *(['resume', resume] if resume is not None else []), '-']


def expected_prompt(slot, original, previous):
    if previous is None:
        return original
    failure = previous.get('responseFailure') or previous.get('failure')
    need(isinstance(failure, dict), 'REUSE_UNPROVEN_REPAIR')
    details = failure.get('details', {})
    if failure['code'] == 'PAGE_RESPONSE_INVALID' and details.get('formatOnly') is True:
        correction = {'slot': slot['key'], 'type': slot['kind'], 'formatRepair': details,
                      'instruction': 'Use the retained packet already in this session. Correct the named JSON fields and '
                                     'return the complete slot response in the supplied schema. Keep the claims, '
                                     'ordering and conditions. Cite only aliases that actually support the corresponding text.'}
        return (json.dumps(correction, ensure_ascii=False, separators=(',', ':')) + '\n').encode()
    details = encode({'instruction': 'Repair only this slot using its retained code. Check the stated issue and preserve supported behavior.',
                      'failure': failure})
    return b'Repair for the packet below:\n' + details + b'\n' + original


def checked_attempt_evidence(run, slot, attempt, previous, backend, flags, *, reviewed=False):
    key, number = slot['key'], attempt['number']
    need(type(number) is int and number >= 0, 'REUSE_ATTEMPT_NOT_ADMITTED')
    relative = f'page-attempts/{key}/{number}'
    thread = attempt.get('threadId')
    resume = previous['threadId'] if previous is not None else None
    started = attempt['startedAt']
    need(attempt['authorMode'] == MODE and type(attempt['exitCode']) is int
         and isinstance(thread, str) and 0 < len(thread) <= 192
         and (resume is None or thread == resume) and attempt['resumeThread'] == resume
         and started <= attempt.get('launchedAt', started) <= attempt['completedAt']
         and (previous is None or previous['completedAt'] <= started)
         and attempt['argv'] == expected_argv(run, slot, number, backend, resume, flags, reviewed=reviewed),
         'REUSE_ATTEMPT_NOT_ADMITTED')
    result = exact_file(run, relative + '/result.json', LOG_LIMIT)
    context = result.parent / 'context'
    need(context.is_dir() and not context.is_symlink() and next(context.iterdir(), None) is None,
         'REUSE_AUTHOR_CONTEXT_CHANGED')
    need(canonical(load(result)) == canonical(attempt), 'REUSE_ATTEMPT_RECEIPT_CHANGED')
    prompt = exact_file(run, relative + '/prompt.txt', LOG_LIMIT).read_bytes()
    need('promptDigest' not in attempt or attempt['promptDigest'] == sha(prompt), 'REUSE_ATTEMPT_PROMPT_CHANGED')
    stdout = exact_file(run, relative + '/stdout.jsonl', LOG_LIMIT).read_bytes()
    events = [decode(line) for line in stdout.splitlines() if line.strip()]
    need(len(events) >= 4 and events[0].get('type') == 'thread.started'
         and events[0].get('thread_id') == thread and events[1].get('type') == 'turn.started',
         'REUSE_AUTHOR_STREAM_CHANGED')
    return result, prompt, stdout, events


def checked_completed_stream(attempt, result, events):
    need(attempt['status'] == 'COMPLETED' and attempt['exitCode'] == 0 and 'failure' not in attempt,
         'REUSE_ATTEMPT_NOT_ADMITTED')
    raw = exact_file(result.parent, 'last-message.txt', RESPONSE_LIMIT).read_bytes()
    need(sha(raw) == attempt['responseDigest'], 'REUSE_RESPONSE_CHANGED')
    need(events[-1].get('type') == 'turn.completed'
         and canonical(events[-1].get('usage')) == canonical(attempt['usage']),
         'REUSE_AUTHOR_STREAM_CHANGED')
    final_message = None
    for event in events[2:-1]:
        item = event.get('item') or {}
        need(event['type'] in ('item.started', 'item.updated', 'item.completed')
             and item.get('type') in ('agent_message', 'reasoning'), 'REUSE_AUTHOR_ACCESS_CHANGED')
        if event['type'] == 'item.completed' and item['type'] == 'agent_message':
            final_message = item.get('text')
    need(isinstance(final_message, str) and final_message.encode().rstrip(b'\r\n') == raw.rstrip(b'\r\n')
         and token_accounting([attempt])['status'] == 'MEASURED', 'REUSE_AUTHOR_STREAM_CHANGED')
    return raw


def check_routing_outage(attempt, result, events):
    need(attempt['status'] == 'FAILED' and attempt['exitCode'] > 0 and attempt['number'] == 0
         and attempt.get('failure') == ROUTING_FAILURE and 'responseFailure' not in attempt
         and attempt['usage'] is None and 'responseDigest' not in attempt, 'REUSE_ATTEMPT_NOT_ADMITTED')
    # A failed dispatch may create an empty output file before it gets a response.
    output = result.parent / 'last-message.txt'
    if output.exists() or output.is_symlink():
        need(exact_file(result.parent, 'last-message.txt', RESPONSE_LIMIT).stat().st_size == 0,
             'REUSE_RESPONSE_CHANGED')
    # The CLI reports WebSocket retries starting at 2 and HTTPS retries at 1
    # after fallback. Keep this admission narrower than the live retry classifier.
    websocket_retries, https_retries = [
        [{'type': 'error', 'message': f'Reconnecting... {number}/5 ({ROUTING_ERROR})'}
         for number in range(start, 6)]
        for start in (2, 1)
    ]
    fallback = {'type': 'item.completed', 'item': {'id': 'item_0', 'type': 'error',
                'message': f'Falling back from WebSockets to HTTPS transport. {ROUTING_ERROR}'}}
    need(events == [
        {'type': 'thread.started', 'thread_id': attempt['threadId']},
        {'type': 'turn.started'},
        *websocket_retries, fallback, *https_retries,
        {'type': 'error', 'message': ROUTING_ERROR},
        {'type': 'turn.failed', 'error': {'message': ROUTING_ERROR}},
    ], 'REUSE_AUTHOR_STREAM_CHANGED')


def attempt_proof(attempt, result, prompt, stdout):
    return {
        'attempt': attempt['number'], 'threadId': attempt['threadId'], 'receiptDigest': sha(result.read_bytes()),
        'streamDigest': sha(stdout), 'attemptPromptDigest': sha(prompt),
        'historicalCumulativeUsage': attempt['usage'],
    }


def checked_attempt(run, slot, original, attempt, previous, backend, flags, *, recovering=False):
    retry = previous is not None and previous.get('failure') == ROUTING_FAILURE
    outage = previous is None and attempt.get('failure') == ROUTING_FAILURE
    need(('serviceRetry' not in attempt and not retry) or (attempt.get('serviceRetry') is True and retry),
         'REUSE_ATTEMPT_NOT_ADMITTED')
    if outage or retry or recovering:
        need('launchedAt' in attempt and 'promptDigest' in attempt, 'REUSE_ATTEMPT_NOT_ADMITTED')
    result, prompt, stdout, events = checked_attempt_evidence(run, slot, attempt, previous, backend, flags)
    expected = original if retry else expected_prompt(slot, original, previous)
    # Format repair details were persisted with sorted keys. Match the
    # structured correction without relying on its historic insertion order.
    if previous is not None and not retry and expected.startswith(b'{'):
        need(decode(prompt) == decode(expected), 'REUSE_ATTEMPT_PROMPT_CHANGED')
    else:
        need(prompt == expected, 'REUSE_ATTEMPT_PROMPT_CHANGED')
    proof = attempt_proof(attempt, result, prompt, stdout)
    if outage:
        check_routing_outage(attempt, result, events)
        return None, proof
    raw = checked_completed_stream(attempt, result, events)
    return raw, dict(proof, responseDigest=sha(raw))


def check_routing_repair(slot, previous, raw, validate):
    try:
        validate(slot, decode(raw))
    except Stop as error:
        failure = {'code': error.code, 'details': error.details}
        need(error.code == 'PAGE_RESPONSE_INVALID' and error.details.get('formatOnly') is True
             and canonical(failure) == canonical(previous.get('responseFailure')), 'REUSE_UNPROVEN_REPAIR')
        return
    raise Stop('REUSE_UNPROVEN_REPAIR')


def checked_history(run, slot, original, history, owners, backend, flags, validate, usage_matches, account_indexes):
    recovery, completed = [], []
    key, previous, previous_raw = slot['key'], None, None
    for number, attempt in enumerate(history):
        need(type(attempt['number']) is int and attempt['number'] == number
             and owners[attempt['threadId']] == {key}, 'REUSE_THREAD_SCOPE_CHANGED')
        if recovery and previous_raw is not None:
            check_routing_repair(slot, previous, previous_raw, validate)
        raw, proof = checked_attempt(run, slot, original, attempt, previous, backend, flags, recovering=bool(recovery))
        routed = bool(recovery) or raw is None
        if routed:
            need(usage_matches, 'REUSE_HISTORICAL_COUNTERS_CHANGED')
            proof['reportedUsageIndex'] = account_indexes[number]
        if raw is not None:
            completed.append(attempt)
            need(token_accounting(completed)['status'] == 'MEASURED', 'REUSE_HISTORICAL_COUNTERS_CHANGED')
            candidate = proof
            if recovery:
                chain_usage = token_accounting(history[:number + 1])
                need(chain_usage['status'] == 'UNAVAILABLE'
                     and all(row['attempt'] == 0 for row in chain_usage['missing']), 'REUSE_HISTORICAL_COUNTERS_CHANGED')
                candidate = dict(proof, routingRecovery={
                    'priorAttempts': recovery.copy(), 'usage': usage_summary(chain_usage)})
            yield raw, candidate
        if routed:
            recovery.append(proof)
        previous, previous_raw = attempt, raw


def candidates(run, target, plan, job, inventory, pins, flags, validate, reserved, rejected):
    source_job, old, histories, owners, receipt, (usage_matches, account_indexes) = history(run, job, inventory, pins)
    values = {}
    for slot in plan['slots']:
        key = slot['key']
        if key in reserved or key not in old or not histories.get(key):
            continue
        try:
            need(SLOT_KEY.fullmatch(key) and slot_scope(slot) == slot_scope(old[key]),
                 'REUSE_SLOT_SCOPE_CHANGED')
            pinned = slot_files(target, slot)
            need(pinned == slot_files(run, old[key]), 'REUSE_PROMPT_OR_SCHEMA_CHANGED')
        except (Stop, OSError, ValueError, TypeError, KeyError, RecursionError) as error:
            rejected[getattr(error, 'code', 'REUSE_SLOT_INVALID')] += 1
            continue
        validated = []
        try:
            validated.extend(checked_history(run, old[key], pinned['prompt'], histories[key], owners,
                                              source_job['author'], flags, validate, usage_matches,
                                              account_indexes[key]))
        except (Stop, OSError, ValueError, TypeError, KeyError, IndexError, AttributeError, RecursionError) as error:
            rejected[getattr(error, 'code', 'REUSE_ATTEMPT_INVALID')] += 1
        for raw, proof in reversed(validated):
            try:
                validate(slot, decode(raw))
            except (Stop, OSError, ValueError, TypeError, KeyError, IndexError, AttributeError, RecursionError) as error:
                rejected[getattr(error, 'code', 'REUSE_RESPONSE_INVALID')] += 1
                continue
            values[key] = dict(proof, raw=base64.b64encode(raw).decode(), scopeDigest=digest(slot_scope(slot)),
                               promptDigest=sha(pinned['prompt']), schemaDigest=sha(pinned['schema']))
            break
    need(sha((run / 'state.json').read_bytes()) == receipt['stateDigest'], 'REUSE_HISTORY_CHANGED')
    return values, receipt


def ready_proof(run, state, pins):
    report = load(exact_file(run, 'report.json', LOG_LIMIT))
    pointer = load(exact_file(run, 'ready/current.json', LOG_LIMIT))
    usage = token_accounting([row for row in state['attempts'] if not row.get('notLaunched')])
    need(report['status'] == 'READY' and report['inputDigest'] == digest(pins)
         and DIGEST.fullmatch(pointer['reportDigest']) and pointer['publicationPerformed'] is False
         and state['status'] == 'AUTHOR_COMPLETED'
         and set(report['checks']) == set(checks) and all(row['status'] == 'PASS' for row in report['checks'].values())
         and report['checks']['author']['tokenAccounting'] == usage
         and state['pageDraftFiles'] == report['draftFiles'] == tree(run / 'workspace/draft')
         and report['draftDigest'] == digest(report['draftFiles']) and DIGEST.fullmatch(pointer['outputDigest'])
         and pointer['output'] == pointer['outputDigest'].removeprefix('sha256:'), 'PAGE_REVIEW_DONOR_NOT_READY')
    promoted = run / 'ready' / pointer['output']
    need(digest(tree(promoted)) == pointer['outputDigest'] and tree(promoted / 'html') == report['deliverableFiles'],
         'PAGE_REVIEW_DONOR_OUTPUT_CHANGED')
    return {'reportDigest': digest(report), 'promotedReportDigest': pointer['reportDigest'], 'outputDigest': pointer['outputDigest'],
            'draftFiles': report['draftFiles']}


def review_work(run, plan, retained, ledger):
    if ledger is None or 'review' not in ledger:
        return {}, {}
    rows = review_api['findings'](ledger['review'])
    slots = {slot['key']: slot for slot in plan['slots']}
    need(rows.keys() <= retained.keys() == slots.keys(), 'PAGE_REVIEW_SLOT_OUTSIDE_RUN')
    work = {key: review_api['author_input'](run, slots[key], ledger['pages'][key], retained[key], row)
            for key, row in rows.items()}
    return rows, work


def reviewed_attempt_evidence(run, backend, slot, attempt, previous, flags, review):
    result, prompt, stdout, events = checked_attempt_evidence(run, slot, attempt, previous, backend, flags, reviewed=True)
    schema = exact_file(result.parent, 'docs-run-page-author.schema.json', LOG_LIMIT)
    need(load(schema) == review[1] and prompt == review[0] and 'promptDigest' in attempt and 'launchedAt' in attempt,
         'PAGE_REVIEW_AUTHOR_INPUT_CHANGED', slot=slot['key'])
    return result, events, attempt_proof(attempt, result, prompt, stdout)


def admitted_completed(run, job, slot, histories, flags, *, review=None):
    last = histories[-1]
    previous = histories[-2] if len(histories) > 1 else None
    backend = job['author']
    if review is None:
        result, prompt, stdout, events = checked_attempt_evidence(run, slot, last, previous, backend, flags)
        # A promoted donor also proves these responses reproduce its accepted
        # draft. Settled repair prompts can include scheduler diagnostics.
        need(prompt == slot_files(run, slot)['prompt'] if previous is None
             else not last.get('serviceRetry') and 'promptDigest' in last
             and token_accounting(histories)['status'] == 'MEASURED', 'REUSE_ATTEMPT_PROMPT_CHANGED')
        proof = attempt_proof(last, result, prompt, stdout)
    else:
        result, events, proof = reviewed_attempt_evidence(run, backend, slot, last, previous, flags, review)
        if previous is not None:
            need(len(histories) == 2 and last['number'] == 1 and last.get('serviceRetry') is True,
                 'REUSE_ATTEMPT_NOT_ADMITTED')
            failed, failed_events, outage = reviewed_attempt_evidence(run, backend, slot, previous, None, flags, review)
            check_routing_outage(previous, failed, failed_events)
            proof['routingRecovery'] = {'priorAttempts': [outage], 'usage': usage_summary(token_accounting(histories))}
        else:
            need('serviceRetry' not in last, 'REUSE_ATTEMPT_NOT_ADMITTED')
    raw = checked_completed_stream(last, result, events)
    return raw, dict(proof, responseDigest=sha(raw))


def review_candidates(run, target, plan, job, inventory, pins, flags, validate, assemble):
    source_job, old, histories, owners, receipt, (usage_matches, account_indexes) = history(run, job, inventory, pins)
    source_pins = load(run / 'pins.json')
    state = load(run / 'state.json')
    old_plan = load(run / 'inputs/author-plan.json')
    acceptance = ready_proof(run, state, source_pins)
    retained, ledger = read_pages(run, state, old_plan, validate)
    rows, work = review_work(run, old_plan, retained, ledger)
    need(old_plan == plan and all(slot_files(target, slot) == slot_files(run, old[slot['key']]) for slot in plan['slots']),
         'PAGE_REVIEW_DONOR_SCOPE_CHANGED')
    need(histories.keys() | retained.keys() == old.keys() and set(rows) <= histories.keys()
         and all(len(keys) == 1 for keys in owners.values()), 'PAGE_REVIEW_DONOR_HISTORY_CHANGED')
    proofs, responses = {}, {}
    if ledger is not None:
        receipt['priorReuse'] = {'ledgerDigest': state['pageReuseDigest'], 'origins': ledger['origins']}
    for key, slot in old.items():
        pinned = slot_files(run, slot)
        prior = ledger['pages'].get(key) if ledger else None
        if key in histories:
            attempts = histories[key]
            if key not in rows and any(row.get('failure') == ROUTING_FAILURE or row.get('serviceRetry') for row in attempts):
                completed = list(checked_history(run, slot, pinned['prompt'], attempts, owners, source_job['author'],
                                                 flags, validate, usage_matches, account_indexes[key]))
                need(completed and completed[-1][1]['attempt'] == attempts[-1]['number'], 'REUSE_ATTEMPT_NOT_ADMITTED')
                raw, proof = completed[-1]
            else:
                raw, proof = admitted_completed(run, source_job, slot, attempts, flags, review=work.get(key))
            if key in rows:
                response = review_api['apply'](slot, retained[key], rows[key], decode(raw), validate)
                raw = encode(response)
                proof = {**proof, 'correctionDigest': proof['responseDigest'], 'responseDigest': sha(raw),
                         'priorResponseDigest': prior['responseDigest'], 'priorOrigin': prior['origin']}
            else:
                response = validate(slot, decode(raw))
        else:
            proof = dict(prior, priorOrigin=prior['origin'])
            raw, response = base64.b64decode(prior['raw'], validate=True), retained[key]
        responses[key] = response
        proofs[key] = dict(proof, raw=base64.b64encode(raw).decode(), scopeDigest=digest(slot_scope(slot)),
                           promptDigest=sha(pinned['prompt']), schemaDigest=sha(pinned['schema']))
    threads = [proof['threadId'] for proof in proofs.values()]
    need(len(set(threads)) == len(threads) and assemble(run, source_job, old_plan, responses) == acceptance.pop('draftFiles')
         and sha((run / 'state.json').read_bytes()) == receipt['stateDigest'], 'PAGE_REVIEW_DONOR_HISTORY_CHANGED')
    receipt['ready'] = acceptance
    return proofs, receipt


def import_pages(run, sources, job, inventory, pins, plan, state, flags, validate, *, review=None, assemble=None):
    need(1 <= len(sources) <= MAX_ORIGINS and len({path.resolve() for path in sources}) == len(sources),
         'REUSE_SOURCE_LIMIT')
    path = run / LEDGER
    need(job.get('authorMode') == MODE and plan['mode'] == MODE and state['status'] == 'PREPARED'
         and not state['attempts'] and state['authorStartedAt'] is None and state.get('pageReuseDigest') is None
         and not path.exists() and not path.is_symlink(),
         'REUSE_REQUIRES_FRESH_ISOLATED_RUN')
    pages, origins, errors, rejected, threads = {}, [], [], Counter(), {}
    if review is not None:
        need(len(sources) == 1, 'PAGE_REVIEW_REQUIRES_ONE_DONOR')
        review_api['findings'](review)
    for source in sources:
        need(source.resolve() != run.resolve(), 'REUSE_SOURCE_IS_TARGET')
        try:
            with read_only_run(source) as historical:
                if review is None:
                    local, origin = candidates(historical, run, plan, job, inventory, pins, flags, validate, pages, rejected)
                else:
                    local, origin = review_candidates(historical, run, plan, job, inventory, pins, flags, validate, assemble)
        except (Stop, OSError, ValueError, TypeError, KeyError, IndexError, AttributeError, RecursionError) as error:
            if review is not None:
                raise Stop(getattr(error, 'code', 'PAGE_REVIEW_DONOR_INVALID'), source=str(source)) from None
            errors.append({'source': str(source), 'code': getattr(error, 'code', 'REUSE_SOURCE_INVALID')})
            continue
        origins.append(origin)
        for key, proof in local.items():
            if threads.setdefault(proof['threadId'], key) != key:
                rejected['REUSE_THREAD_SCOPE_CHANGED'] += 1
                continue
            pages[key] = dict(proof, origin=len(origins)-1)
    need(pages, 'NO_ELIGIBLE_REUSED_PAGES', sources=errors, rejected=dict(rejected))
    receipt = {'schema': SCHEMA, 'targetPinsDigest': digest(pins), 'pages': pages, 'origins': origins,
               'sourceErrors': errors, 'rejected': dict(rejected)}
    review_count = 0
    if review is not None:
        receipt['review'] = review
        values = {key: validate(slot, decode(base64.b64decode(pages[key]['raw'])))
                  for slot in plan['slots'] if (key := slot['key']) in pages}
        rows, _ = review_work(run, plan, values, receipt)
        review_count = len(rows)
    pending = run / PENDING
    save(pending, receipt)
    state['pageReuseDigest'] = sha(pending.read_bytes())
    save(run / 'state.json', state)
    os.replace(pending, path)
    reused = len(pages) - review_count
    return {'status': 'PREPARED', 'run': str(run), 'reusedPages': reused, 'remainingPages': len(plan['slots']) - reused,
            'historicalRuns': origins, 'sourceErrors': errors, 'rejected': dict(rejected),
            **({'reviewSlots': list(rows)} if review is not None else {})}


def read_pages(run, state, plan, validate):
    path, pending = run / LEDGER, run / PENDING
    expected = state.get('pageReuseDigest')
    if expected is None:
        need(not path.exists() and not path.is_symlink() and not pending.exists() and not pending.is_symlink(),
             'REUSE_IMPORT_INCOMPLETE')
        return {}, None
    if not path.exists() and not path.is_symlink():
        path = pending
    ledger = load(path)
    need(sha(path.read_bytes()) == expected and ledger['schema'] == SCHEMA
         and ledger['targetPinsDigest'] == digest(load(run / 'pins.json')), 'REUSE_LEDGER_CHANGED')
    slots = {slot['key']: slot for slot in plan['slots']}
    values = {}
    for key, proof in ledger['pages'].items():
        need(key in slots and type(proof['origin']) is int and 0 <= proof['origin'] < len(ledger['origins']),
             'REUSE_LEDGER_CHANGED')
        slot = slots[key]
        pinned = slot_files(run, slot)
        raw = base64.b64decode(proof['raw'], validate=True)
        need(len(raw) <= RESPONSE_LIMIT and proof['responseDigest'] == sha(raw) and proof['scopeDigest'] == digest(slot_scope(slot))
             and proof['promptDigest'] == sha(pinned['prompt']) and proof['schemaDigest'] == sha(pinned['schema']),
             'REUSE_LEDGER_CHANGED')
        values[key] = validate(slot, decode(raw))
    return values, ledger
