# Semantic Thread Platform — Kotlin MVP

Executable vertical prototype of a language-neutral Rust semantic core and a version-pinned Kotlin/JVM worker. The worker owns Kotlin PSI and compiler interaction; the Rust process owns canonical IR, graph analysis, storage, slicing, transactions, Git and the CLI.

## Prerequisites

- JDK 21
- Git
- `jq` for the reproducible transaction demo
- Rust is installed automatically according to `rust-toolchain.toml` when rustup is available

Kotlin and `protoc` do not need system installations. Kotlin 2.4.10 is resolved by Gradle and `protoc` is vendored by the Rust build.

## Quick start

```bash
./scripts/verify.sh
cargo run --bin sthread -- doctor
cargo run --bin sthread -- project inspect --repo fixtures/kotlin-basic
cargo run --bin sthread -- index --repo fixtures/kotlin-basic
```

For slicing, preview and commit the target must be a Git repository with a committed `HEAD`. The reproducible demonstration creates an isolated copy:

```bash
./scripts/demo.sh
```

The CLI always writes machine-readable canonical JSON to stdout; diagnostics from Gradle, Git, and the JVM go to stderr. Exit codes are stable by error category (`2` input, `3` not found, `4` stale, `5` conflict, `6` validation, `7` worker/protocol).

## Supported vertical

- Gradle Kotlin/JVM inspection and project model fingerprinting
- PSI declaration index with K2 FIR facts persisted in SQLite WAL/content blobs
- FQN function and file+offset expression resolution
- composite semantic anchors with unique replay
- actual K2 FIR CFG normalization, Rust dominance-frontier SSA/PHI/def-use and post-dominator control dependencies
- forward/backward/bidirectional bounded slicing and canonical Thread IR
- `REPLACE_EXPRESSION`, `REPLACE_FUNCTION_BODY`, `ADD_IMPORT`, and `REMOVE_IMPORT` on PSI copies
- K2 candidate diagnostics, type, protected-binding, call-target, callee-summary, and effect validation
- minimal preview diff, isolated worktree validation, configured tests
- candidate commits with provenance trailers and CAS ref update
- append-only SQLite transaction ledger, semantic ReadSet replay, callee staleness, WW/RW conflicts, and project-model invalidation

This is intentionally fail-closed. Android, KMP, scripts, compiler plugins, reflection, precise coroutine lowering, global interprocedural analysis, and ambiguous anchors are rejected or marked as boundaries. See [progress](docs/progress.md) and [final report](docs/final-report.md).

## Repository map

- `crates/sthread`: Rust core and CLI
- `workers/kotlin`: long-lived Kotlin 2.4.10 PSI/compiler worker
- `schemas`: versioned Protobuf contracts
- `fixtures`: executable Kotlin corpus
- `docs`: architecture, safety model, protocol, ADRs, and status
- `scripts`: one-command verification and demo
