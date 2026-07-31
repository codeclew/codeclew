# ADR-005: Semantic anchor

## Context
Offsets move after whitespace and neighboring edits.
## Decision
Combine owner SymbolId, syntax kind, normalized tokens, ancestor path, ordinal, contexts, exact hash and range hint. Replay accepts exactly one match.
## Alternatives considered
Offsets alone, fuzzy nearest match and globally unique text.
## Consequences
Local movement is tolerable while ambiguity is rejected.
## Failure modes
Zero matches is `STALE_TARGET`; multiple matches is `AMBIGUOUS_TARGET`.
## Compatibility implications
New discriminators may be optional; existing anchors retain strict semantics.

