# Codeclew — managed semantic changes for Kotlin

Codeclew builds bounded compiler-backed context, validates an edit plan in an
isolated candidate worktree, and publishes the resulting commit explicitly.
The only supported executable entrypoint is `./clew`.

## Requirements

- macOS or Linux
- Python 3.11+
- Git
- JDK 21
- the Rust toolchain pinned by `rust-toolchain.toml`
- Maven on `PATH` only for Maven projects without `./mvnw`

Kotlin workers for 2.1.21, 2.3.0, and 2.4.10 are packaged into an immutable
runtime capsule. A cold start may build the capsule. A warm invocation verifies
and reuses it without running Cargo, Rustc, Gradle, or Maven.

## Workflow

```bash
./clew doctor

./clew session open \
  --repo /path/to/clean-kotlin-repository \
  --target-ref main

./clew context create \
  --session session:... \
  --intent 'describe the requested change' \
  --term ImportantSymbol \
  --term ImportantBehavior

./clew context expand \
  --session session:... \
  --from context:sha256:... \
  --term MissingCaller

./clew plan validate \
  --session session:... \
  --context context:sha256:... \
  --plan edit-plan.json

./clew task-run start \
  --session session:... \
  --context context:sha256:... \
  --plan plan:sha256:...

./clew task-run status --run run:...
./clew session publish --session session:... --run run:...
```

`task-run start` writes a durable `CREATED` record before detaching. Repeating
the same request attaches to the same content-addressed run. Preparation may
compile, test, and build a staged repository index, but it never changes the
session's target ref. Only `session publish` may fast-forward the ref.

Context stdout is bounded to 64 KiB. It contains the edit-ready projection and
content IDs; full evidence remains in private managed state. Plans are bounded
to 1 MiB, 256 operations, 256 files, and a 256 KiB expected write set.

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
The sealed external contour remains fail-closed.

## Conditional evidence

When evidence is useful but not deterministic, a conditional decision may carry
explicit publication-blocking obligations. Such a run may compile, test, and
index a candidate, but terminates as `VALIDATED_CONDITIONAL`. It cannot be
published. After the obligations are discharged, create a new context, plan,
and run; there is no confidence threshold or automatic promotion.

## Verification

```bash
./scripts/verify.sh
./scripts/demo.sh
./scripts/benchmark.sh
```

The CLI writes canonical JSON to stdout and diagnostics to stderr. The system is
fail-closed for stale authorities, ambiguous anchors, unsupported project
models, dirty checked-out publication targets, and recovery uncertainty.

## Repository map

- `crates/clew`: Rust core, supervisor, sessions, indexes, and CLI
- `workers/kotlin*`: version-pinned Kotlin compiler workers
- `bootstrap`: isolated content-addressed runtime bootstrap
- `schemas`: typed worker protocol
- `fixtures`: executable Kotlin corpus
- `scripts`: CI, demo, and benchmark entrypoints
- `docs`: architecture and experiment history

Licensed under the [Apache License 2.0](LICENSE).
