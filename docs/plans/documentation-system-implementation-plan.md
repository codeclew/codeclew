# Documentation system implementation plan

Status: approved for implementation by the user on 2026-09-12.
Product target: approved on 2026-09-12. Execution status is recorded per task; the original planning
package did not execute implementation tasks. The current runtime remains the base revision recorded below.

## Sources and scope

- **Source of truth:** [Approved target](../product/documentation-system/target-system.md), [decisions](../product/documentation-system/decisions.md), [scenario cards](../product/documentation-system/scenario-cards.md).
- **Baseline:** [Current scenario baseline](../product/documentation-system/current-scenario-baseline.md), [pre-scan](../product/documentation-system/increments/durable-documentation-pre-scan.md), [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Architecture authority:** [Repository agreements](../../AGENTS.md); existing documentation, adapter, fact and Spring modules. Base revision `a2dcefe2296e65f8f98e1cb205b2bbe3589e7989` includes the development PR #8 work; rebase/migrate deliberately if that base changes.
- **Review:** [Independent package verdict](../product/documentation-system/validation/verdict.md). A plan review is not runtime or model qualification.
- **Execution model:** one bounded implementation task at a time, using the user's configured development agent. Production author/reviewer/fallback models are separately configurable; no fixed coding-worker model is required by this plan.
- **Execution authorization:** target approval authorizes planning. The user explicitly approved implementation on 2026-09-12. Credential installation, model budgets and publication destinations remain installation inputs; approval does not invent them.

## Approved delivery extension — 2026-09-12

The same approval adds a real three-service documentation repository and remote
troubleshooting for a work MacBook using Java 17 / Kotlin 1.9 projects. Customer
repository locations and generated source material remain in the separate private
delivery repository, not the public Codeclew source tree.

- T12 must expose a supported, versioned diagnostic/index export and offline
  inspection route. The default report records tool/runtime/project versions,
  capabilities, failed stage, stable reason codes, sanitized diagnostics, input
  digests and exact retry instructions without credentials or absolute home paths.
  Index/evidence content is explicit opt-in and its contents are inspectable before
  sharing. Imported support material is read-only data and never executes repository
  commands or agent instructions.
- T15 must reproduce representative Java 17 / Kotlin 1.9 admission, capture and
  documentation failures in a separate support environment with source checkouts
  absent, using only the report and optional exported index. A report must separate
  a repairable tool defect from missing evidence; universal diagnosis is not promised.
- T17 includes the documented collect/inspect/share workflow and a MacBook smoke
  test. Source language/target compatibility is distinct from the JDK required to
  run Codeclew's compiler workers; missing runtime requirements must be diagnosed
  before a worker is launched.
- After the core tasks, deliver standard service/entity/contract documentation and
  supported cross-service views for the three requested repositories in the new
  documentation repository. Preserve declared versus inferred relationships, exact
  source revisions, gaps, and human ownership. Qualify updates and a source-free
  support round trip on this installation, then record measured results.

## Task conventions

Read Status / Goal / Sources / Depends on / Read first / Modify / Product artifacts /
Steps / Verify / DoD for the selected task. Paths in Modify are repository-relative;
`documentation/` means `crates/clew/src/documentation/`. Existing versus new files are
identified explicitly. A task creates any test/script/API that its Verify command
requires. Proposed `docs` commands below are future interfaces, not claims that they
currently run. Runtime examples use supported `./clew`, never direct capsule binaries.

Use the pinned Rust toolchain, Python 3.11+, JDK 21 and repository wrappers. Preserve
unrelated work and use native source development under AGENTS.md. Do not copy/edit
private CODECLEW_HOME session objects to create a passing interchange test. Keep
historical model IDs and measured evidence unchanged.

Begin with T00's reader-visible stale-status outcome; there is no separate foundation
phase. Shared primitives arrive with their first named consumer. Execute dependencies
before a task, keep commits bounded, and split implementation work operationally if
needed without changing approved acceptance. Do not mark a task done merely because
its test filter matched zero tests. New `docsys_tXX_*` cases are public-CLI behaviors,
not schema-literal mirrors. Run fmt before relevant Rust tests.

When a task changes the skill, keep `skills/codeclew`, `.agents/skills/codeclew`, and
`.claude/skills/codeclew` copies identical, update the embedded reference inventory
when needed, and use the existing installer-derived digest contract. Do not bypass
packaging failures or add a second approval layer to ordinary implementation.

Every task updates the listed scenario cards' implementation notes/regression evidence,
the baseline's observed delta notes and the impact task mapping. Graph topology changes
only when the task changes transitions; otherwise record that it remains accurate.
The approved target and acceptance contract are not rewritten to fit implementation.

The full development gate runs at T15 after material runtime work. Repeat expensive
checks only after a material code delta or a required final gate. T16's actual-model
qualification is a separately configured execution, not a requirement to run paid
models during ordinary deterministic CI. Missing deployment access blocks only the
corresponding qualification claim and keeps that task visibly incomplete.

## Shared technical contracts proposed for approval

**Identity and ownership.** A stable section/entity/claim identity differs from its
immutable version and its source occurrence. Claims and derived views name their
support/influence inputs; public state is product-owned. Human originals, associations,
proposed agent content and assessments have separate ownership. No regex patching of
generated Markdown is used to preserve human contributions.

**State axes.** Persist target and content revision sets separately. Freshness values
include current/stale/unverified; verification includes unassessed/accepted/contradicted/
limited; coverage and authority remain explicit. Exact enum spelling is versioned by
its first consumer. A missing optional capability affects only claims needing it;
no-K2 source-supported claims can be current. Conversely fresh syntax does not verify
an unresolved dispatch claim. A proposed review cannot waive known evidence failures.

**Best effort.** A new immutable publication may reuse last accepted explanation
versions with new status metadata. Each explanation's views remain coherent. Partial
sections are allowed; corrupted artifacts are not admitted. Local conflicts retry or
become visible update failures without overwriting the last valid publication.

**Influence closure.** Start with current conservative per-service source watches and
extend them to registered file/query/module/section inputs. Record negative membership
and every supplied expansion. Document-object dependencies propagate transitively;
cycles use explicit bounded fixed-point/SCC traversal rather than dropping edges.
Uncaptured relevant reads prevent complete-coverage claims. Unknown broader influence
requires wider invalidation, not a guessed unaffected result.

**Agent control.** External role adapters receive immutable work inputs and limited
capabilities. Codeclew owns job state, proposal/review digests and acceptance. Configured
budgets cover orchestration, author, reviewer, repairs and fallback. Before dispatch,
reserve the capped next call and the required remaining review/repair allowance;
stop below the configured ceiling. Unknown usage retains its worst-case reservation.
No model/provider ID is a trust boundary or a suitability certificate. Separate meaning
review is evidence, not formal proof; source-grounded qualification measures its limits.

**Versioned interchange.** Public evidence packages contain bounded immutable parts
and a manifest; they never expose a live private session as a portability contract.
Central jobs validate origin, producer/schema compatibility, exact commits and digests.
Source caches may be disposable; retained publication manifests pin the evidence
required to inspect their history. Storage cleanup must preserve that relationship or
explicitly expire the historical snapshot. Integration configuration sets access and
retention; source snippets in reader artifacts inherit the required source audience.

**Module reuse.** `clew-facts`, `clew-framework-spring`, analysis module metadata and
adapter lifecycle are existing seams. Extend them rather than building parallel Spring
or compiler registries. Normalize source annotations with weaker authority. Initial
OpenAPI scope reuses supported current behavior and explicitly reports unimplemented
versions/features; the compatibility matrix is part of qualification.

## Requirement coverage

| Acceptance | Tasks | Result |
| --- | --- | --- |
| AC01 | T00, T13 | Status publication independent of model success |
| AC02 | T00, T01, T13 | Section-local failure with coherent old/new publication |
| AC03 | T02 | Bounded evidence and tracked expansions/influence |
| AC04 | T03 | Constrained authoring and deterministic checks |
| AC05-AC06 | T04, T16 | Separate review and measured bounded escalation |
| AC07 | T05 | Source baseline and optional compatible modules |
| AC08 | T06 | Shared Rust Spring interpretation |
| AC09 | T07 | Independent declared OpenAPI module |
| AC10-AC11 | T08 | Standard service and stable domain sections |
| AC12 | T09 | Protected human notes and separate assessments |
| AC13 | T10 | Saved maintained processes |
| AC14 | T11 | Entity data-flow view and dependency behavior |
| AC15 | T12 | Portable per-service evidence |
| AC16 | T13 | Exact revision history and concurrency |
| AC17 | T14, T16 | GitLab and neutral external-agent integration |
| AC18 | T00-T03, T08-T11, T13, T15 | Conditional complete influence and mutation qualification |
| AC19 | T13-T15 | Operational fault isolation and recovery |
| AC20 | T16, T17 | Actual model-workflow qualification and honest user guidance |

## Existing evidence to preserve

The baseline already has no-build Python/Java/Kotlin source extraction, conservative
source watches, exact snippets, optional narrowly mapped compiler observations,
source change dossiers, source-bound Narrative views and atomic immutable output.
Shared Rust Spring interpretation is also implemented; T06 adds source-normalized
inputs and compatible authority, not a new language-specific interpreter. Existing
CLI acceptance and the [prior report](../product/validation/source-documentation-qualification.md)
remain regression evidence. No new production result is inferred from that report.

## Qualification boundaries

The plan does not implement universal source influence, runtime execution verification,
a generic optimal context planner, deployment discovery, a new browser editor, model
training, or new Kafka/Avro/Protobuf/SOAP modules. Preserve existing supported behavior.
No fixed latency, cost-saving percentage or routine-model success rate is promised.

Threat checks address this workflow's actual boundaries: source access, prompt data,
external adapter permissions, artifact validation and publication ownership. A formal
security certification or independent penetration test is not claimed. Infrastructure
TLS/identity-provider operation remains the installation's responsibility. Historical
artifact retention and restore are in scope; uncontrolled indefinite retention is not.

T15 supplies deterministic runtime/capacity/recovery evidence. T16 supplies actual-model
quality/economics and actual GitLab-triggered integration evidence. Any unavailable prerequisite remains a visible qualification
gap. A passing plan reviewer, mock adapter or schema checker cannot substitute for either.

## Tasks

## T00. Publish stale status without generating new prose

- **Status:** - [x] Implemented 2026-09-12; two public CLI acceptance tests and 24 documentation unit tests pass (two native-worker tests intentionally not run for this Rust-only slice).
- **Goal:** After a source change, readers immediately see which retained operation and all its views require review, even with no agent configured.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC01, AC02; [scenario cards](../product/documentation-system/scenario-cards.md): S04, S05; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** None
- **Read first:**
  - [documentation/bindings.rs](../../crates/clew/src/documentation/bindings.rs), lines 19-45,243-321.
  - [documentation/render.rs](../../crates/clew/src/documentation/render.rs), lines 903-988,1125-1340.
  - [documentation/check.rs](../../crates/clew/src/documentation/check.rs), lines 58-103.
  - [documentation/model.rs](../../crates/clew/src/documentation/model.rs), lines 329-365.
  - [documentation/cli.rs](../../crates/clew/src/documentation/cli.rs), lines 16-60,185-295.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - Existing documentation/model.rs, bindings.rs, check.rs, render.rs, cli.rs: introduce versioned per-section state while preserving old record readers.
  - Existing crates/clew/assets/documentation/app.js, template.html, style.css: common status/revision/gap presentation.
  - New crates/clew/tests/documentation_system.rs: public-CLI acceptance helpers and docsys_t00_* tests.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S04, S05 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Add per-section content revision set, target revision set, freshness, verification, coverage and missing-information fields. Old bundles have explicit legacy/unassessed verification rather than invented review.
  2. Implement proposed `./clew docs refresh --root <docs> --status-only`: resolve/capture accepted inputs, compare current bindings, and create a new immutable status publication using the last accepted explanation. Do not edit old HTML or silently rebind its semantic content to new code.
  3. Propagate a changed existing claim status to its paragraph, contract row and overview/detail views. For unavailable source, mark potentially affected retained content unverified with the failed input reason. No model invocation is needed.
  4. Test a helper mutation with all model executables absent: the offline reader shows the old text/revision and new stale status; unaffected service remains current; old bundle hashes and human files remain unchanged.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t00_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t00_ -- --test-threads=1
  ```

- **DoD:**
  - A meaningful first slice is usable through the public launcher and offline page.
  - Status-only refresh does not require an author/reviewer response or claim new prose correctness.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T01. Publish valid section updates despite unrelated capture failures

- **Status:** - [x] Implemented 2026-09-12; five T00/T01 public CLI tests, the existing multi-language source and declared-scenario recovery checks, and 24 documentation unit tests pass. Native worker qualification remains in T05/T06.
- **Goal:** Allow one service or operation to update while another is unavailable, preserving coherent explanation versions and explicit failure states.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC02, AC18; [scenario cards](../product/documentation-system/scenario-cards.md): S01, S04, S05; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T00
- **Read first:**
  - [documentation/analysis.rs](../../crates/clew/src/documentation/analysis.rs), lines 127-211.
  - [documentation/check.rs](../../crates/clew/src/documentation/check.rs), lines 69-175.
  - [documentation/render.rs](../../crates/clew/src/documentation/render.rs), lines 1125-1340.
  - [documentation/store.rs](../../crates/clew/src/documentation/store.rs), lines 207-233,374-399.
  - [documentation/bindings.rs](../../crates/clew/src/documentation/bindings.rs), lines 141-243.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - Existing documentation/check.rs, analysis.rs, render.rs, bindings.rs, store.rs, cli.rs: section-local capture/validation/publication outcomes.
  - Existing/new documentation_system.rs tests: docsys_t01_*.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S01, S04, S05 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Separate current target resolution/capture outcomes from retained accepted section records. Add proposed `docs check --service <id>` selection without claiming that unselected services were freshly checked.
  2. Replace the global unresolved-capture publication refusal with local outcome records. Incoming invalid/stale/unreviewed proposals remain local gaps; valid unrelated accepted sections can publish. Preserve the strict `--require-complete` opt-in command as a diagnostic/legacy check, not the default automatic publication path.
  3. Before pointer change, compare definitions, human records and prior publication identity; reject or retry only the conflicting attempted update. Never overwrite a newer snapshot or existing immutable files.
  4. Exercise one missing repository, one failed semantic provider, a stale incoming section, output corruption and two concurrent local refreshes. Verify old explanation/view coherence, a visible next action, and successful unrelated content.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t01_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t01_ -- --test-threads=1
  ```

- **DoD:**
  - Best effort changes content completeness policy without weakening publication integrity.
  - Failure does not erase retained evidence or falsely clear staleness.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T02. Prepare bounded evidence work and capture its influence

- **Status:** - [x] Implemented; three CLI regression cases, 24 documentation unit
  tests, six skill-package tests and embedded package-digest validation pass.
  Two native JVM tests remain outside this Rust-only task.
- **Goal:** A routine agent receives one complete-enough work unit and can request focused expansion without manually managing evidence identity or hiding reads.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC03, AC18; [scenario cards](../product/documentation-system/scenario-cards.md): S02, S05, S08; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T01
- **Read first:**
  - [documentation/cli.rs](../../crates/clew/src/documentation/cli.rs), lines 297-733.
  - [documentation/bindings.rs](../../crates/clew/src/documentation/bindings.rs), lines 45-141.
  - [documentation/syntax.rs](../../crates/clew/src/documentation/syntax.rs), lines 29-167.
  - [documentation/analysis.rs](../../crates/clew/src/documentation/analysis.rs), lines 660-705.
  - [documentation/model.rs](../../crates/clew/src/documentation/model.rs), lines 162-230.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - New documentation/work.rs; existing mod.rs, cli.rs, bindings.rs, model.rs: prepare/read/expand contract and read/query records.
  - New schemas/documentation/work.schema.json and fixtures/documentation-system/work/: closed work-request/result fixtures.
  - Existing skills/codeclew/references/service-documentation.md and its two packaged copies: focused preparation/expansion guidance; operations.rs digest expectation if affected.
  - Existing documentation_system.rs: docsys_t02_*.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S02, S05, S08 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Implement proposed `docs work prepare --root <docs> --subject <stable-subject> --input <request>` and `docs work read/expand --root <docs> --work <id> --input <selection>`. Reuse existing context selectors and bounded pagination; no general optimal-context planner.
  2. Work binds audience, requested scope, exact revisions, provider/rule versions, evidence authority, old content, notes, required coverage and expansion references. Include owner/type/helper/configuration evidence when known necessary; otherwise emit an explicit missing obligation.
  3. Record every supplied page and expansion plus query filters, membership, negative results and enclosing conservative source scopes. External files must be registered as immutable inputs or cause incomplete influence coverage. Importing a file into a prompt is not tracked merely because it is cited later.
  4. Use bounded records and explicit omitted-item identities. Escalate missing package evidence via read/expand, not by fabricating context. Test over-budget items, empty queries gaining results, undeclared reads, changed work revisions and cursor selection mismatch.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t02_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t02_ -- --test-threads=1
  python3 -I -S scripts/test_agent_skill.py
  ```

- **DoD:**
  - A package consumer can understand exact evidence without opaque graph construction.
  - No work can declare complete influence coverage while known required inputs are untracked.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T03. Accept constrained content proposals with deterministic checks

- **Status:** - [x] Implemented; four T03 CLI cases and all twelve T00–T03
  regression cases pass. Meaning acceptance remains a separate T04 obligation.
- **Goal:** The author submits readable bounded content; Codeclew resolves its allowed evidence handles, builds canonical records and validates what can be checked deterministically.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC04, AC18; [scenario cards](../product/documentation-system/scenario-cards.md): S02, S08; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T02
- **Read first:**
  - [documentation/model.rs](../../crates/clew/src/documentation/model.rs), lines 232-365.
  - [documentation/render.rs](../../crates/clew/src/documentation/render.rs), lines 44-494,499-735.
  - [documentation/bindings.rs](../../crates/clew/src/documentation/bindings.rs), lines 45-141.
  - [documentation/cli.rs](../../crates/clew/src/documentation/cli.rs), lines 185-295.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - New documentation/proposals.rs; existing work.rs, model.rs, render.rs, bindings.rs, cli.rs: proposal validation and materialization.
  - New schemas/documentation/proposal.schema.json, fixtures/documentation-system/proposals/.
  - Existing documentation_system.rs: docsys_t03_*.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S02, S08 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Implement proposed `docs proposal submit --root <docs> --work <id> --input <proposal>` and `docs proposal show`. Authorable fields are claims/text/contract descriptions/diagram meaning/uncertainties and allowed work-local evidence handles; ownership and verification fields are rejected.
  2. Have Codeclew materialize stable canonical claim IDs, paragraph/event references, dependency closure and bounded layouts. Keep content proposals separate from accepted section versions. Preserve the existing raw Narrative import path as legacy/unassessed input, never an acceptance bypass.
  3. Validate evidence integrity, schema, required coverage and supported typed claims (for example literal route/field/predicate assertions supported by the selected provider). Unknown predicates stay unknown. Do not treat a syntactically valid paragraph as proven meaning.
  4. Test invented handles, opposite guard outcomes, hidden missing branches, false compiler/runtime authority, cyclic references, unsupported predicates and valid legacy Narrative reading.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t03_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t03_ -- --test-threads=1
  ```

- **DoD:**
  - Authors cannot self-declare acceptance or manufacture evidence authority.
  - Deterministic diagnostics name the failed claim/obligation and repairable evidence.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T04. Run separate review, bounded repair and configurable fallback

- **Status:** - [x]
- **Goal:** A routine agent and separate reviewer can complete or fail a work item automatically, with bounded repair/fallback and section-local publication behavior.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC05, AC06; [scenario cards](../product/documentation-system/scenario-cards.md): S08, S12; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T03
- **Read first:**
  - [documentation/cli.rs](../../crates/clew/src/documentation/cli.rs), lines 16-60,185-295.
  - [documentation/store.rs](../../crates/clew/src/documentation/store.rs), lines 207-233,374-399.
  - [documentation/render.rs](../../crates/clew/src/documentation/render.rs), lines 1125-1340.
  - [documentation/model.rs](../../crates/clew/src/documentation/model.rs), lines 329-365.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - New documentation/agent_jobs.rs and review.rs; existing work.rs, proposals.rs, cli.rs, render.rs.
  - New schemas/documentation/agent-job.schema.json, review.schema.json; new fixtures/documentation-system/agents/ deterministic fake adapters.
  - Existing service-documentation skill reference and packaged copies; new references/documentation-authoring.md and documentation-review.md only if required to keep the primary guide bounded; update embedded packaging inventory/digest and scripts/test_agent_skill.py when adding references.
  - Existing documentation_system.rs: docsys_t04_*.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S08, S12 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Implement proposed `docs work run --root <docs> --work <id> --config <execution-config>` with author, reviewer and optional fallback roles over a versioned external stdin/stdout or job-result interface. Vendor credentials/model SDKs remain outside core records. Missing configuration produces a generation gap, not a guessed default billable model call.
  2. Require an operator-configured isolated adapter at this first consumer: immutable read-only work inputs, registered expansion access, role-specific result destinations, and no direct write access to source, human files, coordinator state or another role's results. Deny unregistered source reads; isolation applies to filesystem and tool capabilities, not just JSON fields. Core supplies a capability contract and validates admission; it does not assume a normal child process is isolated or build a new general-purpose sandbox. An adapter unable to enforce this contract produces a local generation gap. Reviewer input includes the proposal, source evidence and obligations. Bind the review to work/proposal/evidence digests and a separately dispatched role. Only the coordinator accepts review results; author output paths/capabilities cannot write review or acceptance state. Treat source instructions as untrusted content, not executable policy.
  3. Use state transitions prepared/authored/checked/reviewed/accepted or needs-evidence/needs-repair/exhausted. Acceptance requires required machine checks and review; limitations are explicit. Execution configuration supplies finite total ceilings, per-call input/output/time caps, maximum attempts and a stop-loss below the ceiling. Atomically reserve each capped call plus the remaining mandatory review and at least one configured repair allowance before dispatch, including orchestration overhead and fallback when selected. A work item that cannot reserve its required path stops with a local gap; no universal numeric limits are invented.
  4. Escalate on failed meaning checks or persistent contradictions after configured repair; missing source/provider evidence requests expansion or creates a gap. Cancellation, unreported usage or a failed adapter keeps its maximum reserved cost/usage until trusted final usage reconciles it; unavailable fields remain unavailable in reports. Deny further calls when a finite upper bound cannot be established, and cancel calls at enforced time/output caps. Never treat absent accounting as zero.
  5. Use fake agents to test self-approval, stale review replay, source prompt injection, malformed/oversized output, reviewer contradiction, missing context, bounded escalation, cancellation and exhaustion. Include access-denial tests through the supported isolated test adapter (attempt coordinator/source/human writes, cross-role result writes and unregistered reads), not only forged-result tests. Test reservation contention, an author consuming its full cap, missing final usage and a denied unaffordable repair. Demonstrate accepted content or a local gap without any real paid model.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t04_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t04_ -- --test-threads=1
  python3 -I -S scripts/test_agent_skill.py
  ```

- **DoD:**
  - Only the coordinator assigns acceptance; separate role execution is not advertised as uncorrelated model errors.
  - Every attempt, limitation and result is traceable; human notes and application source remain outside agent write permissions.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

**Observed verification (2026-09-12):** All twelve T04 CLI cases passed on macOS
with the real Seatbelt adapter and deterministic fixture drivers. A combined
T00-T04 run passed 23/24 cases and exposed a cancellation/write-lock race; after
the isolated cancellation-signal fix, the failed case passed. The twelve T00-T03
cases remain passing in that combined run. No paid model or real CI qualification
is claimed. Unsupported hosts produce an explicit isolation gap.

## T05. Expose explicit documentation modules using existing producers

- **Status:** - [x]
- **Goal:** A service can select available syntax, K2 and javac evidence modules while preserving baseline availability and producer provenance.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC07; [scenario cards](../product/documentation-system/scenario-cards.md): S01, S11; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T02
- **Read first:**
  - [documentation/analysis.rs](../../crates/clew/src/documentation/analysis.rs), lines 127-258.
  - [documentation/syntax.rs](../../crates/clew/src/documentation/syntax.rs), lines 544-623.
  - [documentation/store.rs](../../crates/clew/src/documentation/store.rs), lines 404-465.
  - [documentation/model.rs](../../crates/clew/src/documentation/model.rs), lines 19-54.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - Existing analysis_modules.rs, adapter_v2.rs only at documentation-facing seams; documentation/analysis.rs, model.rs, store.rs, syntax.rs, cli.rs.
  - New documentation/modules.rs plus schemas/documentation/module-capability.schema.json.
  - Existing documentation_system.rs: docsys_t05_*; module/adapter unit tests as affected.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S01, S11 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Read and reuse crates/clew/src/analysis_modules.rs, adapter_v2.rs and crates/clew-facts/src/lib.rs before changing provider selection. Implement proposed `docs modules list/show` and versioned per-service module configuration. Do not create a second compiler worker lifecycle.
  2. Map existing source profile and optional semantic settings into compatible defaults. Expose supported input/output schemas, project/analyzer versions, applicability, required authority and implementation digest; do not auto-execute tools configured by untrusted source files.
  3. Select K2/javac only when applicable and explicitly enabled. Keep admission/cleanup through existing supported operations. Reject wrong-revision/ambiguous source mappings; provider loss changes only relevant evidence and conservative dependencies.
  4. Prove source-first behavior with missing tools and actual admitted javac/K2 recovery, including analyzer compatibility boundaries and provider/rule changes without source-byte changes. Preserve existing Python source and existing Kafka documentation regressions.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t05_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t05_ -- --test-threads=1
  cargo test --locked -p clew --lib adapter_v2::tests:: -- --test-threads=1
  cargo test --locked -p clew --lib documentation::kotlin::tests::native_kotlin_19_maven_documentation -- --exact --ignored --test-threads=1
  cargo test --locked -p clew --test managed_cli durable_source_documentation_java_enrichment_recovers_on_the_same_source_roots -- --exact --ignored --test-threads=1
  ```

- **DoD:**
  - No module name confers stronger authority than its supplied validated evidence.
  - First-party producer reuse and extension interfaces are available without implementing deferred protocols.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

**Observed verification (2026-09-12):** Three T05 CLI tests pass without build
tools. The actual Kotlin 1.9.25 compiler fixture (27.77 seconds) and admitted
Maven/javac source-enrichment recovery (58.87 seconds) pass with JDK 21 configured
for the worker. Adapter tests pass 37 cases with five separate qualification
cases ignored; the two selected JVM qualification cases above were executed
explicitly. Documentation unit regressions pass, including a new module-rule
invalidation case, and accepted isolated publication/note invalidation still
passes. Kotlin 1.9 analysis retains its explicit language-upgrade limitation.

## T06. Feed shared Spring rules from normalized source annotations

- **Status:** - [ ]
- **Goal:** Kotlin and Java service boundaries can use the same Rust Spring interpretation even when only source evidence is available.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC08; [scenario cards](../product/documentation-system/scenario-cards.md): S07, S11; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T05
- **Read first:**
  - [documentation/syntax.rs](../../crates/clew/src/documentation/syntax.rs), lines 335-513.
  - [documentation/analysis.rs](../../crates/clew/src/documentation/analysis.rs), lines 459-585.
  - [documentation/model.rs](../../crates/clew/src/documentation/model.rs), lines 190-230.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - Existing crates/clew-facts/src/lib.rs, crates/clew-framework-spring/src/lib.rs, crates/clew/src/spring_entrypoints.rs and documentation/syntax.rs, modules.rs, analysis.rs.
  - Existing producer fixtures/worker bridges only if the normalized schema must change; preserve versioned K2/javac compatibility.
  - New fixtures/documentation-system/spring/ Kotlin/Java examples; documentation_system.rs: docsys_t06_*.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S07, S11 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Reuse jvm-annotation-facts and the existing pure Rust clew_framework_spring interpreter. Add a versioned compatible source-annotation representation/authority without allowing compiler-exact fields to be synthesized from spelling.
  2. Extract normalized annotation names/arguments/owners and available import/type context in language adapters. Framework rules consume those facts in Rust, not duplicated Kotlin/Java Spring logic.
  3. Derive supported literal request mappings and annotation declarations with explicit authority. Composed annotations, aliases, inheritance, unresolved constants and dynamic configuration either have qualified evidence or named gaps; runtime registration is never inferred as observed.
  4. Test equivalent Kotlin/Java endpoints, absent K2/javac, same-spelling unrelated annotation, alias/default cases, inherited declarations and unsupported combinations. Publish a Boot 3.3+ compatibility matrix for tested versions; future versions remain explicitly unqualified.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t06_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t06_ -- --test-threads=1
  cargo test --locked -p clew-facts
  cargo test --locked -p clew-framework-spring
  cargo test --locked -p clew --lib spring_entrypoints::tests:: -- --test-threads=1
  ```

  If normalized producer contracts or worker bridges change, also run the affected
  producer checks below; record which are applicable. For a shared schema change,
  run all three Kotlin workers and both actual enrichment paths. Rust-only source
  interpretation changes may record this branch as not applicable.

  ```sh
  ./gradlew --no-daemon :workers:kotlin:test --tests dev.semanticthread.worker.SpringAnnotationFactsTest \
    :workers:kotlin21:test --tests dev.semanticthread.worker.SpringAnnotationFactsTest \
    :workers:kotlin23:test --tests dev.semanticthread.worker.SpringAnnotationFactsTest
  cargo test --locked -p clew --lib documentation::kotlin::tests::native_kotlin_19_maven_documentation -- --exact --ignored --test-threads=1
  cargo test --locked -p clew --test managed_cli durable_source_documentation_java_enrichment_recovers_on_the_same_source_roots -- --exact --ignored --test-threads=1
  ```

- **DoD:**
  - Common Spring rules retain existing semantic evidence quality and source-only limitations.
  - Existing semantic/Kafka rule behavior is covered by regression tests.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T07. Make declared OpenAPI usable as an independent contract module

- **Status:** - [ ]
- **Goal:** Declared API contracts remain documentable and staleable without compiler endpoint resolution, while source-derived comparisons stay separate.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC09; [scenario cards](../product/documentation-system/scenario-cards.md): S07, S11; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T05
- **Read first:**
  - [documentation/contracts.rs](../../crates/clew/src/documentation/contracts.rs), lines 1-160.
  - [documentation/analysis.rs](../../crates/clew/src/documentation/analysis.rs), lines 258-340.
  - [documentation/model.rs](../../crates/clew/src/documentation/model.rs), lines 19-54,279-300.
  - [documentation/render.rs](../../crates/clew/src/documentation/render.rs), lines 903-988.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - Existing documentation/contracts.rs, analysis.rs, syntax.rs, modules.rs, model.rs; new schemas/documentation/contract-facts.schema.json.
  - New fixtures/documentation-system/openapi/; documentation_system.rs: docsys_t07_*.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S07, S11 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Reuse the current declared OpenAPI reader/resolver and separate contract inventory from its present dependence on discovered HTTP endpoints. Register contract files as selected committed inputs with exact source digests.
  2. Preserve nested fields, references, parameters, responses, security/server declarations and constraints at supported schema versions. Map a contract to a source operation only when evidence supports it; unmapped declared operations remain documented declarations.
  3. State tested OpenAPI versions in module capabilities; unimplemented dialect features or external references remain explicit limitations. No network fetch occurs implicitly; referenced files must be registered or produce gaps.
  4. Test local references, cycles, missing/external references, unsupported versions, declared/source mismatch and contract-only changes invalidating dependent service/process content.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t07_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t07_ -- --test-threads=1
  ```

- **DoD:**
  - A declared contract is not silently promoted to source behavior or runtime enforcement.
  - No-K2 documentation can display a complete supported declared contract or exact missing facts.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T08. Generate standard service, responsibility and entity sections

- **Status:** - [ ]
- **Goal:** Registering two different services produces the same useful standard section structure, with stable domain objects and explicit discovery gaps.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC10, AC11; [scenario cards](../product/documentation-system/scenario-cards.md): S01, S04, S07; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T04, T06, T07
- **Read first:**
  - [documentation/model.rs](../../crates/clew/src/documentation/model.rs), lines 19-54,329-365.
  - [documentation/render.rs](../../crates/clew/src/documentation/render.rs), lines 903-988,1240-1275.
  - [documentation/cli.rs](../../crates/clew/src/documentation/cli.rs), lines 185-295.
  - [documentation/store.rs](../../crates/clew/src/documentation/store.rs), lines 79-175.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - New documentation/sections.rs and entities.rs; existing model.rs, store.rs, work.rs, proposals.rs, bindings.rs, render.rs, cli.rs.
  - Existing documentation HTML/CSS/JS assets; new schemas/documentation/section.schema.json and entity.schema.json.
  - New fixtures/documentation-system/services/; documentation_system.rs: docsys_t08_*.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S01, S04, S07 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Add stable predefined section identities and common required fields for service overview, responsibilities, entities and ingress/egress contracts. Keep compatibility mapping for old service/scenario operation IDs.
  2. Implement proposed `docs section list/show` and registration-driven work generation using the accepted author/reviewer pipeline. Distinguish public boundary inventory from internal callable evidence; missing discovery capability produces an explicit inventory gap.
  3. Represent domain entity IDs separately from implementation representations. Record created/changed/read/stored-copy/owned relations, provenance, confidence/limitations and human-declared versus agent-proposed ownership. Agent inference cannot change human ownership metadata.
  4. Extend read/query scopes and transitive document-object dependencies for shared entity/contract facts. Test duplicate/renamed entity candidates without arbitrary rebinding.
  5. Render the same mandatory page structure for a small and a 40-endpoint fixture; check normal desktop/mobile layouts, long labels, no-K2 gaps and source inspections. Save useful documented sections or visible gaps even if a specific work item fails.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t08_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t08_ -- --test-threads=1
  ```

- **DoD:**
  - Required service sections exist and are navigable after registration, even when incomplete.
  - Every in-scope public boundary is represented or its discovery/authoring gap is visible; entity ownership is not fabricated.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T09. Preserve and display imported human notes with separate assessments

- **Status:** - [ ]
- **Goal:** A team can add arbitrary notes related to documented objects and receive agent assessments without losing original text, tags or metadata.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC12, AC18; [scenario cards](../product/documentation-system/scenario-cards.md): S06, S10; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T08
- **Read first:**
  - [documentation/store.rs](../../crates/clew/src/documentation/store.rs), lines 79-175,207-233.
  - [documentation/bindings.rs](../../crates/clew/src/documentation/bindings.rs), lines 141-243.
  - [documentation/render.rs](../../crates/clew/src/documentation/render.rs), lines 903-988.
  - [documentation/model.rs](../../crates/clew/src/documentation/model.rs), lines 329-365.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - New documentation/notes.rs; existing sections.rs, entities.rs, store.rs, work.rs, review.rs, render.rs, cli.rs and reader assets.
  - New schemas/documentation/note-association.schema.json and note-assessment.schema.json.
  - New fixtures/documentation-system/notes/; documentation_system.rs: docsys_t09_*.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S06, S10 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Implement proposed `docs note import/associate --root <docs> --input <record>` using existing human Markdown or newly authored files. Preserve original bytes and metadata; store associations and generated assessments separately. Agents cannot write human-owned paths or ownership flags.
  2. Display related notes in section navigation/search and distinguish facts, historical context, policies, intentions and opinions. A rename changes an association only through a stable identity or explicit human update, not a guessed new target.
  3. Run optional assessments through evidence-bound review jobs. Store period, evidence, assessment status and proposed correction independently. Invalidate assessments when their inputs change without claiming complete strict tracking for arbitrary note prose.
  4. Test imported formatting/frontmatter/tags, malicious embedded instructions, conflicting current/history claims, concurrent note edits, deleted associations and rejected author attempts to rewrite protected material.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t09_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t09_ -- --test-threads=1
  ```

- **DoD:**
  - Original human text and metadata are byte-preserved through generation and concurrency.
  - A stale assessment never silently changes the meaning or provenance of the original note.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T10. Persist requested processes as maintained sections

- **Status:** - [ ]
- **Goal:** A process requested once becomes a stable navigable definition and updates with its dependencies, while transient questions remain transient.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC13, AC18; [scenario cards](../product/documentation-system/scenario-cards.md): S03, S09; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T08
- **Read first:**
  - [documentation/model.rs](../../crates/clew/src/documentation/model.rs), lines 115-152.
  - [documentation/store.rs](../../crates/clew/src/documentation/store.rs), lines 276-310.
  - [documentation/check.rs](../../crates/clew/src/documentation/check.rs), lines 380-603.
  - [documentation/render.rs](../../crates/clew/src/documentation/render.rs), lines 499-573.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - New documentation/processes.rs; existing sections.rs, entities.rs, work.rs, proposals.rs, check.rs, bindings.rs, cli.rs, render.rs and reader assets.
  - New schemas/documentation/process.schema.json and fixtures/documentation-system/processes/.
  - Existing documentation_system.rs: docsys_t10_*.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S03, S09 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Implement proposed `docs process put --root <docs> --input <definition>` through the skill for explicit saved requests. Definitions name scope, stable participants/objects, triggers/outcomes and declared interactions; they are not saved conversation transcripts.
  2. Reuse existing scenario selection and transport provenance. Add bounded linked subviews for wider processes without lifting current safety limits or inventing resolved cross-repository calls. Preserve supported old scenarios and asynchronous/Kafka boundaries.
  3. Compose accepted child explanations with explicit gaps and uncertainty, reviewed as a separate bounded overview. Parent dependencies include child claim/definition versions and their source influence. Handle cycles explicitly with bounded traversal rather than dropping dependencies.
  4. Test save/reopen/update/history-ready identities, transient exploration with no saved side effects, two-service conditional outcome, one unavailable participant and an added interaction after a negative query.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t10_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t10_ -- --test-threads=1
  ```

- **DoD:**
  - Saved processes enter navigation and maintenance through existing reader/refresh paths.
  - Unknown edges or failed children cannot become a verified synchronous happy path.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T11. Add entity data-flow views through the shared view contract

- **Status:** - [ ]
- **Goal:** A saved entity view explains evidence-backed reads, transformations, writes and transfers and stays consistent with related processes and contracts.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC14, AC18; [scenario cards](../product/documentation-system/scenario-cards.md): S09; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T09, T10
- **Read first:**
  - [documentation/model.rs](../../crates/clew/src/documentation/model.rs), lines 279-348.
  - [documentation/render.rs](../../crates/clew/src/documentation/render.rs), lines 499-592,988-1065.
  - [documentation/bindings.rs](../../crates/clew/src/documentation/bindings.rs), lines 45-141.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - New documentation/dataflow.rs; existing entities.rs, processes.rs, sections.rs, work.rs, proposals.rs, bindings.rs, render.rs and reader assets.
  - New schemas/documentation/dataflow.schema.json and view-module.schema.json; fixtures/documentation-system/dataflow/.
  - Existing documentation_system.rs: docsys_t11_*.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S09 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Define a versioned view interface naming input objects, supported edge kinds, authority/limitations, dependency derivation, validation and renderer version. Its first consumer is entity data flow, not a generic executable plugin marketplace.
  2. Implement proposed `docs view put --root <docs> --input <definition>` for a saved entity flow. Distinguish domain entity, DTO/message/table representations and declared transfers. Name equality alone produces a candidate/unknown edge.
  3. Bind mapper/read/write/transfer claims to source/contract/process facts; reuse accepted component explanations and review the bounded view meaning. Preserve human annotations and optional layout metadata with explicit ownership.
  4. Mutate one mapper used by multiple entity/process/contract views; assert every dependent claim/view becomes non-current, unknown edges stay unknown, and independent entities remain reusable where scope evidence permits.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t11_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t11_ -- --test-threads=1
  ```

- **DoD:**
  - Adding a view type requires explicit dependency and verification behavior.
  - Data-flow diagrams do not masquerade as runtime traces or universal taint analysis.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T12. Export and import portable per-service evidence

- **Status:** - [ ]
- **Goal:** A CI worker can hand off one repository result to a central documentation job without copying private Codeclew sessions or requiring every checkout centrally.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC15; [scenario cards](../product/documentation-system/scenario-cards.md): S01, S11, S12; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T05, T07
- **Read first:**
  - [documentation/analysis.rs](../../crates/clew/src/documentation/analysis.rs), lines 127-258,660-705.
  - [documentation/model.rs](../../crates/clew/src/documentation/model.rs), lines 162-230.
  - [documentation/check.rs](../../crates/clew/src/documentation/check.rs), lines 58-175,605-614.
  - [documentation/store.rs](../../crates/clew/src/documentation/store.rs), lines 27-72,188-233.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - New documentation/evidence_package.rs; existing modules.rs, analysis.rs, check.rs, cli.rs and store.rs.
  - New schemas/documentation/evidence-package.schema.json and fixtures/documentation-system/evidence-packages/.
  - Existing documentation_system.rs: docsys_t12_*.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S01, S11, S12 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Implement proposed `docs evidence capture --root <docs> --service <id> --output <package>` and `docs evidence import --root <docs> --input <package>`. Package identity covers source SHA, service/scope config, producer/rule schemas and digests, source fragments, facts, coverage and registered dependencies.
  2. Use bounded content-addressed package parts plus a closed manifest instead of a whole-catalog 64 MiB JSON. Validate byte/count/decompression/path limits and all referenced digests before admitting data. Preserve actual producer failures as outcomes with evidence provenance.
  3. Packages are produced only from supported capture/lifecycle operations. Do not serialize live private session IDs, credential paths or mutable CODECLEW_HOME objects. Import validates compatibility and origin/project association under coordinator-configured trust.
  4. Import in a fresh job with no application checkout and no compiler; reconstitute selected evidence for work/validation. Test corrupt/missing parts, path traversal, wrong SHA/service, unknown schema, forged stronger authority and package replay.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t12_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t12_ -- --test-threads=1
  ```

- **DoD:**
  - Artifact interchange is sufficient for central work without a shared mutable index.
  - Missing or rejected service artifacts become local gaps and cannot replace a newer valid result.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T13. Coordinate revision events and immutable history

- **Status:** - [ ]
- **Goal:** Central updates preserve exact target and per-section revisions across concurrent teams, repeated events and historical inspection.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC16, AC18, AC19; [scenario cards](../product/documentation-system/scenario-cards.md): S04, S05, S06, S12; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T01, T04, T09, T10, T11, T12
- **Read first:**
  - [documentation/check.rs](../../crates/clew/src/documentation/check.rs), lines 69-175.
  - [documentation/bindings.rs](../../crates/clew/src/documentation/bindings.rs), lines 141-243.
  - [documentation/render.rs](../../crates/clew/src/documentation/render.rs), lines 1125-1340.
  - [documentation/store.rs](../../crates/clew/src/documentation/store.rs), lines 207-233,374-399.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - New documentation/updates.rs and history.rs; existing evidence_package.rs, work.rs, sections.rs, notes.rs, bindings.rs, render.rs, cli.rs and reader assets.
  - New schemas/documentation/update-event.schema.json and publication.schema.json; fixtures/documentation-system/updates/.
  - Existing documentation_system.rs: docsys_t13_*.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S04, S05, S06, S12 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Implement proposed `docs update enqueue/run/reconcile --root <docs> --input <event-or-revision-set>` with idempotency keys, configured accepted refs, exact target vectors and per-repository event ordering. Coordinator acceptance of a target is independent of when an analysis result arrives.
  2. A new target first publishes conservative stale/unverified status. Coalesce superseded work; reject old result acceptance without losing its audit/history. Refresh unaffected services from admitted retained evidence, not re-running every compiler.
  3. At acceptance and publication, compare work/evidence/definition/human-note versions and prior publication identity. Update only work still valid for the selected target; conflicting work is locally rescheduled within budget.
  4. Implement proposed `docs history list/show/compare --root <docs>` and reader snapshot navigation. Record optional tags with observed commit targets; moving a tag cannot rewrite old snapshots. Preserve one explanation version across all of its views while displaying stale sections from older vectors honestly.
  5. Test two simultaneous service updates, duplicate/delayed events, a tag move, definition/note edits during generation, lost events followed by reconciliation, one failed service and interrupted publication.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t13_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t13_ -- --test-threads=1
  ```

- **DoD:**
  - Out-of-order work cannot regress accepted targets or overwrite newer authored material.
  - History is inspectable without rerunning a model, compiler or vanished source checkout.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T14. Provide portable CI jobs and a GitLab sandbox recipe

- **Status:** - [ ]
- **Goal:** Operators can run the same update contract locally or through GitLab with an externally configured agent sandbox and explicit permissions.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC17, AC19; [scenario cards](../product/documentation-system/scenario-cards.md): S08, S11, S12; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T13
- **Read first:**
  - [documentation/cli.rs](../../crates/clew/src/documentation/cli.rs), lines 16-185.
  - [documentation/store.rs](../../crates/clew/src/documentation/store.rs), lines 27-72,188-233.
  - [documentation/analysis.rs](../../crates/clew/src/documentation/analysis.rs), lines 75-211.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - New scripts/documentation_ci.py (including a configured qualify-gitlab entry point) and scripts/test_documentation_ci.py; new examples/documentation-ci/gitlab-ci.yml and agent-command.json.
  - New docs/operations/documentation-ci.md; existing docs/operations/source-documentation.md; documentation/agent_jobs.rs, updates.rs or cli.rs only for exercised integration gaps.
  - Existing canonical skill routing/references and both copies; operations.rs packaging inventory/digest if required.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S08, S11, S12 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Provide versioned event/job/result files usable without GitLab. The GitLab example forwards exact source revisions, retains evidence artifacts and serializes the publication step using platform facilities; retries are idempotent and reconciliation is invocable externally. Do not require a new permanent server.
  2. Provide a sandbox command adapter that can dispatch author/reviewer/fallback roles and return results/usage. DeepSeek-V4-Flash-0731 and stronger alternatives are configuration examples, not hard-coded model IDs or vendor SDK requirements. Use fake adapters for repository tests.
  3. Separate source-read/provider permissions, agent documentation-write capabilities and publication credentials. Validate work/result paths; source text cannot configure executable commands or authorize external communication. Publish only to an audience permitted to see the retained code/material; absent configuration exposes an actionable integration gap.
  4. Document worker CODECLEW_HOME isolation, disposable local .codeclew state, supported cleanup, artifact storage/retention, secrets injection, cancellation and notification outputs. Loss of cache must trigger reconstruction, not loss of history.
  5. Run local and GitLab-shaped event flows through the same fixture contract; test failed author/reviewer/process, malformed adapter output, absent usage, artifact expiry, concurrency and unavailable publication credentials. Do not send real external messages in tests. Implement `documentation_ci.py qualify-gitlab --config <installation-config> --output <results>` for the actual configured-platform comparison owned by T16; repository tests validate its adapter behavior with fake platform responses and make no live-integration claim.
- **Verify:**

  ```sh
  python3 -I -S scripts/test_documentation_ci.py
  python3 -I -S scripts/test_agent_skill.py
  ```

  If T14 changes Rust integration code, additionally verify the exercised runtime
  behavior; record not applicable only when this task changes tooling/guidance alone.

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --test-threads=1
  ```

- **DoD:**
  - The recipe is executable with fake jobs and configurable for the real sandbox without modifying core code.
  - Operational prerequisites are explicit; credentials and private source paths do not enter public manifests.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T15. Qualify transitive freshness and operational recovery

- **Status:** - [ ]
- **Goal:** Demonstrate the conditional freshness guarantee and failure isolation on a representative multi-repository workload before claiming production suitability.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC18, AC19; [scenario cards](../product/documentation-system/scenario-cards.md): S05, S06, S07, S09, S10, S11, S12; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T14
- **Read first:**
  - [documentation/bindings.rs](../../crates/clew/src/documentation/bindings.rs), lines 45-141,243-321.
  - [documentation/render.rs](../../crates/clew/src/documentation/render.rs), lines 1125-1340.
  - [documentation/syntax.rs](../../crates/clew/src/documentation/syntax.rs), lines 29-167.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - New fixtures/documentation-system/qualification/ and scripts/qualify_documentation_system.py with scripts/test_qualify_documentation_system.py.
  - Existing documentation_system.rs: create docsys_t15_* mutation/recovery cases; scripts/ci-verify.sh for bounded deterministic acceptance; affected core files only for observed qualification failures.
  - New docs/product/validation/documentation-system-runtime.md and documentation-system-runtime-results.json after execution.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S05, S06, S07, S09, S10, S11, S12 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Generate a deterministic 40-service corpus including a 40-endpoint service, linked entity/process/contract/note assessment claims and up to four independently changed repositories in a batch. Include differently sized registered scopes; record bytes/files and explicit budget boundaries, not only endpoint counts.
  2. Execute mutations for literal/docstring/helper/import/config/contract changes; added/deleted endpoint, file and negative-query member; provider/rule changes; duplicates/rename/split/merge; source-only callbacks; and a mapper shared across several views. Assert zero known affected claims falsely current within each declared test boundary. Count intentional broad invalidation separately.
  3. Exercise cache loss, missing source checkout after artifact capture, damaged package, worker termination, publication crash, concurrent human edits, duplicate/out-of-order events and one failed service. Restore retained history and enforce its referenced evidence retention; expired history is reported rather than silently presented as available.
  4. Measure cold/warm capture, peak memory, status-only latency, publication latency, artifact size and reuse at the declared workload. Do not claim a throughput/latency SLO not supported by measurements. If a bound fails, record the affected operating envelope and fix or narrow the qualification claim before production approval.
  5. Keep routine CI deterministic and bounded; run the larger qualification as an explicit command. Final full development gate is `./scripts/ci-verify.sh`; record actual results and remaining limits rather than treating tests as universal dependency proof.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t15_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t15_ -- --test-threads=1
  python3 -I -S scripts/test_qualify_documentation_system.py
  python3 -I -S scripts/qualify_documentation_system.py --fixture-root fixtures/documentation-system/qualification --output /tmp/codeclew-documentation-qualification
  ./scripts/ci-verify.sh
  ```

- **DoD:**
  - Runtime qualification report has explicit denominators, source revisions, failures and verified restoration behavior.
  - Any false-current or human-data-loss case blocks that safety claim; it does not make best-effort runtime sections disappear.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T16. Qualify the configured agent workflow and GitLab integration

- **Status:** - [ ]
- **Goal:** Establish whether the actual routine author/reviewer setup produces acceptable documentation with bounded fallback, and whether actual GitLab-triggered jobs match the local integration contract.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC17, AC20; [scenario cards](../product/documentation-system/scenario-cards.md): S08, S12; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T15
- **Read first:**
  - [documentation/cli.rs](../../crates/clew/src/documentation/cli.rs), lines 297-733.
  - [documentation/render.rs](../../crates/clew/src/documentation/render.rs), lines 73-494.
  - [documentation/model.rs](../../crates/clew/src/documentation/model.rs), lines 329-365.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - New scripts/evaluate_documentation_agents.py and scripts/test_evaluate_documentation_agents.py; new fixtures/documentation-system/agent-evaluation/ protocol and source-grounded rubrics.
  - New docs/product/validation/documentation-system-agent-results.md and documentation-system-gitlab-results.md with sanitized JSON after actual runs; update service-documentation skill guidance only from observed failures/strengths.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S08, S12 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Run the T14 qualification command against an explicitly configured GitLab test project/runner and the same synthetic source revisions through the vendor-neutral local path. Exercise an actual accepted-source pipeline trigger, evidence transfer, configured sandbox access denials, a failing service/agent job, retry/idempotency and concurrent publication. Use deterministic fake model responses for this platform comparison so it does not depend on paid-model access. Retain sanitized pipeline/job identifiers, revisions, results and failures; compare with local behavior. A GitLab-shaped fixture is not this qualification. Missing platform access leaves AC17 integration qualification incomplete and does not block unrelated local runtime, migration or guidance. Separately freeze a task-local protocol before paid execution: independently varied repositories, initial authoring and maintenance, no-K2 Kotlin/Java, contract/entity ownership, branches/callbacks, process/data-flow, human-note contradictions and missing evidence. Keep expected answers out of producer prompts.
  2. Compare the configured routine pipeline and its fallback behavior on the same tasks/revisions/quality obligations. Fix and record author/reviewer/repair/fallback configuration; use a source-grounded evaluator distinct from production author/reviewer roles. Do not assume suitability from a model name.
  3. Require an explicit installation/run config containing credentials references, compatible models, finite full-contour budgets and pre-dispatch stop conditions. Reuse T04's atomic reservations, capped calls and unknown-usage stop rule; include the independent evaluator and qualification orchestration in the same full-contour accounting. No config means this qualification is not run and no model-readiness claim is made; do not fabricate successful deployment integration from fake adapters.
  4. Retain all failed attempts and native/evidence expansion. Report unsupported claims, missed required facts, false-current results, abstentions/gaps, repairs, escalation rate, latency and actual usage with cache categories when available; missing monetary/usage fields remain unavailable. Include stronger model and evaluator costs in the full-contour report.
  5. Use paired tasks with repetitions across repositories appropriate to the frozen run budget; document sample limits and correlated-error risk. A critical false-current/unsupported assertion fails its tested profile; an explicit correct gap is availability with limitations, not a complete-answer pass. Classify routine-capable, fallback-required and unqualified strata without inventing a universal saving.
- **Verify:**

  ```sh
  python3 -I -S scripts/test_evaluate_documentation_agents.py
  python3 -I -S scripts/documentation_ci.py qualify-gitlab --config /run/secrets/documentation-gitlab-qualification.json --output /tmp/codeclew-documentation-gitlab-qualification
  python3 -I -S scripts/evaluate_documentation_agents.py --protocol fixtures/documentation-system/agent-evaluation/protocol.json --config /run/secrets/documentation-evaluation.json --output /tmp/codeclew-documentation-agent-evaluation
  ```

- **DoD:**
  - AC17 is satisfied only by recorded actual GitLab-triggered/local comparison evidence; missing access keeps this qualification pending and T16 incomplete.
  - Actual configured model combinations are classified using executed evidence; absence of access or results remains a qualification gap.
  - No unacceptable narrative can become verified-current solely because an LLM reviewer approved it; known machine/evidence failures remain decisive.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## T17. Finish skill, migration and production acceptance documentation

- **Status:** - [ ]
- **Goal:** An author and operator can use the implemented system from a fresh checkout with accurate guidance, compatible old documentation, and an honest readiness verdict.
- **Sources:** [Target acceptance](../product/documentation-system/target-system.md#acceptance-contract-and-traceability): AC01-AC20; [scenario cards](../product/documentation-system/scenario-cards.md): S01-S12; [impact](../product/documentation-system/increments/durable-documentation-impact.md).
- **Depends on:** T15
- **Read first:**
  - [documentation/store.rs](../../crates/clew/src/documentation/store.rs), lines 79-175.
  - [documentation/cli.rs](../../crates/clew/src/documentation/cli.rs), lines 16-185.
  - [documentation/render.rs](../../crates/clew/src/documentation/render.rs), lines 1125-1340.
  - The scenario cards and approved target clauses named in Sources; existing test helpers in [managed_cli.rs](../../crates/clew/tests/managed_cli.rs).
- **Modify:**
  - Existing skills/codeclew/SKILL.md and documentation references, .agents/.claude copies, crates/clew/src/operations.rs embedded inventory/digest and scripts/test_agent_skill.py as needed.
  - Existing documentation/store.rs generated AGENTS/examples; docs/operations/source-documentation.md and documentation-ci.md; README.md.
  - Existing documentation_system.rs: create docsys_t17_* migration and fresh-init walkthrough cases.
  - New docs/product/validation/documentation-system-implementation-verdict.md; complete the scoped baseline/cards/graph/impact implementation-status records.
- **Product artifacts:** Update [cards](../product/documentation-system/scenario-cards.md) S01-S12 implementation notes and listed extension-point regression evidence; [baseline](../product/documentation-system/current-scenario-baseline.md) observed deltas; [impact](../product/documentation-system/increments/durable-documentation-impact.md) task status. Update [graph](../product/documentation-system/scenario-graph.dot) if these transitions change; otherwise confirm the existing edges. Keep approved AC wording unchanged.
- **Steps:**
  1. Reconcile every public command, schema version, runtime compatibility check and author/reviewer instruction with implemented behavior. Fix the existing init Narrative 1.1 / authoring example 1.2 / guide 1.3 inconsistency through an explicit current version and supported readers.
  2. Ensure source-first progression, captured influence, strict versus note assessment status, best-effort publication and finite escalation are unambiguous. Readers of the skill must not need to hand-build internal dependency JSON or require K2 to obtain useful sections.
  3. Verify old service/scenario records, direct Narrative imports and existing source/Kafka documentation retain a supported path; migration must preserve protected files and historical bundle meaning. Publish an example using the actual supported launcher.
  4. Perform a fresh operator/author walkthrough using documented commands, including an unavailable optional provider and one missing agent input. Produce a final implementation verdict from T15, the walkthrough and available T16 results; if either T16 qualification is incomplete, explicitly record the corresponding actual GitLab integration or model qualification as pending; T16 evidence gates only those readiness claims, not this task's guidance, migration or walkthrough; unmet production/model conditions remain explicit blockers for those claims.
  5. Update product artifacts only with observed implemented deltas. The product target and acceptance wording remain immutable absent a new approved decision. Full CI is not repeated after documentation-only edits unless a material runtime delta warrants it; packaging and documentation checks still run.
- **Verify:**

  ```sh
  cargo fmt --all --check
  cargo test --locked -p clew --test documentation_system -- --list | rg '^docsys_t17_[^:]+: test$'
  cargo test --locked -p clew --test documentation_system docsys_t17_ -- --test-threads=1
  cargo test --locked -p clew --lib operations::tests::embedded_agent_skill_digest_matches_portable_installer_contract -- --exact --test-threads=1
  python3 -I -S scripts/test_agent_skill.py
  python3 -I -S scripts/check_english_content.py
  python3 -I -S scripts/check_repository_privacy.py --pre-commit
  ```

- **DoD:**
  - Guidance, examples, skill copies, embedded package and implementation agree.
  - The final report distinguishes implemented, tested, operationally configured and unqualified capabilities.
  - Relevant checks pass with the required test cases actually executed; record evidence before changing Status.

## Completion check

Run this only during implementation to report remaining work. During plan review all
18 tasks are intentionally unchecked; that is not a validation failure.

```sh
python3 -I -S - <<'PYCODE'
from pathlib import Path
import re
text = Path('docs/plans/documentation-system-implementation-plan.md').read_text()
remaining = re.findall(r'^- \*\*Status:\*\* - \[ \].*$', text, re.M)
print(f'{len(remaining)} tasks remain' if remaining else 'PLAN-COMPLETE')
raise SystemExit(bool(remaining))
PYCODE
```

The [independent planning verdict](../product/documentation-system/validation/verdict.md)
approves only this package's consistency and suitability as an implementation target.
Final production claims require the executed T15-T17 evidence and installation inputs.
