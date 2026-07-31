# ADR-008: Validation policy

## Context
Parsing alone cannot prove Kotlin type or overload correctness.
## Decision
Validate replacement PSI in memory, then compile the isolated candidate with Kotlin 2.4.10 K2 and run configured tests before CAS.
## Alternatives considered
Parse-only checks, compile-only evidence and validation after publication.
## Consequences
Preview is slower but rejects syntax/type/binding diagnostics without source mutation.
## Failure modes
Parse, compile, test, effect and WriteSet failures have distinct typed codes.
## Compatibility implications
Additional validators append evidence without weakening current gates.

