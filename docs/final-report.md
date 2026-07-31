# Final report

## Implemented

The repository contains a runnable Rust/Kotlin vertical covering inspect, index, resolve, CFG, SSA-style PHI/def-use, control dependencies, slicing, structured PSI-copy edits, preview, compiler/tests validation, detached worktree commits, CAS publication, ledger and typed failures.

## Gates and correctness evidence

`scripts/verify.sh` builds both toolchains, runs Rust unit/integration checks, Kotlin fixture tests, protocol handshake, deterministic inspect/index checks and the demonstration. The `total` slice includes `base`, `premium`, initial and conditional `value` definitions, PHI and return. Invalid Kotlin is rejected without ref movement; valid candidates compile and receive transaction trailers.

## Kotlin constructs

The fixture covers functions/members/parameters/locals, assignments, if/when, loops and jumps, return/throw/try/finally, calls/extensions/named/default arguments, safe calls/Elvis, properties, lambda capture and a suspend boundary. Calls are opaque summaries at call depth zero. Unsupported project forms and ambiguous source targets fail closed.

## Performance

The benchmark runner reports worker startup, inspect, index, CFG/slice and preview separately. The fixture is intentionally small and is not evidence for the 100k LOC SLO. Warm timings are recorded in `benchmarks/reports/latest.json` when `scripts/benchmark.sh` runs.

## Kotlin K2/FIR risks

Kotlin 2.4.10 compiler APIs include K1-deprecated environment setup and do not publish the standalone Analysis API as a supported Maven artifact. The worker opts into the narrow PSI bootstrap API and isolates graph export. Semantic validation uses the pinned K2 Gradle compiler. A next iteration should build a version-pinned standalone Analysis API/FIR distribution from JetBrains sources and replace the adapter behind the same DTO.

## Next stage

Add per-reference Ka symbols/types/call mappings, true FIR CFG golden coverage, semantic summaries with precise RW invalidation, content-addressed blobs, crash-injection recovery tests and corpus-scale SLO measurements. The language-neutral protocol/IR/transaction boundary is ready for a TypeScript worker without importing Kotlin types into Rust.
