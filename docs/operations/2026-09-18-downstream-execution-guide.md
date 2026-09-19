# Downstream execution guide after 5fdfa15

## Decision and scope

The remaining complete product is substantial. That is not a blocker for the next bounded change. Stop treating storage, capture batching, authoring semantics, native qualification and service prose as one deliverable.

This guide records a read-only review of primary checkout `5fdfa15` and the Nessy reports. It dispatches small immutable successors; it is not evidence that their code or tests exist yet. No customer build, cache migration, cleanup or external publication is authorized. Product tests were not run during this planning review.

## What the commit and reports actually establish

`5fdfa15` contains real changes: source-scoped projection, indexed-byte integrity checks, owned attempt disposal, explicit analyzer processor options, and per-fact persistence of `Check.dependencies`. Preserve these. Do not restart a storage/database or source-authority rewrite.

All six source A-F reports pass the installed protocol validator and list their planned final commands. That validates report structure/declared execution, not test adequacy.

### Source F is not the full native lifecycle gate

`tests/practical_source_lifecycle.rs` contains two tests:

- A real `./clew --help` launch: legitimate bootstrap smoke coverage.
- A docs capture whose service JSON explicitly selects `source-syntax` (line 250). Its source assertion permits `EXACT_SNAPSHOT_TEXT`/`DECLARED_OPENAPI`; its two `Check::load` calls occur in the same test process (lines 271-302).

Despite its name, the latter does not prove native Maven writable transformation. Neither test injects post-index mutation or disposal failure. C/D have genuine component-level rejection/disposal tests; F cannot relabel them public failure-path coverage. The historical report remains unchanged. New source G covers native transformed **success and cross-process pinned reopen only**. Fault-path qualification remains open below.

### Snapshot-v2 stopped with useful partial work

The report is terminal `failed`, has no final verification entries, and attributes the stop to a user-directed stop after step 2. Do not describe it as a spontaneous refusal or try to resume the terminal run. Its partial changes are now committed, regardless of the report's historical description of uncommitted files.

The concrete step-3 problem is real: `store_bindings_heavy` sets references without removing inline maps, while `equivalent_publications_share_heavy_evidence_and_stay_portable` requires inline payload for portability. `baseline` currently does not hydrate these references. Clearing maps alone is therefore an incorrect fix.

Step-2 completion is narrower than its report wording:

- `Check::store_manifest` connects per-fact dependency persistence to production (`check.rs:780-798`). Service capture maps still use the older aggregate objects.
- `fact_index::BUCKETS = 64` is a bucket-count bound, **not** a 256 KiB encoded-page bound (`fact_index.rs:24-28`). There is no deterministic page splitting.
- `publish_root` verifies then locks and replaces the pointer without comparing an expected old root (`fact_index.rs:183-187`). It is not currently used by the production Check immutable-manifest path; do not invent a current Check lost-update incident from this helper gap.
- `load_observations` scans buckets; `Check::load` hydrates all service manifests and dependencies (`check.rs:822-830`). `RequestScope` has no production callers.
- `Work.checked` is a full `Check` (`work.rs:120`), and the work-file writer rejects aggregate size above 64 MiB (`work.rs:390`). Existing bounded output packets do not make preparation IO bounded.
- `proposals::current` captures again (`proposals.rs:221`), then publication captures again in `render::publish_internal` (`render.rs:1547`). Unchanged output digests cannot prove these calls did not occur.

## Ready-to-execute immutable requests

All plans live under `.nessy/a2a-runs/<run-id>/plan.json`. Each has named production seams, regression cases, explicit iteration commands, and 4-5 focused final checks instead of the old eight-suite gate. New test targets are implementation work, not missing-environment blockers.

| Order | Run | Small delivery | Explicitly not claimed |
|---|---|---|---|
| 1 | `20260918-docs-a-pinned-snapshot` | Immutable check selector; `docs context --snapshot`; no implicit capture on missing evidence | Selective hydration, topic invalidation |
| 2 | `20260918-docs-b-work-references` | Versioned Work storage referencing snapshot/large tables; prepare/read without recapture | Bounded preparation memory or production selective IO |
| 3 | `20260918-docs-c-pinned-publication` | Same snapshot through proposal/review/render; retain conflict/manual-edit checks | Semantic reuse across changed snapshots |
| 4 | `20260918-docs-d-bindings-formats` | Reference-only local bindings and explicit self-contained portable export | Per-source granular storage and full large-export qualification |
| Separate storage branch | `20260918-docs-e-bounded-index` | Actual 256 KiB index pages and expected-root conflict checking | Production topic reader integration |
| After A, separate qualification branch | `20260918-source-g-native-reopen` | Native writable file-byte transformation and second-process pinned source read | Public injected mutation/disposal failure |

A has no dependency on completing the failed snapshot-v2 umbrella or overstated F gate. B depends on A; C on A/B; D on C. E is independent. G depends on A because a second **read** process must not call `docs check` and silently recapture.

These branches describe dependency independence, not permission for concurrent agents to edit the same working tree. Execute one run at a time unless ownership is separately arranged.

### Fixed implementation choices

1. Reuse `CheckManifest`, `cache::ObjectRef`, `Work`, proposal/review machinery and `Bindings.accepted_versions`. Do not introduce a parallel topic/evidence database.
2. A pinned snapshot is immutable historical capture evidence, not verification of the current checkout. Keep Maven `NON_CACHEABLE` for fresh capture. Missing pinned evidence is an error, never a fallback build.
3. Retain the in-memory `Work` initially; a versioned disk DTO can remove duplicated persistence without rewriting every consumer at once. Verify V2 IDs over the stored representation before hydration; V1 keeps its original hash contract.
4. Preserve stable accepted topic keys already used by review: `subject/operationId`. Merely keeping the key stable is not enough to prove selective invalidation or unchanged text reuse.
5. Local and portable bindings have different serialization obligations. A portable export must materialize the full historical closure, recalculate its own manifest hashes, and open without a private cache. It must not modify the original immutable bundle.
6. For bounded index pages, use deterministic hash-prefix splitting with versioned nodes, not a larger fixed bucket count. Cap actual encoded branch and leaf bytes, including envelopes.
7. Capture-count assertions observe provider entry/process launches. Unchanged `latest-check` bytes, file mtime, `contextDigest`, or output HTML are insufficient evidence of zero builds.

## Independent user-requested compilation limit increase

`20260918-compilation-limit-128` is ready independently of A-G. It raises the selected **compilation** cap from64 to128; one Maven module with main and test source sets can consume two selections. The relevant guards are:

- `session.rs:3319,3343`: initial and persisted canonical selection validation;
- `generation_service.rs:4278`: ready-set authority count;
- `repository_diagnostic.rs:15`: discovery count;
- `documentation/store.rs:497`: service selector count;
- `thread_callables.rs:37`: aggregate callable compilation budget.

No upstream Maven/Gradle count cap was found in the inspected provider paths. Use one shared limit and tests for64/65/128/129 **distinct** selectors. Preserve duplicates/grammar checks and report discovery truncation honestly.

The callable frozen budget is persisted authority, not only a runtime guard (`thread_callables.rs:85-115`). New128 authority must have the correct identity; old64 derived callable evidence requires the supported rebuild path or a targeted compatibility error, not a silent budget upgrade or private-cache deletion.

Do not change fact-index BUCKETS, process participants, graph nodes/edges, aggregated entrypoint session count, build concurrency, or byte/context budgets as part of this request. Count-admission tests do not claim that a128-module native build was benchmarked.

**Count vs modules/main+test.** The 128 cap is a **selection count** (module/source-set selectors), not a module count: one Maven module with both `main` and `test` source sets consumes two selections. The shared `limits::MAX_SELECTED_COMPILATIONS = 128` bounds a single session, ready set, discovery pass, documentation service and callable aggregate, all of which reuse the same constant. Build concurrency (`generation_jobs`), entrypoint session counts, discovery byte bounds, fact-index `BUCKETS`, and the 64 MiB byte budgets are unchanged; only the selection-count guards and their messages moved to 128. `20260918-compilation-limit-128` is implemented: production seams share the constant, boundary tests cover 64/65/128/129, and no runtime/resource budget was raised.

### Doctor task complement

The user identified another compilation-count guard after the original limit plan was written: `operations.rs:352`, inside `task_checks`, accepted at most **32 compilation selectors**. This is not a tool-call budget. `doctor task --compilation` is repeatable and the CLI has no separate count ceiling. `20260918-doctor-compilation-limit-128` (executed after accepted completion of `20260918-compilation-limit-128`) replaced that independent 32 guard with the shared `limits::MAX_SELECTED_COMPILATIONS = 128`, so doctor task now uses the same 128 count bound as session/ready/docs selection; no 128 literal or separate count was introduced. `SELECT_EXACT_COMPILATION` remediation, the nonempty-list and nonempty-element checks, and the required `task.compilation-authority` identity remain intact; unrelated runtime/profile/operation/repository/tool checks are not bypassed, and a passing selector count alone never implies overall readiness.

Its production-check and admitted public CLI regressions cover 32/33/64/65/128/129 distinct repeated `--compilation` flags through the public dispatcher: through 128 the authority check passes, and 129 fails only the count check with `SELECT_EXACT_COMPILATION`; zero/empty selections remain failed required checks.

## Remaining obligations: deliberately not another umbrella plan

The following work is still required. Create its immutable executable plans after reviewing the immediately preceding APIs, not by inventing future interfaces now. The old topic/service-v2 plans remain requirement inventories, not the next execution requests. Do not run them as-is after A-D: their prerequisites still point to the failed umbrella run.

### 1. Granular capture records and selective production reads

**First slice: ingestion/storage only.** Extend `cache::CaptureManifest`/`store_capture`/`load_capture` to reference per-source text and canonical fact payloads plus scope-aware occurrence indexes. Keep separate occurrence provenance (revision, compilation, transformed state) rather than stripping it from authority. Reuse E's bounded pages. Legacy whole-map envelopes remain readable; no automatic rewrite of old objects. Test two scopes sharing payload, same relative path with different bytes, one update, one removal and corruption. Do not include Work/planning changes in this first slice.

**Second slice: one real consumer.** Connect `access::RequestScope` to Work topic reads, then expand callers incrementally. Plan the topic closure from index metadata before opening payloads. A request must open/decode a unique required payload once and not hydrate all of `Check`. Counters must be attached to actual cache IO/decodes, not incremented by assertions. An unrelated large service may grow index depth but must not increase a fixed topic's payload reads or returned bytes. Persist handles/continuations without stuffing a full handle table back into the packet.

**Budget gate.** Default topic packet <=48 KiB UTF-8; estimated token budget <=12,000 with a documented estimator, not a claim of exact provider tokenization. Work manifest <=256 KiB. Continuations are explicit and deterministic; never automatically page an entire service into a model request. Preserve omission reasons, totals and incomplete coverage.

### 2. Semantic topic identity and explicit reuse

Extend existing `review::AcceptedVersion`, not a new competing accepted-description store. Separate:

- stable `subject/operationId`;
- semantic context/schema/extractor contract;
- selected snapshot and occurrence/source-link provenance;
- accepted prose/diagram content hash and review state.

First implement semantic-context computation and tests without CLI queue changes. Use only relevant facts, declared notes/relations and meaningful unknown boundaries. Positive fact IDs alone are insufficient: include discovery query/membership dependencies so a newly added endpoint, handler or domain candidate invalidates its inventory topic. Existing `entity-scope:<service>` and process/view scope observations are extension points, not missing infrastructure.

Preserve compatible unchanged text byte-for-byte across provenance relocation or verified equivalent recapture. Incompatible extractor/profile/processor authority must stale/fail closed; do not achieve stability by blindly dropping metadata. A broad `SOURCE_SCOPE` dependency on every fragment may over-invalidate; refine per-topic influence rather than removing discovery protection.

Then add queue/status behavior: missing, stale, reusable; explicit selected generation request; explicit force creates a new reviewable version. Changed evidence marks stale but does not silently overwrite accepted/manual prose. Context preparation is not text generation. Test insertion/deletion, irrelevant service changes, relevant contract/body changes, line movement, authority incompatibility and force through existing review/accept paths.

### 3. Useful service sections before advanced automatic process inference

`sections::records` currently calls a section AUTHORED solely because an operation with its ID exists (`sections.rs:50-58`). Separate content presence, accepted review, evidence coverage and unresolved gaps. Permit a legitimate prose-only overview without fake participants, but reject empty structural stubs as complete coverage.

Use existing section IDs, `entities` declarations, process definitions and bounded work/proposals to render useful partial pages. A missing process topic must not suppress overview/entities/ingress/egress. Technical JPA/DTO/record candidates are evidence for review, not automatically business entities or ownership. Reviewed domain relations keep their origin and supporting context.

First regression: synthetic multi-compilation service with meaningful accepted overview and interface sections, missing process detail, actual rendered page and visible gaps. No compiler-flow redesign is needed to demonstrate this first useful page.

### 4. Loss-aware flow and compilation-aware process graphs

Split extractor and renderer work:

1. In `java_analyzer.java`, replace the source-order-only 45,000-byte prefix truncation (lines 434-449) and 512-event drop policy with bounded event chunks/retrievable continuation plus compact summary. Preserve a late egress, persistence and dispatch event in a long method. Keep event/source ordering and explicit incomplete coverage. Large lists of essential events still require pagination; do not remove the limit.
2. Add conditional/deferred semantics for supported guards/lambda bodies. Discovering a lambda does not prove execution. Unsupported exception/reflection/concurrency retains explicit boundaries.
3. Propagate compilation occurrence identity to process/dataflow graph participants and evidence references. `processes::Details.scope` is an authored label, not compilation scope. Same symbol in different compilations must not collapse.
4. Label edges compiler-resolved, verified framework-derived, reviewed declared, candidate or unresolved. Existing dataflow authority and candidate support is useful. An injected interface collection proves candidates only unless registration/selection is verified. Separate HTTP submission, asynchronous engine handoff, dispatch and handler effects.
5. Render deterministic validated graphs; external authors supply reviewed labels/explanations, not unvalidated runtime edges. Test escaping, node references, bounded complexity and visible uncertainty without downloading a renderer.

### 5. Capture batching, real portable limits, growth and final integration

These are separate expensive gates, not mandatory tests for every codec change:

- **Reactor batching:** inspect `ensure_java_generation_set`'s per-compilation model-extraction loop. Batch only compatible compiler/settings/profile/processor authority. Share a real compatible reactor build, not incompatible scopes. Use an offline counting Maven fixture; incompatible groups must remain separate.
- **Structured >64 MiB portable qualification:** current `review_portable_limits` creates sparse `.bin` files and manually stages publication state. Keep it as a file-size bound test, but add actual structured bindings/evidence through production publication/export. Test reload, corrupt/missing required object, over-limit rejection and unchanged prior publication. No zero-filled file can stand in for JSON closure qualification.
- **Growth/inventory:** ten unchanged consumer sequences yield zero compiler/model calls, no new heavy payload bytes and unchanged accepted artifact hashes. Inventory distinguishes logical/allocated bytes, owned attempts, retained roots/leases and conservative reclaimable closure. Report only; never delete old customer/private state.
- **Final synthetic seven-module integration:** web, shared contracts, utility and four handlers, plus a second unrelated service. Actual native capture -> bounded topic work -> fixed proposals -> review/accept -> pages/diagrams -> process restart. Include same-symbol scopes, test-only handlers, late events, unknown dispatch, one-handler change and handler removal. Scale variant >=1,300 source records, >=50,000 memberships, >64 MiB structured evidence; counts are not interchangeable. Measure build count, distinct payload reads/decodes/writes, packet bytes/estimated tokens and artifact hashes. Fixed prose validates the pipeline, not external model writing quality.

### 6. Remaining source fault-path qualification

G does not satisfy the old F negative-path requirements. Do not use either of these invalid substitutions:

- Mutating the original checkout **between** two checks is not a post-index/pre-persist race; a fresh capture may legitimately index the new bytes.
- A mutation before `seal_tree` is a capture-to-seal test, not post-index evidence rejection. `ensure_java_generation_set` explicitly seals **before** indexing (generation_service.rs:731-784).

Find the actual post-index/pre-persist seam inside `ensure_java_generation` and the existing owned cleanup boundary. Prefer an explicit dependency-injected observer on a crate-private orchestration function, with a production no-op and tests scoped to their owned fixture; avoid global mutable hooks. Use a crate-local test to exercise the same complete orchestration path if private types prevent an integration crate from injecting faults, and label that layer honestly. A separate subprocess fixture must observe public failure/result and owned attempt counts. Never add an environment-controlled production mutation/authority bypass merely to satisfy a test.

Keep baseline ready/check/publication pointers unchanged on rejection. Test cleanup refusal preserving the original error, successful owned disposal, and ten isolated success/failure cycles. Permission/ownership tests cannot delete preexisting cache data. This remains a required review checkpoint before claiming the entire original native source gate complete.

## Execution protocol and acceptance

Start with A, not with planning the whole roadmap again. After each run, review the changed seam and actual test assertions, then accept or create a new correction ID. Terminal artifacts are immutable. `VALID` is necessary protocol evidence, never a semantic acceptance verdict.

### Deterministic review test seam

The existing meaning-review acceptance path is in `agent_jobs.rs:720-755`: validate the fixed review, call `review::versions`, then `render::publish_reviewed`. There is no need to invent a new review database or claim that unassessed `proposal publish` is meaning-reviewed. For C, extract/reuse that deterministic acceptance segment for fixed synthetic review tests without launching an external agent. Keep one dispatcher test for submit/unassessed publish and a separate honest test of the real reviewed-publication segment. Never directly create accepted-version files to satisfy a test.

Each executor should:

1. Read its plan and the relevant narrow source/diff; do not read every downstream subsystem.
2. Add one regression and implement the named behavior before starting final verification.
3. Use iteration commands for red/green work; report an actual missing artifact/permission/API conflict if blocked.
4. Preserve partial work on failure and state exactly which criterion remains unmet. Anticipated later work size is not a current blocker.
5. Never claim customer documentation was generated: that rollout requires a separate authorized session from the customer workspace after these gates, using its JDK17 configuration and supported CLI.

The staged `tnessy` guidance patch remains in `docs/operations/tnessy-recovery-update/`; it has not been applied or installed into the sibling skill repository by this session. Its documentation/permission/placeholder checks passed; it does not change executor safety rules.
