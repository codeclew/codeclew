# Select a retained operation without guessing IDs

Use this route for ordinary source-backed operation/sequence authoring, including
source-syntax callables without a compiler scope. It uses saved evidence and
does not capture source, infer a framework trigger or elevate authority.

## Four identities have different consumers

| Delivered identity | Meaning and supported use |
| --- | --- |
| `BOUNDARY_INVENTORY.internalCallables[].dependencyId` or a `SYMBOL` observation ID | Declaration evidence; match it to an entrypoint's `record.dependencyIds`. It is not `request.entrypoint`. |
| Inventory/declaration `symbol` | Exact qualified source/compiler identity; match the entrypoint's `record.symbol`, retaining overload and source identity. It is not `request.entrypoint`. |
| An `ENTRYPOINT` row's `record.id` | Native operation identity, also exposed as that row's `id`. Use this exact value in a new Work request's `entrypoint`. |
| That row's `reference`, such as `e7` | A handle local to this Work. Use it in this Work's recorded reads and manual Proposal operation `entrypoint`, not as a new prepare selector. |

Never derive an operation ID from a hash, copy one from another service/arm, or
replace the service prefix in an observed ID. Same source bytes under different
registered service IDs do not establish identical entrypoint IDs.

## Discover the operation in ordinary service Work

If an existing ordinary service Work already delivered matching `ENTRYPOINT`
rows, reuse them. A section orientation/egress Work can instead show only boundary
inventory/declaration seeds; those IDs are not the operation catalogue.

For discovery, save this ordinary request without `entrypoint`, `contextProfile`,
`rootDeclaration`, `question` or `authoringContract`:

```json
{
  "schema": "codeclew-documentation-work-request/1.0",
  "audience": "Source operation author",
  "maxItems": 100,
  "maxBytes": 49152
}
```

```sh
clew docs work prepare --root DOCS_ROOT --subject service:SERVICE_ID \
  --snapshot EXACT_CURRENT_SAVED_SNAPSHOT --input discovery-request.json
```

Use the current compatible snapshot; after declaration edits, select the snapshot
returned by recomposition from the original capture. Follow initial Work pages
and cursors, preserving `omitted` and delivery limits. No new model call is
implicit in discovery. Match the desired `ENTRYPOINT` by its exact symbol and/or
declaration dependency/source IDs. A title or method name alone cannot resolve
overloads, distinct owners or registrations. If no row matches, retain that finite
catalogue gap; do not manufacture an ID or repeatedly change profiles.

Within that Work, read the selected entrypoint using its returned local handle:

```json
{"references":["EXACT_ENTRYPOINT_WORK_REFERENCE"]}
```

```sh
clew docs work read --root DOCS_ROOT --work DISCOVERY_WORK \
  --input entrypoint-selection.json
```

Follow actual source/helper/dependency reads and pagination. The ordinary service
Work can remain the authoring Work. If a focused operation Work is needed, add
`"entrypoint": "EXACT_ENTRYPOINT_RECORD_ID"` to the ordinary request and prepare
against the same current snapshot/subject. Read the focused Work's own entrypoint
and evidence handles before submitting its Proposal. Retain the discovery and
any failed requests; do not transplant Work-local references between them.

## Exact requests for omitted retained and dependency records

These part requests require `schema`, `kind` and `id`; `cursor` is optional.
The IDs below are synthetic. Replace them with the exact delivered record ID
from this immutable Work, not a Work-local reference such as `e7` or `d4`, a
newer narrative's ID, or an ID copied from another Work.

Save a retained operation request as `retained-part.json`:

```json
{"schema":"codeclew-documentation-retained-part-request/1.0","kind":"RETAINED_OPERATION","id":"synthetic-reserve","cursor":null}
```

```sh
clew docs work read-retained-part --root DOCS_ROOT --work EXACT_WORK_ID \
  --input retained-part.json
```

The operation must exist in this Work's retained narrative for its subject. If
the Work selected an `entrypoint`, the retained operation ID must equal it.
This route cannot widen that selection or follow a newer baseline.

For an omitted dependency, save `dependency-part.json` with the distinct schema:

```json
{"schema":"codeclew-documentation-dependency-part-request/1.0","kind":"DEPENDENCY","id":"orders:symbol:synthetic-helper","cursor":null}
```

```sh
clew docs work read-dependency-part --root DOCS_ROOT --work EXACT_WORK_ID \
  --input dependency-part.json
```

The dependency must belong to this immutable Work and its admitted expansion
scope; a known handle alone does not authorize another service's dependency.
For either route, keep the same schema, kind, ID and Work, replacing `cursor`
only with the exact `nextCursor` returned by that part response. Do not reuse a
Work-page cursor, derive offsets or fabricate continuation tokens. Continue
until `nextCursor` is explicitly null; retain the fragments, ranges, digests and
receipts. A failed or incomplete read stays incomplete. Retained narrative
parts are not freshly verified source evidence, and dependency parts remain
immutable captured evidence rather than a new source check.

## Check sequence text bytes before full submission

Every step's `meaning.text`, including nested `children` and `otherwise`, must
fit **1024 UTF-8 bytes**: measure `len(text.encode("utf-8"))`, not characters.
For example, 512 two-byte characters fit; adding one ASCII byte makes 1025 bytes.
Shorten the label or, on a full Proposal route, split an oversized note into
supported steps while preserving meaning, evidence, branch placement and all
operation step/event bounds, including any stricter task cap. Never truncate
silently or change IDs to fix length.

The [existing semantic-repair preflight](semantic-repair.md#repair-assemble-and-validate)
reports oversized step paths and byte counts on that repair route. Its bounded
replacement allowlist still applies; it does not authorize new structural steps.
Ordinary `work_session.py preflight` currently checks participants, not this text
budget. Neither partial preflight establishes native READY or meaning approval.
Older runtimes can report a text overflow as `unsafe or duplicate sequence
fragment ID`; that message alone is not proof of an ID error. Preserve failed
input and diagnostics, check byte counts, and repair only the actual defect.

## `process-graph-v1` has a separate admission contract

That profile selects an exact retained callable `SYMBOL` through `rootDeclaration`.
The native consumer requires a callable observation, owner identity and a nonblank
exact `scope`, all in the selected service/snapshot. An inventory scope of `""`
or an observation with no scope cannot satisfy this gate. The ordinary entrypoint
route above can still support source-based authoring in that capture.

Do not switch to `process-graph-v1` merely because `request.entrypoint` rejected
a declaration ID, and do not invent a scope or change capture authority to force
admission. A scoped declaration is also not by itself proof of runtime dispatch;
preserve the selected profile's actual authority and all unresolved boundaries.
