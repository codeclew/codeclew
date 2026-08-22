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
and reuses it without running Cargo, Rustc, Gradle, or Maven. Workers execute
directly from that sealed capsule under its shared lease; the warm path does not
copy their distributions.

## Workflow

```bash
./clew session open \
  --repo /path/to/clean-kotlin-repository \
  --target-ref main \
  --compilation :app/main

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
./clew session close --session session:...
./clew session gc --session session:...
```

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

`task-run start` writes a durable `CREATED` record before detaching. Repeating
the same request attaches to the same content-addressed run. Preparation may
compile, test, and build a staged repository index, but it never changes the
session's target ref. Only `session publish` may fast-forward the ref.

Interrupted pre-commit work can be continued with `task-run resume`. A committed
candidate is never reset automatically: use `session recover` with its session
and run IDs to reconcile it. `session close` requires no live or unresolved run;
`session abort` is the explicit cancellation terminal. If the repository moved,
use `session relocate --session ... --repo ...`. GC deletes only worktrees proven
to be Codeclew-owned and refuses every non-empty candidate without an exact
receipt, including with `--force`.

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
explicit publication-blocking obligations. Such a run may compile, test, and
index a candidate, but terminates as `VALIDATED_CONDITIONAL`. It cannot be
published. After the obligations are discharged, create a new context, plan,
and run; there is no confidence threshold or automatic promotion.

## Verification

Development verification follows the machine-readable stabilization plan in
`docs/stabilization-plan.json`. Inspect the next admissible step with:

```bash
python3 -I -S scripts/stabilization_control.py status
```

Run only the check named by that status through the controller. The controller
binds immutable receipts to the plan, verifier, command, relevant source bytes,
environment, host qualification, and source revision. A functional failure
cannot be retried against the same evidence key.

Full verification, BTA24, cold/multi-compilation performance gates, and the
warm benchmark refuse direct execution. They become admissible only at their
declared stabilization steps. This keeps ordinary development on bounded
static, unit, and component checks; expensive end-to-end evidence is produced
once per unchanged authority rather than after every edit.

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
