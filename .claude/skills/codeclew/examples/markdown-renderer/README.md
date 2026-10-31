# Trusted Markdown renderer example

This optional reader uses the canonical Codeclew colors, typography and cat
artwork around ordinary Markdown. Human page titles, an active page menu and
keyboard-scrollable tables make the same authored content easier to read. It
uses Node.js 20 or later and the pinned `marked` 16.4.2 dependency, not a
new default renderer or a replacement for native HTML/MDX/Docusaurus workflows.

Copy this directory to your own tooling directory before installing dependencies:

```sh
cp -R "$CODECLEW_SKILL_DIR/examples/markdown-renderer" /path/to/my-renderer
cd /path/to/my-renderer
npm ci --ignore-scripts --no-audit --no-fund
node render.mjs /path/to/markdown /path/to/new-html "Project documentation" --lang en
npm test
```

For the existing checked-export/finalize helper, save a renderer command JSON
array with explicit arguments and literal input/output placeholders:

```json
["node", "/path/to/my-renderer/render.mjs", "{input}", "{output}", "Project documentation", "--lang", "en"]
```

Supply it through `reader_export_check.py finalize --renderer-command FILE`, as
described in [the authoring reference](../../references/compose-original.md).
Choose a trusted command: it executes local code and renders Markdown HTML.
The checker verifies deterministic output, links and mechanical evidence bindings;
it does not sandbox the renderer or certify the prose's meaning.

The input is one directory of top-level Markdown files. Output is the
corresponding HTML pages, with relative `.md` links changed to `.html`. Heading
IDs are deterministic, including duplicate headings. Set `--lang ru` for Russian
pages. The Codeclew theme embeds its style and decorative artwork, and writes a
byte-for-byte local `favicon.svg`. The export checker checks that SVG asset
against the same explicit renderer; it does not allow arbitrary local assets.
The packaged `assets/reader.css`, `typography.css` and `favicon.svg` are copies of
the native documentation assets; `cat-standing.svg` is the existing site artwork.

Use `--theme neutral` to retain the earlier neutral HTML, CSS and slug navigation
for a comparison. Its default English markup remains byte-compatible with the
old renderer. Choose the theme explicitly for a retained comparison.

Write entity field lists as authored GFM tables with meaningful column headings;
keep behavior, conditions and failures in short prose. The renderer does not
infer tables from paragraphs or rewrite any claims. Source bindings remain in
their existing metadata rather than being added to the reader chrome.

No source capture, model,
network request or application code execution is performed by this renderer;
dependency installation is a separate explicit setup step. Keep the installed
skill unchanged and retain its version/digest alongside your chosen renderer.
