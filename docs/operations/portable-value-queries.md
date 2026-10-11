# Portable value query prototype

The S2a prototype separates a language-neutral analysis kernel from language
extraction. `clew_facts::values` defines `portable-value-model/0.1`;
`clew::thread_values::analyze` calculates a bounded backward slice and nullness
of values on **modeled normal completion**. It is a Rust API prototype, not a
public CLI operation or program-wide source slicer. A first automatic javac
immutable-local projection is described in `javac-value-extraction.md`.
The existing `thread question` command retains its earlier Kotlin flow profile.
It does not become a multi-language flow loader through this addition.

## Contract and authority

A model binds language, producer, exact snapshot, fact, configuration and bridge
policy digests. Each value has a unique occurrence/definition ID, evidence IDs,
an operation and explicit predicate dependencies. These IDs are opaque to the
kernel: no JVM descriptors, FIR names, source-name matching or language-specific
parser is involved. Separate assignments and call occurrences need separate IDs.

The operations are `INPUT`, `NULL`, `NON_NULL`, `COPY`, SSA `JOIN`, null
`COALESCE`, and `UNKNOWN` with an explicit boundary. `COALESCE` is not boolean
OR, JavaScript truthiness or optional chaining. Guards are relevance edges;
they do not prune alternatives or establish path feasibility. Calls, field
aliases, mutable storage and unsupported mappers require `UNKNOWN` until a
qualified bridge supplies a correct transfer. Selected definitions can be
statement-sized even when source presentation retains complete methods.

`observationAuthority` is supplied metadata. Neither a compiler label nor a
well-formed digest authenticates evidence. The result always says
`calculationAuthority = DECLARED_MODEL_EQUATIONS` and
`programSemanticClosure = false`. It retains the binding and the unconditional
`MODEL_BINDING_AND_EXTRACTION_NOT_VERIFIED_BY_SOLVER` frontier. A caller must
verify snapshot/evidence ownership and bridge semantics before giving an output
any stronger authority. There is deliberately no command that accepts JSON and
publishes it as compiler-proven analysis.

The result's model digest covers the canonical, sorted model including every
binding and evidence reference. A cache key also needs the query and solver
policy/implementation digest; the model digest alone is not a query result key.
Changing source, facts, configuration, bridge policy or query invalidates the
associated cached result. This prototype does not implement a persistent cache.

## Deterministic calculation

1. Validate schema, digests, unique sorted definitions, nonempty sorted evidence,
   closed operand/guard references and hard model budgets.
2. Select the target first. Expand operands and guards backward using a sorted
   pending set and lexical ID tie-break. A presentation/relevance cut is a
   `BUDGET` frontier; its missing operands evaluate as unknown.
3. Solve equations on the finite lattice `bottom`, `{null}`, `{non-null}`,
   `{null, non-null}` with a sorted worklist. Transfers are monotone. `JOIN`
   unions alternatives; `COALESCE` preserves non-null left values and evaluates
   the right abstraction for possible null left values.
4. Widen remaining bottom definitions once to unknown and propagate again.
   Unseeded cycles or missing modeled normal values cannot establish nullness.
   This is conservative fallback, not a proof of nontermination.
5. If the transfer budget stops a fixpoint, publish unknown for every selected
   value. Never publish an intermediate underapproximation as a result.
6. Return sorted observations, evidence, semantic/discovery/budget frontiers and
   the actual transfer count. Equation convergence is separate from program
   semantic closure. A missing target is discovery unknown, never an empty proof.

The default API examples use 128 selected nodes and 2,048 transfers. Hard limits
are 4,096 model nodes, 16,384 operand/guard links, 16,384 evidence references,
64 global boundaries and 1,000,000 query transfers. Text fields have bounded
lengths. Global extraction gaps affect every query; node-local unsupported
transfers are retained when selected. Memory is linear in the supplied graph;
ordered-set bookkeeping adds logarithmic factors. Lattice height bounds each
node to two bit additions; transfer visits are bounded by the initial nodes and
notifications over dependent links, with the explicit transfer cap as backstop.
This profile does not enumerate paths.

Excluded nodes are outside the backward closure **in the supplied model**.
That is not proof that excluded program code is irrelevant: an omitted alias,
exception, callback or effect dependency invalidates that assumption. The model
must retain its extraction gaps. There is no absence-of-bugs verdict.

## Existing adapters and the next bridge

The repository has several different analysis authorities. This audit describes
source contracts; it does not qualify or execute their native compilers.

| Adapter | Existing useful facts | Missing for a general value bridge |
| --- | --- | --- |
| Java / javac | Exact parameter/local/field declarations and READ/WRITE/READ_WRITE occurrences, source identities and anchors | RHS operand binding, reaching definitions/SSA, semantic CFG predicates, exceptional value transfers; a read is not its reaching definition |
| Kotlin / versioned FIR | Resolved callable/property relations, selected argument mappings, narrow return/null-coalesce observations, local CFG | General immutable local definitions and operands, complete SSA/control/value coverage; narrow relation support must be checked per operation |
| C# / Roslyn | Resolved declarations/calls and portable CLR attributes; source-structure documentation | Qualified value transfers and semantic CFG/SSA projection; documentation events are not that projection |
| TypeScript / JavaScript compiler adapters | Resolved declarations and relations | Qualified value/control transfers; null, undefined and truthiness must not be merged silently |
| Python / tree-sitter | Source-bound syntax observations | Semantic target/value/alias authority; syntax-only hypotheses remain syntax or declared |
| Rust / Cargo-authorized syntax | Syntax index, partial/unsure facts and cfg/macro/name-resolution obligations | rustc semantic transfers, borrowing/alias and cfg/macro qualification |

Evidence locations include `java_adapter_v2.rs` (`JavaVariableDeclaration`,
`JavaVariableAccess`), `java_analyzer.java` (`variableIdentity`,
`variableDeclaration`, `variableAccess`), versioned Kotlin worker plugins,
`csharp_adapter_v2.rs`, `workers/dotnet/src/FactWalker.cs`,
`typescript_adapter_v2.rs`, `python_adapter_v2.rs`, and `rust_adapter_v2.rs`.
Java now emits `javac-value-projection/0.1` on source methods with bodies;
`java_values::project_index` maps it into this portable model. That restricted
profile covers parameters, literals, immutable local copies and one exact null
default pattern. General SSA/control/alias transfers remain unavailable. Other
adapters do not yet emit a qualified projection. This table remains an extraction
backlog, not a declaration of multi-language support.

The next bridge should qualify Kotlin against the javac immutable-local profile
on the same fixtures: parameter input, literal null/non-null, direct copy, branch merge and
null default. Each bridge must prove exact definition/operand/evidence bindings
and emit boundaries for every unsupported construct. Do not infer assignments
from source offsets or choose a reaching definition by name. Null defaults in
TypeScript require a separate domain for undefined; Python and Rust must keep
their present authority until semantic extraction is qualified.

## Verification and useful limits

The focused tests check caller input overriding a default, nullable branch
merges, guard retention, seeded and unseeded cycles, unknown mapper/exception
boundaries, relevance and fixpoint cuts, missing targets, repeated call IDs,
binding invalidation and invalid contracts. Running identical equations with
seven language labels checks absence of a FIR dependency; it does **not** test
seven compiler backends. No worker or paid benchmark runs are claimed.

For `input.status ?: PENDING`, the normal result is non-null, but caller input
remains relevant and its business value is unknown. The kernel does not infer
PENDING, allowed state transitions, repository commit, message delivery,
transaction atomicity, concurrency correctness or the absence of an exception.
Those need richer domains and qualified effect/exception summaries.
