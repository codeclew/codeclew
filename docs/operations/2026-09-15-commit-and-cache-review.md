# Commit and documentation-cache review — 2026-09-15

## Scope and status

Reviewed all 14 commits from `167f6008c97bd893dfa38581cee683ea8ba415d4` **inclusive** through pinned HEAD `5ea56176dcc205ff47c5c6075da5dc115da23acf`, covering all 23 changed files and relevant callers/consumers. Findings below refer to that committed endpoint, not later edits.

No implementation or cache cleanup was performed. A concurrent uncommitted RestClient change appeared in `crates/clew/src/java_analyzer.java` during review and was left untouched. The existing untracked files were also preserved. The architecture workspace was inspected read-only.

This document is a review report, **not an executable Nessy plan**. The requested `orchestrate-nessy-plan` skill was invoked, but its required counterpart `.nessy/skills/execute-claude-plan/SKILL.md` is absent from the Codeclew workspace. Its installed plan/report schemas are therefore unavailable. No partial `plan.json` or Nessy-owned artifacts were created. Install the paired package before preparing the immutable dispatch plan.

## Confirmed regressions

### R1 — P1: annotation registry sharding silently discards definitions

**Location:** `crates/clew/src/java_analyzer.java:162–166`, introduced in `5ea5617`.

`registry(shard)` retains the mutable map itself. `shard.clear()` therefore clears previously accumulated registry records, and subsequent insertions give every record the final shard contents. Canonical output deduplication then collapses those records.

**Reproduced:** on JDK 21, 40 annotation definitions with 1,500-character defaults produced just one registry containing seven definitions, despite successful analyzer exit. The other 33 disappeared. This can lose annotation defaults, aliases, and composed Spring mapping semantics.

**Repair direction:** copy or transfer ownership of completed shard maps; do not clear a map retained by a prior output. Test at least three shards, exact definition preservation, consumer resolution, and an individually oversized definition. The last case needs an explicit typed boundary or rejection; the current shard algorithm does not bound a single oversized entry.

### R2 — P1: class-hierarchy size reduction changes Spring route semantics

**Locations:** `java_analyzer.java:737–743,760–764`; consumer `crates/clew-framework-spring/src/lib.rs:160–171,286–291`.

Callables now carry only `List.of(typeRow(owner))`, and the enclosing `types` field is empty. The comment says hierarchy data is shared, but the shared registry contains annotation definitions, not annotated type rows. Registry reattachment cannot restore missing annotation uses or their values.

**Reproduced at analyzer output:** an implementation of an annotated interface has only its own unannotated class row, no interface mapping row, no `types`, and no incompleteness boundary. The Spring consumer obtains class mappings from these missing rows. For an interface annotated `@RequestMapping("/base")`, an implemented mapped method can lose its `/base` route prefix. Inherited controller and listener context is similarly exposed.

**Repair direction:** preserve complete hierarchy semantics, either inline or through referenced shared type rows resolved before interpretation. Add interface/superclass route-prefix, overridden-method, inherited-handler, and listener tests.

### R3 — P1: arbitrary annotation processors now run during analysis

**Locations:** `java_analyzer.java:107–123`; `java_adapter_v2.rs:226–248`.

Removing `-proc:none` enables service-discovered processors on the classpath. The new comment that `analyze()` cannot emit source files is incorrect: annotation processing executes during that call. The adapter uses the repository as cwd and supplies no isolated source/class output directories.

**Reproduced on JDK 21:** a minimal processor calling `Filer.createSourceFile("Generated")` creates `Generated.java` during `analyze()` when the repository is writable. With that directory sealed to `0555`, analysis fails with `AccessDeniedException`.

**Repair direction:** define explicit project-authorized processor behavior and disposable output isolation, with real Lombok AST-mutation and file-emitting-processor tests. Do not globally enable arbitrary processors merely to accommodate Lombok. Reinstating `-proc:none` is a possible containment measure, but is not a complete Lombok-support solution.

The writable profile also seals too late: `generation_service.rs:641–649` runs after per-compilation indexing/publication. Its mutation-allowed workspace skips original snapshot verification rather than verifying a sealed transformed state. Repair the lifecycle as transform → capture/verify transformed state → seal → index → validate → publish. Keep outputs that must be writable separate from input sources, and do not publish ready bindings before verification succeeds.

### R4 — P1: transformed facts are attributed to original source bytes

**Locations:** `generation_service.rs:599–615,757–764,841`; `java_adapter_v2.rs:320–334,375–385,521–531`; `documentation/analysis.rs:281–290,344–357,423–446`.

The writable profile hashes and analyzes transformed source files, but the ready generation retains the original repository snapshot. New `provenance` and `source_state` fields live only in the in-memory compiler index; the adapter persists its facts, not those fields. The marker serialization tests do not exercise actual generation persistence.

Documentation subsequently loads original snapshot text and slices it using transformed fact coordinates, labeling the result `EXACT_SNAPSHOT_TEXT` and linking to the original revision. A rewrite that inserts lines or changes a declaration yields incorrect snippets/links, or missing anchors when transformed coordinates exceed the original line count.

**Verification:** source-trace confirmed through persistence and documentation consumption; not an end-to-end Maven reproduction.

**Repair direction:** persist immutable transformed bytes and source-state/provenance references as generation authority. Bind identities and receipts to the source state actually indexed. Do not claim original-commit source coordinates without a verified mapping. Test transform → persist → reload → documentation, including line insertion, declaration rename, unchanged transformations, and inaccessible original-to-transformed mappings.

### R5 — P1: generated output can be written successfully but refused on the next command

**Locations:** `documentation/render.rs:2090–2096`; `documentation/bindings.rs:348–351,445–456`; `documentation/history.rs:82,102–107,302`.

The changed publication writer allows 128 MiB records. Bindings and history readers still use 64 MiB, and output verification treats any generated file above 64 MiB as edited/removed.

**Failure:** a valid 70 MiB generated record is published successfully, then an unchanged baseline is rejected on reload, render, refresh, or historical inspection. Oversized bindings can also block check. This is a source-confirmed threshold mismatch; no 70 MiB integration fixture was executed during review.

**Repair direction:** establish a single deliberate portable-record bound across writers, readers, validators, and history, or revert the write-limit increase until storage is normalized. Add boundary tests including publish/reload/verify/refresh/history, not only serialization. Raising limits alone does not solve duplication.

### R6 — P2: the advertised CLI profile is only wired through documentation capture

**Locations:** `generation_service.rs:237–246`; `main.rs:2274–2318`; `context_v2.rs:684`; `working_tree_change_service.rs:166`; `documentation/analysis.rs:270–274`.

The profile is admitted and documented as CLI-usable, but public `ensure_session_generation()` always forwards `writable_then_seal=false`. Committed context opening also drops the selected profile when opening its session. Documentation capture is the only caller forwarding the new gate.

**Failure:** context opening with `java-17plus-maven-writable-then-seal` can pass profile admission and still run the in-place Maven plugin against read-only sources, reproducing the failure the profile promises to solve.

**Repair direction:** carry the selected profile as durable session/generation authority, not a docs-only boolean. Test real CLI admission through generation and verify the original read-only profile remains read-only. This finding is source-trace verified, not a CLI runtime reproduction.

### R7 — P2: raw analyzer stderr bypasses private diagnostic opt-in

**Location:** `java_adapter_v2.rs:250–269`.

Every analyzer failure or output-limit breach unconditionally prints its last 40 raw stderr lines. There is no debug-output opt-in or redaction, and a line count is not a byte limit. Processor failures and JVM diagnostics can contain private paths, source text, or sensitive build values.

This bypasses the bounded private-output contract introduced for Maven diagnostics. The dedicated Maven diagnostics implementation itself passed its focused tests; the unsafe path is the new unconditional analyzer logging.

**Repair direction:** remove raw public stderr output, keep only allowlisted public metadata, and require explicit private bounded capture for raw diagnostics. Add a fake-analyzer sentinel test proving default terminal output, portable evidence, and support summaries never contain the raw placeholder.

## Observed architecture-workspace storage

Measurements are from the user's `arch-kasko` workspace on 2026-09-15. Sizes below use allocated-file accounting (`du`/`st_blocks`), not exclusive physical APFS extent accounting. JSON byte sizes are stated separately where useful. Private repository contents were not copied into this report.

| Area | Allocated size | Contents |
|---|---:|---|
| Entire architecture workspace | 328.19 MiB | 315 files |
| `.codeclew/cache` | 245.64 MiB | Seven JSON files, no subdirectories |
| `docs/generated` | 80.73 MiB | Four retained publication generations, 89 files |

There are two catalogued services. Eleven local binding records exist, but they are tiny and do not explain the footprint.

### Evidence JSON duplication

- Three copies of one service's evidence are **55,527,079 bytes each**.
- Two differently keyed copies are byte-identical by SHA-256. The third differs in exactly one `MODULE_SCOPE` observation's derivation digest and consequent digest.
- Three copies of the other service are approximately 0.87 MB, 0.46 MB, and 0.46 MB.
- `latest-check.json` is **89,182,414 bytes**. Its embedded services exactly repeat the latest service-cache contents, approximately **55.99 MB** of serialized data.
- Its dependency map repeats every service observation: **28,666 observations**, approximately **33.20 MB** of additional JSON.
- The seven cache files observed span approximately 10:06–16:08 local time on the measurement date.

### Publication duplication

- The latest two `bindings.json` files are **25,118,998 bytes each**. Their parsed objects differ only in `outputHashes`; evidence and narrative content match.
- Each retains **28,304 source records**, approximately **22.73 MB**, exactly the union of all sources in the latest two service analyses.
- The 600 retained observations directly reference only **521 distinct source IDs**. This is evidence of excess breadth, **not** a safe deletion list: fragments, narratives, accepted versions, and other references must also be included in reachability.
- One service's 28,091 source records represent 646 files, 11,083 distinct line spans, and 8,297 distinct texts. The source text itself totals approximately 2.78 MB; repeated record metadata, identities, URLs, and spans account for much of the rest.

No symlinks or repository-tree copies were found inside `.codeclew/cache`. Sampled duplicate files have distinct inodes and link count one, so they are not hardlinks. APFS clone sharing was not established.

The current private Codeclew home's workspaces, generations, runs, attempts, temporary state, and dependency-cache directories were empty at observation time. Its approximately 317.71 MiB footprint was predominantly runtime distribution storage. This does not prove source materializations were never created transiently. Installed-release retention is another separate footprint and is not evidence of per-doc-command repository copying.

## Current cache behavior and root causes

These structural duplication issues mostly **predate the reviewed commits**. Recent changes increased limits and changed producer identities; they did not introduce the entire storage design.

### 1. Keyed service files are written, not reused as a capture cache

`documentation/analysis.rs:209–259` computes a key from revision, service declaration digest, extractor identity, language, and runtime key. It opens a new session with `ModelCachePolicy::NonCacheable`, performs extraction, and writes the keyed JSON. No corresponding keyed-cache reader was found.

Identical keys reuse/overwrite the same filename; there is **not a new random filename on every invocation**. Changed runtime/configuration/revision identity leaves another file, even if resulting evidence is byte-identical. The module derivation digest hashes complete `analysis.rs` and `modules.rs` source bytes (`documentation/modules.rs:276`), so a diagnostics-only edit can change a module observation.

Existing keys are not a sufficient authorization for blindly enabling reuse: external build/settings/dependency authority and actual source-state identity must be validated. Non-cacheable behavior should remain explicit when that authority cannot be established.

### 2. Check, render, and refresh all invoke capture

| Operation | Current local-service behavior |
|---|---|
| `docs check` | Runs documentation check and semantic capture |
| `docs render` | Calls `check::run` again before rendering (`render.rs:1528`) |
| `docs refresh` | Calls `check::run` again before refreshing statuses (`status.rs:209`) |
| `docs context --refresh` | Calls check again (`cli.rs:483`) |
| Context without refresh | Can read `latest-check.json`; this does not establish newly captured source freshness |
| Selected portable evidence / coordinator mode | Can avoid local compiler execution under their own admission rules |

Thus the user's repeated-build concern is supported by the code, although the observed local cache stores evidence JSON rather than source checkouts. Capture uses managed abort/GC for its session (`analysis.rs:232`); these operations do not reclaim architecture-local cache files or publication history.

### 3. Latest check duplicates observations internally

`check.rs:231–234` clones all service observations into `Check.dependencies` while retaining them inside each service. Saving the full check (`check.rs:732–741`) serializes both representations. `latest-check.json` is overwritten, rather than accumulated under new filenames.

### 4. Publications retain complete payloads and too many sources

`render.rs:1116–1123` filters observations to publication references, but `:1158` retains `checked.sources()` wholesale. Subsequent publication also merges old sources wholesale (`:1787–1792`). Some per-operation page payloads additionally repeat page-level source/contract maps (`:1940–1953`).

Publication identities are deterministic, but render (`render.rs:1893–1895`) and refresh (`status.rs:213–215`) use different identity envelopes. Changed identities write complete bundles, not shared heavy-object references. Identical operation inputs can reuse a bundle; it is inaccurate to say every invocation always creates a new one.

Old publications are retained intentionally. History only refuses enumeration beyond 4,096 snapshots and refers to an external retention policy (`history.rs:136`); no implemented docs-specific retention/GC path was found.

### 5. Existing CAS GC is not a solution for these local JSON files

The generic CAS provides content/schema identity and batch deduplication within one state authority (`cas.rs:371–420`). Its GC scans private managed roots (`cas.rs:1422–1438`) and reclaims eligible CAS objects/packs (`:1641–1708`). It does not manage architecture-local `.codeclew/cache` or `docs/generated` publications.

## Recommended remediation boundaries

The following are review recommendations to use when the Nessy counterpart is installed, not a dispatchable plan or authorization to execute commands.

1. **Correctness and containment first:** address R1–R7, with a complete transform/seal/index/persist/consume contract and realistic annotation-processor tests. Keep cheap logging/map fixes separate from the architectural persistence changes where possible.
2. **Normalize evidence:** immutable content-addressed source, observation, and service-evidence objects; small current-check/service references; no duplicate full dependency maps on disk. Keep producer diagnostics separate from semantic objects.
3. **Establish sound reuse:** a validated capture index bound to repository/snapshot, service/profile/compilation, producer/adapter/toolchain, settings/dependencies, and transformed input/output authority. Attempt UUIDs, temporary paths, and timestamps must not create semantic identities. Report why a capture is not reusable rather than weakening admission.
4. **Separate commands from extraction:** render should consume an admitted fresh evidence vector; refresh should update status from validated evidence without repeating unchanged producers. A prior check is reusable only while its complete authority remains valid.
5. **Normalize publication references:** retain the full referenced source/observation closure, including historical accepted claims, rather than every source from every checked service. Preserve portable export and existing-version compatibility through explicit schemas/migration.
6. **Add docs-specific lifecycle/accounting:** read-only inventory and dry-run reachability first; explicit publication retention; active leases; byte/object budgets; failure/cancellation cleanup. Only delete owned, unreachable state. Do not treat historical publications as disposable caches or manually delete private runtime objects.
7. **Share immutable artifacts, not mutable source workspaces:** dependency/build reuse needs verified artifact identity and concurrency ownership. Never make hardlinked CAS source files writable or allow one attempt's cleanup to delete another attempt's workspace.

Useful acceptance criteria for the eventual executable plan:

- Unchanged two-service `check → render → refresh` creates no duplicate heavy evidence objects and invokes no additional producer after a still-valid admitted capture.
- Changing one service invalidates only that service where authority permits; changed profiles, settings, dependencies, toolchains, and transforms invalidate correctly.
- Published source snippets correspond exactly to persisted indexed source state, with truthful revision/provenance links.
- Annotation definitions survive multiple shards, hierarchy mappings retain their semantics, and processors cannot write to immutable input trees.
- Writer/read/verification/history limits agree at the boundary and one byte beyond it.
- GC preserves current/retained historical evidence, user-authored inputs, portable packages, and active attempts; concurrent capture and crash recovery cannot delete another run's state.
- Ten repeated unchanged command sequences have stable heavy-object byte/count totals, excluding intentionally retained bounded operational logs.

## Verification performed

- `git diff --check 167f6008^ 5ea5617`: passed.
- `python3 -I -S scripts/check_english_content.py`: failed on the existing Cyrillic diagnostic example at `.nessy/skills/auto-skill-debug-codeclew-maven-admission/SKILL.md:53`, a file changed in the reviewed range. Translate or replace that example with an English placeholder; it was left unchanged during this read-only review.
- Pinned Rust `cargo fmt --all --check`: passed.
- `cargo test --offline --locked -p clew --lib --no-run`: passed.
- Focused tests for repository snapshots, Maven diagnostics, transformed markers, profile gates, Spring entrypoints, documentation analysis/check/evidence packaging, Java adapter, and Kotlin adapter: **54 distinct tests passed**.
- One test failed: `kotlin_adapter_v2::tests::k24_real_worker_cold_then_product_unchanged_skips_index_files`, with `WORKER_JDK_UNSUPPORTED` because the test environment did not discover a compatible JDK 21. Eleven other enabled Kotlin adapter tests passed. This is not proof of a code regression. A local JDK 21 was subsequently identified under jenv and used by the separate analyzer reproductions; the worker test was not rerun with it configured.
- Six relevant integration/qualification tests remained ignored by default, including Maven documentation acceptance and Java compiler qualification.
- Render/status module-specific unit-test filters matched zero tests; these were not counted as passing coverage.
- Three lightweight JDK 21 analyzer reproductions demonstrated shard loss, missing hierarchy context, and processor output during `analyze()`.
- No full CI gate, real-project rebuild, benchmark, network action, deployment, publication, or destructive cleanup was performed.
