#!/usr/bin/env python3
"""Owned deterministic renderer/proof contracts; no model or native execution."""
import contextlib
import io
import json
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
E = runpy.run_path(str(ROOT / "skills/codeclew/scripts/reader_export_check.py"))
M = E["M"]
F = runpy.run_path(str(ROOT / "scripts/test_author_citations.py"))
GUIDE = "# Guide π🙂\n\n## 1. Read\n\nUse `limit <= 10` then read [details](guide.html#detail-1).\n\n### Detail\n\n- Keep one\n- Keep two\n\n### Detail\n\n| Field | Value |\n| --- | --- |\n| limit | 10 |\n\n[Home](index.html#start)\n"
GUIDE_HTML = '<!doctype html><main><h1 id="guide">Guide π🙂</h1><h2 id="1-read">1. Read</h2><p>Use <code>limit &lt;= 10</code> then read <a href="guide.html#detail-1">details</a>.</p><h3 id="detail">Detail</h3><ul><li>Keep one</li><li>Keep two</li></ul><h3 id="detail-1">Detail</h3><table><tr><th>Field</th><th>Value</th></tr><tr><td>limit</td><td>10</td></tr></table><a href="index.html#start">Home</a></main>'
INDEX = '# Start\n\n[Guide](guide.html#1-read)\n'
INDEX_HTML = '<!doctype html><main><h1 id="start">Start</h1><a href="guide.html#1-read">Guide</a></main>'


def sequence_fixture():
    # Use the real packaged producer, without npm, capture or a native build.
    return subprocess.run(['node', '--input-type=module', '-e', '''
import {renderSequence} from './skills/codeclew/examples/markdown-renderer/sequence.mjs';
console.log(renderSequence({title:'Accept reservation',participants:[
 {id:'client',title:'Client'},{id:'handler',title:'Reservation handler'}],steps:[
 {from:'client',to:'handler',kind:'call',label:'POST /reservations'},
 {from:'handler',to:'client',kind:'return',label:'Accepted'}]}));
'''], cwd=ROOT, check=True, capture_output=True, text=True).stdout


class SequencePresenceTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.diagram = sequence_fixture()

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.navigation = {'schema': 'codeclew-markdown-navigation/1', 'services': [
            {'starts': [{'kind': 'http', 'target': 'post.md#handler'},
                        {'kind': 'http', 'target': 'post.md#other'},
                        {'kind': 'scheduled', 'target': 'worker.md'}]}],
            'routes': [{'target': 'process.md'}]}
        self.nav = self.root / 'navigation.json'
        self.argv = ['node', 'render.mjs', '--navigation', str(self.nav)]
        self.pages = {'post.html': self.diagram.encode(), 'process.html': self.diagram.encode(),
                      'worker.html': b'<p>Worker overview</p>'}

    def check(self):
        self.nav.write_text(json.dumps(self.navigation))
        return E['check_sequences'](self.pages, self.argv)

    def test_real_diagrams_cover_http_and_shared_pages_without_requiring_every_overview(self):
        self.pages['process.html'] += self.diagram.encode()  # Split sequences allowed.
        result = self.check()
        self.assertEqual(result['status'], 'SEQUENCE_STRUCTURE_CHECKED')
        self.assertEqual(result['pages'], {'post.html': 1, 'process.html': 2})
        self.assertFalse(result['semanticCoverageAssessed']); self.assertFalse(result['browserBehaviorVerified'])

    def test_headings_lists_code_examples_and_diagram_on_another_page_do_not_cover_entry(self):
        for body in ['<h2>Handler-started sequence</h2><ol><li>Accept</li></ol>',
                     '<pre><code class="language-clew-sequence">{}</code></pre>',
                     '<pre>&lt;figure class="clew-sequence"&gt;Example&lt;/figure&gt;</pre>']:
            with self.subTest(body=body):
                self.pages['post.html'] = body.encode()
                with self.assertRaises(E['Stop']) as caught: self.check()
                self.assertEqual(caught.exception.status, 'REQUIRED_SEQUENCE_MISSING')
                self.assertEqual(caught.exception.details['page'], 'post.html')

    def test_missing_trace_empty_rows_or_mismatched_trace_are_not_complete(self):
        for body in [self.diagram.replace('class="sequence-text"', 'class="other"'),
                     self.diagram.replace('class="sequence-row"', 'class="other"'),
                     self.diagram.replace('</ol>', '<li>Extra</li></ol>'),
                     '<figure class="clew-sequence"><div class="sequence-diagram">'
                     '<div class="sequence-row"></div></div><details class="sequence-text">'
                     '<ol><li></li></ol></details></figure>']:
            with self.subTest(body=body[:70]):
                self.pages['post.html'] = body.encode()
                with self.assertRaises(E['Stop']) as caught: self.check()
                self.assertEqual(caught.exception.status, 'REQUIRED_SEQUENCE_INCOMPLETE')

    def test_local_registry_narrative_is_required_remote_narrative_is_not_fetched(self):
        registry = {'schema': 'codeclew-cross-docset-routes/1.1', 'routes': [
            {'narrative': {'docset': 'local', 'page': 'events.html'}},
            {'narrative': {'docset': 'peer', 'page': 'remote.html'}}]}
        path = self.root / 'routes.json'; path.write_text(json.dumps(registry))
        self.argv += ['--docset-routes', str(path), '--docset', 'local']
        with self.assertRaises(E['Stop']) as caught: self.check()
        self.assertEqual(caught.exception.status, 'SEQUENCE_TARGET_MISSING')
        self.pages['events.html'] = b'<p>Only prose</p>'
        with self.assertRaises(E['Stop']) as caught: self.check()
        self.assertEqual(caught.exception.status, 'REQUIRED_SEQUENCE_MISSING')
        self.pages['events.html'] = self.diagram.encode()
        result = self.check()
        self.assertEqual(set(result['pages']), {'post.html', 'process.html', 'events.html'})
        self.assertEqual(result['registryDigest'], E['sha'](path.read_bytes()))

    def test_missing_configuration_and_empty_selection_do_not_report_success(self):
        self.argv = []
        with self.assertRaises(E['Stop']) as caught: self.check()
        self.assertEqual(caught.exception.status, 'SEQUENCE_CONFIGURATION_MISSING')
        self.argv = ['--navigation', str(self.nav)]
        self.navigation.update(services=[], routes=[])
        with self.assertRaises(E['Stop']) as caught: self.check()
        self.assertEqual(caught.exception.status, 'SEQUENCE_REQUIRED_PAGES_EMPTY')


class ProcessSearchTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.registry = {
            'schema': 'codeclew-cross-docset-routes/1.1', 'revision': 'r1',
            'catalog': {'docset': 'docs', 'page': 'index.html'},
            'docsets': [{'id': 'docs', 'version': 'v1', 'baseURL': 'https://example.invalid/docs/',
                         'services': [{'id': s, 'overview': {'docset': 'docs', 'page': s + '.html'}} for s in ['a', 'b']]}],
            'routes': [{'id': 'request', 'narrative': {'docset': 'docs', 'page': 'process.html'},
                        'origin': {'status': 'unknown', 'reason': 'Not classified'},
                        'occurrences': [{'id': 'step', 'target': {'docset': 'docs', 'page': 'b.html', 'anchor': 'step'}}]}]}
        self.path = self.root / 'registry.json'
        self.argv = ['node', 'render.mjs', '{input}', '{output}', '--docset-routes', str(self.path), '--docset', 'docs', '--docset-version', 'v1']
        self.payload = {'registry': self.registry}
        self.controls = ''.join(f'<input type="checkbox" name="{field}" value="docs/{s}">' for field in ['start', 'end', 'intermediate'] for s in ['a', 'b'])
        self.controls += ''.join(f'<input type="checkbox" name="outcomeKinds" value="{kind}">' for kind in ['normal', 'error', 'handoff', 'unknown'])
        self.controls += '<input type="checkbox" name="includeUnknown" checked>'
        self.card = '<article class="route-card" data-route-id="request"><a href="process.html">Process</a><a href="b.html#step">Step</a></article>'

    def pages(self):
        self.path.write_text(json.dumps(self.registry))
        html = ('<section class="route-catalog" id="clew-route-catalog"><a href="a.html">A</a><a href="b.html">B</a>'
                '<form class="route-filters">' + self.controls + '</form>' + self.card + '</section>'
                '<script type="application/json" id="clew-route-data">' + json.dumps(self.payload) + '</script>'
                '<script>catalogClient(evaluate,"clew-route",savedFilters)</script>')
        return {'index.html': html.encode(), 'a.html': b'<h1>A</h1>', 'b.html': b'<h1 id="step">B</h1>', 'process.html': b'<h1>Request</h1>'}

    def refused(self, status, *, pages=None, argv=None):
        with self.assertRaises(E['Stop']) as caught:
            E['check_process_search'](self.pages() if pages is None else pages, self.argv if argv is None else argv)
        self.assertEqual(caught.exception.status, status)

    def test_actual_catalog_controls_registry_routes_and_links_preserve_unknown(self):
        result = E['check_process_search'](self.pages(), self.argv)
        self.assertEqual(result['status'], 'PROCESS_SEARCH_STRUCTURE_CHECKED')
        self.assertEqual(result['routes'], 1)
        self.assertFalse(result['browserBehaviorVerified'])
        self.assertFalse(result['remoteTargetsVerified'])
        self.assertEqual(self.payload['registry']['routes'][0]['origin']['status'], 'unknown')

    def test_packaged_catalog_passes_and_filters_one_outcome_at_a_time(self):
        self.registry['externalActors'] = []
        docset = self.registry['docsets'][0]
        docset['title'] = 'Docs'
        for service in docset['services']:
            service['title'] = service['id'].upper()
        def occurrence(identity, service, page, anchor=None):
            target = {'docset': 'docs', 'page': page}
            if anchor: target['anchor'] = anchor
            return {'id': identity, 'actor': {'kind': 'service', 'docset': 'docs', 'service': service},
                    'role': identity, 'target': target}
        route = self.registry['routes'][0]
        route.update(version='v1', title='Request',
                     origin={'status': 'known', 'occurrence': 'start', 'kind': 'http', 'title': 'Request', 'summary': 'Accept request'},
                     occurrences=[occurrence('start', 'a', 'a.html'), occurrence('step', 'b', 'b.html', 'step'),
                                  occurrence('done', 'b', 'b.html', 'step'), occurrence('failed', 'a', 'a.html')],
                     branches=[{'id': 'result', 'title': 'Result', 'condition': 'Request completes', 'target': route['narrative']}],
                     coverage={'outcomeInventory': 'complete-for-declared-scope', 'scope': 'Declared request', 'reference': route['narrative']})
        route['outcomes'] = [
            {'id': identity, 'branch': 'result', 'kind': kind, 'title': identity, 'target': route['narrative'],
             'internal': {'coverage': 'complete-for-this-outcome', 'occurrences': internal},
             'end': {'status': 'known', 'ownerOccurrence': owner, 'boundary': 'return', 'recipient': {'kind': 'none'}}}
            for identity, kind, internal, owner in [('ok', 'normal', ['step'], 'done'), ('bad', 'error', [], 'failed')]]
        script = '''
import {readFileSync} from 'node:fs';
import {validateNavigation} from './skills/codeclew/examples/markdown-renderer/navigation.mjs';
import {validateDocsetRoutes,filterRoutes} from './skills/codeclew/examples/markdown-renderer/docset-routes.mjs';
import {docsetView} from './skills/codeclew/examples/markdown-renderer/route-view.mjs';
const registry=JSON.parse(readFileSync(0,'utf8'));
const pages=[{file:'index.md',html:'<h1>Catalog</h1>'},{file:'a.md',html:'<h1>A</h1>'},
 {file:'b.md',html:'<h1 id="step">B</h1>'},{file:'process.md',html:'<h1>Request</h1>'}];
const nav=validateNavigation({schema:'codeclew-markdown-navigation/1',home:{title:'Docs',target:'index.md'},
 services:['a','b'].map(id=>({id,title:id,overview:id+'.md',pages:[id+'.md'],
 starts:id==='a'?[{title:'Request',kind:'http',summary:'Accept request',target:'a.md'}]:[],data:[]})),routes:[]},pages);
const model=validateDocsetRoutes(registry,nav,'docs','v1'),view=docsetView(model,'index.md');
const status=query=>filterRoutes(registry,query)[0].status;
const results=[status({start:['docs/a'],end:['docs/b'],intermediate:['docs/b'],outcomeKinds:['normal']}),
 status({end:['docs/b'],outcomeKinds:['error']}),status({end:['docs/a'],outcomeKinds:['error']}),
 status({intermediate:['docs/a']})];
registry.routes[0].outcomes[1].internal.coverage='partial';
results.push(status({intermediate:['docs/a'],outcomeKinds:['error']}));
console.log(JSON.stringify({html:view.hub+view.script,results}));
'''
        # The HTML payload must reflect the exact registry, before the separate
        # evaluator's deliberate partial-coverage mutation.
        generated = json.loads(subprocess.run(['node', '--input-type=module', '-e', script], cwd=ROOT,
            input=json.dumps(self.registry), check=True, capture_output=True, text=True).stdout)
        self.assertEqual(generated['results'], ['MATCH', 'NO_MATCH', 'MATCH', 'NO_MATCH', 'UNCLASSIFIED'])
        pages = self.pages()
        pages['index.html'] = generated['html'].encode()
        self.assertEqual(E['check_process_search'](pages, self.argv)['routes'], 1)
        for field in ['start', 'end', 'intermediate', 'outcomeKinds', 'includeUnknown']:
            with self.subTest(field=field):
                broken = dict(pages)
                broken['index.html'] = broken['index.html'].replace(('name="' + field + '"').encode(), b'name="removed"')
                self.refused('PROCESS_SEARCH_CONTROLS_MISSING', pages=broken)

    def test_legacy_service_links_are_not_process_search(self):
        self.refused('PROCESS_SEARCH_CONFIGURATION_MISSING', argv=['node', 'render.mjs', '--navigation', 'services.json'])
        self.refused('PROCESS_SEARCH_CATALOG_MISSING', pages={'index.html': b'<a href="a.html">A</a><a href="b.html">B</a>'})

    def test_missing_disabled_or_unscoped_controls_refuse(self):
        for change in [lambda p: p.replace(b'name="intermediate"', b'name="other"'),
                       lambda p: p.replace(b'name="end"', b'name="other"'),
                       lambda p: p.replace(b'value="error"', b'value="invalid"'),
                       lambda p: p.replace(b'name="includeUnknown" checked', b'name="includeUnknown"'),
                       lambda p: p.replace(b'name="start"', b'disabled name="start"'),
                       lambda p: p.replace(b'<form class=', b'<form hidden class='),
                       lambda p: p.replace(b'class="route-filters"', b'class="unrelated"')]:
            with self.subTest(change=change):
                pages = self.pages(); pages['index.html'] = change(pages['index.html'])
                self.refused('PROCESS_SEARCH_CONTROLS_MISSING', pages=pages)

    def test_empty_registry_and_missing_route_card_refuse(self):
        self.card = ''
        self.refused('PROCESS_SEARCH_ROUTES_MISSING')
        self.registry['routes'] = []
        self.refused('PROCESS_SEARCH_EMPTY')

    def test_payload_drift_and_missing_client_refuse(self):
        pages = self.pages(); pages['index.html'] = pages['index.html'].replace(b'"revision": "r1"', b'"revision": "r2"')
        self.refused('PROCESS_SEARCH_REGISTRY_MISMATCH', pages=pages)
        pages = self.pages(); pages['index.html'] = pages['index.html'].replace(b'catalogClient(', b'noClient(')
        self.refused('PROCESS_SEARCH_CLIENT_MISSING', pages=pages)

    def test_missing_narrative_step_or_service_link_refuse(self):
        for old, status in [(b'href="process.html"', 'PROCESS_SEARCH_ROUTE_LINK_MISSING'),
                            (b'href="b.html#step"', 'PROCESS_SEARCH_ROUTE_LINK_MISSING'),
                            (b'href="a.html"', 'PROCESS_SEARCH_SERVICE_LINK_MISSING')]:
            with self.subTest(old=old):
                pages = self.pages(); pages['index.html'] = pages['index.html'].replace(old, b'href="other.html"')
                self.refused(status, pages=pages)
        pages = self.pages(); pages['b.html'] = b'<h1 id="other">B</h1>'
        self.refused('PROCESS_SEARCH_TARGET_MISSING', pages=pages)

    def test_legacy_visits_cannot_replace_outcome_filters(self):
        self.controls = self.controls.replace('name="intermediate"', 'name="visits"')
        self.refused('PROCESS_SEARCH_CONTROLS_MISSING')

    def test_remote_catalog_declaration_cannot_pass_without_its_export(self):
        self.registry['catalog']['docset'] = 'peer'
        self.refused('PROCESS_SEARCH_LOCAL_CATALOG_REQUIRED')


class ReaderExportCheckTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name); self.serial = 0
        self.manifest, view, _ = F["fixture"](self.root)
        self.handle = view["records"][0]["cite"]
        self.authored = self.root / "authored"; self.authored.mkdir()
        self.renderer = self.root / "renderer.py"
        # Finite fixture renderer, not an ad hoc Markdown parser. Actual renderer
        # semantics remain caller-owned; the checker must not reinterpret them.
        self.renderer.write_text('''import json,sys
from pathlib import Path
source,destination,contract=map(Path,sys.argv[1:])
rows=json.loads(contract.read_text())
destination.mkdir()
for path in source.glob('*.md'):
    row=rows[path.name]
    if path.read_bytes().decode()!=row['markdown']:sys.exit(9)
    (destination/path.with_suffix('.html').name).write_text(row['html'])
if rows.get('_favicon') is not None:
    (destination/'favicon.svg').write_text(rows['_favicon'])
''')
        self.contract = self.root / "contract.json"
        self.command = self.root / "renderer.json"
        self.command.write_text(json.dumps([sys.executable, '-I', '-S', str(self.renderer), '{input}', '{output}', str(self.contract)]))
        self.prepare()

    def prepare(self, guide=GUIDE, html=GUIDE_HTML, index=INDEX, index_html=INDEX_HTML, favicon=None):
        self.serial += 1
        rows = {'guide.md': {'markdown': guide, 'html': html}, 'index.md': {'markdown': index, 'html': index_html}}
        self.contract.write_text(json.dumps({**rows, **({'_favicon': favicon} if favicon is not None else {})}))
        specs = []
        for name, row in rows.items():
            raw = row['markdown'].encode(); (self.authored / name).write_bytes(raw)
            specs.append({'page': name, 'markdownDigest': M['sha'](raw),
                          'claims': [{'id': 'owned', 'startByte': 0, 'endByte': len(raw), 'evidence': [self.handle]}]})
        request = self.root / f'request-{self.serial}.json'; request.write_text(json.dumps({'schema': M['REQUEST'], 'pages': specs}))
        self.package = self.root / f'bound-{self.serial}'
        M['bind']([self.manifest], self.authored, request, self.package)
        self.exported = self.root / f'exported-{self.serial}'
        subprocess.run([sys.executable, '-I', '-S', str(self.renderer), str(self.package / 'markdown'),
                        str(self.exported), str(self.contract)], check=True, capture_output=True)

    def verify(self):
        self.serial += 1; self.output = self.root / f'check-{self.serial}'
        return E['verify']([self.manifest], self.package / 'index.json', self.authored,
                           self.exported, self.command, self.output)

    def refused(self, status=None):
        with self.assertRaises(E['Stop']) as error:
            self.verify()
        if status:
            self.assertEqual(error.exception.status, status)

    def test_numbered_duplicate_headings_unicode_code_list_and_table_pass_unchanged(self):
        before = {str(p): p.read_bytes() for folder in (self.authored, self.package, self.exported, self.manifest.parent)
                  for p in folder.rglob('*') if p.is_file()}
        result = self.verify()
        self.assertEqual((result['status'], result['pages'], result['claims'], result['internalLinks']), ('EXPORT_CHECKED', 2, 2, 3))
        self.assertFalse(result['semanticTruthAssessed']); self.assertFalse(result['nativePublicationApproval'])
        self.assertEqual({p: Path(p).read_bytes() for p in before}, before)
        self.assertTrue((self.output / 'native-check.json').is_file())
        self.assertEqual(json.loads((self.output / 'expected.render.command.json').read_text())['exitCode'], 0)

    def test_explicit_search_requirement_blocks_legacy_export_without_changing_inputs(self):
        before = {p: p.read_bytes() for p in self.exported.iterdir()}
        with self.assertRaises(E['Stop']) as caught:
            E['verify']([self.manifest], self.package / 'index.json', self.authored,
                        self.exported, self.command, self.root / 'required-search', require_process_search=True)
        self.assertEqual(caught.exception.status, 'PROCESS_SEARCH_CONFIGURATION_MISSING')
        self.assertEqual({p: p.read_bytes() for p in self.exported.iterdir()}, before)

    def test_explicit_sequence_requirement_rejects_prose_only_export_through_cli(self):
        nav = self.root / 'navigation.json'
        nav.write_text(json.dumps({'schema': 'codeclew-markdown-navigation/1', 'services': [
            {'starts': [{'kind': 'http', 'target': 'guide.md'}]}], 'routes': []}))
        self.renderer.write_text(self.renderer.read_text().replace('sys.argv[1:]', 'sys.argv[1:4]'))
        self.command.write_text(json.dumps([*json.loads(self.command.read_text()), '--navigation', str(nav)]))
        before = {p: p.read_bytes() for p in self.exported.iterdir()}
        output = self.root / 'required-sequences'
        args = ['--manifest', str(self.manifest), '--index', str(self.package / 'index.json'),
                '--authored-root', str(self.authored), '--rendered-root', str(self.exported),
                '--renderer-command', str(self.command), '--output-dir', str(output), '--require-sequences']
        stream = io.StringIO()
        with contextlib.redirect_stdout(stream): self.assertEqual(E['main'](args), 2)
        result = json.loads(stream.getvalue())
        self.assertEqual(result['status'], 'REQUIRED_SEQUENCE_MISSING')
        self.assertEqual(result['details']['page'], 'guide.html')
        self.assertEqual(json.loads((output / 'result.json').read_text()), result)
        self.assertEqual({p: p.read_bytes() for p in self.exported.iterdir()}, before)

    def test_stale_outer_markdown_and_page_inventory_refuse(self):
        path = self.authored / 'guide.md'; path.write_text(GUIDE.replace('10', '11'))
        self.refused('AUTHORED_MARKDOWN_MISMATCH'); path.write_text(GUIDE)
        path.unlink(); self.refused('AUTHORED_PAGE_INVENTORY_MISMATCH'); path.write_text(GUIDE)
        (self.authored / 'extra.md').write_text('Extra')
        self.refused('AUTHORED_PAGE_INVENTORY_MISMATCH')

    def test_changed_number_operator_table_cell_or_removed_content_refuses(self):
        path = self.exported / 'guide.html'
        for old, new in [('1. Read', '2. Read'), ('&lt;=', '&gt;'), ('<td>10</td>', '<td>11</td>'),
                         ('<li>Keep two</li>', ''), ('<code>', '<span>')]:
            with self.subTest(old=old):
                path.write_text(GUIDE_HTML.replace(old, new)); self.refused('EXPORTED_HTML_MISMATCH')
        path.write_text(GUIDE_HTML)

    def test_missing_extra_or_symlinked_rendered_pages_refuse(self):
        path = self.exported / 'guide.html'; path.unlink()
        self.refused('EXPORT_PAGE_INVENTORY_MISMATCH'); path.write_text(GUIDE_HTML)
        extra = self.exported / 'stale.html'; extra.write_text('stale')
        self.refused('EXPORT_PAGE_INVENTORY_MISMATCH'); extra.unlink()
        path.unlink(); path.symlink_to(self.exported / 'index.html')
        self.refused('UNSUPPORTED_EXPORT_LAYOUT')

    def test_missing_local_link_and_fragment_refuse_even_when_render_matches(self):
        for bad, status in [('missing.html#x', 'EXPORT_LINK_TARGET_MISSING_OR_UNSUPPORTED'),
                            ('guide.html#missing', 'EXPORT_FRAGMENT_MISSING')]:
            with self.subTest(bad=bad):
                self.prepare(index=INDEX.replace('guide.html#1-read', bad),
                             index_html=INDEX_HTML.replace('guide.html#1-read', bad))
                self.refused(status)

    def test_external_urls_and_percent_encoded_targets_need_no_network(self):
        self.prepare(index=INDEX + '\n[Remote](https://example.invalid/x#missing)\n',
                     index_html=INDEX_HTML.replace('guide.html#1-read', '%67uide.html#%31-read') + '<a href="https://example.invalid/x#missing">Remote</a>')
        self.assertEqual(self.verify()['status'], 'EXPORT_CHECKED')

    def test_local_svg_favicon_is_byte_checked_without_allowing_asset_navigation(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>'
        icon = '<link rel="icon" type="image/svg+xml" href="favicon.svg">'
        self.prepare(html=GUIDE_HTML + icon, favicon=svg)
        self.assertEqual(self.verify()['status'], 'EXPORT_CHECKED')
        (self.exported / 'favicon.svg').write_text(svg.replace('M0 0', 'M1 1'))
        self.refused('EXPORTED_FAVICON_MISMATCH')
        self.prepare(html=GUIDE_HTML + icon + '<a href="favicon.svg">Not a page</a>', favicon=svg)
        self.refused('EXPORT_LINK_TARGET_MISSING_OR_UNSUPPORTED')
        self.prepare(html=GUIDE_HTML + '<link rel href="favicon.svg">', favicon=svg)
        self.refused('UNREFERENCED_EXPORT_FAVICON')
        self.prepare(html=GUIDE_HTML + '<a href="data:image/svg+xml;base64,PHN2Zy8+">Not navigation</a>')
        self.refused('UNSUPPORTED_EXPORT_LINK')
        for bad in ['not XML', '<html/>']:
            with self.subTest(bad=bad):
                self.prepare(html=GUIDE_HTML + icon, favicon=bad)
                self.refused('INVALID_EXPORT_FAVICON')
        self.prepare(html=GUIDE_HTML + icon)
        self.refused('EXPORT_FAVICON_MISSING')
        self.prepare(favicon=svg)
        self.refused('UNREFERENCED_EXPORT_FAVICON')

    def test_duplicate_ids_base_urls_and_unhandled_assets_refuse(self):
        for suffix, status in [('<p id="detail">duplicate</p>', 'AMBIGUOUS_EXPORT_ANCHOR'),
                               ('<a href="missing.html" href="index.html">ambiguous</a>', 'AMBIGUOUS_EXPORT_ATTRIBUTES'),
                               ('<base href="https://example.invalid">', 'UNSUPPORTED_EXPORT_BASE_URL'),
                               ('<img src="image.png">', 'EXPORT_LINK_TARGET_MISSING_OR_UNSUPPORTED')]:
            with self.subTest(status=status):
                self.prepare(html=GUIDE_HTML + suffix); self.refused(status)

    def test_native_proof_and_stale_map_refuse_before_renderer(self):
        proof = self.manifest.parent / 'part.stdout.json'; old = proof.read_bytes(); proof.write_bytes(old + b' ')
        with patch('subprocess.run', side_effect=AssertionError('No render before full native validation')):
            self.refused('REUSE_NATIVE_STREAM_DIGEST_MISMATCH')
        proof.write_bytes(old)
        mapped = self.package / 'maps/guide.md.json'; mapped.write_bytes(mapped.read_bytes() + b' ')
        with patch('subprocess.run', side_effect=AssertionError('No render before map reconstruction')):
            self.refused('PAGE_MAP_PROOF_MISMATCH')

    def test_renderer_errors_unsupported_argv_and_fresh_output_are_explicit(self):
        original = self.command.read_bytes(); self.command.write_text(json.dumps('not argv'))
        self.refused('INVALID_RENDERER_COMMAND'); self.command.write_bytes(original)
        self.contract.write_text('{}')
        self.refused('RENDERER_FAILED')
        self.assertTrue((self.output / 'expected.render.stderr').is_file())
        with self.assertRaises(E['Stop']) as error:
            E['verify']([self.manifest], self.package / 'index.json', self.authored, self.exported, self.command, self.output)
        self.assertEqual(error.exception.status, 'OUTPUT_ALREADY_EXISTS')

    def test_cli_failure_retains_report_and_does_not_overwrite_prior_output(self):
        (self.authored / 'guide.md').write_text('stale')
        output = self.root / 'cli-check'; stream = io.StringIO()
        argv = ['--manifest', str(self.manifest), '--index', str(self.package / 'index.json'),
                '--authored-root', str(self.authored), '--rendered-root', str(self.exported),
                '--renderer-command', str(self.command), '--output-dir', str(output)]
        with contextlib.redirect_stdout(stream):
            self.assertEqual(E['main'](argv), 2)
        before = (output / 'result.json').read_bytes()
        self.assertEqual(json.loads(before)['status'], 'AUTHORED_MARKDOWN_MISMATCH')
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(E['main'](argv), 2)
        self.assertEqual((output / 'result.json').read_bytes(), before)


class PageFinalizationTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name); self.serial = 0
        self.a, av, _ = F['fixture'](self.root, 'proof-a')
        self.b, bv, _ = F['fixture'](self.root, 'proof-b', service='other', work='work-b')
        self.ha, self.hb = av['records'][0]['cite'], bv['records'][0]['cite']
        self.mapping = self.root / 'repositories.json'
        self.mapping.write_text(json.dumps({service: {'repository': 'https://example.invalid/' + service,
            'urlTemplate': 'https://example.invalid/' + service + '/blob/{revision}/{path}#L{startLine}-L{endLine}'}
            for service in ['owned', 'other']}))
        self.renderer = self.root / 'renderer.py'
        self.renderer.write_text('''import json,sys
from pathlib import Path
source,destination,contract=map(Path,sys.argv[1:])
rows=json.loads(contract.read_text())
destination.mkdir()
for path in source.glob('*.md'):
    row=rows[path.name]
    if path.read_bytes().decode()!=row['markdown']:sys.exit(9)
    (destination/path.with_suffix('.html').name).write_text(row['html'])
''')
        self.contract = self.root / 'contract.json'; self.command = self.root / 'renderer.json'
        self.command.write_text(json.dumps([sys.executable, '-I', '-S', str(self.renderer), '{input}', '{output}', str(self.contract)]))
        self.original = {'pages': [
            {'page': 'guide.md', 'fragments': [{'id': 'guide', 'text': GUIDE + '[Proof](cite:' + self.ha + ')\r\n', 'evidence': [self.ha]},
                                            {'id': 'retired', 'text': 'Historical line.\r\n', 'evidence': [self.ha]}]},
            {'page': 'index.md', 'fragments': [{'id': 'index', 'text': INDEX, 'evidence': [self.ha]}]}]}
        self.package = self.compose(self.original)

    def compose(self, original):
        self.serial += 1; path = self.root / f'original-{self.serial}.json'; path.write_text(json.dumps(original, ensure_ascii=False))
        output = self.root / f'compose-{self.serial}'; M['compose'](path, output); return output

    def manual(self, package, label, *, previous=None, manifests=None, bad_link=False):
        output = self.root / label; shutil.copytree(package, output / 'compose')
        authored = output / 'compose/markdown'; bound = authored / '.codeclew/bound'
        manifests = manifests or [self.a]
        M['bind'](manifests, authored, output / 'compose/request.json', bound, previous=previous, repositories=self.mapping)
        (authored / 'sources.json').write_text(E['encode']({'checkedIndex': '.codeclew/bound/index.json'}))
        rows = {}
        for page in (bound / 'markdown').glob('*.md'):
            raw = page.read_bytes().decode()
            html = GUIDE_HTML if page.name == 'guide.md' else INDEX_HTML
            if '11' in raw: html = html.replace('10', '11')
            if bad_link: html += '<a href="missing.html">Broken</a>'
            rows[page.name] = {'markdown': raw, 'html': html}
        self.contract.write_text(json.dumps(rows, ensure_ascii=False))
        E['R']['run_renderer'](E['R']['renderer_command'](self.command), bound / 'markdown', output / 'html', output, 'manual')
        return output

    def finalize(self, *, package=None, manifests=None, previous=None, repositories=None, label=None):
        self.serial += 1; self.output = self.root / (label or f'final-{self.serial}')
        return E['finalize'](package or self.package, manifests or [self.a], self.command, self.output,
                             previous=previous, repositories=repositories)

    def snapshot(self, root):
        return {str(p.relative_to(root)): p.read_bytes() for p in root.rglob('*') if p.is_file()}

    def test_append_required_evidence_uses_explicit_rebind_and_finalizes_without_dropping_new_source(self):
        old_text = '# Guide\n\nThe owned fixture declares Ledger.\n'
        new_text = old_text + '\nThe other fixture includes π🙂.\n'
        original = {'pages': [{'page': 'guide.md', 'fragments': [
            {'id': 'guide', 'text': old_text, 'evidence': [self.ha]}]}]}
        base = self.compose(original)
        self.contract.write_text(json.dumps({'guide.md': {'markdown': old_text,
            'html': '<h1 id="guide">Guide</h1><p>The owned fixture declares Ledger.</p>'}}))
        initial = self.finalize(package=base, repositories=self.mapping)
        self.assertEqual(initial['status'], 'CHECKED_PAGES_FINALIZED')
        previous = self.output / initial['checkedIndex']
        revised = {'id': 'guide', 'text': new_text, 'evidence': [self.ha, self.hb]}
        edits = self.root / 'append-evidence.json'
        edits.write_text(json.dumps({'baseSHA256': M['sha']((base/'original.json').read_bytes())[7:],
            'edits': [{'page': 'guide.md', 'start': 0, 'delete': 1, 'fragments': [revised]}]}))
        with self.assertRaises(M['Stop']) as rejected:
            M['update_original'](base/'original.json', edits, self.root/'rejected-text-update')
        self.assertEqual(rejected.exception.status, 'ORIGINAL_BINDINGS_CHANGED')
        details = rejected.exception.details
        self.assertEqual(details['changes'], [{'page': 'guide.md', 'id': 'guide',
            'previousEvidence': [self.ha], 'requestedEvidence': [self.ha, self.hb]}])
        self.assertEqual(details['recovery']['pageMetadata'], [{'page': 'guide.md', 'replace': ['guide'], 'remove': []}])
        original['pages'][0]['fragments'] = [revised]
        original['pages'][0]['replace'] = details['recovery']['pageMetadata'][0]['replace']
        updated = self.compose(original)
        self.contract.write_text(json.dumps({'guide.md': {'markdown': new_text,
            'html': '<h1 id="guide">Guide</h1><p>The owned fixture declares Ledger.</p><p>The other fixture includes π🙂.</p>'}}))
        result = self.finalize(package=updated, manifests=[self.a, self.b], previous=previous, repositories=self.mapping)
        self.assertEqual((result['status'], result['exportStatus']), ('CHECKED_PAGES_FINALIZED', 'EXPORT_CHECKED'))
        index_path = self.output/result['checkedIndex']; index = json.loads(index_path.read_bytes())
        mapped = json.loads((index_path.parent/index['pages'][0]['map']).read_bytes())
        self.assertEqual(mapped['claims'][0]['evidence'], [self.ha, self.hb])
        self.assertEqual((self.output/result['authoredRoot']/'guide.md').read_text(), new_text)
        original['pages'][0]['fragments'][0]['evidence'] = [self.ha, 'm000000000000:e1']
        foreign = self.compose(original)
        with self.assertRaises(M['Stop']):
            M['bind']([self.a, self.b], foreign/'markdown', foreign/'request.json', self.root/'foreign-refused',
                      previous=previous, repositories=self.mapping)

    def test_initial_and_update_match_manual_native_maps_checked_html_and_pointer(self):
        initial = self.manual(self.package, 'manual-initial')
        manual_check = E['verify']([self.a], initial / 'compose/markdown/.codeclew/bound/index.json',
            initial / 'compose/markdown', initial / 'html', self.command, initial / 'check')
        before = self.snapshot(self.package)
        result = self.finalize(repositories=self.mapping); first = self.output
        self.assertEqual(result['status'], 'CHECKED_PAGES_FINALIZED'); self.assertEqual(result['exportStatus'], 'EXPORT_CHECKED')
        self.assertEqual(self.snapshot(first / 'compose'), self.snapshot(initial / 'compose'))
        self.assertEqual(self.snapshot(first / 'html'), self.snapshot(initial / 'html'))
        self.assertEqual(result['claims'], manual_check['claims']); self.assertEqual(self.snapshot(self.package), before)
        self.assertEqual((first / result['rendererCommand']).read_bytes(), self.command.read_bytes())
        pointer = json.loads((first / result['sourcesJson']).read_bytes())
        self.assertEqual((first / result['authoredRoot'] / pointer['checkedIndex']).resolve(), (first / result['checkedIndex']).resolve())
        self.assertNotIn('cite:', (first / 'compose/markdown/.codeclew/bound/markdown/guide.md').read_text())
        self.assertIn('cite:', (first / 'compose/markdown/guide.md').read_text())
        self.assertFalse(result['semanticTruthAssessed']); self.assertTrue(result['visualReviewRequired'])
        for stage in ['bind', 'export-check']:
            self.assertEqual(json.loads((first / 'receipts' / (stage + '.command.json')).read_bytes())['exitCode'], 0)
        changed = json.loads(json.dumps(self.original)); guide = changed['pages'][0]
        guide['fragments'][0].update(text=guide['fragments'][0]['text'].replace('10', '11'), evidence=[self.hb])
        guide['fragments'][0]['text'] = guide['fragments'][0]['text'].replace(self.ha, self.hb)
        guide['fragments'][1] = {'id': 'added', 'text': 'New explicit explanation.\r\n', 'evidence': [self.hb]}
        guide.update(remove=['retired'], replace=['guide'])
        update = self.compose(changed); old = first / result['checkedIndex']
        manual = self.manual(update, 'manual-update', previous=old, manifests=[self.a, self.b])
        result = self.finalize(package=update, manifests=[self.a, self.b], previous=old)
        self.assertEqual(result['status'], 'CHECKED_PAGES_FINALIZED')
        self.assertEqual(self.snapshot(self.output / 'compose'), self.snapshot(manual / 'compose'))
        self.assertEqual(self.snapshot(self.output / 'html'), self.snapshot(manual / 'html'))
        mapped = json.loads((self.output / 'compose/markdown/.codeclew/bound/maps/guide.md.json').read_bytes())
        self.assertEqual([c['id'] for c in mapped['claims']], ['guide', 'added'])
        self.assertTrue(all(c['sources'][0]['work'] == 'work-b' for c in mapped['claims']))

    def test_failed_bind_stops_before_render_and_retains_full_native_failure(self):
        self.manual(self.package, 'manual')
        proof = self.a.parent / 'part.stdout.json'; proof.write_bytes(proof.read_bytes() + b' ')
        before = self.snapshot(self.package)
        result = self.finalize(repositories=self.mapping)
        self.assertEqual((result['status'], result['stage']), ('FINALIZATION_FAILED', 'bind'))
        receipt = json.loads((self.output / 'receipts/bind.command.json').read_bytes())
        self.assertEqual(receipt['exitCode'], 2)
        native = json.loads((self.output / 'receipts/bind.stdout').read_bytes())
        self.assertEqual(native['status'], 'REUSE_NATIVE_STREAM_DIGEST_MISMATCH')
        self.assertTrue((self.output / 'receipts/bind.stderr').is_file())
        self.assertFalse((self.output / 'receipts/render.render.command.json').exists())
        self.assertEqual(self.snapshot(self.package), before)

    def test_failed_renderer_stops_before_export_with_stdout_stderr_and_exit(self):
        self.manual(self.package, 'manual')
        self.renderer.write_text('import sys;print("render-out");print("render-error",file=sys.stderr);sys.exit(7)')
        result = self.finalize(repositories=self.mapping)
        self.assertEqual((result['stage'], result['stageStatus']), ('render', 'RENDERER_FAILED'))
        self.assertEqual(json.loads((self.output / 'receipts/render.render.command.json').read_bytes())['exitCode'], 7)
        self.assertEqual((self.output / 'receipts/render.render.stdout').read_bytes(), b'render-out\n')
        self.assertEqual((self.output / 'receipts/render.render.stderr').read_bytes(), b'render-error\n')
        self.assertFalse((self.output / 'receipts/export-check.command.json').exists())

    def test_failed_full_export_keeps_native_and_renderer_receipts_and_no_success(self):
        self.manual(self.package, 'manual', bad_link=True)
        result = self.finalize(repositories=self.mapping)
        self.assertEqual((result['status'], result['stage']), ('FINALIZATION_FAILED', 'export-check'))
        checked = json.loads((self.output / 'check/result.json').read_bytes())
        self.assertEqual(checked['status'], 'EXPORT_LINK_TARGET_MISSING_OR_UNSUPPORTED')
        self.assertTrue((self.output / 'check/native-check.json').is_file())
        self.assertTrue((self.output / 'check/expected.render.command.json').is_file())
        self.assertEqual(json.loads((self.output / 'receipts/export-check.command.json').read_bytes())['exitCode'], 2)

    def test_update_missing_explicit_intent_and_changed_reviewed_package_fail_closed(self):
        self.manual(self.package, 'manual')
        first = self.finalize(repositories=self.mapping); old = self.output / first['checkedIndex']
        changed = json.loads(json.dumps(self.original)); changed['pages'][0]['fragments'].pop()
        update = self.compose(changed)
        result = self.finalize(package=update, previous=old)
        self.assertEqual(result['stage'], 'bind')
        self.assertEqual(json.loads((self.output / 'receipts/bind.stdout').read_bytes())['status'], 'EXPLICIT_CLAIM_REMOVAL_OR_REBIND_REQUIRED')
        (self.package / 'markdown/guide.md').write_text('Human edit')
        before = self.snapshot(self.package); result = self.finalize(repositories=self.mapping)
        self.assertEqual(result['stageStatus'], 'COMPOSE_PACKAGE_CHANGED')
        self.assertEqual(self.snapshot(self.package), before)
        self.assertFalse((self.output / 'receipts/bind.command.json').exists())

    def test_cli_success_failure_existing_output_and_overlap_preserve_inputs(self):
        self.manual(self.package, 'manual'); output = self.root / 'cli'; before = self.snapshot(self.package)
        args = ['finalize', '--compose-root', str(self.package), '--manifest', str(self.a), '--renderer-command', str(self.command),
                '--repositories', str(self.mapping), '--output-dir', str(output)]
        stream = io.StringIO()
        with contextlib.redirect_stdout(stream): self.assertEqual(E['main'](args), 0)
        self.assertEqual(json.loads(stream.getvalue()), json.loads((output / 'result.json').read_bytes()))
        (output / 'sentinel').write_bytes(b'Human'); old = self.snapshot(output)
        with contextlib.redirect_stdout(io.StringIO()): self.assertEqual(E['main'](args), 2)
        self.assertEqual(self.snapshot(output), old); self.assertEqual(self.snapshot(self.package), before)
        with self.assertRaises(E['Stop']) as error:
            E['finalize'](self.package, [self.a], self.command, self.package / 'inside', repositories=self.mapping)
        self.assertEqual(error.exception.status, 'FINALIZATION_OUTPUT_OVERLAPS_INPUT')
        self.assertFalse((self.package / 'inside').exists())

    def test_exit_zero_without_checked_status_is_not_success(self):
        receipts = self.root / 'receipts'; receipts.mkdir()
        with self.assertRaises(E['Stop']) as error:
            E['checked_command']([sys.executable, '-c', 'print(\'{"status":"NEEDS_REPAIR"}\')'], receipts, 'native', 'PAGE_MAPS_BOUND')
        self.assertEqual(error.exception.status, 'FINALIZATION_STAGE_NOT_CHECKED')
        self.assertEqual(json.loads((receipts / 'native.command.json').read_bytes())['exitCode'], 0)

    def test_renderer_cannot_silently_change_the_supplied_command_copy(self):
        self.manual(self.package, 'manual')
        with self.renderer.open('a') as stream:
            stream.write("\n(source.parents[4]/'renderer.json').write_text('[]')\n")
        original = self.command.read_bytes(); result = self.finalize(repositories=self.mapping)
        self.assertEqual((result['status'], result['stageStatus']), ('FINALIZATION_FAILED', 'EXPORT_INPUT_CHANGED'))
        self.assertEqual(self.command.read_bytes(), original)
        self.assertFalse((self.output / 'receipts/export-check.command.json').exists())

    def test_required_process_search_stops_finalization_before_success(self):
        self.manual(self.package, 'manual')
        output = self.root / 'required-process-search'
        result = E['finalize'](self.package, [self.a], self.command, output,
                               repositories=self.mapping, require_process_search=True)
        self.assertEqual((result['status'], result['stage'], result['stageStatus']),
                         ('FINALIZATION_FAILED', 'process-search', 'PROCESS_SEARCH_CONFIGURATION_MISSING'))
        self.assertFalse((output / 'receipts/export-check.command.json').exists())

    def test_required_sequence_finalization_blocks_lists_then_passes_real_diagram_and_recheck(self):
        self.manual(self.package, 'manual')
        nav = self.root / 'navigation.json'
        nav.write_text(json.dumps({'schema': 'codeclew-markdown-navigation/1', 'services': [
            {'starts': [{'kind': 'http', 'target': 'guide.md'}]}], 'routes': []}))
        self.renderer.write_text(self.renderer.read_text().replace('sys.argv[1:]', 'sys.argv[1:4]'))
        self.command.write_text(json.dumps([*json.loads(self.command.read_text()), '--navigation', str(nav)]))
        before = self.snapshot(self.package)
        output = self.root / 'no-sequences'
        result = E['finalize'](self.package, [self.a], self.command, output,
                               repositories=self.mapping, require_sequences=True)
        self.assertEqual((result['status'], result['stage'], result['stageStatus']),
                         ('FINALIZATION_FAILED', 'sequences', 'REQUIRED_SEQUENCE_MISSING'))
        self.assertFalse((output / 'receipts/export-check.command.json').exists())
        contract = json.loads(self.contract.read_text())
        contract['guide.md']['html'] += sequence_fixture()
        self.contract.write_text(json.dumps(contract))
        output = self.root / 'with-sequences'
        result = E['finalize'](self.package, [self.a], self.command, output,
                               repositories=self.mapping, require_sequences=True)
        self.assertEqual(result['status'], 'CHECKED_PAGES_FINALIZED')
        self.assertEqual(result['sequences']['pages'], {'guide.html': 1})
        check = json.loads((output / 'check/result.json').read_text())
        self.assertEqual(check['sequences'], result['sequences'])
        self.assertIn('--require-sequences', json.loads((output / 'receipts/export-check.command.json').read_text())['argv'])
        self.assertEqual(self.snapshot(self.package), before)


if __name__ == '__main__':
    unittest.main()
