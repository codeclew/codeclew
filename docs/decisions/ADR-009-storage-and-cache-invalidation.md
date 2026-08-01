# ADR-009: Storage and cache invalidation

## Context
Indexes must be deterministic, incremental and crash-safe.
## Decision
Store canonical facts in compilation-scoped SQLite, keyed by normalized path/SymbolId and hashes. Build each post-commit snapshot in a private copy, checkpoint it into a self-contained database, and atomically rename it only after the target-ref CAS. Roll the ref back if publication fails. Treat project model changes as compilation-wide invalidation.
## Alternatives considered
In-memory-only indexes, serialized compiler objects and timestamp invalidation.
## Consequences
Single-file updates are transactional, index hashes are reproducible, and fallible K2/SQLite work cannot leave the target branch ahead of its index snapshot.
## Failure modes
Pre-CAS K2/SQLite failure preserves branch and index. A post-CAS rename failure triggers inverse CAS; a competing ref movement produces `TRANSACTION_RECOVERY_REQUIRED`. Model mismatch marks transactions stale.
## Compatibility implications
Schema migrations may rebuild derived tables from source truth.
