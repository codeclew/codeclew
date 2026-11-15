"""Inventory-owned structure and local edits in the existing original format.

Scaffolding and a checked local preview never assert semantic completion or READY.
"""
import html
import json
import re
import shlex
import uuid
from pathlib import Path

KINDS = {'HTTP_ENDPOINT': 'http', 'KAFKA_LISTENER': 'kafka',
         'SCHEDULED_JOB': 'scheduled', 'OTHER': 'other'}
MARKER = '<!-- codeclew:unfinished -->'


def discovered_entry(parent, declaration, api):
    """One identity/address rule for registration, early checks and final review."""
    local = declaration.get('id')
    api['need'](isinstance(local, str) and re.fullmatch(r'[A-Za-z0-9_-]+', local),
                'INVALID_DISCOVERED_ENTRY_ID', actual=local)
    kind = declaration.get('kind')
    api['need'](kind in ('HTTP', 'KAFKA', 'SCHEDULED', 'OTHER'), 'INVALID_DISCOVERED_ENTRY_KIND')
    name = api['M']['page_name'](declaration.get('page'))
    row = {'id': parent['id'] + ':discovered:' + local, 'kind': 'ENTRY',
           'service': parent['service'], 'page': name, 'handlerId': 'discovered-' + local,
           'target': name + '#handler', 'sourceIds': parent.get('sourceIds', []),
           'method': declaration.get('method'), 'path': declaration.get('path'),
           'authority': 'AUTHOR_DISCOVERED_SYNTAX_NOT_COMPILER_BINDING',
           'entryKind': {'HTTP': 'HTTP_ENDPOINT', 'KAFKA': 'KAFKA_LISTENER',
                         'SCHEDULED': 'SCHEDULED_JOB', 'OTHER': 'OTHER'}[kind]}
    entry_kind(row, api['need'])
    return row


def entry_registry(inventory, accounts, api):
    rows = [{**r, 'target': r['page'] + '#handler'} for r in inventory['requirements'] if r['kind'] == 'ENTRY']
    additional = []
    parents = {r['id']: r for r in inventory['requirements'] if r['kind'] == 'DISCOVERY'}
    for account in accounts:
        parent = parents.get(account.get('requirement'))
        if not parent:continue
        for candidate in account.get('discoveredEntries') or []:
            row = discovered_entry(parent, candidate, api)
            api['need'](not any(r['id'] == row['id'] for r in rows), 'DUPLICATE_DISCOVERED_ENTRY', requirement=row['id'])
            api['need'](not any(r['page'] == row['page'] for r in rows), 'ENTRY_PAGE_COLLISION', page=row['page'],
                        recovery='Register each entry on its own page; link shared explanations.')
            rows.append(row);additional.append({**candidate, 'requirement': row['id']})
    return rows, additional


def registered_accounts(run, api):
    path = run / 'workspace/draft/entries.json'
    return api['load'](path) if path.exists() else []


def review_requirements(run, api):
    path = run / 'workspace/draft/self-review.json'
    if not path.exists():return []
    rows = api['load'](path)['requirements']
    return [{'requirement': key, **value} for key, value in rows.items()] if isinstance(rows, dict) else rows


def validate_discovered_sources(inventory, accounts, entries, api):
    parents = {r['id']: r for r in inventory['requirements'] if r['kind'] == 'DISCOVERY'}
    for account in accounts:
        parent = parents[account['requirement'].rsplit(':discovered:', 1)[0]]
        evidence = account.get('evidence')
        api['need'](isinstance(evidence, list) and evidence and all(h in entries for h in evidence)
                    and any(entries[h]['source']['file'] == parent['file']
                            and entries[h]['source']['service'] == parent['service'] for h in evidence),
                    'INVALID_DISCOVERED_ENTRY_ACCOUNT', page=account['page'], field='discoveredEntries/evidence',
                    expected={'service': parent['service'], 'file': parent['file']}, actual=evidence,
                    recovery='Use genuine retained handles from this discovery file; keep the entry identity and other pages.')


def declaration_identity(candidates):
    return [{k: v for k, v in c.items() if k != 'disposition'} for c in candidates]


def local_registry(run, inventory, api):
    accounts = registered_accounts(run, api)
    review = review_requirements(run, api)
    if review:
        known = {a['requirement']: a for a in accounts}
        for account in review:
            if account.get('discoveredEntries'):
                old = known.get(account['requirement'])
                if old:
                    registered = {c['id']: c for c in declaration_identity(old['discoveredEntries'])}
                    api['need'](all(c == registered.get(c['id']) for c in declaration_identity(account['discoveredEntries'])),
                                'ENTRY_REGISTRATION_REVIEW_MISMATCH', requirement=account['requirement'],
                                recovery='Use registered entries in the review template; do not redeclare their identities.')
                else:accounts.append(account)
    return entry_registry(inventory, accounts, api)


def register_entry(run, parent_id, declaration, api):
    _, inventory, _ = api['frozen_inputs'](run)
    parent = next((r for r in inventory['requirements'] if r['id'] == parent_id and r['kind'] == 'DISCOVERY'), None)
    api['need'](parent is not None, 'DISCOVERY_REQUIREMENT_REQUIRED', actual=parent_id)
    row = discovered_entry(parent, declaration, api)
    api['need'](declaration.get('evidence'), 'DISCOVERED_ENTRY_EVIDENCE_REQUIRED')
    candidate = {**declaration, 'section': 'handler', 'diagram': 0}
    accounts = registered_accounts(run, api)
    account = next((a for a in accounts if a['requirement'] == parent_id), None)
    if account is None:
        account = {'requirement': parent_id, 'discoveredEntries': []};accounts.append(account)
    previous = next((c for c in account['discoveredEntries'] if c['id'] == candidate['id']), None)
    same_identity = lambda item: {k: v for k, v in item.items() if k != 'evidence'}
    api['need'](previous is None or same_identity(previous) == same_identity(candidate), 'ENTRY_ALREADY_REGISTERED',
                requirement=row['id'], recovery='Keep the registered identity and title. Repeat register-entry with corrected evidence; update content with put or sequence.')
    changed = previous != candidate
    if previous is None:account['discoveredEntries'].append(candidate)
    else:account['discoveredEntries'][account['discoveredEntries'].index(previous)] = candidate
    entry_registry(inventory, accounts, api)
    original = api['load'](run / 'workspace/draft/original.json')
    api['need'](previous is not None or not any(p['page'] == row['page'] for p in original['pages']),
                'ENTRY_PAGE_EXISTS', page=row['page'], recovery='Choose a new entry page; existing content is preserved.')
    if previous is None:
        info = {'anchors': ['handler'], 'title': declaration['title'], 'entry': row}
        body = starter(row['page'], info)
        api['save'](run / 'workspace/pages' / (row['page'] + '.json'), {'pages': [body]})
        original['pages'].append({'page': row['page'], 'fragments': [anchors(info), *body['fragments']]})
        api['save'](run / 'workspace/draft/original.json', original)
    api['save'](run / 'workspace/draft/entries.json', accounts)
    if changed:
        # Evidence repair preserves content/identity and never carries forward a
        # semantic verdict made against different evidence declarations.
        review_path = run / 'workspace/draft/self-review.json'
        if review_path.exists():
            review = api['load'](review_path)
            rows = review['requirements']
            reviewed = rows.get(parent_id) if isinstance(rows, dict) else next((r for r in rows if r['requirement'] == parent_id), None)
            if reviewed:
                reviewed['disposition'] = None
                for found in reviewed.get('discoveredEntries') or []:
                    if found['id'] == candidate['id']:found.update(evidence=candidate['evidence'], disposition=None)
            api['save'](review_path, review)
        hashes_path = run / 'workspace/draft/reviewed-pages.json'
        if hashes_path.exists():
            hashes = api['load'](hashes_path)
            hashes.pop(parent['page'], None);hashes.pop(row['page'], None)
            api['save'](hashes_path, hashes)
    navigation = api['load'](run / 'workspace/draft/navigation.json')
    service = next(s for s in navigation['services'] if s['id'] == row['service'])
    if row['page'] not in service['pages']:service['pages'].append(row['page'])
    if not any(s['target'] == row['target'] for s in service['starts']):
        service['starts'].append({'title': declaration['title'], 'target': row['target'],
                                  'kind': entry_kind(row, api['need']), 'summary': declaration.get('summary', declaration['title'])})
    api['save'](run / 'workspace/draft/navigation.json', navigation)
    return {'status': 'ENTRY_REGISTERED', 'entry': row, 'ready': False}


def entry_kind(row, need):
    kind = row.get('entryKind')
    need(isinstance(kind, str) and kind in KINDS, 'INVALID_ENTRY_KIND', requirement=row.get('id'),
         field='entryKind', actual=kind, expected=list(KINDS))
    if kind == 'HTTP_ENDPOINT':
        need(isinstance(row.get('method'), str) and row['method'] and
             isinstance(row.get('path'), str) and row['path'],
             'INVALID_HTTP_ENTRY', requirement=row.get('id'), field='method/path')
    return KINDS[kind]


def build(inventory, api):
    pages = {}
    for row in inventory['requirements']:
        name = api['M']['page_name'](row['page'])
        if row['kind'] == 'ENTRY':
            entry_kind(row, api['need'])
        info = pages.setdefault(name, {'anchors': [], 'requirements': [], 'title': row['service'], 'entry': None})
        info['requirements'].append(row['id'])
        anchor = 'handler' if row['kind'] == 'ENTRY' else 'clew-scope'
        if anchor not in info['anchors']:info['anchors'].append(anchor)
        if row['kind'] == 'ENTRY':
            info['entry'] = row
            info['title'] = row['method'] + ' ' + row['path'] if row.get('method') else row['entryKind']
    units = [{'id': service, 'pages': [name for name, info in pages.items()
              if any(r['id'] in info['requirements'] and r['service'] == service for r in inventory['requirements'])]}
             for service in inventory['services']]
    units.append({'id': 'complete-docset', 'pages': list(pages)})
    return {'schema': 'codeclew-docs-scaffold/1.0', 'pages': pages, 'units': units}


def starter(name, info):
    fragments = [{'text': '# ' + info['title'] + '\n\n'},
                 {'text': MARKER + '\n\n> Draft: source explanation and review required.\n\n'}]
    row = info['entry']
    if row:
        sequence = {'title': info['title'], 'participants': [
            {'id': 'caller', 'title': 'Caller'},
            {'id': row['handlerId'], 'title': 'Handler', 'target': name + '#handler'}],
            'steps': [{'from': 'caller', 'to': row['handlerId'],
                       'label': info['title'], 'kind': 'call', 'target': name + '#handler'}]}
        fragments.append({'text': '```clew-sequence\n' + json.dumps(sequence, ensure_ascii=False) + '\n```\n'})
    return {'page': name, 'fragments': fragments}


def anchors(info):
    return {'text': '\n\n' + '\n\n'.join('<a id="' + html.escape(key, quote=True) + '"></a>'
                                         for key in info['anchors']) + '\n\n'}


def install(inventory, run, api):
    scaffold = build(inventory, api)
    api['save'](run / 'inputs/scaffold.json', scaffold)
    original = {'pages': []}
    for name, info in scaffold['pages'].items():
        body = starter(name, info)
        api['save'](run / 'workspace/pages' / (name + '.json'), {'pages': [body]})
        original['pages'].append({'page': name, 'fragments': [anchors(info), *body['fragments']]})
    api['save'](run / 'workspace/draft/original.json', original)
    return scaffold


def unfinished(original):
    return [p['page'] for p in original['pages'] if
            any(MARKER in f.get('text', '') for f in p['fragments']) or
            not any('id' in f and 'evidence' in f for f in p['fragments'])]


def install_author_entry(run, skill, python):
    workspace = run / 'workspace'
    contract = str(skill / 'references/docs-run-author.md')
    instructions = ('# Prepared Codeclew author\n\n'
        'This run has already admitted its runtime, source scope and workflow.\n'
        'Use the local codeclew skill profile and read ' + contract + '.\n'
        'Keep that full semantic contract through every unit and context compaction.\n'
        'Do not restart the globally installed skill, generic HTTP closure, source\n'
        'admission or export workflow. The caller already owns those operations.\n'
        'Use ./docs put/sequence for small semantic updates, ./docs check --page PAGE.md,\n'
        'and ./docs review-page for explicit local judgments. Finish the assigned unit, retain source reads\n'
        'and local page files, and perform the required bidirectional semantic review.\n'
        'Successful checks print compact summaries. Full receipts remain on disk\n'
        'and --full is available for diagnostics; do not reread assembled checker\n'
        'inputs after PASS. Review the actual source and relevant saved page instead.\n'
        'Neither a local check nor this scaffold certifies meaning or full readiness.\n'
        'Do not modify this entry, ./docs or the local skill profile.\n')
    (workspace / 'AGENTS.md').write_text(instructions, encoding='utf-8')
    profile = workspace / '.agents/skills/codeclew/SKILL.md';profile.parent.mkdir(parents=True)
    profile.write_text('---\nname: codeclew\ndescription: Author documentation in this already prepared Codeclew run, using its admitted retained sources and fixed final gates.\n---\n\n'
        '# Prepared documentation author\n\nRead ' + contract + '.\n\n'
        'This project profile selects the prepared-author workflow for this run.\n'
        'Use ./docs and the local AGENTS.md; keep the full request, source scope,\n'
        'semantic self-review and final source/route/export requirements unchanged.\n'
        'Do not restart acquisition/admission or substitute the global workflow.\n', encoding='utf-8')
    helper = workspace / 'docs'
    helper.write_text('#!/bin/sh\ncase "${1-}" in\n'
        ' save-pages|check|review-template|register-entry|put|sequence|page-info|route|review-page|finish-review) ;;\n'
        ' *) printf "%s\\n" "Use: ./docs save-pages --input pages/PAGE.md.json | check --page PAGE.md | review-template" >&2; exit 2 ;;\n'
        'esac\nexec ' + shlex.quote(python) + ' -I -S ' + shlex.quote(str(skill / 'scripts/docs_run.py')) +
        ' "$@" --run-dir ' + shlex.quote(str(run)) + '\n', encoding='utf-8');helper.chmod(0o700)
    return ['AGENTS.md', '.agents/skills/codeclew/SKILL.md', 'docs']


def author_skill_config(run):
    # Per-process CLI selection: the prepared profile wins; global user config
    # stays unchanged. Path selection overrides the disabled generic name.
    profile = str(run / 'workspace/.agents/skills/codeclew/SKILL.md')
    return 'skills.config=[{name="codeclew",enabled=false},{path=' + json.dumps(profile) + ',enabled=true}]'


def summary(result):
    """Small author feedback; complete receipts and source diagnostics stay available."""
    small = {'status': result['status'], 'ready': False}
    if result['status'] == 'PAGES_SAVED':
        small.update(changed=result['changed'], unchangedCount=len(result['unchanged']),
                     unfinishedCount=len(result['unfinishedPages']), receipt='last-save-pages.json')
    elif 'selectedPages' in result:
        small.update(selectedCount=len(result['selectedPages']), unfinishedCount=len(result['unfinishedPages']),
                     sourceBindingsChecked=result['sourceBindingsChecked'], receipt='last-local-check.json')
    for key in ('code', 'stage', 'details', 'locations', 'pendingTargets', 'recovery'):
        if key in result:small[key] = result[key]
    return small


def save_pages(run, input_path, api):
    _, inventory, _ = api['frozen_inputs'](run)
    patch = api['load'](input_path)
    api['need'](set(patch) == {'pages'} and isinstance(patch['pages'], list) and patch['pages'], 'INVALID_PAGE_UPDATE')
    original = api['load'](run / 'workspace/draft/original.json')
    scaffold = api['load'](run / 'inputs/scaffold.json')
    entries, _ = local_registry(run, inventory, api)
    by_name = {p['page']: p for p in original['pages']}
    changed = []
    for page in patch['pages']:
        api['need'](isinstance(page, dict) and set(page) == {'page', 'fragments'}, 'INVALID_PAGE_UPDATE')
        name = api['M']['page_name'](page['page'])
        api['need'](name not in changed, 'DUPLICATE_PAGE_UPDATE', page=name)
        api['need'](isinstance(page['fragments'], list) and page['fragments'], 'INVALID_PAGE_UPDATE', page=name)
        info = scaffold['pages'].get(name, {'anchors': ['handler'] if any(r['page'] == name for r in entries) else ['clew-scope']})
        fragments = []
        for index, fragment in enumerate(page['fragments']):
            api['need'](isinstance(fragment, dict) and isinstance(fragment.get('text'), str),
                        'INVALID_PAGE_FRAGMENT', page=name, fragmentIndex=index)
            text = fragment['text']
            for key in info['anchors']:
                api['need'](not re.search(r'<a\s+id=[\"\x27]' + re.escape(key) + r'[\"\x27]', text),
                            'MANAGED_ANCHOR_IN_AUTHORED_FRAGMENT', page=name, fragmentIndex=index,
                            field='text', anchor=key, recovery='Remove the managed anchor; save-pages supplies it.')
            # Dedicated, unannotated anchor blocks are assembly, not prose. Never
            # rewrite an annotated fragment or a tag embedded inside source/code.
            if set(fragment) == {'text'} and re.fullmatch(r'\s*<a id="[A-Za-z0-9._-]+"></a>\s*', text):
                fragment = {'text': '\n\n' + text.strip() + '\n\n'}
            fragments.append(fragment)
        candidate = {'page': name, 'fragments': [anchors(info), *fragments]}
        if any('id' in f or 'evidence' in f for f in fragments):
            api['M']['compose_pages']({'pages': [candidate]})
        by_name[name] = candidate
        changed.append(name)
    before = {p['page']: api['digest'](p) for p in original['pages']}
    original['pages'] = list(by_name.values())
    api['save'](run / 'workspace/draft/original.json', original)
    result = {'status': 'PAGES_SAVED', 'changed': changed,
            'unchanged': [name for name, value in before.items() if name not in changed and api['digest'](by_name[name]) == value],
            'unfinishedPages': unfinished(original), 'ready': False, 'semanticTruthAssessed': False}
    api['save'](run / 'workspace/last-save-pages.json', result)
    return result


def locations(original, details):
    """Locate a structural error in authored fields; no condition interpretation."""
    message = str(details)
    if isinstance(details, dict) and details.get('page') and details.get('field'):
        return [{key: details[key] for key in ('page', 'fragment', 'field') if key in details}]
    def find(value, key):
        if isinstance(value, dict):
            if key in value:return value[key]
            for item in value.values():
                found = find(item, key)
                if found is not None:return found
        return None
    owner = find(details, 'page')
    if isinstance(owner, str):owner = owner.replace('.html', '.md')
    component = re.search(r'Invalid sequence (/[^:]*):', message)
    candidates = []
    for pi, page in enumerate(original['pages']):
        for fi, fragment in enumerate(page['fragments']):
            text = fragment.get('text', '')
            targets = re.findall(r'[A-Za-z0-9._-]+\.md(?:#[A-Za-z0-9._-]+)?', text)
            anchor = re.findall(r'<a\s+id=[\"\x27]([^\"\x27]+)', text)
            if owner and owner != page['page']:continue
            matched = ('clew-sequence' in text if component else
                       any(t in message.replace('.html', '.md') for t in targets) or any(a in message for a in anchor))
            if matched:
                row = {'page': page['page'], 'fragment': fragment.get('id'),
                       'field': '/pages/' + str(pi) + '/fragments/' + str(fi) + '/text'}
                if component:row['componentField'] = component[1]
                candidates.append(row)
    if not candidates and owner:
        candidates = [{'page': owner, 'field': 'fragments'}]
    return candidates


def put(run, page, fragment, text, evidence, sequence, api):
    """Replace one meaningful fragment, serializing envelopes/fences ourselves."""
    _, inventory, _ = api['frozen_inputs'](run)
    name = api['M']['page_name'](page)
    api['need'](isinstance(fragment, str) and re.fullmatch(r'[A-Za-z0-9._-]+', fragment), 'INVALID_FRAGMENT_ID')
    api['need'](evidence and all(isinstance(h, str) and h for h in evidence), 'CLAIM_EVIDENCE_REQUIRED')
    original = api['load'](run / 'workspace/draft/original.json')
    existing = next((p for p in original['pages'] if p['page'] == name), None)
    managed = {'handler', 'clew-scope'}
    body = ([f for f in existing['fragments'] if MARKER not in f['text']
             and not (set(f) == {'text'} and all(not line.strip() or any(line.strip() == '<a id="' + a + '"></a>' for a in managed)
                                               for line in f['text'].splitlines()))] if existing else [])
    if sequence:
        value = json.loads(text)
        entries, _ = local_registry(run, inventory, api)
        entry = next((r for r in entries if r['page'] == name), None)
        if entry:
            # 'handler' is a local role, not another author-managed global ID.
            for participant in value.get('participants', []):
                if participant.get('id') == 'handler':
                    participant['id'] = entry['handlerId'];participant['target'] = entry['target']
            for step in value.get('steps', []):
                for key in ('from', 'to'):
                    if step.get(key) == 'handler':step[key] = entry['handlerId']
            if value.get('steps') and value['steps'][0].get('to') == entry['handlerId']:
                value['steps'][0]['target'] = entry['target']
                if entry.get('method'):value['steps'][0]['label'] = entry['method'] + ' ' + entry['path']
        text = '```clew-sequence\n' + json.dumps(value, ensure_ascii=False, separators=(',', ':')) + '\n```'
        # Replace the unbound starter, not the semantic work on another fragment.
        body = [f for f in body if not (set(f) == {'text'} and f['text'].startswith('```clew-sequence'))]
    if any(set(f) == {'text'} and f['text'].startswith('```clew-sequence') for f in body):
        body.insert(0, {'text': MARKER + '\n\n'})
    update = {'id': fragment, 'text': '\n\n' + text.strip() + '\n\n', 'evidence': list(dict.fromkeys(evidence))}
    index = next((i for i, f in enumerate(body) if f.get('id') == fragment), None)
    if index is None:body.append(update)
    else:body[index] = update
    path = run / 'workspace/pages' / (name + '.json')
    api['save'](path, {'pages': [{'page': name, 'fragments': body}]})
    return save_pages(run, path, api)


def page_info(run, name, service, title, data, api):
    _, inventory, _ = api['frozen_inputs'](run)
    name = api['M']['page_name'](name)
    api['need'](service in inventory['services'], 'UNKNOWN_PAGE_OWNER', actual=service)
    navigation = api['load'](run / 'workspace/draft/navigation.json')
    api['need'](not any(name in s['pages'] and s['id'] != service for s in navigation['services']), 'PAGE_ALREADY_OWNED', page=name)
    owner = next(s for s in navigation['services'] if s['id'] == service)
    if name not in owner['pages']:owner['pages'].append(name)
    if data:
        owner['data'] = [d for d in owner['data'] if d['target'] != name] + [{'title': title, 'target': name}]
    api['save'](run / 'workspace/draft/navigation.json', navigation)
    return {'status': 'PAGE_REGISTERED', 'page': name, 'ready': False}


def route(run, value, api):
    job, inventory, _ = api['frozen_inputs'](run)
    path = run / 'workspace/draft/routes.json'
    api['closed'](value, ('route',), ('catalog', 'externalActors'))
    api['need'](path.exists() or isinstance(value.get('catalog'), str), 'ROUTE_CATALOG_REQUIRED',
                recovery='Supply catalog: PAGE.html on the first route update.')
    registry = api['load'](path) if path.exists() else {
        'schema': 'codeclew-cross-docset-routes/1.1', 'revision': 'authored-v1',
        'catalog': {'docset': 'docs', 'page': value['catalog']},
        'docsets': [{'id': 'docs', 'title': 'Service documentation', 'version': 'v1',
                     'baseURL': 'https://docs.invalid/', 'services': [
                         {'id': service, 'title': service, 'overview': {'docset': 'docs', 'page': next(
                             r['page'].removesuffix('.md') + '.html' for r in inventory['requirements'] if r['service'] == service and r['kind'] == 'SERVICE')}}
                         for service in job['services']]}], 'externalActors': [], 'routes': []}
    for actor in value.get('externalActors', []):
        old = next((a for a in registry['externalActors'] if a['id'] == actor['id']), None)
        api['need'](old is None or old == actor, 'EXTERNAL_ACTOR_CONFLICT', actor=actor['id'])
        if old is None:registry['externalActors'].append(actor)
    row = json.loads(json.dumps(value['route']))
    def local_addresses(item):
        if isinstance(item, dict):
            if 'page' in item:
                item.setdefault('docset', 'docs')
                item['page'] = item['page'].removesuffix('.md').removesuffix('.html') + '.html'
            if item.get('kind') == 'service':item.setdefault('docset', 'docs')
            for child in item.values():local_addresses(child)
        elif isinstance(item, list):
            for child in item:local_addresses(child)
    local_addresses(row)
    row.setdefault('id', row['narrative']['page'].removesuffix('.html'))
    row.setdefault('version', 'v1')
    registry['routes'] = [r for r in registry['routes'] if r['id'] != row['id']] + [row]
    api['save'](path, registry)
    return {'status': 'ROUTE_SAVED', 'route': row['id'], 'ready': False}


def review_page(run, page, value, api):
    _, inventory, _ = api['frozen_inputs'](run)
    original = api['load'](run / 'workspace/draft/original.json')
    template = api['review_for_run'](run)
    name = api['M']['page_name'](page)
    owned = [r['id'] for r in inventory['requirements'] if r['page'] == name]
    shell = {'requirements': {key: template['requirements'][key] for key in owned},
             'claims': template['claims'].get(name, {})}
    if value is None:return shell
    api['closed'](value, ('requirements', 'claims'))
    api['need'](isinstance(value['requirements'], dict) and isinstance(value['claims'], dict)
                and value['requirements'].keys() == shell['requirements'].keys()
                and value['claims'].keys() == shell['claims'].keys(), 'PAGE_REVIEW_SCOPE_MISMATCH', page=name,
                expected=shell, actual=value, recovery='Use ./docs review-page --page ' + name + ' to obtain the current local assessment shell.')
    api['need'](all(v in ('SUPPORTED', 'EXPLICIT_UNKNOWN') for v in value['claims'].values()),
                'CLAIM_ASSESSMENT_REQUIRED', page=name)
    api['need'](all(v.get('disposition') in ('EXPLAINED', 'EXPLICIT_UNKNOWN') for v in value['requirements'].values()),
                'REQUIREMENT_ASSESSMENT_REQUIRED', page=name)
    template['requirements'].update(value['requirements'])
    if name in template['claims']:template['claims'][name] = value['claims']
    api['save'](run / 'workspace/draft/self-review.json', template)
    path = run / 'workspace/draft/reviewed-pages.json'
    reviewed = api['load'](path) if path.exists() else {}
    reviewed[name] = api['digest'](next(p for p in original['pages'] if p['page'] == name))
    api['save'](path, reviewed)
    return {'status': 'PAGE_REVIEW_RECORDED', 'page': name, 'ready': False, 'semanticTruthAssessed': False}


def process_pages(run, api):
    path = run / 'workspace/draft/routes.json'
    if not path.exists():return []
    registry = api['load'](path)
    local = registry['catalog']['docset']
    return sorted({r['narrative']['page'].removesuffix('.html') + '.md' for r in registry['routes']
                   if r['narrative']['docset'] == local})


def finish_review(run, value, api):
    api['frozen_inputs'](run)
    api['closed'](value, ('unresolved', 'visual'))
    review = api['review_for_run'](run)
    api['need'](isinstance(value['unresolved'], list) and isinstance(value['visual'], dict), 'GLOBAL_REVIEW_REQUIRED')
    review.update(value)
    api['save'](run / 'workspace/draft/self-review.json', review)
    return {'status': 'GLOBAL_REVIEW_RECORDED', 'ready': False, 'semanticTruthAssessed': False}


def check(run, pages, scaffold_only, api):
    job, inventory, _ = api['frozen_inputs'](run)
    original = api['load'](run / 'workspace/draft/original.json')
    before = api['tree'](run / 'workspace/draft')
    pending = unfinished(original)
    selected = pages or [p['page'] for p in original['pages'] if p['page'] not in pending]
    folder = run / 'workspace/checks' / uuid.uuid4().hex
    folder.mkdir(mode=0o700, parents=True)
    result = {'status': 'FAILED', 'ready': False, 'semanticTruthAssessed': False,
              'selectedPages': selected, 'unfinishedPages': pending, 'directory': str(folder),
              'sourceBindingsChecked': False, 'sourceBindingScope': selected, 'fullSiteReady': False}
    stage = 'selection'
    try:
        names = [p['page'] for p in original['pages']]
        api['need'](len(names) == len(set(names)), 'DUPLICATE_PAGE_SELECTION')
        api['need'](scaffold_only or selected, 'NO_COMPLETE_PAGES')
        api['need'](set(selected) <= set(names), 'UNKNOWN_CHECK_PAGE', pages=sorted(set(selected) - set(names)))
        api['need'](scaffold_only or not set(selected) & set(pending), 'UNFINISHED_SCAFFOLD', pages=sorted(set(selected) & set(pending)))
        stage = 'source-bindings'
        if not scaffold_only:
            chosen = {'pages': [p for p in original['pages'] if p['page'] in selected]}
            api['save'](folder / 'original.json', chosen)
            api['M']['configure_engine'](job['clew'], job['root'],
                job.get('components', {}).get('path'), job.get('components', {}).get('digest'))
            api['M']['compose'](folder / 'original.json', folder / 'compose')
            stage = 'entry-sequences'
            entries, additional = local_registry(run, inventory, api)
            reqs = [r for r in entries if r['page'] in selected]
            accounts = {r['id']: {'requirement': r['id'], 'diagram': 0} for r in reqs}
            accounts.update({r['requirement']: r for r in review_requirements(run, api)
                             if r['requirement'] in accounts and r.get('diagram') is not None})
            accounts.update({r['requirement']: r for r in additional if r['requirement'] in accounts})
            api['save'](folder / 'sequences.json', {'pages': {p['page']: ''.join(f['text'] for f in p['fragments']) for p in original['pages']},
                'requirements': reqs, 'accounts': list(accounts.values()),
                'processPages': [p for p in process_pages(run, api) if p in selected]})
            api['command']([job['node'], str(Path(__file__).with_name('docs_run_sequences.mjs')),
                            job['renderer'], str(folder / 'sequences.json')], folder, 'sequences')
            stage = 'source-bindings'
            manifests, entries = api['cite_entries'](job, inventory, run / 'workspace/draft', folder / 'sources', run / 'inputs/author-source')
            validate_discovered_sources(inventory, [r for r in additional if r['page'] in selected], entries, api)
            for page in chosen['pages']:
                for fi, fragment in enumerate(page['fragments']):
                    for ei, handle in enumerate(fragment.get('evidence', [])):
                        api['need'](handle in entries, 'INVALID_CITATION_HANDLE', page=page['page'],
                            fragment=fragment.get('id'), field='fragments/' + str(fi) + '/evidence/' + str(ei), handle=handle)
            bound = api['M']['bind'](manifests, folder / 'compose/markdown', folder / 'compose/request.json', folder / 'bound')
            api['M']['check'](manifests, bound['index'])
            result['sourceBindingsChecked'] = True
        stage = 'render'
        markdown = folder / 'markdown'; markdown.mkdir()
        for page in original['pages']:
            name = api['M']['page_name'](page['page'])
            (markdown / name).write_text(''.join(f['text'] for f in page['fragments']), encoding='utf-8')
        options = {'label': 'Service documentation', 'theme': 'codeclew', 'language': job['language'],
                   'navigation': str(run / 'workspace/draft/navigation.json')}
        routes = run / 'workspace/draft/routes.json'
        if routes.exists():
            registry = api['load'](routes); local = registry.get('catalog', {}).get('docset')
            docsets = [d for d in registry['docsets'] if d['id'] == local]
            api['need'](len(docsets) == 1, 'LOCAL_PROCESS_CATALOG_REQUIRED')
            options.update(docsetRoutes=str(routes), docset=local, docsetVersion=docsets[0]['version'])
        api['save'](folder / 'renderer.json', {'input': str(markdown), 'output': str(folder / 'html'), 'options': options})
        api['command']([job['node'], str(Path(__file__).with_name('docs_workspace_render.mjs')),
                        job['renderer'], str(folder / 'renderer.json')], folder, 'render')
        documents = {p.name: p.read_bytes() for p in (folder / 'html').glob('*.html')}
        for name, raw in documents.items():
            try:
                parser = api['E']['Links'](); parser.feed(raw.decode()); parser.close()
            except api['E']['Stop'] as error:
                raise api['Stop'](error.status, page=name.replace('.html', '.md'), **error.details)
        result['internalLinks'] = api['E']['check_links'](documents, (folder / 'html/favicon.svg').read_bytes())
        api['frozen_inputs'](run)
        api['need'](api['tree'](run / 'workspace/draft') == before, 'DRAFT_CHANGED_DURING_CHECK')
        result['status'] = 'SCAFFOLD_CHECKED' if scaffold_only else 'UNIT_CHECKED'
        result['routesPending'] = len(job['services']) > 1 and not routes.exists()
        api['save'](run / 'workspace/last-local-check.json', result)
        if not scaffold_only:
            api['save'](run / 'workspace/last-valid-preview.json', {'directory': str(folder), 'draftDigest': api['digest'](before), 'ready': False})
    except (api['Stop'], api['E']['Stop'], api['M']['Stop'], OSError, ValueError, KeyError, TypeError) as error:
        result.update(stage=stage, code=getattr(error, 'code', getattr(error, 'status', 'LOCAL_CHECK_FAILED')),
                      details=getattr(error, 'details', str(error)))
        result['locations'] = locations(original, result['details'])
        result['pendingTargets'] = sorted({target for target in re.findall(r'[A-Za-z0-9._-]+\.md(?:#[A-Za-z0-9._-]+)?', str(result['details']).replace('.html','.md'))
                                          if target.split('#')[0] in pending})
        result['recovery'] = ('Use the named page/fragment/field with ./docs put for prose or ./docs sequence for diagram rows; '
                              'repair registration evidence by repeating ./docs register-entry --requirement ID --page PAGE --evidence HANDLE. '
                              'Then repeat check only for affected pages. Preserve other pages and reviews. Unfinished targets are provisional; this is not READY.')
        api['save'](run / 'workspace/last-local-check.json', result)
    api['save'](folder / 'result.json', result)
    return result


def run_units(run, api, debug=False):
    """One author/context, bounded related-page stages, shared aggregate budget."""
    job, _, _ = api['inputs_unchanged'](run)
    units = api['load'](run / 'inputs/scaffold.json')['units']
    state = api['load'](run / 'state.json')
    api['need'](state['status'] not in {'INTERRUPTED', 'AUTHOR_FAILED', 'AUTHOR_BUDGET_EXCEEDED',
                'AUTHOR_USAGE_UNAVAILABLE', 'AUTHOR_USAGE_INVALID'}, 'AUTHOR_NOT_RESUMABLE')
    if state['status'] == 'AUTHOR_COMPLETED' and state['attempts'][-1].get('kind', 'full') == 'full':
        state['unitCursor'] = len(units);api['save'](run / 'state.json', state)
    while True:
        state = api['load'](run / 'state.json')
        cursor = state.get('unitCursor', 0)
        if cursor == len(units):break
        unit = units[cursor]
        repair = state.get('status') == 'REPAIR_REQUIRED'
        if not (state.get('status') == 'AUTHOR_COMPLETED' and state.get('completedUnit') == unit['id']):
            accounting = api['token_accounting'](state['attempts'])
            limit = job['budget']['maxModelTokens']
            remaining = None if limit is None else limit - api['budget_tokens'](accounting)
            repairs = [a for a in state['attempts'] if a.get('kind') == 'repair']
            observed = repairs[-1:] or state['attempts'][-1:]
            deltas = accounting.get('attemptUsage', [])
            reserve = (deltas[state['attempts'].index(observed[-1])]['budgetTokens'] or 0) if observed else 0
            state['budgetPlan'] = {'remainingTokens': remaining, 'repairReserveEstimate': reserve,
                'basis': 'latest completed repair' if repairs else 'latest completed unit proxy' if observed else 'no observed cost yet',
                'advisoryOnly': True, 'hardProviderCeiling': False}
            api['save'](run / 'state.json', state)
            api['author'](run, unit=unit, repair=repair)
        pages = [] if unit['id'] == 'complete-docset' else unit['pages']
        checked = check(run, pages, False, api)
        state = api['load'](run / 'state.json')
        if checked['status'] != 'UNIT_CHECKED':
            report = {'status': 'FAILED', 'ready': False, 'unit': unit['id'], 'checks': {'local': checked}}
            api['save'](run / 'report.json', report)
            count = sum(a.get('kind') == 'repair' for a in state['attempts'])
            if count >= job['budget']['maxRepairs']:return report
            state['status'] = 'REPAIR_REQUIRED';api['save'](run / 'state.json', state)
            continue
        state['unitCursor'] = cursor + 1
        state['status'] = 'AUTHOR_COMPLETED' if cursor + 1 == len(units) else 'UNIT_COMPLETED'
        api['save'](run / 'state.json', state)
    # The existing complete gate is still authoritative; a unit receipt cannot
    # bypass source/review/route/export/usage checks or promote a scaffold.
    while True:
        result = api['verify'](run, debug=debug)
        if result['status'] == 'READY':return api['promote'](run)
        state = api['load'](run / 'state.json')
        count = sum(a.get('kind') == 'repair' for a in state['attempts'])
        if result['status'] == 'BLOCKED' or result['checks']['inputs']['status'] != 'PASS' or count >= job['budget']['maxRepairs']:
            return result
        api['author'](run, unit=units[-1], repair=True)
