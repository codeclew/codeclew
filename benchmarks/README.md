# Benchmarks

Run `scripts/benchmark.sh` for an isolated cache-clean fixture and 20-sample p95 measurements. Changed-file reindex performs K2 semantic analysis for a distinct source revision on every sample (never `syntaxOnly`). Edit preview uses 20 distinct, semantically equivalent candidates after a ready slice, so no candidate result can be served from a persistent cache. The report keeps the first ready-slice preview, p95 values, cold semantic index, and Gradle validation separately and fails if any applicable p95 SLO is exceeded.

Run `scripts/benchmark-corpus.sh` for the non-extrapolated cold syntax/declaration gate. It generates 100,002 Kotlin lines, indexes them in `--syntax-only` mode, records the measured duration and fails the report field when the 20-second SLO is exceeded.
