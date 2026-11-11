# Trusted Markdown renderer example

This is the default local preview for ordinary human-page documentation requests
without a caller-selected renderer or theme. It uses the canonical Codeclew colors, typography and cat
artwork around ordinary Markdown. Human page titles, an active page menu and
keyboard-scrollable tables make the same authored content easier to read. It
uses Node.js 20 or later and the pinned `marked` 16.4.2 dependency, not a
new rendering engine or a replacement for explicitly selected native
document workflows. It renders authored Markdown;
installation and trusted argv selection remain explicit.

## Shared semantic catalog navigation

The optional `codeclew-cross-docset-routes/1.2` registry adds `semanticCatalog`
to the existing docset/process navigation. Version 1.1 keeps its original shape.
See [the owned five-service example](docset-semantic.example.json): two independent
service docsets plus a catalog docset with no captured services. This example
declares navigation only; it does not supply source evidence or authored meanings.

`semanticCatalog` contains `id`, `revision` and explicitly chosen `records`.
Each record has `id`, `kind`, `title`, a canonical `target`, scoped local `aliases`
and local `uses`. Kinds are `external`, `concept`, `local` and `representation`.
Each alias declares `input`, `docset`, `localId` and optionally `service`; identical
names in different inputs never establish equivalence. A local/representation
record requires service `scope`; a representation's `represents` names an existing
concept. Local usage targets remain service-owned. An optional usage `process`
links to the corresponding whole-process page; do not infer one from a name.

An existing `externalActors` entry can explicitly set `canonical` to the external
record ID. Its process participant/recipient links then lead to the same canonical
system. In authored `clew-sequence`, a participant or arrow can use
`"catalogRef":"EXACT_EXTERNAL_RECORD_ID"` instead of `target`. The full renderer
resolves it against this explicit registry after checking destinations. Keep an
arrow's ordinary local `target` when it should open the service contract, and put
`catalogRef` on its external participant. An unknown ID, a concept used as an
external, missing registry or simultaneous `target`/`catalogRef` refuses; labels
and matching host names never select a destination.

One shared external or concept has one canonical page. The renderer links service
details to that page and adds consumer backlinks there. It separately links a
representation to its concept. Same-spelling scoped terms show the owning service
and docset. Catalog-only collections do not show empty process filters. Common
pages remain authored Markdown, composed/bound/finalized through the same pipeline;
the registry does not generate their descriptions or prove declared interactions.
Local pages/anchors and ownership are checked; remote destinations remain explicit
declarations and are not fetched. Keep operations, addresses, configuration and
message contracts in their service pages. See [retained catalog inputs](../../references/shared-semantic-catalog.md).

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

## Optional service navigation

Supply `--navigation /path/to/navigation.json` to replace the flat menu with
explicit service groups, local starts, data links and shared-route participation.
Service overview pages gain entry links; shared-route pages gain participant
links. Breadcrumbs show the current page and, with JavaScript, its linked section.
Ordinary links and native disclosures work without JavaScript. Existing exports
without this option retain their flat menu; neutral output remains unchanged.

```json
{
  "schema": "codeclew-markdown-navigation/1",
  "home": {"title":"Guide","target":"index.md"},
  "services": [{
    "id":"catalog", "title":"Catalog", "overview":"catalog.md",
    "pages":["catalog.md","work.md","data.md"],
    "starts":[{"title":"Accept request","kind":"http","target":"work.md#accept","summary":"Stores the accepted request."}],
    "data":[{"title":"Record fields","target":"data.md"}]
  }],
  "routes": [{
    "id":"add", "title":"Add record", "target":"journey.md#add",
    "origin":{"service":"catalog","title":"Accept request","kind":"http","target":"work.md#accept"},
    "participants":[{"service":"catalog","title":"Storage","role":"Persists the request.","target":"work.md#accept","section":"journey.md#add"}]
  }]
}
```

The example assumes these files and IDs exist. Targets use flat Markdown page
names with optional existing anchors. `pages` declares ownership explicitly;
local starts/data/participant targets must belong to that service. `section`
links back to the participant's section on the shared-route page. `origin`
identifies the cross-service origin separately from each local start and must
also be represented in `participants`. One service can have several roles.
`routes` may be `[]` for a service guide without a documented shared route.
Kinds are `http`, `kafka`, `scheduled`, `background`, `manual` or `other`.
These are authored labels, never inferred from filenames or shared topic names.

Use short titles/summaries/roles pointing to already described bound sections;
they introduce no new behavior, delivery or ordering guarantees. Keep navigation
with the owned renderer command and update it when pages/sections change.
Explicit invalid metadata fails with a property path instead of silently hiding
destinations. Unassigned pages and extra owned pages remain reachable as Other
pages. This is a reader index, not a graph engine or an evidence-binding scheme.

No source capture, model,
network request or application code execution is performed by this renderer;
dependency installation is a separate explicit setup step. Keep the installed
skill unchanged and retain its version/digest alongside your chosen renderer.

## Independent docsets and process filters

Optional `--docset-routes FILE --docset ID --docset-version VERSION` augments
`--navigation FILE`; all four options are required together for this feature.
Without them the existing flat export remains supported. One local service uses
its own title without a redundant Services heading; multiple local services keep
that heading. Evidence roots, authored docsets and hosting directories stay distinct.

Start with [the complete declaration](docset-routes.example.json). Replace its
example names/URLs with existing authored destinations, retaining the schema shape.
Render each input/output separately with the same registry bytes:

```sh
node /path/to/renderer/render.mjs ORDERS_INPUT ORDERS_OUTPUT --navigation orders-nav.json --docset-routes routes.json --docset orders-docs --docset-version orders-r1
node /path/to/renderer/render.mjs INVENTORY_INPUT INVENTORY_OUTPUT --navigation inventory-nav.json --docset-routes routes.json --docset inventory-docs --docset-version inventory-r1
node /path/to/renderer/render.mjs PORTAL_INPUT PORTAL_OUTPUT --navigation portal-nav.json --docset-routes routes.json --docset portal --docset-version portal-r1
```

Use these options in the ordinary trusted renderer argv for checked finalize or
render-only updates. `baseURL` must be an explicit HTTP(S) directory with a safe
unencoded ASCII path and trailing slash, without credentials/query/fragment.
Namespaces on one origin must be disjoint. Page is a raw flat `.html` filename;
anchor is a raw identifier, encoded once. No slash/encoded traversal or filesystem
URLs. Local `.html` maps to existing `.md`: anchors must be unique, service targets
owned, and a local origin must match a declared start. Shared canonical pages and
the optional catalog are not owned by a service; zero-service docsets are valid.
Remote URLs never enter the local path resolver and are not fetched/verified.

Route IDs stay stable; declaration/docset versions and the exact registry digest
are recorded separately in HTML. Versions are owner declarations, not deployed
content checks. Changed registry bytes invalidate old exact replay. Keep registry
and renderer inputs with the export. No peer capture/importer, deploy or redirect
engine is introduced. Navigation never changes source/Work/revision/claim authority.

Occurrences have distinct IDs; a service may recur in several roles. Start is the
explicit origin service, End the terminal operation owner (not recipient/last call),
and Intermediate an explicit internal occurrence in that outcome. Values within a
filter are OR; facets AND in one compatible branch/outcome. No cross-branch join.
Start-only lookup works without classified outcomes. Known matches survive partial
scope; absent metadata is unclassified, not a proven negative. Unknown origins/ends
stay explicit; all routes appear unfiltered. External actors are not fake services.

The portal shows declared service links, process cards and keyboard-accessible
checkbox filters; narrow layouts collapse the filter panel. It distinguishes query
uncertainty from overall classification gaps. A reported remote `problem` has kind
`reported-missing`, `reported-renamed` or `reported-version-drift`, note and reportedBy
(optional observedVersion): its entry remains visible with that link disabled.
Unsafe or missing local targets still reject rendering. Remote availability/anchors
need a separate authorized deployment check; a checked export does not provide it.

Metadata-only index pages may be rendered as navigation; they are not source-checked
service explanations. Do not invent claims/manifests to make an empty metadata page
pass compose/bind. Real cross-service narrative claims still need their real source
manifests; this feature does not repair imported-library/external-input bindings.

## Authored sequences

The `clew-sequence` fence renders an authored sequence rather than a code listing.
It uses finite HTML/CSS lifelines and arrows; no new dependency, source inference,
graph engine or network request is involved. Desktop shows the diagram and a
disclosure with the same numbered steps. Narrow screens show the text trace,
keeping sender, receiver, action and condition together without horizontal panning.
Without JavaScript the trace stays open. Neutral pages also render sequences.

````markdown
```clew-sequence
{"title":"Accept an item","participants":[
  {"id":"client","title":"Client"},
  {"id":"handler","title":"ItemController.post","target":"post.md#handler"},
  {"id":"stock","title":"Stock API","target":"integrations.md#stock"}
],"steps":[
  {"from":"client","to":"handler","kind":"call","label":"POST /v1/items","target":"post.md#handler"},
  {"from":"handler","to":"stock","kind":"call","label":"Check stock","condition":"Only when stock.check is enabled","target":"post.md#check"},
  {"from":"stock","to":"handler","kind":"return","label":"Stock result","condition":"Check was made"},
  {"from":"handler","to":"client","kind":"return","label":"200 with result","condition":"Validation and stock check succeed"}
]}
```
````

Top-level keys are exactly `title`, `participants`, `steps`. Participants have
unique `id`, nonempty `title` and optional `target`. Steps have `from`, `to`,
`kind`, `label`, optional `condition` and `target`. Kinds: `call` (no implicit wait),
`return`, `async` (start without waiting), `await` (explicit evidenced wait),
`note` (local work/decision, usually same sender and receiver). Every condition
applies only to its own row: write else/skip/error alternatives explicitly.
No implicit branches, returns, ACK, ordering or completion are added.

Targets use an existing flat `.md` page with an optional unique anchor. Invalid
targets/schema fail before output is written. A client needs no fictitious page;
external participants should link to their local integration description. Actor
headers and message labels link directly to these destinations on the diagram;
the same links remain available in its text trace. Steps can link to the explained local section. These links are navigation, not clew
evidence: bind substantive JSON rows/conditions to their exact delivered handles
through ordinary compose fragments, alongside bound prose. Retain existing source
maps and originals. Rendering and link checks never certify semantic completeness.

Keep labels concise; full explanations and source/provenance belong in linked
steps and secondary details. Multiple consecutive blocks may split a long
sequence at meaningful boundaries while preserving all meaningful work.

## Checked local sources footer

For a new full guide, use `reader_export_check.py finalize --source-footers` with
this renderer's ordinary argv. It binds once, materializes local excerpts and
checks the final output, recording the effective argv and source bundle. Add
`--source-view SAVED_VIEW` for existing supported views/unions. The following
materialization command is for refreshing an already checked render without
another bind.

Sources are selected by the page's bound claims, not a mandatory full-service
DTO inventory. Annotated semantic fragments require `{id,text,evidence}` with
nonempty genuine cite-handles. Plain headings/navigation may use `{text}`;
do not remove annotations to present unsupported field meanings as established.
One read fragment may support several claims when it actually covers them.
Compose can reject unpaired annotations or empty evidence before binding; bind
can reject unavailable handles; source materialization can reject missing excerpts.
Keep the actual stage/status instead of calling every failure
`CITATION_BINDING_UNAVAILABLE`. No partial/skip-invalid-claims footer mode exists;
an invalid required claim can block finalize for the selected page set.

Git mapping is optional for the local footer. Saved `--source-view` inputs must
be previously delivered supported views/unions containing every cited excerpt;
they are not arbitrary quotations. Without them the existing inspector reads
bound ranges. For a render-only refresh below, the materializer command is
`reader_sources.py --index ...`, without a `materialize` subcommand.

After bind/finalize, materialize cited excerpts without changing Markdown/maps:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/reader_sources.py" \
  --index /path/to/checked/index.json --manifest /path/to/citations.json \
  --view /path/to/saved-source-union.json --output /path/to/new-sources.json
```

`--index` also accepts the existing Markdown `sources.json` checked-index pointer.
Repeat `--manifest`/`--view` for multiple inputs. Saved supported author-read/union
views reuse exact UTF-8 `byteSlice` ranges, validated against citation identities,
locations and text digests. With views, missing/conflicting excerpts refuse rather
than fetching again. Without views, one bounded grouped read per manifest/batch
obtains only cited ranges; no capture or source fallback. SOURCE_NATIVE keeps its
existing explicit `--clew` and `--root` admission arguments.

Add `--sources /path/to/new-sources.json` to the owned renderer argv. Every page
gets Sources (localized for Russian): one file row per service/path/revision/
snapshot, deduplicated exact ranges separated by `;`. Each range opens a local
verbatim excerpt on that page. Full paths/versions remain discernible; missing
Git mappings need no invented URL. No extra pages or full cache are exported.
Changed index/maps/Markdown refuse stale bundles. Refresh after binding changes.

For a render-only update, render the existing checked bound Markdown and run
`reader_export_check.py` against the same checked index and original authored
root with the new renderer argv/output. Reuse the existing bind; do not author or
finalize again just for this presentation change. Keep originals/maps authoritative.

Portable `npm test` is a smoke check. Full repository renderer regressions run as
`CODECLEW_MARKDOWN_RENDERER=/path/to/npm-renderer node --test scripts/test_markdown_renderer.mjs`;
sequence regression coverage also uses `node --test scripts/test_markdown_sequence.mjs`.
