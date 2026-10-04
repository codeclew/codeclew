#!/usr/bin/env python3
"""Check site reachability, search destinations and vector asset integrity."""
from html.parser import HTMLParser
import hashlib
import json
from pathlib import Path
import re
import tempfile
import unittest
from urllib.parse import unquote, urlsplit
import xml.etree.ElementTree as ET

SITE = Path(__file__).resolve().parents[1] / 'site'
GENERATED_DOCS = SITE / 'examples/codeclew-source/docs'


class Page(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.ids = []
        self.resources = []
        self.links = []
        self.references = []
        self.dynamic_fragments = False
        self.navigation = {}
        self.nav = None
        self.feed(path.read_text())

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        self.references.extend(attrs[key] for key in ('href', 'src') if key in attrs)
        if tag == 'script' and attrs.get('id') == 'document-data' and attrs.get('type') == 'application/json':
            # These fragment routes are materialized by the generated reader,
            # not static anchors. Reader tests/browser checks own their behavior.
            self.dynamic_fragments = True
        if 'id' in attrs:
            self.ids.append(attrs['id'])
        if tag == 'nav':
            self.nav = attrs.get('aria-label')
            self.navigation[self.nav] = []
        if tag == 'a' and 'href' in attrs:
            self.links.append(attrs['href'])
            if self.nav:
                self.navigation[self.nav].append(attrs['href'])
        if tag in ('img', 'script') and 'src' in attrs:
            self.resources.append(attrs['src'])
        if tag == 'link' and 'href' in attrs:
            self.resources.append(attrs['href'])

    def handle_endtag(self, tag):
        if tag == 'nav':
            self.nav = None


def assert_local_destination(case, site, pages, source, url):
    target = urlsplit(url)
    if target.scheme or target.netloc:
        return
    path = (source.parent / unquote(target.path)).resolve() if target.path else source.resolve()
    case.assertTrue(path.is_relative_to(site.resolve()), f'{source}: destination escapes site: {url}')
    if path.is_dir():
        path = path / 'index.html'
    case.assertTrue(path.is_file(), f'{source}: missing {url}')
    if target.fragment and path.suffix == '.html':
        page = pages.get(path)
        if page is None:
            page = Page(path)
            pages[path] = page
        if not page.dynamic_fragments:
            case.assertIn(unquote(target.fragment), page.ids, f'{source}: missing fragment {url}')


def archived_root_context(case, root, path):
    """Admit the native root-output role, not a filename-based link exemption."""
    relative = path.relative_to(root)
    case.assertEqual(len(relative.parts), 3, f'{path}: invalid archived root placement')
    folder, bundle, filename = relative.parts
    case.assertTrue(folder == 'generated' and filename == 'root-overview.html'
                    and re.fullmatch(r'[0-9a-f]{64}', bundle),
                    f'{path}: invalid archived root placement')
    manifest_path = path.parent / 'publication.json'
    bindings_path = path.parent / 'bindings.json'
    case.assertTrue(manifest_path.is_file() and bindings_path.is_file(),
                    f'{path}: missing native root-output records')
    manifest = json.loads(manifest_path.read_text())
    bindings_bytes = bindings_path.read_bytes()
    bindings = json.loads(bindings_bytes)
    digest = lambda data: 'sha256:' + hashlib.sha256(data).hexdigest()
    case.assertEqual(manifest.get('schema'), 'codeclew-documentation-publication/1.0')
    case.assertEqual(manifest.get('id'), bundle, f'{path}: publication identity mismatch')
    case.assertIn(bindings.get('schema'), ('codeclew-documentation-bindings/1.4',
                                         'codeclew-documentation-bindings/1.5'))
    files = manifest.get('files', {})
    case.assertEqual(files.get('bindings.json'), digest(bindings_bytes),
                     f'{path}: bindings digest mismatch')
    root_digest = digest(path.read_bytes())
    case.assertEqual(files.get('root-overview.html'), root_digest,
                     f'{path}: archived root digest mismatch')
    case.assertEqual(bindings.get('outputHashes', {}).get('root-overview.html'), root_digest,
                     f'{path}: root-output binding mismatch')
    # Native commit deploys these bytes at this path. Historical templates keep
    # their own self-anchor IDs, even when the current root points at a new bundle.
    return root / 'index.html'


def assert_generated_documentation(case, site, root):
    site = site.resolve()
    root = root.resolve()
    pages = {path.resolve(): Page(path) for path in root.rglob('*.html')}
    case.assertTrue(pages, f'{root}: no generated HTML pages')
    archived_roots = []
    for path, page in list(pages.items()):
        case.assertEqual(len(page.ids), len(set(page.ids)), f'{path}: duplicate fragment IDs')
        context = path
        destinations = pages
        if path.name == 'root-overview.html':
            context = archived_root_context(case, root, path)
            archived_roots.append(path)
            destinations = dict(pages)
            destinations[context] = page
        for url in page.references:
            with case.subTest(page=path.relative_to(site), url=url):
                assert_local_destination(case, site, destinations, context, url)
    if archived_roots:
        deployed = root / 'index.html'
        case.assertTrue(deployed.is_file(), 'native root output was not deployed')
        case.assertTrue(any(path.read_bytes() == deployed.read_bytes() for path in archived_roots),
                        'deployed root differs from every verified native root output')


class SiteTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.pages = {path.name: Page(path) for path in SITE.glob('*.html')}
        cls.destination_pages = {(SITE / name).resolve(): page for name, page in cls.pages.items()}

    def assert_destination(self, source, url):
        assert_local_destination(self, SITE, self.destination_pages, SITE / source, url)

    def test_local_links_and_assets_resolve(self):
        for name, page in self.pages.items():
            for url in page.links + page.resources:
                with self.subTest(page=name, url=url):
                    self.assert_destination(name, url)

    def test_every_page_is_in_every_primary_and_footer_menu(self):
        expected = {'./' + name for name in self.pages}
        for name, page in self.pages.items():
            for nav in ('Primary navigation', 'Footer navigation'):
                with self.subTest(page=name, nav=nav):
                    self.assertTrue(expected <= set(page.navigation[nav]))
            if name != 'index.html':
                self.assertTrue(expected <= set(page.navigation['Site navigation']))

    def test_unique_fragment_ids(self):
        for name, page in self.pages.items():
            self.assertEqual(len(page.ids), len(set(page.ids)), name)

    def test_search_reaches_every_page_and_real_sections(self):
        index = json.loads((SITE / 'search-index.json').read_text())
        self.assertTrue({'./' + name for name in self.pages} <= {entry['url'] for entry in index})
        for entry in index:
            self.assert_destination('index.html', entry['url'])

    def test_mascots_and_favicon_are_real_vectors(self):
        for name in ('assets/cat.svg', 'assets/cat-adult.svg', 'assets/cat-face.svg', 'assets/cat-standing.svg', 'favicon.svg'):
            root = ET.parse(SITE / name).getroot()
            self.assertTrue(root.tag.endswith('svg'))
            self.assertIsNotNone(root.get('viewBox'))
            self.assertFalse(any(node.tag.endswith('image') for node in root.iter()), name)
            self.assertTrue(any(node.tag.endswith('path') for node in root.iter()), name)


class GeneratedDocumentationTests(unittest.TestCase):
    @unittest.skipUnless(GENERATED_DOCS.exists(), 'Generated Codeclew documentation has not been copied to the site')
    def test_copied_generated_documentation_local_closure_and_anchors(self):
        assert_generated_documentation(self, SITE, GENERATED_DOCS)

    def test_nested_history_assets_and_same_named_pages_resolve_from_source(self):
        with tempfile.TemporaryDirectory() as directory:
            site = Path(directory)
            root = site / 'examples/codeclew-source/docs'
            first = root / 'generated/first'
            second = root / 'generated/second'
            for bundle in (first, second):
                (bundle / 'services').mkdir(parents=True)
                (bundle / 'assets').mkdir()
                (bundle / 'assets/reader.js').write_text('')
            (root / 'history.html').write_text('<h1 id="history">History</h1>')
            (first / 'overview.html').write_text('<h1 id="first">First publication</h1>')
            (second / 'overview.html').write_text('<h1 id="second">Second publication</h1>')
            (second / 'services/service.html').write_text(
                '<a href="../overview.html#second">Overview</a>'
                '<a href="../../first/overview.html#first">Previous</a>'
                '<a href="../../../history.html#history">History</a>'
                '<script src="../assets/reader.js?v=retained"></script>'
                '<a href="https://example.invalid/source#L1">Public source</a>'
                '<img src="data:image/svg+xml,%3Csvg/%3E">'
            )
            assert_generated_documentation(self, site, root)
            # A same-basename page in another bundle cannot satisfy this anchor.
            with self.assertRaisesRegex(AssertionError, 'missing fragment'):
                assert_local_destination(self, site, {}, second / 'services/service.html', '../overview.html#first')

    def test_missing_asset_and_duplicate_anchors_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            site = Path(directory)
            root = site / 'examples/codeclew-source/docs'
            root.mkdir(parents=True)
            page = root / 'index.html'
            page.write_text('<h1 id="same">First</h1><h2 id="same">Second</h2>')
            with self.assertRaisesRegex(AssertionError, 'duplicate fragment IDs'):
                assert_generated_documentation(self, site, root)
            page.write_text('<h1 id="content">Overview</h1>')
            with self.assertRaisesRegex(AssertionError, 'missing absent.css'):
                assert_local_destination(self, site, {}, page, 'absent.css')
            outside = site.parent / 'outside.html'
            with self.assertRaisesRegex(AssertionError, 'escapes site'):
                assert_local_destination(self, site, {}, page, str(outside))

    def test_archived_native_root_uses_deployment_context_and_exact_records(self):
        with tempfile.TemporaryDirectory() as directory:
            site = Path(directory)
            root = site / 'examples/codeclew-source/docs'
            bundle = 'a' * 64
            frozen = root / 'generated' / bundle
            (frozen / 'services').mkdir(parents=True)
            (root / 'history.html').write_text('<h1 id="history">History</h1>')
            (frozen / 'services/service.html').write_text('<h1 id="section">Service</h1>')
            markup = ('<h1 id="root">Root</h1><a href="index.html#root">Home</a>'
                      '<a href="history.html#history">History</a>'
                      f'<a href="generated/{bundle}/services/service.html#section">Service</a>')
            archive = frozen / 'root-overview.html'
            archive.write_text(markup)
            (root / 'index.html').write_bytes(archive.read_bytes())
            digest = lambda data: 'sha256:' + hashlib.sha256(data).hexdigest()
            bindings = {'schema': 'codeclew-documentation-bindings/1.4',
                        'outputHashes': {'root-overview.html': digest(archive.read_bytes())}}
            binding_path = frozen / 'bindings.json'
            binding_path.write_text(json.dumps(bindings))
            manifest = {'schema': 'codeclew-documentation-publication/1.0', 'id': bundle,
                        'files': {'root-overview.html': digest(archive.read_bytes()),
                                  'bindings.json': digest(binding_path.read_bytes())}}
            manifest_path = frozen / 'publication.json'
            manifest_path.write_text(json.dumps(manifest))
            assert_generated_documentation(self, site, root)
            (root / 'index.html').write_text(markup + '<p>Changed root output</p>')
            with self.assertRaisesRegex(AssertionError, 'deployed root differs'):
                assert_generated_documentation(self, site, root)
            (root / 'index.html').write_bytes(archive.read_bytes())
            # Role recognition still checks actual assets and static fragments.
            with self.assertRaisesRegex(AssertionError, 'missing absent.css'):
                assert_local_destination(self, site, {}, root / 'index.html', 'absent.css')
            with self.assertRaisesRegex(AssertionError, 'missing fragment'):
                assert_local_destination(self, site, {}, root / 'index.html',
                                         f'generated/{bundle}/services/service.html#absent')
            archive.write_text(markup + '<p>Modified</p>')
            with self.assertRaisesRegex(AssertionError, 'archived root digest mismatch'):
                archived_root_context(self, root, archive)
            archive.write_text(markup)
            bindings['outputHashes']['root-overview.html'] = 'sha256:' + '0' * 64
            binding_path.write_text(json.dumps(bindings))
            manifest['files']['bindings.json'] = digest(binding_path.read_bytes())
            manifest_path.write_text(json.dumps(manifest))
            with self.assertRaisesRegex(AssertionError, 'root-output binding mismatch'):
                archived_root_context(self, root, archive)
            manifest_path.unlink()
            with self.assertRaisesRegex(AssertionError, 'missing native root-output records'):
                archived_root_context(self, root, archive)

    def test_unregistered_archive_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            site = Path(directory)
            root = site / 'examples/codeclew-source/docs'
            fake = root / 'generated/fake/root-overview.html'
            fake.parent.mkdir(parents=True)
            fake.write_text('<h1>Not a native root output</h1>')
            with self.assertRaisesRegex(AssertionError, 'invalid archived root placement'):
                assert_generated_documentation(unittest.TestCase(), site, root)

    def test_reader_routes_are_distinct_from_static_anchors(self):
        with tempfile.TemporaryDirectory() as directory:
            site = Path(directory)
            page = site / 'service.html'
            page.write_text('<main id="content"></main><script type="application/json" id="document-data">{"sections":[{"id":"section-overview"}]}</script>')
            assert_local_destination(self, site, {}, page, '#section-overview')
            page.write_text('<main id="content"></main>')
            with self.assertRaisesRegex(AssertionError, 'missing fragment'):
                assert_local_destination(self, site, {}, page, '#section-overview')


if __name__ == '__main__':
    unittest.main()
