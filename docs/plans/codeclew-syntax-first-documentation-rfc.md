# Syntax-first documentation, semantic enrichment, and freshness

Date: 2026-09-10.
Status: research RFC and architecture proposal; not implemented or qualified.
Inspected revision: `codeclew/codeclew@4430b5af1e82b0bf89b7eca988517c1802108b0c`.

This document is the English repository edition of the research prepared for
this discussion. Its evidence is static inspection of the implementation and
primary technical sources. No new engine comparison or agent benchmark was run.
The guarantees below depend on explicit assumptions; benefits to agents remain
experimental hypotheses. The [implementation and qualification plan](codeclew-syntax-first-documentation-plan.md)
defines the proposed next steps.

## 1. Decision

Make build-independent source capture and structural observations the foundation
of Codeclew. K2, javac, and other analyzers should enrich this foundation rather
than decide whether the user can access source-bound documentation at all.

Tree-sitter is suitable for this foundation, but need not be the only parser.
The common contract should describe observations, not prescribe one universal
AST. Retain the Rust `syn` adapter where useful. Qualify Kotlin PSI parsing and
candidate Tree-sitter grammars separately for coverage, resilience, and delivery
cost before selecting a baseline provider.

The product contract is bounded: for accessible, explicitly selected sources
within resource limits, return available observations and named gaps. It does
not promise an exact cross-service runtime graph for arbitrary repositories.

## 2. Current implementation

| Language | Mechanism | Available evidence | Boundary |
|---|---|---|---|
| Kotlin | K2/FIR, PSI, and a custom extractor | Declarations, resolved relations, and exported compiler facts in qualified profiles | Project model, dependencies, compiler versions, options, and plugins affect availability and authority |
| Java | JDK Compiler API: `JavacTask.parse()` followed by `analyze()` | Resolved elements, JVM descriptors, annotations, and source-bound relations | Compiler errors switch the inspected analyzer to diagnostic boundaries |
| Rust | `syn` and `proc-macro2`, plus a separate Cargo model | Syntax declarations, ranges, and bounded direct-call-path and match hints | No compiler-resolved call graph; cfg and macro expansion remain boundaries; the current model requires Cargo and Cargo.lock |
| Python | Tree-sitter with a pinned Python grammar | Syntax declarations, observations, source hashes, and parse-error boundaries | No runtime import/type/decorator resolution; resolved declaration relations are empty |

These findings are grounded in the [Kotlin worker][R1], [Java analyzer][R2],
[Rust adapter][R3], [Cargo model][R4], [Python adapter][R5], and [Python model][R6].

Syntax-index availability is not documentation availability. The inspected
`documentation/store.rs::validate_service` admits Java/Kotlin JVM profiles only.
`documentation/analysis.rs::capture` performs task admission and obtains a
compiler-fact generation. Existing Rust/Python navigation therefore does not
provide an automatic fallback for `clew docs`. See [admission][R7] and
[documentation analysis][R8].

Much of the required documentation foundation already exists in the
[shared model][R9], [bindings and freshness][R10], and [checking/composition][R11]:

- normalized `Observation` values and digests;
- revision-bound `Source` records with text hashes;
- `Event`, `Fragment`, and `Explanation` records with dependency and source IDs;
- diagram elements connected to event IDs;
- portable baselines with dependencies, producer versions, and output hashes;
- bounded transitive expansion of some within-service dependencies;
- separate dependency, source-link, and entrypoint-catalogue changes.

The existing freshness result explicitly limits its scope to recorded
dependencies and supported static analysis. Coordinate relocation is separated
from normalized-observation changes. Preserve these distinctions instead of
claiming unrestricted semantic freshness.

## 3. K2 and Tree-sitter answer different questions

Tree-sitter builds a concrete syntax tree and supports incremental updates and
useful output in the presence of syntax errors [S1]. Compiler analysis also
resolves names, types, and calls. For example, Kotlin call resolution can expose
the selected callable, receivers, and argument mapping [S2] and [S3].

For `repository.save(order)`, a structural observation can establish the exact
expression and its lexical enclosure. The expression alone does not establish
the selected overload, receiver origin, runtime implementation, or transaction
boundary. The same source text analyzed with different dependencies can have a
different resolution; a file-only parser has not been given those inputs.

An agent benefits from normalized answers to specific questions, not a raw FIR
dump: which symbol is selected, where its source is, which relations are
established, and which scopes remain unanalyzed. These answers can replace
repeated name searches and disambiguation. Codeclew obtains only the compiler
facts its extractor actually exports and qualifies [R1].

K2 alone does not prove message delivery, production routing, successful
execution, or activation of a transaction proxy. These require additional
models or observations. The current interaction checker already preserves
runtime activation, destination, serialization, and delivery boundaries [R11].

Conversely, syntax-first does not prohibit name resolution. A resolver can add
language-specific scope and binding rules above the CST. GitHub Stack Graphs
illustrates that architecture [S4]; it is an additional analysis, not a free
property of the parser. Its treatment of overloads, types, and dynamic behavior
would require separate qualification in Codeclew.

Semantic analysis is not synonymous with a complete build. Kotlin Analysis API
separates parsing without project context from semantic analysis that needs
dependencies and compilation options [S5]. Obtaining those inputs and handling
partial results are architectural choices, not reasons to withhold source data.

## 4. Layers and contracts

### 4.1 Independent source and syntax foundation

A `RepositorySnapshot` binds repository namespace, revision/tree, selected path
inventory, blob digests, scope policy, and limitations. Multi-service analysis
binds an explicit revision vector; it does not imply one deployment instant.

A proposed `StructuralIndex` requires no Maven/Gradle/Cargo execution, Python
module imports, project build scripts, or network access. It exposes:

- source occurrences and exact ranges;
- syntax declarations and containment;
- call expressions and annotations/decorators as written;
- branch, loop, return, and throw structure;
- literal and configuration occurrences;
- parse errors, omissions, and unavailable grammars.

Without a grammar, return `FILE_ONLY`: retained source, explicit ranges, and
conservative change detection. Parser recovery must not authorize stronger
structural claims than its evidence supports. Inaccessible, unsafe, or oversized
inputs remain named failures or gaps rather than successful empty results.

### 4.2 Independent enrichment providers

A compiler provider consumes the same snapshot plus its compilation inputs. It
need not consume the Tree-sitter tree. Connect its facts to source occurrences
through checked mappings; one occurrence may have several semantic instances
in different compilations. Do not combine incompatible configurations into one
unqualified fact set.

Framework and contract providers retain their own rules, configurations,
schemas, sources, and derivation provenance. Runtime observations bind an
environment and time scope. Engineer-declared interactions remain declarations.

The separation of syntax from richer semantics in rust-analyzer is a useful
precedent [S6]. It demonstrates architectural feasibility, not an existing
Codeclew integration or a guarantee for every language.

### 4.3 Independent authority, coverage, and freshness axes

Represent at least the following separately:

- basis: exact source, syntax-derived, name-resolved, compiler-resolved,
  framework-derived, declared, observed, or agent-inferred;
- scope: snapshot, compilation, configuration, and environment;
- coverage: complete within a stated scope or partial;
- freshness of required inputs;
- concrete boundaries and verification obligations.

An exact syntax observation does not become uncertain simply because K2 is
unavailable. It also does not become a resolved call edge through name matching.

New evidence may refute a provisional hypothesis. Evidence storage can be
append-only, while accepted interpretations require revision, conflict handling,
and explicit withdrawal. Enrichment is not unconditional confidence promotion.

### 4.4 Enrichment failure

A K2/javac failure must not erase the source/syntax foundation. Keep the last
successful documentation readable with its original binding. Claims requiring
an unavailable provider are unresolved or review-required for the new snapshot;
similar source text alone does not establish their semantic freshness.

Read-only documentation fallback does not extend mutation/publication authority
or bypass existing admission and publication guards.

## 5. One canonical documentation source

The proposed central object is a versioned `ExplanationBundle`, extending the
existing `Narrative`, `Operation`, `Event`, `Observation`, and `FragmentBinding`.

```text
Source snapshots + configuration + declared contracts
                      |
          Structural and semantic observations
                      |
             Claims / Scenario IR
                      |
              ExplanationBundle
           /             |            \
         prose       sequence       state/flow
```

A scenario has one canonical set of events, claims, and relationships. It can
depend on many files and services. One documentation source does not mean one
code file.

The agent authors explanations and proposes domain interpretations. Codeclew
checks bindings, supported typed predicates, reference closure, boundaries, and
dependencies. Render every view from the same accepted bundle version. Correct
the canonical model rather than editing an independent Mermaid copy that can
diverge from prose.

Sequence views must distinguish textual order, statically established control
order, and observed runtime order. State diagrams require explicit states and
an abstraction rule for transitions. An AST does not automatically define a
business state machine; agent-proposed abstractions retain their provenance.

Source attribution does not prove arbitrary natural-language prose. Check
formalizable claims deterministically and evaluate the remaining agent-authored
interpretation through review. The agent generation step itself is not claimed
to be deterministic: persist its accepted result, version its inputs, and render
that result deterministically.

## 6. Three identities

### Source occurrence

An immutable locator contains repository namespace, snapshot, file/blob, byte
range, and a digest of the selected bytes. Compute line numbers for presentation.
Normalize compiler UTF-16 offsets and UTF-8 storage ranges where applicable.

### Logical documentation entity

A long-lived fragment/scenario ID does not depend on its current source line.
Establish correspondence between revisions separately: unchanged, relocated,
renamed, split, merged, ambiguous, or missing. A short-name match alone does not
prove a rename or justify silently attaching an old explanation to a candidate.

### Semantic symbol instance

Compiler identity includes compilation context and input version. Link it to
source occurrences without replacing them. Synthetic symbols without exact
ranges require separate provenance, not invented source links.

Tree-sitter `Node.id` is an identity within a tree that may be reused during
incremental parsing; it is not a persistent entity ID across arbitrary Git
revisions [S7]. Neither a content hash nor a line number solves correspondence
for copied, edited, split, or merged declarations.

## 7. Source binding is not dependency completeness

Distinguish two dependency sets.

**Support dependencies** are the evidence currently supporting a claim.

**Influence dependencies** are inputs whose changes can alter that evidence or
the result of finding it.

For overload selection, influence dependencies can include imports, declarations
in scope, classpath, and compiler options. A new declaration can change the
resolution without changing the quoted call site.

For a catalogue of all HTTP operations, watching the currently found controllers
is insufficient. Record the query, its scope and filter, membership, extraction
rules, result, and coverage. Even an empty result needs dependencies; otherwise
a newly added route can remain invisible to invalidation.

Watching only old anchors or old positive edges is not sound change detection.
Use conservative module/service/configuration/repository scope watches where
precise influence analysis is unavailable. This increases review fan-out but
avoids silently declaring freshness. Unknown external influences remain explicit
boundaries; broader local watches do not prove external completeness.

An agent's citation list is not necessarily its complete read set. Record the
evidence package and query scopes supplied to each generation step. Untracked
reads require a conservative wider dependency or an explicit incomplete manifest.
Narrow dependencies only with a justified policy, not by assuming that uncited
material was unused.

## 8. Fingerprints and changed ranges

Track separate fingerprints for:

- exact source bytes;
- declaration/signature;
- body tokens, including literal values;
- comments/docstrings used as evidence;
- imports, namespaces, and membership catalogues;
- compilation and configuration inputs;
- grammar, extractor, interpreter, renderer, and policy versions.

`Tree.changed_ranges` compares syntactic structure, not every meaningful content
change [S8]. A numeric-value change with the same tree shape must still be
found through bytes or tokens. A shape hash alone is insufficient.

Removing all whitespace and comments is not a universal semantic normalization.
Language and purpose matter: Python indentation, string contents, and docstrings
used in an explanation retain significance. Suppression of cosmetic changes
requires a versioned, independently tested fingerprint policy.

The current JVM documentation projection already strips coordinates and adds
source tokens; freshness reports source-link changes separately [R8] and [R10].
Generalize and qualify these mechanisms rather than replacing them with an
unqualified AST-shape equality test.

## 9. Formal freshness model

Let `I_t` contain all registered inputs at a snapshot: source bytes, scope
inventories, configuration, and producer versions. An edge `x -> y` means that
result `y` depends on input `x`.

For changed, added, deleted, or newly unavailable inputs `delta_I`:

```text
Affected = ReachableForward(delta_I) intersect DocumentationObjects
```

This is reverse-dependency closure when edges are stored as consumer-to-input.
Scope nodes are necessary to bring newly added files into propagation.

### Conditional guarantee

Assume:

1. Every relevant computation input is registered; there are no hidden reads.
2. Queries depend on scope membership, including negative lookups.
3. Snapshots are stable and input comparison is reliable under the digest model.
4. Derivation rules are deterministic and versioned.
5. Provider gaps produce conservative dependencies or explicit incompleteness.
6. Dependency cycles use correct fixed-point or strongly connected component
   handling.

Then closure does not miss a represented dependency path from a changed input
to a potentially affected document. For a DAG, the argument follows topological
induction: unchanged inputs and rules preserve a derived result; changed
predecessors trigger checking of dependents. Scope observations reduce file
additions and deletions to ordinary input changes.

This is propagation completeness for the represented dependency model, not a
proof of arbitrary narrative correctness or program equivalence. Treat the
accepted agent-generated text as a retained artifact with a recorded read set;
new generation is not assumed to reproduce the same text deterministically.

Bazel Skyframe is a primary reference for registered inputs, transitive
invalidation, and change pruning after recomputation [S9]. The analogy does not
substitute for implementing and testing Codeclew's own documentation contract.

### Meaning of CURRENT

`CURRENT` means the relevant observed grounds are unchanged within the declared
scope and the checking policy is satisfied. It does not mean every sentence is
universally true.

A relocated source may preserve its meaningful fingerprint. A changed dependency
may leave a claim valid after checking. Keep binding status, input freshness,
claim authority, and review status separate. Unknown or unavailable evidence is
not evidence of unchanged inputs.

## 10. Update cycle

1. Capture a new revision vector without assuming it represents a deployment.
2. Compare source, configuration, and inventory inputs; reparse affected files.
3. Recompute affected observations and version correspondence.
4. Invalidate query dependencies, including membership and negative lookups.
5. Identify affected claims. Prune further propagation only when the relevant
   recomputed projection is unchanged and its dependency closure is sufficient.
6. Update relocation-only source links without another LLM explanation.
7. Supply the agent with old claims, before/after evidence, invalidation reasons,
   boundaries, and dependent view IDs.
8. Check the new canonical bundle and publish its views together. Until then,
   retain the old bundle with a visible freshness status.

Source failures and engine failures are not unchanged results. Preserve existing
write-conflict protection for manually edited outputs. An explicit revision
vector describes selected source versions; deployment coherence requires its
own release/configuration evidence.

## 11. Expected agent benefit and evidence limits

The expected benefit comes from reducing repeated investigation, not merely
from having an AST. The agent receives selected sources, structural/semantic
observations, and a bounded change dossier.

Aider already uses Tree-sitter repository maps [S10]. This supports feasibility
of syntax scaffolding but does not measure Codeclew's benefit. Codebase-Memory
[S11] reinforces the need to evaluate quality and cost together; its results
must not be transferred to Codeclew or treated as proof of equivalent quality.

The principal hypothesis is that syntax-first improves availability while
compiler evidence reduces ambiguity and unnecessary invalidation. Evaluate the
full creation-and-maintenance lifecycle:

```text
C = C_capture + C_syntax + C_enrichment
    + C_agent_review + C_generation + C_render
```

Record tokens, CPU, wall time, and monetary cost separately. Include failed
attempts and fallback. Initial generation savings must not be purchased by
missing stale claims during subsequent updates.

## 12. First vertical slice

Use Python through the existing Tree-sitter extractor and a Java fixture with
intentionally unavailable compilation. Both feed one engine-independent
documentation contract. Restoring Java compilation adds semantic facts to the
same source authority and revisits claims depending on that enrichment.

Do not begin with a new universal cross-service call graph. First establish a
useful bundle without a build, exact bindings, no silent false-current results
on controlled mutations, and consistent views. Then measure the incremental
benefit of K2/javac. See the [staged plan](codeclew-syntax-first-documentation-plan.md).

## Primary sources

Implementation links are pinned to the inspected revision. External references
support the architecture discussion, not claims that the proposal is implemented.

[R1]: https://github.com/codeclew/codeclew/blob/4430b5af1e82b0bf89b7eca988517c1802108b0c/workers/kotlin/src/main/kotlin/dev/semanticthread/worker/Worker.kt
[R2]: https://github.com/codeclew/codeclew/blob/4430b5af1e82b0bf89b7eca988517c1802108b0c/crates/clew/src/java_analyzer.java
[R3]: https://github.com/codeclew/codeclew/blob/4430b5af1e82b0bf89b7eca988517c1802108b0c/crates/clew/src/rust_adapter_v2.rs
[R4]: https://github.com/codeclew/codeclew/blob/4430b5af1e82b0bf89b7eca988517c1802108b0c/crates/clew/src/rust_project_model.rs
[R5]: https://github.com/codeclew/codeclew/blob/4430b5af1e82b0bf89b7eca988517c1802108b0c/crates/clew/src/python_adapter_v2.rs
[R6]: https://github.com/codeclew/codeclew/blob/4430b5af1e82b0bf89b7eca988517c1802108b0c/crates/clew/src/python_project_model.rs
[R7]: https://github.com/codeclew/codeclew/blob/4430b5af1e82b0bf89b7eca988517c1802108b0c/crates/clew/src/documentation/store.rs
[R8]: https://github.com/codeclew/codeclew/blob/4430b5af1e82b0bf89b7eca988517c1802108b0c/crates/clew/src/documentation/analysis.rs
[R9]: https://github.com/codeclew/codeclew/blob/4430b5af1e82b0bf89b7eca988517c1802108b0c/crates/clew/src/documentation/model.rs
[R10]: https://github.com/codeclew/codeclew/blob/4430b5af1e82b0bf89b7eca988517c1802108b0c/crates/clew/src/documentation/bindings.rs
[R11]: https://github.com/codeclew/codeclew/blob/4430b5af1e82b0bf89b7eca988517c1802108b0c/crates/clew/src/documentation/check.rs
[S1]: https://tree-sitter.github.io/tree-sitter/index.html
[S2]: https://kotlinlang.org/docs/custom-compiler-plugins.html
[S3]: https://kotlin.github.io/analysis-api/resolving-calls.html
[S4]: https://github.blog/open-source/introducing-stack-graphs/
[S5]: https://kotlin.github.io/analysis-api/in-memory-file-analysis.html
[S6]: https://rust-analyzer.github.io/book/contributing/architecture.html
[S7]: https://tree-sitter.github.io/py-tree-sitter/classes/tree_sitter.Node.html
[S8]: https://tree-sitter.github.io/py-tree-sitter/classes/tree_sitter.Tree.html
[S9]: https://bazel.build/reference/skyframe
[S10]: https://aider.chat/2023/10/22/repomap.html
[S11]: https://arxiv.org/abs/2603.27277

### Source index

- [R1: Kotlin worker][R1]; [R2: Java analyzer][R2].
- [R3: Rust syntax][R3]; [R4: Cargo model][R4].
- [R5: Python syntax][R5]; [R6: Python model][R6].
- [R7: Documentation admission][R7]; [R8: Documentation analysis][R8].
- [R9: Shared documentation model][R9]; [R10: Bindings/freshness][R10]; [R11: Checking/composition][R11].
- [S1: Tree-sitter][S1]; [S2: Compiler stages][S2]; [S3: Call resolution][S3].
- [S4: Stack Graphs][S4]; [S5: Parsing versus semantic context][S5]; [S6: rust-analyzer architecture][S6].
- [S7: Node identity][S7]; [S8: Changed ranges][S8]; [S9: Skyframe][S9].
- [S10: Aider repository maps][S10]; [S11: Codebase-Memory][S11].
