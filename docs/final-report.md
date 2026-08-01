# Final report

## Implemented

The repository contains a runnable Rust/Kotlin vertical covering Gradle model inspection, indexing, K2 FIR semantic facts and CFG, dominance-frontier SSA/PHI/def-use, control dependencies, slicing, structured edits, preview validation, detached worktree commits, CAS publication, ledger and typed failures.

## Gates and correctness evidence

`scripts/verify.sh` builds both toolchains, runs unit, golden-language, metamorphic and real-worktree concurrency tests, checks typed/batch/blob protocol paths and deterministic inspect/index output, executes the 100k-LOC corpus gate, then runs the transaction demonstration. The `total` slice includes `base`, `premium`, initial and conditional `value` definitions, PHI and return. Invalid Kotlin is rejected without ref movement; valid candidates compile and receive transaction trailers. Repeating the same transaction is deduplicated by its transaction ID and Edit IR hash. Ledger inspection reconciles both post-publication crashes from reachable trailers and pre-CAS crashes from the validated candidate commit.

## Kotlin constructs

The fixtures cover functions/members/parameters/locals, assignments, if/when, loops and jumps, return/throw/try/finally, calls/extensions/named/default arguments, safe calls/Elvis, properties, lambda capture and suspend/Java boundaries. Each call-site normalizes to one CALL node with explicit CALL/RETURN/ARG_PARAM/RECEIVER edges and an opaque content-hashed summary at call depth zero. ReadSets include source/signature/symbol/type/call/summary/diagnostics/inheritance/compiler/classpath facts; preview enforces semantic ExpectedWriteSet/ActualWriteSet scope and protected ABI.

## Performance

The stage benchmark separately reports worker startup, IPC plus PSI parse, warm one-file reindex, composite-anchor resolution, cold/first and warm K2 analysis, FIR extraction, SSA/control construction, slicing, canonical serialization, project-model loading, declaration indexing, cold/warm edit preview and Gradle validation. The recorded warm run is one-file reindex 27 ms, anchor 4 ms, K2 20 ms, FIR 43 ms and preview 460 ms. The separate generated corpus contains 100,002 Kotlin lines and its cold syntax/declaration index passes the 20 s SLO; the exact latest duration is kept in the machine-readable report. Reports live in `benchmarks/reports/latest.json` and `benchmarks/reports/corpus-100k.json`.

## Kotlin K2/FIR isolation

The worker launches the pinned Kotlin 2.4.10 K2 compiler with an in-distribution compiler plugin. That plugin exports resolved expression types, selected callable symbols, receivers, argument-to-parameter mappings, suspend effects, diagnostics, and the compiler's actual FIR CFG. Rust sees only versioned JSON/Protobuf DTOs and never imports FIR classes.

## Next stage

Extend the supported project contour to Android/KMP and add deeper interprocedural summaries. The language-neutral protocol/IR/transaction boundary is ready for another language worker without importing Kotlin types into Rust.
