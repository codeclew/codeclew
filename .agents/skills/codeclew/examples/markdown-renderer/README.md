# Trusted Markdown renderer example

This is the existing public research renderer, with the local dependency anchor
replaced by ordinary package resolution. The Markdown parsing, heading IDs,
Markdown-to-HTML links, navigation and CSS are unchanged. It uses Node.js 20 or
later and the pinned `marked` 16.4.2 dependency. It is an optional example, not a
new default renderer or a replacement for native HTML/MDX/Docusaurus workflows.

Copy this directory to your own tooling directory before installing dependencies:

```sh
cp -R "$CODECLEW_SKILL_DIR/examples/markdown-renderer" /path/to/my-renderer
cd /path/to/my-renderer
npm ci --ignore-scripts --no-audit --no-fund
node render.mjs /path/to/markdown /path/to/new-html "Project documentation"
```

For the existing checked-export/finalize helper, save a renderer command JSON
array with explicit arguments and literal input/output placeholders:

```json
["node", "/path/to/my-renderer/render.mjs", "{input}", "{output}", "Project documentation"]
```

Supply it through `reader_export_check.py finalize --renderer-command FILE`, as
described in [the authoring reference](../../references/compose-original.md).
Choose a trusted command: it executes local code and renders Markdown HTML.
The checker verifies deterministic output, links and mechanical evidence bindings;
it does not sandbox the renderer or certify the prose's meaning.

The input is one directory of top-level Markdown files. Output is the
corresponding HTML pages, with relative `.md` links changed to `.html`. Heading
IDs are deterministic, including duplicate headings. No source capture, model,
network request or application code execution is performed by this renderer;
dependency installation is a separate explicit setup step. Keep the installed
skill unchanged and retain its version/digest alongside your chosen renderer.
