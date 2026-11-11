# Read a typed view from retained evidence

Use an exact saved snapshot after entity, process, interaction or view declarations
change. `docs recompose` applies those declaration changes to the original capture;
it does not capture source or move the latest-check pointer. If a convenience
command selects latest, prepare generic Work with the exact returned snapshot.

1. Save the intended declarations through their supported commands.
2. Run `docs recompose --root DOCS_ROOT --snapshot ORIGINAL_CAPTURE` and retain
   its returned snapshot. Partial source coverage remains explicit.
3. Save a bounded Work request for the existing view:

```json
{
  "schema": "codeclew-documentation-work-request/1.0",
  "audience": "Domain and service maintainers",
  "entrypoint": "entity-dataflow",
  "maxItems": 20,
  "maxBytes": 40960
}
```

```sh
clew docs work prepare --root DOCS_ROOT --subject scenario:VIEW_ID \
  --snapshot EXACT_RECOMPOSED_SNAPSHOT --input view-work-request.json
```

Read the returned context, source and dependency references, continuing cursors
and oversized-record parts. Preparation and navigation references do not establish
complete evidence delivery or meaning approval. A missing body remains a precise
gap; no fresh capture is needed merely to read sufficient retained evidence.

To explain the view, use [authored originals and checked HTML](compose-original.md#frozen-source-to-reviewed-pages).
Describe the entities, actual representations, transformations, writes and
boundaries supported by the selected source. Keep source references, provenance,
conditions, unknowns and failure behavior beside the relevant action. The former
native Proposal and typed-view document publisher are retired.

## Preserve declared identities and local boundaries

Use the exact registered entity identity when a domain association is supported.
A DTO, technical field or correlation value does not become a business entity
because its name resembles one. Keep absent or uncertain associations explicit.
Separate declared ownership, source interpretation and runtime observations.

A captured client call can establish its caller-side construction, payload,
parsing and guard. It does not establish the receiving service's implementation,
wire compatibility or observed remote effect. Link an available receiving fragment
and contract; otherwise retain that boundary as unknown. Do not fabricate a
captured service or change an interaction's declared origin to complete a graph.

The [dataflow schema](../schemas/dataflow.schema.json) describes retained structured
graph records with definition/module digests and source bindings. It remains a
data carrier, not an authored-original document or a new publication command.
Existing graph records and historical exports remain inspectable; new prose and
updates follow the same compose/bind/finalize pipeline.
