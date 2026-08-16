# Multi-language Codeclew M1 implementation brief

Date: 2026-08-13

Status: preregistered before production changes

## Research interpretation

| Field | Decision |
| --- | --- |
| Research verdict | `PIVOT` |
| Selected product track | `B` — Repository Understanding + Change Impact |
| Research input | `/workspace/user/Downloads/deep-research-report (1).md` |
| Research SHA-256 | `6b9d9c73a809e896506dfd2645d09b77e8251940138eb813c85aeb573a270791` |
| Execution contract | `/workspace/user/.codex/attachments/2febcdc6-a823-4cad-b1af-7fabcaa55f81/pasted-text.txt` |
| Execution-contract SHA-256 | `a115a0690a7fe9ffc79d6cfbe2f31f2a58bc3412f9af44d22dd6e336765c35ee` |
| Architecture choice | Federated, language-owned adapters over a small versioned evidence protocol and the existing content-addressed Codeclew store/projection infrastructure |

## First falsifiable milestone

Build one runnable, zero-model, bounded Repository Understanding + Change
Impact contour over Kotlin, Rust, and TypeScript strict:

```text
exact workspace snapshot
-> adapter capability tuple
-> entities + resolved/navigation occurrences + typed relation assertions
-> bounded context projection
-> conservative may-impact paths + mandatory validation obligations
-> explicit completeness/UNKNOWN boundaries
-> complete cold/warm cost record
```

The Kotlin implementation is built first. Its shared protocol, decision rules,
UNKNOWN semantics, relation specifications, and conformance tests are then
content-hashed into a core contract lock. Rust must be added without changing
that frozen core; the lock is repeated before TypeScript strict, which has the
same constraint.

This milestone may establish protocol portability and a runnable product
surface. It cannot establish model-token or agent-correctness benefit. Those
claims require the later prospective four-arm experiment.

## Explicit non-goals

- no automatic edit generation or application;
- no authority token for source mutation;
- no universal AST, type system, ownership, nullability, effects, bytecode,
  LLVM IR, WASM IR, JBMC, or ByteBack;
- no behavioral-equivalence claim;
- no model canary or full model benchmark;
- no benchmark-family dispatch or repository-specific recipe;
- no relabelling of syntax/LSP/SCIP evidence as a proof;
- no claim that a missing edge proves absence unless the exact relation scope
  is enumerated completely by the adapter contract.

## Required capability surface

Each capability is bound to the exact tuple
`(language, adapter version, toolchain identity, build configuration,
operation URI/version, evidence grade)`.

Required per language:

1. content-addressed workspace/build snapshot;
2. source artifacts with USER/GENERATED/VENDORED/EXTERNAL origin;
3. adapter-owned entity identities;
4. definition/reference/call/read/write occurrences, with unresolved states
   represented explicitly;
5. compiler/type-check acceptance receipt;
6. may-call/reference impact edges with query-relative coverage;
7. bounded context and impact projection;
8. mandatory validation obligations derived by a provider-owned relation
   specification;
9. generated/dynamic/macro/source-map boundaries;
10. cold and warm end-to-end cost telemetry.

The milestone does not require identical SSA, ownership, nullability, effect,
or formal-proof depth across languages.

## Proof and evidence policy

The protocol treats grades as different evidence kinds, not as an ordinal
score. It distinguishes at least:

- `NAVIGATION_EVIDENCE`;
- `COMPILER_RESOLVED`;
- `COMPILER_CHECKED`;
- `SOUND_STATIC_WITHIN_SCOPE`;
- `BOUNDED_FORMAL_PROOF`;
- `CONTRACT_CHECKED`;
- `TESTED`;
- `RUNTIME_OBSERVED`.

`UNKNOWN` is a result/boundary, not a weak proof grade. Multiple weak facts do
not compose into a stronger grade without a registered composition rule.
Compiler acceptance proves only the exact compiler/type-check claim. SCIP,
LSP, Tree-sitter, textual search, and runtime observations cannot close a
sound static obligation by themselves.

## M1 GO criteria

These criteria decide only whether the implementation is ready for the later
prospective product experiment:

1. all three adapters execute on real source projects and emit no prepared
   fact fixtures;
2. schemas and outputs are versioned, canonical, content-addressed, and bound
   to exact snapshots/toolchains/build configurations;
3. stale snapshot, toolchain/configuration drift, forged/truncated evidence,
   missing mandatory obligations, unsupported relation, and false-completeness
   cases fail closed;
4. every `COMPLETE_IN_SCOPE` claim is backed by the adapter's declared
   enumeration guarantee; otherwise results are partial or unknown;
5. decision-core semantic changes after the Kotlin freeze are zero for Rust,
   and zero after the Rust freeze for TypeScript;
6. language-conditioned branches in the shared decision core are zero;
7. false `PROVEN` and false completeness in the conformance/adversarial suite
   are zero;
8. cold and warm telemetry includes workspace/build discovery, adapter/index
   execution, store/projection work, and emitted bytes;
9. the official pre-change baseline and all relevant post-change tests are
   reported without hiding failures;
10. a machine-readable decision receipt and exact reproduction commands are
    retained.

## Product GO criteria reserved for the later experiment

The research report's Repository Understanding product gate is not evaluated
by M1. It remains frozen as:

- at least 20% noncached-token reduction versus default on each language and
  the worst-language estimate;
- at least 10% warm end-to-end wall-time reduction;
- hidden-correctness delta lower 95% confidence bound at least -2 percentage
  points;
- median cold break-even by at most three agent tasks;
- Codeclew plus AST must improve tokens or wall time by at least 10% over AST
  alone without correctness degradation.

Until a prospective four-arm experiment evaluates these thresholds, the
experiment status must be `NOT_STARTED_WITH_REASON`, and no model-benefit claim
is permitted.

## STOP/PIVOT criteria

- `STOP / UNIVERSAL_CORE_FALSIFIED_BY_RUST` if Rust requires changing shared
  relation meaning, UNKNOWN/completeness semantics, proof rules, or adding a
  language branch to the core.
- `STOP / UNIVERSAL_CORE_FALSIFIED_BY_TYPESCRIPT` under the same condition for
  TypeScript strict.
- `STOP / FALSE_PROVEN` on any reproducible false proof.
- `STOP / FALSE_COMPLETENESS` on any complete result with an undisclosed
  mandatory boundary.
- `PIVOT / FEDERATION_ONLY` if the real adapters provide useful navigation but
  cannot support the shared impact contract without semantic weakening.
- Later, `PIVOT / THIN_ORCHESTRATION` if the prospective A3 arm adds no
  material value over A1 on at least two languages.

## Expected portability sequence

```text
Kotlin implementation
-> core-contract K lock
-> Rust adapter-only implementation
-> verify zero shared semantic-core drift
-> core-contract R lock
-> TypeScript strict adapter-only implementation
-> verify zero shared semantic-core drift
```

Registration metadata and language-owned descriptors may be added after a
freeze. Shared decision semantics may not change.

## Expected reuse/refactor/removal map

| Existing area | M1 treatment |
| --- | --- |
| `canonical.rs`, content hashes/blobs | reuse |
| `index.rs` SQLite/WAL staging and invalidation | reuse/refactor behind generic evidence records |
| `projection.rs` budgets, evidence links, expansion handles | reuse through a repository-evidence adapter |
| `semantic_kernel.rs` snapshot validity, provenance, soundness, UNKNOWN | reuse/refactor into the protocol nucleus |
| Git/worktree/read-set/CAS transaction machinery | retain, but do not invoke for M1 edits |
| Kotlin worker/K2 facts | retain as the Kotlin adapter source |
| `worker.rs` Kotlin variant dispatch | isolate behind a generic adapter boundary |
| `MAP_EDGE_WITH_CONTEXT`, `kotlin_replacement` | retain only as legacy/Kotlin edit code; exclude from M1 shared protocol and decision path |
| E04 readiness/corpus code | retain as experiment infrastructure, not product semantics |
| benchmark-family/task recipes | prohibit in the M1 product path |

## Initial evidence matrix

| Research claim | Research section | Repository evidence | Status before M1 |
| --- | --- | --- | --- |
| Pivot to Repository Understanding and Change Impact | “Итоговый вердикт”, “Продуктовая гипотеза” | active legacy path `task_context.rs` plus `projection.rs`; `agent_context.rs` is a reachability/removal candidate; the README product identity remains a Kotlin semantic change engine | `PARTIAL` |
| Existing store/projection/transaction foundation is reusable | “Карта reuse / refactor / removal” | `index.rs`, `projection.rs`, `transaction.rs`, `canonical.rs` | `VERIFIED` |
| Shared worker protocol leaks Kotlin | “Итоговый вердикт”, “Semantic edits” | `worker.proto` has `k2_validated`; `edit_ir.proto` has `kotlin_replacement` and `MAP_EDGE_WITH_CONTEXT` | `VERIFIED` |
| Capability is not bound to the required tuple | “Capability negotiation” | `WorkerCapabilities` exposes language/worker/compiler and string lists, but no toolchain/configuration/operation/grade key | `VERIFIED` |
| Current snapshot is too narrow | “Snapshot” | `worker.proto::SnapshotId` contains only base revision and project-model hash; `model.rs::Snapshot` has Kotlin defaults | `VERIFIED` |
| Facts lack proof grade and completeness | “Facts и relations”, “Completeness” | `semantic_facts.proto::SemanticFact` stores string kind/types/effects without provider, coverage, boundary, or grade | `VERIFIED` |
| Kotlin path is real and compiler-backed | “Языковые особенности MVP” | version-pinned Kotlin workers and K2/FIR extraction under `workers/kotlin*`; trusted distributions under `workers/manifests` | `VERIFIED` |
| A generic multi-language adapter path exists | “Portability MVP” | `worker.rs::WorkerVariant` contains only Kotlin 2.1/2.3/2.4 | `ABSENT` |
| Generic edit protocol is language-owned | “Semantic edits” | shared `Replacement.kotlin` and the MAP operation contradict the desired boundary | `CONTRADICTED` |
| E04 established product benefit | “Итоговый вердикт” | terminal report records upper bound 2/14 and `modelCalls=0` | `CONTRADICTED` |

The matrix will be extended with exact post-M1 files, tests, lock digests, cost
receipts, and deviations. Research claims that do not match executable code
remain marked partial, absent, or contradicted rather than being silently
reinterpreted.

## Pre-change baseline

Baseline revision: `be19fd06ce7509ffbaf4622b8f8d13ec7cbcd854`.

| Command | Result before M1 |
| --- | --- |
| `cargo fmt --all --check` | `PASS` |
| `cargo test -p clew --lib semantic_kernel::tests -- --test-threads=1` | `PASS`, 17 tests |
| `cargo test -p semantic-corpus product_coverage -- --test-threads=1` | `PASS`, 5 focused tests; the frozen ceiling remains 2/14 |
| `./scripts/verify.sh` | `FAIL` at the pre-existing workspace Clippy gate after the Gradle worker/fixture and formatting gates passed |
| `cargo clippy -p clew --lib -- -D warnings` | `FAIL`, 12 pre-existing diagnostics in `evidence_authority.rs` |
| `cargo clippy -p semantic-corpus --lib -- -D warnings` | `FAIL`, 4 pre-existing diagnostics in `lib.rs`, `e04.rs`, and `e04_authorization.rs` |

The official verifier failure is part of the baseline, not attributed to M1
and not hidden behind a narrower green test. Full workspace test completion was
not established before M1; its baseline status is therefore `UNKNOWN` rather
than implicitly `PASS`.
