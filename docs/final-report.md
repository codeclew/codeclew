# Final report

## Implemented

The repository contains a runnable Rust/Kotlin vertical covering Gradle model inspection, indexing, K2 FIR semantic facts and CFG, dominance-frontier SSA/PHI/def-use, control dependencies, slicing, structured edits, preview validation, detached worktree commits, CAS publication, ledger and typed failures.

## Gates and correctness evidence

`scripts/verify.sh` builds both toolchains, runs unit, golden-language, metamorphic and real-worktree concurrency tests, checks the protocol handshake and deterministic inspect/index output, then runs the transaction demonstration. The `total` slice includes `base`, `premium`, initial and conditional `value` definitions, PHI and return. Invalid Kotlin is rejected without ref movement; valid candidates compile and receive transaction trailers.

## Kotlin constructs

The fixtures cover functions/members/parameters/locals, assignments, if/when, loops and jumps, return/throw/try/finally, calls/extensions/named/default arguments, safe calls/Elvis, properties, lambda capture and suspend/Java boundaries. Calls are opaque, content-hashed summaries at call depth zero. Unsupported project forms and ambiguous source targets fail closed.

## Performance

The benchmark runner reports worker startup, inspect, index, CFG/slice and preview separately. The fixture is intentionally small and is not evidence for the 100k LOC SLO. Warm timings are recorded in `benchmarks/reports/latest.json` when `scripts/benchmark.sh` runs.

## Kotlin K2/FIR isolation

The worker launches the pinned Kotlin 2.4.10 K2 compiler with an in-distribution compiler plugin. That plugin exports resolved expression types, selected callable symbols, receivers, argument-to-parameter mappings, suspend effects, diagnostics, and the compiler's actual FIR CFG. Rust sees only versioned JSON/Protobuf DTOs and never imports FIR classes.

## Next stage

Extend the supported project contour to Android/KMP, add deeper interprocedural summaries and corpus-scale SLO measurements. The language-neutral protocol/IR/transaction boundary is ready for another language worker without importing Kotlin types into Rust.
