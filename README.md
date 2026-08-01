# Semantic Thread Platform — Kotlin MVP

Executable vertical prototype of a language-neutral Rust semantic core and a version-pinned Kotlin/JVM worker. The worker owns Kotlin PSI and compiler interaction; the Rust process owns canonical IR, graph analysis, storage, slicing, transactions, Git and the CLI.

## Prerequisites

- JDK 21
- Git
- `jq` for the reproducible transaction demo
- Maven on `PATH` for Maven repositories without `./mvnw`
- Rust is installed automatically according to `rust-toolchain.toml` when rustup is available

Kotlin and `protoc` do not need system installations. Version-pinned Kotlin 2.1.21, 2.3.0, and 2.4.10 workers are resolved by Gradle, selected automatically from the target project, and `protoc` is vendored by the Rust build.

## Quick start

```bash
./scripts/verify.sh
cargo run --bin sthread -- doctor
cargo run --bin sthread -- project inspect --repo fixtures/kotlin-basic
cargo run --bin sthread -- index --repo fixtures/kotlin-basic
cargo run --bin sthread -- index --repo fixtures/kotlin-basic --syntax-only
cargo run --bin sthread -- project inspect --repo fixtures/kotlin-maven
cargo run --bin sthread -- agent-context --repo fixtures/kotlin-2-1 \
  --term applyAdaptive --term Adaptive --max-bytes 12288 \
  --evidence .semantic-thread/agent-context.json
printf '%s\n' '{"id":1,"method":"health"}' '{"id":2,"method":"shutdown"}' | cargo run --bin semanticd
```

For slicing, preview and commit the target must be a Git repository with a committed `HEAD`. The reproducible demonstration creates an isolated copy:

```bash
./scripts/demo.sh
```

The CLI always writes machine-readable canonical JSON to stdout; diagnostics from Gradle, Maven, Git, and the JVM go to stderr. Exit codes are stable by error category (`2` input, `3` not found, `4` stale, `5` conflict, `6` validation, `7` worker/protocol).

## Supported vertical

- Gradle Wrapper and single-module Maven Kotlin/JVM inspection with project model fingerprinting
- §11 PSI declaration/file/semantic facts with typed invalidation persisted in compilation-scoped SQLite WAL/content blobs
- FQN function and file+offset expression resolution
- composite semantic anchors with unique replay
- actual K2 FIR CFG normalization, Rust dominance-frontier SSA/PHI/def-use and post-dominator control dependencies
- forward/backward/bidirectional bounded slicing and canonical Thread IR
- one-shot `agent-context` discovery with deduplicated source, references, tests,
  semantic edit anchors, a hard stdout byte budget, and full evidence stored separately
- `REPLACE_EXPRESSION`, `REPLACE_FUNCTION_BODY`, `ADD_IMPORT`, and `REMOVE_IMPORT` on PSI copies
- K2 candidate diagnostics, type, protected-binding, call-target, callee-summary, and effect validation
- minimal preview diff with Expected/ActualWriteSet and ABI enforcement, isolated worktree validation, and configured tests run by default
- candidate commits with provenance trailers, a completely staged index, CAS ref update, atomic index rename, and inverse-CAS rollback
- typed Protobuf requests, mandatory snapshots, batching and content-addressed large-source BlobRefs
- append-only SQLite transaction ledger with pre/post-CAS crash recovery and idempotent retry, semantic ReadSet replay, callee staleness, WW/RW conflicts, and project-model invalidation
- immutable RepositoryIndex snapshots with pre-CAS construction, atomic publication, executable caller/downstream invalidation, and recovery repair
- complete language-neutral SymbolId, AST/type edges, and LOCAL/THIS/OBJECT/STATIC/UNKNOWN memory abstractions
- separate long-lived Rust `semanticd` JSONL service with structured logs and required metrics

This is intentionally fail-closed. Android, KMP, scripts, reflection, precise coroutine lowering, global interprocedural analysis, and ambiguous anchors are rejected or marked as boundaries. Compiler plugins for the selected JVM compilation are honored and content-hashed, but arbitrary plugin-specific semantic interpretation remains outside the MVP. See [progress](docs/progress.md) and [final report](docs/final-report.md).

## Repository map

- `crates/sthread`: Rust core and CLI
- `workers/kotlin`: long-lived Kotlin 2.4.10 PSI/compiler worker
- `workers/kotlin21`: Kotlin 2.1.21 adapter reusing the common worker implementation
- `workers/kotlin23`: Kotlin 2.3.0 adapter used by Maven/Spring services such as `product-repo`
- `schemas`: versioned Protobuf contracts
- `fixtures`: executable Kotlin corpus
- `docs`: architecture, safety model, protocol, ADRs, and status
- `scripts`: one-command verification and demo
