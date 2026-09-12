# Impact pre-scan: durable documentation system

## Purpose

This increment extends the existing documentation subsystem across authoring,
reader navigation, evidence providers and update operations. It does not replace
Codeclew's general code-navigation, editing, release or research product.

## Evidence opened

| Evidence | Why opened | Signal |
| --- | --- | --- |
| [Baseline](../current-scenario-baseline.md) | Identify current journeys | S01-S06 are current; S07-S12 are approved growth |
| [Cards](../scenario-cards.md) | Read user stories and extension points | Registration, authoring, reading and maintenance all change |
| [Documentation model](../../../../crates/clew/src/documentation/model.rs) | Determine native objects | Service/scenario operations exist; domain entities and owned note assessments do not |
| [Check](../../../../crates/clew/src/documentation/check.rs) and [render](../../../../crates/clew/src/documentation/render.rs) | Inspect update semantics | Sequential full capture and global unresolved-source publication block conflict with approved best effort |
| [Bindings](../../../../crates/clew/src/documentation/bindings.rs) | Identify reusable freshness | Per-fragment references and conservative source watches exist |
| [Facts](../../../../crates/clew-facts/src/lib.rs), [Spring](../../../../crates/clew-framework-spring/src/lib.rs), [module metadata](../../../../crates/clew/src/analysis_modules.rs) | Avoid duplicate provider infrastructure | Shared Rust annotation rules and module metadata already exist; generic documentation selection/interchange is missing |
| [Skill](../../../../skills/codeclew/references/service-documentation.md) and [init instructions](../../../../crates/clew/src/documentation/store.rs) | Inspect actual author burden | Manual Narrative assembly; inconsistent version guidance; no bounded author/reviewer job protocol |

## Candidate affected scenarios

| Scenario | Why candidate | Touched extension points | Evidence to read next |
| --- | --- | --- | --- |
| S01 | Selective capture, modular providers and automatic onboarding | source.capture, provider.selection | analysis.rs, store.rs, S07/S11 cards |
| S02 | Bounded authoring replaces manual internal record assembly | authoring.package, authoring.proposal, authoring.verification | cli.rs, model.rs, S08 card |
| S03 | Persistent process and entity views extend declared scenarios | process.definition, process.composition | check.rs, S09 card |
| S04 | New section navigation and independently visible status | reader.structure, reader.navigation, reader.status | render.rs, app.js, S07/S10/S12 cards |
| S05 | Immediate status, transitive influence and bounded rechecking | freshness.propagation, freshness.status-publication, freshness.review | bindings.rs, S08/S12 cards |
| S06 | Protected notes and portable recovery/history | notes.ownership, history.recovery | store.rs, S10/S12 cards |

## Rejected scenarios

| Scenario | Why not affected | Evidence |
| --- | --- | --- |
| External: general managed source editing | This workflow analyzes application source read-only; it does not change mutation authorization or publication of code | [Repository agreements](../../../../AGENTS.md), [target scope](../target-system.md#explicit-scope-boundaries) |
| External: existing release qualification/research arms | Historical qualification and token studies are evidence, not new execution instructions | [Prior report](../../validation/source-documentation-qualification.md), [target G9](../target-system.md#g9-portable-integration-and-production-qualification) |
| External: deployment discovery | Latest accepted source and optional labels were approved; deployment-specific mechanics were excluded | [D02](../decisions.md#d02-latest-accepted-source-and-immutable-history) |

## Cross-cutting checklist

| Area | Baseline scenario | Affected? | Rationale |
| --- | --- | --- | --- |
| Auth / session / legal | S01 | yes | Preserve source access boundaries, existing supported session lifecycle and credential-free records; no new identity provider or legal workflow |
| Search / recall / navigation | S04 | yes | Structured objects, saved views and related human notes need stable links and discoverability |
| Settings / preferences / privacy | S06 | yes | Protected authored metadata and provider/agent configuration must not leak credentials or lose ownership |
| Operator / admin / runtime | S05 | yes | Change events, selective work, partial failures, retries, history and recovery replace whole-catalog blocking behavior |

## Open decisions

No unresolved product scope decision. D01-D12 are approved. Operational values and
technical contracts are defined in the plan as configuration or implementation proposals,
with measured qualification before a production-readiness claim.

## Recommendation

Proceed to [impact](durable-documentation-impact.md) with S01-S06 affected and
S07-S12 added through the existing user paths. The evidence above and the scoped
scenario cards are the planning baseline; no full-product archive audit is required.
