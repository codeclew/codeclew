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

LEDGER = 'page-reuse.json'
PENDING = '.page-reuse-pending.json'
SCHEMA = 'codeclew-docs-page-reuse/1.0'
MODE = 'ISOLATED_PAGES'
MAX_ORIGINS = 4
LOG_LIMIT = 2 * 1024 * 1024
RESPONSE_LIMIT = 128 * 1024
DIGEST = re.compile(r'sha256:[0-9a-f]{64}')
SLOT_KEY = re.compile(r'[a-z][a-z0-9_-]{0,127}')
SLOT_FIELDS = ('key', 'kind', 'page', 'service', 'requirement', 'citations', 'citationSpans', 'sourceRegions', 'returns')
SCOPE_FIELDS = ('revision', 'contextDigest', 'inputDigest', 'coverage', 'sourceInventory')


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


def run_scope(job, inventory):
    return {
        'snapshot': job['snapshot'], 'sourceSnapshot': inventory['snapshot'], 'services': job['services'],
        'mode': job.get('authorMode'), 'evidenceMode': job.get('evidenceMode'),
        'language': job['language'], 'request': job['request'],
        'sourceScopes': {service: {key: inventory['services'][service].get(key) for key in SCOPE_FIELDS}
                         for service in job['services']},
        'author': {key: job['author'].get(key) for key in ('model', 'reasoningEffort', 'stateRoot')},
    }


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
    launcher = Path(job['clew']).resolve(strict=True)
    package = launcher.parents[1]
    retained = load(exact_file(package, 'release.json', LOG_LIMIT))
    kit = package / 'source/skills/codeclew'
    need(caps['runtimeMode'] == 'RELEASE' and DIGEST.fullmatch(caps['runtimeKey'])
         and DIGEST.fullmatch(caps['runtimeManifestDigest'])
         and retained['runtimeKey'] == caps['runtimeKey'] and retained['runtimeManifestDigest'] == caps['runtimeManifestDigest']
         and not (package / 'source').is_symlink() and not kit.parent.is_symlink()
         and tree(kit, skill=True) == pins['skillFiles']
         and skill_digest(kit, pins['skillFiles']) == caps['agentContract']['skillDigest'], 'REUSE_RELEASE_KIT_CHANGED')
    state_file = exact_file(run, 'state.json', 32 * 1024 * 1024)
    seal = sha(state_file.read_bytes())
    state = load(state_file)
    need(state.get('schema') == 'codeclew-docs-run-state/1.0' and isinstance(state.get('attempts'), list)
         and len(state['attempts']) <= 512, 'REUSE_HISTORY_INVALID')
    old_plan = load(exact_file(run, 'inputs/author-plan.json', 32 * 1024 * 1024))
    need(old_plan['mode'] == MODE, 'REUSE_RUN_SCOPE_MISMATCH')
    rows = defaultdict(list)
    owners = defaultdict(set)
    for attempt in state['attempts']:
        if attempt.get('notLaunched'):
            continue
        rows[attempt['slot']].append(attempt)
        if thread := attempt.get('threadId'):
            owners[thread].add(attempt['slot'])
    historical = token_accounting([row for row in state['attempts'] if not row.get('notLaunched')])
    receipt = {
        'run': str(run), 'pinsDigest': digest(pins), 'stateDigest': seal, 'runtimeKey': caps['runtimeKey'],
        'skillDigest': caps['agentContract']['skillDigest'], 'author': run_scope(job, inventory)['author'],
        'recordedAttempts': len(state['attempts']),
        'reportedUsage': {key: historical[key] for key in ('status', 'counterScope', 'budgetTokens',
                                                          'knownBudgetTokensLowerBound', 'monetaryCost')},
    }
    return job, {slot['key']: slot for slot in old_plan['slots']}, rows, owners, receipt


def expected_argv(run, slot, number, backend, resume, flags):
    folder = run / 'page-attempts' / slot['key'] / str(number)
    return [backend['executable'], 'exec', '--ignore-user-config', '--skip-git-repo-check', '--json',
            '-m', backend['model'], '-c', 'model_reasoning_effort=' + json.dumps(backend['reasoningEffort']),
            '-c', 'approval_policy="never"', '-C', str(folder / 'context'), '--disable', 'multi_agent',
            '-o', str(folder / 'last-message.txt'), '-s', 'read-only', *flags(run / 'inputs' / slot['schema']),
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


def checked_attempt(run, slot, original, attempt, previous, backend, flags):
    key, number = slot['key'], attempt['number']
    relative = f'page-attempts/{key}/{number}'
    thread = attempt.get('threadId')
    resume = previous['threadId'] if previous is not None else None
    need(attempt['authorMode'] == MODE and attempt['status'] == 'COMPLETED' and type(attempt['exitCode']) is int
         and attempt['exitCode'] == 0 and isinstance(thread, str) and 0 < len(thread) <= 192
         and (resume is None or thread == resume) and attempt['resumeThread'] == resume
         and attempt['completedAt'] >= attempt['startedAt']
         and attempt['argv'] == expected_argv(run, slot, number, backend, resume, flags), 'REUSE_ATTEMPT_NOT_ADMITTED')
    result = exact_file(run, relative + '/result.json', LOG_LIMIT)
    context = result.parent / 'context'
    need(context.is_dir() and not context.is_symlink() and next(context.iterdir(), None) is None,
         'REUSE_AUTHOR_CONTEXT_CHANGED')
    need(load(result) == attempt, 'REUSE_ATTEMPT_RECEIPT_CHANGED')
    prompt = exact_file(run, relative + '/prompt.txt', LOG_LIMIT).read_bytes()
    expected = expected_prompt(slot, original, previous)
    # Repair details were persisted with sorted keys. Reconstruct the same
    # structured correction without relying on its historic insertion order.
    if previous is not None and expected.startswith(b'{'):
        need(decode(prompt) == decode(expected), 'REUSE_ATTEMPT_PROMPT_CHANGED')
    else:
        need(prompt == expected, 'REUSE_ATTEMPT_PROMPT_CHANGED')
    raw = exact_file(run, relative + '/last-message.txt', RESPONSE_LIMIT).read_bytes()
    need(sha(raw) == attempt['responseDigest'], 'REUSE_RESPONSE_CHANGED')
    stdout = exact_file(run, relative + '/stdout.jsonl', LOG_LIMIT).read_bytes()
    events = [decode(line) for line in stdout.splitlines() if line.strip()]
    need(len(events) >= 4 and events[0].get('type') == 'thread.started'
         and events[0].get('thread_id') == thread and events[1].get('type') == 'turn.started'
         and events[-1].get('type') == 'turn.completed' and events[-1].get('usage') == attempt['usage'],
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
    return raw, {
        'attempt': number, 'threadId': thread, 'receiptDigest': sha(result.read_bytes()),
        'streamDigest': sha(stdout), 'attemptPromptDigest': sha(prompt), 'responseDigest': sha(raw),
        'historicalCumulativeUsage': attempt['usage'],
    }


def candidates(run, target, plan, job, inventory, pins, flags, validate, reserved, rejected):
    source_job, old, histories, owners, receipt = history(run, job, inventory, pins)
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
        previous = None
        for number, attempt in enumerate(histories[key]):
            try:
                need(type(attempt['number']) is int and attempt['number'] == number
                     and owners[attempt['threadId']] == {key}, 'REUSE_THREAD_SCOPE_CHANGED')
                raw, proof = checked_attempt(run, old[key], pinned['prompt'], attempt, previous, source_job['author'], flags)
                need(token_accounting(histories[key][:number + 1])['status'] == 'MEASURED',
                     'REUSE_HISTORICAL_COUNTERS_CHANGED')
                validated.append((raw, proof))
                previous = attempt
            except (Stop, OSError, ValueError, TypeError, KeyError, IndexError, AttributeError, RecursionError) as error:
                rejected[getattr(error, 'code', 'REUSE_ATTEMPT_INVALID')] += 1
                break
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


def import_pages(run, sources, job, inventory, pins, plan, state, flags, validate):
    need(1 <= len(sources) <= MAX_ORIGINS and len({path.resolve() for path in sources}) == len(sources),
         'REUSE_SOURCE_LIMIT')
    path = run / LEDGER
    need(job.get('authorMode') == MODE and plan['mode'] == MODE and state['status'] == 'PREPARED'
         and not state['attempts'] and state['authorStartedAt'] is None and state.get('pageReuseDigest') is None
         and not path.exists() and not path.is_symlink(),
         'REUSE_REQUIRES_FRESH_ISOLATED_RUN')
    pages, origins, errors, rejected, threads = {}, [], [], Counter(), {}
    for source in sources:
        need(source.resolve() != run.resolve(), 'REUSE_SOURCE_IS_TARGET')
        try:
            with read_only_run(source) as historical:
                local, origin = candidates(historical, run, plan, job, inventory, pins, flags, validate, pages, rejected)
        except (Stop, OSError, ValueError, TypeError, KeyError, IndexError, AttributeError, RecursionError) as error:
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
    pending = run / PENDING
    save(pending, receipt)
    state['pageReuseDigest'] = sha(pending.read_bytes())
    save(run / 'state.json', state)
    os.replace(pending, path)
    return {'status': 'PREPARED', 'run': str(run), 'reusedPages': len(pages), 'remainingPages': len(plan['slots'])-len(pages),
            'historicalRuns': origins, 'sourceErrors': errors, 'rejected': dict(rejected)}


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
