# Practical warm-path dogfood — 2026-08-28

Date: 2026-08-28

## Purpose

This is an internal product check, not a publication benchmark. It asks whether
an agent can obtain useful, bounded evidence from already indexed local
repositories. Cold-start duration, corpus-wide scoring, and model comparisons
are deliberately out of scope.

## Accepted cases

### 1. Exact single-repository context

- Runtime: `RELEASE`; Kotlin compilation: explicit `:/main`.
- The repeated warm request completed in 10.8 seconds.
- It returned 12 bounded facts from two source files.
- The requested class and entrypoint were independently confirmed as exact
  `K2_FIR` facts with `PROVEN` resolution.
- The warm result was byte-identical to the priming result and emitted no cold
  stage events.
- Overall generation certainty remained `UNSURE` because unrelated partial CFG
  boundaries were retained. This did not change the exact status of the
  requested declaration facts.

### 2. Exact two-repository context

- Two clean, doctor-admitted local repositories were bound into one immutable,
  read-only thread.
- The warm thread context completed in 25.1 seconds and emitted no cold stage
  events.
- It returned 16 `K2_FIR` facts with `PROVEN` resolution, split evenly between
  the two members.
- The expected client endpoint and service endpoint were independently found in
  their respective members.
- Composite identifier terms remained listed as unmatched even though the exact
  endpoints were present. This is retained as a query-usability limitation; it
  is not treated as proof of a cross-service call edge.

### 3. Honest fallback

- A request for a non-semantic repository artifact completed in 10.9 seconds.
- It returned no semantic match, certainty `UNSURE`, and blocked publication.
- The result retained both `UNSURE_GENERATION_AUTHORITY` and
  `VERIFY_LEXICAL_SOURCE_SELECTION` obligations.
- The named lexical check was then performed against the same bound Git
  revision: the selected artifact was tracked, non-empty, YAML-shaped, and
  lexically relevant.
- Completing that check did not promote the stored Codeclew evidence to
  compiler-backed or exact authority.

## Product verdict

Codeclew is useful on the tested warm contour for bounded fact retrieval:

1. it reuses an immutable generation without rebuilding the language adapter;
2. it can compose exact facts from two repositories within the 30-second
   interactive target; and
3. it fails honestly when semantic evidence is unavailable and names the check
   an agent must perform next.

This evidence does not qualify cold start, arbitrary natural-language requests,
mutation, publication, or a general cross-service call graph. The next product
slice should improve how an agent consumes and records these mixed exact and
conditional facts, rather than adding a broad benchmark harness.

## v0.2.4 release gate

- Canonical capabilities reported product version `0.2.4`, runtime mode
  `RELEASE`, and status `PILOT_READY`.
- `cargo test --workspace` passed 420 tests. Two separately gated real-worker
  acceptance probes remained explicitly ignored by the unit suite.
- The bounded language-mutation pilot passed 6/6 isolated cases in `RELEASE`
  mode: 3/3 Rust and 3/3 Python. Each case completed native validation and
  explicit conditional publication.
- Repository CI passed on the tagged revision, including the history privacy
  gate and the complete `ci-verify.sh` suite.
- The release workflow repeated the conditional-mutation qualification and
  published checksum-backed macOS arm64 and x86_64 bundles.

These checks qualify the published small release; they do not turn the three
warm-path cases above into a publication benchmark.

## Next product priority

The next product priority is the evidence-native Development Record. Its first
implementation slice is the minimal M1 `MissionAuthority`/`ChangeSpec` foundation;
M1 is not a standalone product detour. It immediately enables M2 to present
claim-level authority so an agent can distinguish exact requested facts from
unrelated partial boundaries, retain named verification obligations, and carry
the resulting checked record into implementation and documentation.

This is higher leverage than immediately adding another language: the current
Kotlin, Rust, and Python contours already produce useful evidence, while the
dogfood showed that consuming mixed `PROVEN` and `UNSURE` facts remains the main
agent-facing usability gap.

## Retained limitations

- Aggregate context certainty can be `UNSURE` while individual requested facts
  are exact. Agent-facing output needs a clearer per-claim certainty summary.
- Composite identifier tokenization may report unmatched terms alongside exact
  endpoint facts.
- The one-time generation build remains expensive but was not evaluated here.
