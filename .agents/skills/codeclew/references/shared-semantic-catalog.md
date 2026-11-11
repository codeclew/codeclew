# Shared semantic catalog inputs

## Collect retained declarations

Start with the existing documentation roots and exact retained snapshots. Select
only the services needed for this collection. Roots remain independent; the same
service or external ID in two roots does not establish shared identity.

Use installed `clew` with `semantic-inventory-v1` support and the matching skill.
An older runtime refusing this profile needs a supported upgrade, not recapture.
Keep a running trial on its admitted runtime. The collector never captures or
issues SOURCE-body read or source-checkout materialization commands. Its counters report actual
native commands; they do not measure internal object-store reads.

Write `collection.json` using [the input schema](../schemas/semantic-collection.schema.json):

```json
{"schema":"codeclew-semantic-collection/1.0","id":"platform","revision":"r1","inputs":[
  {"id":"east","docset":"east","services":[
    {"id":"orders","snapshot":"EXACT_EAST_SNAPSHOT"},
    {"id":"billing","snapshot":"EXACT_EAST_SNAPSHOT"},
    {"id":"enrollment","snapshot":"EXACT_EAST_SNAPSHOT"}]},
  {"id":"west","docset":"west","services":[
    {"id":"orders","snapshot":"EXACT_WEST_SNAPSHOT"},
    {"id":"enrollment","snapshot":"EXACT_WEST_SNAPSHOT"}]}]}
```

Replace the snapshot strings with returned saved snapshot IDs. A service can
select its own exact snapshot; the example uses one composed snapshot per root.
For five services in one root, use one input with five service entries. Do not
copy unrelated roots into a merged private store.

Keep machine paths in caller-selected `local-roots.json`, separate from the
shareable collection and inventory:

```json
{"schema":"codeclew-evidence-inputs/1.0","inputs":{
  "east":{"root":"/absolute/docs-east"},
  "west":{"root":"/absolute/docs-west"}},"manifests":{}}
```

Then collect and check the retained declared information:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/shared_catalog.py" collect --collection collection.json --roots local-roots.json --clew "$CLEW" --output-dir inventory-r1
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/shared_catalog.py" check --collection collection.json --roots local-roots.json --clew "$CLEW" --inventory inventory-r1 --output-dir check-r1
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/shared_catalog.py" inspect --inventory inventory-r1
```

`CLEW` must be the caller-selected absolute launcher path. Collection prepares a
metadata-only frozen Work per selected service unless one was explicitly supplied.
It drains native pages and dependency parts before declaring a service complete.
`inventory.json` retains exact Work/snapshot/context/record bindings and source
authority. `objects/` retains original declared values; `inspect --record ID`
displays a selected local projection. Use IDs returned by the unfiltered inspector;
repeat `--record` for up to 64 records. Equal displayed bodies are shared without
merging their scoped identities. Configuration references remain in local originals.
Private `receipts/` contains actual commands, local paths, timing and native output;
do not publish that directory as reader content.

`check` reloads native inventory headers through each explicitly selected root.
It verifies saved object bytes and the complete selected membership against that
immutable Work. A changed local catalog does not change an old snapshot's meaning.
This is declared provenance, not SOURCE proof, runtime verification, a shared
identity decision or approval of a term definition. Entity names seed candidates;
DTO names do not automatically become additional domain concepts.

## Reuse and selected updates

For unchanged inputs, reuse the previous inventory into a new output directory:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/shared_catalog.py" collect --collection collection.json --roots local-roots.json --clew "$CLEW" --previous-inventory inventory-r1 --output-dir inventory-r1-reused
```

This checks native headers and reuses unchanged declared values. Expect zero
prepare/part calls when the previous Work remains available. After an intentional
catalog edit and supported recompose, put its returned snapshot in a new collection
revision and run the same command with that collection. Only changed value digests
need dependency-part reads; unchanged values can be reused across snapshots.
Old inventories and checked HTML are never overwritten.

Collection can return `complete:false` with per-service issues. Do not promote it
as a complete shared catalog. Failed or incomplete pagination/parts never mark
records as deleted. `check` exits nonzero when selected inputs cannot be verified.
Restore the owning retained root/closure or finish the specifically missing input;
do not recapture every service to hide a gap.

## Bind declared references in human pages

Create a separately typed citation manifest for each selected input:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/shared_catalog.py" citations --inventory inventory-r1 --input east --output-dir east-declarations
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/shared_catalog.py" citations --inventory inventory-r1 --input west --output-dir west-declarations
```

Copy each returned `manifestId` into `local-roots.json` under `manifests`, mapping
it to the corresponding input alias. These exact caller-selected assignments are
mandatory during binding. The saved manifest never supplies a launcher or root.
Binding reloads the owning immutable Work inventory and checks the original value
digest/pointer against its native header; a self-consistent edited manifest alone
cannot supply that proof.

Use the returned `d...:r...` handle in an original fragment's `declaredEvidence`
array, for example `{"id":"declared-name","text":"Authored declared information.",
"declaredEvidence":["RETURNED_DECLARED_HANDLE"]}`. Keep compiler/SOURCE fragments
in the existing `evidence` field. A fragment selects one basis; a page can contain
both kinds. Declared handles cannot satisfy SOURCE claims, and SOURCE handles
cannot silently become declared catalog references.

Continue through the same [compose/bind/finalize route](compose-original.md#original-pages-and-completion),
passing both source and declared manifests. For multiple roots, pass the trusted
mapping using `--clew "$CLEW" --evidence-inputs local-roots.json` instead of
`--root`. With `--source-footers`, the packaged renderer displays declared
references separately from actual code excerpts. A declared-only page requires
no unrelated SOURCE claim or fake captured service. Typed maps preserve native
authority and exact provenance without asserting that the prose is true.

An incremental rebind or removal still needs the existing explicit `replace` or
`remove` claim intent. Changing a declared definition, a mapping proposal and
refreshing mechanical references are separate decisions. Keep unchanged originals
and checked outputs; do not reauthor every service to refresh a common catalog.

## Establish meanings and link local details

Write one canonical decision per meaning using
[the decision schema](../schemas/semantic-catalog-decisions.schema.json).
For example, a bounded external record can explicitly map two local API profiles:

```json
{
  "schema":"codeclew-semantic-catalog-decisions/1.0",
  "collection":"platform","revision":"r1","baseDigest":null,
  "records":[{
    "id":"payments","kind":"external","title":"Payments",
    "target":{"docset":"catalog","page":"payments.html"},
    "aliases":[{"input":"east","docset":"east","localId":"provider"},
               {"input":"west","docset":"west","localId":"payments-alt"}],
    "status":"CONFIRMED",
    "decision":{"origin":"agent","rationale":"Explicit established identity from the selected declarations."},
    "original":{"page":"payments.md","fragments":[
      {"text":"# Payments\n\n"},
      {"id":"definition","text":"Authored definition from the selected information.\n",
       "declaredEvidence":["RETURNED_DECLARED_HANDLE"]}]}
  }],
  "locations":[
    {"record":"east/orders/EXTERNAL_USE:RETURNED_LOCAL_USE_ID",
     "target":{"docset":"east","page":"orders.html","anchor":"payments"}}
  ],
  "remove":[]
}
```

Replace example identities, claims, handles and destinations with actual selected
values. Supply a local detail location for every selected usage of a confirmed
system, and for each selected entity/representation context linked to a term.
Use record IDs returned by `inspect`; `process` can optionally name the matching
whole-process destination. Do not infer a process or equivalent system from its
name. Several APIs can share one explicit canonical external identity. An
unassigned external keeps zero service uses, rather than inventing a consumer.

`CONFIRMED` means the mapping is explicitly established in this workflow, not a
new approval dialog. Record its actual `human`, `agent`, `model` or `import` origin
and rationale. An authorized agent can establish an unambiguous mapping; it must
not falsely label it human. Leave ambiguity `PROPOSED` or scoped. Existing human
decisions remain protected; proposed/rejected records cannot replace them.

Concepts use `kind:"concept"`. A service-local clarification uses `kind:"local"`
and `scope:{docset,service}`; its aliases explicitly name that service.
Representations use `kind:"representation"`, the same scope and `represents`
pointing to a confirmed concept. Representation aliases must match an explicitly
retained entity representation, not an invented DTO/entity equivalence.

Keep literal authored fragments in `original`, using the existing page format.
The helper copies them unchanged into per-docset compose inputs. It does not
generate prose, make a second author call, or publish pages. A shared definition
is authored once; service addresses, configuration, contracts and operations stay
in local details. Keep the complete original question and do the same-author
scope check through the ordinary human-page workflow.

Prepare an explicit docset registry with the selected service namespaces,
publication bases and versions; the
[five-service example](../examples/markdown-renderer/docset-semantic.example.json)
shows the shape. Its catalog docset has no captured services. Then run:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/shared_catalog.py" links --collection collection.json --roots local-roots.json --clew "$CLEW" --inventory inventory-r1 --catalog decisions.json --routes routes.base.json --output-dir linked-r1
```

This rechecks owning native inventory headers and writes `routes.json`,
`catalog.json`, `local-uses.json` and `originals.DOCSET.json`. The local sidecar
keeps original declared contracts/configuration and provenance; the common
registry contains links, not a duplicate integration description. Compose the
selected originals and finalize with the declared/source manifests and the
packaged renderer configured with `--navigation`, `--docset-routes`, `--docset`
and `--docset-version`. Preserve existing landing/service originals and checked
exports; add the selected literal pages through the existing compose/update path.
The renderer checks actual local destinations and supplies canonical backlinks,
scoped terms, representation links and explicit external sequence links.

For an update, pass `--previous-catalog linked-r1` and a new output directory.
An unchanged confirmed record is reused; an unapplied proposal is saved separately.
A deliberate changed record requires `replaces` equal to its prior
`definitionDigest`. Deletion requires `remove:[{id,previousDigest}]`. Omitting a
record, losing a detail location or failing a read does not delete it. Optional
`baseDigest` pins an exact previous `catalog.json` digest. No new manual approval
is required for unchanged imports or established agent decisions.

After a local cache update, use the new selected inventory while preserving exact
old citations for unchanged definitions. They remain citations of their original
snapshots, not relabelled current proof. Changed declared values are reported for
review; a proposal never overwrites the established meaning. `complete:false`
identifies bounded unresolved mappings/targets. Unmapped candidates remain
visible and do not silently become shared definitions. Successful linking is
mechanical preparation; the full page/export and desktop/narrow reader checks
remain part of completion.

## Finish the five-service export

For the collection above, keep three original inputs: East (orders, billing,
enrollment), West (orders, enrollment), and the shared catalog. Each service
original contains its local contract/configuration sections; the catalog contains
only the common descriptions. Use `local-uses.json` to select exact retained
fields and their declared handles. Missing payloads, retries or errors remain
explicit gaps; a method/path declaration does not establish those behaviors.
Keep existing service prose unchanged when adding common links.

A location such as `east/orders.html#orders-pay` needs an actual stable section
in the East original, for example a plain fragment
`<h2 id="orders-pay">Charge customer order</h2>\n\n` followed by bound contract
fragments. Keep the ID when the title changes. Do not use a heading-derived slug
as the durable location of a mutable title.

Save existing landing/service originals as `local-originals.east.json` and
`local-originals.west.json`; save a catalog landing original as
`local-originals.catalog.json`. These use the ordinary `{pages:[...]}` format.
The shared definitions are already in `linked-r1/originals.DOCSET.json`.
Combine the page arrays without rewriting their fragments:

```sh
python3 -I -S - <<'PYTHON'
import json
from pathlib import Path
for docset in ('east', 'west', 'catalog'):
    local = json.loads(Path(f'local-originals.{docset}.json').read_text())
    common_path = Path(f'linked-r1/originals.{docset}.json')
    common = json.loads(common_path.read_text()) if common_path.exists() else {'pages': []}
    pages = local['pages'] + common['pages']
    names = [page['page'] for page in pages]
    if len(names) != len(set(names)):
        raise SystemExit(f'{docset}: reconcile duplicate original page explicitly')
    with Path(f'originals.{docset}.json').open('x') as output:
        json.dump({'pages': pages}, output, ensure_ascii=False)
PYTHON
```

Do not replace an existing common page with a second copy. For updates use the
existing original-edit route and explicit claim replacement/removal intent.
The links helper emits common originals only for docsets with canonical pages;
an external-only catalog can legitimately have none in East or West.
Save an explicit navigation file for each docset. For example, with local
`orders.md`, `billing.md`, `enrollment.md` and the representation pages:

```json
{"schema":"codeclew-markdown-navigation/1",
 "home":{"title":"East","target":"index.md"},
 "services":[
  {"id":"orders","title":"Orders","overview":"orders.md",
   "pages":["orders.md","order-dto.md"],"starts":[],"data":[]},
  {"id":"billing","title":"Billing","overview":"billing.md",
   "pages":["billing.md"],"starts":[],"data":[]},
  {"id":"enrollment","title":"Enrollment","overview":"enrollment.md",
   "pages":["enrollment.md","device-dto.md"],"starts":[],"data":[]}],
 "routes":[]}
```

West declares its two services and owns its scoped term page under the appropriate
service. Catalog navigation has `home:{"title":"Shared catalog","target":"index.md"}`,
`services:[]` and `routes:[]`; common systems are not captured services. Match
these page names to the selected originals and explicit registry destinations.
Set every docset's publication `baseURL` in the registry to its actual HTTP(S)
directory. A sample `.invalid` URL is not a working remote destination.

Copy the packaged renderer to a new owned directory and install its locked
dependency as described in its README. Save three trusted argv arrays as
`renderer.east.json`, `renderer.west.json`, `renderer.catalog.json`, each using
absolute paths. The East array has this shape:

```json
["node","/absolute/renderer/render.mjs","{input}","{output}","Service documentation",
 "--lang","en","--navigation","/absolute/navigation.east.json",
 "--docset-routes","/absolute/linked-r1/routes.json",
 "--docset","east","--docset-version","r1"]
```

Use the corresponding navigation/docset for West and Catalog. All three arrays
must use the same exact registry. Compose and finalize each selected original:

```sh
for docset in east west catalog; do
  python3 -I -S "$CODECLEW_SKILL_DIR/scripts/page_citations.py" compose --input "originals.$docset.json" --output-dir "compose-$docset" || exit "$?"
  python3 -I -S "$CODECLEW_SKILL_DIR/scripts/reader_export_check.py" finalize --compose-root "compose-$docset" --manifest east-declarations/citations.json --manifest west-declarations/citations.json --clew "$CLEW" --evidence-inputs local-roots.json --renderer-command "renderer.$docset.json" --source-footers --output-dir "final-$docset" || exit "$?"
done
```

Require all three results to contain `CHECKED_PAGES_FINALIZED` and
`exportStatus:EXPORT_CHECKED`; the loop alone is not a completion assertion.
Include additional retained SOURCE/glossary manifests when those originals use
them. Pass each exact manifest ID only once: unchanged old/new manifests can be
identical, and duplicate input manifests correctly refuse with
`AMBIGUOUS_CITATION_MANIFEST`. Preserve old citations for unchanged definitions;
add current manifests for changed local facts, assigning every ID to its owning
input in `local-roots.json`.

Deploy only each result's `renderedRoot` at the corresponding configured base.
Do not serve private roots, command receipts or entire finalization directories.
Check a common external's local backlinks and return links, a shared concept's
representation link, the differently scoped same-spelling term, an unassigned
external, and Sources from both inputs. Check desktop and narrow widths with
configuration and provenance expanded. The renderer validates local targets;
remote links still need this actual cross-docset check. Checked bindings and
navigation do not replace a review of the descriptions' meaning.

## Import an explicitly selected saved glossary

When a known approved Work already has an operation-answer glossary, read it
without a new author or reviewer. Use its exact Work/review IDs from the existing
handoff; do not discover a glossary by scanning every cached run. The underlying
native read is:

```sh
"$CLEW" docs work answer-context --root /absolute/docs-east --work EXACT_WORK --review-run EXACT_APPROVED_REVIEW --snapshot EXACT_COMPARED_SNAPSHOT --include-glossary
```

Optional repeated `--glossary-id EXACT_ID` selects up to 32 original IDs; default
reads the complete bounded glossary. The response is limited to 2 MiB. The
ordinary command without this flag retains its existing comparison response.
Original entry IDs, definitions, uncertainty, `glossaryRefs`, `subjectRefs` and
`technicalNames` stay unchanged. Packet/audit bindings are metadata references,
not delivered source content or SOURCE citation handles. The historical approval
and comparison with the selected captured Check remain separate; neither is
fresh runtime verification or a new meaning review.

For collection, write [explicit selections](../schemas/semantic-glossary-inputs.schema.json):

```json
[{"input":"east","service":"orders","work":"EXACT_WORK",
  "reviewRun":"EXACT_APPROVED_REVIEW","ids":["EXACT_GLOSSARY_ID"]}]
```

Replace identities with actual returned IDs. The collection supplies the compared
snapshot for that service. Then run:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/shared_catalog.py" glossary --collection collection.json --roots local-roots.json --clew "$CLEW" --selection saved-glossaries.json --output-dir glossary-r1
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/shared_catalog.py" inspect --inventory glossary-r1
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/shared_catalog.py" citations --inventory glossary-r1 --input east --output-dir east-glossary-citations
```

Register the returned manifest ID under the caller's evidence-input map. The
`g...:r...` handles use the same nonsource `declaredEvidence` page field; their
bindings and footer explicitly identify a saved glossary. They cannot satisfy
SOURCE claims. Binding reloads the exact approved Work/review in bounded batches;
edited saved objects or another root cannot substitute a definition.

Add a glossary alias to an explicit canonical decision using `input`, `docset`,
`service`, original `localId`, `work` and `reviewRun`. Pass
`--glossary-inventory glossary-r1` to `links`. This verifies those historical
bindings alongside current selected inventory headers. Original glossary kinds
remain queryable: a technical carrier, code identifier or uncertain business
meaning does not automatically become a domain concept. Choose a shared/local
meaning or representation explicitly, preserving qualifications and the actual
decision origin. One scoped glossary candidate cannot silently become both an
independent concept and its duplicate representation.

A missing/unapproved review, old response without a glossary, or incomplete read
is a retained input gap (`complete:false`), not an invented approval or a shared
definition. Continue from retained entity candidates when available; leave missing
common definitions proposed or author only the needed definitions through the
existing workflow. Source-only snapshot recovery does not restore old Work/review
history. No fresh capture, author/reviewer trial or broad run scan is started by
this import. Native tests/protocol fixtures establish binding behavior; they do
not establish the quality of a real model's definitions.

## Cache boundaries

Prefer original retained documentation roots. `snapshot recover --from-export`
with a selected service recovers its supported source captures and definitions;
it does not automatically restore external/entity catalogs, Work or authored
glossaries. A source-only recovery is not a recovered semantic catalog. Preserve
that gap and explicitly import available saved declarations through supported
catalog commands, or author the missing declared information within the requested
scope. Never infer missing external identities or common term definitions from
similar host names, routes or source identifiers. Absence of selected declared
records is not proof that the service has no integrations or domain concepts.
