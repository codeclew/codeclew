#!/usr/bin/env python3
"""Check site reachability, search destinations and vector asset integrity."""
from html.parser import HTMLParser
import json
from pathlib import Path
import unittest
from urllib.parse import unquote, urlsplit
import xml.etree.ElementTree as ET

SITE = Path(__file__).resolve().parents[1] / 'site'


class Page(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.ids = []
        self.resources = []
        self.links = []
        self.navigation = {}
        self.nav = None
        self.feed(path.read_text())

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
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


class SiteTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.pages = {path.name: Page(path) for path in SITE.glob('*.html')}

    def assert_destination(self, source, url):
        target = urlsplit(url)
        if target.scheme or target.netloc:
            return
        path = SITE / unquote(target.path or source)
        self.assertTrue(path.is_file(), f'{source}: missing {url}')
        if target.fragment and path.suffix == '.html':
            self.assertIn(unquote(target.fragment), self.pages[path.name].ids, f'{source}: missing fragment {url}')

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


if __name__ == '__main__':
    unittest.main()
