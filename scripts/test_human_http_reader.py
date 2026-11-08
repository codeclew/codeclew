#!/usr/bin/env python3
"""Owned SOURCE_SYNTAX protocol fixture through real compose/bind/render/export.

No capture, model call, application execution or semantic approval is implied.
Use --renderer DIR (an npm-installed copy), optionally --output-dir NEW to retain
the local fixture and receipts for independent desktop/narrow acceptance.
"""
import argparse
import json
from pathlib import Path
import runpy
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
E = runpy.run_path(str(ROOT / 'skills/codeclew/scripts/reader_export_check.py'))
F = runpy.run_path(str(ROOT / 'scripts/test_author_citations.py'))


def build(root, renderer):
    root.mkdir(parents=True, exist_ok=False)
    source = (ROOT / 'scripts/fixtures/human-http-reader/Intake.java').read_text()
    record = {**F['source'](source), 'file': 'Intake.java', 'startLine': 1,
              'endLine': len(source.splitlines())}
    declaration = F['declaration']()
    declaration['record']['symbol'] = 'class:Intake'
    with patch.dict(F['fixture'].__globals__, {'declaration': lambda: declaration}):
        manifest, view, _ = F['fixture'](root, record=record)
    handle = view['records'][0]['cite']
    (root / 'view.json').write_text(json.dumps(view))
    original = {'pages': []}

    def page(name, fragments):
        original['pages'].append({'page': name, 'fragments': fragments})

    def plain(text):
        return {'text': text}

    def bound(identity, text):
        return {'id': identity, 'text': text, 'evidence': [handle]}

    page('index.md', [plain('# Item intake\n\n'), bound('entries',
        '| HTTP entry | Handler | Thread |\n| --- | --- | --- |\n'
        '| [POST /v1/items](post.md#handler) | `Intake.postV1` | [accept(false)](process.md#v1) |\n'
        '| [PUT /v2/items](put.md#handler) | `Intake.putV2` | [accept(true)](process.md#v2) |\n\n'),
        plain('[Objects](domain.md) · [Integrations](integrations.md)\n')])
    page('process.md', [plain('# Shared intake process\n\n## V1\n\n'), bound('v1',
        '[POST entry](post.md#handler) passes `false`; it does not send an audit message.\n\n'),
        plain('## V2\n\n'), bound('v2',
        '[PUT entry](put.md#handler) validates positive quantity and passes `true`.\n\n'),
        plain('## Flag\n\n'), bound('flag',
        '`accept` → `flag` → `Config.get("stock.check", false)`. When disabled, skip stock and return `checked=false`.\n\n'),
        plain('## Check\n\n'), bound('check',
        'When enabled, `checkStock` calls Stock with `itemId` and `quantity`; its boolean becomes `checked`. '
        'A thrown config/stock error propagates before the response and before audit.\n\n'),
        plain('## Notify\n\n'), bound('notify',
        'Only v2 with `!isTest(request)` sends `items.accepted`; `isTest` returns `request.test()`. '
        '`Audit.send` returns a CompletionStage that this source never awaits. A synchronous throw prevents the response; '
        'a later failed stage is not observed here. No broker ACK or retry is proved.\n\n')])
    page('integrations.md', [plain('# Integrations\n\n## Config\n\n'), bound('config',
        'Config provider: `stock.check`, default `false`; physical provider/override unavailable.\n\n'),
        plain('## Stock\n\n'), bound('stock',
        'Stock API: `check(itemId, quantity)` returns a boolean. Address/protocol not retained.\n\n'),
        plain('## Audit\n\n'), bound('audit',
        'Audit producer: `items.accepted` with the Request payload; returns CompletionStage. '
        'Physical broker and delivery guarantees unavailable.\n\n')])
    page('domain.md', [plain('# Item intake objects\n\n'), bound('fields',
        'An intake request identifies an item and the requested quantity; a result reports that identity and stock decision.\n\n'
        '| Field | Meaning |\n| --- | --- |\n| `itemId` | Requested item identity. |\n'
        '| `quantity` | Requested number of items; v2 rejects values below one. |\n'
        '| `test` | Suppresses v2 audit when true. |\n| `checked` | Stock result, or false when the flag is disabled. |\n\n'),
        plain('## Representations\n\n'), bound('types',
        '`Intake.Request(itemId, quantity, test)` and `Intake.Result(itemId, checked)` are separate representations. '
        'No database entity or alias mapping is established.\n')])
    for version, method, file, handler in [(1, 'POST', 'post.md', 'postV1'), (2, 'PUT', 'put.md', 'putV2')]:
        actors = [{'id': 'client', 'title': 'Client'},
                  {'id': 'http', 'title': 'Intake.' + handler, 'target': file + '#handler'},
                  {'id': 'local', 'title': 'Intake.accept', 'target': 'process.md#v' + str(version)},
                  {'id': 'config', 'title': 'Config', 'target': 'integrations.md#config'},
                  {'id': 'stock', 'title': 'Stock API', 'target': 'integrations.md#stock'}]
        if version == 2:
            actors.append({'id': 'audit', 'title': 'Audit', 'target': 'integrations.md#audit'})
        steps = []

        def step(sender, receiver, kind, label, target=None, condition=None):
            value = {'from': sender, 'to': receiver, 'kind': kind, 'label': label}
            if target:
                value['target'] = target
            if condition:
                value['condition'] = condition
            steps.append(value)

        step('client', 'http', 'call', f'{method} /v{version}/items', file + '#handler')
        if version == 2:
            step('http', 'http', 'note', 'Reject quantity below one', file + '#handler', 'quantity < 1; throw before shared work')
        step('http', 'local', 'call', 'accept(request, ' + str(version == 2).lower() + ')', 'process.md#v' + str(version), 'Entry validation succeeds')
        step('local', 'config', 'call', 'flag → get stock.check, default false', 'process.md#flag')
        step('config', 'local', 'return', 'Configured boolean', 'process.md#flag', 'Provider succeeds')
        step('local', 'local', 'note', 'checkStock helper', 'process.md#check', 'Flag enabled')
        step('local', 'stock', 'call', 'check(itemId, quantity)', 'process.md#check', 'Flag enabled')
        step('stock', 'local', 'return', 'checked boolean', 'process.md#check', 'Flag enabled and check succeeds')
        if version == 2:
            step('local', 'local', 'note', 'Read request.test through isTest', 'process.md#notify')
            step('local', 'audit', 'async', 'send items.accepted with Request', 'process.md#notify', 'test=false; no await of returned stage')
        step('local', 'http', 'return', 'Result(itemId, checked)', file + '#outcome', 'All immediate calls succeed; disabled flag keeps checked=false')
        step('http', 'client', 'return', 'HTTP handler result', file + '#outcome', 'Success; HTTP status mapping not retained')
        fragments = [plain(f'# {method} /v{version}/items\n\n## Handler\n\n'),
                     bound('input', f'`Intake.{handler}` accepts Request fields `itemId`, `quantity`, `test`. '
                           + ('V2 rejects quantity below one before shared work.\n\n' if version == 2 else 'V1 adds no quantity validation here.\n\n')),
                     plain('## Sequence\n\n```clew-sequence\n'),
                     bound('actors', json.dumps({'title': f'{method} /v{version}/items', 'participants': actors})[:-1] + ',"steps":[\n')]
        fragments.extend(bound('step-' + str(i + 1), json.dumps(s) + (',' if i + 1 < len(steps) else '') + '\n') for i, s in enumerate(steps))
        fragments.extend([plain(']}\n```\n\n## Outcome\n\n'), bound('outcome',
            'Returns Result only after immediate calls complete. Config/stock exceptions propagate; there is no catch or retry here. '
            + ('Audit is skipped for test requests; an immediate producer throw prevents Result. Later stage completion is not awaited.\n\n' if version == 2 else 'This entry never sends audit.\n\n')),
            plain('[Complete shared process](process.md#v' + str(version) + ') · [All HTTP entries](index.md)\n\n<details><summary>Source and scope</summary>\n\n'),
            bound('source', '[Source](cite:' + handle + ')\n\nSOURCE_SYNTAX fixture; no runtime/deployment verification.\n\n'), plain('</details>\n')])
        page(file, fragments)
    original_path = root / 'original.json'
    original_path.write_text(json.dumps(original))
    E['M']['compose'](original_path, root / 'compose')
    navigation = {'schema': 'codeclew-markdown-navigation/1', 'home': {'title': 'Intake', 'target': 'index.md'},
                  'services': [{'id': 'intake', 'title': 'Intake', 'overview': 'index.md',
                                'pages': [p['page'] for p in original['pages']],
                                'starts': [{'title': method + ' /v' + str(v) + '/items', 'target': file + '#handler', 'kind': 'http', 'summary': summary}
                                           for v, method, file, summary in [(1, 'POST', 'post.md', 'Accept without audit.'), (2, 'PUT', 'put.md', 'Validate and conditionally notify.')]],
                                'data': [{'title': 'Objects', 'target': 'domain.md'}]}], 'routes': []}
    (root / 'navigation.json').write_text(json.dumps(navigation))
    (root / 'renderer.json').write_text(json.dumps(['node', str(renderer / 'render.mjs'), '{input}', '{output}', 'Owned HTTP fixture', '--navigation', str(root / 'navigation.json')]))
    mapping = root / 'repositories.json'
    mapping.write_text(json.dumps({'owned': {'repository': 'https://example.invalid/owned', 'urlTemplate': 'https://example.invalid/owned/blob/{revision}/{path}#L{startLine}-L{endLine}'}}))
    result = E['finalize'](root / 'compose', [manifest], root / 'renderer.json', root / 'final', repositories=mapping,
                           source_footers=True, source_views=[root / 'view.json'])
    assert result['status'] == 'CHECKED_PAGES_FINALIZED', result
    assert result['exportStatus'] == 'EXPORT_CHECKED', result
    assert result['semanticTruthAssessed'] is False
    assert (root / 'final' / result['sourceFooterBundle']).is_file()
    bound_root = root / 'final' / Path(result['checkedIndex']).parent
    index = json.loads((root / 'final' / result['checkedIndex']).read_text())
    for file in ['post.md', 'put.md']:
        body = (bound_root / 'markdown' / file).read_text()
        assert 'cite:' not in body and 'https://example.invalid/owned/blob/' in body
        page_map = json.loads((bound_root / next(p['map'] for p in index['pages'] if p['page'] == file)).read_text())
        assert any(c['id'] == 'actors' for c in page_map['claims'])
        assert sum(c['id'].startswith('step-') for c in page_map['claims']) >= 9
    html = root / 'final/html'
    for file, count in [('post.html', 9), ('put.html', 12)]:
        content = (html / file).read_text()
        assert content.count('class="sequence-direction"') == count
        assert 'language-clew-sequence' not in content and 'class="clew-sequence"' in content
        assert 'class="page-sources"' in content and 'class="source-fragments"' in content
        assert 'class="sequence-message await' not in content
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--renderer', required=True, type=Path)
    parser.add_argument('--output-dir', type=Path)
    args = parser.parse_args()
    if args.output_dir:
        print(json.dumps(build(args.output_dir.resolve(), args.renderer.resolve())))
    else:
        with tempfile.TemporaryDirectory() as directory:
            print(json.dumps(build(Path(directory) / 'run', args.renderer.resolve())))
