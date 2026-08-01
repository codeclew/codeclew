# Final report

## Implemented

The repository contains a runnable Rust/Kotlin vertical covering Gradle compilation inspection, immutable repository-index snapshots, K2 FIR semantic facts and CFG, AST/type/memory edges, dominance-frontier SSA/PHI/def-use, control dependencies, slicing, structured edits, preview validation, detached worktree commits, CAS plus index publication, ledger, contextual typed failures, and the separate long-lived Rust `semanticd` service with metrics.

## Gates and correctness evidence

`scripts/verify.sh` builds both toolchains, runs unit, golden-language, metamorphic, daemon, and real-worktree concurrency tests, checks typed/batch/blob protocol paths and deterministic inspect/index output, executes the 100k-LOC corpus gate and real p95 benchmark, then runs the transaction demonstration. The `total` slice includes `base`, `premium`, initial and conditional `value` definitions, PHI and return. Invalid Kotlin is rejected without ref movement; valid candidates compile and receive transaction trailers. Repeating the same transaction is deduplicated by transaction ID and Edit IR hash and also repairs missing post-CAS index publication. Ledger inspection reconciles both post-publication crashes from reachable trailers and pre-CAS crashes from the validated candidate commit.

## Kotlin constructs

The fixtures cover functions/members/parameters/locals, assignments, if/when, short-circuit branching, loops and jumps, return/throw/try/finally, calls/extensions/named/default arguments, safe calls/Elvis, properties, lambda capture and suspend/Java boundaries. Each call-site normalizes to one CALL node with explicit CALL/RETURN/ARG_PARAM/RECEIVER edges and an opaque content-hashed summary at call depth zero. The executable graph includes AST_CHILD/TYPE plus LOCAL/THIS_PROPERTY/OBJECT_PROPERTY/STATIC_PROPERTY/UNKNOWN_HEAP abstractions. ReadSets include source/signature/symbol/type/call/summary/diagnostics/inheritance/compiler/classpath facts; preview enforces semantic ExpectedWriteSet/ActualWriteSet scope and protected ABI. Symbol IDs carry the complete language-neutral identity and a JVM descriptor; legacy FQNs are lookup aliases only.

## Performance

The stage benchmark uses an isolated cache-clean fixture and 20 samples per p95. Every changed-file reindex is K2-semantic and changes source content; every edit preview uses a distinct candidate after a ready slice. The current report records semantic reindex p95 246 ms, anchor p95 12 ms, resolve p95 30 ms, local CFG p95 40 ms, first preview 152 ms and preview p95 38 ms. Cold semantic indexing (1.56 s) and Gradle validation (2.56 s) are reported separately. The generated corpus contains 100,002 real Kotlin lines and its cold syntax/declaration index passes the 20 s SLO in 7.72 s. Reports live in `benchmarks/reports/latest.json` and `benchmarks/reports/corpus-100k.json`.

## Kotlin K2/FIR isolation

The worker invokes the pinned Kotlin 2.4.10 K2 compiler in-process with the selected Gradle compilation's language/API versions, JVM target, free arguments, opt-ins, friend paths, classpath and project compiler plugins, plus the in-distribution facts plugin. Cache identity includes the content of all those artifacts and options. The plugin exports resolved expression types, selected callable symbols, receivers, argument-to-parameter mappings, suspend effects, diagnostics, and the compiler's actual FIR CFG. Rust sees only versioned JSON/Protobuf DTOs and never imports FIR classes.

## Next stage

Extend the supported project contour to Android/KMP and add deeper interprocedural summaries. The language-neutral protocol/IR/transaction boundary is ready for another language worker without importing Kotlin types into Rust.
