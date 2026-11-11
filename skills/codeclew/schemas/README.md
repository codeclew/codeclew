# Authoring contracts

The native authoring schemas listed below are byte-for-byte copies of their
Codeclew source contracts. They are portable documentation, not a runtime upgrade
or a new validation engine; use the contract of the admitted runtime.

| File | Use |
| --- | --- |
| [entity.schema.json](entity.schema.json) | Closed `docs entity put` catalogue record; see the [complete shape and owned examples](../references/domain-entities.md) |
| [dataflow.schema.json](dataflow.schema.json) | Canonical materialized graph, not Proposal input; see [retained preparation and exact domain node invariants](../references/retained-view-work.md) |
| [source-part-request.schema.json](source-part-request.schema.json) | `docs work read-part` input: schema, one SOURCE reference, optional cursor; no projection |
| [source-part.schema.json](source-part.schema.json) | Recorded SOURCE part response, not request input |
| [work-batch-request.schema.json](work-batch-request.schema.json) | `docs work read-batch` input: 1..64 tracked SOURCE selections, including `SOURCE_ROWS` |
| [work-batch.schema.json](work-batch.schema.json) | Per-selection page or error response; each page has independent cursor and omitted rows |

See [complete SOURCE reads](../references/source-read-parts.md) for copyable
requests, pagination, own-body accounting and old Kotlin capture behavior.

These schemas contain no remote `$ref` dependencies. The command-specific validator also enforces Work
identity, read coverage, claim evidence, root-specific shapes, UTF-8 limits and
required FLOW coverage. A generic schema or syntax check cannot establish those
facts or meaning review. Use the contract selected by the installed command and
its actual job; do not interchange a Proposal, operation answer, role response or
materialized Narrative.

The optional task-oriented reader extensions also ship `external-service.schema.json`
and `data-model-plan.schema.json`. Keep their exact native copies in every skill mirror.

Helper-owned inputs also include [evidence-inputs.schema.json](evidence-inputs.schema.json)
for caller-selected roots/manifest assignments and
[semantic-collection.schema.json](semantic-collection.schema.json) for explicit
frozen catalog selections, plus
[semantic-catalog-decisions.schema.json](semantic-catalog-decisions.schema.json)
for established decisions and literal authored originals.
[semantic-glossary-inputs.schema.json](semantic-glossary-inputs.schema.json)
selects exact historical Work/review glossary inputs without searching runs.
See [shared catalog inputs](../references/shared-semantic-catalog.md).
These helpers do not turn declared information into native SOURCE evidence.
