"""Caller-owned, prepare-only continuation of terminal work, without approval transfer."""
import fcntl
import json
import re
from pathlib import Path
import shutil


def quality_rejected(run, api):
    path = run / 'inputs/successor.json'
    return path.exists() and api['load'](path).get('callerDecision', {}).get('reason') == 'INDEPENDENT_QUALITY_REJECTED'


def review_stamp(run, page, api):
    if not quality_rejected(run, api):return api['digest'](page)
    # Bind current decisions to this admitted continuation, not its relocatable
    # filesystem path. Restoring a parent's opaque hashes is not a new decision.
    return api['digest']({'page': page, 'reviewScope': {
        'successor': api['digest'](api['load'](run / 'inputs/successor.json')),
        'profile': api['load'](run / 'pins.json')['authorSkillsConfig']}})


def pending_review(run, api, pages=None):
    if not quality_rejected(run, api):return {'pages': [], 'claims': [], 'requirements': []}
    original = api['load'](run / 'workspace/draft/original.json')
    path = run / 'workspace/draft/reviewed-pages.json'
    reviewed = api['load'](path) if path.exists() else {}
    unreviewed = [p for p in original['pages'] if (pages is None or p['page'] in pages)
                  and reviewed.get(p['page']) != review_stamp(run, p, api)]
    names = [p['page'] for p in unreviewed]
    return {'pages': names, 'claims': [{'page': p['page'], 'claim': f['id']} for p in unreviewed
            for f in p['fragments'] if 'id' in f], 'requirements': [r['id'] for r in
            api['load'](run / 'inputs/inventory.json')['requirements'] if r['page'] in names]}


def require_current_reviews(run, api, pages=None):
    pending = pending_review(run, api, pages)
    api['need'](not pending['pages'], 'QUALITY_REJECTED_REVIEW_PENDING', **pending,
        recovery='Compare the listed reader fragments with source and record fresh decisions using ./docs review-page. Preserve valid reader text; finish-review after all pending pages.')


def pending_decisions(review):
    """Keep locations and source references as hints, never inherited approval."""
    for row in review['requirements'].values():
        row['disposition'] = None
        if 'discoveryDisposition' in row:row['discoveryDisposition'] = None
        for discovered in row.get('discoveredEntries') or []:discovered['disposition'] = None
        for topic in (row.get('topics') or {}).values():topic['disposition'] = None
    review['claims'] = {p: {c: None for c in claims} for p, claims in review['claims'].items()}
    review['unresolved'], review['visual'] = None, {'status': None}
    return review


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
    previous_thread = None
    for number, attempt in enumerate(attempts):
        api['need'](attempt.get('status') == 'COMPLETED' and attempt.get('exitCode') == 0
                    and attempt.get('threadId'), 'SUCCESSOR_HISTORY_UNVERIFIABLE', attempt=number)
        if previous_thread is not None or attempt.get('contextMode') == 'FRESH':
            previous_thread = api['DH']['transition'](run, attempt, previous_thread, api)
        else:previous_thread = attempt['threadId']
        folder = run / 'attempts' / str(number)
        files[str(number)] = api['tree'](folder)
        starts, completed = [], []
        # Read only public terminal counters/thread boundaries, never reasoning.
        for line in (folder / 'stdout.jsonl').read_text().splitlines():
            try:event = json.loads(line)
            except ValueError:continue
            if event.get('type') == 'thread.started':starts.append(event.get('thread_id'))
            if event.get('type') == 'turn.completed':completed.append(event.get('usage'))
        api['need'](starts == [attempt['threadId']] and completed == [attempt.get('usage')],
                    'SUCCESSOR_HISTORY_UNVERIFIABLE', attempt=number)
    api['need'](previous_thread == thread, 'AUTHOR_THREAD_CHANGED')
    accounting = api['token_accounting'](attempts)
    api['need'](accounting['status'] == 'MEASURED' and accounting['budgetTokens'] == state.get('modelTokens'),
                'SUCCESSOR_ACCOUNTING_UNVERIFIABLE')
    return files, accounting


def prepare(job_path, previous, previous_skill, run, api, *, revise_ready=False, revise_budget_stopped=False):
    api['need'](not (revise_ready and revise_budget_stopped), 'SUCCESSOR_REVISION_MODE_CONFLICT')
    rejected = revise_ready or revise_budget_stopped
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
        state = api['load'](previous / 'state.json')
        report_path = previous / 'report.json'
        if revise_budget_stopped:
            api['need'](state.get('status') == 'AUTHOR_BUDGET_EXCEEDED' and not report_path.exists()
                        and not report_path.is_symlink(), 'SUCCESSOR_BUDGET_STOP_REQUIRED')
            report = None
        else:
            report = api['load'](report_path)
            api['need'](state.get('status') == 'AUTHOR_COMPLETED' and
                        report.get('status') == ('READY' if revise_ready else 'FAILED'),
                        'SUCCESSOR_TERMINAL_FAILED_REQUIRED')
        ready_files = {}
        if revise_ready:
            api['need'](all(report.get('checks', {}).get(c, {}).get('status') == 'PASS' for c in api['CHECKS']),
                        'SUCCESSOR_READY_UNVERIFIABLE')
            output = Path(report.get('output', ''))
            api['need'](not output.is_absolute() and output.parts and '..' not in output.parts
                        and api['tree'](previous / output) == report.get('outputFiles')
                        and report.get('outputDigest') == api['digest'](report['outputFiles']),
                        'SUCCESSOR_READY_UNVERIFIABLE')
            ready = api['load'](previous / 'ready/current.json')
            key = ready.get('output')
            api['need'](isinstance(key, str) and re.fullmatch(r'[0-9a-f]{64}', key),
                        'SUCCESSOR_READY_UNVERIFIABLE')
            # promote preserves the initially accepted package, then records its
            # recheck report. Receipt paths can differ; reader bytes must not.
            api['need'](ready.get('outputDigest') == api['digest'](api['tree'](previous / 'ready' / key))
                        and ready['outputDigest'] == 'sha256:' + key
                        and ready.get('reportDigest') == api['digest'](report)
                        and api['tree'](previous / 'ready' / key / 'html') == report.get('deliverableFiles'),
                        'SUCCESSOR_READY_UNVERIFIABLE')
            ready_files = api['tree'](previous / 'ready')
        files, accounting = history(previous, state, api)
        terminal = None
        if revise_budget_stopped:
            api['need'](('successor' in state) == (previous / 'inputs/successor.json').exists(), 'SUCCESSOR_STATE_MISMATCH')
            base = validate(previous, state, api) if 'successor' in state else 0
            used = api['scoped_budget_tokens'](state, accounting)
            maximum = old_job['budget']['maxModelTokens']
            api['need'](len(state['attempts']) > base and type(maximum) is int and used > maximum,
                        'SUCCESSOR_GENUINE_BUDGET_STOP_REQUIRED', scopedModelTokens=used, originalLimit=maximum)
            terminal = {'status': state['status'], 'nativeReportPresent': False,
                        'budgetScope': 'NEW_ATTEMPTS_ONLY' if base else 'RUN',
                        'scopedModelTokens': used, 'originalLimit': maximum}
        before = {name: api['sha']((previous / name).read_bytes()) for name in
                  ('state.json', 'pins.json', *(['report.json'] if report is not None else []))}
        draft = api['tree'](previous / 'workspace/draft')
        pages = api['tree'](previous / 'workspace/pages')
        api['need'](draft == state['attempts'][-1]['draftFiles'], 'SUCCESSOR_PARENT_DRAFT_CHANGED')
        if revise_ready:
            api['need'](draft == report.get('draftFiles') and api['digest'](draft) == report.get('draftDigest'),
                        'SUCCESSOR_READY_UNVERIFIABLE')
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
        if (previous / 'workspace/notes').exists():
            shutil.copytree(previous / 'workspace/notes', run / 'workspace/notes')
        (run / 'attempts').mkdir()
        for number in files:shutil.copytree(previous / 'attempts' / number, run / 'attempts' / number)
        archive = run / 'history';archive.mkdir()
        if (previous / 'history').exists():shutil.copytree(previous / 'history', archive / 'earlier')
        for name in before:shutil.copyfile(previous / name, archive / name)
        if revise_ready:shutil.copytree(previous / 'ready', archive / 'ready')
        api['need'](api['tree'](run / 'workspace/draft') == draft
                    and api['tree'](run / 'workspace/pages') == pages, 'SUCCESSOR_DRAFT_COPY_CHANGED')
        if rejected:
            # Preserve exact prior decisions alongside other inherited history.
            for name in ('self-review.json', 'reviewed-pages.json'):
                path = run / 'workspace/draft' / name
                if path.exists():shutil.copyfile(path, archive / name)
        receipt = {'schema': 'codeclew-docs-successor/1', 'predecessor': str(previous), 'predecessorPreparedOrigin': str(origin),
                   'predecessorFiles': before, 'predecessorChecks': str(previous / 'checks'),
                   'historyFiles': files, 'archiveFiles': api['tree'](archive), 'inheritedAttempts': len(state['attempts']),
                   'inheritedAttemptsDigest': api['digest'](state['attempts']), 'threadId': state['threadId'],
                   'inheritedAccounting': accounting, 'originalLimits': old_job['budget'],
                   'originalAuthorStartedAt': state.get('authorStartedAt'), 'continuationAllowance': job['budget'],
                   'budgetScope': 'NEW_ATTEMPTS_ONLY', 'draftDigest': api['digest'](draft),
                   'sourceBindingsDigest': api['digest'](old_bindings), 'modelCallsOnPrepare': 0,
                   'standaloneFullDocsetReviewRequired': not rejected or not (run / 'inputs/scaffold.json').exists(),
                   'fullFinalGateRequired': True}
        if revise_ready:
            receipt.update(callerDecision={'status': 'REVISION_REQUESTED',
                'reason': 'INDEPENDENT_QUALITY_REJECTED', 'parentNativeStatus': 'READY'},
                predecessorReadyFiles=ready_files)
        if revise_budget_stopped:
            receipt.update(predecessorTerminal=terminal, callerDecision={'status': 'REVISION_REQUESTED',
                'reason': 'INDEPENDENT_QUALITY_REJECTED', 'parentNativeStatus': 'UNAVAILABLE'})
        api['save'](run / 'inputs/successor.json', receipt)
        new_pins['inputFiles'] = api['tree'](run / 'inputs');api['save'](run / 'pins.json', new_pins)
        if rejected:
            api['save'](run / 'workspace/draft/reviewed-pages.json', {})
            api['save'](run / 'workspace/draft/self-review.json', api['review_for_run'](run))
        inherited = {'inheritedAttemptCount': len(state['attempts']), 'inheritedModelTokens': accounting['budgetTokens'],
                     'receiptDigest': api['digest'](receipt)}
        api['save'](run / 'state.json', {**state, 'status': 'SUCCESSOR_PREPARED', 'authorStartedAt': None,
                   'successor': inherited, 'unitCursor': 0 if rejected else None,
                   'completedUnit': None, 'handoff': None})
        api['need'](all(api['sha']((previous / name).read_bytes()) == value for name, value in before.items())
                    and api['tree'](previous / 'workspace/draft') == draft
                    and (not revise_budget_stopped or not report_path.exists() and not report_path.is_symlink())
                    and (not revise_ready or api['tree'](previous / 'ready') == ready_files), 'SUCCESSOR_PARENT_CHANGED')
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
    expected = receipt['threadId']
    for attempt in state['attempts'][count:]:
        if attempt.get('threadId'):expected = api['DH']['transition'](run, attempt, expected, api)
    api['need'](state.get('threadId') == expected, 'AUTHOR_THREAD_CHANGED')
    return count


def prompt_facts(run, api):
    report = run / 'history/report.json'
    return {'predecessorMachineFailure': api['repair_facts'](api['load'](report)) if report.is_file() else None,
            'predecessorTerminal': api['load'](run / 'inputs/successor.json').get('predecessorTerminal')}


def execute(run, api, debug=False):
    return api['DH']['execute'](run, api, debug=debug)
