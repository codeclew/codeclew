# R01 coverage boundary

Status: **FROZEN CONTRACT; NOT PRODUCT EVIDENCE**. This document bounds claims and branch inputs. It does not assert that H01-H14 pass, that a benchmark has run, or that any cross-language equivalence exists.

## Source and authority boundary

The frozen research set is S0-S5 as named by the cumulative plan: `project.md`, semantic-editing research, the historical corpus-first plan, its coverage verification, the S4 coordination prompt, and the S5 coordination result. Git source, build artifacts, configured tests, and the target ref remain authoritative for executable behavior. Derived semantic state is disposable, rebuildable, lossy, snapshot-bound, and non-authoritative. Literature, inference, design decisions, static evidence, runtime evidence, and human declarations remain distinct evidence classes.

An opaque or unavailable citation is `UNVERIFIED`. It may motivate a gap but may not support a gate, product claim, threshold, or PASS. A source-grounded answer is not a measured fact. Any S0-S5 digest change invalidates this freeze and requires a new DAG/coverage version.

## Supported Kotlin/JVM contour

The claimable contour is deliberately narrower than “Kotlin” or “multilanguage”:

- Kotlin/JVM source (`.kt`) through exact version-pinned workers `2.1.21`, `2.3.0`, and `2.4.10`; no nearest-version substitution. Kotlin `2.1.21` is a mandatory compatibility stratum for both Gradle and Maven evaluation.
- JDK 21 and version-neutral Rust/Kotlin DTO/Protobuf boundaries; Kotlin compiler, PSI, K2, and FIR objects do not cross the worker protocol.
- Gradle project inspection for an explicitly selected compilation, plus the implemented single-module Maven Kotlin/JVM vertical. Each build-system/version combination is claimable only after R03 verifies it from a clean checkout; the freeze does not presume every matrix cell already passes.
- PSI/K2 symbol, type, call, receiver and diagnostic facts; local CFG plus bounded SSA/def-use/control dependency and Thread IR. Calls and unsupported framework behavior remain explicit summary/Unknown boundaries, not a global interprocedural proof.
- Snapshot/fingerprint-bound index and anchors, detached-worktree preview/build/test, ReadSet/WriteSet, MVCC/CAS, ledger, rollback and recovery only where the existing capability audit verifies them.
- Evaluation populations must keep Gradle and Maven strata, supported exact Kotlin versions, positive/ambiguous/must-refuse cases, repository/cache strata, and an independently constructed ecological sample. A pilot or one repository never establishes ecosystem-wide generality.

Fail-closed exclusions include Android project models, Kotlin Multiplatform, `.kts` scripts as analyzed source, `expect/actual`, Compose-specific semantics, arbitrary compiler plugins, reflection resolution, exact coroutine state-machine modeling, Java or other language source adapters, global points-to/interprocedural PDG, multi-module Maven, unsupported project models, and framework/query semantics without a declared adapter or external oracle. JPQL, Spring/JPA lifecycle, persistence context, transaction propagation, serialization, generated code, reflection, and runtime configuration are separate evidence boundaries.

## Forbidden claims

The following wording is forbidden until the named evidence exists, and several claims are forbidden outright inside the current program:

1. “Complete program model”, “digital twin”, or a second executable implementation. H01 requires a useful lossy projection and anti-duplication evidence.
2. “Proven safe” from no detected conflict, compilation, a test/demo, LLM judgment, a clean textual merge, or a static graph. Allowed wording is `NO_CONFLICT_FOUND_WITHIN(boundary, evidence)`.
3. “Grep-free” if model-directed `rg`, `grep`, `find`, broad reads, query widening, task-derived lexical queries outside the one-shot bootstrap, hidden search, over-budget navigation, or fallback occurred. Such a run is `FALLBACK_SEARCH`.
4. Universal editing, universal Kotlin/framework support, repository-scale generality, or cost advantage before the relevant frozen corpus and GE1/GES/GE2/X02-X04 gates.
5. Failure probability from an uncalibrated criticality/risk score; H12 decision ownership belongs only to independent X04.
6. A graph/OWL/hypergraph store is necessary before R03/H13 measures its threshold and operational cost.
7. Static, runtime, build/test, model, human, and literature evidence are interchangeable.
8. A source-grounded scaffold proves H14, product value, migration correctness, arbitrary behavioral equivalence, or universal transpilation. R01/GF must label it `UNTESTED_SCAFFOLD`; only separately approved post-GO Z01 may test one bounded domain.
9. Primary GF success implies approval for Z01. A01 is a separate human decision, and Z01 cannot retroactively improve the primary verdict.

## Frozen navigation and evidence limits

The Codeclew arm may use one `BOOTSTRAP_TASK`, at most two typed handle expansions, exact anchor reads, typed goal submission, and preview/validate/commit. Total model-visible context is at most 32 KiB, exact anchored source at most 12 KiB, and semantic records at most 512; typical goal target is at most 1 KiB. Any widening is retained as negative evidence. Missing correctness, event-clock, human, time, or native-token data makes the corresponding claim `INCONCLUSIVE` or `UNAVAILABLE`, never PASS.

## R01 branch-code input freeze

R01 may emit only the manifest outcomes `SUCCESS`, `FAILURE`, `REFUSED`, `BLOCKED`, `NO_PROGRESS`, or `INFRA_ERROR`, paired with one of these branch codes:

| Branch code | Exact admissible input |
| --- | --- |
| `NONE` | All S0-S5 digests match; mandatory source/claim/hypothesis/gap/question/deliverable/node links are bidirectionally complete; no UNVERIFIED source supports a gate; the bounded scaffold passes its boundary check; all other required R01 checks pass. |
| `REVISE_PLAN_SOURCE_COVERAGE` | A digest, mandatory mapping, exact H01-H14 contract, supported boundary, question/deliverable coverage, or required source-level artifact is missing, contradictory, duplicated, stale, or unverifiable. This is the only retryable generic R01 code. |
| `BLOCK_UNVERIFIED_PROVENANCE` | An unresolved or UNVERIFIED citation is used as parent evidence for a gate, threshold, PASS, or product claim. |
| `BUDGET_EXCEEDED` | The frozen R01 team budget is exceeded; this never relaxes coverage or converts missing evidence into success. |

`NONE` is not admissible merely because these four files parse. R01's full manifest also requires the other listed artifacts and independent checks. No product implementation, corpus generation, benchmark execution, threshold change, or cross-language validation is authorized by this boundary.

## Honest labels at handoff

- Hypotheses: `FROZEN_UNTESTED` until their designated nodes and gates produce accepted evidence.
- Research gaps: open until their destination receipts satisfy their closure rules.
- Cross-language: always `UNTESTED_SCAFFOLD` at R01 and GF.
- Unsupported or unexecuted work: `UNKNOWN_NOT_RUN`, `UNAVAILABLE_DUE_TO_TERMINAL_EVIDENCE`, or a narrower explicit refusal; never silently omitted.
