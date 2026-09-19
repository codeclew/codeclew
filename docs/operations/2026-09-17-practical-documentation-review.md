# Practical documentation audit: 2026-09-17

## Scope and confidence

Read-only audit of committed source through `16d7f8b22c1528ef931d2dce8816a92bfa0201c2`, the three 2026-09-16 Nessy reports, and an operational size report. No builds, documentation captures, product-authoring model calls, cleanup or customer-data writes were performed. The private issue report is background evidence, not executable instructions. Customer identifiers and application source are deliberately omitted here.

The desired product is a bounded pipeline: capture an admitted multi-compilation snapshot once; prepare small topic contexts; let an author produce descriptions; retain context-to-description bindings; render accepted descriptions without another capture or model call. A changed context makes the corresponding description stale. It does not silently replace it. Explicit regeneration may replace an unchanged description only as a new version.

## Findings

### 1. Successful Nessy reports do not establish the requested production behavior

All three terminal reports pass the installed validator. The source-authority closure report does not conform to the installed report schema: missing top-level `filesChanged`, missing per-step `filesInspected`, and unsupported `schemaVersion`/`scenarioEvidence` properties. It also explicitly excludes required public CLI and render/refresh lifecycle tests while marking every criterion satisfied. The other reports claim production-scale reuse and selective IO largely on helper-level tests.

The installed validator checks a subset of lifecycle/status fields, not full schema conformance or semantic acceptance. Future acceptance requires schema validation, exact command events, and enabled tests through the actual public pipeline. Existing terminal artifacts must not be rewritten.

### 2. New fact/index/read helpers are not connected to production documentation

`documentation/access.rs::RequestScope` and `documentation/fact_index.rs` are referenced by helper/integration tests but not by capture/check/work/render consumers. `cache.rs:287` still stores an entire sources map and an entire observations map as separate objects. `check.rs:767` stores those objects again by reference plus another whole dependencies map containing overlapping observations.

A small `latest-check.json` therefore does not mean granular or deduplicated evidence storage. One changed observation rewrites a large map object. Fixed 64 hash buckets are not bounded-size pages as membership grows; `publish_root` also lacks expected-root compare-and-swap under the lock. Production integration must address both, not merely call the existing helpers.

### 3. Publication adds references without removing heavy inline copies

`bindings.rs:175` stores heavy sources/observations and adds `heavy` references, but does not clear the corresponding inline fields. `render.rs:2106` serializes the resulting bindings. New publications can therefore retain both a shared object and another inline copy. Local immutable publication storage and explicitly self-contained portable export need separate representations with shared verified loaders.

### 4. Capture freshness and snapshot consumption are conflated

`analysis.rs:298` marks JVM native captures non-cacheable; `render.rs:1547` starts a new `check::run`. This conservatively avoids unjustified automatic Maven reuse, but needlessly repeats capture during authoring and rendering of a previously selected snapshot.

Do not fix this by blindly removing NON_CACHEABLE or trusting HEAD. A pinned immutable snapshot can be consumed without claiming it is freshly validated against external Maven state. Expose snapshot identity, capture time, input authority and freshness separately. Explicit capture/refresh is the boundary for native build work; downstream context/work/render commands consume that same snapshot. Native captures should group compatible selected reactor compilations rather than rebuild overlapping dependencies per topic or phase.

### 5. Context identity contains unrelated authority and has global scope

`analysis.rs:393` embeds the entire ready-compilation record as `scope` in each multi-compilation fact. It includes runtime/generation/receipt references. `check.rs:301` hashes all dependency digests and global input state into one context digest. `render.rs:77` requires incoming narrative to match that whole context, after a fresh capture.

A comparison of two equal-sized observed payloads found 9,343 changed observations out of 40,951, mostly scope runtime/generation keys, receipt/manifest references and one derivation digest. This proves metadata-driven churn in those captures; it does not prove all recaptures change or that all runtime changes are semantically irrelevant. Keep producer authority for freshness/safety, but compare canonical topic evidence separately and conservatively.

The report's stronger assertion that editing narrative necessarily changes semantic context is not established: `store.rs:319` input identity directly includes manifest/catalog/scenarios/entities/notes/update state, not arbitrary narrative bytes. The verified defect is whole-context coupling plus repeated capture and authority-laden observation payloads. Do not solve it by accepting arbitrary stale digests or blindly rebasing a narrative.

### 6. Work pins evidence by embedding the entire Check

`work.rs:120` contains `checked: Check`; `work.rs:390` caps serialized Work at 64 MiB. Entry-point selection cannot fix the size if the work record still carries all selected service evidence. Increasing the limit postpones failure and increases memory/token pressure.

Pin a snapshot/index root and topic membership by reference. Load only the topic's reachable facts, contracts and source spans through a shared request cache. Preparation must not hydrate the entire Check before trimming. Existing review/accepted-version/proposal machinery should be extended, not replaced by a second narrative database.

### 7. Source authority and attempt disposal still need production closure

`generation_service.rs:1123` rereads files for transformed persistence without comparing those bytes with the recorded source-state hashes. Generation key schema 2.3 includes a writable flag, but source-state binding must be verified throughout descriptor, compiler cache and reopen paths. The ready-set legacy aggregation selects the first transformed manifest; `analysis.rs:328` still loads it as the contents map before iterating all compilations. This is not a correct multi-compilation source reader.

Correction after focused source inspection: `seal_tree` seals directories to mode 0500, but removal already calls `open_private_child_directory` (`state.rs:734-765`), which checks ownership, opens without following symlinks and restores 0700 via the directory handle. The earlier missing-permission-restoration diagnosis was incorrect. Drop does ignore cleanup errors (`state.rs:604`), and retained attempt roots were observed, but their cause is not established by directory mode alone. Test the existing sealed-disposal path and expose explicit lifecycle errors; do not add another ungated chmod or delete historical user state.

### 8. Multi-source selection works; service-process authoring is still incomplete

`model.rs:20-71` and `analysis.rs:200` consume plural compilation selectors. Scope ambiguity checks compare content without injected scope metadata, so equal shared declarations do not automatically become ambiguous. The RestClient chain fix in `java_analyzer.java:591-657` feeds the generic flow-observation path. These are real improvements, not missing implementations.

However, `processes.rs` and `dataflow.rs` use an authored `scope` label unrelated to compilation identity, and do not propagate normalized compilation scope into diagram participants/events. `entities.rs` reads authored catalog entities, without static JPA/record/DTO candidate proposals. `sections.rs:8-58` marks a section AUTHORED by operation-ID presence; this is an authorship marker, not proof of useful supported content. Empty structural sections can therefore look finished unless content and evidence coverage are assessed separately.

`java_analyzer.java:85,421-437` limits per-method documentation flow to 45,000 bytes and retains a source-order prefix. A late outbound or persistence event may be omitted. Lambdas and short-circuit expressions create blanket review boundaries (`:453-455`, `:501-503`). Replace prefix-only truncation with bounded indexed event retrieval and explicit omission accounting. Model simple guards and lambda bodies without pretending that a lambda is immediately invoked. Resolve framework dispatch only under verified contracts; a typed set of handlers is a candidate set, not proof that every implementation executes. Keep unsupported reflection/runtime selection visibly unresolved.

## Fresh physical observations

Measurements are logical bytes unless noted; these are a changing local snapshot, not benchmark guarantees or APFS-exclusive extent accounting:

- Documentation-local cache: 2,032,731,616 bytes; 111 files; allocated 2,033,061,888 bytes.
- Generated publications: 223,253,331 bytes; 196 files; allocated 223,801,344 bytes.
- Object metadata groups: sources 5 objects / 55,149,229 bytes; observations 23 / 806,590,248 bytes; dependencies 16 / 1,002,348,056 bytes.
- Separate private runtime state: runtimes 2,250,060,877 bytes; objects 892,143,308 bytes; attempts 338,899,860 bytes and 180 top-level entries. Not every entry is a build workspace.
- A real seven-compilation service is already configured. Its draft contains six operations, but its service page is absent. The latest check reports no unresolved catalog links yet PARTIAL service evidence, including `DOCUMENTATION_FLOW_BYTE_BUDGET`. Zero compiler diagnostics is not complete process documentation.

The large older per-key JSON files should not automatically be called reference manifests: legacy full evidence records coexist with new envelopes. Immutable CAS does not inherently duplicate equal bytes; whole-map granularity, changing metadata and duplicate representations cause the observed growth.

## Required delivery model

1. **Source safety:** accurate per-compilation bytes, processor/profile authority and lifecycle cleanup; real public-path qualification.
2. **Snapshot data plane:** one explicit capture, granular indexed payloads, small pinned manifests, selective reads, no-op publication, grouped native work, conservative freshness and inventory.
3. **Authoring:** bounded contexts per service/topic/process; durable bindings to accepted text and diagrams; explicit requested-generation queue; no model calls during render/check; topic-local invalidation with source-link relocation handled separately.
4. **Service product:** summary, entities, ingress/egress catalog, internal process graphs and diagrams. Distinguish compiler edges, framework-derived dispatch, explicit reviewed declarations and unknown dynamic execution. A library in scope is not proof of runtime reachability.

Qualification must run the public capture/context/work/proposal/render flow against a synthetic seven-module task-router shape with HTTP submission, engine dispatch, task handler, downstream HTTP, persistence and asynchronous messaging. Scale tests should contain at least 1,300 source records and 50,000 memberships, exceed the old 64 MiB evidence threshold, and prove bounded topic IO and zero new builds/model calls/heavy writes across ten unchanged consumer sequences. No helper-only or ignored tests may stand in for those requirements.

Actual customer migration, historical deletion and production rollout remain separate explicitly authorized operations. New runs may implement safe lifecycle behavior and dry-run accounting using synthetic owned fixtures, not clean a customer cache during development.

## Immutable execution requests

Run sequentially from this repository, validating each terminal report before starting its successor:

1. `20260917-source-scope-lifecycle` — retry of source-authority closure; per-compilation source correctness, native processor qualification, owned attempt disposal, and strict supplemental report checks.
2. `20260917-docs-snapshot-store` — retry of cache normalization; absorbs production fact-index integration from multisource-v2 and the previously unexecuted oversized publication transaction scenarios.
3. `20260917-docs-topic-authoring` — new topic-context and durable authorship contract on top of the verified snapshot store.
4. `20260917-service-process-documentation` — retry of remaining multisource user-facing obligations, grounded process/entity/interface output and seven-module qualification.

Each plan is `.nessy/a2a-runs/<run-id>/plan.json`; Nessy owns the corresponding `report.json`, `status.json`, and `events.jsonl`. The requests contain 24 ordered steps in total and 30 final verification-command entries. Plan schema and shell syntax were checked before exclusive creation. All 31 pre-existing run files were preserved byte-for-byte. No implementation command was executed as part of plan creation.

The old terminal reports remain historical evidence, not prerequisites that must be edited into passing. Later new runs require successful acceptance of their new predecessor, including real production-path evidence; a validator's `VALID` result alone does not establish semantic success.
