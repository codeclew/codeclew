"""Bounded independent Codex sessions for immutable documentation page packets."""
from __future__ import annotations

from collections import deque
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
import json
import os
import signal
import subprocess
import threading
import time

POOL_SIZE = 3
LOG_LIMIT = 2 * 1024 * 1024
MAX_RESPONSE_BYTES = 128 * 1024
SERVICE_FAILURE = 'PAGE_AUTHOR_SERVICE_UNAVAILABLE'
SERVICE_ERROR_MESSAGES = {'workspace routing discovery failed'}
SERVICE_RETRY_DELAY = 10
MAX_SERVICE_RETRIES = 3


def service_failure(attempt):
    failure = attempt.get('failure') or {}
    return failure.get('code') == SERVICE_FAILURE


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
    starts, usage, terminal = [], None, None
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
                usage, terminal = event.get('usage'), event
            if event.get('type') == 'turn.failed':
                terminal = event
    if terminal and terminal.get('type') == 'turn.failed':
        usage = terminal.get('usage')
        failure = terminal.get('error')
        return starts, usage, failure.get('message') if isinstance(failure, dict) else None
    return starts, usage, None


def attempt_dir(run, attempt):
    return run / 'page-attempts' / attempt['slot'] / str(attempt['number'])


def launch(run, deadline, cancelled, attempt):
    attempt = attempt.copy()
    folder = attempt_dir(run, attempt)
    out, err = folder / 'stdout.jsonl', folder / 'stderr.log'
    expected = attempt['resumeThread']
    try:
        with (folder / 'prompt.txt').open('rb') as prompt, out.open('wb') as stdout, err.open('wb') as stderr:
            if cancelled.is_set() or time.monotonic() >= deadline:
                attempt.update(status='FAILED', notLaunched=True, cancelledBeforeLaunch=True,
                               failure={'code': 'PAGE_AUTHOR_INTERRUPTED', 'details': {}},
                               completedAt=time.time())
                save(folder / 'result.json', attempt)
                return attempt
            process = subprocess.Popen(attempt['argv'], cwd=folder / 'context', stdin=prompt,
                                       stdout=stdout, stderr=stderr, start_new_session=True)
            attempt['launchedAt'] = time.time()
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
        starts, usage, message = ([], None, None) if over_log_limit else author_events(out)
        valid = len(starts) == 1 and isinstance(starts[0], str) and bool(starts[0])
        if over_log_limit:
            cause = 'PAGE_AUTHOR_LOG_LIMIT'
        elif not valid:
            cause = cause or 'PAGE_AUTHOR_THREAD_REQUIRED'
        elif expected is not None and starts[0] != expected:
            cause = 'PAGE_AUTHOR_THREAD_CHANGED'
        elif process.returncode:
            transient = (process.returncode > 0 and isinstance(message, str)
                         and message.strip().lower() in SERVICE_ERROR_MESSAGES)
            cause = cause or (SERVICE_FAILURE if transient else 'PAGE_AUTHOR_PROCESS_FAILED')
        attempt.update(status='FAILED' if cause else 'COMPLETED', exitCode=process.returncode,
                       threadId=starts[0] if valid else None, usage=usage)
        if cause:
            attempt['failure'] = {'code': cause, 'details': {'backendMessage': message} if cause == SERVICE_FAILURE else {}}
    path = response_path(run, attempt)
    if attempt['status'] == 'COMPLETED' and path.is_file() and not path.is_symlink() and path.stat().st_size <= MAX_RESPONSE_BYTES:
        attempt['responseDigest'] = sha(path.read_bytes())
    attempt['completedAt'] = time.time()
    save(folder / 'result.json', attempt)
    return attempt


def record_accounting(run, state):
    tokens = token_accounting([row for row in state['attempts'] if not row.get('notLaunched')])
    state.update(tokenAccounting=tokens, modelTokens=tokens['budgetTokens'], usageAvailable=tokens['status'] == 'MEASURED')
    save(run / 'state.json', state)


def record_result(state, index, result):
    state['attempts'][index] = result
    if not result.get('cancelledBeforeLaunch'):
        state.get('pageRepairs', {}).pop(result['slot'], None)


def reconcile(run, state):
    changed, failure = False, None
    for index, pending in enumerate(state['attempts']):
        if pending['status'] != 'RUNNING':
            continue
        receipt = attempt_dir(run, pending) / 'result.json'
        try:
            need(receipt.is_file(), 'PAGE_AUTHOR_ATTEMPT_INDETERMINATE', slot=pending['slot'], blocked=True)
            result = load(receipt)
            need(all(result.get(key) == pending.get(key) for key in ('slot', 'number', 'argv', 'resumeThread', 'promptDigest', 'serviceRetry'))
                 and result.get('status') in ('COMPLETED', 'FAILED'),
                 'PAGE_AUTHOR_ATTEMPT_CHANGED', slot=pending['slot'], blocked=True)
        except Stop as error:
            failure = failure or error
            continue
        record_result(state, index, result)
        changed = True
    if changed:
        record_accounting(run, state)
    if failure:
        raise failure


def launch_attempt(run, schema, backend, slot, previous, failure, flags):
    number = len(previous)
    resume = next((row['threadId'] for row in reversed(previous) if row.get('threadId')), None)
    attempt = {'slot': slot['key'], 'number': number, 'authorMode': 'ISOLATED_PAGES', 'resumeThread': resume,
               'status': 'RUNNING', 'startedAt': time.time(), 'exitCode': None}
    folder = attempt_dir(run, attempt)
    original_prompt = (run / 'inputs' / slot['prompt']).read_bytes()
    prompt = original_prompt
    if failure and failure['code'] == SERVICE_FAILURE:
        interrupted = next(row for row in reversed(previous) if not row.get('cancelledBeforeLaunch'))
        old_prompt = attempt_dir(run, interrupted) / 'prompt.txt'
        need(resume == interrupted['threadId'] and old_prompt.is_file() and not old_prompt.is_symlink(),
             'PAGE_AUTHOR_SERVICE_RETRY_EVIDENCE_CHANGED', slot=slot['key'])
        prompt = old_prompt.read_bytes()
        need(sha(prompt) == interrupted.get('promptDigest'),
             'PAGE_AUTHOR_SERVICE_RETRY_EVIDENCE_CHANGED', slot=slot['key'])
        attempt['serviceRetry'] = True
    elif failure:
        details = failure.get('details', {})
        original_attempt = attempt_dir(run, previous[0]) / 'prompt.txt' if previous else None
        if (resume is not None and failure['code'] == 'PAGE_RESPONSE_INVALID' and details.get('formatOnly') is True
                and original_attempt.is_file() and original_attempt.read_bytes() == original_prompt):
            correction = {'slot': slot['key'], 'type': slot['kind'], 'formatRepair': details,
                          'instruction': 'Use the retained packet already in this session. Correct the named JSON fields and '
                                         'return the complete slot response in the supplied schema. Keep the claims, '
                                         'ordering and conditions. Cite only aliases that actually support the corresponding text.'}
            prompt = (json.dumps(correction, ensure_ascii=False, separators=(',', ':')) + '\n').encode()
        else:
            details = encode({'instruction': 'Repair only this slot using its retained code. Check the stated issue and preserve supported behavior.',
                              'failure': failure})
            prompt = b'Repair for the packet below:\n' + details + b'\n' + original_prompt
    (folder / 'context').mkdir(mode=0o700, parents=True)
    (folder / 'prompt.txt').write_bytes(prompt)
    attempt['promptDigest'] = sha(prompt)
    if 'schema' in slot:
        schema = run / 'inputs' / slot['schema']
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


def check_author_state(state, *, settled_only=False):
    attributable = [row for row in state['attempts'] if not row.get('notLaunched')
                    and (not settled_only or row['status'] != 'RUNNING')]
    tokens = token_accounting(attributable)
    need(tokens['status'] != 'INVALID', 'AUTHOR_USAGE_INVALID', accounting=tokens)
    unknown = [attributable[row['attempt']] for row in tokens['missing']]
    if tokens['status'] == 'UNAVAILABLE':
        failures = [{'slot': row['slot'], 'number': row['number'], 'failure': row.get('failure')}
                    for row in unknown if row.get('failure')]
        need(unknown and all(service_failure(attempt) and attempt.get('threadId') for attempt in unknown),
             'AUTHOR_USAGE_UNAVAILABLE', accounting=tokens, blocked=True,
             **({'attemptFailures': failures} if failures else {}))
    owners = {}
    for attempt in attributable:
        thread = attempt.get('threadId')
        failure = attempt.get('failure') or {}
        need(failure.get('code') != 'PAGE_AUTHOR_THREAD_CHANGED', 'PAGE_AUTHOR_THREAD_CHANGED', slot=attempt['slot'])
        need(thread, 'PAGE_AUTHOR_THREAD_REQUIRED', slot=attempt['slot'])
        need(owners.setdefault(thread, attempt['slot']) == attempt['slot'],
             'PAGE_AUTHOR_THREAD_REUSED_ACROSS_SLOTS', slot=attempt['slot'])
    return tokens


def next_action(run, slot, history, scheduled, max_repairs, responses, validate, read_response):
    launched = [row for row in history if not row.get('cancelledBeforeLaunch')]
    previous = launched[-1] if launched else None
    key = slot['key']
    failure = scheduled.get(key)
    if previous and previous['status'] == 'COMPLETED' and not failure:
        path = response_path(run, previous)
        try:
            need(previous.get('responseDigest'), 'PAGE_AUTHOR_RESPONSE_MISSING_OR_OVERSIZE')
            value = read_response(path)
            need(sha(path.read_bytes()) == previous['responseDigest'], 'PAGE_AUTHOR_RESPONSE_CHANGED')
            responses[key] = validate(slot, value)
            return None
        except (Stop, OSError, ValueError, TypeError, KeyError, IndexError, RecursionError) as error:
            failure = {'code': getattr(error, 'code', 'INVALID_PAGE_AUTHOR_RESPONSE'),
                       'details': getattr(error, 'details', {'failure': type(error).__name__})}
            previous['responseFailure'] = failure
            save(attempt_dir(run, previous) / 'result.json', previous)
    elif previous and not failure:
        failure = previous.get('failure')
    if previous and service_failure(previous):
        need(not previous.get('serviceRetry'), 'PAGE_AUTHOR_SERVICE_RETRIES_EXHAUSTED',
             slot=key, failure=previous['failure'])
    content_attempts = sum(not service_failure(attempt) for attempt in launched)
    need(content_attempts <= max_repairs, 'PAGE_AUTHOR_REPAIRS_EXHAUSTED', slot=key, page=slot['page'], failure=failure)
    return slot, failure


def execute_authors(run, job, plan, schema, flags, validate, read_response, repairs=None):
    """Validate each settled slot and refill up to three independent author sessions."""
    state = load(run / 'state.json')
    reconcile(run, state)
    if failure := state.get('authorTerminal'):
        raise Stop(failure['code'], blocked=failure['blocked'], **failure['details'])
    if state.get('status') == 'AUTHOR_REPAIRS_EXHAUSTED':
        raise Stop('PAGE_AUTHOR_REPAIRS_EXHAUSTED', **state['terminalFailure'])
    check_author_state(state)
    fresh = state['authorStartedAt'] is None and not state['attempts']
    now = time.time()
    paused = state.get('serviceRetryPending')
    if paused:
        need(now >= paused['retryNotBefore'], SERVICE_FAILURE, blocked=True, **paused)
        state['authorPausedSeconds'] = state.get('authorPausedSeconds', 0) + max(0, now - paused['pausedAt'])
        del state['serviceRetryPending']
    if state['authorStartedAt'] is None:
        state['authorStartedAt'] = now
    elapsed = now - state['authorStartedAt'] - state.get('authorPausedSeconds', 0)
    remaining = job['budget']['wallSeconds'] - elapsed
    deadline = time.monotonic() + remaining
    scheduled = state.setdefault('pageRepairs', {})
    scheduled.update(repairs or {})
    histories = {slot['key']: [] for slot in plan['slots']}
    for attempt in state['attempts']:
        histories[attempt['slot']].append(attempt)
    responses, pending, active = {}, deque(), {}
    deferred, pause = {}, None
    service_retries = sum(row.get('serviceRetry', False) and not row.get('cancelledBeforeLaunch')
                          for row in state['attempts'])
    cancelled = threading.Event()

    def assess(slot):
        action = next_action(run, slot, histories[slot['key']], scheduled, job['budget']['maxRepairs'],
                             responses, validate, read_response)
        if action:
            pending.append(action)

    def settle(future):
        index, slot = active[future]
        result = future.result()
        record_result(state, index, result)
        histories[slot['key']][-1] = result
        del active[future]
        return slot

    with ThreadPoolExecutor(max_workers=POOL_SIZE) as executor:
        try:
            for slot in plan['slots']:
                assess(slot)
            if fresh and not scheduled:
                # Prompt bytes approximate author time. Stable ties keep the plan's order.
                pending = deque(sorted(pending, key=lambda action: (run / 'inputs' / action[0]['prompt']).stat().st_size,
                                       reverse=True))
            elif paused:
                pending = deque(sorted(pending, key=lambda action: (action[1] or {}).get('code') != SERVICE_FAILURE))
            while (pending and not deferred) or active:
                while (pending and not deferred and len(active) < POOL_SIZE
                       and not any(future.done() for future in active)):
                    need(time.monotonic() < deadline, 'AUTHOR_BUDGET_EXHAUSTED')
                    slot, failure = pending.popleft()
                    history = histories[slot['key']]
                    if failure and failure['code'] == SERVICE_FAILURE:
                        need(service_retries < MAX_SERVICE_RETRIES, 'PAGE_AUTHOR_SERVICE_RETRIES_EXHAUSTED',
                             slot=slot['key'], failure=failure)
                        service_retries += 1
                    attempt = launch_attempt(run, schema, job['author'], slot, history, failure, flags)
                    history.append(attempt)
                    index = len(state['attempts'])
                    state['attempts'].append(attempt)
                    state['status'] = 'AUTHOR_RUNNING'
                    record_accounting(run, state)
                    active[executor.submit(launch, run, deadline, cancelled, attempt)] = index, slot
                done, _ = wait(active, timeout=max(0, deadline - time.monotonic()), return_when=FIRST_COMPLETED)
                need(done, 'AUTHOR_BUDGET_EXHAUSTED')
                settled = [settle(future) for future in sorted(done, key=lambda future: active[future][0])]
                record_accounting(run, state)
                if time.monotonic() >= deadline:
                    need(not active and not pending and all(
                        histories[slot['key']][-1].get('failure', {}).get('code') != 'PAGE_AUTHOR_INTERRUPTED'
                        for slot in settled), 'AUTHOR_BUDGET_EXHAUSTED')
                # Check the settled subset before sending any more work. Active
                # siblings have not reported their final counters yet.
                check_author_state(state, settled_only=True)
                for slot in settled:
                    result = histories[slot['key']][-1]
                    if service_failure(result):
                        need(not result.get('serviceRetry') and service_retries < MAX_SERVICE_RETRIES,
                             'PAGE_AUTHOR_SERVICE_RETRIES_EXHAUSTED', slot=slot['key'], failure=result['failure'])
                        deferred[slot['key']] = result['failure']
                    else:
                        assess(slot)
            if deferred:
                remaining = deadline - time.monotonic()
                need(remaining > 0, 'AUTHOR_BUDGET_EXHAUSTED')
                paused_at = time.time()
                pause = {'slots': deferred, 'pausedAt': paused_at, 'remainingAuthorSeconds': remaining,
                         'retryNotBefore': paused_at + SERVICE_RETRY_DELAY * (1 + service_retries)}
        except Stop as error:
            cancelled.set()
            state['authorTerminal'] = {'code': error.code, 'blocked': error.blocked, 'details': error.details}
            state['status'] = error.code if error.code.startswith('AUTHOR_') else 'AUTHOR_FAILED'
            if error.code == 'PAGE_AUTHOR_REPAIRS_EXHAUSTED':
                state.update(status='AUTHOR_REPAIRS_EXHAUSTED', terminalFailure=error.details)
            save(run / 'state.json', state)
            raise
        except KeyboardInterrupt:
            state['status'] = 'INTERRUPTED'
            save(run / 'state.json', state)
            raise
        finally:
            if active:
                cancelled.set()
                wait(active)
                for future in list(active):
                    if future.exception() is None:
                        settle(future)
                # Preserve completed counters even if an unrelated child was interrupted
                # or a worker failed before it could persist its own result.
                record_accounting(run, state)
    if pause:
        state.update(status='AUTHOR_SERVICE_RETRY_PENDING', serviceRetryPending=pause)
        record_accounting(run, state)
        raise Stop(SERVICE_FAILURE, blocked=True, **pause)
    return state, {key: responses[key] for key in histories if key in responses}
