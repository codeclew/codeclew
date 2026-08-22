# Benchmarks

`../scripts/benchmark.sh` measures only the supported managed workflow through
`./clew`: launcher reuse, session creation, and content-addressed context reuse.
Generated reports live under `benchmarks/reports/`, which is ignored by Git
because diagnostic output may contain host-specific measurements.

`../scripts/cold-multicore-gate.sh` measures the two-lane RELEASE runtime build
(Cargo plus all Kotlin worker distributions). It compares three counterbalanced
serial/parallel pairs, requires byte-identical runtime, artifact, and worker
digests, and retains the median wall-time ratio in
`reports/cold-multicore-latest.json`.

`../scripts/multi-compilation-gate.sh` separately measures one twelve-module
Kotlin repository generation with `generation-jobs=1` and the host-admitted
parallel lane count. It requires byte-identical per-compilation generations,
aggregate authority, facts, and completeness, plus one snapshot capture and the
declared shared-model request contour. Its evidence is stored in
`reports/multi-compilation-latest.json`.

Each release gate requires at least four physical cores and four admitted jobs.
Smaller hosts produce `SKIPPED_UNQUALIFIED_HOST`; a skip is accepted for local
verification but is never represented as a release-gate pass.

Release comparisons use fresh temporary repositories and separate cold, warm,
incremental, recovery, and unchanged-hit results. Historical E04/K1 harnesses
are not part of the product or benchmark contour.
