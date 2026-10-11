# Source-bound process threads

A saved process may contain `process.thread` to describe a bounded selection of
retained Kotlin or Java methods. This is a declared interpretation, not automatic
program slicing, a compiler-proven call graph, or an observed execution trace.
`docs render --snapshot` validates and renders it without reading a checkout or
invoking a model. The input remains the normal saved process declaration.

## Input and validation

The thread contains `scope`, `rootMethod`, ordered `methods`, hierarchical `steps`
and `boundaries`. Each method has a unique ID, label, service, exact entrypoint,
source ID and text digest. Each step has a unique ID, title, text, selected method
and original inclusive line range; optional `target` selects another method.
`children` group explanations. They do not establish execution order or control
flow. Optional `kind` is one of `STEP`, `ACTION`, `DECISION`, `ALTERNATIVE`, `ERROR`
or `BOUNDARY`. Missing kinds remain `STEP`; titles are never parsed into control
semantics. Kind labels are declarations and must be reviewed against source.

Validation checks revision, source digest, entrypoint ownership, complete
method-only spans, exact selected process root, citation bounds and selected
method use. Conservative syntax checks reject unsupported or clipped bodies,
unbalanced delimiters, class spans and unrelated members. Unsupported spans fail
rendering rather than publishing a partial source section. Limits are 64 methods,
256 steps, 16 nested levels, 256 KiB per method and 1 MiB of source per thread.
These are presentation limits, not semantic completeness guarantees.

An optional `branchGroup` declares `IF_ELSE`, `FIRST_MATCH` or `INDEPENDENT`
arms. Each conditional arm has a nonempty condition, guard citation and steps;
an explicit default has `condition: null`, no guard and must be last. IF_ELSE
requires exactly one conditional arm and its default. Independent arms have no
default. Explanatory `children` and branch groups cannot coexist on one step.
The renderer does not derive exclusivity, exhaustiveness or path feasibility from
source: `declaredBranchCoverage` stays `NOT_PROVEN`.

Optional `effects` carry unique IDs, labels, kind and a citation. Kinds are
STATE_ASSIGNMENT, STORAGE_CALL, MESSAGE_SEND, EXTERNAL_CALL, RETURN, THROW and
TRANSACTION_SCOPE. They describe selected declared effects, not every possible
effect. Machine checks admit source references and shape, not the business label.
`selectedEffectCompleteness` stays `NOT_PROVEN`. Calls never imply commit or
delivery. Arm/effect IDs share the step ID namespace; the 256-item limit counts
steps, arms and effects together. Old declarations omit both optional fields.

The optional input preserves existing saved processes without a thread. Readers
that use older closed schemas may reject the new field; qualify the producer and
consumer together. Renderer freshness includes the thread implementation and
reader assets.

## Reader behavior

The process opens with its summary, trigger, participants and selected-source
count. The reader distinguishes accepted overview text from a summary supplied
in the declaration. Requested outcomes retain their declared meaning; they are not automatically
displayed as proven program results.

Source-backed steps retain their declared reading order as unnumbered cards.
Each shows its service, explicit kind, explanation and exact citation. Child
headings remain visible before expansion. Targets whose selected methods belong
to different services appear in a declared-connections index and carry an
unverified-delivery label. The index also shows explicitly selected saved interaction declarations, their
transport/configuration declarations and independent unresolved checks. It never
infers a relation from adjacent cards, names or a shared service pair. It is not
topology discovery and does not establish transport or routing.

All complete selected methods appear in one initially collapsed source section,
with file, revision and original line numbers. Citation links reveal it, focus
and highlight the selected lines. Deep links restore the process after catalogue
or coverage navigation. Evidence outside the thread remains available through
its existing source context; it is not rebound to selected methods.

Thread scope and boundaries stay visible. Publication-wide diagnostic codes are
preserved in a separate collapsed section. Additional state diagrams, accepted
visuals and local source outlines remain accessible as supporting material.
Markdown keeps declared steps, citation links, boundaries and complete selected
source bodies together; its source fences tolerate embedded backticks.

## Meaning and remaining work

Exact source binding does not validate prose, infer business intent, resolve
call targets or prove structural FLOW coverage. It does not establish database
commit, message delivery, rollback, routing, runtime configuration or a strict
state machine. Empty declared boundaries do not mean complete evidence.

To improve explanations, author concrete trigger, inputs, decisions, alternatives,
selected state changes, normal outcomes and errors, each with source evidence.
Keep generic status updates separate from validated transition policies. Model
proposals still require deterministic binding/coverage checks and semantic review.

The next semantic slice needs explicit branch groups and exception/effect
frontiers backed by qualified language/framework facts, not inference from titles
or indentation. Declared branch groups now provide the presentation contract;
compiler-backed semantic qualification remains separate. The bounded portable
extraction packet supplements a saved process via `process.extraction`; see
[process-extraction.md](process-extraction.md). Responsive geometry and
accessibility require a real browser run in addition to DOM tests.
