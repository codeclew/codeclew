# Benchmarks

`../scripts/benchmark.sh` measures only the supported managed workflow through
`./clew`: launcher reuse, session creation, and content-addressed context reuse.
Generated reports live under `benchmarks/reports/`, which is ignored by Git
because diagnostic output may contain host-specific measurements.

`../scripts/cold-multicore-gate.sh` runs the release multicore gates over pinned
Codeclew source bytes. It compares three paired `jobs=1` and `jobs=N` samples
for the two-lane runtime DAG and a twelve-compilation generation DAG, requires
byte-identical output digests, and retains the median wall-time ratios in
`reports/cold-multicore-latest.json`. A separate K24-monolith-shaped run records
work/span evidence and must report exactly one sealed compiler stream. Hosts
with fewer than eight physical cores produce `SKIPPED_UNQUALIFIED_HOST`; their
smoke measurements are retained but never represented as a release-gate pass.

Release comparisons use fresh temporary repositories and separate cold, warm,
incremental, recovery, and unchanged-hit results. Historical E04/K1 harnesses
are not part of the product or benchmark contour.
