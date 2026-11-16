"""Structural service-topic closure, never a semantic completeness judgment."""


def validate(inventory, accounts, original, navigation, entries, api, *, pages=None,
             recorded_only=False, record=False):
    rows = accounts if isinstance(accounts, dict) else {a['requirement']: a for a in accounts}
    fragments = {(p['page'], f['id']): f for p in original['pages'] for f in p['fragments'] if 'id' in f}
    owners = {p: s['id'] for s in navigation['services'] for p in s['pages']}
    issues = []
    for requirement in inventory['requirements']:
        if requirement['kind'] != 'SERVICE':continue
        account = rows.get(requirement['id'])
        if recorded_only and (account is None or account.get('disposition') not in ('EXPLAINED', 'EXPLICIT_UNKNOWN')):continue
        topics = (account or {}).get('topics')
        referenced = [v.get('page') for v in topics.values() if isinstance(v, dict)] if isinstance(topics, dict) else []
        if pages is not None and requirement['page'] not in pages and not set(referenced) & set(pages):continue
        def issue(topic, code, expected, actual):
            issues.append({'code': code, 'requirement': requirement['id'], 'page': requirement['page'],
                'field': '/requirements/' + requirement['id'] + '/topics' + ('/' + topic if topic else ''),
                'expected': expected, 'actual': actual,
                'recovery': './docs review-page --page ' + requirement['page'] +
                    ' (then --input FILE with source-reviewed topic references or bounded absence explanations)'})
        notes = (account or {}).get('sourceNotes')
        handles = notes.get('evidence', []) if isinstance(notes, dict) else []
        if not isinstance(notes, dict) or not isinstance(notes.get('text'), str) or not notes['text'].strip() or not handles or not all(h in entries for h in handles) or not any(entries[h]['source']['service'] == requirement['service'] for h in handles):
            issue('', 'SERVICE_SOURCE_NOTES_REQUIRED',
                  './docs review-page --page ' + requirement['page'] + ' --text-file NOTES.md --evidence HANDLE (source-first notes, then compare existing prose/arrows)', None)
        if not isinstance(topics, dict) or set(topics) != set(requirement['topics']):
            issue('', 'SERVICE_TOPICS_REQUIRED', requirement['topics'], list(topics) if isinstance(topics, dict) else topics)
            continue
        for topic, row in topics.items():
            if not isinstance(row, dict) or row.get('disposition') not in ('EXPLAINED', 'EXPLICIT_UNKNOWN'):
                issue(topic, 'SERVICE_TOPIC_ASSESSMENT_REQUIRED', ['EXPLAINED', 'EXPLICIT_UNKNOWN'], row);continue
            page, claim = row.get('page'), row.get('claim')
            fragment = fragments.get((page, claim)) if isinstance(page, str) and isinstance(claim, str) else None
            if fragment is None or owners.get(page) != requirement['service'] or claim == 'clew-scope':
                issue(topic, 'SERVICE_TOPIC_REFERENCE_REQUIRED', 'named reader fragment owned by this service, not the aggregate scope anchor', {'page': page, 'claim': claim});continue
            handles = fragment.get('evidence', [])
            if not handles or not all(h in entries for h in handles) or not any(entries[h]['source']['service'] == requirement['service'] for h in handles):
                issue(topic, 'SERVICE_TOPIC_EVIDENCE_REQUIRED', 'genuine retained evidence including this service', handles);continue
            if row['disposition'] == 'EXPLICIT_UNKNOWN' and not (isinstance(row.get('reason'), str) and row['reason'].strip()):
                issue(topic, 'SERVICE_TOPIC_ABSENCE_BOUND_REQUIRED', 'source-bounded reason and a reader-facing explanation in the referenced fragment', row.get('reason'));continue
            if topic == 'data fields and domain distinctions' and row['disposition'] == 'EXPLAINED':
                service = next(s for s in navigation['services'] if s['id'] == requirement['service'])
                if not any(d['target'].split('#')[0] == page for d in service['data']):
                    issue(topic, 'SERVICE_DATA_NAVIGATION_REQUIRED', './docs page-info --page ' + page + ' --service ' + requirement['service'] + ' --title "Reader data title" --data', page);continue
            if record:
                row['fragmentDigest'] = api['digest'](fragment)
                row['evidence'] = handles
            elif row.get('fragmentDigest') != api['digest'](fragment) or row.get('evidence') != handles:
                issue(topic, 'SERVICE_TOPIC_REVIEW_STALE', 'explicit review of the current referenced fragment and evidence', {'page': page, 'claim': claim})
    api['need'](not issues, 'SERVICE_TOPIC_CLOSURE_REQUIRED', issues=issues,
                recovery=issues[0]['recovery'] if issues else '', semanticTruthAssessed=False)


def sources(run, api):
    job, inventory, _ = api['frozen_inputs'](run)
    directory = run / 'workspace/checks' / ('topics-' + api['uuid'].uuid4().hex)
    return api['cite_entries'](job, inventory, run / 'workspace/draft', directory,
                               run / 'inputs/author-source')[1]


def save_notes(run, page, text, handles, api):
    _, inventory, _ = api['frozen_inputs'](run)
    requirement = next((r for r in inventory['requirements'] if r['kind'] == 'SERVICE' and r['page'] == page), None)
    entries = sources(run, api)
    api['need'](requirement and isinstance(text, str) and text.strip() and handles
        and all(h in entries for h in handles)
        and any(entries[h]['source']['service'] == requirement['service'] for h in handles),
        'SERVICE_SOURCE_NOTES_REQUIRED', page=page, field='sourceNotes',
        recovery='Save your compact source-first Markdown outline for this service with genuine retained handles; this is not a semantic approval.')
    fragment = {'id': 'source-notes', 'text': text, 'evidence': list(dict.fromkeys(handles))}
    api['save'](run / 'workspace/notes' / (page + '.json'), {'page': page, 'fragments': [fragment]})
    return {'status': 'SOURCE_NOTES_SAVED', 'page': page, 'ready': False, 'semanticTruthAssessed': False}
