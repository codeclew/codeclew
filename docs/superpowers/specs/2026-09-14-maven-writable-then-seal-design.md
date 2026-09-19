# Maven writable-then-seal admission profile

Date: 2026-09-14
Status: Design (approved)

## Problem

`clew docs` and Java analysis run the native Maven build inside a disposable
worktree materialized from the immutable captured source. The materialization is
sealed read-only before the build. For repositories whose Maven lifecycle
rewrites source files in place (for example the `ru.tins:java-code-transform-plugin`
`mask-fields-annotate` goal used by the Kasko fleet), the build fails with
`Permission denied` because existing source files are `0o400`/`0o500`.

This is not a JDK issue: clew correctly inherits `JAVA_HOME`; a writable
materialization compiles fine. The blocker is the read-only source materialization.

## Goal

Add a separately opt-in Java Maven profile that:

1. materializes the immutable captured source into a **disposable writable**
   workspace;
2. runs the ordinary Maven lifecycle there (so in-place code generation and
   transformation plugins can write source);
3. verifies and records the **transformation boundary**;
4. **seals** the resulting materialization read-only;
5. indexes **only that sealed transformed workspace**.

Constraints:

- The original checkout stays untouched.
- The existing read-only Maven profile (`java-17plus-maven-read-only`) is
  unchanged.
- The new behavior is separately opt-in.
- Resulting evidence explicitly identifies the compiler result as
  **transformed-workspace evidence**, never claiming the immutable captured
  source was compiled unchanged.

## Chosen approach

Profile-gated re-seal + re-hash on the existing materialization/build/index
path (Approach A).

## New profile

`java-17plus-maven-writable-then-seal` — registered wherever Java Maven
profiles are validated/selected:

- `crates/clew/src/documentation/store.rs` (~L427): docs service profile
  validation (`"java-17plus-maven-read-only" | "java-17plus-gradle-read-only"`).
- support matrix / `analysis_modules`: allowed `--profile` values for the CLI.
- usable from `codeclew.yaml`, docs `service.json`, and CLI `--profile`.

## Flow

Current path (unchanged for read-only profile), from
`kotlin_adapter_v2.rs::prepare_language` and `repository_snapshot.rs::materialize`:

1. `materialize(...)` — CAS snapshot → `attempts/<id>/repo`; `seal_tree` (all read-only).
2. `mount_project_derived_state(...)` — dirs → `0o700`, writable symlink stubs for `target/`, `build/`, `.gradle/`.
3. `extract_maven(...)` — ordinary Maven lifecycle (effective-pom, compile + dependency:build-classpath, release).
4. No post-build seal exists today.

For the new profile, insert the writable phase and a post-build seal:

1. `materialize(...)` — as today (seal first, keeps existing invariant).
2. `mount_project_derived_state(...)` — as today (dirs writable, derived stubs).
3. **Writable materialization (new, profile-gated):** one pass over the
   worktree adding the **owner-write** bit to files (`mode | 0o200`; dirs
   already `0o700`), preserving exec and other bits so a later `seal_tree` can
   restore the original executability. This is the
   `disposable writable Maven materialization`.
4. `extract_maven(...)` — unchanged. The lifecycle runs; in-place
   transform/codegen plugins can write source.
5. **Post-build seal + boundary (new):** on success, `seal_tree(repo)` makes
   the whole materialization read-only again, and the transformation boundary
   is recorded.
   - On build failure: do **not** seal; return `BUILD_COMMAND_FAILED` as today
     (the materialization is disposable and will be GC'd).

`extract_maven` itself is unchanged; the only edit in `prepare_language` is the
profile-gated writable-file pass and the post-build seal.

## Transformation boundary

Recorded in the model authority as `sourceState`:

```json
{
  "kind": "TRANSFORMED_WORKSPACE",
  "before":  { "<relative-path>": "<content-digest from CAS snapshot>" },
  "after":   { "<relative-path>": "<content-digest from sealed transformed worktree>" },
  "changedFiles": [ { "path": "...", "before": "...", "after": "..." } ]
}
```

- `before` = digest map of `source_files` from the immutable CAS snapshot.
- `after` = digest map of the same relative paths re-hashed from the sealed
  transformed worktree after the build.
- If `before == after`, `changedFiles` is empty but `kind` stays
  `TRANSFORMED_WORKSPACE`; we never claim "compiled as-is".

## Indexing

In `generation_service.rs` (`effective_java_sources`,
`java_source_content_digests`):

- For the new profile, source content is read from the **sealed transformed
  worktree** filesystem (re-hashed), not from the CAS snapshot.
- The existing `source outside the sealed snapshot` guard is redirected to the
  transformed worktree for this profile.
- `JAVA_GENERATED_DECLARATIONS_NOT_INDEXED` (generated sources under
  `target/generated-sources`) is preserved: only `src` transformed in place is
  indexed.

## Downstream identity / evidence honesty

- The model authority carries `provenance: TRANSFORMED_WORKSPACE`.
- Consumers (nav/context/explanation) see that the compiler result is evidence
  of a transformed workspace, not of the immutable captured source compiled
  unchanged.
- Facts/`sourceIds` keep relative paths, but source↔content binding uses the
  transformed digests, and explanations label this explicitly.

## Error handling

- Build failure → no seal, `BUILD_COMMAND_FAILED` (unchanged semantics).
- Missing/extra generated files in the transformed worktree handled by the
  existing source-set boundaries; a source selected by the model but absent
  after transform is a hard error (existing guard, redirected to worktree).

## Testing

- Unit (fixtures): for the new profile, files are writable after mount, then
  `seal_tree` restores read-only after a successful build.
- Indexing: a source modified in the worktree by a transform yields
  `after != before`, `provenance: TRANSFORMED_WORKSPACE`, correct
  `changedFiles`.
- Regression: the existing `java-17plus-maven-read-only` path is unchanged
  (still fails on in-place transform, reads from CAS).
- Acceptance: `clew docs check` on `task-router-leads` with the new profile
  passes admission.

## Files touched

- `crates/clew/src/kotlin_adapter_v2.rs` — writable-file pass + post-build seal (profile-gated).
- `crates/clew/src/repository_snapshot.rs` — reusable seal helper (already exists as `seal_tree`); no structural change.
- `crates/clew/src/generation_service.rs` — indexing reads transformed worktree for the new profile.
- `crates/clew/src/java_project_model.rs` — the `JavaProjectModel` authority carries `provenance: TRANSFORMED_WORKSPACE` and `sourceState` (before/after/changedFiles) for the new profile.
- `crates/clew/src/documentation/store.rs` + support matrix — profile registration.
- tests in the affected crates.

## Out of scope

- Gradle in-place transform support (same pattern may be added later).
- `source-syntax` profile (already separate; no compiler run).
- Changing the existing read-only profile's semantics.