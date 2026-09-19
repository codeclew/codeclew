# Maven writable-then-seal admission profile — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a separately opt-in `java-17plus-maven-writable-then-seal` profile so Maven admission materializes source writable, runs the ordinary Maven lifecycle (allowing in-place transform/codegen plugins), records the transformation boundary, then seals and indexes only the transformed workspace.

**Architecture:** Profile-gated branch on the existing materialization/build/index path. On the new profile: make worktree files writable before `extract_maven`, run the build unchanged, seal the materialization read-only on success, and index source content re-hashed from the sealed transformed worktree. Evidence carries `provenance: TRANSFORMED_WORKSPACE` + `sourceState` (before/after digests + changedFiles) on the **JavaCompilerIndex** (not the `JavaProjectModel`, which stays schema-stable).

**Tech Stack:** Rust (Codeclew crate `crates/clew`), `std::process::Command`, `walkdir`, Maven. Tests via `cargo test --locked -p clew`.

**Spec:** `docs/superpowers/specs/2026-09-14-maven-writable-then-seal-design.md`.

---

## Repository context (verify before editing)

- Profile is available on the session: `SessionAuthority.profile_id` (`crates/clew/src/session.rs:81`).
- The Java generation path is `ensure_java_generation_set` in `crates/clew/src/generation_service.rs:~554-601`.
  - `prepare_language(...)` creates the materialization (`attempts/<id>/repo`), seals it (`seal_tree`), then `mount_project_derived_state` makes dirs writable (`0o700`) and symlinks `target/`, `build/`, `.gradle/` to writable derived dirs. Files stay `0o400`/`0o500` → the blocker.
  - Loop calls `extract_java_model_with_settings_and_diagnostics(workspace.repository(), ...)` (runs Maven) then `java_source_content_digests(...)`.
  - `workspace.finish()` at the end.
- `effective_java_sources(snapshot)` (`generation_service.rs:784`) builds the source→CAS-content map from the immutable snapshot.
- `java_source_content_digests(store, &sources, &model)` (`generation_service.rs:822`) hashes those CAS contents, erroring if a model source is missing from the snapshot.
- `seal_tree` is private in `crates/clew/src/repository_snapshot.rs:1746`; expose as `pub(crate)`.
- Docs profile validation: `crates/clew/src/documentation/store.rs:431`.

---

### Task 1: Register the new profile

**Files:**
- Modify: `crates/clew/src/documentation/store.rs:427-432`
- Modify: `crates/clew/src/operations.rs` (support matrix for `--profile`; search for `java-17plus-maven-read-only`)
- Test: `crates/clew/src/documentation/store.rs` (existing tests around profile validation)

- [ ] **Step 1: Add the profile to the docs profile allow-list**

In `documentation/store.rs:431` change:

```rust
"java-17plus-maven-read-only" | "java-17plus-gradle-read-only"
```

to:

```rust
"java-17plus-maven-read-only"
| "java-17plus-gradle-read-only"
| "java-17plus-maven-writable-then-seal"
```

- [ ] **Step 2: Register in the support matrix**

Find the matrix/catalog rows that list `"java-17plus-maven-read-only"` (grep `java-17plus-maven-read-only` under `crates/clew/src`, ignoring test fixtures). Add a sibling row with `"profileId":"java-17plus-maven-writable-then-seal"` mirroring the maven-read-only row (same language `java`, same `profile.kind` read-only analysis semantics, same `compilation` acceptance), plus `"writableThenSeal": true` metadata if the matrix carries per-profile flags. If the matrix is fully data-driven (a table/array), add the row there; otherwise add the match arm in the profile parser.

- [ ] **Step 3: Run the crate tests**

Run: `cargo test --locked -p clew --lib 'documentation::store::' -- --test-threads=1`
Expected: PASS (profile validation accepts the new profile; existing tests unchanged).

- [ ] **Step 4: Commit**

```bash
git add crates/clew/src/documentation/store.rs crates/clew/src/operations.rs
git commit -m "feat(java): register java-17plus-maven-writable-then-seal profile"
```

---

### Task 2: Expose seal helper + add writable-files pass

**Files:**
- Modify: `crates/clew/src/repository_snapshot.rs:1746` (make `seal_tree` `pub(crate)`)
- Modify: `crates/clew/src/repository_snapshot.rs` (add `make_files_writable`)
- Test: `crates/clew/src/repository_snapshot.rs` (new unit test)

- [ ] **Step 1: Make `seal_tree` `pub(crate)` and add `make_files_writable`**

At `repository_snapshot.rs` near `seal_tree` (currently `fn seal_tree(...)`), change to `pub(crate) fn seal_tree(...)` and add:

```rust
/// Make every regular file under `root` writable (dirs are left as-is; the
/// derived-state mount already restores dirs). Used only by the writable-then-
/// seal profile so in-place transform/codegen plugins can rewrite source.
pub(crate) fn make_files_writable(root: &Path) -> Result<(), ClewError> {
    let mut entries = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(internal)?;
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.path().components().count()));
    for entry in entries {
        if entry.file_type().is_file() {
            fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o600))
                .map_err(io_error)?;
        }
    }
    Ok(())
}
```

Ensure `std::path::Path` is imported in `repository_snapshot.rs` (add if absent).

- [ ] **Step 2: Write the failing test**

Add to `repository_snapshot.rs` tests:

```rust
#[test]
fn make_files_writable_then_seal_restores_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("src/main/java")).unwrap();
    fs::write(root.join("src/main/java/A.java"), "class A {}".unwrap());
    make_files_writable(root).unwrap();
    let mode = fs::metadata(root.join("src/main/java/A.java")).unwrap().permissions().mode();
    assert_eq!(mode & 0o222, 0o222, "file should be writable after make_files_writable");
    seal_tree(root).unwrap();
    let sealed = fs::metadata(root.join("src/main/java/A.java")).unwrap().permissions().mode();
    assert_eq!(sealed & 0o222, 0, "file should be read-only after seal_tree");
}
```

(`seal_tree` is `pub(crate)`; the test lives in the same crate so it can call both.)

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --locked -p clew --lib 'repository_snapshot::tests::make_files_writable_then_seal_restores_read_only' -- --test-threads=1`
Expected: compile fails (function not yet present) → after adding the function, test PASSES (both assertions hold).

- [ ] **Step 4: Commit**

```bash
git add crates/clew/src/repository_snapshot.rs
git commit -m "feat(java): expose seal_tree and add make_files_writable helper"
```

---

### Task 3: Writable phase + post-build seal in the Java generation path

**Files:**
- Modify: `crates/clew/src/generation_service.rs:554-601` (`ensure_java_generation_set`)

Gate on `session.profile_id == "java-17plus-maven-writable-then-seal"`.

- [ ] **Step 1: Add a profile constant and the writable/seal branch**

Add near the top of `generation_service.rs`:

```rust
pub(crate) const JAVA_MAVEN_WRITABLE_THEN_SEAL_PROFILE: &str =
    "java-17plus-maven-writable-then-seal";
```

In `ensure_java_generation_set`, after `let workspace = ProjectNativeKotlinWorkspace::prepare_language(...)` and before the compilation loop, insert:

```rust
let writable_then_seal = session.profile_id == JAVA_MAVEN_WRITABLE_THEN_SEAL_PROFILE;
if writable_then_seal {
    crate::repository_snapshot::make_files_writable(workspace.repository())?;
}
```

The loop is unchanged (`extract_java_model_with_settings_and_diagnostics(...)` runs the build; plugins can now write source).

- [ ] **Step 2: Seal on success; skip seal on failure**

Replace the tail of `ensure_java_generation_set` (the current `workspace.finish()?;` line) with:

```rust
    let results = (|| {
        let mut results = Vec::with_capacity(session.compilations.len());
        for compilation in &session.compilations {
            // ... existing per-compilation body (component, settings, model,
            // source_content_digests, results.push(ensure_java_generation(...)))
        }
        Ok(results)
    })();
    let results = results?; // on Err the materialization is left unwritable; disposable/GC
    if writable_then_seal {
        crate::repository_snapshot::seal_tree(workspace.repository())?;
    }
    workspace.finish()?;
    let ready = assemble_ready_set(session, snapshot_object, results)?;
    write_ready_set(state, binding_path, &ready)?;
    Ok(ready)
```

Keep the per-compilation body identical to today (the plan references it; do not reorder). If a build step returns `Err`, the `?` returns early and we never seal.

- [ ] **Step 3: Run the crate tests**

Run: `cargo test --locked -p clew --lib 'generation_service::' -- --test-threads=1`
Expected: PASS (existing tests unaffected; new profile path is additive).

- [ ] **Step 4: Commit**

```bash
git add crates/clew/src/generation_service.rs
git commit -m "feat(java): writable-then-seal phase in java generation path"
```

---

### Task 4: Index source content from the sealed transformed worktree + record boundary

**Files:**
- Modify: `crates/clew/src/generation_service.rs:784-860` (`effective_java_sources`, `java_source_content_digests`)

- [ ] **Step 1: Add a transformed-source reader**

Add near `java_source_content_digests`:

```rust
/// Re-hash the .java files present in the (sealed) worktree, relative to
/// `repository`. Used only for the writable-then-seal profile, where the build
/// may rewrite source in place; the returned map is the "after" boundary.
fn transformed_java_source_digests(repository: &Path) -> Result<BTreeMap<String, String>, ClewError> {
    let mut out = BTreeMap::new();
    for entry in walkdir::WalkDir::new(repository).follow_links(false).into_iter() {
        let entry = entry.map_err(|e| io_error(std::io::Error::other(e)))?;
        if !entry.file_type().is_file() || !entry.path().extension().is_some_and(|e| e == "java") {
            continue;
        }
        let bytes = fs::read(entry.path()).map_err(io_error)?;
        out.insert(
            entry.path().strip_prefix(repository).map_err(internal)?.to_string_lossy().into_owned(),
            digest_hex(&bytes),
        );
    }
    Ok(out)
}
```

Where `digest_hex` is the crate's existing content-hash helper (grep `fn digest_hex` / `canonical::hash_bytes`; use whichever returns a hex/`sha256:` string consistent with `source_content_digest`). If `source_content_digest` returns `sha256:<hex>`, make `transformed_java_source_digests` return the same format by hashing bytes with the same helper.

- [ ] **Step 2: Thread the "after" digests into the per-compilation result**

In `ensure_java_generation_set`'s loop, after `let model = extract_java_model_with_settings_and_diagnostics(...)`, compute the digests for the profile:

```rust
let source_content_digests = if writable_then_seal {
    transformed_java_source_digests(workspace.repository())?
} else {
    java_source_content_digests(store, &sources, &model)?
};
```

Keep the immutable `sources` map (from `effective_java_sources(snapshot)`) available as the "before" boundary. Pass both `source_content_digests` (after) and `sources` (before) into `ensure_java_generation`.

- [ ] **Step 3: Update `ensure_java_generation` signature to receive before/after for the marker**

Change `ensure_java_generation(...)` to also take `before_digests: &BTreeMap<String, String>` (the CAS `sources` content map) and the `writable_then_seal: bool`. For the writable-then-seal profile compute `changedFiles = paths where before != after`, and pass them onward to the index-marker step (Task 5). For the read-only profile pass empty/None.

- [ ] **Step 4: Run tests**

Run: `cargo test --locked -p clew --lib 'generation_service::' -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/clew/src/generation_service.rs
git commit -m "feat(java): index transformed workspace source and record boundary"
```

---

### Task 5: Add transformed-workspace provenance to the compiler index evidence

**Files:**
- Modify: the `JavaCompilerIndex` type (grep `struct JavaCompilerIndex` in `crates/clew/src/java_adapter_v2.rs` or `java_compiler_index` module)
- Modify: `crates/clew/src/generation_service.rs` (`build_java_compiler_index` call site, ~L717)
- Test: unit test asserting the marker/sourceState is present

- [ ] **Step 1: Add provenance/sourceState fields to `JavaCompilerIndex`**

Add optional, serialized fields to `JavaCompilerIndex` (derive `Serialize`/`Deserialize`, `#[serde(default, skip_serializing_if = "Option::is_none")]` so old indices stay readable):

```rust
pub provenance: Option<String>,                 // Some("TRANSFORMED_WORKSPACE") for the new profile
pub source_state: Option<serde_json::Value>,    // { before, after, changedFiles }
```

Set both to `None` for the existing read-only path (defaults preserved).

- [ ] **Step 2: Populate at the build call site**

At `build_java_compiler_index(repository, &model, &source_content_digests)` in `generation_service.rs`, add the before/after arguments and set `provenance = Some("TRANSFORMED_WORKSPACE")` and `source_state = Some({before, after, changedFiles})` only when `writable_then_seal`; otherwise `None`.

- [ ] **Step 3: Write the failing test**

Add a unit test (in the module owning `JavaCompilerIndex`) that constructs an index with the new fields serialized and asserts `provenance == Some("TRANSFORMED_WORKSPACE")` and `source_state["changedFiles"]` is non-empty when `before != after`, and that an index serialized without these fields round-trips with `provenance == None`.

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test --locked -p clew --lib '<index module>::tests::transformed_workspace_provenance' -- --test-threads=1`
Expected: FAIL until fields added; PASS after.

- [ ] **Step 5: Run the crate suite**

Run: `cargo test --locked -p clew --lib -- --test-threads=1`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/clew/src/java_adapter_v2.rs crates/clew/src/generation_service.rs
git commit -m "feat(java): mark transformed-workspace provenance on compiler index"
```

---

### Task 6: Acceptance on task-router-leads

**Files:** none (validation).

- [ ] **Step 1: Build and run `docs check` with the new profile**

```bash
export JAVA_HOME=$HOME/Library/Java/JavaVirtualMachines/jdk-17.0.20+8/Contents/Home
export PATH=$JAVA_HOME/bin:$PATH
cargo build --locked -p clew
```

Edit `~/repo/kasko/arch-kasko/catalog/services/task-router-leads.json` to set `"profile":"java-17plus-maven-writable-then-seal"`, then:

```bash
$PWD/target/debug/clew docs check --root ~/repo/kasko/arch-kasko
```

Expected: task-router-leads admission passes (no `BUILD_COMMAND_FAILED`); the generated compiler index carries `provenance: TRANSFORMED_WORKSPACE`.

If it still fails, capture the real Maven error with `--debug-output` (research checkout supports it) and report it; do not weaken the evidence contract.

- [ ] **Step 2: Confirm the read-only profile is unchanged**

Revert the service profile to `java-17plus-maven-read-only` and run the check again; it must still report `BUILD_COMMAND_FAILED` for the in-place transform (existing behavior), proving the new behavior is opt-in.

- [ ] **Step 3: Restore the service catalog file to `java-17plus-maven-read-only`** (or the agreed default) and commit nothing from this step.

---

## Self-review notes

- **Spec coverage:** profile (Task 1), writable materialization + build + seal (Tasks 2-3), boundary + index-only-transformed (Task 4), evidence marker/honesty (Task 5), acceptance/regression (Task 6). All spec sections covered.
- **Marker placement:** per approved decision, on the `JavaCompilerIndex` (evidence), not `JavaProjectModel`.
- **Compatibility:** new `JavaCompilerIndex` fields are optional/defaulted, so existing serialized indices stay readable.