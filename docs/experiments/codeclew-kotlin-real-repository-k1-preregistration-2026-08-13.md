# Codeclew Kotlin Real-Repository K1.4 preregistration

Date: 2026-08-13

Series: `KOTLIN_REAL_REPOSITORY_K1_4_2026_08_13`

Status: `PENDING_INDEPENDENT_PREREG_AUDIT`

This series supersedes the cancelled
`KOTLIN_REAL_REPOSITORY_K1_2026_08_13` preregistration through the canonical
[`preregistration-amendment-k1.1.json`](../../benchmarks/kotlin-real-repository/k1/preregistration-amendment-k1.1.json).

Before any qualification attempt, K1.1 was superseded by the independently
pinned [`preregistration-amendment-k1.2.json`](../../benchmarks/kotlin-real-repository/k1/preregistration-amendment-k1.2.json).
K1.2 preserves the exact 12 repositories, commits/trees, thresholds and
workload policy. It corrects only prequalification authority gaps: mutable
candidate distribution hashes move to candidateTools/freeze; dependency state
is prepared and verified per repository; Git-index source/link identities and
default-deny sandboxes are explicit; all K1-R01..R20 predicates and a pinned
read-only auditor are mandatory before GO. Recorded counters remain
qualificationAttempts=0, holdoutAttempts=0, modelCalls=0, holdoutOpened=false.

One infrastructure-only K1.2 preflight then exposed an unseeded isolated
`CARGO_HOME`; it ran no qualification or holdout attempt, did not materialize
the holdout, and made no model call. The pinned
[`preregistration-amendment-k1.3.json`](../../benchmarks/kotlin-real-repository/k1/preregistration-amendment-k1.3.json)
supersedes that diagnostic store. K1.3 preserves the corpus, thresholds,
workload, K0.1 bytes, and baseline command targets/test filters while adding a
Cargo.lock-derived credential-free offline Cargo seed, exact Rust launcher
identity, `--locked` test/clippy execution, baseline packet schema 0.2, and
live repository-head/source/tool bindings. The superseded store cannot be
reused; candidate tools and harness inputs must be rebuilt and rebound.

One baseline-only K1.3 run then exposed a second infrastructure authority
error: the shared baseline environment passed `JAVA_TOOL_OPTIONS` and
`GRADLE_USER_HOME` into Rust tests. The trusted worker correctly rejected that
JVM/Gradle injection state, so one required test was red before qualification.
The pinned
[`preregistration-amendment-k1.4.json`](../../benchmarks/kotlin-real-repository/k1/preregistration-amendment-k1.4.json)
supersedes that store. K1.4 keeps the exact graph, corpus, thresholds, workload,
baseline commands and tool/dependency identities. It changes only the series
bindings and splits baseline environments by tool family: Cargo/Rust receives
no JVM/Gradle injection variables; Gradle receives an isolated
`GRADLE_USER_HOME` and an explicit `-Duser.home` argument. The fail-closed
worker injection check is unchanged. The K1.3 store produced no qualification
or holdout attempts, no model calls, and never materialized holdout source.
The predecessor produced zero qualification attempts, zero holdout attempts,
zero model calls, and never opened the holdout. It was cancelled before an
outcome because its dependency DAG required holdout dependency preparation
before candidate freeze, contradicting K1-R15. K1.1 preserves the exact corpus,
repository commits/trees, workload, and decision thresholds; it changes only
the registered series identity, source-set authority binding, and the ordering
needed to prepare holdout dependencies after candidate freeze and holdout
source materialization.

## Objective

K1 qualifies the existing Track-B Kotlin contour on pinned real repositories.
It must turn a real Gradle or Maven Kotlin/JVM project into one of two retained,
canonical outcomes:

1. an exact-snapshot-bound, evidence-core-validated Repository Understanding
   projection plus conservative Change Impact result; or
2. a typed `PARTIAL`, `REFUSED`, or `FAILED` attempt that retains the exact
   stage, safe failure identity, provenance and cost.

An exception, panic, timeout, OOM, nonzero worker exit, invalid JSON or empty
stdout without a retained typed attempt is not an acceptable refusal.

This is an engineering/applicability qualification, not a model-benefit
experiment. It makes zero model calls and cannot unlock edit/apply.

## Why K1 exists

M1 reached a valid frozen language-neutral K0.1 core, but its first real Kotlin
Maven repository failed after 174.84 seconds and about 4.82 GiB RSS. K2/FIR
emitted six local constructor descriptors with
`effectiveVisibility = "local"`; a closed legacy Rust validator rejected the
compiler enum. The process produced no canonical output and no typed refusal.

The failure was fail-safe—there was no false proof or source mutation—but it
showed that fixture conformance was insufficient. K1 therefore addresses the
whole real-project contour: total language-owned translation, build and
dependency identity, multi-module Gradle/Maven discovery, canonical failure
retention, offline replay, cross-process semantic caching and cost telemetry.

## Immutable inputs

- research report SHA-256:
  `6b9d9c73a809e896506dfd2645d09b77e8251940138eb813c85aeb573a270791`;
- original execution contract SHA-256:
  `a115a0690a7fe9ffc79d6cfbe2f31f2a58bc3412f9af44d22dd6e336765c35ee`;
- M1 real-repository failure artifact SHA-256:
  `9d8137ac0063dc8fc81b1f0f3c577ad41550000863741421b690265b1a3e2d49`;
- historical K0.1 lock SHA-256:
  `2fe26d4605f20137f4309067773c6764fffe2933696a338ec269f9d240bf4d91`;
- baseline repository revision:
  `be19fd06ce7509ffbaf4622b8f8d13ec7cbcd854`.

Historical K0.1 files are byte-read-only. K1 may add Kotlin-owned translation,
attempt, cache and qualification code, but it may not rewrite the K0.1 schema,
decision core, conformance corpus or four frozen shared adapter-authority
files.

## Frozen requirements

The normative requirements and thresholds are in
[`requirements.json`](../../benchmarks/kotlin-real-repository/k1/requirements.json).
The important conjunction is:

- exactly 12 retained attempts: six qualification and six blind holdout;
- all 12 replay offline twice with equal semantic digests;
- at least five of six holdout repositories produce nonempty validated
  projections; at most one holdout and at most two total typed refusals;
- at least one validated result for each Gradle KTS, Gradle Groovy and Maven,
  and for each Kotlin 2.1, 2.3 and 2.4 minor line;
- zero untyped failures, false `PROVEN`, false `COMPLETE_IN_SCOPE`, source
  mutations and model calls;
- at least five of six holdout entries demonstrate a real cross-process cache
  hit; median warm provider wall is at most 80% of cold and median warm
  end-to-end wall is at most 90% of cold;
- each invocation is bounded to 900 seconds and 8 GiB maximum RSS;
- cost telemetry is complete for every attempt.

These thresholds cannot be changed after seeing qualification or holdout
outcomes. A cost/applicability miss with intact totality and proof safety is a
`PIVOT`, not permission to lower the threshold.

## Frozen corpus

The normative manifest is
[`corpus.json`](../../benchmarks/kotlin-real-repository/k1/corpus.json).
It contains 12 exact commit/tree pins from ten organizations:

| Cohort | Maven | Gradle Groovy | Gradle KTS |
| --- | ---: | ---: | ---: |
| Qualification | 1 | 2 | 3 |
| Blind holdout | 2 | 1 | 3 |
| Total | 3 | 3 | 6 |

The corpus covers Kotlin 2.1.x, 2.3.x and 2.4.x, multi-module Gradle and Maven,
composite build logic, compiler plugins, symbol processing, generated sources,
mixed Java/Kotlin and an IntelliJ plugin boundary.

Qualification projects may be inspected and rerun while implementing K1.
Before the candidate freeze, holdout access is limited to repository identity
and build/config metadata. Kotlin source semantics, tests, issues, prior
adapter outcomes and project-specific semantic recipes are prohibited. After
the candidate source/binary/harness freeze, a failed holdout result terminates
the series as preregistered; it cannot trigger a patch-and-retry cycle.

Holdout dependency preparation happens after candidate freeze and holdout
source materialization, and its seed is then verified before any holdout run.
Exact Maven 3.9.12, Temurin
21.0.11 and trusted Kotlin analyzer distributions 2.1.21, 2.3.0 and 2.4.10
are recorded in the corpus manifest. Project-declared patch versions remain
separate snapshot inputs and are never rewritten to the analyzer version.

## Required implementation surface

### Total Kotlin-owned FIR translation

Raw compiler strings remain adapter-owned. Every descriptor and relation row
is translated independently. Known values map to the stable Kotlin contract;
unknown/future values quarantine the row and create a deterministic boundary.
References to quarantined identities also become boundaries. The regression
must include `CONSTRUCTOR/effectiveVisibility=local`, all known enum values and
synthetic future declaration kind, visibility, effective visibility, modality,
type, relation, identity and range values.

### Exact build and dependency identity

Compiler argument order is semantic and must be preserved. Gradle reflective
model fields cannot silently default after an exception. Maven reactors and
Gradle multi-project builds must select the manifest's exact JVM module/source
set. Snapshot identity binds declared project Kotlin/JDK/build-tool versions,
the exact analyzer distribution, target/options, classpath member bytes,
resolved coordinates/scopes/repositories where available, plugins/buildscript
inputs and generated-source producers/manifests.

### Typed attempts

The adapter or controller writes a canonical attempt for every stage. Success
is published only after schema, seal and evidence-core validation. Semantic or
infrastructure failure writes a separate immutable attempt with no positive
receipt. Caller-authored error JSON cannot be adopted as authority.

### Offline and cache contour

Dependency preparation is an explicit network-enabled PREPARE node. Decision
runs use Gradle offline/Maven offline plus an OS-level network-denial sandbox.
The semantic cache lives outside the source checkout, is content-addressed by
all semantic inputs and is revalidated before use. Repository-owned cache JSON,
symlinks, partial objects and stale/corrupt entries never authorize evidence.

The workload is harness-derived, never caller-selected: after a cold adapter
result validates, the harness chooses the lexicographically first resolved,
source-defined entity with an incident relation fact. The frozen query uses
depth 2 and at most 128 entities. A nonempty projection includes an
evidence-core binding, entity, relation, source-bound occurrence and impact
receipt; an ungrounded whole-repository may-set does not count.

Cold and warm are separate processes. Refusals replay through a terminal
semantic digest over canonical stage, reason, boundaries and provenance, so
all 12 entries stay in the replay denominator. Refusals never improve the
warm-speed ratio population.

### Telemetry

Both `/usr/bin/time -l` and internal monotonic events are retained. Required
stages include source hashing, build discovery, dependency prepare/verify,
adapter startup, cold/warm index, provider work, serialization, store write and
read, query/projection, byte/fact/boundary counts and cache requests/hits.

## Readiness graph

The normative DAG is
[`readiness-graph.json`](../../benchmarks/kotlin-real-repository/k1/readiness-graph.json).
Its critical ordering is:

```text
inputs + K0.1 + requirements + corpus
-> baseline + harness + qualification dependency seed
-> six qualification runs
-> candidate source/binary/harness freeze
-> holdout source materialization + dependency seed
-> six blind offline cold/warm pairs
-> totality/safety + applicability + cache/cost audits
-> independent audit
-> exactly one GO, PIVOT, or STOP terminal root
```

No model-run node exists. Receipt currentness is live-input-based and
transitive; alternate caller graphs, corpora, thresholds, success strings or
report paths cannot mint a root.

## Decision semantics

- `GO / KOTLIN_REAL_REPOSITORY_READY`: every requirement and threshold passes.
- `PIVOT / KOTLIN_APPLICABILITY_OR_COST_GAP`: proof safety and totality pass,
  but applicability, offline dependency closure, real cache or cost fails.
- `STOP`: false proof/completeness, mutation, K0.1 drift, authority bypass,
  post-freeze holdout tuning, corpus/threshold rewrite or an unretained failure.

The later model experiment remains `NOT_STARTED_WITH_REASON` regardless of K1
outcome. A K1 GO only authorizes a separately preregistered product-benefit
experiment.
