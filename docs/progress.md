# Progress

| Stage | Gate | Status |
|---|---|---|
| 0 Bootstrap | Rust starts worker, handshakes and shuts down | Passed |
| 1 Project model | repeated canonical model/hash | Passed on `kotlin-basic` |
| 2 Declaration index | complete §11 facts, compilation-scoped SQLite/WAL incremental update and typed invalidation | Passed, including 100k-LOC corpus gate |
| 3 Semantic facts | K2 types, strict full SymbolId, compiler/JVM-verified descriptors, call targets, receivers, argument mappings, diagnostics | Passed by FIR-plugin golden and negative identity tests |
| 4 Local CFG | actual K2 FIR CFG exported and normalized | Passed for short-circuit/safe-call/Elvis true-false, loop, exception, call and Java-boundary fixtures |
| 5 Rust graph | AST/type/all memory abstractions (including arbitrary receiver `UNKNOWN_HEAP`), local-only dominance-frontier SSA, PHI, def-use, post-dominator control dependencies | Passed by golden and permutation tests |
| 6 Slicer | bounded directions, Thread IR, full semantic ReadSet, explicit boundaries | Passed |
| 7 Preview | replacements/imports, unique anchors, K2 candidate facts, protected bindings/type/diagnostic/effect/ABI/WriteSet checks | Passed by metamorphic tests |
| 8 Commit | immutable base index, affected compile/configured tests by default, trailers, pre-CAS staged index, ref CAS + atomic rename/rollback, recovery, idempotent repair | Passed by C14/default-test integration paths |
| 9 Parallel transactions | semantic replay, ReadSet/callee invalidation, WW and project-model conflicts | Passed by executable concurrency matrix |
| 10 Daemon/observability | separate long-lived Rust semanticd, structured logs, worker-reported real cache hits, orphan/Gradle metrics, contextual typed errors | Passed by semanticd success/failure metric tests and demo |
| 11 Docs/benchmarks | docs, ADRs, isolated 20-sample semantic p95 with separate mandatory stage profiles and 100k-LOC corpus | Passed for MVP scope |
| 12 Maven Kotlin/JVM | single-module effective POM/classpath/plugins, exact Kotlin 2.3 worker, bounded context, detached Maven compile/test/commit | Passed by six Maven integration tests and independent review |
| 13 Product-repo agent benchmark | blind default/ast-index/Clew run plus strict no-recipe rerun with token telemetry and fresh hidden acceptance | Passed: generic Clew beats ast-index on edit/commit time, calls and tokens; 109/109 hidden tests |
| E03 Typed change understanding | authority-backed `MAP_EDGE_WITH_CONTEXT` binding and invariant computation from live Kotlin 2.1 evidence | Implemented: public `clew prove` returns `BOUND`, bounded `AMBIGUOUS`, or fail-closed `REFUSED`; independent verdict is recorded in the E03 report |
| E04 product semantic materialization | compile a live E03 proof into a worker-owned Kotlin change and atomic commit | Implemented for the narrow direct `List<T>`/Gradle contour: public `clew apply` commits one K2-checked change; ambiguity, refusal, dirty evidence and divergent target revisions fail without mutation. This is not the frozen blind-benchmark E04 node. |
| E04-S0 blind binder experiment | 42 tasks × default/ast-index/Codeclew with native token telemetry and hidden judge | Executed and retained 126/126 runs, but independently rejected as `INFRA_ERROR` (`infrastructure-invalid`) / `NO_DECISION`: shell-wrapped tool calls were mis-audited, AST evidence was checked after repository teardown, build caches were sandbox-denied and oracle classes were undefined. This is a non-node diagnostic attempt; frozen graph E04 is not accepted and GE1/E05 remain closed. |

Known limitations are deliberately visible: FIR APIs are version-unstable, so all Kotlin/FIR internals remain isolated in exact 2.1.21, 2.3.0, and 2.4.10 workers. Interprocedural slicing stops at explicit call boundaries, overload assignability is intentionally conservative, and Android/KMP/multi-module Maven project models are outside the supported contour. The first large-repository win is one task on one repository, not yet a statistical generalization; evidence size and generic plan assembly remain optimization gates.
