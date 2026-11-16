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
        referenced = [v['page'] for v in topics.values() if isinstance(v, dict) and isinstance(v.get('page'), str)] if isinstance(topics, dict) else []
        notes = (account or {}).get('sourceNotes')
        references = notes.get('references') if isinstance(notes, dict) else None
        if isinstance(references, list):referenced += [v['page'] for v in references if isinstance(v, dict) and isinstance(v.get('page'), str)]
        if pages is not None and requirement['page'] not in pages and not set(referenced) & set(pages):continue
        def issue(topic, code, expected, actual):
            issues.append({'code': code, 'requirement': requirement['id'], 'page': requirement['page'],
                'field': '/requirements/' + requirement['id'] + '/topics' + ('/' + topic if topic else ''),
                'expected': expected, 'actual': actual,
                'recovery': './docs review-page --page ' + requirement['page'] +
                    ' (then --input FILE with source-reviewed topic references or bounded absence explanations)'})
        handles = notes.get('evidence', []) if isinstance(notes, dict) else []
        canonical = isinstance(notes, dict) and set(notes) == {'references'} and isinstance(references, list) and references
        legacy = isinstance(notes, dict) and 'references' not in notes and isinstance(notes.get('text'), str) and notes['text'].strip() and handles and all(h in entries for h in handles) and any(entries[h]['source']['service'] == requirement['service'] for h in handles)
        if not canonical and not legacy:
            issue('', 'SERVICE_SOURCE_NOTES_REQUIRED',
                  'Explicit source-reviewed canonical reader references in sourceNotes.references, or private source-first notes saved with ./docs review-page --page ' + requirement['page'] + ' --text-file NOTES.md --evidence HANDLE', notes)
        if canonical:
            seen = set()
            for index, reference in enumerate(references):
                page, claim = (reference.get('page'), reference.get('claim')) if isinstance(reference, dict) else (None, None)
                fragment = fragments.get((page, claim)) if isinstance(page, str) and isinstance(claim, str) else None
                code = None
                if not isinstance(reference, dict) or not {'page', 'claim'} <= reference.keys() <= {'page', 'claim', 'evidence', 'fragmentDigest'} or fragment is None or owners.get(page) != requirement['service'] or claim == 'clew-scope' or (page, claim) in seen:
                    code = 'SERVICE_SOURCE_REVIEW_REFERENCE_REQUIRED'
                else:
                    handles = fragment.get('evidence', [])
                    if not handles or not all(h in entries for h in handles) or not any(entries[h]['source']['service'] == requirement['service'] for h in handles):
                        code = 'SERVICE_SOURCE_REVIEW_EVIDENCE_REQUIRED'
                    elif any(key in reference and reference[key] != expected for key, expected in
                             (('fragmentDigest', api['digest'](fragment)), ('evidence', handles))):
                        code = 'SERVICE_SOURCE_REVIEW_STALE'
                    elif record:
                        reference.update(fragmentDigest=api['digest'](fragment), evidence=handles)
                    elif reference.get('fragmentDigest') != api['digest'](fragment) or reference.get('evidence') != handles:
                        code = 'SERVICE_SOURCE_REVIEW_STALE'
                if code:
                    issue('', code, 'Explicitly compare the retained source block with the current canonical reader fragment, then submit its page/claim reference with ./docs review-page --input FILE', reference)
                    issues[-1]['field'] = '/requirements/' + requirement['id'] + '/sourceNotes/references/' + str(index)
                if isinstance(page, str) and isinstance(claim, str):seen.add((page, claim))
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
                target = api['W']['fragment_target'](page, claim)
                targets = [target] if page == requirement['page'] else [page, target]
                if not any(d['target'] in targets for d in service['data']):
                    repair = './docs page-info --page ' + page + ' --service ' + requirement['service'] + ' --title "Reader data title" --data --fragment ' + claim
                    issue(topic, 'SERVICE_DATA_NAVIGATION_REQUIRED', repair,
                          {'page': page, 'fragment': claim, 'targets': [d['target'] for d in service['data']], 'expectedTargets': targets})
                    issues[-1]['recovery'] = repair
                    continue
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
    review_path = run / 'workspace/draft/self-review.json'
    if review_path.exists():
        review = api['load'](review_path)
        row = review['requirements'].get(requirement['id']) if review.get('schema') == api['COMPACT_REVIEW'] else None
        if row and isinstance(row.get('sourceNotes'), dict) and 'references' in row['sourceNotes']:
            # Explicit --text-file selects a fresh legacy outline. Reopen that
            # SERVICE review instead of silently approving a different basis.
            row.update(sourceNotes=fragment, disposition=None)
            api['save'](review_path, review)
            reviewed_path = run / 'workspace/draft/reviewed-pages.json'
            if reviewed_path.exists():
                reviewed = api['load'](reviewed_path);reviewed.pop(page, None);api['save'](reviewed_path, reviewed)
    return {'status': 'SOURCE_NOTES_SAVED', 'page': page, 'ready': False, 'semanticTruthAssessed': False}
