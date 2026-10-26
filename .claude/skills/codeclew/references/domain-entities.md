# Register a domain entity

`docs entity put` takes the [entity schema](../schemas/entity.schema.json), an
exact copy of the supported product contract. Use schema
`codeclew-documentation-entity/1.0`; `domain-entity` is not a supported namespace.
This is a catalogue declaration, not a Work Proposal or an operation answer.

## Describe supported objects even when their owner is unknown

Unknown master ownership is a local gap, not a reason to leave business data
undocumented. Establish a concept from the selected behavior and field meanings;
keep its business identity or lifecycle unknown where the evidence does not
establish them. Do not equate equal class names with the same business object.
A source-proven read can use relation `kind:"read"`, `role:"unknown"`,
`origin:"agent-proposal"` and `confidence:"supported"`; explain the exact read and
unknown ownership in its rationale. Use `owned` or `master` only with evidence.
Register that concept with `docs entity put`, recompose the captured snapshot,
and explicitly bind its canonical model using `entity:"entity:<id>"`.

If only a representation is established, describe it as such instead of inventing
an entity. An explicitly authored VALUE_OBJECT or READ_MODEL may use a
`processLinks` item with the exact service subject and `anchor:"section-entities"`
without an `entity` association. Its business meaning and fields remain visible;
its service role/ownership stay unknown. A DTO catalogue alone is not a business
catalogue: group justified aliases as representations of one scoped attribute;
keep copied objects, historical versions and derived values separate, with their
specific mapping or unresolved relationship. Bind models deliberately; do not
promote every discovered type or group fields by spelling.

Use the current frozen publication and snapshot in a data-model plan, then render
through the supported command:

```sh
clew docs recompose --root ROOT --snapshot SOURCE_SNAPSHOT
# Use the returned snapshot and matching publication identity below.
clew docs render --root ROOT --snapshot SNAPSHOT --publication PUBLICATION \
  --data-model MODEL_PLAN.json --output NEW_EXPORT
```

Open the service's domain landing and an object. Verify that business meaning,
fields and their source representations are reachable. An empty landing while
supported authored objects exist is a failed reader task, even if export succeeds.

The six required fields are `schema`, `id`, `title`, `description`, `relations`
and `limitations`. `relatedEntities` is optional and defaults to an empty list.
Every object is closed: additional fields such as `identifiers`, `declaration`,
`lifecycle` or `evidence` are rejected. Describe evidenced business identity,
rules and lifecycle in `description`, preserving unknowns in `limitations`.
Provenance belongs on each relation, not a root `declaration` object.

The [minimal owned input](../examples/domain-entities/minimal.json) is directly
usable in a caller-owned file:

```json
{
  "schema": "codeclew-documentation-entity/1.0",
  "id": "reservation",
  "title": "Reservation",
  "description": "An explicitly declared business reservation concept. Its business identity and rules require evidence.",
  "relations": [],
  "relatedEntities": [],
  "limitations": ["Service responsibility and lifecycle have not been established."]
}
```

An empty `relations` array registers the identity without asserting a service
relationship. It does not make the concept appear as that service's domain
responsibility. For a supported service relation, see the
[owned service-relation input](../examples/domain-entities/service-relation.json).
Replace its registered service ID and declared rationale using actual evidence.
Each relation requires all five fields below:

```json
{
  "service": "orders",
  "kind": "read",
  "origin": "agent-proposal",
  "rationale": "Owned example of a declared read relation; assess it against selected source before accepting meaning.",
  "confidence": "declared",
  "representations": [],
  "dependencyIds": []
}
```

- `kind`: `created`, `changed`, `read`, `stored-copy`, `owned`, `deleted`, or `enriched`. Optional `role` is `master`, `consumer`, `enricher`, or `unknown`.
- `origin`: `human` or `agent-proposal`. Use `agent-proposal` for a relation
  authored or inferred by the agent, including when its source or business brief
  came from a human. Use `human` only for an exact relation declaration actually
  supplied by a human. Ordinary writes preserve existing human relations.
  `--human` is for an explicitly authorized maintainer edit, not a workaround
  for a validation error or a way to gain traversal authority.
- `confidence`: `declared`, `supported`, or `uncertain`. These are declaration
  labels, not native meaning approval or observed runtime ownership.
- `representations` and `dependencyIds` are optional arrays of strings. Dependency
  IDs are exact saved dependency identities, not Work-local `s1` handles or a
  newly invented evidence object. Missing dependencies stay visible after
  recomposition. DTO/table representations do not establish domain identity.

Every relation's service must already be registered. Every `relatedEntities`
entry must be a distinct, already registered domain ID; register independent
identities first, then add links. IDs use `[A-Za-z][A-Za-z0-9_-]{0,99}`.
Native budgets include 512 UTF-8 bytes for title, 8192 for description, at most
128 relations, 64 related entities and 64 limitations. Relation rationale is
nonblank and at most 4096 UTF-8 bytes. Consult the bundled schema for remaining
array limits; syntax/schema validity does not establish business meaning.

Copy an example to the caller-owned output directory, edit that copy and read
the current catalogue before writing:

```sh
clew docs entity list --root DOCS_ROOT --limit 100
clew docs entity put --root DOCS_ROOT --input ENTITY.json \
  --expected-input-digest EXACT_CURRENT_INPUT_DIGEST
clew docs entity list --root DOCS_ROOT --limit 100
```

Pass the list result's exact `inputDigest`, including for an empty catalogue.
The saved result is `SAVED` with a new input digest. Generic closed-schema
errors require comparing the input to this record shape, not guessing new
field names or editing private state. After catalogue edits, recompose from
the original captured snapshot and prepare new Work using the returned snapshot.
For a saved typed view, follow [retained-view Work preparation](retained-view-work.md)
rather than a latest-only view shortcut after recomposition.
Keep earlier rejected attempts and model/review history. The command itself
performs no model dispatch or semantic approval.
