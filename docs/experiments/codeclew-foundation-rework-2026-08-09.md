# Codeclew: bounded foundation rework for the product-first execution epoch

Date: 2026-08-09

Plan: `docs/superpowers/plans/2026-08-08-codeclew-cumulative-evidence-graph-plan.md`

Plan SHA-256: `83933d98913af3c4b016f674f73b76af3cfe4db190e30294ebb469d6d6cd6f93`

Outcome: `SUCCESS + NARROW_SUPPORTED_CONTOUR`

Independent verdict: `ACCEPT` after one delta-only DAG-edge correction

## Why this rework exists

The first strict `GK` recomputation correctly returned
`INCONCLUSIVE_FOUNDATION` when it replayed the old execution ledger. That
ledger terminated after the historical `R01` attempt exceeded its call budget;
it therefore cannot authorize the later `K01`–`K04` commits.

The current Codex goal is a distinct, explicitly approved product-first
execution epoch. It starts from accepted commit `5b3ea86`, requires an
independent verifier for every meaningful node, and forbids rebuilding the
old controller/governance layer. The user's current-session approval and
resume events are the approval evidence; RSA/host attestation is explicitly
out of scope. This record does not rewrite the historical ledger. It closes
the single `REWORK_FOUNDATION` attempt by normalizing the evidence that is
actually available in the current epoch.

No product code, benchmark outcome, threshold, corpus instance, or gate
predicate is changed by this record.

## Frozen inputs

| Input | Identity | Status in this epoch |
| --- | --- | --- |
| Cumulative plan | `83933d98913af3c4b016f674f73b76af3cfe4db190e30294ebb469d6d6cd6f93` | Approved; unchanged |
| DAG | `5990bb9c17421aac0821acc5b7ff6a464d498b338d151a911787ebf2eb4ffb18` | Unchanged |
| R01 source manifest | `ca12d0683413130821dd2daa922c297a73b5964c152d437b9312992f7db25e35` | Carried forward as source content, not as the rejected old execution outcome |
| Measurement schema | `238457bc4a6cee9fcd2e620db17deed75b340b6641a68fb5aa8be21669d086a6` | Native token fields and `UNAVAILABLE` branch frozen |
| Storage ADR | `ffda80a4e87bf1fac94adbd846d5eb5891543dfe918da0e7ba6d8ffd55829070` | Reuse SQLite; no parallel semantic store |
| K01 | `5b3ea86626950183e4bdca743178311fe436dd15` | Current goal's accepted starting point |
| K02 | `dc65bb9bab298bdfde058c40b9aaf76e6b897373` | Conservative identity contour |
| K03 | `99141ea5deffb47d20e16ec581935177ae90566b` | `NARROW_TO_REBUILD_MODE` |
| K04 | `7b61c1a12e992a6183c255166af36f5cabcb85c8` | `NARROW_PROJECTIONS`; fresh verifier `ACCEPT` |

## R01–R03 evidence normalization

### R01 — source and hypothesis freeze

The accepted research content is present under `docs/research/codeclew/`:
S0–S5 source identities, H01–H14, GP-001–GP-016, source-to-claim links,
coverage boundaries, bibliography lock, the cross-language scaffold and the
synthetic evidence view. Unverified literature is explicitly ineligible as
gate evidence. The earlier controller budget failure remains historical
evidence about process overhead; it does not invalidate the immutable source
content.

Outcome in this epoch: `SUCCESS`, with no claim that the historical R01
controller succeeded.

### R02 — measurement contract

The cumulative plan and EvidencePacket v0 freeze correctness-first acceptance,
cold/warm/amortized clocks, exclusions, retries, topology parity, provider
token fields, raw/cached/noncached separation, missing-data handling,
populations, thresholds and final-system lock. Current Codex execution exposes
only aggregate goal usage, not provider-native per-arm token decomposition.
Bytes are not substituted for tokens.

Outcome in this epoch: `SUCCESS + TOKEN_TELEMETRY_UNAVAILABLE` for semantic
design. This keeps product work reachable, but forbids every token-win verdict
until a later benchmark runner supplies provider-native telemetry. If it never
does, the final token claim must remain `INCONCLUSIVE`, not positive.

### R03 — reuse and storage boundary

The product continues to use the existing Rust core, compilation-scoped
SQLite/WAL index, version-pinned Kotlin workers, Thread IR, semantic
transaction/CAS/recovery and repository-owned skill. K01–K04 extend those
types and the same `RepositoryIndex`; they do not add a graph database,
parallel source of truth, Ruby service, OWL layer or second identity model.
ADR-009 selects SQLite and rebuildable derived state. Existing measurements
bound the current contour: worker startup `349 ms`, cold semantic fixture
index `1571 ms`, warm semantic file reindex p95 `260 ms`, and cold 100,002
Kotlin-LOC syntax indexing `8140 ms` against a `20000 ms` SLO.

Outcome in this epoch: `SUCCESS + NARROW_BASELINE_CONTOUR`. The measurements
do not prove repository-scale semantic update latency or universal advantage.

## K01–K04 executable evidence

| Node | Executable fact added | Accepted limitation |
| --- | --- | --- |
| K01 | Lossy semantic records, composite snapshot, provenance, Unknown, dependency invalidation, commit preconditions and source-removal/anti-duplication tests | Kernel/conformance subset only; no universal task completeness |
| K02 | `SAME/RENAMED/MOVED/SPLIT/MERGED/DELETED/AMBIGUOUS` lifecycle with source-provenanced Kotlin 2.1 identity and fail-closed decoys | Conservative supported identity subset |
| K03 | Durable event/checkpoint/replay freshness; gaps and stale dependencies become `UNKNOWN`/stale and require an authoritative rebuild | Rebuild-only publication is trusted; incremental latency is not claimed |
| K04 | Exact L5→L0 ladder, nine evidence-bound thread kinds, query-bound replay, byte budget, explicit boundaries and fail-closed source provenance | Anchored Thread IR only; no H03/H04 applicability or speed claim |

The post-K04 workspace suite passed in the current epoch: 97 library tests and
the concurrency, Kotlin, Maven, metamorphic, projection CLI, semantic daemon
and vertical integration suites. `cargo fmt --all --check` and
`git diff --check` also passed. K04 required several independent rejections;
the final fresh verifier returned `ACCEPT` only after the public provenance,
taxonomy, kind relevance, skipped-level and rendered-byte counterexamples
were closed.

## Recomputed GK predicate

| Predicate | Result | Boundary |
| --- | --- | --- |
| R01–R03 and K01–K04 have current-epoch evidence | Narrow pass | This record deliberately does not convert the rejected historical ledger into success |
| Consistency model is executable with provenance, freshness and composite snapshot | Pass | K01 schema/kernel plus K03 rebuild-only freshness |
| No second implementation | Pass for inspected current contour | Same Rust model/index and Kotlin worker protocol; lossy source references only |
| Stable identity and fail-closed ambiguity | Narrow pass | K02 supported Kotlin contour only |
| Storage decision reuses the core or has measured cause | Narrow pass | SQLite reuse; only existing fixture/100k measurements |
| L0–L5 does not hide Unknown/coverage boundaries | Narrow pass | K04 anchored Thread IR only |

Candidate GK branch: `NARROW_SUPPORTED_CONTOUR`, not `PASS`.

## Authorized contour and exact non-claims

Authorized for the next product nodes:

- Gradle/Maven Kotlin/JVM projects that open with an exact supported worker,
  including the mandatory Kotlin 2.1.21 stratum;
- lossy semantic records backed by source/build/index provenance;
- conservative identity where ambiguity refuses rather than retargets;
- authoritative full rebuild before publishing fresh derived views;
- anchored Thread IR projections with explicit Unknown/boundary/refusal;
- typed-goal and coordination experiments only after their own D02/D03
  prerequisites are independently accepted.

Not authorized:

- incremental-freshness performance claims;
- unanchored or synthetic upper-level facts;
- universal applicability, `COMPLETE_TASK`, token or wall-time advantage;
- production materialization before binder and safety gates;
- treating old PIM/product-repo runs as withheld evidence;
- treating compact bytes as token telemetry.

## Edges after independent verification

The fresh verifier accepted this record and recomputed the code facts. The
resulting edge state is:

- open `D01` and `D03` from the accepted K01 contour;
- open `Q01` and `Q03` from accepted `K04 + NARROW_PROJECTIONS`;
- keep `D02` closed until independent acceptance of `D01`;
- keep `E01` closed until accepted `D02`; the GK half of its join is satisfied;
- keep `M01` closed until accepted `D03`; the GK half of its join is satisfied;
- keep all benchmark victory, universal-completeness and final-verdict edges
  closed;
- retain `GK -> GF0` as the historical old-epoch outcome, not the current
  product-first epoch outcome.

If the verifier finds a product counterexample or a missing predicate rather
than a ledger-format objection, the one rework attempt fails and the current
epoch returns `INCONCLUSIVE_FOUNDATION`.

## Gate-efficiency verdict

The strict first GK was useful: it found a real evidence-lineage break. The
repair is intentionally one Markdown record, one independent read-only
verification and no controller, schema runtime, repeated full suite or product
rewrite. Any request for more governance machinery is a stop-loss failure;
the plan then narrows or stops instead of consuming product budget.
