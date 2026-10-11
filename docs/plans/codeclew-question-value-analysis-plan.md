# Question slices and portable value analysis

Date: 2026-10-11. Status: prototypes integrated; subsequent slices authorized
and in progress. This is an implementation roadmap, not a new approval or admission
layer. It records the current branch results without claiming release-wide
qualification or measured agent savings.

## Current implementation checkpoint

The authorized implementation order is: integrate the prototype, correct the two
confirmed lifecycle description errors through native rendering, expose an
authenticated retained Java value query (S2d), qualify one Kotlin bridge (S2c),
then implement bounded call summaries, selected effects/process composition,
and source-bound publication. Measured agent scheduling follows useful retained
queries and summaries. No new paid authoring or benchmark arms are authorized.

The primary `develop` checkout was fast-forwarded from `fa0b374` to the actual
remote tip `0327221` on 2026-10-11. The fetched branch uses rewritten commit IDs
`e12e0f8`, `8a9e514`, `e603e17`, and `0327221`; the historical cloud IDs in the
baseline table below remain as recorded provenance. The separate documentation
worktree and its uncommitted changes were preserved.

Confirmed locally after integration: formatting; 14 retained-flow tests, five
control-query tests, and 12 portable-value tests; and
`cargo clippy --locked -p clew-facts -p clew --lib --tests -- -D warnings`.
The two reported collapsible-if diagnostics are already fixed in the fetched
source. Native Java qualification is being checked separately. Full CI has not
been run.

S2d now exposes `session values` through an existing saved Java session generation.
Four component binding tests and one managed Maven/javac public-path test passed;
the latter covers deterministic repeats, exact retained source references,
operation without analysis tools on PATH, later live checkout edits, unknown
owners/targets, node/transfer cuts, and long owner/path packet reduction.
Selected equations and occurrences are materialized with the complete model
digest; omitted dependencies remain explicit frontiers. An initial public-path
test found a mismatch between canonical payload hashes and schema-bound CAS
digests in fact identity reconstruction; it was corrected before the passing
native test. Formatting, Clippy for both affected packages, English-content and
privacy checks passed. This is focused qualification, not full CI.

S2c now emits a bounded immutable-local projection from the Kotlin 2.4.10 FIR
worker and translates it to the same portable equations. Native compiler tests
cover exact shadowing/repeated-use definitions, overloads, literals/defaulted
inputs, null-only Elvis, Unicode/CRLF coordinate mapping and explicit unsupported
or budget results. Rust consumes the native golden projection and shared
Java/Kotlin nullness expectations. Smart casts retain their original resolved
occurrences without importing type refinements. The shared host code also
compiles with 2.1.21; that worker gains no projection capability. The Kotlin
retained public loader is not implemented by this bridge.

Next slice: bounded exact direct-call summaries and interprocedural dependency
transfer. General heap fields, effects/exceptions and concrete enum-value
semantics remain outside the qualified nullness profile. See
[Kotlin extraction](../operations/kotlin-value-extraction.md) for bounds.

## Implemented baseline

| Slice | Commit | Observed result | Remaining limit |
| --- | --- | --- | --- |
| S1: retained-flow question packets | `c8cb7c2` | Read-only `thread question` command, bounded criteria, deterministic packets and local control queries | Retained flow input is Kotlin-specific; structural envelopes do not prove semantic closure |
| S2a: portable value equations | `8cbc0b5` | Language-neutral model and Rust nullness query API, backward relevance, monotone fixed point and explicit budget frontiers | Declared equations, no authenticated publication loader or public value-query CLI |
| S2b: automatic Java extraction | `501b97f` | Real javac extraction for a closed immutable-local profile, validated portable projection and native fixture | No general branches, heap aliases, exception/effect model or interprocedural value transfer |

Operational contracts:
[question packets](../operations/question-packets.md),
[portable value queries](../operations/portable-value-queries.md), and
[automatic javac extraction](../operations/javac-value-extraction.md).

The last implementation verification passed 41 focused tests, including explicit
native javac qualification. Formatting, focused Clippy, English-content and
privacy checks passed. This does not represent full CI, native Gradle/Maven
discovery qualification, or qualification of other compiler workers.

## Shared invariants

- Compiler observations, framework derivations, declared relationships and
  model interpretation retain separate authority. A precise source span or
  descriptor does not establish deployed routing, business intent or execution.
- Deterministic calculation over equations is not proof that those equations
  completely model the program. The current solver reports
  `DECLARED_MODEL_EQUATIONS` and `programSemanticClosure=false`.
- Semantic boundaries and budget cuts remain distinct. Missing evidence,
  unsupported operations and exhausted budgets produce unknown outcomes;
  they never justify a claim that an error is absent.
- Definition identity, occurrence identity, source ownership, evidence binding
  and snapshot freshness require deterministic checks. Source names are not
  sufficient to resolve calls or reaching definitions.
- Source presentation may include complete methods while relevance selects
  individual definitions, uses, guards and effects. Presentation size must not
  silently redefine semantic coverage.
- Backend capability is qualified per operation, worker and version. Registry
  membership alone does not grant value-analysis authority.

## Next four reviewable slices

### S2c: one independently qualified Kotlin bridge

Inspect the available Kotlin compiler worker contracts and select one installed
version with suitable resolved value observations. Do not assume every FIR
bridge exposes the same facts or is available. Keep Java support independent.

1. Map resolved parameters, literals and immutable local definitions to the
   portable model with exact owner, occurrence and source evidence.
2. Add COPY and null-only COALESCE for an explicitly supported expression shape.
   Unsupported writes, calls, fields, control shapes and missing metadata remain
   unknown. Boolean defaults and truthiness require separate semantics.
3. Run shared fixtures and a real native worker fixture. Compare result sets,
   operand/guard dependencies and frontiers rather than backend-specific IDs.

Acceptance: reproducible normalized output; exact definition matching under
shadowing and repeated uses; explicit unsupported/budget results; native evidence
for the selected worker version. Additional Kotlin versions require their own
qualification. General JOIN/path refinement is a later extension, not an
implicit consequence of matching the Java examples.

### S2d: authenticated retained queries

Connect value projection and queries to immutable retained compiler runs before
adding a bounded public command. The loader must verify generation, compilation,
source/configuration bindings and retained CAS closure. Arbitrary input JSON must
not become compiler authority through structural validation alone.

Expose exact owner/target selection, backend capability, query budgets, equations,
evidence references and semantic/budget frontiers. Missing or stale projections
return explicit unavailable/unknown results. Preserve old retained-flow inputs as
a separate supported contour until the new loader is qualified.

Acceptance: reject foreign ownership, changed source/configuration and missing
retained objects; deterministic repeat; old supported facts remain readable;
unsupported value capability is never a successful empty result. Validate CLI
behavior and evidence materialization through the actual retained-run path.

### S3a: bounded call summaries

Start with exact same-snapshot direct calls and a closed pure-value profile.
Transfer arguments to parameters and returns to callers; carry guards and
unknown transfers. Use demand-driven backward expansion, SCC condensation and
bounded fixed points for recursive components. Distinguish call contexts and
overloads; do not substitute a callable-family match for an exact target.

Summaries expose inputs, outputs, relevant dependencies, assumptions, coverage
and boundaries. Unknown aliasing or side effects invalidate a pure summary rather
than disappearing from it. Cache summaries only with complete freshness keys.

Acceptance: shared helper reuse without duplicate extraction; exact overload and
recursive fixtures; bounded cycle handling; omitted code has a checked irrelevant
transfer or an explicit frontier. A budget stop must remain visible to callers.

### S3b: selected effects and process integration

Add READ/WRITE, exception and selected I/O summaries only where facts support
them. Keep repository implementations, shared databases, framework dispatch,
runtime configuration, serialization, callbacks and transport as explicit
frontiers when unresolved. Transaction annotations are declarations interpreted
under a versioned framework policy, not runtime commit proof. A send invocation
does not prove delivery or distributed rollback.

Reuse the existing source-bound process rendering and mandatory FLOW coverage
contracts. Generate a proposal, validate exact citations, ownership, digests,
connections and structural coverage, then accept/reject/unknown. Business labels
and intent remain reviewer judgments; model confidence cannot upgrade evidence.

Acceptance: preserve exclusive branches and exceptional exits; distinguish
potential static process, declared scenario and observed execution; retain
frontiers in diagrams and packets. Generic status updates must not produce an
invented strict FSM.

## Additional backends and agent scheduling

After the first bridges, qualify Roslyn and other semantic adapters against the
same portable contract where appropriate. JavaScript/TypeScript need explicit
undefined and truthiness semantics. Python/Rust syntax inputs retain syntax
authority and boundaries until independently justified semantic facts exist.
Maintain an operation-level capability matrix, not a blanket language label.

Use one interpreter by default. Extract once into a shared immutable evidence
cache; deduplicate concurrent requests with single-flight. Deliver only selected
facts and registered bounded expansions. Partition larger questions by dependency
components or effect regions with explicit interface summaries, rather than
assigning one agent per service. Schedule the resulting dependency DAG with
bounded parallelism and a dedicated deterministic integration check.

Freshness keys include source, compiler/toolchain, build and configuration,
extractor, framework policy, summary schema, solver implementation and query
policy. Include prompt/model identity when model output materially contributes
to a reusable summary. Cache hit status does not upgrade semantic authority.

Compare total input/output tokens, duplicated evidence bytes, model calls,
extraction time, queueing and critical-path latency. Prefer one agent when
coordination and repeated context exceed independent useful work. Stop expansion
when the accepted question is answered, a hard budget is reached, or expected
uncertainty reduction no longer justifies cost. Unresolved required evidence
still appears as a frontier. Parallelism trades token cost for latency; no
measured savings are claimed by this roadmap.

## Compact verification plan

Build cheap deterministic fixtures first: caller-overridden defaults, exact
overloads and shadowing, repeated definitions, null/branch joins, hidden writes,
fields/aliases, seeded and unseeded cycles, exception exits, missing configuration,
and send/save boundaries. Pin ground truth separately for graph structure,
modeled results, runtime evidence and domain judgments.

For later real-case evaluation, use the same pinned questions and evidence across
full reading, manual search, one interpreter and bounded distributed scheduling.
Record correctness, structural/semantic coverage, unknown calibration, tokens,
latency, duplicated bytes and cache hits. Label calculations as illustrative
until measured. Paid model runs and broad research benchmarks are not launched
by this plan.

Pass/fail conditions for each qualified contour:

- All binding, unsupported-operation and budget adversarial fixtures fail closed.
- Repeated facts/configuration/policy produce identical deterministic results.
- Expected dependencies and alternatives are retained; omissions have explicit
  boundaries or checked summaries.
- No test accepts unsupported certainty, a fabricated target, commit/delivery
  proof, or absence-of-error claim from incomplete evidence.
- Qualified exact-result fixtures match ground truth; unsupported cases remain
  unknown. Broader soundness/completeness claims require a stated model and
  separate qualification.

The immediate next action is S2d: make the existing Java value analysis accessible
through authenticated retained evidence, then S2c: qualify one Kotlin worker
bridge against the same contract. Full cross-service semantics and agent parallelism follow
only after those contracts can preserve authority and incompleteness end to end.
