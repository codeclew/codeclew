# Documentation-system current scenario baseline

Status: observed development baseline plus explicitly marked growth scenarios.
Scope: the documentation subsystem only; unrelated Codeclew navigation, editing,
release and research workflows are external to this increment. Base revision:
`a2dcefe2296e65f8f98e1cb205b2bbe3589e7989`.

## Purpose

Anchor the [approved target](target-system.md) and [implementation plan](../../plans/documentation-system-implementation-plan.md)
in existing reader/author/operator journeys. Current means source-observed behavior;
it does not assert a released capability or production qualification.

## Evidence sources

| Area | Evidence paths | Confidence | Unknowns |
| --- | --- | --- | --- |
| Reader and publication | [Renderer](../../../crates/clew/src/documentation/render.rs), [assets](../../../crates/clew/assets/documentation/template.html) | High, source-inspected | No multi-service production visual qualification |
| Public workflow | [CLI](../../../crates/clew/src/documentation/cli.rs), [model](../../../crates/clew/src/documentation/model.rs), [store](../../../crates/clew/src/documentation/store.rs) | High, source-inspected | New section and agent APIs absent |
| Freshness | [Bindings](../../../crates/clew/src/documentation/bindings.rs), [check](../../../crates/clew/src/documentation/check.rs) | High, source-inspected | Current publication recaptures every service and globally refuses unresolved capture |
| Providers | [Analysis](../../../crates/clew/src/documentation/analysis.rs), [JVM facts](../../../crates/clew-facts/src/lib.rs), [Spring rules](../../../crates/clew-framework-spring/src/lib.rs), [module metadata](../../../crates/clew/src/analysis_modules.rs) | High, independent read-only discovery | No generic docs provider selection or public evidence interchange |
| Existing acceptance | [CLI tests](../../../crates/clew/tests/managed_cli.rs), [recorded qualification](../validation/source-documentation-qualification.md) | Source inspection plus prior recorded local results | No runtime tests rerun for this planning package; routine-agent qualification remains absent |

## Baseline scenarios

| ID | Scenario | Status | Persona | Entry | Exit / next | Surfaces | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| S01 | Register and capture a service | current | Documentation author | docs init / service add / bind | S02 or S03 | cli.rs, store.rs, analysis.rs, syntax.rs | Source profile is explicit; catalogue capture is sequential; provider selection is restricted. |
| S02 | Author a source-bound operation | current | Documentation author | docs context | S04 | cli.rs, model.rs, render.rs | Author manually builds Narrative records; reference checks do not verify arbitrary prose. |
| S03 | Describe a declared cross-service scenario | current | Documentation author | Interaction records and scenario definition after S01 | S02 then S04 | model.rs, store.rs, check.rs | Bounded scenarios require explicit roots/links; syntax-only unresolved handoffs do not become resolved calls. |
| S04 | Read a consistent generated publication | current | Documentation reader | docs/index.html | Operation, scenario or source detail; S05 | render.rs, assets/documentation | Shared renderer exists; root index is a service/scenario catalogue, not a complete entity knowledge portal. |
| S05 | Detect changes and review affected explanations | current | Documentation author | docs check / docs changes | S02 then S04 | bindings.rs, cli.rs, check.rs | Reports expose staleness; stale HTML is not independently republished and old affected subjects block render. |
| S06 | Preserve manual files and recover a publication | current | Documentation author | Manual notes outside generated/narratives or relocated documentation root | S01, S04 or S05 | store.rs, bindings.rs, render.rs | Manual files survive but have no automatic related-section view or assessment model; caches are local. |
| S07 | Generate a structured service and entity catalogue | growth | Documentation author | S01 registration | S04; S09; S10 | Target G1/G2/G6; AC08-AC11 | Requires standard section kinds, domain identities, complete boundary inventory and shared framework/contract interpretation. |
| S08 | Author, review and escalate bounded agent work | growth | Documentation author | S02 authoring or S05 invalidation | S04 or visible generation gap | Target G3/G4/G8; AC03-AC06/AC20 | Requires a constrained proposal API, captured reads, review-bound acceptance and portable agent roles. |
| S09 | Save a process or entity data-flow view | growth | Documentation author | S03 scenario or S07 entity page | S04; S05 after source change | Target G6; AC11/AC13/AC14 | Requires persistent semantic section definitions and typed evidence-backed entity flow edges. |
| S10 | Attach and reassess protected human notes | growth | Service team member | S06 manual files or S07 entity/contract/process | S04; S05 reassesses generated opinions | Target G7; AC12 | Requires import, stable associations and independent assessment ownership. |
| S11 | Configure optional evidence modules | growth | CI operator | S01 service registration/settings | S07 or S08; S05 on provider change | Target G2; AC07-AC09/AC15 | Built-in metadata and adapter registry exist; documentation does not expose a generic module contract. |
| S12 | Update central documentation through CI and inspect history | growth | CI operator | Accepted source event or explicit revision reconciliation | S05/S08 then S04; S06 recovery | Target G4/G5/G9; AC15-AC17/AC19/AC20 | Requires selective evidence interchange, event ordering, status-only publication, history manifests and integration recipes. |

## Scenario graph

The [DOT graph](scenario-graph.dot) and [cards](scenario-cards.md) distinguish observed
paths from dashed increment paths. Entry/exit nodes are explicit. Growth rows do not
change the observed status of the six current scenarios.

## Baseline invariants

Every current scenario has a card, live evidence, entry, outcome and regression checks.
Growth journeys attach to current registration, authoring, reading or maintenance.
The [pre-scan](increments/durable-documentation-pre-scan.md) and
[impact](increments/durable-documentation-impact.md) identify the changed extension points.
After implementation, each task records its demonstrated delta; do not rewrite this
snapshot as if growth behavior existed at the base revision.

## Observed implementation deltas

- T00: status-only publication now retains original explanations and evidence while
  exposing content/target revisions and current/stale/unverified state. Missing
  sources are local gaps; old bundles and human files remain unchanged. Public CLI
  acceptance covers helper changes, unavailable source, repeat publication and
  output-edit conflicts. Graph transitions remain accurate.

- T01: publication now accepts valid operations independently, preserves rejected
  siblings with their own evidence versions, and records local update gaps.
  Selective service checks mark unselected inputs explicitly. Existing strict
  refusal semantics remain available through `--require-complete`.

- T02: immutable work packages preserve source authority, revisions, conservative
  influence scopes, prior explanations and admitted human/imported text. Every
  supplied page and expansion is recorded, including empty query membership and
  oversized omissions. Work reads survive source changes; a new preparation sees
  the new facts. Three CLI regression cases pass. Read recording does not claim
  external-process isolation or semantic approval. Existing graph edges remain
  accurate; S08 still requires the later proposal/review tasks.

- T03: constrained proposals now materialize stable claims, explanations and
  bounded diagrams without author-created canonical IDs or approval fields.
  Supported structured equality checks can contradict a proposed outcome;
  unsupported predicates require a visible uncertainty. Required flow coverage,
  recorded reads and exact current work inputs are checked before review. Four
  CLI cases demonstrate these boundaries; prose meaning remains UNASSESSED.
  Work preparation also supplies retained-fragment change reasons. Existing
  graph transitions remain accurate pending the T04 reviewer/acceptance path.

- T04: isolated author/reviewer execution, constrained repair/fallback and atomic
  whole-path budget reservation now publish reviewed content or a local gap.
  The macOS Seatbelt adapter denies unregistered reads, writes, subprocesses and
  inherited launcher descriptors. Twelve CLI cases cover acceptance, expansion,
  replay/self-approval, injection, access denial, cancellation, malformed/time-
  limited output, accounting contention, missing usage and exhaustion. The
  cancellation regression exposed a shared-lock race; cancellation now uses an
  independent atomic signal. T00-T03 cases remain regression evidence. Separate
  reviewer calls do not imply independent model error distributions. No real
  paid model or GitLab automation qualification is claimed. Existing S08/S12
  graph transitions remain accurate; event ordering/history awaits later tasks.

- T05: `docs modules list/show` exposes existing source/javac/K2/Spring producer
  contracts and applicability without executing a project. Versioned per-service
  module settings enable optional enrichment; legacy settings retain behavior.
  Three CLI cases cover no-tool capability discovery, missing/disabled compiler
  readability, wrong-language and conflicting/executable settings. Actual
  Kotlin 1.9.25 and Maven/javac enrichment/recovery tests pass; the former retains
  its explicit language-upgrade boundary and Kafka observations. Module rule
  changes invalidate bindings without requiring source changes. S01/S11 graph
  transitions remain unchanged; normalized source Spring rules are still T06.

- T06: Java/Kotlin source adapters now normalize annotations and explicit import
  qualification into a separate source contract. The shared Rust Spring rule
  engine derives literal declarations with source authority and named gaps;
  existing compiler facts/worker contracts remain unchanged. Three CLI cases,
  two source-fact tests, two shared-source rule tests and five existing Spring
  regressions pass. Documentation unit regressions pass 25 cases. Boot 3.3.0 /
  Spring MVC 6.1.8 MockMvc verifies the Java fixture's POST and rejected GET;
  Java bytecode targets 17 while the test JVM is 21. This is bounded controller
  registration evidence, not universal Boot or production qualification. The
  fixture README records exact tested configurations. S07/S11 edges are unchanged.

- T07: declared OpenAPI operations are now independent of source endpoint
  discovery and language roots. Exact registered committed input digests and
  source occurrences bind 3.0.0/3.0.3 contracts; nested references/constraints,
  parameters, responses, security and servers remain available. Three CLI tests
  pass, including unmapped declarations, input/reference gaps, source-route
  differences and contract-only service/process dependency invalidation. Existing
  documentation unit regressions and an expansion-budget test pass. Graph S07/S11
  transitions remain unchanged; no runtime contract enforcement is claimed.

- T08: required sections are inspectable immediately after registration and rendered
  with source-bound content or visible gaps. Domain IDs and their explicit
  relationships remain distinct from classes and DTOs. Human ownership is
  protected from proposal writes; related entity facts invalidate transitive
  document bindings. Three focused CLI regressions passed for small/forty-boundary
  services, partial failure, identity/ownership and isolated section author/reviewer
  work. Shared regression and reader QA results are in the implementation plan.
  Existing S01/S04/S07 graph edges are unchanged.

- T09: protected note import/inspection/association and separate assessment roots
  are implemented. Original Markdown bytes and metadata remain distinct from
  generated conclusions and corrections. Four CLI cases pass, including concurrent
  edits, rejected embedded instructions, real sandbox write denials and stale
  assessment retention. The shared T00/T08/T09 run passes nine cases and 25
  documentation unit cases pass. Reader QA confirms escaped original text and
  mobile source inspection. Historical outcomes bind an explicit captured revision;
  arbitrary prose/calendar claims remain bounded. S06/S10 edges are unchanged.


- T10: explicit maintained process definitions and separate reviewed overviews
  are implemented through the existing scenario pipeline. Linked children retain
  accepted claim/definition versions and exact source influence; changed accepted
  child prose invalidates its parent in the same publication. Four focused CLI
  cases pass, as do seven shared publication/note/process cases and 25 documentation
  unit cases. Final exact-revision admission refinement passes the focused child/
  parent case. Desktop/mobile navigation and exact source inspection pass, including
  390-pixel layout without horizontal overflow. Conditional syntax observations
  do not establish a resolved cross-service HTTP call. S03/S09 edges are unchanged;
  entity data-flow was subsequently delivered in T11 below.
- T11: the versioned built-in entity view contract now exposes domain and
  field/function/DTO/message/table nodes with typed read/transform/write/transfer
  edges. Four CLI cases passed for protected human material, bounded independent
  review, source attribution, declared/unknown authority and transitive mapper
  invalidation with independent entity reuse. Adding a relevant interaction and
  editing a human annotation invalidate retained views. Shared process, note and
  section regressions pass. Desktop/mobile source inspection preserves exact
  retained code and contains diagram overflow at 390 px. S09 transitions remain
  unchanged; portable evidence is still T12.
