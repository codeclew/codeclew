# ADR-004: Thread IR and Edit IR

## Context
Semantic context and source mutation have different safety properties.
## Decision
Thread IR is immutable language-neutral evidence; Edit IR contains narrow source-backed operations and is never a code printer.
## Alternatives considered
Whole-file generation, unified diff transactions and AST round-tripping.
## Consequences
Intent, evidence and writes can be audited separately.
## Failure modes
Unknown operations, mismatched thread IDs and exceeded WriteSets fail validation.
## Compatibility implications
Both schemas carry `semantic-*/0.1` versions and canonical JSON debug forms.

