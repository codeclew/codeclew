#!/usr/bin/env python3
"""Render shared, no-JavaScript site navigation and the local search index.

Run after adding a page or changing a heading; --check detects stale output.
"""
import argparse
import html
from html.parser import HTMLParser
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
SITE = ROOT / "site"
PAGES = [
    ("index.html", "Home", "Start here", "Install Codeclew, supported languages and the everyday analysis flow."),
    ("architecture.html", "Architecture", "Documentation", "How source snapshots, language facts and bounded context support an explanation."),
    ("nav-query.html", "Navigation walkthrough", "Documentation", "Follow nav query from a scoped question to exact source evidence."),
    ("documentation.html", "Service documentation", "Documentation", "Maintain source-linked service explanations, diagrams and freshness checks."),
    ("working-tree.html", "Saved edits guide", "Documentation", "Compare saved changes against HEAD and inspect their consequences."),
    ("working-tree-example.html", "Saved edits example", "Documentation", "An interactive retained report with before and after source."),
    ("extend.html", "Extend Codeclew", "Documentation", "Contributor guide: add a capability, language adapter, build provider, framework rule or CLI workflow."),
    ("evidence.html", "Evidence & studies", "Research", "Release references and historical studies, with methods and limitations."),
    ("pilot.html", "Spring case study", "Research", "A historical two-service pilot: 31 Spring roots checked against committed source."),
]


def link(file, label, current, **attrs):
    attributes = ''.join(f' {key}="{html.escape(value)}"' for key, value in attrs.items())
    if file == current:
        attributes += ' aria-current="page"'
    return f'<a href="./{file}"{attributes}>{html.escape(label)}</a>'


def grouped(current):
    result = []
    for group in dict.fromkeys(page[2] for page in PAGES):
        result.append(f'<div class="nav-group"><p>{group}</p>')
        result.extend(link(file, label, current) for file, label, category, _ in PAGES if category == group)
        if group == "Start here":
            result.extend([link("index.html#install", "Installation", current), link("index.html#support", "Language support", current)])
        result.append('</div>')
    return '\n'.join(result)


def blocks(file, label, group):
    menu = grouped(file)
    header = f'''<a class="skip-link" href="#main-content">Skip to content</a>
<header class="site-header"><div class="site-header-inner shell">
  <a class="brand" href="./index.html" aria-label="Codeclew home"><img src="./assets/cat-face.svg" width="48" height="48" alt=""><span>Codeclew<small>Find code. Follow the thread.</small></span></a>
  <nav class="primary-nav" aria-label="Primary navigation">
    <details class="site-menu"><summary>Explore <span aria-hidden="true">⌄</span></summary><div class="menu-panel">{menu}</div></details>
    {link('documentation.html', 'Documentation', file)}
    {link('evidence.html', 'Evidence', file)}
    <a href="https://github.com/codeclew/codeclew">GitHub <span aria-hidden="true">↗</span></a>
  </nav>
  <button class="search-trigger" type="button" aria-haspopup="dialog" aria-controls="site-search" hidden><span>Search docs…</span><kbd>⌘ K</kbd></button>
</div></header>
<dialog id="site-search" class="search-dialog" aria-labelledby="search-title">
  <div class="search-heading"><h2 id="search-title">Find your next thread</h2><button type="button" data-close-search aria-label="Close search">Close <kbd>Esc</kbd></button></div>
  <label for="site-search-input">Search pages and sections</label><input id="site-search-input" type="search" placeholder="Try “Python” or “service documentation”" autocomplete="off">
  <p id="search-status" role="status"></p><div id="search-results"></div>
</dialog>'''
    sidebar = f'''<aside class="site-sidebar"><details class="sidebar-disclosure" open><summary>On this site</summary><nav aria-label="Site navigation">{menu}</nav></details>
<a class="sidebar-note" href="./documentation.html"><img src="./assets/cat-standing.svg" alt="" width="90" height="90" loading="lazy"><strong>Curious how it works?</strong><span>Follow the clues.<br>Inspect the evidence.</span></a></aside>'''
    breadcrumb = f'<nav class="breadcrumbs" aria-label="Breadcrumb">{link("index.html", "Home", "")}<span aria-hidden="true">/</span><span>{group}</span><span aria-hidden="true">/</span><span aria-current="page">{label}</span></nav>'
    footer = f'''<footer class="site-footer shell"><div class="footer-brand"><a class="brand" href="./index.html"><img src="./assets/cat-face.svg" width="38" height="38" alt=""><span>Codeclew<small>Find code. Follow the thread.</small></span></a><p>Better developers.<br>A more understandable world.</p><small>Apache-2.0 · Built with curiosity.</small></div>
<nav class="footer-nav" aria-label="Footer navigation">{menu}<div class="nav-group"><p>Community</p><a href="https://github.com/codeclew/codeclew">GitHub</a><a href="https://github.com/codeclew/codeclew/releases">Changelog & releases</a><a href="https://github.com/codeclew/codeclew/security">Security</a></div></nav></footer>'''
    return {'HEADER': header, 'SIDEBAR': sidebar, 'BREADCRUMB': breadcrumb, 'FOOTER': footer}


class Sections(HTMLParser):
    def __init__(self):
        super().__init__()
        self.sections = []
        self.anchor = ''
        self.heading = None
        self.in_main = False

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == 'main':
            self.in_main = True
        if self.in_main and tag in ('section', 'article', 'h2') and attrs.get('id'):
            self.anchor = attrs['id']
        if self.in_main and tag == 'h2':
            self.heading = []
        if tag == 'br' and self.heading is not None:
            self.heading.append(' ')

    def handle_data(self, text):
        if self.heading is not None:
            self.heading.append(text)

    def handle_endtag(self, tag):
        if tag == 'h2' and self.heading is not None:
            title = ' '.join(''.join(self.heading).split())
            if title and self.anchor:
                self.sections.append((self.anchor, title))
            self.heading = None
            self.anchor = ''
        if tag == 'main':
            self.in_main = False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    stale, index = [], []
    for file, label, group, description in PAGES:
        path = SITE / file
        original = path.read_text()
        content = original
        for name, block in blocks(file, label, group).items():
            pattern = rf'<!-- SITE_{name} -->.*?<!-- /SITE_{name} -->'
            if name in ('HEADER', 'FOOTER') or file != 'index.html':
                assert re.search(pattern, content, flags=re.S), f'{file}: missing {name} slot'
            content = re.sub(pattern, f'<!-- SITE_{name} -->\n{block}\n<!-- /SITE_{name} -->', content, flags=re.S)
        if content != original:
            stale.append(file)
            if not args.check:
                path.write_text(content)
        index.append({'title': label, 'url': './' + file, 'description': description, 'group': group})
        sections = Sections()
        sections.feed(content)
        for anchor, title in sections.sections:
            description = label
            if file == 'index.html' and anchor == 'support':
                description = 'Language support: Kotlin, Java, Rust, Python, TypeScript and JavaScript; analysis and managed changes.'
            index.append({'title': title, 'url': f'./{file}#{anchor}', 'description': description, 'group': group})
    output = json.dumps(index, indent=2, ensure_ascii=False) + '\n'
    target = SITE / 'search-index.json'
    if not target.exists() or target.read_text() != output:
        stale.append(target.name)
        if not args.check:
            target.write_text(output)
    if args.check and stale:
        parser.exit(1, 'Stale navigation/search: ' + ', '.join(stale) + '\nRun scripts/build_site_navigation.py\n')
    print(f'PASS: shared navigation for {len(PAGES)} pages; {len(index)} search entries')


if __name__ == '__main__':
    main()
