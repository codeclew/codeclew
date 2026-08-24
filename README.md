# Codeclew — managed semantic context and changes

Codeclew builds bounded compiler-backed context, validates an edit plan in an
isolated candidate worktree, and publishes the resulting commit explicitly.
The only supported executable entrypoint is `./clew`.

## Requirements

- macOS or Linux
- Python 3.11+
- Git
- JDK 21
- the Rust toolchain pinned by `rust-toolchain.toml`
- Cargo on `PATH` for Rust repositories
- Maven on `PATH` only for Maven projects without `./mvnw`

The exact Kotlin 2.3.0 and 2.4.10 workers are packaged into an immutable runtime
capsule. A cold runtime start may build the capsule. A warm runtime invocation
verifies and reuses it without running Cargo, Rustc, Gradle, or Maven. The
workers execute directly from that sealed capsule under its shared lease; the
runtime warm path does not copy their distributions. Project analysis may still
invoke the project wrapper under the model-cache policy described below.

The current supported product contour is deliberately narrower than the
source-level research surface: Kotlin 2.4.10, Gradle, `PROJECT_NATIVE`, and one
exact compilation. Exact Kotlin 2.3.0 with Maven is qualified as a read-only
compiler-backed context preview: it has real-project context acceptance, but no
mutation or publish claim. Kotlin 2.1, multiple compilations, Android/KMP and
`EXTERNAL` remain unqualified contours until they have their own acceptance
tests.

Rust has a separate read-only syntax preview contour. Open it with `--language rust`
and an exact target selector such as
`cargo:crates/clew/Cargo.toml#clew#lib#clew`. The repository must have a regular
root `Cargo.lock`; live `cargo metadata --no-deps` authority and all paths are
normalized before entering CAS or stdout. The preview follows only unambiguous
snapshot-backed `mod name;` files from the selected target and exposes bounded
declaration occurrences with exact syntax ranges. It deliberately emits no
resolved references or call edges: cfg, derive/procedural macros, custom module
paths, parse failures, ambiguity, and resource caps remain explicit boundaries,
so the generation is `PARTIAL/UNSURE`. Task preparation and publication are
rejected in both the CLI and transaction layer until compiler name resolution,
cfg/macros, compilation, and tests receive their own mutation acceptance.

This revision is pilot-ready, not general availability. Team use follows the
[Kotlin 2.4 pilot runbook](docs/pilot/README.md); a signed prebuilt release is a
separate decision after 20 recorded in-contour cases meet its numerical gate.

## Workflow

```bash
./clew change open \
  --repo /path/to/clean-kotlin-repository \
  --target-ref main \
  --language kotlin \
  --compilation :app/main \
  --intent 'describe the requested change' \
  --term ImportantSymbol \
  --term ImportantBehavior

./clew change prepare \
  --session session:... \
  --context context:sha256:... \
  --plan edit-plan.json

./clew change status --run run:...
./clew change publish --session session:... --run run:...
./clew session close --session session:...
./clew session gc --session session:...
```

`change open` atomically returns the session and its bounded initial context.
`change prepare` validates the immutable plan and idempotently starts its
isolated candidate run. The low-level `session`, `context`, `plan`, and
`task-run` commands remain an advanced protocol for expansion, cancellation,
relocation, and diagnostics; they are not required for the happy path.

Terms may be exact identities or natural identifier components. The query
index retains the full identifier and language-neutral camel-case/snake-case
aliases, so `Maven` can discover `MavenProjectModel` without changing the exact
request authority. Component discovery remains `UNSURE` until exact semantic
evidence is selected and its reported obligations are checked. Prefer a few
distinctive terms over broad words such as `model` or `test`, which may produce
an intentionally truncated conditional context.

Each normalized term retains at most 1024 deterministic fact references in the
query index. A term with greater fan-out is recorded as overflow authority and
every result that uses it reports `truncated=true`; refine that request with a
class, callable, or file identity before treating the context as complete. This
keeps generic vocabulary from turning the bounded routing index into a second
copy of the repository fact store.

`--compilation` is mandatory and names an exact build compilation authority:
use `:/main` for a Kotlin root project or `:module/main` for a Gradle
subproject. Repeat the option to select up to 64 compilations. Session authority
sorts and deduplicates the set and accepts only `:/sourceSet` or
`:module[:nested]/sourceSet`; option-like and path-traversal values are rejected
before any build tool starts. Codeclew never guesses a root compilation because
that would defer a deterministic configuration error from session admission to
context creation.

Multi-compilation generation captures the repository snapshot once and admits
independent compiler lanes from the detected CPU and memory budget. By default
the lane count is host-adaptive and capped at 16. Reproducible measurements may
pin it with `--generation-jobs N`; the selected job count is session authority
and cannot exceed current admission.

`change prepare` writes a durable `CREATED` record before detaching. Repeating
the same request attaches to the same content-addressed run. Preparation may
compile, test, and build a staged repository index, but it never changes the
session's target ref. Only `change publish` may fast-forward the ref.

Interrupted pre-commit work can be continued with the advanced `task-run
resume`. A committed candidate is never reset automatically: use `change
recover` with its session and run IDs to reconcile it. `session close` requires no live or unresolved run;
`session abort` is the explicit cancellation terminal. If the repository moved,
use `session relocate --session ... --repo ...`. GC deletes only worktrees proven
to be Codeclew-owned and refuses every non-empty candidate without an exact
receipt, including with `--force`.

Context stdout is bounded to 64 KiB. It contains the edit-ready projection and
content IDs; full evidence remains in private managed state. Plans are bounded
to 1 MiB, 256 operations, 256 files, and a 256 KiB expected write set.

Opaque per-file `semanticFacts` arrays are normalized before hashing and
replaced in public/query file facts by a fact count and digest. Other normalized
file metadata and declarations remain present, while granular compiler
descriptors and relations remain queryable. This prevents private operational
paths and oversized opaque payloads from dominating context selection; recall
inside the omitted array is deliberately not claimed.

## State and build authority

All mutable Codeclew state lives under private `CODECLEW_HOME` (by default the
user cache directory):

```text
runtimes/
repos/
sessions/
runs/
locks/
tmp/
quarantine/
```

Codeclew does not discover, read, import, update, or delete `.semantic-thread`.
Old receipts, indexes, and runs are inert. Absolute repository paths exist only
in private `0600` locator files and are never emitted in stdout or evidence.

`PROJECT_NATIVE` uses the project's wrapper and ordinary user build
environment. Model caching is `NON_CACHEABLE` unless the session explicitly
selects a tracked `codeclew.model-cache.json` or sealed external authority.
The sealed external contour remains fail-closed. Within an open session,
context creation reuses its immutable generation. For every new `NON_CACHEABLE`
session, Codeclew extracts and compares the live project model; it reuses a
previous generation only when the repository snapshot, derived model manifest,
and compiler-store authority are all exact matches. The worker still opens the
project, but skips `IndexFiles`. Any mismatch returns to the fail-closed
full/delta planner.

A tracked cache manifest must exactly match `HEAD`, be canonical JSON plus one
newline, and explicitly list the authorized compilations:

```json
{"compilations":[":/main"],"schema":"codeclew-model-cache-policy/2.0"}
```

Select it with `session open --model-cache tracked-manifest`. Sealed external
state requires a RELEASE capsule, a private absolute directory with its signed
manifest/seed, and both `--model-cache sealed-external` and
`--external-build-state /private/path`.

## Conditional evidence

When evidence is useful but not deterministic, a conditional decision may carry
explicit publication obligations. Such a run may compile, test, and index a
candidate. If both context and candidate remain within the supported contour,
it reaches `READY_TO_PUBLISH_CONDITIONAL`; incomplete or unsupported evidence
still terminates as `VALIDATED_CONDITIONAL` and cannot be published.

Conditional publication is fail-closed by default. The caller must pass
`--allow-conditional` and acknowledge every qualified `approvalId` reported by
`change status`. The durable approval binds the session, run, context evidence,
plan, candidate commit and snapshot, exact changed files, bounded diff and
successful validation evidence. The result is `PUBLISHED_CONDITIONAL` and its
certainty remains `UNSURE`; acknowledgement never upgrades evidence to
`VERIFIED`.

```bash
./clew change status --run run:...
./clew change publish \
  --session session:... \
  --run run:... \
  --allow-conditional \
  --prepared-authority-digest sha256:... \
  --acknowledge-obligation context:sha256:...
```

## Verification

Ordinary development and GitHub CI use the same repository-owned entrypoint:

```bash
./scripts/ci-verify.sh
```

The stabilization controller and its receipts remain optional research/release
evidence. They do not gate product development or CI. Cold/multi-compilation
performance, BTA24, self-hosting and agent comparisons are qualification work,
not prerequisites for the supported Kotlin 2.4 publish path. Kotlin 2.3.0
qualification covers exact worker admission, capsule identity, privacy-safe
fact translation, incremental receipt construction, and read-only context on a
representative Maven repository; it does not extend the publish contour.

The CLI writes canonical JSON to stdout and diagnostics to stderr. The system is
fail-closed for stale authorities, ambiguous anchors, unsupported project
models, dirty checked-out publication targets, and recovery uncertainty.
Each cold generation also retains a private `dag-report.json` with total work,
critical path, observed parallelism, and the number of sealed compiler streams.

## Repository map

- `crates/clew`: Rust core, supervisor, sessions, indexes, and CLI
- `workers/kotlin*`: version-pinned Kotlin compiler workers
- `bootstrap`: isolated content-addressed runtime bootstrap
- `schemas`: typed worker protocol
- `fixtures`: executable Kotlin corpus
- `scripts`: CI, demo, and benchmark entrypoints
- `docs`: architecture and experiment history

Licensed under the [Apache License 2.0](LICENSE).
