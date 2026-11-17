"""Explicit page context using existing retained-source reads and attempt history."""
from pathlib import Path
from types import SimpleNamespace


def notes_digest(run, api):
    path = run / 'workspace/notes'
    return api['digest'](api['tree'](path)) if path.exists() else api['digest']({})


def build(run, page, requested_views, api):
    job, inventory, pins = api['frozen_inputs'](run)
    original = api['load'](run / 'workspace/draft/original.json')
    selected = next((p for p in original['pages'] if p['page'] == page), None)
    api['need'](selected is not None, 'SHOW_PAGE_NOT_FOUND', actual=page)
    state = api['load'](run / 'state.json')
    previous_handoff = state.get('handoff') or {}
    views, required, evidence_regions = {}, set(), set()
    handles = {h for f in selected['fragments'] for h in f.get('evidence', [])}
    for service in inventory['services']:
        home = run / 'inputs/author-source' / service
        if not (home / 'views.json').is_file():continue
        metadata = api['load'](home / 'views.json')
        requirements = api['load'](home / 'requirements.json')
        for key, row in metadata.items():
            identity = service + '/' + key
            owners = [r for r, value in requirements.items() if row['view'] in value.get('views', [])]
            views[identity] = {'file': row['file'], 'codeBytes': row['codeBytes'], 'requirements': owners}
            if any(r.get('page') == page and row['view'] in r.get('views', []) for r in requirements.values()):
                required.add(identity)
        index_path = run / 'inputs/source-proof' / service / 'citation-index.json'
        if index_path.exists():
            index = api['load'](index_path)
            evidence_regions.update(service + '/' + index[h]['region'] for h in handles if h in index)
    api['need'](all(v in views for v in requested_views), 'HANDOFF_SOURCE_VIEW_NOT_FOUND', actual=requested_views,
                available=list(views))
    # Requirements, not the existing prose's citations, establish the default.
    chosen = set(requested_views) if requested_views else required | evidence_regions
    api['need'](chosen and required <= chosen, 'HANDOFF_REQUIRED_SOURCE_VIEWS', required=sorted(required),
                selected=sorted(chosen), available=list(views),
                recovery='Select complete views with --source-view SERVICE/VIEW and include all required views; use show --review-context --service SERVICE for the inventory.')
    sources = []
    for view in sorted(chosen):
        args = SimpleNamespace(review_context=True, page=[], fragment=None, service=view.split('/')[0], view=view,
            entries=False, entry=None, routes=False, route=None, offset=0, limit=0, input=None, include_notes=False)
        sources.append(api['H']['show'](run, args, api)['source'])
    notes = run / 'workspace/notes' / (page + '.json')
    note_index = [{'page': p.name.removesuffix('.json'), 'path': 'notes/' + p.name,
                   'bytes': p.stat().st_size} for p in sorted((run / 'workspace/notes').glob('*.json'))]
    review = api['W']['review_page'](run, page, None, api)
    reviewed_path = run / 'workspace/draft/reviewed-pages.json'
    reviewed = api['load'](reviewed_path) if reviewed_path.exists() else {}
    return {'schema': 'codeclew-author-handoff/1', 'request': job['request'],
        'instruction': 'Work only on the current page. Read its complete source material, write reader text/sequence, compare source to text and text to source, repair locally, record review-page decisions, then check the page and end the turn. Discover uncovered requirements and consequential helpers through the full scope index; existing citations are not proof of completeness. Keep branches, before/after effects, failures, schemas and completion boundaries. Do not read old conversation/tool logs or rewrite completed unrelated pages. The caller owns the full final gate.',
        'model': job['author']['model'], 'reasoningEffort': job['author']['reasoningEffort'],
        'snapshot': job['snapshot'], 'admissionDigest': api['digest'](pins),
        'draftDigest': api['digest'](api['tree'](run / 'workspace/draft')), 'notesDigest': notes_digest(run, api),
        'previousThreadId': state.get('threadId'),
        'accounting': {'status': api['token_accounting'](state['attempts'])['status'],
                       'historicalAttempts': len(state['attempts']), 'reportedModelTokens': state.get('modelTokens')},
        'scope': {'requirements': [{k: r[k] for k in ('id', 'kind', 'service', 'page', 'sourceIds', 'file', 'topics') if k in r}
                                   for r in inventory['requirements']],
                  'pages': [p['page'] for p in original['pages']],
                  'pendingPages': [p['page'] for p in original['pages']
                                   if reviewed.get(p['page']) != api['S']['review_stamp'](run, p, api)],
                  'sourceViews': views,
                  'registeredStarts': api['W']['registered_accounts'](run, api),
                  'bindingIndexes': {s: '../inputs/source-proof/' + s + '/citation-index.json' for s in inventory['services']}},
        'currentPage': selected, 'sources': sources,
        'ownNotes': api['load'](notes) if notes.exists() else None, 'noteIndex': note_index,
        'notesAuthority': 'Prior authored notes, not source proof; compare them with the admitted source.',
        'decisions': review,
        'localFailure': {k: v for k, v in previous_handoff.get('result', {}).get('checks', {}).get('local', {}).items()
                         if k in ('code', 'stage', 'details', 'locations', 'recovery')}
                        if previous_handoff.get('page') == page else {},
        'contract': str(api['SKILL'] / 'references/docs-run-author.md'),
        'commands': './docs show --review-context; ./docs put; ./docs sequence; ./docs review-page; ./docs check --page ' + page}


def prepare(run, page, requested_views, api, *, renew=False):
    api['frozen_inputs'](run)
    state = api['load'](run / 'state.json')
    api['need'](state['status'] != 'AUTHOR_RUNNING', 'AUTHOR_HANDOFF_BUSY')
    current = state.get('handoff')
    draft = api['digest'](api['tree'](run / 'workspace/draft'))
    notes = notes_digest(run, api)
    if not renew and current and current['page'] == page and current['requestedViews'] == sorted(set(requested_views)):
        packet = api['load'](run / 'workspace/author-context.json')
        expected = current.get('completedDraftDigest', packet['draftDigest'])
        expected_notes = current.get('completedNotesDigest', packet['notesDigest'])
        if draft == expected and notes == expected_notes:
            api['need'](api['digest'](packet) == current['digest'], 'AUTHOR_HANDOFF_CHANGED')
            return {'status': 'HANDOFF_PREPARED', 'context': current['digest'], 'page': page,
                    'consumed': current['status'] != 'PREPARED', 'modelCalls': 0, 'ready': False}
    if current and current['status'] == 'PREPARED':
        api['need'](len(state['attempts']) == current['attempt'], 'AUTHOR_HANDOFF_ATTEMPT_EXISTS',
                    recovery='Preserve and diagnose the recorded attempt; never retry it implicitly.')
    repair = bool(current and current['page'] == page and current.get('result', {}).get('status') == 'FAILED')
    packet = build(run, page, requested_views, api)
    api['save'](run / 'workspace/author-context.json', packet)
    state['handoff'] = {'digest': api['digest'](packet), 'page': page, 'requestedViews': sorted(set(requested_views)),
                        'status': 'PREPARED', 'attempt': len(state['attempts']), 'repair': repair}
    api['save'](run / 'state.json', state)
    return {'status': 'HANDOFF_PREPARED', 'context': state['handoff']['digest'], 'page': page,
            'requestBytes': len(api['encode'](packet)), 'modelCalls': 0, 'ready': False}


def transition(run, attempt, previous_thread, api):
    """Validate explicit fresh identity using the existing immutable attempt files."""
    if attempt.get('contextMode') != 'FRESH':
        api['need'](attempt.get('threadId') == previous_thread, 'AUTHOR_THREAD_CHANGED')
        return previous_thread
    packet = api['load'](run / 'attempts' / str(attempt['contextAttempt']) / 'context.json')
    prior = api['load'](run / 'state.json')['attempts'][:attempt['contextAttempt']]
    job = api['load'](run / 'inputs/job.json')
    api['need'](api['digest'](packet) == attempt['contextHandoff'] and packet['previousThreadId'] == previous_thread
                and packet['model'] == job['author']['model'] and packet['snapshot'] == job['snapshot']
                and attempt.get('threadId') and attempt['threadId'] not in {a.get('threadId') for a in prior}
                and 'resume' not in attempt['argv'],
                'AUTHOR_CONTEXT_LINEAGE_CHANGED')
    return attempt['threadId']


def execute(run, api, *, debug=False):
    api['inputs_unchanged'](run)
    state = api['load'](run / 'state.json');handoff = state.get('handoff')
    api['need'](handoff, 'AUTHOR_HANDOFF_REQUIRED',
                recovery='Prepare the next page without a model: docs_run.py handoff --run-dir RUN --page PAGE.md; then explicitly run it.')
    packet = api['load'](run / 'workspace/author-context.json')
    api['need'](api['digest'](packet) == handoff['digest'], 'AUTHOR_HANDOFF_CHANGED')
    def reported(result):
        return {**result, 'observability': {'requestBytes': len(api['encode'](packet)),
                 'sourceViews': len(packet['sources']), 'historyReplayed': False}} if debug else result
    if handoff['status'] == 'COMPLETED':
        api['need'](handoff['completedDraftDigest'] == api['digest'](api['tree'](run / 'workspace/draft'))
                    and handoff['completedNotesDigest'] == notes_digest(run, api), 'AUTHOR_HANDOFF_STALE',
                    recovery='Prepare a new handoff for the changed page; the old outcome is retained.')
        return reported(handoff['result'])
    index = handoff['attempt']
    if len(state['attempts']) == index:
        api['need'](packet['draftDigest'] == api['digest'](api['tree'](run / 'workspace/draft'))
                    and packet['notesDigest'] == notes_digest(run, api), 'AUTHOR_HANDOFF_STALE')
        api['author'](run, unit={'id': 'page:' + handoff['page'], 'pages': [handoff['page']]},
                      repair=handoff['repair'], context=packet)
    else:
        api['need'](len(state['attempts']) == index + 1 and state['attempts'][index].get('contextHandoff') == handoff['digest']
                    and state['attempts'][index].get('status') == 'COMPLETED' and state['status'] == 'AUTHOR_COMPLETED',
                    'AUTHOR_HANDOFF_ATTEMPT_EXISTS', recovery='The recorded attempt is not resumable; preserve its status, usage and errors.')
    state = api['load'](run / 'state.json')
    api['need'](state['attempts'][index]['draftFiles'] == api['tree'](run / 'workspace/draft'), 'AUTHOR_HANDOFF_STALE')
    checked = api['W']['check'](run, [handoff['page']], False, api, require_review=True)
    result = {'status': 'PAGE_COMPLETED' if checked['status'] == 'UNIT_CHECKED' else 'FAILED',
              'page': handoff['page'], 'context': handoff['digest'], 'checks': {'local': checked},
              'ready': False, 'semanticTruthAssessed': False, 'fullFinalGate': 'NOT_RUN'}
    state = api['load'](run / 'state.json')
    state['handoff'].update(status='COMPLETED', result=result,
        completedDraftDigest=api['digest'](api['tree'](run / 'workspace/draft')), completedNotesDigest=notes_digest(run, api))
    api['save'](run / 'state.json', state)
    return reported(result)
