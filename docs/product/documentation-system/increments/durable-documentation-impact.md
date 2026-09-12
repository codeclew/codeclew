# Increment impact: durable documentation system

## Purpose

Extend the existing source-bound service/scenario workflow into durable structured
knowledge with bounded agent authoring, protected human material and best-effort
multi-repository updates. Existing source snapshots, annotation interpreters,
Narrative compatibility and atomic publication are reused rather than replaced by
a second product. [Target](../target-system.md) and [decisions](../decisions.md) are approved;
the [implementation plan](../../../plans/documentation-system-implementation-plan.md) was approved on 2026-09-12.

## Pre-scan

- **Pre-scan:** [durable-documentation-pre-scan.md](durable-documentation-pre-scan.md)

## Change types

`extends`, `changes`, and `adds`. Existing public commands, supported Narrative readers
and Kafka behavior are preserved unless a versioned compatibility path is explicitly
provided. No current scenario is silently deprecated.

## Affected scenarios

| Scenario card | Impact type | Touched extension points | Changed happy-path steps | Required artifact updates | Regression checks |
| --- | --- | --- | --- | --- | --- |
| S01 | extends | source.capture, provider.selection | Capture one service, import compatible evidence, request standard pages | Baseline S01/S07/S11, cards, graph, plan | No-build admission, identity validation, optional-provider loss |
| S02 | changes | authoring.package, authoring.proposal, authoring.verification | Replace manual internal record construction with constrained proposals and review | Baseline S02/S08, cards, graph, plan | Evidence/branch coverage and old Narrative readability |
| S03 | extends | process.definition, process.composition | Save maintained processes and entity flow views with explicit uncertain edges | Baseline S03/S09, cards, graph, plan | Declared HTTP/Kafka boundaries and bounded composition |
| S04 | extends | reader.structure, reader.navigation, reader.status | Display standard objects/notes/history and immediate stale status | Baseline S04/S07/S10/S12, cards, graph, plan | Offline exact source, consistent views, immutable outputs |
| S05 | changes | freshness.propagation, freshness.status-publication, freshness.review | Mark before generation, track captured influence and review only affected work | Baseline S05/S08/S12, cards, graph, plan | Relocation, helpers/configuration, missing history, transitive changes |
| S06 | extends | notes.ownership, history.recovery | Import/associate human notes and restore portable snapshots independently of caches | Baseline S06/S10/S12, cards, graph, plan | Manual byte preservation, concurrent edits and previous bundle recovery |

## Added scenarios

| New scenario | Entry | Exit / next | Why this is not an island |
| --- | --- | --- | --- |
| S07 | S01 | S04, S09, S10 | Standard descriptions enrich existing registration and reading |
| S08 | S02, S05 | S04 | Bounded author/reviewer work implements existing authoring/refresh intent |
| S09 | S03, S07 | S04, S05 | Saved process/entity views reuse section navigation and freshness |
| S10 | S06, S07 | S04 | Human note association extends existing protected manual content |
| S11 | S01 | S07, S05 | Explicit modules extend existing analysis selection |
| S12 | S05 | S08, S04, S06 | CI update/history extends existing maintenance and recovery |

## DOT diff

[Canonical graph](../scenario-graph.dot) contains these dashed increment edges and
all existing paths; the snippet illustrates the main connected addition.

```dot
S01 -> S07 [label="increment: durable-documentation"];
S02 -> S08 [label="increment: durable-documentation"];
S05 -> S12 [label="increment: durable-documentation"];
S12 -> S08 [label="bounded refresh"];
S08 -> S04 [label="accepted content or explicit gap"];
S07 -> S09 [label="saved view request"];
S09 -> S05 [label="maintained dependency"];
S06 -> S10 [label="protected note association"];
S10 -> S04 [label="related human knowledge"];
```

## Requirements changes

| Scenario | FR/NFR/AC/Test plan | Change | Reason |
| --- | --- | --- | --- |
| S04, S05 | AC01/AC02 | changed | Visible status and best-effort updates replace stale-report-only and global blocking |
| S02, S08 | AC03-AC06/AC20 | changed | Product owns authoring mechanics; independent meaning review and measured fallback |
| S01, S11 | AC07-AC09/AC15 | extends | Reuse optional semantics and Rust Spring while exposing modular facts/interchange |
| S07, S09 | AC10/AC11/AC13/AC14 | adds | Stable standard entity/contract/process/data-flow sections |
| S06, S10 | AC12 | extends | Preserve human material while separately assessing it |
| S05, S07, S09, S10, S11 | AC18 | extends | Full modeled influence and transitive view mutation qualification |
| S06, S12 | AC16/AC17/AC19 | adds | Immutable revision history, portable CI and operational recovery |

The canonical wording of AC01-AC20 remains in the [target](../target-system.md#acceptance-contract-and-traceability).

## Verification impact

Existing source-only and actual JVM acceptance remain regression inputs. New public
CLI tests must cover status-only publication, partial capture, supported work-package
reads, proposal/review binding, entity/contract/module changes, human note concurrency,
portable evidence validation, event ordering and cache-free recovery. Visual checks
compare heterogeneous service fixtures using the same renderer. Runtime production
qualification and actual routine/fallback model evaluation are future plan tasks,
not claimed by this planning package's structural or independent review.

## Implemented task mapping

- T00 → S04/S05, AC01/AC02: independent status publication is implemented and covered
  by public CLI acceptance. Other listed growth behavior remains pending.
- T01 → S01/S04/S05, AC02/AC18: local outcomes and immutable operation evidence
  versions are implemented. Five T00/T01 CLI cases cover current regression behavior.

- T02 → S02/S05/S08, AC03/AC18: bounded immutable evidence work, explicit external
  input admission and recorded query/read influence are implemented. Regression
  evidence covers retained content, negative queries gaining members, changed
  source and notes, cursor misuse, forged handles and over-budget records. The
  remaining isolation and proposal obligations belong to T03/T04.

- T03 → S02/S08, AC04/AC18: closed proposals, canonical materialization, provider
  field checks and deterministic repair diagnostics are implemented. Four CLI
  cases cover unsupported/contradicted claims, required branches, forged fields,
  stale/untracked/incomplete work and legacy rendering without meaning approval.

- T04 → S08/S12, AC05/AC06/AC20: isolated stdio roles, coordinator acceptance,
  immutable review bindings, explicit limitations and conservative finite
  accounting are implemented. Twelve public CLI cases exercise real Seatbelt
  access denials and deterministic fake models. Captured note changes invalidate
  retained accepted content; legacy replacements lose review acceptance. The
  adapter is currently macOS-only; actual model/CI qualification remains T16.

- T05 → S01/S11, AC07/AC15: module discovery and explicit optional producer
  selection reuse the existing registry and compiler lifecycle. New settings
  preserve source-first readability and conservative module influence. Three
  new CLI tests and actual Kotlin 1.9/javac recovery checks pass; adapter registry,
  Python syntax, Kafka and accepted-review paths remain regression evidence.

- T06 → S07/S11, AC08: one Rust framework interpreter accepts separately validated
  source annotations and existing compiler facts. Literal Java/Kotlin declarations
  share rules; unresolved names, values, composition and inheritance stay gaps.
  Source/public CLI, compiler-metadata regressions and a Boot 3.3.0 managed MVC
  fixture pass. No worker protocol change or new runtime-registration claim.

- T07 → S07/S11, AC09: an independent declared OpenAPI module captures explicit
  committed selections and registered references without compiler endpoint
  discovery. Unmapped operations render as declarations; unsafe/missing/cyclic/
  external references and unsupported versions remain gaps. Three CLI cases,
  documentation regressions and the shared expansion-budget test pass. Contract
  changes invalidate bound service/process claims outside language source roots.

- T08 → S01/S04/S07, AC10/AC11: predefined sections and declared domain entity
  relationships are implemented. Existing operations retain their IDs, required
  section gaps remain navigable, agent proposals cannot alter human relationships,
  and explicit linked entity facts participate in transitive bindings. Targeted
  CLI tests passed; final shared-pipeline and layout evidence is recorded in T08.

- T09 → S06/S10, AC12/AC18: exact human/imported text, association metadata and
  generated assessments have separate ownership. Stable explicit service/section/
  entity/process targets require no name inference. Note and association changes
  invalidate work/accepted assessments, while status-only refresh preserves the
  prior snapshot. Four focused CLI cases, nine shared publication/section/note
  regressions and 25 documentation unit cases pass; layout and source inspection
  checks are recorded in T09. No strict completeness claim for arbitrary prose.
