"""Inventory-owned structure and local edits in the existing original format.

Scaffolding and a checked local preview never assert semantic completion or READY.
"""
import html
import json
import re
import uuid
from pathlib import Path

KINDS = {'HTTP_ENDPOINT': 'http', 'KAFKA_LISTENER': 'kafka',
         'SCHEDULED_JOB': 'scheduled', 'OTHER': 'other'}
MARKER = '<!-- codeclew:unfinished -->'


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
        'Use ./docs save-pages --input pages/PAGE.md.json, ./docs check --page PAGE.md,\n'
        'and ./docs review-template. Finish the assigned unit, retain source reads\n'
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
    helper.write_text('#!' + python + '\nimport runpy,sys\n'
        'if len(sys.argv)<2 or sys.argv[1] not in ("save-pages","check","review-template"):\n'
        ' raise SystemExit("Use: ./docs save-pages --input pages/PAGE.md.json | check --page PAGE.md | review-template")\n'
        'sys.argv=[' + repr(str(skill / 'scripts/docs_run.py')) + ',*sys.argv[1:],"--run-dir",' + repr(str(run)) + ']\n'
        'runpy.run_path(sys.argv[0],run_name="__main__")\n', encoding='utf-8');helper.chmod(0o700)
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
    api['frozen_inputs'](run)
    patch = api['load'](input_path)
    api['need'](set(patch) == {'pages'} and isinstance(patch['pages'], list) and patch['pages'], 'INVALID_PAGE_UPDATE')
    original = api['load'](run / 'workspace/draft/original.json')
    scaffold = api['load'](run / 'inputs/scaffold.json')
    by_name = {p['page']: p for p in original['pages']}
    changed = []
    for page in patch['pages']:
        api['need'](isinstance(page, dict) and set(page) == {'page', 'fragments'}, 'INVALID_PAGE_UPDATE')
        name = api['M']['page_name'](page['page'])
        api['need'](name not in changed, 'DUPLICATE_PAGE_UPDATE', page=name)
        api['need'](isinstance(page['fragments'], list) and page['fragments'], 'INVALID_PAGE_UPDATE', page=name)
        info = scaffold['pages'].get(name, {'anchors': ['clew-scope']})
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
            reqs = [r for r in inventory['requirements'] if r['page'] in selected]
            api['save'](folder / 'sequences.json', {'pages': {p['page']: ''.join(f['text'] for f in p['fragments']) for p in original['pages']},
                'requirements': reqs, 'accounts': [{'requirement': r['id'], 'diagram': 0} for r in reqs if r['kind'] == 'ENTRY']})
            api['command']([job['node'], str(Path(__file__).with_name('docs_run_sequences.mjs')),
                            job['renderer'], str(folder / 'sequences.json')], folder, 'sequences')
            stage = 'source-bindings'
            manifests, entries = api['cite_entries'](job, inventory, run / 'workspace/draft', folder / 'sources', run / 'inputs/author-source')
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
        options = {'label': job['request'], 'theme': 'codeclew', 'language': job['language'],
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
        result['recovery'] = 'Edit the affected saved workspace/pages file, call save-pages, then repeat check. Unfinished targets are provisional; this is not READY.'
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
