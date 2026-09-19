# Source-scope recovery after the failed 2026-09-17 run

## Observed failure

`20260917-source-scope-lifecycle` is terminal **failed**, not input-required. Its timestamps span 09:45:04Z–09:59:54Z (14m50s); its plan allowed six hours. The report says steps 2–6 were not executed because the multi-file implementation could not be completed reliably in that session. It does not identify a denied permission, unavailable native dependency or an architectural contradiction. This does not prove no internal resource constraint existed; it means the artifacts give no concrete one to resolve.

Only the checker, checker tests and an obligation map were created. The report records exit code 101 for four commands explicitly described as not executed. Those are not measured exit codes. The installed validator returns VALID for the failed artifacts, which is not implementation acceptance.

The original plan combined source projection, source-state integrity, annotation processing, temporary ownership and public CLI qualification. Recovery separates these into bounded reviewable deliveries instead of requiring every local change to prove the whole pipeline.

## Preserve, then repair the existing preparation

Keep the existing Python checker and obligation map; do not start another checker project. Fix only concrete defects:

- Test setup deletes a fixed `.nessy/a2a-runs/synth-run` path. Replace this before running it with unique owned fixture directories.
- CLI accepts only a positional argument, whereas the existing downstream plans use `--run`. Support the alias and add explicit `--require-completed` prerequisite semantics.
- The installed event schema uses `properties`/`required` without an explicit object `type` inside conditionals. The current mini-validator skips those assertions; evaluate them correctly and propagate unsupported schema errors.
- Structural validity and successful execution are different. A failed report may honestly omit unexecuted commands. It must never be made complete by inventing exit codes or command events.

Old terminal artifacts remain unchanged even when invalid. New reports contain actual execution evidence only.

## Narrow execution sequence

1. `20260917-source-a-report-guard`: repair existing checker/test ownership and completion gate; no Rust or Maven.
2. `20260917-source-b-scoped-readers`: per-compilation documentation source projection and the direct public source-reader callers; local regression tests.
3. `20260917-source-c-index-integrity`: compare persisted bytes with indexed source state and bind/revalidate semantic identity; local production-seam tests.
4. `20260917-source-d-owned-disposal`: owned sealed attempt disposal using file handles and explicit errors; local lifecycle tests.
5. `20260917-source-e-native-processors`: real offline processor qualification using existing fixtures; no unrelated source/cache redesign.
6. `20260917-source-f-cli-acceptance`: integrate the preceding changes through the admitted public native CLI; this is the full source-lifecycle gate.

Each successor requires accepted completion of the preceding new run. The failed parent is historical context, never a successful prerequisite. Local tests in B–D are sufficient for those slices but are explicitly not substitutes for E/F's native integration acceptance.

After F, use the replacement downstream plans, not the original dependency chain:

- `20260917-docs-snapshot-store-v2`
- `20260917-docs-topic-authoring-v2`
- `20260917-service-process-documentation-v2`

These preserve the original product requirements and correct predecessor IDs/completion-check commands. They do not reduce source qualification or claim the larger documentation work has been implemented.

## Execution discipline

- Stay within the slice: inspect named symbols and their direct callers, add one failing regression, implement its fix, run focused step tests, then finalize verification. A new missing test target is work to implement, not an environmental failure.
- Test/red-green commands run in the implementation phase. Do not enter terminal verification until required implementation/tests exist. Final verification failure is terminal under the executor contract.
- Concrete missing native artifacts, permissions or incompatible APIs justify input-required with the exact observed evidence. The anticipated size of other slices does not block the current one.
- Do not change old plan/status/events/report files, customer repositories, private caches, installed skill schemas or secret settings. Do not download dependencies or commit changes.
- At least one launcher-based end-to-end test remains mandatory in F. Helpers and synthetic DTOs can prove local invariants but cannot be relabeled as native capture evidence.
- Actual customer JDK17 rollout and historical cleanup remain separate from synthetic JDK21 qualification.

Plans are immutable requests. Nessy owns each new run's status, events and report. This document is a routing and implementation guide, not evidence that the planned fixes passed.

## Slice A implementation note (20260917-source-a-report-guard)

The existing report checker was repaired in place; no installed schema or new
checker was introduced.

- `scripts/validate_nessy_acceptance.py`:
  - `MiniSchema._check_keywords` now raises on any unsupported schema keyword,
    propagating from conditional/oneOf/allOf sub-validators independently of
    instance branches.
  - `MiniSchema.validate` evaluates `properties`/`required` on dict instances
    even when `type` is omitted (used by the installed event schema inside
    `if/then`).
  - `validate_cross` checks event run IDs, a balanced ordered
    started/step-start/step-end/verification/terminal lifecycle, completed-only
    step statuses and `blockingQuestion: null`, and enforces exact ordered
    verification coverage for completed runs while allowing failed/input-required
    reports to list only executed commands (never inventing missing results).
    Optional iteration step commands are no longer required to have run.
  - CLI accepts a positional ID/path, a `--run <id>` alias, and an explicit
    `--require-completed` gate; any non-completed status returns nonzero under
    the gate even when structurally valid.
- `scripts/test_validate_nessy_acceptance.py`: 26 tests use unique test-owned
  `mkdtemp` roots (no fixed-name destructive setup), plus a pre-existing
  sentinel and two simultaneous isolated fixtures. Coverage now includes
  conditional-schema required failure, event runId mismatch, unbalanced
  lifecycle, completed-with-failed-step, true unknown schema keyword,
  failed-with-omitted-commands structural acceptance, both CLI forms,
  completion gating and safe-path rejection.

Old terminal reports were not modified. The completed-run gate is now usable as
a prerequisite check for later slices.

## Slice B implementation note (20260917-source-b-scoped-readers)

Documentation source projection now selects source per compilation scope.

- `crates/clew/src/documentation/analysis.rs`:
  - `resolve_scope_key` is the one canonical scope-key resolver: an object's
    `compilation` field is authoritative and must name a registered compilation;
    a nonempty legacy string is accepted; absent scope yields empty only for a
    single admitted compilation; malformed/unknown scope is an explicit error,
    never an empty fallback.
  - `CompilationSource` (pub(crate)) holds `contents`/`transformed` keyed by
    canonical scope plus a `contracts` table loaded once from the original
    snapshot. `project` remains a legacy wrapper delegating to the new
    `project_scoped`.
  - `capture_session` no longer reads the set-level first-present
    `ready.transformed_source` for every scope; it loads each
    compilation's `transformed_source`, keeps the original snapshot only for
    read-only occurrences and contract files, prunes wanted paths per scope
    against the total evidence budget, and calls `project_scoped`.
  - Six new enabled regressions cover object scopes sharing a path with
    different bytes, mixed readonly/transformed authority, malformed and
    unregistered scope errors, the single-scope empty-key legacy path, identical
    cross-scope declarations without false ambiguity, and independent contract
    loading. Legacy `project` callers in kotlin/check/generation_service compile
    unchanged.
- Public/native lifecycle qualification remains with slices E/F.

## Slice C implementation note (20260917-source-c-index-integrity)

Transformed source persistence and reopen now verify persisted bytes against
the indexed source state.

- `crates/clew/src/generation_service.rs`:
  - Added `TRANSFORMED_SOURCE_FILE_SCHEMA` and helpers
    `is_safe_relative_source_path`, `transformed_source_state_after`, and
    `transformed_paths_match_source_files`. `sourceState.after` is the
    authoritative digest map; `before`/`changedFiles` remain attribution.
  - `persist_transformed_source` now requires the after-map to exactly match the
    admitted source files, reads each file once, and compares its digest to the
    after-map BEFORE publishing the manifest reference. Missing/extra/malformed
    state or bytes that changed after index-state construction fail closed;
    unsafe relative paths are rejected.
  - `load_transformed_source` now requires the integrity source state, verifies
    per-file schema, safe relative paths, exact after-map membership, and byte
    hashes. A legacy manifest without source state, a manifest lacking the
    after-map, a state/file membership mismatch, or a hash mismatch is rejected
    rather than silently promoted to trusted transformed evidence.
  - Existing placeholder-hash fixtures were corrected to real digests. Eleven
    new enabled regressions cover readonly-vs-no-op-writable key separation,
    two indexed after-states (incompatible one rejected, previous binding
    unchanged), missing/extra paths, bytes-changed-after-index, unsafe paths,
    legacy/no-after/mismatch/extra-file/unsafe-path reopen rejection, and
    deterministic stable-state round-trip.
- `crates/clew/src/java_adapter_v2.rs` inspected; `transformed_index_marker`
  and adapter tests unchanged and passing. `final_generation_key` was retained
  (writable-flag separation proven by test) rather than redesigned.
- Public/native lifecycle qualification remains with slices E/F.

## Slice D implementation note (20260917-source-d-owned-disposal)

Sealed attempt disposal was proven on the existing path and an explicit
consuming dispose API now surfaces cleanup failures.

- `crates/clew/src/state.rs`:
  - Added `ManagedTemporaryDirectory::close` (consuming, `Result`): pins the held
    directory identity, compares it against the entry currently named at the
    parent path, and refuses (typed error, Drop suppressed) to remove an
    unrelated same-name replacement. On success it marks the tree removed so the
    Drop fallback does not double-remove. Drop remains best-effort and
    non-panicking.
  - Added `entry_identity` (dev/ino via `fstatat` AT_SYMLINK_NOFOLLOW).
  - Three new sealed-disposal tests (ten cycles leave no entries, external
    symlink sentinel untouched, root-replacement removal from the held moved
    inode) prove `open_private_child_directory` already restores 0700, so no
    chmod/reseal of doomed workspaces is needed.
- `crates/clew/src/kotlin_adapter_v2.rs`: `ProjectNativeKotlinWorkspace`'s owned
  attempt is now `Option<ManagedTemporaryDirectory>` and `finish` explicitly
  disposes it after unmount and verification, surfacing a removal failure when no
  primary error exists; on an earlier primary error the best-effort Drop still
  cleans the attempt while preserving the original error. Five new finish tests
  cover owned-root disposal, injected removal refusal, early-unwind Drop
  cleanup, and verification/unmount failure that preserves the primary error and
  cleans the attempt.
- Public/native lifecycle qualification remains with slices E/F.

## Processor execution contract (slice E)

Admitted annotation-processor behavior during analysis is now explicit and
tested on the real offline native fixture; this replaces any blanket
"processors never rerun" assumption.

Allowed stages:
- **Native compilation / model extraction**: processors may run normally as
  configured by the project (e.g. Maven `annotationProcessorPaths` +
  `annotationProcessors`) while the build rewrites source.
- **Analyzer execution**: an annotation processor runs inside the analyzer only
  when it is **explicitly admitted** by the model — an admitted processor name
  (from `-processor`/`annotationProcessors`/`annotationProcessorPaths`) AND the
  writable-then-seal profile is active. Its emitted sources/classes are isolated
  to a disposable generated root, never the sealed repository. When nothing is
  admitted, the analyzer runs `-proc:none`, so arbitrary classpath-discovered
  processors never run or mutate.
- **Admitted processor options** (`-A...` from the model's `compiler_options`)
  are surfaced to the analyzer and only reach an admitted processor; they
  participate in model identity so an option change invalidates authority.

Provenance honesty:
- Indexed source coordinates refer to the persisted file bytes. Members generated
  by a processor (e.g. Lombok getters) are resolved semantically and are never
  claimed to have textual spans in the original/transformed file; the getter text
  is not fabricated into the source bytes.
- A separate text-transform stage runs before seal if textual rewriting is
  required; no delomboked text is invented from AST members.

This is processor qualification for the real fixture; mandatory public CLI
acceptance remains with slice F.

## Aggregate source-acceptance matrix (slice F)

Every original source-scope requirement maps to a concrete enabled test and a
truthful report. No requirement relies on an unexecuted substitute.

| Requirement | Evidence | Report |
|---|---|---|
| Report/completion gate honesty; no invented exit codes | `scripts/validate_nessy_acceptance.py` + `scripts/test_validate_nessy_acceptance.py` (26 tests) | A |
| Per-compilation source selection (object scope, single-scope legacy, contract isolation) | `documentation::analysis` tests (resolve_scope_key, project_scoped, 6 regressions) | B |
| Persisted/reopened transformed bytes match indexed state; reject before publication | `generation_service::tests` (persist/load integrity, 11 regressions incl. two-after-states) | C |
| Sealed attempt disposal proven + explicit close()/finish dispose | `state::tests` (sealed-drop, close) + `kotlin_adapter_v2::tests` (finish disposal/failure) | D |
| Native admitted processors, `-A` options, unadmitted never run, sanitized errors | `source_authority_regressions` (9 native tests, synthetic counter processor) | E |
| Public CLI: actual `./clew` source-bootstrap launch + scope-correct capture/read/reopen | `practical_source_lifecycle` (`source_bootstrap_launches_real_product`, `native_maven_capture_read_reopen_returns_scope_correct_authority`) | F |

Disposal and post-index rejection are enforced at their production seams in
slices C/D and observed end-to-end through the F public lifecycle. Oversized
render and live customer rollout remain explicitly downstream and unclaimed.

## Slice F implementation note (20260917-source-f-cli-acceptance)

Final public source gate.

- `crates/clew/tests/practical_source_lifecycle.rs`: new enabled integration
  target reusing the admitted managed-dispatch helpers from managed_cli.rs.
  `source_bootstrap_launches_real_product` launches the `./clew` source
  development launcher through the supported bootstrap and asserts the real
  product runs. `native_maven_capture_read_reopen_returns_scope_correct_authority`
  drives a real offline docs init/bind/check through the compiled binary with
  admitted runtime FDs and asserts scope-correct authority
  (`EXACT_SNAPSHOT_TEXT`) and a deterministic close/reopen in the same
  test-owned state. All fixtures are synthetic; no customer build or
  private-state editing.
- Post-index byte-mutation rejection is enforced by slice C at the production
  persist seam (persist_transformed_source rejects before publication, previous
  binding unchanged) and explicit disposal refusal by slice D
  (ManagedTemporaryDirectory::close identity check); F observes the successful
  public lifecycle through the product path.
- Aggregate matrix above maps every original requirement to a passing test.
  Oversized render remains downstream.
