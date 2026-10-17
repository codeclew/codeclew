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
An enabled process view requires its explicit sequential reading plan. The existing
Process view remains the default. The accessible
Process/Data Flow tabs retain the process anchor and selected field. Canonical
entity links carry a return location for the same process step. Unconfigured
processes retain their previous payload and presentation.

The standalone field reader offers a matrix and one focused field flow. Selecting
a cell opens its statement and bounded source evidence; contract/condition and
technical details stay in disclosures. New interpretations still require review.

The projected JSON uses `codeclew-data-model-reading/1.0`, distinct from the input
plan. Explicit process transfers reuse the `entity-dataflow/1.0` edge kinds and
authority values. Their claims bind the same immutable evidence. Missing links
remain reading order with an unestablished transfer; the reader does not invent
a handoff from adjacent nodes. Consumer-stage selection keeps narrow views
readable.
