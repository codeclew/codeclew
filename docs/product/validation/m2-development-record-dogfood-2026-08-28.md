# M2 Development Record dogfood — 2026-08-28

## Scope

This is a small internal product check, not a publication-grade benchmark. It
used a clean, temporary Kotlin 2.4.10 Gradle repository and the public `./clew`
launcher. Raw sessions, repository identity, source, and absolute paths remained
in private managed state.

The check covered two bounded cases:

1. a context-only record with one `UNSURE` claim and one compiler fact pointer;
2. one real change with a production KDoc operation, a canonical README
   operation, successful Gradle validation, conditional publication, and a
   mission binding to the final run.

## Accepted facts

- The context-only record had one claim, zero coverage obligations, three graph
  nodes, and two edges. Its readiness stayed `CONDITIONAL`, matching the source
  context's unresolved compiler boundaries.
- Two independent full dossier renders were byte-identical.
- Selecting the claim returned exactly one node, zero graph edges, and that
  claim's `CONTEXT_EVIDENCE` pointer. It did not return a graph-wide evidence
  block.
- The real change reached `PUBLISHED_CONDITIONAL`. Its candidate contained two
  planned files and successful native Gradle validation.
- The resulting record linked requirement, claim, context evidence, both
  operations, run validation, acceptance criterion, and canonical
  documentation. Codeclew reported zero unresolved coverage obligations.
- Documentation has its own graph node. Selecting it returned one node, zero
  edges, and only its `PLAN_OPERATION` evidence.
- A deliberately malformed first plan failed compilation and remained a failed
  run. The corrected immutable plan produced a separate successful run; no
  failed evidence was rewritten.
- A mission created by an earlier runtime capsule was later closed and its
  session garbage-collected through the current launcher. Runtime advancement no
  longer strands immutable mission evidence.
- After mission closure and session garbage collection, the retained claim still
  resolved as current and the dossier remained reviewable.

## Warm-path finding and correction

The first dossier implementation reopened and fully validated the context
evidence object on every render:

| Operation | Before | After |
| --- | ---: | ---: |
| Mission status baseline | 0.11 s | 0.11 s |
| Context-only dossier | 3.46 s | 0.07 s |
| Selected claim node | 3.46 s class | 0.07 s |
| Full real-change dossier | not measured | 0.14 s |
| Full-record admission | 17.01 s | 3.67 s |

The correction admits each member context, plan, and run once when the immutable
record is created. Later renders verify the small retained projections and their
content digests. The context-only dossier became roughly 49 times faster;
full-record admission became roughly 4.6 times faster.

## Honest limits

- Both Kotlin contexts were `UNSURE` because the selected code exposed explicit
  unresolved semantic boundaries. The pilot therefore proves honest
  conditional documentation, not an `EXACT` behavior claim.
- This check does not establish the roadmap's held-out quality percentages. It
  establishes the product mechanics and removes the measured warm-path blocker.
- Multi-repository coordination and cross-language envelopes are subsequent
  roadmap slices; they were not pulled into M2.
