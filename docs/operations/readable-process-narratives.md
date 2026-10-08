# Author readable process narratives

Use a detailed sequence operation when readers need an evidence-backed account
of a selected entrypoint's decisions, actions and outcomes. The current narrative
contract can express concise branch labels and linked explanations; it does not
require a sentence enumerating every source construct or helper call.

## Structural coverage

The sequence validator requires the following source-backed FLOW coverage for
the selected operation:

| FLOW kind | Allowed event kind |
|---|---|
| IF, TRY | alt |
| DEFERRED | opt |
| LOOP | loop |
| FINALLY, BREAK, CONTINUE | note |
| RETURN, THROW | return or note |

CALL is optional. Include a call or business note when it explains a material
action, external boundary or effect. Its optional status does not justify
discarding useful supported behavior. Every mandatory FLOW still needs an event
of an allowed kind whose dependencyIds include that FLOW. Citing it only in the
summary or explanation does not satisfy structural coverage.

Author jobs receive the same coverage as `sequenceGuidance.mandatoryFlowCoverage`,
using Work references and `allowedStepKinds`. The matching step's meaning evidence
must materialize to the required FLOW. A delivered ENTRYPOINT or SOURCE reference
can cover it when materialization includes that FLOW; a navigation-only reference
cannot be cited. Request a registered expansion when it can supply missing proof.
If no evidence can cover a mandatory FLOW, use a permitted operation gap.

## Write the behavior, preserve the evidence

Name a supported condition or outcome in each label. For example, synthetic
source evidence for a readiness check and an early return can support an `alt`
label such as "The request is ready" with a child `return` label "Return the
prepared result". Both events cite their respective FLOWs. The proposal's nested
`children` and `otherwise` become branch events and closing `end` markers during
materialization. Direct narratives must supply balanced group/end events.

Place supported branch actions inside the corresponding group. An empty group
does not explain supported behavior. When evidence only establishes a condition
and leaves its effect unresolved, state that precise limit without inventing an
action. Preserve exclusive and first-match alternatives. A coverage list is not
execution order: derive grouping and chronology from delivered source evidence,
not lexicographic dependency IDs or their list positions.

Keep the operation summary focused on the trigger and result. Add explanation
only when it supplies useful context about decisions, outcomes, failure paths or
limits. Explicit explanation paragraphs retain the evidence of their linked
diagram events. Do not manufacture business meaning from method names or copy
source snippets into prose such as "The source contains ...; this records lexical
structure".

Generic proposals currently materialize each step meaning into both an event
label and a linked explanation paragraph. Explicit operation explanations are
additional paragraphs, so they should add context rather than repeat the step.
The Markdown renderer deduplicates identical explanation text within each of its
main-prose and detail sections, accounting for authored edit identity. It does
not deduplicate explanation against diagram labels or across those sections.
Author guidance does not change this materialization or hide accepted events.

## Correct missing-flow errors

A rejected sequence reports its mandatory FLOW kind, opaque flow/source IDs,
allowed event kinds and the corrective action. Path-shaped or otherwise unsafe
IDs are redacted; source text and private absolute paths are not diagnostics.
Use the ID to find the delivered evidence, add a matching cited event, and close
any `alt`, `opt` or `loop` group with `end`. A RETURN or THROW may use a `note`
instead of an arrow when endpoints do not help explain the outcome. An arrow of
kind `return` still needs valid participants.

Readability guidance is an authoring and review contract, not a semantic prose
validator. Structural acceptance alone does not prove an explanation is useful
or establish runtime effects. It does not authorize dropping mandatory branches,
inventing domain language, or rewriting the process model.
