# Benchmarks

`../scripts/benchmark.sh` measures only the supported managed workflow through
`./clew`: launcher reuse, session creation, and content-addressed context reuse.
Generated reports live under `benchmarks/reports/`, which is ignored by Git
because diagnostic output may contain host-specific measurements.

Release comparisons use fresh temporary repositories and separate cold, warm,
incremental, recovery, and unchanged-hit results. Historical E04/K1 harnesses
are not part of the product or benchmark contour.
