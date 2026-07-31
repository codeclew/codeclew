# ADR-009: Storage and cache invalidation

## Context
Indexes must be deterministic, incremental and crash-safe.
## Decision
Store canonical facts in SQLite WAL, keyed by normalized path/SymbolId and hashes. Treat project model changes as compilation-wide invalidation.
## Alternatives considered
In-memory-only indexes, serialized compiler objects and timestamp invalidation.
## Consequences
Single-file updates are transactional and index hashes are reproducible.
## Failure modes
SQLite transaction failure preserves the prior snapshot; model mismatch marks transactions stale.
## Compatibility implications
Schema migrations may rebuild derived tables from source truth.
