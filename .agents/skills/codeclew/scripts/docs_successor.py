"""Caller-owned, prepare-only continuation of a terminal failed author run."""
import fcntl
import json
import re
from pathlib import Path
import shutil


def prepared_bindings(run, api):
    result = {}
    for folder in ('author-source', 'source-proof'):
        for path in sorted((run / 'inputs' / folder).rglob('citations.json')):
            _, _, _, entries = api['M']['C']['load'](path)
            for handle, entry in entries.items():
                api['need'](handle not in result or result[handle] == entry, 'SUCCESSOR_BINDING_CONFLICT')
                result[handle] = entry
    api['need'](result, 'SUCCESSOR_SOURCE_PROOF_REQUIRED')
    return result


def history(run, state, api):
    thread = state.get('threadId'); attempts = state.get('attempts', [])
    api['need'](thread and attempts, 'SUCCESSOR_HISTORY_REQUIRED')
    files = {}
    for number, attempt in enumerate(attempts):
        api['need'](attempt.get('status') == 'COMPLETED' and attempt.get('exitCode') == 0
                    and attempt.get('threadId') == thread, 'SUCCESSOR_HISTORY_UNVERIFIABLE', attempt=number)
        folder = run / 'attempts' / str(number)
        files[str(number)] = api['tree'](folder)
        starts, completed = [], []
        # Read only public terminal counters/thread boundaries, never reasoning.
        for line in (folder / 'stdout.jsonl').read_text().splitlines():
            try:event = json.loads(line)
            except ValueError:continue
            if event.get('type') == 'thread.started':starts.append(event.get('thread_id'))
            if event.get('type') == 'turn.completed':completed.append(event.get('usage'))
        api['need'](starts == [thread] and completed == [attempt.get('usage')],
                    'SUCCESSOR_HISTORY_UNVERIFIABLE', attempt=number)
    accounting = api['token_accounting'](attempts)
    api['need'](accounting['status'] == 'MEASURED' and accounting['budgetTokens'] == state.get('modelTokens'),
                'SUCCESSOR_ACCOUNTING_UNVERIFIABLE')
    return files, accounting


def prepare(job_path, previous, previous_skill, run, api):
    previous, run = Path(previous).absolute(), Path(run).absolute()
    api['need'](previous.is_dir() and not previous.is_symlink()
                and not run.is_relative_to(previous) and not previous.is_relative_to(run), 'INVALID_SUCCESSOR_LOCATION')
    # A read-only lock attempt rejects a still-running supported launcher without
    # creating or modifying any file in the predecessor (including its pins).
    lock_path = previous / '.runner.lock'
    api['need'](lock_path.is_file() and not lock_path.is_symlink(), 'SUCCESSOR_TERMINAL_REQUIRED')
    with lock_path.open('rb') as lock:
        try:fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:api['need'](False, 'SUCCESSOR_PREDECESSOR_BUSY')
        # A terminal freeze is a byte-identical relocation. Keep its originally
        # pinned workspace identity; never rewrite the frozen profile or pins.
        frozen_pins = api['load'](previous / 'pins.json')
        match = re.search(r'\{path=("(?:[^"\\]|\\.)*"),enabled=true\}\]$', frozen_pins.get('authorSkillsConfig', ''))
        api['need'](match is not None, 'SUCCESSOR_PREPARED_PROFILE_REQUIRED')
        profile = Path(json.loads(match[1]))
        api['need'](profile.is_absolute() and profile.parts[-5:] == ('workspace', '.agents', 'skills', 'codeclew', 'SKILL.md'),
                    'SUCCESSOR_PREPARED_PROFILE_REQUIRED')
        origin = profile.parents[4]
        old_job, inventory, old_pins = api['frozen_inputs'](previous, skill=Path(previous_skill), prepared_origin=origin)
        state, report = api['load'](previous / 'state.json'), api['load'](previous / 'report.json')
        api['need'](state.get('status') == 'AUTHOR_COMPLETED' and report.get('status') == 'FAILED',
                    'SUCCESSOR_TERMINAL_FAILED_REQUIRED')
        files, accounting = history(previous, state, api)
        before = {name: api['sha']((previous / name).read_bytes()) for name in ('state.json', 'pins.json', 'report.json')}
        draft = api['tree'](previous / 'workspace/draft')
        pages = api['tree'](previous / 'workspace/pages')
        api['need'](draft == state['attempts'][-1]['draftFiles'], 'SUCCESSOR_PARENT_DRAFT_CHANGED')
        job = api['validate_job'](api['load'](job_path))
        stable = lambda value: {k: v for k, v in value.items() if k not in ('renderer', 'budget')}
        api['need'](stable(job) == stable(old_job), 'SUCCESSOR_REQUEST_OR_SCOPE_CHANGED')
        old_bindings = prepared_bindings(previous, api)
        # Normal prepare admits the exact new helper, profile, renderer and runtime.
        api['prepare'](job_path, run)
        pending = api['load'](run / 'state.json');pending['status'] = 'SUCCESSOR_ADMISSION_PENDING'
        api['save'](run / 'state.json', pending)
        new_pins = api['load'](run / 'pins.json')
        api['need'](api['load'](run / 'inputs/inventory.json') == inventory
                    and api['tree'](run / 'inputs/author-source') == api['tree'](previous / 'inputs/author-source')
                    and prepared_bindings(run, api) == old_bindings, 'SUCCESSOR_SOURCE_CHANGED')
        for key in ('clewVersion', 'clewCapabilities', 'nodeExecutableDigest', 'authorExecutableDigest'):
            api['need'](new_pins[key] == old_pins[key], 'SUCCESSOR_RUNTIME_CHANGED', field=key)
        shutil.copytree(previous / 'workspace/draft', run / 'workspace/draft', dirs_exist_ok=True)
        shutil.copytree(previous / 'workspace/pages', run / 'workspace/pages', dirs_exist_ok=True)
        (run / 'attempts').mkdir()
        for number in files:shutil.copytree(previous / 'attempts' / number, run / 'attempts' / number)
        archive = run / 'history';archive.mkdir()
        if (previous / 'history').exists():shutil.copytree(previous / 'history', archive / 'earlier')
        for name in ('state.json', 'pins.json', 'report.json'):shutil.copyfile(previous / name, archive / name)
        api['need'](api['tree'](run / 'workspace/draft') == draft
                    and api['tree'](run / 'workspace/pages') == pages, 'SUCCESSOR_DRAFT_COPY_CHANGED')
        receipt = {'schema': 'codeclew-docs-successor/1', 'predecessor': str(previous), 'predecessorPreparedOrigin': str(origin),
                   'predecessorFiles': before, 'predecessorChecks': str(previous / 'checks'),
                   'historyFiles': files, 'archiveFiles': api['tree'](archive), 'inheritedAttempts': len(state['attempts']),
                   'inheritedAttemptsDigest': api['digest'](state['attempts']), 'threadId': state['threadId'],
                   'inheritedAccounting': accounting, 'originalLimits': old_job['budget'],
                   'originalAuthorStartedAt': state.get('authorStartedAt'), 'continuationAllowance': job['budget'],
                   'budgetScope': 'NEW_ATTEMPTS_ONLY', 'draftDigest': api['digest'](draft),
                   'sourceBindingsDigest': api['digest'](old_bindings), 'modelCallsOnPrepare': 0,
                   'standaloneFullDocsetReviewRequired': True, 'fullFinalGateRequired': True}
        api['save'](run / 'inputs/successor.json', receipt)
        new_pins['inputFiles'] = api['tree'](run / 'inputs');api['save'](run / 'pins.json', new_pins)
        inherited = {'inheritedAttemptCount': len(state['attempts']), 'inheritedModelTokens': accounting['budgetTokens'],
                     'receiptDigest': api['digest'](receipt)}
        api['save'](run / 'state.json', {**state, 'status': 'SUCCESSOR_PREPARED', 'authorStartedAt': None,
                   'successor': inherited, 'unitCursor': None})
        api['need'](all(api['sha']((previous / name).read_bytes()) == value for name, value in before.items())
                    and api['tree'](previous / 'workspace/draft') == draft, 'SUCCESSOR_PARENT_CHANGED')
        return {'status': 'SUCCESSOR_PREPARED', 'run': str(run), 'modelCalls': 0, 'ready': False,
                'inheritedModelTokens': accounting['budgetTokens'], 'continuationAllowance': job['budget'],
                'continuationModelTokens': 0, 'totalModelTokens': accounting['budgetTokens'],
                'budgetScope': 'NEW_ATTEMPTS_ONLY', 'receipt': 'inputs/successor.json'}


def validate(run, state, api):
    receipt = api['load'](run / 'inputs/successor.json'); info = state.get('successor', {})
    count = receipt['inheritedAttempts']
    api['need'](info == {'inheritedAttemptCount': count,
                'inheritedModelTokens': receipt['inheritedAccounting']['budgetTokens'],
                'receiptDigest': api['digest'](receipt)}
                and api['digest'](state['attempts'][:count]) == receipt['inheritedAttemptsDigest'],
                'SUCCESSOR_HISTORY_CHANGED')
    for number, expected in receipt['historyFiles'].items():
        api['need'](api['tree'](run / 'attempts' / number) == expected, 'SUCCESSOR_HISTORY_CHANGED', attempt=number)
    api['need'](api['tree'](run / 'history') == receipt['archiveFiles'], 'SUCCESSOR_HISTORY_CHANGED')
    api['need'](state.get('threadId') == receipt['threadId'], 'AUTHOR_THREAD_CHANGED')
    return count


def execute(run, api, debug=False):
    job, _, _ = api['inputs_unchanged'](run)
    state = api['load'](run / 'state.json');base = validate(run, state, api)
    if len(state['attempts']) == base:
        api['author'](run)  # Mandatory standalone same-author review, not unit replay.
    elif state['status'] != 'AUTHOR_COMPLETED':
        api['need'](False, 'SUCCESSOR_AUTHOR_NOT_COMPLETED')
    while True:
        result = api['verify'](run, debug=debug)
        if result['status'] == 'READY':return api['promote'](run)
        state = api['load'](run / 'state.json')
        repairs = sum(a.get('kind') == 'repair' for a in state['attempts'][base:])
        if result['status'] == 'BLOCKED' or result['checks']['inputs']['status'] != 'PASS' or repairs >= job['budget']['maxRepairs']:
            return result
        api['author'](run, repair=True)
