# Architecture

The authoritative state is the tuple `(Git revision, exact source bytes, project model hash)`. Every index, anchor, local graph and Thread IR is derived and disposable.

`sthread` starts one long-lived `kotlin-worker-2.4.10` process per CLI invocation. The separate `semanticd` Rust service keeps that worker alive across JSONL service requests. Both use length-prefixed Protobuf envelopes internally and canonical JSON DTO payloads with snapshots, batches, and content-addressed blobs. The worker owns Kotlin PSI construction, declaration/source facts, source-backed graph origins and PSI-copy edits. No Kotlin compiler object crosses the protocol.

The Rust boundary owns SQLite WAL state, canonical serialization, graph normalization, SSA PHI and def-use edges, control dependencies, bounded slicing, ReadSet/WriteSet, validation orchestration, Git detached worktrees, compare-and-swap ref updates and the append-only ledger.

Calls have depth zero in this vertical and therefore become explicit external boundaries. The pinned compiler plugin exports the actual K2 FIR CFG; the worker normalizes it into language-neutral nodes and explicit true/false/exception/back edges. The adapter is isolated behind `BUILD_LOCAL_GRAPH`.

## Data flow

```text
Git snapshot -> Kotlin project/K2 facts -> Rust SQLite index snapshot
             -> FIR CFG DTO -> AST/type/memory + PHI/def-use/control dependencies
             -> bounded Thread IR + ReadSet
Edit IR -> unique anchor replay -> PSI-copy candidate -> Gradle/K2 validation
        -> preview -> detached worktree -> affected compile/explicit tests -> commit
        -> CAS ref -> publish new index snapshot + applied invalidations
```

## Invalidation

Project model files, classpath/friend artifacts, compiler plugins and effective compiler options are content-hashed deterministically and form both ProjectModelHash and K2 cache identity. Source facts carry content, signature, body, ABI and summary hashes. Each transaction starts from the persistent RepositoryIndex hash; successful CAS publication updates that index from the candidate revision and records caller/downstream invalidations. A missing or ambiguous target is never guessed. A moved target with unchanged composite facts can replay; any semantic uncertainty fails closed.
