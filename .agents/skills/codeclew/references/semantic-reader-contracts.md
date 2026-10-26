# Semantic objects, payloads and static configuration

Apply these rules to a requested semantic reader task alongside the existing
[task-oriented readers](task-oriented-readers.md) and normal evidence lifecycle.
Do not convert a translated table, many rows or a protocol PASS into acceptance.

## Object-scoped attributes

Reuse `data-model-plan/1.0`: `models[].fields[]` are canonical attributes;
`representations[]` and `usage[]` explain their exact code appearances and uses.
Use one field for a justified attribute even when several DTO names or operations
carry it. Preserve every representation, mapping claim and scoped use. Never
collapse fields by spelling, type, path suffix or business label.

Keep three different questions separate: same business attribute, copied/equal
value in a selected mapping, and same object identity. Current object and
historical record have distinct object-scoped attributes even when a value was
copied unchanged. A reference to another object belongs to its containing record.
A shared vocabulary or copy relationship is separate from canonical identity.

Year and derived age remain different. Two mileage representations may belong
to the same attribute without equal values across unrelated requests. Conversely,
identical `request.year` paths in different scopes can mean different attributes.
When evidence cannot decide a mapping, retain a named unresolved or proposed
relationship, its exact basis and the missing evidence. Do not silently assert
an alias or a separate business attribute. An observed read remains distinct
from a proposed concept mapping or ownership claim.

Business names and concise meaning come first. Exact DTO paths, units, types,
nullability, representation-specific constraints and source evidence belong in
mapping/usage details. Keep material ambiguity beside the field. Routine caveats
about exhaustive enums, time zones or unestablished identity remain secondary.
Review the complete requested catalogue: fixing one year example does not account
for every original field. Track each original field/use to its canonical target,
explicit copy/derivation relation or named unresolved mapping.
The author/orchestrator derives and freezes that bounded source-to-attribute
inventory autonomously from the authorized question and selected evidence before
dispatch; no new user approval or user-supplied manifest is required. A shorter
self-selected inventory cannot redefine the requested scope. Use the same
inventory and question in normal and debug mode. Include local
and history types, copied representations, formulas and named unresolved inputs
when the selected evidence contains them. A missing framework route does not
make available method/type semantics unavailable: retain honest compiler/SYNTAX
limits without dropping those parts of the requested reader. Do not invent a
framework entrypoint or an empty operation to bypass a native gate.

For this semantic reader outcome, author an evidence-bound `readableName` for
every canonical field in the requested documentation language. Optionality in the
general schema does not make an absent business label acceptable here. A path
such as `record.measuredValue` is an exact code representation, not a translated
business name. Preserve it in `originalName`/representations; do not translate
or rename source identifiers. Review whether the authored name is actually
supported, including technical fields that have no established business meaning.
For an unresolved technical attribute use a specific neutral readable label and
retain its unresolved meaning proposition; never invent a business interpretation
to satisfy label presence.

## Source-derived payload views

Prefer existing registered declared schemas and `contracts`/`interfaceBindings`
for exact OpenAPI/JSON Schema declarations. Source type definitions and mapper
accesses are different evidence; nullable source type does not mean optional wire
property. Positional arguments never establish exhaustive generated Avro fields,
message key position or consumer schema.

`data-model-plan/1.0` optionally accepts `payloadViews[]` over the exact selected
publication and snapshot. Each view names `id`, `title`, `subject`, exact
`operation`, `component`, `direction`, `format`, `coverage` and `fields`.
Components are `request-body`, `request-path`, `request-query`, `request-headers`,
`response-body`, `error-response-body`, `message-key`, `message-headers` and
`message-value`. Direction is `incoming`, `outgoing` or `local` relative to the
selected service. An optional exact `interaction` links an external profile;
request/response side and direction must match that local service. Optional
`status` belongs only to a response component.

A real retained ingress/egress catalogue section may own a payload view even
when its operation has zero events and no authored business-process trace.
For an outgoing topic, bind the actual service `section-egress` operation and
the exact outgoing `interaction`; cite the selected mapper/source evidence and
state partial or unknown schema coverage. The external profile links to this
section's data view. This describes exchanged data, not an ordered trace. Do not
invent a process, attach the view to an unrelated process, or create a fictitious
section merely to satisfy validation. A catalogue gap without a retained owner
operation is not an eligible owner.

Each field selects an existing `model`, `field` ID and exact `representation`
label, with a source-supported observed `path`. No fuzzy representation selection
is performed. The view reuses type, presence, nullability, meaning and mapping
claims from that representation; it does not synthesize a wire schema.

Coverage reuses the source-bound `status`, `scope` and `claim` shape. Use
`observed-fields` for partial mapping, `unknown` with zero fields for a missing
definition, and `complete-definition` only with complete selected model definitions
and every field. Complete type coverage still does not establish complete wire
schema, serialization or runtime conformance. Separate normal and error response
views. Say exactly which definition is missing, retaining useful observed fields.

When the task requests inputs/results/errors or exchanged data, inventory each
in-scope operation and author its supported payload views; a behavior card or
separate unlinked DTO page does not meet that request. Reuse the exact existing
canonical fields and representation labels, including retained nested paths and
type/presence/nullability limits. Keep normal/error results separate. A local
method result or status is not HTTP: use local direction and label its source
scope without inventing serialization, a network response or a wire declaration.
Do not turn an available type definition into unknown coverage to avoid mapping.

Before export, run the bounded preflight from the installed skill directory:

```sh
python3 -I -S SKILL_DIR/scripts/reader_plan_preflight.py --plan MODEL_PLAN.json \
  --service service:sample --payload-owner service:sample/apply \
  --expected-field record/value --nested-field request/record=record
```

Repeat `--payload-owner` for each exact owner selected by the requested task.
Supply the complete scoped canonical target inventory with repeated
`--expected-field MODEL/FIELD`; this is not a name-matching source extractor.
Use `--nested-field MODEL/FIELD=TARGET_MODEL` for each source-supported nested
definition that needs navigation. Set the representation's exact `relatedModel`,
not only a type name or a prose mention. Every representation-only model needed
on the service landing needs an explicit `processLinks` item for that service
and `section-entities`; a domain model instead needs its exact entity association
and retained service relation. Do not guess association from class names.
This checks label presence and selected inventory/navigation only. Repair findings through
the assigned author, then run native export and independent rendered review;
neither this check nor an export success approves language or business meaning.
Run the same requested inventory checks for normal and debug plans. Independently
compare the actual reader answers: a different mode or shorter packet does not
authorize dropping copies, historical attributes, formulas or unresolved mappings.

## Static configuration references

External-service records optionally accept `configurationReferences[]`. Each row
requires `id`, literal `key`, `binding`, authored `scope`, nonempty `sources` and
`declaration` provenance. A source binding is `{service,sourceId,textDigest}` from
the selected Check. Source mismatch/unavailability must remain visible beside
the key; missing evidence is not an unconfigured service.

Bindings are `static-reference`, `unresolved-reference` or
`explicitly-unconfigured`. The last is a specific sourced declaration, never a
default inferred from missing capture. A `static-reference` needs a `reference`
with `kind` (`environment-placeholder`, `property-placeholder` or `code-symbol`)
and a static `name`. Do not put resolved values into this structure.

Keep base/profile resource references separate. Filename/profile scope is not
active profile selection or precedence. Retain key/reference/location when
available through a supported selected resource path. Never read live environment,
Vault, etcd, network configuration or secrets to fill a documentation gap.
A source-text capture with no typed binding projection is different from lost
resource evidence; prove an acquisition defect with a bounded generic case first.

## Business decisions and targeted missing evidence

Explain the actual check, data meaning/unit, combined predicate, consequence and
source-supported contrast. A default passed into a provider is not the current
runtime flag value. Do not invent the backend or business rationale from a flag
name or numeric threshold.

Before calling meaning/status/fields unknown, resolve the specific named handler,
mapper, definition or version-service method in the already selected evidence.
Use bounded targeted lookups, not full inventory/context replay or recapture.
A one-shot author cannot read outside its admitted packet: the coordinator supplies
the exact supplement before dispatch. If the definition remains absent, name that
missing fact without replacing available source-backed facts by a vague unknown.

Preserve failure boundaries: mapping before `try` can fail outside the send catch;
a caught send failure does not mean the call was never attempted. Ignored response,
send attempt, successful receiver processing and durable persistence are separate.
Timing is a textual annotation, not another source/disclosure action.

For normal/debug acceptance use the new [public example](../examples/semantic-reader/README.md).
Require independent rendered answers for equivalent, distinct and unresolved
attributes, current/history copies, payload coverage and contrasting predicates.
The debug result keeps the same answer and adds only bounded identities/lookups
and checks. Reject invented business labels and an unjustified unknown when the
handler/definition is available outside the initial fragment. Preserve rejected
attempts and their costs; no automatic semantic PASS from counts or assertions.
