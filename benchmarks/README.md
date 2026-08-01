# Benchmarks

Run `scripts/benchmark.sh` for separately instrumented worker startup, IPC plus PSI parse, anchor resolution, K2 analysis, FIR extraction, Rust SSA/control dependencies, slicing, canonical serialization, edit preview, and Gradle validation. Corpus-scale SLO validation remains distinct; the small fixture protects against gross regressions and reports stage attribution rather than claiming 100k-LOC evidence.
