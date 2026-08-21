# Codeclew — Kotlin semantic change engine

Executable vertical prototype of a language-neutral Rust semantic core and a version-pinned Kotlin/JVM worker. The worker owns Kotlin PSI and compiler interaction; the Rust process owns canonical IR, graph analysis, storage, slicing, transactions, Git and the CLI.

## Prerequisites

- JDK 21
- Git
- `jq` for the reproducible transaction demo
- Python 3 for the durable agent-facing `task-apply` runner
- Maven on `PATH` for Maven repositories without `./mvnw`
- Rust is installed automatically according to `rust-toolchain.toml` when rustup is available

Kotlin and `protoc` do not need system installations. Version-pinned Kotlin 2.1.21, 2.3.0, and 2.4.10 workers are resolved by Gradle, selected automatically from the target project, and `protoc` is vendored by the Rust build.

## Quick start

```bash
cargo run --bin clew -- doctor
cargo run --bin clew -- project inspect --repo fixtures/kotlin-basic
cargo run --bin clew -- index --repo fixtures/kotlin-basic
cargo run --bin clew -- index --repo fixtures/kotlin-basic --syntax-only
cargo run --bin clew -- project inspect --repo fixtures/kotlin-maven
cargo run --bin clew -- agent-context --repo fixtures/kotlin-2-1 \
  --term applyAdaptive --term Adaptive --max-bytes 12288 \
  --evidence .semantic-thread/agent-context.json
cargo run --bin clew -- prove map-edge-with-context \
  --repo /path/to/clean-kotlin-repository \
  --workflow-symbol com.example.valuesAwaitingContext \
  --test-symbol 'applies the mapping context to one value'
printf '%s\n' '{"id":1,"method":"health"}' '{"id":2,"method":"shutdown"}' | cargo run --bin semanticd
```

`project inspect`, `agent-context`, and `task-apply` use the target project's
normal build environment by default: its wrapper, user-level Maven/Gradle
caches, settings, mirrors, credentials, and network policy. Codeclew does not
create or require a second dependency repository below the target checkout.
The exact Kotlin worker distribution is built lazily on first use and verified
against its committed manifest before it starts.

`scripts/sthread_preflight.py` and `./scripts/verify.sh` are release/CI
diagnostics, not prerequisites for an agent task. Use the preflight only when
you explicitly need the sealed external offline build-state contour and its
reproducibility receipt. That external contour is currently inspect/index-only;
`task-apply` fails closed instead of validating a candidate against a different
ambient dependency authority.

Long `task-apply` operations should be launched through the repository-owned
runner so an agent tool session can end without terminating or duplicating the
transaction:

```bash
python3 scripts/task_apply_runner.py start \
  --clew "$PWD/target/debug/clew" \
  --repo /path/to/clean-kotlin-repository \
  --context /path/to/task-context-evidence.json \
  --edit-plan /path/to/edit-plan.json \
  --target-ref main \
  --actor semantic-task-agent
```

`start` returns after a short handshake with a deterministic `runId`, a
`statusCommand` argv, and the bound transaction ID. Repeating the byte-exact
request attaches to the same run and never invokes `task-apply` again. Run the
returned status command until `terminal` is true; `STARTING`, `RUNNING`,
`DRAINING`, and `RUNNING_UNSUPERVISED` are nonterminal. Durable logs, the
transaction artifact, and `completion.json` live below
`.semantic-thread/task-runs/<request-digest>/`.
`UNKNOWN_REQUIRES_INSPECTION` is terminal and is never retried automatically;
after both lifetime locks have been released, use only the returned
`transactionInspectCommand` to reconcile the existing transaction.

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
- authority-backed `MAP_EDGE_WITH_CONTEXT` proof: compiler-derived role binding,
  twelve preservation invariants, a closed fifteen-obligation change graph, and
  explicit `BOUND`, `AMBIGUOUS`, or `REFUSED` outcomes without source mutation
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

- `crates/clew`: Rust core and CLI
- `workers/kotlin`: long-lived Kotlin 2.4.10 PSI/compiler worker
- `workers/kotlin21`: Kotlin 2.1.21 adapter reusing the common worker implementation
- `workers/kotlin23`: Kotlin 2.3.0 adapter used by Maven/Spring services such as `product-repo`
- `schemas`: versioned Protobuf contracts
- `fixtures`: executable Kotlin corpus
- `docs`: architecture, safety model, protocol, ADRs, and status
- `scripts`: one-command verification and demo

## License

Licensed under the [Apache License 2.0](LICENSE).
