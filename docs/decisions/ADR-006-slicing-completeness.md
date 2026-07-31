# ADR-006: Slicing completeness

## Context
Budgets, external calls and unsupported syntax prevent complete static slices.
## Decision
Every slice has a completeness enum and evidence boundaries. PHI inputs are intrinsic to DEF_USE traversal.
## Alternatives considered
Best-effort graphs labeled complete and unbounded analysis.
## Consequences
Consumers can distinguish sound supported subsets from partial context.
## Failure modes
Deadline/node limits produce `PARTIAL_BUDGET`; depth-zero calls produce `PARTIAL_EXTERNAL_BOUNDARY`.
## Compatibility implications
New boundary kinds do not change existing status meaning.

