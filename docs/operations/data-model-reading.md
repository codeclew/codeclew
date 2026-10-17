# Data model and optional field-flow reading

`docs pages render --publication PUBLICATION --snapshot SNAPSHOT --data-model PLAN`
adds authored domain pages to a full native publication render. It requires the
same explicit saved inputs as the reading plan. The original narratives, source
bytes, claims and review states remain separate. A missing data-model input adds
no pages or process Data Flow.

The versioned input contract is
[`codeclew-data-model-plan/1.0`](../../schemas/documentation/data-model-plan.schema.json).
Models distinguish domain concepts, embedded value objects and read models.
Each purpose, identity, field meaning, representation mapping and usage statement
must bind existing retained claims or selected source IDs, text digests and line
ranges. The renderer validates those bindings using the reading-plan admission
path. New mappings are `AGENT_PROPOSAL` / `UNASSESSED`; no accepted entity record,
ownership, freshness assessment or runtime trace is created by rendering them.

Contract branches identify an exact admitted JSON Schema source and JSON pointer.
Field constraints are derived from that schema. Requiredness is conditional on
the selected branch and required parents; an array element leaf is counted once,
not by cardinality. Required and nullable are independent. Unresolved composition
or references keep nullability unknown instead of inventing a type guarantee.
Representation mappings explicitly describe aliases, conversions, defaults and
missing-value handling. Related model IDs provide canonical links.

Usage statements keep producer, consumer, exact contract branch, representation
path, condition, covered scope, unresolved calls, source evidence and assessment.
Categories overlap: validation, identity/routing, decision/calculation, display,
persistence, forwarding and whole-object serialization/comparison. Whole-object
use does not establish a named business read of every nested field. A scoped
negative finding is not proof that a field is unused or safe to remove. Unknown,
not-investigated and stale observations are never converted into zero usage.
Contract counts include unmodeled required leaves as not investigated, with no
utilization percentage or inferred runtime performance.

A process Data Flow requires an explicit `processViews` entry with `enabled:true`,
a real process subject/reading anchor, requested model/field IDs and consumer
stages. Only usage statements explicitly assigned to that process and those
stages enter its projection. A global entity flow is not silently substituted.
Contract-specific representations in that projection follow its documented usage
contracts. Each canonical model page embeds only contracts referenced by its own
fields; the complete authored model and contract catalog remain in `data-model.json`.
An unrelated contract change therefore does not rewrite a canonical model page.
An enabled process view requires its explicit sequential reading plan. The existing
Process view remains the default. The accessible
Process/Data Flow tabs retain the process anchor and selected field. Canonical
entity links carry a return location for the same process step. Unconfigured
processes retain their previous payload and presentation.

A model's `processLinks` can optionally specify `replacesServiceEntities` with
the legacy service ID. The process and question anchor must identify an existing
reading-plan link to that service's `section-entities`. This explicit mapping makes
the canonical model the primary destination and retains the original service page
as Technical context, without changing retained prose or claims. It works even
when Data Flow is disabled. Links preserve the originating question and selected
field for return navigation; unrelated questions keep their existing links.

The standalone field reader offers a matrix and one focused field flow. Selecting
a cell opens its statement and bounded source evidence; contract/condition and
technical details stay in disclosures. New interpretations still require review.

The projected JSON uses `codeclew-data-model-reading/1.0`, distinct from the input
plan. Explicit process transfers reuse the `entity-dataflow/1.0` edge kinds and
authority values. Their claims bind the same immutable evidence. Missing links
remain reading order with an unestablished transfer; the reader does not invent
a handoff from adjacent nodes. Consumer-stage selection keeps narrow views
readable.

Data Flow opens with the field-by-stage overview. Its bounded inventory includes
all fields and container paths in directly referenced models, plus unmapped leaves
of the documented contracts. Selected process fields, model paths and contract
leaves have separate counts; they are not a utilization percentage. Fields outside
the selected process investigation and unmapped leaves remain unknown, rather than
unused. An explicit contract filter selects that contract's mapped fields and
unmapped leaves. Required-leaf statistics retain their separate branch scope.

Selecting a cell opens secondary field details without changing overview filters.
The explicit consumer-stage selector narrows columns. Return to the overview keeps
those filters; canonical-model navigation preserves field, contract and stage
context. The wide view uses a contained comparison table; narrow screens show every
stage in vertical field rows.
Native buttons support keyboard activation; detail and return controls move focus
to their destination. HTML and MDX share the same typography roles: 16px reading,
24px section headings and 14px technical text, with 32px reserved for page titles.

A process view may include an additive `readingGuide` with a plain-language
`purpose` and one to four short `points`. Each point names selected `fields`
(model/field pairs), selected `stages`, and a source-bound `claim` using the same
block/evidence contract as usage statements. References outside the selected
process scope or changed source bindings fail admission. The guide remains an
agent proposal, not runtime proof; it does not replace original usage descriptions.

The overview groups canonical fields and unmapped contract paths. Recorded
representations and conditions appear beside documented field operations.
Object handling is distinguished from a separate field read, and unknowns cannot
establish removable fields. Narrow screens show all stages in vertical field rows;
wide screens keep the comparison matrix. Explicit evidence actions open the
recorded claim and source, while the sticky detail toolbar returns to the selected
field/stage with filters preserved. Keyboard/DOM checks do not establish full
screen-reader certification.
