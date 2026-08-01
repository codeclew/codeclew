# Benchmarks

Run `scripts/benchmark.sh` for separately instrumented worker startup, IPC plus PSI parse, anchor resolution, cold/first and warm K2 analysis, FIR extraction, Rust SSA/control dependencies, slicing, canonical serialization, cold-cache and warm edit preview, and Gradle validation. Warm measurements exercise the content-addressed cross-process caches required by the SLO table; cold misses are retained explicitly instead of being hidden.

Run `scripts/benchmark-corpus.sh` for the non-extrapolated cold syntax/declaration gate. It generates 100,002 Kotlin lines, indexes them in `--syntax-only` mode, records the measured duration and fails the report field when the 20-second SLO is exceeded.
