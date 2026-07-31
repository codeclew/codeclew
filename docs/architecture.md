# Architecture

The authoritative state is the tuple `(Git revision, exact source bytes, project model hash)`. Every index, anchor, local graph and Thread IR is derived and disposable.

`sthread` starts one long-lived `kotlin-worker-2.4.10` process per CLI invocation. It receives length-prefixed Protobuf envelopes over stdin/stdout and returns canonical JSON payloads in batched messages. The worker owns Kotlin PSI construction, declaration/source facts, source-backed graph origins and PSI-copy edits. No Kotlin compiler object crosses the protocol.

The Rust boundary owns SQLite WAL state, canonical serialization, graph normalization, SSA PHI and def-use edges, control dependencies, bounded slicing, ReadSet/WriteSet, validation orchestration, Git detached worktrees, compare-and-swap ref updates and the append-only ledger.

Calls have depth zero in this vertical and therefore become explicit external boundaries. The current local CFG exporter uses supported PSI constructs rather than version-unstable FIR internals; the adapter is isolated behind `BUILD_LOCAL_GRAPH` and can be replaced without changing core schemas.

## Data flow

```text
Git snapshot -> Kotlin project/PSI facts -> Rust SQLite index
             -> local CFG DTO -> PHI/def-use/control dependencies
             -> bounded Thread IR + ReadSet
Edit IR -> unique anchor replay -> PSI-copy candidate -> Gradle/K2 validation
        -> preview -> detached worktree -> compile/tests -> commit -> CAS ref
```

## Invalidation

Project model files are content-hashed deterministically. Source facts carry content, signature and body hashes. A changed model invalidates the transaction. A missing or ambiguous target is never guessed. A moved target with unchanged composite facts can replay; any semantic uncertainty fails closed.

