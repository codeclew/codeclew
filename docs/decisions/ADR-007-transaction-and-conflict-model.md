# ADR-007: Transaction and conflict model

## Context
Agents edit concurrently while compilation is expensive.
## Decision
Use immutable snapshots, ReadSet/WriteSet evidence, detached worktrees, validation before publication, and Git `update-ref` compare-and-swap.
## Alternatives considered
Long repository locks, direct branch mutation and post-commit validation.
## Consequences
Failures before CAS cannot move the branch and conflicts are explicit.
## Failure modes
Changed targets are WW conflicts; changed project models require reslicing; final ref movement fails CAS.
## Compatibility implications
Ledger states and commit trailers are durable protocol surfaces.

