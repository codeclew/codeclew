"""Bounded independent Codex sessions for immutable documentation page packets."""
from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor, as_completed
import json
import os
import signal
import subprocess
import threading
import time

POOL_SIZE = 3
LOG_LIMIT = 2 * 1024 * 1024
MAX_RESPONSE_BYTES = 128 * 1024


def stop_process(process):
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()


def author_events(path):
    starts, usage = [], None
    with path.open('rb') as stream:
        for line in stream:
            try:
                event = json.loads(line)
            except (ValueError, UnicodeError, RecursionError):
                continue
            if not isinstance(event, dict):
                continue
            if event.get('type') == 'thread.started':
                starts.append(event.get('thread_id'))
            if event.get('type') == 'turn.completed':
                usage = event.get('usage')
    return starts, usage


def attempt_dir(run, attempt):
    return run / 'page-attempts' / attempt['slot'] / str(attempt['number'])


def launch(run, deadline, cancelled, attempt):
    folder = attempt_dir(run, attempt)
    out, err = folder / 'stdout.jsonl', folder / 'stderr.log'
    expected = attempt['resumeThread']
    try:
        with (folder / 'prompt.txt').open('rb') as prompt, out.open('wb') as stdout, err.open('wb') as stderr:
            process = subprocess.Popen(attempt['argv'], cwd=folder / 'context', stdin=prompt,
                                       stdout=stdout, stderr=stderr, start_new_session=True)
            cause = None
            try:
                while process.poll() is None:
                    if cancelled.is_set() or time.monotonic() >= deadline:
                        cause = 'PAGE_AUTHOR_INTERRUPTED'
                    elif out.stat().st_size > LOG_LIMIT or err.stat().st_size > LOG_LIMIT:
                        cause = 'PAGE_AUTHOR_LOG_LIMIT'
                    if cause:
                        stop_process(process)
                        break
                    try:
                        process.wait(timeout=0.2)
                    except subprocess.TimeoutExpired:
                        continue
            except BaseException:
                stop_process(process)
                raise
    except OSError as error:
        attempt.update(status='FAILED', notLaunched='process' not in locals(), failure={
            'code': 'PAGE_AUTHOR_PROCESS_FAILED', 'details': {'failure': type(error).__name__}})
    else:
        over_log_limit = out.stat().st_size > LOG_LIMIT or err.stat().st_size > LOG_LIMIT
        starts, usage = ([], None) if over_log_limit else author_events(out)
        valid = len(starts) == 1 and isinstance(starts[0], str) and bool(starts[0])
        if over_log_limit:
            cause = 'PAGE_AUTHOR_LOG_LIMIT'
        elif not valid:
            cause = cause or 'PAGE_AUTHOR_THREAD_REQUIRED'
        elif expected is not None and starts[0] != expected:
            cause = 'PAGE_AUTHOR_THREAD_CHANGED'
        elif process.returncode:
            cause = cause or 'PAGE_AUTHOR_PROCESS_FAILED'
        attempt.update(status='FAILED' if cause else 'COMPLETED', exitCode=process.returncode,
                       threadId=starts[0] if valid else None, usage=usage)
        if cause:
            attempt['failure'] = {'code': cause, 'details': {}}
    path = response_path(run, attempt)
    if attempt['status'] == 'COMPLETED' and path.is_file() and not path.is_symlink() and path.stat().st_size <= MAX_RESPONSE_BYTES:
        attempt['responseDigest'] = sha(path.read_bytes())
    attempt['completedAt'] = time.time()
    save(folder / 'result.json', attempt)
    return attempt


def accounting(state):
    return token_accounting([row for row in state['attempts'] if not row.get('notLaunched')])


def reconcile(run, state):
    for number, pending in enumerate(state['attempts']):
        if pending['status'] != 'RUNNING':
            continue
        receipt = attempt_dir(run, pending) / 'result.json'
        need(receipt.is_file(), 'PAGE_AUTHOR_ATTEMPT_INDETERMINATE', slot=pending['slot'], blocked=True)
        result = load(receipt)
        need(all(result.get(key) == pending.get(key) for key in ('slot', 'number', 'argv', 'resumeThread')),
             'PAGE_AUTHOR_ATTEMPT_CHANGED', slot=pending['slot'], blocked=True)
        state['attempts'][number] = result


def launch_attempt(run, schema, backend, slot, previous, failure, flags):
    number = len(previous)
    resume = next((row['threadId'] for row in reversed(previous) if row.get('threadId')), None)
    attempt = {'slot': slot['key'], 'number': number, 'authorMode': 'ISOLATED_PAGES', 'resumeThread': resume,
               'status': 'RUNNING', 'startedAt': time.time(), 'exitCode': None}
    folder = attempt_dir(run, attempt)
    (folder / 'context').mkdir(mode=0o700, parents=True)
    original_prompt = (run / 'inputs' / slot['prompt']).read_bytes()
    prompt = original_prompt
    if failure:
        # Keep the original packet before the focused correction and never pass
        # the whole draft or another page's author output to this session.
        details = encode({'instruction': 'Repair only this slot using its retained code. Check the stated issue and preserve supported behavior.',
                          'failure': failure})
        prompt = b'Repair for the packet below:\n' + details + b'\n' + original_prompt
    (folder / 'prompt.txt').write_bytes(prompt)
    argv = [backend['executable'], 'exec', '--ignore-user-config', '--skip-git-repo-check', '--json',
            '-m', backend['model'], '-c', 'model_reasoning_effort=' + json.dumps(backend['reasoningEffort']),
            '-c', 'approval_policy="never"', '-C', str(folder / 'context'), '--disable', 'multi_agent',
            '-o', str(folder / 'last-message.txt'), '-s', 'read-only', *flags(schema)]
    if resume is not None:
        argv += ['resume', resume]
    attempt['argv'] = argv + ['-']
    return attempt


def response_path(run, attempt):
    return attempt_dir(run, attempt) / 'last-message.txt'


def check_author_state(state):
    tokens = accounting(state)
    need(tokens['status'] != 'INVALID', 'AUTHOR_USAGE_INVALID', accounting=tokens)
    need(tokens['status'] == 'MEASURED', 'AUTHOR_USAGE_UNAVAILABLE', accounting=tokens, blocked=True)
    owners = {}
    for attempt in state['attempts']:
        if attempt.get('notLaunched'):
            continue
        thread = attempt.get('threadId')
        failure = attempt.get('failure') or {}
        need(failure.get('code') != 'PAGE_AUTHOR_THREAD_CHANGED', 'PAGE_AUTHOR_THREAD_CHANGED', slot=attempt['slot'])
        need(thread, 'PAGE_AUTHOR_THREAD_REQUIRED', slot=attempt['slot'])
        need(owners.setdefault(thread, attempt['slot']) == attempt['slot'],
             'PAGE_AUTHOR_THREAD_REUSED_ACROSS_SLOTS', slot=attempt['slot'])
    return tokens


def execute_authors(run, job, plan, schema, flags, validate, read_response, repairs=None):
    """Retain successful slots and retry only the owning failed slot."""
    state = load(run / 'state.json')
    reconcile(run, state)
    check_author_state(state)
    if state['authorStartedAt'] is None:
        state['authorStartedAt'] = time.time()
    if repairs:
        state.setdefault('pageRepairs', {}).update(repairs)
    max_repairs = job['budget']['maxRepairs']
    cancelled = threading.Event()
    while True:
        responses, needed = {}, []
        scheduled = state.get('pageRepairs', {})
        for slot in plan['slots']:
            key = slot['key']
            history = [row for row in state['attempts'] if row['slot'] == key]
            previous = history[-1] if history else None
            failure = scheduled.get(key)
            if previous and previous['status'] == 'COMPLETED' and not failure:
                path = response_path(run, previous)
                try:
                    need(previous.get('responseDigest'), 'PAGE_AUTHOR_RESPONSE_MISSING_OR_OVERSIZE')
                    value = read_response(path)
                    need(sha(path.read_bytes()) == previous['responseDigest'], 'PAGE_AUTHOR_RESPONSE_CHANGED')
                    responses[key] = validate(slot, value)
                    continue
                except (Stop, OSError, ValueError, TypeError, KeyError, IndexError, RecursionError) as error:
                    failure = {'code': getattr(error, 'code', 'INVALID_PAGE_AUTHOR_RESPONSE'),
                               'details': getattr(error, 'details', {'failure': type(error).__name__})}
                    previous['responseFailure'] = failure
                    save(attempt_dir(run, previous) / 'result.json', previous)
            elif previous and not failure:
                failure = previous.get('failure')
            if len(history) > max_repairs:
                save(run / 'state.json', state)
                raise Stop('PAGE_AUTHOR_REPAIRS_EXHAUSTED', slot=key, page=slot['page'], failure=failure)
            needed.append((slot, history, failure))
        if not needed:
            return state, responses

        remaining = job['budget']['wallSeconds'] - (time.time() - state['authorStartedAt'])
        need(remaining > 0, 'AUTHOR_BUDGET_EXHAUSTED')
        deadline = time.monotonic() + remaining
        batch = []
        # Reserve only the active batch, so an interrupted run never counts work
        # that was still queued behind workers and never submitted to Codex.
        for slot, history, failure in needed[:POOL_SIZE]:
            attempt = launch_attempt(run, schema, job['author'], slot, history, failure, flags)
            scheduled.pop(slot['key'], None)
            state['attempts'].append(attempt)
            batch.append((len(state['attempts']) - 1, attempt))
        state['status'] = 'AUTHOR_RUNNING'
        save(run / 'state.json', state)
        with ThreadPoolExecutor(max_workers=POOL_SIZE) as executor:
            futures = {executor.submit(launch, run, deadline, cancelled, row): i for i, row in batch}
            try:
                for future in as_completed(futures):
                    index = futures[future]
                    attempt = future.result()
                    state['attempts'][index] = attempt
                    save(run / 'state.json', state)
            except BaseException:
                cancelled.set()
                raise
        tokens = accounting(state)
        state.update(tokenAccounting=tokens, modelTokens=tokens['budgetTokens'], usageAvailable=tokens['status'] == 'MEASURED')
        save(run / 'state.json', state)
        check_author_state(state)
