# Bounded cross-language specification scaffold

Status: **UNTESTED_SCAFFOLD**. Owner hypothesis: H14. Source obligations: S4 questions 25-26 and deliverable 15; S5's migration analysis. Reporting destinations: R01 and GF. Empirical validation is outside the current product proof and may occur only in separately human-approved post-GO Z01.

This artifact is a template for stating and testing a narrow behavioral preservation claim. It is not a source-to-source transform, universal IR, universal transpiler, proof of migration correctness, or evidence that Kotlin and Rust programs are generally equivalent.

## Q25 — How the model may support migration

Use threads, contracts, declared states, effects, invariants, example traces, and source-linked evidence to define a **specification scaffold and differential-test plan**. For one selected source/target component, record what callers can observe, which state transitions are accepted or rejected, required external effects and ordering, compatibility/security obligations, and where platform adaptation is required. Implementations remain language- and framework-native; Codeclew must not mechanically translate source semantics.

The motivating example may be a bounded `Kotlin/Spring/JPA -> Rust/Axum/SQLx` component, but selecting that pair does not assert equivalence. The scaffold must not copy JPA persistence-context behavior, Spring transaction propagation, JVM exception/nullability/concurrency semantics, coroutines, reflection, DI lifecycle, or resource disposal into Rust as if they matched ownership, futures, Axum, or SQLx.

## Q26 — What can and cannot be established

The strongest permitted claim shape is:

> Equivalent with respect to the declared observables, tested input domain, specified state transitions, and stated platform assumptions listed in one immutable Z01 case.

Schema/API compatibility, language-local type constraints, and formally declared finite-state transitions may admit static checks. Output/effect equivalence, ordering, failure categories, and idempotency require differential/property tests and often runtime traces or fault injection. Performance requires a benchmark or production telemetry. Runtime traces demonstrate observed cases, not completeness. Full concurrency equivalence and arbitrary whole-program behavioral equivalence are normally unprovable here and must remain `UNKNOWN` or out of scope.

## Immutable case header

A future Z01 case must fill every field before target results are opened:

```yaml
case_id: "Z01-<version>-<domain>"
status: "PREREGISTERED_UNTESTED"
source_snapshot_digest: "<digest>"
target_snapshot_digest: "<digest>"
source_platform: "Kotlin/JVM <exact>; JDK 21; <Gradle|Maven>; <framework versions>"
target_platform: "<language/runtime/framework/database exact versions>"
domain_boundary: "<one component or journey; explicit entry/exit points>"
input_generator_digest: "<digest>"
oracle_digest: "<independent specification or corpus digest>"
declared_observable_ids: ["OBS-..."]
declared_transition_ids: ["TR-..."]
platform_assumption_ids: ["PA-..."]
excluded_behavior_ids: ["EX-..."]
```

Changing any field creates a new case version. Counterexamples, refused inputs, timeouts, exclusions, and unsupported platform behavior stay in the result set.

## Declared observables only

Each observable needs an ID, source anchor/provenance, canonical comparison rule, oracle owner, and explicit Unknown behavior. The default bounded registry is:

| ID family | May be declared | Not implied |
| --- | --- | --- |
| `OBS-HTTP-*` | Request method/path, normalized input schema, response status/schema, error category | Framework interceptor order, exception identity, byte-for-byte incidental headers |
| `OBS-STATE-*` | Accepted/rejected transition and externally visible post-state | Internal object graph, ORM identity map, hidden cache state |
| `OBS-DB-*` | Committed logical row/value effects, uniqueness and isolation obligation | JPA flush timing or SQLx transaction implementation identity |
| `OBS-MSG-*` | Message schema/key, emission condition, multiplicity, declared ordering | Broker/runtime scheduling not declared in the case |
| `OBS-EFFECT-*` | Named external effect, cardinality, idempotency key and happens-before constraints | Undeclared logging, timing, allocation, thread identity |
| `OBS-SEC-*` | Declared authorization decision and data-release policy | General security equivalence outside selected entry points |

Inputs outside the frozen domain return `OUTSIDE_TESTED_DOMAIN`; an unobservable or unsupported result returns `UNKNOWN`, not equivalent.

## Platform assumptions and adaptation obligations

Every case must explicitly decide or exclude:

- nullability, numeric and serialization semantics;
- exception/error mapping and cancellation;
- scheduler, thread, coroutine/future, and memory-model assumptions;
- database isolation, transaction boundary, retry, flush, and rollback behavior;
- dependency-injection lifecycle, framework interception, reflection, and generated code;
- resource acquisition/disposal and timeout behavior;
- clock, locale, randomness, external services, and message-delivery semantics.

An assumption is not evidence. If an assumption affects an observable, the target needs an adaptation obligation and an independent oracle. Unresolved assumptions block the affected equivalence claim.

## Property and evidence matrix

| Property | Static check | Differential/property test | Runtime/fault evidence | Permitted conclusion |
| --- | --- | --- | --- | --- |
| API/schema compatibility | Often | Required for encoded examples | Optional | Compatible for declared schema/version |
| Language-local type constraints | Yes, within each language | Sometimes | No | Each side satisfies its own declared constraint |
| Declared finite-state transitions | Only with a formal model | Required | Useful | Same tested accepted/rejected transitions |
| Output equivalence | No general proof | Required | Useful | Same canonical output over tested inputs |
| External effects and ordering | Partial | Required | Required where observable | Same declared effects/order over tested schedules |
| Failure semantics | Partial mapping | Fault injection required | Required | Same declared error category for injected cases |
| Concurrency equivalence | Rarely | Bounded schedules only | Does not prove completeness | No universal claim; only tested schedule obligations |
| Performance obligation | No | Benchmark | Production telemetry if claimed | Only threshold on stated environment/population |

## Differential-test obligations

Before any bounded evidence verdict, Z01 must provide:

1. An independent oracle or specification; source implementation output alone cannot be the sole oracle.
2. Canonical input/output and state normalization that excludes only preregistered incidental fields.
3. Positive, rejected, boundary, malformed, repeated/idempotency, stateful-sequence, and fault-injection cases.
4. Tests for each declared transition, error category, external effect, multiplicity, and partial-order edge.
5. Bounded schedule exploration for every concurrency statement; untested schedules remain Unknown.
6. Source and target snapshot digests, exact platform versions, seeds/generator digest, raw observations, and mismatch/counterexample retention.
7. Hidden or independently held-out differential/property cases and an anti-overclaim review.
8. A result per observable: `SUPPORTED_ON_TESTED_DOMAIN`, `COUNTEREXAMPLE`, `INCONCLUSIVE`, `OUTSIDE_TESTED_DOMAIN`, or `UNKNOWN_PLATFORM_SEMANTICS`.

Passing examples alone cannot yield `BOUNDED_CROSS_LANGUAGE_EVIDENCE`; every declared observable must have a result, every exclusion must remain visible, and no counterexample may be discarded by changing the case after reveal.

## Explicit no-transpiler boundary

Forbidden outputs include generated target source from this scaffold, claims of semantic preservation for undeclared behavior, arbitrary program equivalence, framework-runtime equivalence, or statements that an IR can mechanically port Kotlin/Spring/JPA behavior to Rust/Axum/SQLx. Language-specific implementation choices, ownership, error handling, transactions, async execution, and lifecycle remain target-engineering decisions checked only against declared observables.

R01 materializes this scaffold; GF cites it as `UNTESTED_SCAFFOLD`; A01 may separately authorize one Z01 experiment; Z01 may return only bounded evidence, inconclusive evidence, or a cross-language overclaim stop. None of those outcomes changes the already audited primary GF verdict.
