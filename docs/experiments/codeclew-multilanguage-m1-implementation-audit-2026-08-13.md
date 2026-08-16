# Codeclew multi-language M1 implementation audit

Date: 2026-08-13

Baseline revision: `be19fd06ce7509ffbaf4622b8f8d13ec7cbcd854`

Research input SHA-256:
`6b9d9c73a809e896506dfd2645d09b77e8251940138eb813c85aeb573a270791`.

Execution-contract SHA-256:
`a115a0690a7fe9ffc79d6cfbe2f31f2a58bc3412f9af44d22dd6e336765c35ee`.

## Audit verdict

The existing repository contains reusable integrity, indexing, projection, and
transaction mechanics, but it does not contain a language-neutral semantic
product. Its active product path is a Kotlin-specific editing engine and its
bounded context CLI is explicitly a legacy heuristic. M1 therefore adds a
read-only evidence and impact contour alongside the legacy implementation. It
does not rename the MAP/PTC vertical into a generic capability.

## External boundaries

- `clew` is the Rust CLI. It currently starts only trusted Kotlin 2.1, 2.3, or
  2.4 workers.
- Kotlin workers call the Gradle or Maven project model and K2/FIR providers.
- The repository index publishes SQLite/WAL state and content-addressed blobs
  under `.semantic-thread`.
- The legacy editing path owns detached worktrees, validation, read-set replay,
  and Git compare-and-swap.
- E04 readiness, corpus construction, and signed authorities are experiment
  infrastructure and are not a product semantic dependency.

## Reuse map

| Area | Decision | Evidence |
| --- | --- | --- |
| canonical JSON and SHA-256 | reuse | `crates/clew/src/canonical.rs` |
| staged repository index and blobs | reuse behind new envelopes | `crates/clew/src/index.rs` |
| bounded evidence projection | reuse/refactor | `crates/clew/src/projection.rs` |
| exact snapshot validity, provenance, UNKNOWN propagation | reuse mechanics, replace vocabulary | `crates/clew/src/semantic_kernel.rs` |
| worker framing, trusted distribution and session sealing | reuse in Kotlin adapter | `crates/clew/src/worker.rs` |
| detached worktree/CAS | retain for a later optional editing track | `crates/clew/src/transaction.rs` |

## Isolation and removal candidates

| Area | Treatment |
| --- | --- |
| `semantic_goal.rs` and `evidence_authority.rs` | legacy Kotlin/MAP editing strategy; excluded from M1 core |
| `schemas/edit_ir.proto` and `Replacement.kotlin` | legacy edit protocol; excluded from M1 |
| `task_context.rs` | legacy heuristic/debug context only; never evidence authority |
| `agent_context.rs` | reachability/removal candidate after compatibility audit |
| Kotlin-specific CFG/type/effect interpretation | adapter-owned |
| `crates/semantic-corpus/src/e04*`, `scripts/e04_*`, E04 contracts | experiment infrastructure only |

## Safety findings

1. MAP/PTC roles and recipes are still hard-coded in the legacy product path.
2. The typed legacy proof path seals mandatory closure and does not let the
   caller delete obligations, but there is no generic multi-language closure.
3. Kernel/projection UNKNOWN does not become proof; the separate legacy
   context can say `LEGACY_HEURISTIC_READY`, which is explicitly opt-in and is
   not proof authority.
4. Existing non-serializable receipts prevent cross-session apply replay, but
   there is no reusable evidence receipt bound to the complete
   adapter/toolchain/configuration tuple.
5. Clean HEAD and per-file hashes are checked in proof-bearing Kotlin paths;
   generated sources, features, targets, dependency identity, and dirty state
   are not unified into one workspace snapshot.
6. Shared worker/model/edit schemas leak Kotlin and Gradle assumptions.
7. Production evidence authority still contains an E04 issuer identifier.
8. Model-readable summaries cannot grant legacy apply authority; this is a
   reusable invariant.
9. Complete cold/warm analyzer cost is not a product receipt today.
10. Adding a non-Kotlin adapter currently requires changing common dispatch and
    semantic assumptions.

## Recorded baseline

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | PASS |
| `cargo test -p clew --lib semantic_kernel::tests -- --test-threads=1` | PASS, 17/17 |
| `cargo test -p semantic-corpus product_coverage -- --test-threads=1` | PASS, 5/5; current supported upper bound is 2/14 |
| `cargo clippy -p clew --lib -- -D warnings` | FAIL, 12 pre-existing diagnostics in `evidence_authority.rs` |
| `cargo clippy -p semantic-corpus --lib -- -D warnings` | FAIL, 4 pre-existing diagnostics |
| `./scripts/verify.sh` | FAIL at the same pre-existing Clippy gate after preceding Gradle and formatting gates passed |

The full workspace test baseline is `UNKNOWN`: it was not established before
production changes. Post-M1 reporting must distinguish this from new failures.

## First implementation boundary

The new shared layer may contain only snapshots, adapter-owned opaque entities,
occurrences, namespaced relations, capability tuples, evidence grades,
query-relative coverage/boundaries, provider-owned obligations, receipts,
bounded projections, typed refusals, and full cost telemetry. It must expose no
edit/apply authority. Kotlin, Rust, and TypeScript adapters own their compiler
semantics and publish only claims their exact providers support.
