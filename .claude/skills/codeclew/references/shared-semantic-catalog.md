# Shared semantic catalog inputs

## Collect retained declarations

Start with the existing documentation roots and exact retained snapshots. Select
only the services needed for this collection. Roots remain independent; the same
service or external ID in two roots does not establish shared identity.

Use installed `clew` with `semantic-inventory-v1` support and the matching skill.
An older runtime refusing this profile needs a supported upgrade, not recapture.
Keep a running trial on its admitted runtime. The collector never captures,
reads SOURCE bodies or materializes a source checkout. Its counters report actual
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
