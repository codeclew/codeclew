# Documentation-system scenario cards

Read the [baseline](current-scenario-baseline.md), [graph](scenario-graph.dot), and
[approved target](target-system.md) first. These cards are the scoped scenario index
for a product-plan package, not a new full-product PRD. Current cards describe the
base revision; growth cards are approved target journeys. [Impact](increments/durable-documentation-impact.md)
and the [plan](../../plans/documentation-system-implementation-plan.md) cover both.

## S01 — Register and capture a service

- **Status:** current
- **Persona:** Documentation author
- **Entry:** docs init / service add / bind
- **Exit / next:** S02 or S03
- **Read before:** [cli.rs](../../../crates/clew/src/documentation/cli.rs), [store.rs](../../../crates/clew/src/documentation/store.rs), [analysis.rs](../../../crates/clew/src/documentation/analysis.rs), [syntax.rs](../../../crates/clew/src/documentation/syntax.rs), [CLI acceptance](../../../crates/clew/tests/managed_cli.rs)

### User story

As a documentation author, I register a repository and capture source-bound evidence so that I can document it without changing application code.

### Happy path

1. Initialize a separate documentation root and register a service with its current input digest.
2. Bind a checkout with the expected repository identity and capture the selected revision.
3. Inspect the catalogue and explicit coverage before authoring.

### Alternative / error paths

- Preserve explicit uncertainty and the last valid publication when the relevant step cannot complete.
- Source profile is explicit; catalogue capture is sequential; provider selection is restricted.

### Extension points

- `source.capture` — bounded extension within this journey; preserve its regression checks.
- `provider.selection` — bounded extension within this journey; preserve its regression checks.

### Regression checks

- Wrong origin and unsafe paths are rejected.
- Kotlin/Java/Python syntax remains available without build tools.
- Partial/failed evidence is reported without a stronger authority claim.

### Planning notes

Source profile is explicit; catalogue capture is sequential; provider selection is restricted. The acceptance mapping is in the [target](target-system.md#acceptance-contract-and-traceability).

T01 acceptance adds malformed-input and unavailable-service isolation, different
operation evidence versions on one page, selective checking and concurrent status
publication. The existing source/relocation and declared-scenario recovery checks
remain regression inputs; strict refusals use `--require-complete`.


### T05 implementation evidence

`docs modules list/show` reports built-in producer contracts, implementation
identity, availability and project applicability. Versioned per-service settings
select javac or K2 through existing admission and cleanup. Three public CLI tests
cover missing tools and invalid/ambiguous selection; actual Kotlin 1.9.25 and
Maven/javac recovery checks pass. Module/rule changes affect freshness without
source-byte changes. Python syntax and Kafka checks remain regression evidence.
Source annotation interpretation and further framework compatibility are T06.

### T08 implementation evidence

Registration now exposes five stable required section records and bounded work requests through `docs section list/show/prepare`, including before source capture. Section roots remain separate from source callable IDs.
Three targeted CLI regressions pass; final reader and shared-pipeline regression
results are recorded in the approved implementation plan. Existing graph edges
are unchanged.

### T12 implementation evidence

Per-service capture now produces a closed versioned manifest and bounded
content-addressed parts through supported local producers. A source-free report
is the default support artifact; application index/source inclusion is explicit.
Five CLI cases cover offline inspection/import/rendering without a checkout or
compiler, separate meaning review, corruption/path/authority rejection, increasing
coordinator expectations and preserved producer failures. A trusted digest is
configured separately from package integrity. See T12 in the implementation plan
and the [portable workflow](../../../skills/codeclew/references/documentation-evidence.md).
Existing S01 transitions are unchanged.

## S02 — Author a source-bound operation

- **Status:** current
- **Persona:** Documentation author
- **Entry:** docs context
- **Exit / next:** S04
- **Read before:** [cli.rs](../../../crates/clew/src/documentation/cli.rs), [model.rs](../../../crates/clew/src/documentation/model.rs), [render.rs](../../../crates/clew/src/documentation/render.rs), [CLI acceptance](../../../crates/clew/tests/managed_cli.rs)

### User story

As a documentation author, I explain an operation using exact source evidence so that readers understand its decisions and outcomes.

### Happy path

1. Read the operation context and follow required pages or focused source references.
2. Write a Narrative with explanation, contracts, events and overview bindings.
3. Supply a description or explicit gap for discovered operations and submit for rendering.

### Alternative / error paths

- Preserve explicit uncertainty and the last valid publication when the relevant step cannot complete.
- Author manually builds Narrative records; reference checks do not verify arbitrary prose.

### Extension points

- `authoring.package` — bounded extension within this journey; preserve its regression checks.
- `authoring.proposal` — bounded extension within this journey; preserve its regression checks.
- `authoring.verification` — bounded extension within this journey; preserve its regression checks.

### Regression checks

T03 adds constrained proposal submission and bounded inspection. Four public CLI
cases demonstrate stable canonical claims, legacy unassessed rendering, rejected
opposite structured outcomes and missing branches, forged authority/handles/cycles,
unknown predicates requiring gaps, stale work and unread required input. Separate
meaning acceptance is intentionally still pending T04.

T02 adds immutable `docs work prepare/read/expand` with work-local references,
recorded pagination, negative query membership and conservative scopes. Three
`docsys_t02_*` CLI tests cover source/note mutation, retained content, forged
references, cursor selection mismatch, explicit oversized records and sticky
untracked-read limitations. This implements the bounded-package extension;
proposal acceptance and isolated reviewer authority remain separate tasks.

- Omitted context items remain explicit and recoverable.
- Unbound events or missing mandatory branch coverage are rejected.
- Existing Narrative versions stay readable.

### Planning notes

Author manually builds Narrative records; reference checks do not verify arbitrary prose. The acceptance mapping is in the [target](target-system.md#acceptance-contract-and-traceability).

## S03 — Describe a declared cross-service scenario

- **Status:** current
- **Persona:** Documentation author
- **Entry:** Interaction records and scenario definition after S01
- **Exit / next:** S02 then S04
- **Read before:** [model.rs](../../../crates/clew/src/documentation/model.rs), [store.rs](../../../crates/clew/src/documentation/store.rs), [check.rs](../../../crates/clew/src/documentation/check.rs), [CLI acceptance](../../../crates/clew/tests/managed_cli.rs)

### User story

As a documentation author, I connect declared service interactions into a named scenario so that readers can follow a bounded process.

### Happy path

1. Declare interaction endpoints, transport and provenance.
2. Save a named scenario with its root and selected interaction IDs.
3. Inspect composed evidence and author the scenario explanation while retaining unknown boundaries.

### Alternative / error paths

- Preserve explicit uncertainty and the last valid publication when the relevant step cannot complete.
- Bounded scenarios require explicit roots/links; syntax-only unresolved handoffs do not become resolved calls.

### Extension points

- `process.definition` — bounded extension within this journey; preserve its regression checks.
- `process.composition` — bounded extension within this journey; preserve its regression checks.

### Regression checks

- Ambiguous selectors do not resolve by bare name.
- Declared transport and runtime proof remain distinct.
- Kafka reply paths are not converted into synchronous HTTP returns.

### Planning notes

Bounded scenarios require explicit roots/links; syntax-only unresolved handoffs do not become resolved calls. The acceptance mapping is in the [target](target-system.md#acceptance-contract-and-traceability).

### T10 implementation evidence

`process.definition`, `process.composition` and `process.saved-section` now use
explicit versioned definitions through `docs process list/show/put/inspect/prepare`.
Transient inspection saves no definition or publication. Existing scenario IDs
and HTTP/Kafka authority remain compatible. Linked current accepted child summaries
feed a separately reviewed overview; missing, cyclic, stale and unavailable scopes
remain gaps. Cyclic edges retain their transitive dependencies. Four CLI cases
cover identity/CAS, conditional source outcomes and unresolved HTTP transport,
negative interaction membership, sandboxed definition protection, child/parent
reviews and same-publication parent invalidation after a child version changes.
Final shared regression and desktop/mobile source-inspection evidence is in T10
of the implementation plan. Existing graph edges are unchanged.

## S04 — Read a consistent generated publication

- **Status:** current
- **Persona:** Documentation reader
- **Entry:** docs/index.html
- **Exit / next:** Operation, scenario or source detail; S05
- **Read before:** [render.rs](../../../crates/clew/src/documentation/render.rs), [model.rs](../../../crates/clew/src/documentation/model.rs), [bindings.rs](../../../crates/clew/src/documentation/bindings.rs), [CLI acceptance](../../../crates/clew/tests/managed_cli.rs)

### User story

As a documentation reader, I navigate readable explanations and inspect their evidence so that I can distinguish described behavior from unknowns.

### Happy path

1. Open the overview and choose a service or scenario.
2. Read the explanation, contract cards and bounded overview diagram.
3. Expand details or exact retained source text as needed.

### Alternative / error paths

- Preserve explicit uncertainty and the last valid publication when the relevant step cannot complete.
- Shared renderer exists; root index is a service/scenario catalogue, not a complete entity knowledge portal.

### Extension points

- `reader.structure` — bounded extension within this journey; preserve its regression checks.
- `reader.navigation` — bounded extension within this journey; preserve its regression checks.
- `reader.status` — bounded extension within this journey; preserve its regression checks.

### Regression checks

- The same accepted explanation supplies HTML, Markdown and Mermaid.
- Source text is readable offline.
- Output edits or inconsistent immutable bundles produce a conflict.

### Planning notes

Shared renderer exists; root index is a service/scenario catalogue, not a complete entity knowledge portal. The acceptance mapping is in the [target](target-system.md#acceptance-contract-and-traceability).

### Implementation evidence

T00 adds `docs refresh --status-only`: retained content with independently published
freshness and revision targets. `docsys_t00_*` verifies helper-change invalidation,
local unavailable-source gaps, immutable prior outputs and preserved human files.


T01 acceptance adds malformed-input and unavailable-service isolation, different
operation evidence versions on one page, selective checking and concurrent status
publication. The existing source/relocation and declared-scenario recovery checks
remain regression inputs; strict refusals use `--require-complete`.


### T08 implementation evidence

Standard overview, responsibility, entity and ingress/egress navigation is shared by small and forty-boundary services. Public boundary gaps stay visible alongside accepted sibling sections. Existing source inspection and per-section freshness use the retained evidence path.
Three targeted CLI regressions pass; final reader and shared-pipeline regression
results are recorded in the approved implementation plan. Existing graph edges
are unchanged.

### T13 implementation evidence

Reader navigation now exposes immutable snapshots and keeps each snapshot’s Overview links local. Publication manifests bind targets, section state, canonical explanation digests, input records, files and observed tags. T13 history fixtures preserve old source pages after a tag move and offline cache recovery. Existing S04 graph transitions remain unchanged.

## S05 — Detect changes and review affected explanations

- **Status:** current
- **Persona:** Documentation author
- **Entry:** docs check / docs changes
- **Exit / next:** S02 then S04
- **Read before:** [bindings.rs](../../../crates/clew/src/documentation/bindings.rs), [cli.rs](../../../crates/clew/src/documentation/cli.rs), [check.rs](../../../crates/clew/src/documentation/check.rs), [CLI acceptance](../../../crates/clew/tests/managed_cli.rs)

### User story

As a documentation maintainer, I identify affected explanations after source changes so that obsolete claims are not reused as current.

### Happy path

1. Capture current evidence and compare it with published bindings.
2. Inspect affected/unaffected fragments, moved links and before/after context.
3. Prepare changed explanations and render against the current context digest.

### Alternative / error paths

- Preserve explicit uncertainty and the last valid publication when the relevant step cannot complete.
- Reports expose staleness; stale HTML is not independently republished and old affected subjects block render.

### Extension points

- `freshness.propagation` — bounded extension within this journey; preserve its regression checks.
- `freshness.status-publication` — bounded extension within this journey; preserve its regression checks.
- `freshness.review` — bounded extension within this journey; preserve its regression checks.

### Regression checks

T02 adds immutable `docs work prepare/read/expand` with work-local references,
recorded pagination, negative query membership and conservative scopes. Three
`docsys_t02_*` CLI tests cover source/note mutation, retained content, forged
references, cursor selection mismatch, explicit oversized records and sticky
untracked-read limitations. This implements the bounded-package extension;
proposal acceptance and isolated reviewer authority remain separate tasks.

- Meaningful helper/configuration changes invalidate involved source scopes.
- Relocation changes links without rewriting unchanged prose.
- Missing history and unsupported evidence versions are unresolved.

### Planning notes

Reports expose staleness; stale HTML is not independently republished and old affected subjects block render. The acceptance mapping is in the [target](target-system.md#acceptance-contract-and-traceability).

### Implementation evidence

T00 adds `docs refresh --status-only`: retained content with independently published
freshness and revision targets. `docsys_t00_*` verifies helper-change invalidation,
local unavailable-source gaps, immutable prior outputs and preserved human files.


T01 acceptance adds malformed-input and unavailable-service isolation, different
operation evidence versions on one page, selective checking and concurrent status
publication. The existing source/relocation and declared-scenario recovery checks
remain regression inputs; strict refusals use `--require-complete`.


### T13 implementation evidence

Coordinator events select exact targets before evidence arrives and republish conservative status. Per-service sequences, immutable event IDs and target-bound package admission reject late results. Bounded configured refresh rechecks target, note and definition versions; T13 fixtures cover two-service reconciliation and conflicting in-flight work. Existing S05 graph transitions remain unchanged.

## S06 — Preserve manual files and recover a publication

- **Status:** current
- **Persona:** Documentation author
- **Entry:** Manual notes outside generated/narratives or relocated documentation root
- **Exit / next:** S01, S04 or S05
- **Read before:** [store.rs](../../../crates/clew/src/documentation/store.rs), [bindings.rs](../../../crates/clew/src/documentation/bindings.rs), [render.rs](../../../crates/clew/src/documentation/render.rs), [CLI acceptance](../../../crates/clew/tests/managed_cli.rs)

### User story

As a service team member, I retain manual context and recover generated documentation so that machine updates do not destroy authored knowledge.

### Happy path

1. Keep manual prose outside machine-owned outputs and retain published bundles.
2. Rebind relocated source checkouts and check the retained baseline.
3. Inspect or regenerate outputs; resolve a manual-output conflict explicitly.

### Alternative / error paths

- Preserve explicit uncertainty and the last valid publication when the relevant step cannot complete.
- Unassociated manual files survive; explicit note associations enable related-section views and assessments. Caches remain local.

### Extension points

- `notes.ownership` — bounded extension within this journey; preserve its regression checks.
- `history.recovery` — bounded extension within this journey; preserve its regression checks.

### Regression checks

- Manual prose remains byte-preserved.
- Modified generated files are not overwritten silently.
- Old bundles retain narratives, dependencies and available source history.

### Planning notes

Unassociated manual files survive; explicit note associations enable related-section views and assessments. Caches remain local. The acceptance mapping is in the [target](target-system.md#acceptance-contract-and-traceability).

T09 extends `notes.ownership` with byte-preserving imports, explicit associations,
separate assessments and captured-input invalidation. Protected-path sandbox
checks and concurrent-edit/association-removal CLI regressions preserve original
human files. Existing recovery/publication transitions remain unchanged.

### T13 implementation evidence

Durable admitted packages and trusted expectations reconstruct lost selection caches without a checkout or compiler. Frozen history reports missing files and expired evidence separately. An interrupted status publication retains the accepted target and repairs on retry. T13 fixtures verify old source bytes and protected human-note edits survive these paths. Existing S06 graph transitions remain unchanged.

## S07 — Generate a structured service and entity catalogue

- **Status:** growth
- **Persona:** Documentation author
- **Entry:** S01 registration
- **Exit / next:** S04; S09; S10
- **Read before:** [Approved target](target-system.md#g1-durable-authoring-and-consistent-presentation), [model.rs](../../../crates/clew/src/documentation/model.rs), [analysis.rs](../../../crates/clew/src/documentation/analysis.rs), [CLI acceptance](../../../crates/clew/tests/managed_cli.rs)

### User story

As a documentation author, I enroll a service and receive a standard responsibility, entity and contract description so that the service is understandable before custom process work.

### Happy path

1. Register the source scope and explicit analysis modules.
2. Generate standard sections from bounded work packages, with inferred ownership and missing facts visible.
3. Read the common page structure and follow entities or contracts into deeper views.

### Alternative / error paths

- Preserve explicit uncertainty and the last valid publication when the relevant step cannot complete.
- Requires standard section kinds, domain identities, complete boundary inventory and shared framework/contract interpretation.

### Extension points

- `service.standard-sections` — bounded extension within this journey; preserve its regression checks.
- `entity.identity` — bounded extension within this journey; preserve its regression checks.
- `contract.catalogue` — bounded extension within this journey; preserve its regression checks.

### Regression checks

- Two different repositories share required reader sections or explicit gaps.
- Entity ownership is not inferred as authority from a class/table name.
- Every discovered in-scope public boundary is described or visibly incomplete.

### Planning notes

Requires standard section kinds, domain identities, complete boundary inventory and shared framework/contract interpretation. The acceptance mapping is in the [target](target-system.md#acceptance-contract-and-traceability).

### T06 implementation evidence

Java/Kotlin source annotation facts feed the shared Rust Spring interpreter.
Qualified imports, Kotlin aliases and literal mappings derive source declarations;
unrelated/ambiguous annotations and unresolved aliases/values/hierarchy remain
explicit gaps. Three CLI tests and typed-contract/shared-rule tests pass alongside
existing Spring compiler-metadata and documentation regressions. The Boot 3.3.0
fixture verifies a bounded MockMvc route; see its exact [compatibility matrix](../../../fixtures/documentation-system/spring/README.md).
The existing compiler schema and worker bridges are unchanged.

### T07 implementation evidence

The independent declared OpenAPI module captures explicitly registered committed
files outside language roots. Three no-tool CLI cases preserve nested contracts,
local/cross-file references and unmatched operations; reference failures and
unsupported versions remain visible. Tested dialects are 3.0.0 and 3.0.3. A unique
source route match remains separate from contract authority and runtime behavior.
Contract-only mutations invalidate service and process fragment dependencies;
a shared expansion budget bounds repeated references. Existing documentation
unit regressions pass. See the [fixture scope](../../../fixtures/documentation-system/openapi/README.md).

### T08 implementation evidence

The new section and entity records distinguish domain IDs from implementation representations. Human relationships cannot be changed by agent-proposal writes; duplicate titles and explicit related-entity links do not cause identity rebinding. Section authoring reuses the isolated author/reviewer pipeline and transitive entity dependencies invalidate affected documentation.
Three targeted CLI regressions pass; final reader and shared-pipeline regression
results are recorded in the approved implementation plan. Existing graph edges
are unchanged.

## S08 — Author, review and escalate bounded agent work

- **Status:** growth
- **Persona:** Documentation author
- **Entry:** S02 authoring or S05 invalidation
- **Exit / next:** S04 or visible generation gap
- **Read before:** [Approved responsibilities](target-system.md#g8-bounded-agents-and-explicit-verification), [cli.rs](../../../crates/clew/src/documentation/cli.rs), [render.rs](../../../crates/clew/src/documentation/render.rs), [CLI acceptance](../../../crates/clew/tests/managed_cli.rs)

### User story

As a documentation maintainer, I let a routine agent perform bounded authoring with checks and fallback so that updates are useful without relying on a top-tier model for every task.

### Happy path

1. Request or receive a bounded evidence package with obligations and expansion references.
2. Run the author and separate meaning review through configured adapters.
3. Apply bounded repair or justified fallback; publish accepted content or a section-local gap.

### Alternative / error paths

- Preserve explicit uncertainty and the last valid publication when the relevant step cannot complete.
- Requires a constrained proposal API, captured reads, review-bound acceptance and portable agent roles.

### Extension points

- `agent.work-unit` — bounded extension within this journey; preserve its regression checks.
- `agent.review` — bounded extension within this journey; preserve its regression checks.
- `agent.escalation` — bounded extension within this journey; preserve its regression checks.

### Regression checks

T03 adds constrained proposal submission and bounded inspection. Four public CLI
cases demonstrate stable canonical claims, legacy unassessed rendering, rejected
opposite structured outcomes and missing branches, forged authority/handles/cycles,
unknown predicates requiring gaps, stale work and unread required input. Separate
meaning acceptance is intentionally still pending T04.

T02 adds immutable `docs work prepare/read/expand` with work-local references,
recorded pagination, negative query membership and conservative scopes. Three
`docsys_t02_*` CLI tests cover source/note mutation, retained content, forged
references, cursor selection mismatch, explicit oversized records and sticky
untracked-read limitations. This implements the bounded-package extension;
proposal acceptance and isolated reviewer authority remain separate tasks.

- An author cannot self-approve or reuse a review for changed evidence.
- Missing inputs remain gaps rather than prompting invented answers.
- Author/reviewer/repair/fallback usage and failures are included in totals.

### Planning notes

Requires a constrained proposal API, captured reads, review-bound acceptance and portable agent roles. The acceptance mapping is in the [target](target-system.md#acceptance-contract-and-traceability).

### T04 implementation evidence

Isolated author/reviewer runs now publish coordinator-accepted content or a local
generation gap. The twelve `docsys_t04_*` CLI tests exercise separate review,
recorded expansion, bounded repair/fallback, exact review binding, local failure,
actual OS access denials, cancellation and conservative accounting. Accepted
content retains revision/read/review digests; protected note changes invalidate
it. The current adapter is macOS Seatbelt with trusted operator transport code.
Missing configuration/platform support stays explicit; real model results and
CI event/history behavior are not established by fake adapters. Existing graph
edges and extension-point regression obligations remain unchanged.

### T14 implementation evidence

T14 adds a configured stdio-to-HTTPS role gateway with exact role/model/invocation binding, bounded process execution, explicit credentials and unknown usage preservation. Durable execution/accounts ledgers prevent cache loss from resetting recorded spending; legacy ledgers migrate before dispatch or denial. The admitted core adapter still requires macOS Seatbelt. Existing S08 graph transitions are unchanged.

## S09 — Save a process or entity data-flow view

- **Status:** growth
- **Persona:** Documentation author
- **Entry:** S03 scenario or S07 entity page
- **Exit / next:** S04; S05 after source change
- **Read before:** [Approved connected objects](target-system.md#g6-connected-entities-contracts-and-processes), [model.rs](../../../crates/clew/src/documentation/model.rs), [check.rs](../../../crates/clew/src/documentation/check.rs), [CLI acceptance](../../../crates/clew/tests/managed_cli.rs)

### User story

As a documentation author, I request a process or entity data-flow section once so that it remains navigable and maintained after the conversation ends.

### Happy path

1. Choose the process/entity and the scope of the requested saved view.
2. Build bounded linked explanations with transforms, decisions and uncertain edges.
3. Publish the persistent view and later inspect its updates or historical snapshot.

### Alternative / error paths

- Preserve explicit uncertainty and the last valid publication when the relevant step cannot complete.
- Requires persistent semantic section definitions and typed evidence-backed entity flow edges.

### Extension points

- `process.saved-section` — bounded extension within this journey; preserve its regression checks.
- `entity.dataflow` — bounded extension within this journey; preserve its regression checks.
- `view.module` — bounded extension within this journey; preserve its regression checks.

### Regression checks

- Ad hoc exploration does not accidentally create a maintained section.
- Matching names alone do not prove entity transfer.
- A shared mapper change affects every dependent process/data-flow representation.

### Planning notes

Requires persistent semantic section definitions and typed evidence-backed entity flow edges. The acceptance mapping is in the [target](target-system.md#acceptance-contract-and-traceability).

### T10 implementation evidence

`process.definition`, `process.composition` and `process.saved-section` now use
explicit versioned definitions through `docs process list/show/put/inspect/prepare`.
Transient inspection saves no definition or publication. Existing scenario IDs
and HTTP/Kafka authority remain compatible. Linked current accepted child summaries
feed a separately reviewed overview; missing, cyclic, stale and unavailable scopes
remain gaps. Cyclic edges retain their transitive dependencies. Four CLI cases
cover identity/CAS, conditional source outcomes and unresolved HTTP transport,
negative interaction membership, sandboxed definition protection, child/parent
reviews and same-publication parent invalidation after a child version changes.
Final shared regression and desktop/mobile source-inspection evidence is in T10
of the implementation plan. Existing graph edges are unchanged.

### T11 implementation evidence

`view.module`, `view.definition`, `view.dataflow` and `view.dependencies` now use
the built-in versioned `entity-dataflow/1.0` contract. `docs view` exposes its
input, representation, edge, authority, dependency, validator and renderer
contracts and maintains explicit saved definitions. Domain identities stay
distinct from field/function/DTO/message/table representations. Name-only edges
remain UNKNOWN; cross-service edges require a declaration and retain
DECLARED_TRANSFER authority. These are static interpretations, not runtime traces.

Four CLI cases cover protected human annotations/layout and linked original notes,
separate author/reviewer acceptance, reuse of accepted process components, source
service attribution, mapper mutation across dependent views/processes/contracts,
independent entity reuse, interaction membership changes and concurrent edits.
The reader renders the same bound graph with source inspection and accessible
edge descriptions. Desktop/mobile checks and shared regressions are recorded in
T11 of the implementation plan. Existing S09 graph edges are unchanged.

## S10 — Attach and reassess protected human notes

- **Status:** growth
- **Persona:** Service team member
- **Entry:** S06 manual files or S07 entity/contract/process
- **Exit / next:** S04; S05 reassesses generated opinions
- **Read before:** [Approved ownership](target-system.md#g7-human-material-and-assessment-ownership), [store.rs](../../../crates/clew/src/documentation/store.rs), [bindings.rs](../../../crates/clew/src/documentation/bindings.rs), [CLI acceptance](../../../crates/clew/tests/managed_cli.rs)

### User story

As a service team member, I attach free-form context to a documented object so that readers retain human knowledge and can see evidence-based assessments without rewriting my words.

### Happy path

1. Add/import a human note and associate its stable identity with a section or entity.
2. Display the original alongside an explicitly separate optional agent assessment.
3. After related changes, reassess the generated opinion while preserving human text/tags/metadata.

### Alternative / error paths

- Preserve explicit uncertainty and the last valid publication when the relevant step cannot complete.
- Requires import, stable associations and independent assessment ownership.

### Extension points

- `notes.import` — bounded extension within this journey; preserve its regression checks.
- `notes.assessment` — bounded extension within this journey; preserve its regression checks.

### Regression checks

- Concurrent human edits are never overwritten by older generated work.
- Historical facts are checked in their applicable period.
- Note assessments do not imply strict dependency completeness for arbitrary prose.

### Planning notes

Requires import, stable associations and independent assessment ownership. The acceptance mapping is in the [target](target-system.md#acceptance-contract-and-traceability).

### T09 implementation

`notes.import` and `notes.assessment` now have public CLI paths and separate
human/canonical ownership. Four CLI cases cover frontmatter/tags/CRLF/Unicode,
explicit service/section/entity/process associations, rename and removal, current
contradictions versus unknown history, concurrent edits, rejected embedded
instructions, blocked original/catalogue writes and stale retained assessments.
Historical conclusions require the exact captured revision as their period;
calendar claims without such evidence remain unknown. Navigation/search and
Markdown/JSON preserve original snapshots and distinguish assessment meaning
review from the note's declared classification. Deterministic fixture review
establishes the pipeline contract, not real model assessment quality. S06/S10
edges are unchanged.

## S11 — Configure optional evidence modules

- **Status:** growth
- **Persona:** CI operator
- **Entry:** S01 service registration/settings
- **Exit / next:** S07 or S08; S05 on provider change
- **Read before:** [Approved modules](target-system.md#g2-source-first-evidence-with-optional-modules), [analysis.rs](../../../crates/clew/src/documentation/analysis.rs), [syntax.rs](../../../crates/clew/src/documentation/syntax.rs), [CLI acceptance](../../../crates/clew/tests/managed_cli.rs)

### User story

As a CI operator, I enable compatible evidence modules per repository so that available tools improve evidence without becoming mandatory for all services.

### Happy path

1. Inspect versioned module capabilities and choose allowed providers/rules.
2. Capture source evidence and request optional compatible semantic/framework/contract facts.
3. Retain unavailable or ambiguous results and invalidate claims on relevant provider changes.

### Alternative / error paths

- Preserve explicit uncertainty and the last valid publication when the relevant step cannot complete.
- Built-in metadata and adapter registry exist; documentation does not expose a generic module contract.

### Extension points

- `provider.module-contract` — bounded extension within this journey; preserve its regression checks.
- `provider.authority` — bounded extension within this journey; preserve its regression checks.
- `evidence.portability` — bounded extension within this journey; preserve its regression checks.

### Regression checks

- Provider loss leaves baseline text and identities intact.
- Wrong revision or producer schema cannot upgrade authority.
- Deferred protocols have extension seams without pretending their new modules exist.

### Planning notes

Built-in metadata and adapter registry exist; documentation does not expose a generic module contract. The acceptance mapping is in the [target](target-system.md#acceptance-contract-and-traceability).

### T05 implementation evidence

`docs modules list/show` reports built-in producer contracts, implementation
identity, availability and project applicability. Versioned per-service settings
select javac or K2 through existing admission and cleanup. Three public CLI tests
cover missing tools and invalid/ambiguous selection; actual Kotlin 1.9.25 and
Maven/javac recovery checks pass. Module/rule changes affect freshness without
source-byte changes. Python syntax and Kafka checks remain regression evidence.
Source annotation interpretation and further framework compatibility are T06.

### T06 implementation evidence

Java/Kotlin source annotation facts feed the shared Rust Spring interpreter.
Qualified imports, Kotlin aliases and literal mappings derive source declarations;
unrelated/ambiguous annotations and unresolved aliases/values/hierarchy remain
explicit gaps. Three CLI tests and typed-contract/shared-rule tests pass alongside
existing Spring compiler-metadata and documentation regressions. The Boot 3.3.0
fixture verifies a bounded MockMvc route; see its exact [compatibility matrix](../../../fixtures/documentation-system/spring/README.md).
The existing compiler schema and worker bridges are unchanged.

### T07 implementation evidence

The independent declared OpenAPI module captures explicitly registered committed
files outside language roots. Three no-tool CLI cases preserve nested contracts,
local/cross-file references and unmatched operations; reference failures and
unsupported versions remain visible. Tested dialects are 3.0.0 and 3.0.3. A unique
source route match remains separate from contract authority and runtime behavior.
Contract-only mutations invalidate service and process fragment dependencies;
a shared expansion budget bounds repeated references. Existing documentation
unit regressions pass. See the [fixture scope](../../../fixtures/documentation-system/openapi/README.md).

### T12 implementation evidence

Portable packages retain producer/rule schemas and digests, source bindings,
coverage and semantic-provider outcomes. The consumer checks rule compatibility
without requiring its own compiler installation. A missing worker or failed
producer remains an explicit captured outcome; importing it grants no stronger
source/runtime authority. Source-free support reports include available module
versions, stable reason codes and safe worker metadata; T12 unit tests establish
private-envelope exclusion and bounded multipart encoding. Successful Kotlin 1.9
compiler qualification remains a separate runtime check. Existing S11 edges remain.

### T14 implementation evidence

T14 portable capture jobs verify the observed exact revision and transfer the complete authorized package. Configuration selects commands, roots and audience; source/package text cannot select executables. Source-launcher execution exposed and fixed a missing embedded proposal-schema input in the runtime registry, now covered by an embedding-closure regression. Existing S11 graph transitions are unchanged.

## S12 — Update central documentation through CI and inspect history

- **Status:** growth
- **Persona:** CI operator
- **Entry:** Accepted source event or explicit revision reconciliation
- **Exit / next:** S05/S08 then S04; S06 recovery
- **Read before:** [Approved integration](target-system.md#g9-portable-integration-and-production-qualification), [check.rs](../../../crates/clew/src/documentation/check.rs), [store.rs](../../../crates/clew/src/documentation/store.rs), [render.rs](../../../crates/clew/src/documentation/render.rs), [CLI acceptance](../../../crates/clew/tests/managed_cli.rs)

### User story

As a CI operator, I process accepted changes from multiple teams so that the central documentation stays honest, available and recoverable without a platform-specific core.

### Happy path

1. Resolve accepted source refs to exact revisions and capture only required service evidence.
2. Validate/import portable results, mark affected sections and schedule bounded updates.
3. Publish a consistent snapshot with per-section revisions; inspect earlier snapshots or recover after failure.

### Alternative / error paths

- Preserve explicit uncertainty and the last valid publication when the relevant step cannot complete.
- Requires selective evidence interchange, event ordering, status-only publication, history manifests and integration recipes.

### Extension points

- `ci.event-contract` — bounded extension within this journey; preserve its regression checks.
- `ci.publication` — bounded extension within this journey; preserve its regression checks.
- `history.snapshot` — bounded extension within this journey; preserve its regression checks.
- `runtime.recovery` — bounded extension within this journey; preserve its regression checks.

### Regression checks

- Duplicates and old results cannot regress accepted targets or overwrite notes.
- One failed service or exhausted agent budget does not block unrelated content.
- Cache loss does not lose retained publications/evidence and no live shared private session state is required.

### Planning notes

Requires selective evidence interchange, event ordering, status-only publication, history manifests and integration recipes. The acceptance mapping is in the [target](target-system.md#acceptance-contract-and-traceability).

### T04 implementation evidence

Isolated author/reviewer runs now publish coordinator-accepted content or a local
generation gap. The twelve `docsys_t04_*` CLI tests exercise separate review,
recorded expansion, bounded repair/fallback, exact review binding, local failure,
actual OS access denials, cancellation and conservative accounting. Accepted
content retains revision/read/review digests; protected note changes invalidate
it. The current adapter is macOS Seatbelt with trusted operator transport code.
Missing configuration/platform support stays explicit; real model results and
CI event/history behavior are not established by fake adapters. Existing graph
edges and extension-point regression obligations remain unchanged.

### T12 implementation evidence

Central jobs can import a captured service at an exact coordinator-selected
revision with no application checkout, Git command or compiler. Expectations bind
origin/project, configuration, trusted manifest digest and increasing sequence;
idempotent repeats are accepted and older/mismatched results leave retained
selection untouched. A missing new result becomes a local gap. Captured failures
retain their original stable code and portable report provenance. The five CLI
cases include separate author/reviewer acceptance and denied writes to trust
configuration. Event ingestion, retained history and CI recipes remain T13/T14;
existing S12 transitions are unchanged.

### T13 implementation evidence

Local update configure/enqueue/reconcile/run/status and history list/show/compare commands now implement exact targets, idempotency, out-of-order rejection and immutable history. Four T13 integration cases exercise batches, failed/missing service evidence, tag moves, cache loss, interrupted publication and finite queue work. Actual GitLab execution and model quality remain later qualification tasks. Existing S12 graph transitions remain unchanged.

### T14 implementation evidence

T14 implements versioned local/CI jobs, exact event forwarding, retained idempotency results, conservative acceptance before evidence transfer, coordinator locks, configured publication and a GitLab resource-group recipe. The configured qualification entry point triggers pipelines and compares returned artifacts with local outcomes. Thirteen fake contract tests and an actual local source-launcher capture/import/status flow passed; this is not actual GitLab qualification. Existing S12 graph transitions are unchanged.
