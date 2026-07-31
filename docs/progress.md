# Progress

| Stage | Gate | Status |
|---|---|---|
| 0 Bootstrap | Rust starts worker, handshakes and shuts down | Passed |
| 1 Project model | repeated canonical model/hash | Passed on `kotlin-basic` |
| 2 Declaration index | deterministic SQLite/WAL incremental update | Passed |
| 3 Semantic facts | PSI symbols plus K2 compilation diagnostics | Partial: compiler-backed validation; rich per-expression Ka facts deferred |
| 4 Local CFG | supported local constructs exported | Partial: PSI exporter; FIR adapter replacement isolated |
| 5 Rust graph | PHI, def-use, control dependencies, dominators | Passed for vertical fixture |
| 6 Slicer | bounded directions, Thread IR, ReadSet, explicit boundaries | Passed |
| 7 Preview | both edit operations, unique anchors, PSI copy, diff, K2 compile | Passed for vertical fixture |
| 8 Commit | worktree, compile/tests, trailers, CAS, ledger | Passed by demo/integration path |
| 9 Parallel transactions | hard same-target/project conflicts | Partial: conservative reslice; full semantic merge matrix deferred |
| 10 Docs/benchmarks | docs, ADRs, smoke timing | Passed for MVP scope |

Known limitations are deliberately visible: the public Kotlin distribution does not publish a stable standalone Analysis API artifact, FIR internals are version-unstable, and the current worker therefore uses PSI plus the actual K2 Gradle compiler as its validation oracle. Fine-grained protected binding and callee-summary MVCC are conservative rather than complete.

