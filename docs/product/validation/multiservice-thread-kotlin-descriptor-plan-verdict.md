# Independent Pre-Run Verdict — Kotlin Descriptor G1K

## Status

**PASS** — 2026-08-25.

This verdict permits the full G1K readiness run. It does not claim that G1K
has passed, and it does not permit S1K to begin before successful run evidence,
checked-evidence verification, and an independent result audit.

## Verified scope

- The v2 private runner and checked PASS verifier bind the frozen corpus and
  benchmark digests, 11 units, 10 tasks, 20 task sides, eight pairs, exact
  revisions, and oracle blob identities.
- A task side qualifies only through `PROVEN` compiler-backed K2 descriptors
  in approved files. Descriptor/relation boundaries do not qualify, and
  `SYNTAX_ONLY` is rejected for both boundary families.
- Each task requires approved-file minima on both members plus at least one
  callable descriptor and one type descriptor across the task.
- Manual-verification and resource-budget records are represented only as
  bound authorities. The gate does not claim that manual verification ran.
- Cross-repository relationships remain `DECLARED_TOPOLOGY`; the gate makes
  zero HTTP, Spring, endpoint-equivalence, or compatibility claims.
- Checked evidence is canonical and path/name/source/credential free. Private
  inputs and output require caller-owned regular files with mode `0600`.
- Actual CLI output is compact canonical JSON with its trailing LF included in
  the frozen 64 KiB limit. Structured exact-identity facts are selected before
  high-volume aggregate facts, while no-identity ordering and compilation-lane
  fairness remain deterministic.

## Verification evidence

- Runner and verifier self-tests, including negative mutations: `PASS`.
- Focused context, thread-context, session stdout-boundary, and CLI tests:
  `PASS`.
- Rust formatting, `clippy -D warnings`, diff integrity, compact-output probe,
  and private-locator leak scan: `PASS`.
- Independent audit: P0 = 0, P1 = 0.

## Explicit non-scope

- The pre-run portion of this document did not assert a full 11-unit G1K
  result. That result was subsequently produced and audited as recorded below.
- Historical v1 STOP artifacts are not valid v2 PASS evidence.
- S1K-S4K implementation and adoption claims are not covered.

## Post-Run Result Verdict

**PASS** — 2026-08-25.

The canonical checked v2 result at
`docs/plans/evidence/thread-kotlin-descriptor-gate.json` records 11/11 ready
units, 10/10 covered tasks, 20/20 qualified task sides, eight distinct service
pairs, 89 callable descriptors, 64 type descriptors, and zero failures. Every
qualifying side is backed by `PROVEN` K2 descriptor evidence. Boundaries and
syntax-only evidence do not qualify; all ten cross-repository relationships
remain `DECLARED_TOPOLOGY`; HTTP claims remain zero.

An independent read-only result audit reran the checked verifier and its
negative self-test, recomputed the unit/task/side aggregates and all frozen
authority digests, checked private-to-checked binding and file modes, matched
execution authority to the current `./clew` bytes, and scanned checked string
values for private identifiers. It returned P0=0 and P1=0. This post-run
verdict closes G1K and permits S1K; it does not assert any S1K result.
