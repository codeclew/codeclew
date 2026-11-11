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
