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

## Historical Kotlin engineering sample

The 2026-09-01 sample at source revision
`6281138ecbf73bc5de1a9c7eaeb2cdf7009e6ca1` examined five manually selected Kotlin
engineering concerns. All 16 focused checks passed without skips; their warm
run took 784 ms. A comparable lexical search returned 160 matching lines.
One preserved released navigation receipt supplied exact source, compiler
identity and evidence digests. A fresh attempt stopped with `RESOURCE_LIMIT`
before returning facts and was excluded from the positive sample. These are
historical sample facts, not general agent-success or token-reduction estimates.

The generated sample report and PDFs have been removed from the current tree.
The [full historical sample](https://github.com/codeclew/codeclew/blob/f0b874f5f8e0d6e5b96a2f5f65592c7c4e07cb0a/benchmarks/marketing/kotlin-evidence-sample-v1.json)
remains available at its archived revision; public evidence pages link the
methodology PDF at that same immutable revision. The PDF generator and source
planning documents remain in the repository.
