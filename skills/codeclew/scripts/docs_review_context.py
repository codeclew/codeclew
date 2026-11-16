"""One complete source view beside an explicit reader selection; no verdict."""
import re
import shlex
import tempfile
from pathlib import Path


def display_citations(key, region, bindings, covering, api):
    """Select verified byte coverage for display; never reduce the proof map."""
    size = len(region['text'].encode())
    spans = []
    for index, handle in enumerate(region['citations']):
        row = bindings.get(handle, {});span = row.get('byteSlice', {})
        start, end = span.get('startByte'), span.get('endByte')
        api['need'](row.get('region') == key and type(start) is int and type(end) is int and 0 <= start <= end <= size,
                    'REVIEW_SOURCE_COVERAGE_INVALID', handle=handle)
        spans.append((start, -end, index, handle))
    full = {handle for start, end, _, handle in spans if start == 0 and -end == size}
    api['need'](isinstance(covering, list) and all(h in full for h in covering),
                'REVIEW_SOURCE_COVERING_REFERENCE_INVALID', view=key)
    if covering:return covering[:1]
    if full:return [next(handle for handle in region['citations'] if handle in full)]
    spans.sort();result=[];cursor=0;index=0
    while cursor < size:
        best=None
        while index < len(spans) and spans[index][0] <= cursor:
            candidate=spans[index];index+=1
            if best is None or candidate[1] < best[1]:best=candidate
        api['need'](best is not None and -best[1] > cursor, 'REVIEW_SOURCE_COVERAGE_INCOMPLETE', view=key, offset=cursor)
        result.append(best[3]);cursor=-best[1]
    api['need'](result, 'REVIEW_SOURCE_COVERAGE_INCOMPLETE', view=key)
    return result


def show(run, args, api):
    job, inventory, pins = api['frozen_inputs'](run)
    selection = getattr(args, 'input', None)
    api['need'](len(args.page) <= 1 and not (args.entries or args.entry or args.routes or args.route)
                and not (selection and (args.page or args.fragment)),
                'REVIEW_CONTEXT_PAGE_REQUIRED')
    original = api['load'](run / 'workspace/draft/original.json')
    pages = {p['page']: p for p in original['pages']}
    requested = []
    if selection:
        value = api['load'](selection);api['closed'](value, ('claims',))
        api['need'](isinstance(value['claims'], dict) and value['claims'], 'REVIEW_CONTEXT_SELECTION_REQUIRED')
        for name, claims in value['claims'].items():
            api['need'](isinstance(claims, dict) and claims and all(
                v in (None, 'SUPPORTED', 'EXPLICIT_UNKNOWN') for v in claims.values()), 'REVIEW_CONTEXT_SELECTION_REQUIRED')
            requested += [(name, identity) for identity in claims]
        api['need'](len(requested) <= 8, 'REVIEW_CONTEXT_SELECTION_TOO_LARGE', maximumFragments=8)
    elif args.page:
        name = args.page[0];api['need'](name in pages, 'SHOW_PAGE_NOT_FOUND', actual=name)
        requested = [(name, f['id']) for f in pages[name]['fragments'] if 'id' in f
                     and (args.fragment is None or f['id'] == args.fragment)]
        api['need'](requested, 'SHOW_FRAGMENT_NOT_FOUND', page=name, actual=args.fragment)
    else:
        api['need'](args.fragment is None, 'REVIEW_CONTEXT_PAGE_REQUIRED')
    fragments = []
    for name, identity in dict.fromkeys(requested):
        api['need'](name in pages, 'SHOW_PAGE_NOT_FOUND', actual=name)
        fragment = next((f for f in pages[name]['fragments'] if f.get('id') == identity), None)
        api['need'](fragment is not None, 'SHOW_FRAGMENT_NOT_FOUND', page=name, actual=identity)
        fragments.append((name, fragment))
    navigation = api['load'](run / 'workspace/draft/navigation.json')
    owners = [s['id'] for s in navigation['services'] if args.page and args.page[0] in s['pages']]
    service = args.service or (owners[0] if len(owners) == 1 else None)
    api['need'](service in inventory['services'] and (not owners or owners == [service]),
                'REVIEW_CONTEXT_SERVICE_REQUIRED', actual=service, owners=owners,
                recovery='Use the page owner; for an explicit group, shared page or source-only read select --service.')
    home = run / 'inputs/author-source' / service
    api['need']((home / 'views.json').is_file() and (home / 'requirements.json').is_file(), 'REVIEW_SOURCE_INVENTORY_REQUIRED', service=service)
    views = api['load'](home / 'views.json')
    requirements = api['load'](home / 'requirements.json')
    required = {view for r in requirements.values() if r.get('page') in {p for p, _ in fragments} for view in r.get('views', [])}
    base = ['--review-context', '--service', service]
    if args.page:base += ['--page', args.page[0]]
    if selection:base += ['--input', str(selection)]
    if args.fragment is not None:base += ['--fragment', args.fragment]
    if getattr(args, 'include_notes', False):base += ['--include-notes']
    def command(key, limit=None):
        return './docs show ' + shlex.join(base + ['--view', service + '/' + key] +
            (['--limit', str(limit)] if limit is not None else []))
    def inventory_text(exclude=None):return 'Available source views (inventory, not semantic dependency closure):\n' + ''.join(
        service + '/' + key + ' | ' + row.get('file', '(retained file)') + ' | ' + str(row.get('codeBytes', '?')) +
        ' source bytes' + (' | required for this page' if row['view'] in required else '') + '\n  ' + command(key) + '\n'
        for key, row in views.items() if key != exclude)
    api['need'](args.limit >= 0 and args.offset >= 0, 'INVALID_SHOW_WINDOW')
    if args.view is None:
        return api['H']['window']('Select one complete source view. No comparison context selected.\n' + inventory_text(),
                                   base, args.offset, args.limit)
    api['need'](args.offset == 0, 'REVIEW_CONTEXT_REQUIRES_WHOLE_VIEW',
                recovery='Select --view and optionally --fragment; source/code blocks are not character-windowed.')
    prefix, separator, key = args.view.partition('/')
    api['need'](separator and prefix == service and key in views, 'REVIEW_SOURCE_VIEW_NOT_FOUND',
                actual=args.view, available=[service + '/' + k for k in views])
    # Keep native membership reads on the admitted runtime without writing the
    # normal execution receipts into a read-only context request.
    version = api['subprocess'].run([job['clew'], '--version'], capture_output=True, text=True, timeout=30)
    api['need'](version.returncode == 0 and version.stdout.strip() == pins['clewVersion'], 'ADMITTED_RUNTIME_CHANGED')
    manifest = run / 'inputs/source-proof' / service / 'read/citations.json'
    api['need'](manifest.is_file(), 'REVIEW_SOURCE_PROOF_REQUIRED', view=args.view)
    api['M']['configure_engine'](job['clew'], job['root'], job.get('components', {}).get('path'), job.get('components', {}).get('digest'))
    try:_, packet, _, entries = api['M']['C']['load'](manifest)
    except Exception as error:
        api['need'](False, 'REVIEW_SOURCE_PROOF_INVALID', view=args.view,
                    cause=getattr(error, 'code', getattr(error, 'status', type(error).__name__)), details=str(error))
    api['need'](packet['scope']['snapshot'] == job['snapshot'] and all(
        e['source']['service'] == service and e['source']['revision'] == inventory['services'][service]['revision']
        for e in entries.values()), 'REVIEW_SOURCE_SCOPE_CHANGED', view=args.view)
    regions, bindings, _ = api['source_projection'](packet, entries)
    api['need'](key in regions and views[key].get('file') == regions[key]['file']
                and views[key].get('codeBytes') == len(regions[key]['text'].encode()),
                'REVIEW_SOURCE_VIEW_UNVERIFIED', view=args.view)
    region = regions[key];handles = set(region['citations'])
    displayed = display_citations(key, region, bindings, views[key].get('coveringCitations', []), api)
    with tempfile.TemporaryDirectory(prefix='clew-review-membership-') as temporary:
        cache = {}
        for handle in handles:
            if entries[handle].get('nativeBinding', {}).get('kind') != 'SOURCE_NATIVE':
                api['check_source_membership'](job, inventory, entries[handle], Path(temporary), cache)
    known = set(entries)
    for path in (run / 'inputs/source-proof').glob('*/citation-index.json'):known.update(api['load'](path))
    for name, fragment in fragments:
        api['need'](fragment.get('evidence') and all(h in known for h in fragment['evidence']),
                    'REVIEW_READER_EVIDENCE_REQUIRED', page=name, fragment=fragment['id'])
    parts = ['Review context: ' + args.view + '\nNo semantic verdict; compare source to reader for omissions, then reader to source for unsupported claims.\n',
        'Source: ' + service + '/' + region['file'] + ' @ ' + inventory['services'][service]['revision'] +
        '\nSource digest: ' + api['sha'](region['text'].encode()) + '\nCites: ' + ', '.join(displayed) + '\n']
    if len(handles) > len(displayed):
        parts.append('Other source bindings: ' + str(len(handles) - len(displayed)) +
                     '; full map: ../inputs/source-proof/' + service + '/citation-index.json\n')
    fence = '`' * max(3, max((len(m[0]) + 1 for m in re.finditer(r'`+', region['text'])), default=3))
    parts += [fence + '\n', region['text'], '\n' + fence + '\n\nCurrent reader fragments:\n']
    for name, fragment in fragments:
        parts += [name + '#' + fragment['id'] + ' @ ' + api['digest'](fragment) +
                  '\nCites: ' + ', '.join(fragment['evidence']) + '\n', fragment['text'], '\n']
    candidates = list(dict.fromkeys((p['page'], f['id']) for p in original['pages'] for f in p['fragments']
        if 'id' in f and handles.intersection(f.get('evidence', []))))
    selected = {(name, f['id']) for name, f in fragments}
    parts.append('\nUnselected candidate excerpts (shared evidence/view, not complete dependencies):\n' +
                 (''.join(p + '#' + identity + '\n' for p, identity in candidates if (p, identity) not in selected) or '(none)\n'))
    notes = []
    for requirement in inventory['requirements']:
        if requirement['kind'] != 'SERVICE' or requirement['service'] != service:continue
        path = run / 'workspace/notes' / (requirement['page'] + '.json')
        if path.exists():
            if getattr(args, 'include_notes', False):notes += api['load'](path)['fragments']
            else:parts.append('\nOwn notes available: notes/' + path.name + ' (' + str(path.stat().st_size) +
                              ' bytes); add --include-notes to include applicable notes.\n')
    for note in notes:
        api['need'](isinstance(note.get('text'), str) and note['text'].strip() and note.get('evidence')
                    and all(h in known for h in note['evidence']), 'REVIEW_NOTES_EVIDENCE_REQUIRED', service=service)
        if handles.intersection(note['evidence']):
            parts += ['\nExisting own scratch notes (prior claims, not source proof):\n', note['text'], '\n']
    remaining = [service + '/' + identity for identity in views if identity != key]
    parts.append('\nRemaining source views (not included): ' + (', '.join(remaining) or '(none)') +
                 '\nInventory/selection: ./docs show ' + shlex.join(base) + '\n')
    text = ''.join(parts)
    api['need'](args.limit == 0 or len(text) <= args.limit, 'REVIEW_CONTEXT_TOO_LARGE',
                view=args.view, sourceBytes=len(region['text'].encode()), readerFragments=len(fragments),
                contextCharacters=len(text), limit=args.limit,
                recovery='Select a smaller --fragment, or explicitly request this complete single-view context: ' + command(key, len(text)))
    return {'text': text, 'offset': 0, 'end': len(text), 'total': len(text), 'remaining': 0, 'next': None}
