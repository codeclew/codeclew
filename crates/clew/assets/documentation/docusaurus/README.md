# Codeclew documentation export

This is a native Markdown/MDX publication with source-bound reading text,
contracts, models, implementation explanations and optional field readers.
The original claims, evidence, revision boundaries and review states remain
separate from the editorial reading guide. Export does not establish runtime
conformance, delivery or ownership.

Run `npm ci`, then `npm run build`. Dependencies and their transitive lockfile are
pinned. The build prepares shared reader packs and creates the Docusaurus site.
Use `npm run serve` to inspect the production build locally. Set
`CODECLEW_DOCS_BASE_URL` before building for a non-root deployment path, and
`CODECLEW_DOCS_URL` for its origin. Deploy `build/` only. The export command does
not publish a website.

Commit `inputs/`, `docs/`, `static/clew-data/`, `scripts/`, `src/`, `package.json`,
`package-lock.json`, `sidebars.js`, `docusaurus.config.js` and the export manifest.
Do not commit `node_modules/`, `build/`, `.docusaurus/` or
`static/clew-reader-packs/`. Those are reproducible dependency/build outputs.
Source objects remain separate for Git review; build-time packs group objects
with the same reader consumers, avoiding repeated objects and unrelated data.

The Markdown files are the reading projection. JSON under `static/clew-data/`
uses `codeclew-normalized-data/1.0` and content-addressed objects. Expanding
`$clewRef` through the object store reconstructs the complete original data.
`$clewLiteral` protects original objects with reserved reference keys. Object
identity hashes the full original value, including source revision, text digest,
range, occurrence and authority when present. Identical source text at another
revision is not the same binding. Every native JSON payload is checked against
its expanded export before writing the site.

The original author inputs are copied to `inputs/`. Regenerating documentation
still needs its explicit immutable Codeclew publication/snapshot and documentation
root. The export contains bounded retained evidence, not a Git repository backup.
Historical paths inside retained source or test artifacts remain original evidence;
they are not dependencies on the author machine. Structural output paths are
relative to this export.

Standalone HTML remains the default format. Select this profile with
`clew docs pages render --publication PUBLICATION --snapshot SNAPSHOT --root ROOT
--format docusaurus --output NEW_DIRECTORY`, together with the desired
`--reading-plan`, `--contract-comparison` and `--data-model` inputs.
