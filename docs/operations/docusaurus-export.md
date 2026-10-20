# Portable documentation with Docusaurus

Use the Docusaurus profile to review and maintain a full saved publication as
Markdown/MDX in Git. The standalone HTML reader remains the default.

Export the exact immutable publication and selected snapshot to a new directory:

```sh
clew docs pages render \
  --root "$DOCS_ROOT" \
  --snapshot "$SNAPSHOT" \
  --publication "$PUBLICATION" \
  --reading-plan reading-plans.json \
  --data-model data-model-plan.json \
  --format docusaurus \
  --output documentation-site
```

The reading plan and data model are optional, explicit author inputs. Add
`--contract-comparison comparison-input.json` to include a retained HTTP
comparison bound to the same selected snapshot. All input bindings are validated
before publishing files. A declaration-only `--input` render does not support this
profile; use its standalone output or a full saved publication. Existing nonempty
output directories are refused.

The site contains separate process, service, operation, HTTP contract, model and
source-fragment pages. Reading guides retain the original accounts and code
explanations as linked technical context. Diagrams project visible interactions,
conditional branches and participating actors; full original events and Notes
remain in a disclosure. A model or process Data Flow is present only when the
author input explicitly requests it. See [data-model reading](data-model-reading.md)
for field mappings and process return links.

Build and inspect the exported project:

```sh
cd documentation-site
npm ci
npm run build
npm run serve
```

The lockfile pins dependencies. The build prepares shared field-reader packs and
checks links and anchors strictly. For a deployment beneath a path, set its origin
and base path before building, for example:

```sh
CODECLEW_DOCS_URL=https://docs.example.com \
CODECLEW_DOCS_BASE_URL=/weather/ npm run build
```

Deploy `build/` through your chosen hosting process. The export command does not
publish a website. Use the generated README for the project layout and commands.

Commit `docs/`, `inputs/`, `static/clew-data/`, `src/`, `scripts/`, configuration,
the dependency lockfile and `export-manifest.json`. The generated `.gitignore`
excludes dependencies, build output and derived reader packs. Regenerate into a
fresh directory with the same immutable evidence and explicit author inputs to
compare changes. Output paths are relative and do not depend on the destination
directory.

Complete native JSON is normalized through shared content-addressed objects.
Resolving `$clewRef` reconstructs its original value; `$clewLiteral` preserves
original objects that use reserved keys. Export verifies reconstruction before
writing. Object identity covers the whole original value, including source
revision, range, text digest and authority. Equal source text at another revision
does not become the same source binding. Build-time packs preserve these records
and group them by reader consumers without duplicating objects or fetching
unrelated evidence.

The export includes bounded retained source fragments and original evidence
metadata. It is not a Git repository backup. Declarations, source interpretations,
review states and unresolved runtime behavior remain distinct in both formats.

Use [source-link export](source-link-export.md) with `--source-mode remote` to omit automatic
source attachments and open exact GitHub/GitLab revisions instead. The default
`embedded` mode retains source fragments. Authored quotations and examples remain.
