# Authored Markdown and field tables

Explanation bodies, readable behavior text and domain descriptions support pipe
Markdown tables, inline code, links and emphasis. Tables require a header and a
matching delimiter row. Fenced code, exact evidence, JSON and source excerpts
remain literal. Native/full Pages and MDX share one escaping/parser implementation.
Raw HTML is escaped; unsafe URL schemes remain text. A malformed table is shown
as prose, preserving its content. Summaries retain their existing plain-prose
contract and size limits. Explanations retain their source/step bindings and
8192-byte limit; backticks and less-than signs are now allowed in those bodies.

Data-model plans may supply optional `readableName` and `originalName` claims on
each field. Each uses the same evidence-bound block shape as `meaning`: `title`,
single-line `text` of at most 256 UTF-8 bytes and exact `evidence` references.
They remain authored interpretations with the plan's authority and review state.
`readableName` is never inferred from a generic `meaning.title`. `originalName`
explicitly binds an exact source spelling when the existing `path` also contains
a representation qualifier; the original path and comment remain untouched.

The primary field table presents original spelling, readable name and complete
comment. Missing labels/comments are explicit. Evidence and representation
details retain the existing `field-ID` anchors behind accessible disclosures;
field, source and Data Flow navigation remain separate. Existing plans without
these optional claims remain readable. Render new examples with their actual
producer identity; do not rewrite frozen historical output or provenance.
