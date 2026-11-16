"""Addressed, read-only views of existing authored objects. No proof-registry dump."""
import json
import shlex


def source_links(views, requirements=None):
    links = {}
    for service, rows in views.items():
        for row in rows.values():
            for handle in row['coveringCitations']:
                links.setdefault(handle, []).append('../inputs/author-source/' + service + '/' + row['view'])
    for service, rows in (requirements or {}).items():
        for row in rows.values():
            # A single explicit view locates every candidate in this requirement.
            # Multiple views give requirement context, not a per-citation mapping.
            paths = row.get('views', [])
            if len(paths) == 1:
                for handle in row.get('evidenceCandidates', []):
                    links.setdefault(handle, []).append('../inputs/author-source/' + service + '/' + paths[0])
    return links


def format_page(page, fragment=None, links=None, requirement_paths=()):
    parts = ['Page: ' + page['page'] + '\n']
    handles = []
    for item in page['fragments']:
        if fragment is not None and item.get('id') != fragment:continue
        parts.append('\nFragment: ' + (item.get('id') or '(unannotated structure)') + '\n')
        if item.get('evidence'):
            parts.append('Cites: ' + ', '.join(item['evidence']) + '\n')
            handles.extend(item['evidence'])
        parts.append(item['text'])  # Preserve every selected text/fence/sequence byte.
        parts.append('\n')
    destinations = list(dict.fromkeys(list(requirement_paths) +
        [path for handle in handles for path in (links or {}).get(handle, [])]))
    if destinations:parts.append('\nSource views:\n' + '\n'.join(destinations) + '\n')
    elif handles:parts.append('\nSource views: no exact prepared link for these citations.\n')
    return ''.join(parts)


def format_routes(routes, identity=None):
    if identity is None:
        return 'Routes:\n' + '\n'.join(r['id'] + ' | ' + r['title'] + ' | ' + r['narrative']['page'] for r in routes) + '\n'
    row = next(r for r in routes if r['id'] == identity)
    return 'Route: ' + identity + '\n' + json.dumps(row, ensure_ascii=False, indent=2) + '\n'


def window(text, arguments, offset=0, limit=8000):
    end = len(text) if limit == 0 else min(len(text), offset + limit)
    command = './docs show ' + shlex.join(arguments + ['--offset', str(end), '--limit', str(limit)]) if end < len(text) else None
    return {'text': text[offset:end], 'offset': offset, 'end': end, 'total': len(text),
            'remaining': len(text) - end, 'next': command}


def show(run, args, api):
    _, inventory, _ = api['frozen_inputs'](run)  # Pure pin reads; no runtime/receipts/locks.
    page = args.page[0] if len(args.page) == 1 else None
    families = bool(args.page) + bool(args.entries or args.entry) + bool(args.routes or args.route)
    api['need'](families == 1 and len(args.page) <= 1, 'SHOW_SELECTOR_REQUIRED',
                recovery='Use show --page PAGE [--fragment ID], show --entries [--service ID] [--entry ID], or show --routes [--route ID].')
    api['need'](not args.fragment or page, 'SHOW_FRAGMENT_NEEDS_PAGE')
    api['need'](not args.service or args.entries or args.entry, 'SHOW_SERVICE_NEEDS_ENTRIES')
    api['need'](args.offset >= 0 and args.limit >= 0, 'INVALID_SHOW_WINDOW', expected='nonnegative character offset/limit; limit 0 returns the full remainder')
    arguments = []
    views = {p.parent.name: api['load'](p) for p in (run / 'inputs/author-source').glob('*/views.json')}
    requirements = {p.parent.name: api['load'](p) for p in (run / 'inputs/author-source').glob('*/requirements.json')}
    links = source_links(views, requirements)
    if page:
        original = api['load'](run / 'workspace/draft/original.json')
        rows = {p['page']: p for p in original['pages']}
        api['need'](page in rows, 'SHOW_PAGE_NOT_FOUND', actual=page, available=sorted(rows))
        ids = [f['id'] for f in rows[page]['fragments'] if 'id' in f]
        api['need'](args.fragment is None or args.fragment in ids, 'SHOW_FRAGMENT_NOT_FOUND',
                    page=page, actual=args.fragment, available=ids)
        paths = ['../inputs/author-source/' + service + '/' + path
                 for service, records in requirements.items() for row in records.values()
                 if row.get('page') == page for path in row.get('views', [])] if args.fragment is None else []
        text = format_page(rows[page], args.fragment, links, paths)
        arguments = ['--page', page] + (['--fragment', args.fragment] if args.fragment else [])
    elif args.routes or args.route:
        path = run / 'workspace/draft/routes.json'
        rows = api['load'](path)['routes'] if path.exists() else []
        api['need'](args.route is None or args.route in [r['id'] for r in rows], 'SHOW_ROUTE_NOT_FOUND',
                    actual=args.route, available=[r['id'] for r in rows])
        text = format_routes(rows, args.route)
        arguments = ['--routes'] + (['--route', args.route] if args.route else [])
    else:
        accounts = api['W']['registered_accounts'](run, api)
        rows, discovered = api['W']['entry_registry'](inventory, accounts, api)
        api['need'](args.service is None or args.service in inventory['services'], 'SHOW_SERVICE_NOT_FOUND',
                    actual=args.service, available=list(inventory['services']))
        rows = [r for r in rows if args.service is None or r['service'] == args.service]
        api['need'](args.entry is None or args.entry in [r['id'] for r in rows], 'SHOW_ENTRY_NOT_FOUND',
                    actual=args.entry, available=[r['id'] for r in rows])
        arguments = ['--entries'] + (['--service', args.service] if args.service else [])
        if args.entry:
            row = next(r for r in rows if r['id'] == args.entry)
            declaration = next((r for r in discovered if r['requirement'] == args.entry), {})
            sources = requirements.get(row['service'], {}).get(row['id'], {})
            handles = declaration.get('evidence', sources.get('evidenceCandidates', []))
            # Internal SOURCE IDs are redundant with genuine handles/view links.
            item = {k: v for k, v in row.items() if k != 'sourceIds'}
            if declaration:item['declaration'] = declaration
            item['evidence'] = handles
            text = 'Entry: ' + args.entry + '\n' + json.dumps(item, ensure_ascii=False, indent=2) + '\n'
            paths = list(dict.fromkeys(
                ['../inputs/author-source/' + row['service'] + '/' + p for p in sources.get('views', [])] +
                [p for h in handles for p in links.get(h, [])]))
            if paths:text += '\nSource views:\n' + '\n'.join(paths) + '\n'
            else:text += '\nSource views: no exact prepared link for this entry.\n'
            arguments += ['--entry', args.entry]
        else:
            text = 'Entries:\n' + '\n'.join(r['id'] + ' | ' + r['entryKind'] + ' | ' + r['page'] +
                (' | ' + r['method'] + ' ' + r['path'] if r.get('method') else '') for r in rows) + '\n'
    api['need'](args.offset <= len(text), 'SHOW_OFFSET_OUT_OF_RANGE', actual=args.offset, total=len(text))
    return window(text, arguments, args.offset, args.limit)
