#!/usr/bin/env python3
"""Render shared, no-JavaScript site navigation and the local search index.

Run after adding a page or changing a heading; --check detects stale output.
"""
import argparse
import hashlib
import html
from html.parser import HTMLParser
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
SITE = ROOT / "site"
PAGES = [
    ("index.html", "Start with Clew", "Start", "Install Clew and choose one engineering question."),
    ("nav-query.html", "Find the code", "Tasks", "Find an exact function, read its source and handle a no-match result."),
    ("documentation.html", "Create a first document", "Tasks", "Write one useful overview, inspect a citation and ask the saved source a question."),
    ("working-tree.html", "Review saved edits", "Tasks", "Inspect a changed body, signature or removed call and choose the next checks."),
    ("evidence.html", "Checks and limits", "Reference", "Read current qualification status and distinguish historical evidence."),
    ("extend.html", "Contribute", "Contribute", "Choose the source layer for a capability, adapter or build provider."),
]
HISTORICAL_PAGES = [("pilot.html", "Historical Spring case", "History", "A pinned two-service source study.")]


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
        result.append('</div>')
    return '\n'.join(result)


def blocks(file, label, group):
    menu = grouped(file)
    tasks = '\n'.join(link(path, title, file) for path, title, category, _ in PAGES if category == 'Tasks')
    header = f'''<a class="skip-link" href="#main-content">Skip to content</a>
<header class="site-header"><div class="site-header-inner shell">
  <a class="brand" href="./index.html" aria-label="Codeclew home"><img src="./assets/cat-face.svg" width="48" height="48" alt=""><span>Codeclew<small>Find code. Follow the thread.</small></span></a>
  <nav class="primary-nav" aria-label="Primary navigation">
    <details class="site-menu"><summary>Tasks <span aria-hidden="true">⌄</span></summary><div class="menu-panel"><div class="nav-group">{tasks}</div></div></details>
    {tasks}
    {link('evidence.html', 'Checks and limits', file)}
  </nav>
  <button class="search-trigger" type="button" aria-haspopup="dialog" aria-controls="site-search" hidden><span>Search</span><kbd>Ctrl / ⌘ K</kbd></button>
</div></header>
<dialog id="site-search" class="search-dialog" aria-labelledby="search-title">
  <div class="search-heading"><h2 id="search-title">Find your next thread</h2><button type="button" data-close-search aria-label="Close search">Close <kbd>Esc</kbd></button></div>
  <label for="site-search-input">Search pages and sections</label><input id="site-search-input" type="search" placeholder="Try “source”, “saved edits” or “document”" autocomplete="off">
  <p id="search-status" role="status"></p><div id="search-results"></div>
</dialog>'''
    sidebar = ''
    category = f'<span>{group}</span><span aria-hidden="true">/</span>' if group != label else ''
    breadcrumb = f'<nav class="breadcrumbs" aria-label="Breadcrumb">{link("index.html", "Home", "")}<span aria-hidden="true">/</span>{category}<span aria-current="page">{label}</span></nav>'
    footer = f'''<footer class="site-footer shell"><div class="footer-brand"><a class="brand" href="./index.html"><img src="./assets/cat-face.svg" width="38" height="38" alt=""><span>Codeclew<small>Find code. Follow the thread.</small></span></a><p>Better developers.<br>A more understandable world.</p><small>Apache-2.0 · Built with curiosity.</small></div>
<nav class="footer-nav" aria-label="Footer navigation">{menu}<div class="nav-group"><p>History</p>{link('pilot.html', 'Historical Spring case', file)}</div><div class="nav-group"><p>Community</p><a href="https://github.com/codeclew/codeclew">GitHub</a><a href="https://github.com/codeclew/codeclew/releases">Changelog & releases</a><a href="https://github.com/codeclew/codeclew/security">Security</a></div></nav></footer>'''
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
        if self.in_main and tag in ('section', 'article', 'h2') and attrs.get('id') and (tag != 'h2' or not self.anchor):
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
    for file, label, group, description in PAGES + HISTORICAL_PAGES:
        path = SITE / file
        original = path.read_text()
        content = original
        for name, block in blocks(file, label, group).items():
            pattern = rf'<!-- SITE_{name} -->.*?<!-- /SITE_{name} -->'
            if name in ('HEADER', 'FOOTER') or file != 'index.html':
                assert re.search(pattern, content, flags=re.S), f'{file}: missing {name} slot'
            content = re.sub(pattern, f'<!-- SITE_{name} -->\n{block}\n<!-- /SITE_{name} -->', content, flags=re.S)
        for asset in ('theme.css', 'tasks.css', 'app.js'):
            version = hashlib.sha256((SITE / asset).read_bytes()).hexdigest()[:12]
            pattern = rf'((?:href|src)=["\']\./{re.escape(asset)})(?:\?v=[^"\']*)?(["\'])'
            content = re.sub(pattern, rf'\1?v={version}\2', content)
        if content != original:
            stale.append(file)
            if not args.check:
                path.write_text(content)
        if (file, label, group, description) in HISTORICAL_PAGES:
            continue
        index.append({'title': label, 'url': './' + file, 'description': description, 'group': group})
        sections = Sections()
        sections.feed(content)
        for anchor, title in sections.sections:
            description = label
            if file == 'index.html' and anchor == 'support':
                description = 'Language support: Kotlin, Java, Rust, Python, TypeScript and JavaScript; C# Roslyn read-only analysis is included since 0.13.11 and requires .NET 10+ SDK (dotnet) and caller restore.'
            if file == 'evidence.html' and anchor == 'csharp-preview':
                description = 'Published C# Roslyn read-only analysis: installed example, .NET 10+ SDK (dotnet), exact project or solution scope and caller restore.'
            if file == 'extend.html' and anchor == 'csharp-preview':
                description = 'Contribute to C# Roslyn read-only analysis with .NET 10+ SDK (dotnet), restored project inputs and explicit MVC attribute-route boundaries.'
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
