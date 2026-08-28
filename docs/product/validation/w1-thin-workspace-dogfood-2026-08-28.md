# W1 thin Workspace dogfood — 2026-08-28

## Scope

This is a bounded internal product check, not a publication benchmark. It used
two clean local repositories with independent sessions: one compiler-backed
Kotlin/Maven member and one syntax-backed Python member. Repository identity,
absolute paths, source, and managed IDs remained in private state.

The catalog declared one directional relationship. It did not claim compiler
shape, artifact ownership, contract verification, or observed runtime authority.

## Accepted facts

- Reversing catalog member order reopened the same content-addressed workspace.
- The workspace bound exactly two distinct repositories, the mission identity,
  the ChangeSpec digest, and one declared edge.
- The edge reported `DECLARED_CATALOG` topology while all four independent
  semantic/runtime axes remained `UNKNOWN`.
- One workspace context returned five ranked facts and six source windows under
  the existing shared thread budget. Both required member aliases had retained
  evidence authority: required-member recall was 2/2 (100%) with zero critical
  miss on this frozen case.
- The result was honestly `PARTIAL/UNSURE` with one unmatched broad term. The
  workspace did not promote that result because another member had compiler
  facts.
- Canonical stdout was 55,931 bytes, below the shared 64 KiB limit.
- Repeating the same request returned a byte-identical result. The first
  successful request completed in about 31 seconds and the repeat in about 18
  seconds; timing is diagnostic only.
- Closing the workspace left both member session statuses and authority digests
  unchanged. The sessions remained `OPEN` until explicitly closed later.

## Dogfood fallback finding

An earlier attempt paired the Kotlin member with a Rust workspace whose local
`Cargo.lock` existed but was not tracked by the bound Git revision. Rust model
extraction rejected it as `UNSUPPORTED_PROJECT_CONFIGURATION`. The composite
context was not published partially, while the already-created Kotlin member
context and generation remained reusable. The successful retry replaced only
the unsupported member and reused the Kotlin work instead of restarting the
analysis from zero.

This is useful fail-closed behavior and a separate usability finding: a future
Rust slice should surface the tracked-lock prerequisite at session open rather
than waiting for the first context request. It is not required to expand W1.

## Honest limits

- W1 composes existing per-repository facts; it does not prove that the declared
  edge exists at compile time or runtime.
- The small frozen case establishes product mechanics and member recall, not a
  general quality percentage.
- Workspace mutation, coordinated preparation, and publication remain outside
  W1. Closing a workspace affects only its private analysis view.
