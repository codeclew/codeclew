# ADR-002: Source of truth

## Context
PSI, graphs and indexes become stale independently.
## Decision
Only Git revision, exact source bytes and normalized project model hash are authoritative.
## Alternatives considered
Persisted PSI/FIR, offset identity, and index-as-authority.
## Consequences
Derived state is rebuildable and transactions have immutable bases.
## Failure modes
Any model/hash mismatch makes a transaction stale.
## Compatibility implications
New derived formats can be migrated or discarded without rewriting source truth.

