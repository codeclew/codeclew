"""One selected retained source view beside current reader claims; no verdict."""
import re
import shlex
import tempfile
from pathlib import Path


def show(run, args, api):
    job, inventory, pins = api['frozen_inputs'](run)
    api['need'](len(args.page) == 1 and not (args.entries or args.entry or args.routes or args.route),
                'REVIEW_CONTEXT_PAGE_REQUIRED')
    original = api['load'](run / 'workspace/draft/original.json')
    page = next((p for p in original['pages'] if p['page'] == args.page[0]), None)
    api['need'](page is not None, 'SHOW_PAGE_NOT_FOUND', actual=args.page[0])
    fragments = [f for f in page['fragments'] if 'id' in f and (args.fragment is None or f['id'] == args.fragment)]
    api['need'](fragments and (args.fragment is None or len(fragments) == 1), 'SHOW_FRAGMENT_NOT_FOUND', page=page['page'], actual=args.fragment)
    navigation = api['load'](run / 'workspace/draft/navigation.json')
    owners = [s['id'] for s in navigation['services'] if page['page'] in s['pages']]
    service = args.service or (owners[0] if len(owners) == 1 else None)
    api['need'](service in inventory['services'] and (not owners or owners == [service]),
                'REVIEW_CONTEXT_SERVICE_REQUIRED', page=page['page'], actual=service, owners=owners,
                recovery='Use the page owner; for an unowned shared page select --service explicitly.')
    home = run / 'inputs/author-source' / service
    api['need']((home / 'views.json').is_file() and (home / 'requirements.json').is_file(), 'REVIEW_SOURCE_INVENTORY_REQUIRED', service=service)
    views = api['load'](home / 'views.json')
    requirements = api['load'](home / 'requirements.json')
    required = {view for r in requirements.values() if r.get('page') == page['page'] for view in r.get('views', [])}
    base = ['--review-context', '--page', page['page'], '--service', service]
    if args.fragment is not None:base += ['--fragment', args.fragment]
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
    with tempfile.TemporaryDirectory(prefix='clew-review-membership-') as temporary:
        cache = {}
        for handle in handles:
            if entries[handle].get('nativeBinding', {}).get('kind') != 'SOURCE_NATIVE':
                api['check_source_membership'](job, inventory, entries[handle], Path(temporary), cache)
    known = set(entries)
    for path in (run / 'inputs/source-proof').glob('*/citation-index.json'):known.update(api['load'](path))
    for fragment in fragments:
        api['need'](fragment.get('evidence') and all(h in known for h in fragment['evidence']),
                    'REVIEW_READER_EVIDENCE_REQUIRED', page=page['page'], fragment=fragment['id'])
    parts = ['Review context: ' + args.view + '\nNo semantic verdict; compare source to reader for omissions, then reader to source for unsupported claims.\n',
        'Source: ' + service + '/' + region['file'] + ' @ ' + inventory['services'][service]['revision'] +
        '\nSource digest: ' + api['sha'](region['text'].encode()) + '\nCites: ' + ', '.join(region['citations']) + '\n']
    fence = '`' * max(3, max((len(m[0]) + 1 for m in re.finditer(r'`+', region['text'])), default=3))
    parts += [fence + '\n', region['text'], '\n' + fence + '\n\nCurrent reader fragments:\n']
    for fragment in fragments:
        parts += [page['page'] + '#' + fragment['id'] + ' @ ' + api['digest'](fragment) +
                  '\nCites: ' + ', '.join(fragment['evidence']) + '\n', fragment['text'], '\n']
    candidates = list(dict.fromkeys((p['page'], f['id']) for p in original['pages'] for f in p['fragments']
        if 'id' in f and handles.intersection(f.get('evidence', []))))
    parts.append('\nCandidate related excerpts (shared evidence/view, not complete dependencies):\n' +
                 ''.join(p + '#' + identity + '\n' for p, identity in candidates))
    notes = []
    for requirement in inventory['requirements']:
        if requirement['kind'] != 'SERVICE' or requirement['service'] != service:continue
        path = run / 'workspace/notes' / (requirement['page'] + '.json')
        if path.exists():notes += api['load'](path)['fragments']
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
