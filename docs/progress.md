# Progress

| Stage | Gate | Status |
|---|---|---|
| 0 Bootstrap | Rust starts worker, handshakes and shuts down | Passed |
| 1 Project model | repeated canonical model/hash | Passed on `kotlin-basic` |
| 2 Declaration index | compilation-scoped SQLite/WAL incremental update without unchanged-file rewrites | Passed |
| 3 Semantic facts | K2 types, call targets, receivers, argument mappings, diagnostics | Passed by FIR-plugin golden tests |
| 4 Local CFG | actual K2 FIR CFG exported and normalized | Passed for branch/loop/exception/safe-call fixtures |
| 5 Rust graph | dominance-frontier SSA, PHI, def-use, post-dominator control dependencies | Passed by golden and permutation tests |
| 6 Slicer | bounded directions, Thread IR, ReadSet, explicit boundaries | Passed |
| 7 Preview | replacements/imports, unique anchors, K2 candidate facts, protected bindings/type/diagnostic/effect checks | Passed by metamorphic tests |
| 8 Commit | worktree, compile/tests, trailers, CAS, recoverable ledger, idempotent retry | Passed by demo/integration path |
| 9 Parallel transactions | semantic replay, ReadSet/callee invalidation, WW and project-model conflicts | Passed by executable concurrency matrix |
| 10 Docs/benchmarks | docs, ADRs, smoke timing | Passed for MVP scope |

Known limitations are deliberately visible: FIR APIs are version-unstable, so all Kotlin/FIR internals remain isolated in the pinned 2.4.10 worker. Interprocedural slicing stops at explicit call boundaries, overload assignability is intentionally conservative, and Android/KMP project models are outside the first-version support contour.
