# Plan: build-independent documentation and semantic enrichment

Date: 2026-09-10.
Status: proposed; implementation and qualification have not been performed.
Basis: [research RFC](codeclew-syntax-first-documentation-rfc.md), grounded in
`codeclew/codeclew@4430b5af1e82b0bf89b7eca988517c1802108b0c`.

This is the English repository edition of the accompanying research plan.
Merging these documents records a proposal, not completion of its milestones
or authorization to change runtime/publication behavior.

## Goal

Let an agent create and maintain source-bound documentation without requiring
successful compilation. Compiler and framework providers improve fact precision
and invalidation while preserving one shared documentation foundation.

The product unit is a checkable scenario `ExplanationBundle` with common events
and claims and several consistent views, not independently generated pages and
pictures. Preserve the existing documentation models and evolve their evidence
inputs instead of creating a parallel documentation subsystem.

## Invariants

- Available source/syntax evidence does not depend on semantic-provider success.
- A syntax call occurrence does not become a resolved call edge by name matching.
- A current source binding does not prove arbitrary narrative correctness.
- Compact views retain material compilation, runtime, and configuration boundaries.
- Unavailable dependencies are not interpreted as unchanged.
- All views in one publication use one explanation version and revision vector.
- Documentation fallback neither permits mutation nor bypasses existing guards.
- Baseline parsing requires no production credentials, project build scripts,
  module imports, or dependency installation.

## D0. Freeze contracts and test tasks

Define source occurrence, logical documentation entity, semantic instance,
observation, claim, query dependency, and rendering dependency separately.
Specify independent binding, freshness, authority, and review states. Keep
proposed schema names provisional until the vertical slice is qualified.

Prepare small Python and Java service fixtures. Java has buildable and
intentionally missing-dependency variants. Tasks cover a local operation, a
conditional branch, a helper call, and an engineer-declared service handoff.
Freeze expected source bindings, admissible claims, and unresolved boundaries
before implementation. Record source revisions and producer/profile versions.

**Definition of Done:** the generator does not define its own success oracle.
The fixtures distinguish exact source attachment from behavior interpretation
and static relations from runtime claims.

## D1. Source-only admission

Extract read-only source capture for explicit repository/revision/source roots
without mandatory project-model extraction. Reuse CAS, safe-path validation,
immutable snapshots, resource limits, and provenance.

Add inventory/scope observations for path membership, language/root selection,
and selection-policy versions. Unsupported files retain `FILE_ONLY` evidence.
Do not automatically download grammars or execute repository code.

**Definition of Done:** without JDK, Maven, Gradle, Cargo, or a target Python
environment, the baseline returns a source manifest and available data. Process
and network audits show no attempts to execute these tools or install project
dependencies. Missing, unsafe, and oversized inputs produce named boundaries or
failures. Existing mutation admission is unchanged.

## D2. Common structural observations

Reuse the Python Tree-sitter extractor and add a build-independent Java parser
adapter for the first slice. Subsequently separate Rust syntax extraction from
Cargo-target authority, retaining distinct profiles. Qualify Kotlin grammar/PSI
choices separately rather than mandating a universal parser migration.

The shared contract covers declarations, containment, call expressions, control
syntax, annotations as written, source ranges, parse errors, and coverage. It
does not introduce resolved relations without a qualifying resolver.

**Definition of Done:** identical source bytes and parser/rule versions produce
identical normalized observations. Literal changes are detected even when tree
shape is unchanged. `ERROR`/`MISSING` recovery never upgrades authority. Positive
syntax observations remain available without semantics. Each newly supported
language passes its own range, encoding, grammar, and normalization checks.

## D3. Engine-independent documentation

Adapt existing `documentation/model.rs`, `analysis.rs`, and `store.rs`. Replace
mandatory JVM-compilation admission at the documentation boundary with explicit
source/analysis scopes and capability-based validation. Plan compatibility with
existing service records rather than silently rewriting their authority.

Retain portable bindings, authored/generated ownership, manual-edit protection,
and content-addressed outputs. Extend current Event/Explanation/Fragment records
with engine-independent evidence references. Start with prose and sequence views;
permit a state view only with an explicit state abstraction and provenance.

**Definition of Done:** Python and Java with an unavailable build produce useful
bundles and resolvable source links. Text fragments and diagram elements connect
to parent events/claims. Unresolved service handoffs are declared/candidate, not
proven runtime calls. All views derive from one accepted explanation version.
No runtime-order guarantee is inferred merely from source line order.

## D4. Conservative freshness without a compiler

Implement version correspondence separately from immutable source locators.
Return ambiguous/missing when a unique correspondence is not established. A
short-name match does not prove a rename, split, or merge.

Register both positive support dependencies and influence dependencies. Query
records include scope, filter, result membership, coverage, and negative lookups.
Use broader scope watches when precise semantic influence is unknown, or expose
an incomplete influence closure; do not silently ignore possible dependencies.

Record the evidence supplied to each agent-generation step and its query scopes.
Untracked source reads require a wider conservative watch or an explicit
incomplete read manifest. Citations alone are not a complete dependency trace.

Recompute observations and prune propagation only when the consumer-relevant
projection is unchanged and its dependency closure is sufficient. Version the
equality policy; do not describe it as general program equivalence.

**Definition of Done:** no known-affected fixture claim is marked current.
Relocation-only changes update anchors without an LLM call. Added/deleted routes
and scope members are observed, including additions after a previously empty
query. Partial evidence produces unresolved status rather than false-current.
Invalidation names affected claims, dependent views, and supporting reasons.

## D5. Semantic enrichment

Connect existing K2/javac providers to the same snapshot authority. Each fact
records its producer, compilation/environment, source mapping, relevant input
dependencies, and boundaries.

Reuse or factor shared typed-predicate verification. Do not require the agent to
assemble every intermediate JSON object manually. A resolved result may refute
a candidate; preserve history and revise the accepted interpretation.

**Definition of Done:** restored compilation enriches the same documentation
object instead of creating a separate copy. Subsequent compiler failure leaves
syntax documentation readable but marks dependent semantic claims for review.
Synthetic or multi-compilation symbols never receive fabricated source links.
Parser results do not silently replace failed K2 evidence with stronger claims.
Measure whether enrichment reduces ambiguity and unnecessary review fan-out.

## D6. Incremental change dossier and publication

Create a bounded agent package containing the old claim, invalidation reasons,
old/new snippets and facts, unresolved assumptions, and affected view IDs. Include
necessary imports, configuration, and helper context rather than only a changed
line. Retain references for focused expansion when the first package is
insufficient; do not hide material omissions to meet a size target.

After review, retain a new canonical explanation, render its views
deterministically, and atomically switch the documentation bundle pointer.
Previous bundles remain reproducible. CI/webhook execution is a separately
configured integration, not an implied background service.

**Definition of Done:** no mixed-version publication; manual changes are
protected; repeated operations are idempotent; failed generation neither removes
the baseline nor marks it fresh. Publication refers to documentation outputs,
not an atomic transaction across independently deployed services.

## Required mutation matrix

| Change | Required behavior |
|---|---|
| Lines inserted before a function | Rebind coordinates without revising the claim when meaningful inputs are unchanged |
| Literal `100` changed to `200` | Detect through bytes/tokens even when tree shape is unchanged |
| Meaningful docstring change | Review explanations that used the docstring |
| Called helper changed | Invalidate dependent explanations or explicitly report incomplete influence closure |
| Controller/endpoint added | Update query membership and catalogue without requiring an old link to the new file |
| New overload or changed import | Recompute binding/semantic goals; use scope watches in syntax-only mode |
| Rename or relocation | Establish correspondence and an exact new link or return explicit ambiguity |
| Split, merge, or duplicate functions | Never attach the old documentation ID silently to an arbitrary candidate |
| Configuration/contract-only change | Review corresponding scenario and service-interaction claims |
| Classpath, cfg, or analyzer changed | Recheck dependent semantic projections; unchanged source bytes are insufficient |
| Parser error or missing grammar | Return partial/FILE_ONLY with available source; do not promote unsupported structure claims |
| Compiler unavailable, then restored | Preserve baseline availability and report actual semantic freshness/authority |
| Asynchronous callback or new retry branch | Do not derive runtime order from source-line order |
| Only one service advances its revision | Bind the new revision vector and revisit transitively affected scenarios |
| Previously empty query gains a result | Invalidate its scope/result dependency and any dependent inventory or absence claim |

## Agent evaluation

Run separate experiments for initial explanation and maintenance across a series
of changes. Use the same model configuration, task statements, source snapshots,
quality requirements, and ordinary source-reading tools within each comparison.

**A. Strong Default:** `rg` and bounded reads, without forced full-file reading.

**B. Syntax Codeclew:** structural evidence plus the shared documentation IR.

**C. Enriched Codeclew:** the same IR plus compiler evidence.

**D. Hybrid:** selective enrichment for tasks where it provides measured value.

Buildable and intentionally unbuildable fixtures are separate strata. Do not
give one arm hidden oracle names or relationships. Runtime truth must not be
replaced by an expert guess; evaluate unknown scenarios on whether their
boundaries are represented correctly.

Measure availability, anchor precision, ambiguity, invalidation recall,
false-current cases, over-invalidation, view consistency, reproducibility, and
evidence coverage for the deterministic path. Measure factual correctness,
unsupported claims, task coverage, regeneration scope, every model input/output
token category with cache separation, model rounds, recovery cost, latency,
and monetary cost for the agent path.

Count failed attempts and fallback. Report cold indexing/enrichment and their
amortization across updates separately. Use paired repetitions and uncertainty
intervals clustered by repository/scenario; several mutations of one function
are not independent projects. No numeric advantage is claimed before this
experiment.

A zero-error controlled mutation suite is a release gate for that suite, not a
proof of universal dependency completeness. Report denominators and unresolved
cases explicitly rather than treating abstention as a correct semantic answer.

## Decisions after measurement

- B improves availability but not cost: keep the availability feature without
  advertising a token win.
- C reduces ambiguity and over-invalidation versus B: promote enrichment for
  the task classes where that improvement is measured.
- C costs more without a quality/maintenance benefit: keep it optional, not a
  prerequisite for documentation.
- Any arm produces material false-current results: fix dependency coverage first;
  fewer regenerated pages are not a win when stale pages were missed.
- Sequence/state/prose views diverge: repair the shared IR and renderer rather
  than rely on independently generating and manually reconciling outputs.

## Order and first acceptance boundary

```text
D0 contracts and fixtures
 -> D1 source admission
 -> D2 structural observations
 -> D3 shared documentation
 -> D4 conservative freshness
 -> D5 semantic enrichment
 -> D6 selective regeneration and qualification
```

First acceptance covers D1-D4 on Python and the broken-build Java fixture.
Further languages, exact service-dependency analysis, and fine-grained cost-based
routing follow that correctness result. D2 lists extension points, not a demand
to implement every language before the first vertical slice.

The implementation plan complements the token-economics research in PR #6.
It does not depend on that branch: this proposal concerns baseline availability,
evidence identity, and documentation maintenance; token savings remain a shared
experimental question rather than an established outcome.
