# Independent implementation-plan critique

Date: 2026-09-12. Scope: the [implementation plan](../../../plans/documentation-system-implementation-plan.md)
and its approved product sources. A separate read-only critic performed one full
review and one targeted recheck. Paths below refer to the plan unless stated
otherwise. Initial line numbers identify the reviewed draft, before repairs.
This report reviews a plan; it does not establish implemented or production behavior.

## Initial report

### Verdict

Clarification is needed: coverage is strong, but four execution gaps prevent the plan from being fully self-contained and consistent.

### Major findings

1. **T04 — Adapter isolation is stated without an enforceable execution contract.**
   `documentation-system-implementation-plan.md:315` allows an external command interface and requires protected capabilities, but does not specify how the first adapter prevents direct filesystem writes or unregistered reads; a command running with coordinator permissions could bypass proposal-field validation. The sandbox recipe arrives only in T14.
   **Minimal fix:** Specify T04's minimum adapter isolation contract: immutable read-only work inputs, registered expansion access, role-specific result destinations, no coordinator/human/source write access, and a generation gap when the adapter cannot enforce these boundaries; test actual denied access alongside forged outputs.

2. **T04/T16 — Budget accounting lacks a dispatch reservation and unknown-usage rule.**
   `documentation-system-implementation-plan.md:317` requires finite budgets and records unavailable usage, while line 744 requires full-contour limits; neither defines how another call is admitted after missing usage or how review and a repair remain affordable after authoring. Checking previously reported consumption before dispatch does not bound the next call.
   **Minimal fix:** Define execution-config limits and reservations covering orchestration, author, reviewer and one repair, including per-call caps, a stop-loss below the ceiling, and conservative treatment of unreported usage; require T16 to reuse that admission rule and include evaluator costs.

3. **T17 — Model qualification blocks unrelated migration and guidance.**
   `documentation-system-implementation-plan.md:764` makes all T17 work depend on T16, although T16 explicitly remains incomplete when deployment credentials or model results are unavailable. This prevents compatible examples, skill corrections and the operator walkthrough despite the locality rule at lines 47–51 and T17's own permission to report unqualified capabilities.
   **Minimal fix:** Make implemented runtime/integration readiness the prerequisite for T17's documentation and walkthrough; require completed T16 evidence only for the model-readiness portion of its verdict, otherwise record that qualification gap.

4. **T06 — Permitted worker/protocol changes lack corresponding task-local verification.**
   `documentation-system-implementation-plan.md:385` permits normalized-schema and worker-bridge changes, but its Verify block at lines 395–402 runs only Rust tests. A worker change could therefore satisfy T06's written completion check without testing the changed producer, contrary to repository verification requirements.
   **Minimal fix:** Add a conditional verification branch using the existing `SpringAnnotationFactsTest` worker commands from [ci-verify.sh](../../../../scripts/ci-verify.sh), line 31, plus the relevant actual javac/K2 compatibility checks when their bridges or shared contracts change; record “not applicable” when no producer contract changes.

### Open Questions

None requiring a new product-scope decision. These are clarifications within the approved target.

## Targeted recheck report

### Verdict

All four findings are resolved; the targeted changes introduce no blocking contradiction, and the plan is sufficient to close this review phase.

- **T04 isolation — Resolved.** Plan line 318 now requires enforceable isolation at first use, defines protected resources and registered reads, and produces a local gap when isolation is unavailable. Line 321 requires actual access-denial tests.
- **T04/T16 budgets — Resolved.** Plan line 319 specifies atomic reservations, mandatory review/repair allowance, per-call caps and a stop-loss. Unknown usage retains its maximum reservation. Line 759 reuses these rules and includes evaluator and orchestration costs.
- **T17 dependency locality — Resolved.** Plan line 779 depends on T15 and restricts T16's gate to positive model-readiness claims. Guidance, migration and walkthrough work can finish with qualification explicitly pending.
- **T06 producer verification — Resolved.** Plan line 406 conditionally requires real worker and enrichment checks, covering all three Kotlin workers and both enrichment paths for shared-contract changes.

The accompanying verification changes are consistent: T15 explicitly creates its named acceptance cases, and T17 creates migration/init cases and invokes an existing embedded-skill digest test.

### Open Questions

None.
