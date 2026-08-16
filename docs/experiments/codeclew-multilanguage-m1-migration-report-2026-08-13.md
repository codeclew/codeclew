# Codeclew multi-language M1 migration and isolation report

Date: 2026-08-13

Decision: `PIVOT / REAL_REPOSITORY_CONFORMANCE_GAP`

## Retain

The following new components are useful independently of the failed adapter
gate and should remain as an experimental read-only branch:

- `schemas/evidence_core.proto`: versioned language-neutral evidence wire
  contract;
- `crates/evidence-core`: exact snapshot/capability/fact/obligation/receipt
  validation, non-ordinal evidence policy, and content-addressed freeze;
- `schemas/adapter_output.schema.json`: closed adapter envelope;
- `crates/evidence-adapters/src/lib.rs` and `core_bridge.rs`: generic envelope
  and typed-core bridge;
- `crates/evidence-adapters/src/bin/evidence.rs`: bounded read-only runtime,
  CAS store, projection, telemetry, and machine-readable refusal boundary;
- `scripts/multilang_portability_stage.py`: adapter-only stage capture against
  an unchanged core lock.

These components expose no source-edit/apply authority. Their retained value
is evidence coordination and fail-closed validation, not proof of product
benefit or portability.

## Keep isolated as legacy

- `semantic_goal.rs`, `evidence_authority.rs`, `schemas/edit_ir.proto`, and
  MAP/PTC-specific transaction validation remain the legacy Kotlin editing
  vertical. They must not be imported into `evidence-core`.
- `task_context.rs` remains explicitly legacy heuristic/debug context.
- E04 readiness, corpus, controllers, and signed authorities remain experiment
  infrastructure rather than product semantics.
- The current Kotlin descriptor graph and K2 worker are adapter-owned input
  providers. Their enums and validation must not be promoted into shared
  multi-language semantics.

## Experimental and unqualified

- `crates/evidence-adapters/src/bin/kotlin.rs` passes its fixture contour but
  fails the real-repository gate.
- `crates/rust-evidence-adapter` contains partially verified source but no
  completed shared-runtime or real-repository R0 receipt.
- `adapters/typescript` is source-ready but its final envelope version is
  untested.

None of these adapters should be advertised as generally supported. Their
presence in the tree is not a capability claim.

## No deletion in this series

No legacy product code was deleted. The first milestone was intended to prove
a replacement path before removal. Because the real Kotlin gate failed,
deleting the existing Kotlin editing or context paths would be unjustified.

## Required M1.1 pivot

Start a new preregistered series rather than patching K0.1 in place:

1. Make the Kotlin provider/ingestion boundary total over compiler enum drift.
   The reproduced `effectiveVisibility = local` case must become a
   Kotlin-owned fact or explicit `LOCAL_ONLY`/`UNKNOWN` boundary, never an
   untyped process abort and never a guessed common visibility meaning.
2. Add the exact six-descriptor shape as an adversarial regression and retain
   the original raw-provider digest.
3. Run the direct adapter and the generic runtime on at least two unprepared
   real Kotlin repositories (Gradle and Maven). A failure must produce a
   canonical typed refusal with complete cost evidence.
4. Measure dependency-cold and exact-snapshot warm behavior. The existing
   second invocation repeats cold indexing and is not a warm-cache success.
5. Freeze a new K1 contract only after those gates pass; then restart Rust R1
   and TypeScript T1 in order without editing the frozen shared core.

This repair is language-owned. If it requires weakening shared UNKNOWN,
coverage, evidence grades, or adding a Kotlin branch to the shared decision
core, the correct outcome becomes `STOP / UNIVERSAL_CORE_FALSIFIED`, not a
new mapping exception.
