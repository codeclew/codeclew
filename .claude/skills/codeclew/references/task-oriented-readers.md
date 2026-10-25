# Task-oriented documentation authoring and review

Use the existing evidence collection, Work proposal, validation and publication
workflow. This checklist changes the reader outcome, not the approval lifecycle.
Draft missing semantic content with the assigned author model. Keep independent
review separate from authorship. Never turn a protocol PASS, object count or
non-overlapping screenshot into semantic or UX acceptance.

## Four reader tasks

1. **Objects:** From the domain landing, identify the business objects, actual
   code representations, service roles and scoped actions. One detail action
   reveals English field paths, business-language labels and meanings, then usage.
   A source popup must not be required to understand the fields.
2. **A result field:** Choose an operation and model/field. Explain the actual
   inputs, transformation/default, condition, output and destination. Selection
   focuses the incoming/outgoing context; opening source is a separate action.
3. **Dependencies:** From egress, identify systems/APIs, logical storage, queues,
   configuration-only profiles and unresolved destinations. Show operations under
   each destination, known method/path/config key and supplied address provenance.
4. **Process:** Read a compact overview, then inspect branches/errors locally.
   Distinguish synchronous work, waiting, parallel waiting and background work
   only when the selected source supports that interpretation.

## Content and additive contracts

Use the requested documentation language for business names, meanings, operation
labels and authored hypotheses. Retain actual code terms, paths, config keys and
protocol names exactly. For Russian documentation, an English business-purpose
sentence is a content defect even when its field path is correctly English.
Long repeated uncertainty paragraphs are also a defect: keep one compact evidence
legend and essential local conditions, with provenance and full originals in
secondary details. Do not strip text heuristically to manufacture a short label.

- Keep `entity/1.0` IDs and relations. Optional relation `role` is `master`,
  `consumer`, `enricher` or `unknown`; action kinds also include `deleted` and
  `enriched`. Existing origin/confidence/rationale/dependency IDs retain authority.
  A DTO is a representation, not an ownership declaration. Missing actions mean
  unknown, never proven absence. An uncertain master role must remain a hypothesis.
- Reuse `data-model-plan/1.0`, its exact publication/snapshot/source claims,
  `readableName`, `originalName`, `meaning`, representations and usage states.
  Optional model `entity` explicitly names a selected declared `entity:<id>`.
  Do not associate models by name equality or publish the entire DTO inventory
  as domain objects. Conceptual objects may have no canonical code type.
- Optional model `fieldCoverage` has `status` (`complete-definition`,
  `observed-fields`, `unknown`), `scope` and an evidence-bound `claim` block.
  Use complete-definition only for an exact retained complete definition. A
  nonempty field list or many observed field uses never proves completeness.
- Optional DataFlow node `readerRole` is `input`, `output`, `stage`,
  `side-effect` or `object`, supported by its existing node meaning claim.
  Absence gives a plain object/value selector; graph topology must not invent
  semantic input/output roles or execution stages. Stage display is unordered.
- Optional field `graphBindings` contains `{subject,node,claim}`. The subject
  must be an explicitly linked process; the node must exist in that exact saved
  graph and the mapping claim must bind admitted evidence. This is an explicit
  navigation mapping, not proof that every object edge causes every field.
- Optional usage `inputs` lists exact input paths backed by the existing usage
  claim. The selected field is the output; existing claim text, condition,
  producer/consumer, stage and process scope explain the rule and destination.
  Group a multi-input transformation once. Distinguish observed use, explicitly
  bounded complete-scope non-use, uninvestigated, dynamic and stale evidence.
- Optional proposal step `reader` has `overview`, `label` (ordinary Work claim)
  and `execution: {mode,meaning}` (another Work claim). Modes are `synchronous`,
  `wait`, `parallel-await`, `background`, `unknown`. A compact label is at most
  160 UTF-8 bytes. Set `overview:true` only on explicitly chosen meaningful
  stages; preserve validation/version branches, required waits, distinct calls,
  save flags, background work and response/failure outcomes. The complete event
  text and algorithm remain retained. HTTP, optionality or a return arrow alone
  never proves waiting or background execution.
  Reader evidence is separate from the original event evidence and explanation
  closure. A declared boundary label may cite that event's exact delivered
  declaration without SOURCE evidence, or cite a separately admitted caller
  source claim; neither changes boundary authority. Execution always needs a
  caller source claim and cannot be established from a declaration alone.
  An `alt` with nonempty `otherwise` may set optional `otherwiseReader` using
  the same reader shape for its generated else marker. Parent label, overview
  and execution never propagate to else/end; end has no reader metadata.
- Optional external-service `category` is `system`, `storage`, `queue`,
  `configuration` or `unknown`. Optional `identity` has stable grouping key,
  title, profile, proposed/confirmed/rejected status, declaration and dependency
  IDs. Proposed grouping is visibly a hypothesis, not proven physical identity.
  Keep original external IDs, endpoint/config profiles and interaction targets.
  Rejection stops grouping while preserving the original profiles and links.
  Use existing external-service PUT with expectedInputDigest; human-confirmed
  decisions require human provenance and `--human`.
- Optional external-service `references[]` has stable `id`, `title`, `kind`
  (`wiki`, `document`, `code`), clean HTTPS `location`, `status` (`unverified`,
  `reviewed`, `stale`) and declaration provenance. Keep legacy wiki/repository
  links. Supplemental references do not become captured implementation evidence
  or improve source/runtime confidence. Do not fetch them automatically.
- Interaction transport may include declared `messageKey` and `purpose`.
  Preserve topic/config evidence. Never fabricate consumer delivery, broker ACK,
  a business response, a concrete resolver, physical database or durable commit.

These optional additions require a qualified runtime that supports them. Older
runtimes reject unknown properties: do not silently drop fields or manually edit
private cache objects. Canonical schemas and all portable copies must agree.

## Common evidence mistakes

A type or persistence declaration establishes a field definition, not a read of
that field by this service. Query-filter values from the current request must
not be attributed to reads of returned or stored entity fields. For example,
`repository.find(request.ownerId)` supports a request-filter input; it does not
prove reading `result.ownerId`. Kotlin type nullability, persistence-column
nullability and wire requiredness describe different scopes and must stay
separate. Keep the actual code type/name in the code-term column; a registry
slug or prose navigation path is not a code type.

## Bounded critic rubric

Use fresh screenshots and actual interactions against the exported artifact.
Report the attempted task, answer obtained, clicks/scroll burden, ambiguity and
blocker. Review the default state and one detail/selection state at 1280 and
390 px (or the closest available viewport). Recheck only affected states after a
material delta; preserve accepted original evidence and failure reports.

- **D1–D4:** Domain catalogue first; no egress leakage; code representation is
  separate from concept/ownership; fields, roles, CRUD/enrichment and unknowns
  are understandable without reading provenance IDs. Partial definitions visibly
  remain partial. Do not infer storage/durability from a DTO or save attempt.
- **F1–F4:** No all-edge default hairball. Pointer and keyboard select context,
  with incoming/outgoing direction, reset/one-hop expansion and text equivalent.
  Source action is explicit; Escape restores selection or closes source first.
  Field rules and conditions answer task 2, rather than listing internal IDs or
  repeating one transformation for every incoming arrow. Unknown is not unused.
- **E1–E6:** Grouped inventory leads egress; operations/config evidence are
  readable without raw JSON; all Kafka chrome uses the requested language and
  labels do not concatenate. Identity hypotheses/profile separation and multiple
  references are visible and editable through supported lifecycle commands.
  Remove the default redundant relationship hairball; overview has a compact
  dependency entry, domain and ordinary processes have no full external list.
- **P1–P3:** Compact, engineer-readable call/return/branch overview; one evidence
  legend with local material uncertainty. Required waits, parallel waits,
  background work, distinct formula calls/save flags and response/error branches
  stay distinct. No semantic clipping, text overlap or accidental page overflow;
  intentional graph scrolling has a discoverable label and keyboard access.

Reject these plausible but insufficient outcomes: technically valid JSON;
correct counts; screenshots showing only an empty default; all unknown
placeholders; untranslated business prose; a long explanation renamed Overview;
a focused object graph presented as exact field-level lineage; a partial glossary
presented as all fields; a model-approved paragraph with no successful reader task.
A source-unavailable historical interpretation must keep its limits, while still
answering the bounded question using the retained evidence available.

- F1 regression: inspect an expanded high-degree graph and one further hop;
  a collapsed default does not pass if the opened context is a hairball.
- P3 regression: mobile must connect sender, message and receiver without panning
  between them; working horizontal scroll alone is not a pass. An equivalent
  adaptive trace is acceptable.
- P2 regression: inspect deeply nested middle and end trace calls and returns; initial cards alone are insufficient.

For canonical fields, current/history scope, payloads, static configuration and
normal/debug reader-answer gates, use the [semantic reader contracts](semantic-reader-contracts.md).
Keep business meaning separate from occurrence inventories and source uncertainty.
