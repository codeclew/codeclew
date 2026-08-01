# Final report

## Implemented

The repository contains a runnable Rust/Kotlin vertical covering Gradle compilation inspection, immutable repository-index snapshots, K2 FIR semantic facts and CFG, AST/type/memory edges, dominance-frontier SSA/PHI/def-use, control dependencies, slicing, structured edits, preview validation, detached worktree commits, CAS plus index publication, ledger, contextual typed failures, and the separate long-lived Rust `semanticd` service with metrics.

## Gates and correctness evidence

`scripts/verify.sh` builds both toolchains, runs unit, golden-language, metamorphic, daemon, and real-worktree concurrency tests, checks typed/batch/blob protocol paths and deterministic inspect/index output, executes the 100k-LOC corpus gate and real p95 benchmark, then runs the transaction demonstration. The `total` slice includes `base`, `premium`, initial and conditional `value` definitions, PHI and return. Invalid Kotlin is rejected without ref movement; the affected compilation and configured snapshot tests run automatically. A failing-test integration case proves target ref, worktree list, and published index hash remain unchanged. K2 facts and SQLite changes are fully staged before CAS; one atomic rename publishes the index, with inverse CAS on publication failure. Recovery never records `COMMITTED` from a trailer until the matching index is published, and late inspection/retry of an ancestor transaction always preserves or rebuilds the index for the current target HEAD.

## Kotlin constructs

The fixtures cover functions/members/parameters/locals, assignments, if/when, short-circuit branching, loops and jumps, return/throw/try/finally, calls/extensions/named/default arguments, safe calls/Elvis, properties, lambda capture and suspend/Java boundaries. Each call-site normalizes to one CALL node with explicit CALL/RETURN/ARG_PARAM/RECEIVER edges and an opaque content-hashed summary at call depth zero. The executable graph includes AST_CHILD/TYPE plus LOCAL/THIS_PROPERTY/OBJECT_PROPERTY/STATIC_PROPERTY/UNKNOWN_HEAP abstractions; `box.field` is conservatively modeled as unknown heap with read/write dependencies. ReadSets include source/signature/symbol/type/call/summary/diagnostics/inheritance/compiler/classpath facts; preview enforces semantic ExpectedWriteSet/ActualWriteSet scope and protected ABI. Symbol IDs carry K2-resolved inferred types and JVM descriptors verified against `javap` for collections, boxed generic arrays, bounded type parameters, and extensions. Legacy FQNs are lookup aliases only; public lookup of a full JSON SymbolId compares the complete canonical identity and rejects any tampered field.

## Performance

The stage benchmark uses an isolated cache-clean fixture and 20 samples per p95. Every changed-file reindex is K2-semantic and changes source content; every edit preview uses a distinct candidate after a ready slice. The current report records semantic reindex p95 260 ms, anchor p95 20 ms, resolve p95 27 ms, full FIR CFG + Rust graph + SSA p95 41 ms, first preview 154 ms and preview p95 37 ms. Independent 20-sample instrumentation reports IPC 51 µs p95, Rust protocol serialization 2 µs, PSI parse 113 µs, cold K2 compiler analysis 1.467 s, changed-file K2 analysis 208.286 ms p95, FIR-plugin extraction 1.491 ms, Rust graph construction 86 µs, SSA/control 1.903 ms, slicing, and edit validation. Gradle compile (2.56 s) and tests (2.63 s) are measured separately. The generated 100,002-LOC corpus passes its independent cold syntax/declaration SLO in 8.14 s. Reports live in `benchmarks/reports/latest.json` and `benchmarks/reports/corpus-100k.json`.

## Kotlin K2/FIR isolation

The selected worker invokes a pinned Kotlin 2.1.21 or 2.4.10 K2 compiler in-process with the Gradle compilation's language/API versions, JVM target, free arguments, opt-ins, friend paths, classpath and project compiler plugins, plus the matching in-distribution facts plugin. Cache identity includes the compiler version and the content of all those artifacts and options. The plugin exports resolved expression types, selected callable symbols, receivers, argument-to-parameter mappings, suspend effects, diagnostics, and the compiler's actual FIR CFG. Rust sees only versioned JSON/Protobuf DTOs and never imports FIR classes.

## Next stage

Extend the supported project contour to Android/KMP and add deeper interprocedural summaries. The language-neutral protocol/IR/transaction boundary is ready for another language worker without importing Kotlin types into Rust.
