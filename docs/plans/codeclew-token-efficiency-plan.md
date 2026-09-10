# Codeclew token-efficiency implementation plan

Date: 2026-09-10
Status: proposed staged plan. No implementation is authorized by this document alone.

## Goal

Demonstrate and then implement a Codeclew path that reduces total model-token cost by replacing LLM-mediated search/orchestration with internal evidence planning, while preserving or improving task quality and evidence authority.

The plan intentionally starts with falsification. A new planner is not justified unless a one-step sufficient evidence package shows a meaningful achievable advantage over a strong lexical baseline.

## Non-goals

- Do not replace `rg` for exact literal/name lookups where it is already efficient.
- Do not build a new general-purpose graph before proving value from existing facts and thread flow.
- Do not weaken `decisionAuthority`, completeness or boundary semantics to manufacture a token win.
- Do not treat digest/provenance metadata as a substitute for source required to understand an edit.
- Do not claim runtime/service behavior from static or declared topology evidence.

## M0 — Freeze measurement semantics

### Work

Define a versioned token-economics record containing:

- original task and frozen repository revision(s);
- arm identity;
- total/cached/uncached model input and output tokens;
- tool output bytes and source bytes delivered to the model;
- LLM-mediated round count;
- failed calls and fallback transitions;
- success/oracle score;
- critical misses and false exact claims;
- cold/warm execution classification.

Freeze a small task corpus with at least these strata:

1. exact lexical lookup;
2. single-repository semantic analysis;
3. ambiguous/overloaded symbol analysis;
4. cross-file path reasoning;
5. cross-repository reasoning.

### Definition of Done

- Default and Codeclew arms receive the same task statement and repository authority.
- Default may use `rg` plus bounded source reads.
- Failed attempts are counted.
- The harness can distinguish model tokens from tool-output bytes.
- Historical `noncachedInputTokens` remains reportable for comparison with Q1.

### Stop condition

If the harness cannot produce replayable per-round accounting without materially changing one arm, stop and fix measurement before product work.

## M1 — Current-version baseline

### Work

Run the frozen corpus against current release behavior, not v0.2.5 assumptions. Attribute model-visible bytes to categories:

- skill/protocol;
- admission/schema;
- source;
- semantic facts;
- authority/provenance;
- boundaries/obligations;
- repeated content;
- error/recovery output.

Record the number of LLM decisions between the first user request and sufficient evidence.

### Definition of Done

Produce a report identifying current cost distribution without assuming that historical cumulative `nav expand` behavior still exists.

## M2 — Oracle sufficient-package gate

### Work

For each semantic task, an expert constructs the smallest reasonable evidence package from existing Codeclew authorities. Deliver it to the same model in one retrieval step. The package must include all source and boundaries required for a correct answer; it may not contain an answer unavailable from the evidence.

Compare:

- strong Default;
- current Codeclew;
- oracle sufficient package.

### Decision gate

Proceed to planner implementation only for task strata where the oracle arm shows a material and repeatable token advantage without quality loss.

Suggested threshold for promotion: median >=30% reduction plus no critical miss/false-exact regression, with aggregate confidence reported rather than relying on five anecdotes.

### Stop/redirect conditions

- Oracle ~= Default: do not build a planner for that stratum.
- Oracle wins only by removing provenance required for correctness: redesign projection, not authority.
- Oracle wins only because Default is artificially forced to read too much source: fix baseline.

## M3 — Projection-only optimization

### Work

Before new retrieval logic, construct an agent projection from existing evidence that contains only:

- selected claim/goal status;
- authority classification;
- minimal exact source windows needed for interpretation/editing;
- critical boundaries and verification obligations;
- stable references for optional drill-down.

Do not transmit full intermediate contexts merely because they exist in CAS.

### Definition of Done

Re-run M2 tasks. Quantify how much of the oracle gap is closed by serialization/projection alone.

### Decision

If projection-only reaches most of the oracle improvement, prioritize this path and defer planner complexity.

## M4 — Internal Evidence Goal prototype

### Scope

Implement one narrow, high-confidence goal family using existing JVM facts and verifier semantics. Preferred initial predicates:

- `CALL_EXISTS`;
- `REACHABLE_STATIC_PATH` where current flow authority is sufficient.

Do not start with arbitrary natural-language goals.

### Architecture

Introduce an internal typed goal representation. The exact public schema is deliberately deferred until the prototype stabilizes.

Planner states:

```text
DISCOVERY
PROVISIONAL_BINDING
VERIFIED_CLAIM
ABSTAIN
```

A provisional binding can drive internal traversal but cannot be rendered as an exact task conclusion.

Execution should compose existing primitives inside one public operation:

```text
resolve candidates
-> bind provisional/exact root
-> traverse retained relations
-> verify predicate
-> select source/boundary closure
-> persist proof closure
-> render compact agent projection
```

### Definition of Done

- No compiler/build rerun when existing sealed generation is sufficient.
- The result cannot promote a generic discovery candidate to exact authority without verifier support.
- Missing/ambiguous/truncated evidence returns typed abstention or unresolved goals.
- The model does not receive intermediate flow/index payloads unless explicitly drilled down.
- Existing `thread explain` semantics are reused or factored into a shared verifier rather than duplicated inconsistently.

## M5 — Minimum-cost evidence selection

### Work

Represent available verified evidence bundles as candidate packages with:

- obligation coverage bitset;
- serialized agent-projection cost;
- required boundary closure;
- source-window dependencies.

For small goal sets, implement exact selection using dynamic programming or branch-and-bound. Measure actual serialized union cost where shared source windows make costs non-additive.

### Definition of Done

For each planned result, retain:

- selected packages;
- total projected cost;
- best known lower bound;
- optimality gap within the generated candidate space.

When lower and upper bounds coincide, report exact optimality only within that finite candidate space. Never claim global token optimality.

## M6 — Early routing and cheap fallback

### Work

Add a deterministic/cheap router for obvious cases:

- exact literal/config -> lexical locate/read path;
- exact unique declaration -> exact navigation path;
- supported relational goal -> evidence planner;
- unsupported/low-confidence semantic goal -> early abstention to agent/default search.

The router should minimize the failed-attempt penalty. Do not invoke an expensive semantic path merely to discover that the profile cannot prove the requested relation.

### Definition of Done

Report per stratum:

- semantic-route success probability;
- savings when successful;
- failed-route overhead;
- always-paid protocol overhead.

Verify the measured break-even condition rather than assuming Codeclew-first is universally favorable.

## M7 — Qualification

### Primary gate

On a predeclared task distribution, require:

- task success non-inferior to Default under a fixed margin;
- zero critical evidence misses and no false-exact regression;
- aggregate token ratio materially below Default;
- proposed product target: upper 95% confidence bound of Codeclew/Default total logical token ratio < 0.70.

Also report cached/uncached tokens and monetary cost separately.

### Secondary gates

- warm latency remains interactive for promoted paths;
- cold indexing/build cost is reported separately;
- no hidden repository/source data is added to public diagnostics;
- fallback terminality is 100%;
- exact evidence remains reproducible from retained authority.

## Implementation order

```text
M0 measurement semantics
 -> M1 current baseline
 -> M2 oracle gate
 -> M3 projection-only
 -> M4 one Evidence Goal family
 -> M5 bounded optimal selection
 -> M6 router/fallback
 -> M7 qualification
```

M2 is the critical economic gate. Do not commit to M4-M6 for a task class that fails M2.

## Expected code seams

Likely reusable seams in the current repository include:

- `query_v2.rs` for indexed lookup;
- `context_v2.rs` for evidence/source projection;
- `navigation.rs` for compact agent-facing contracts;
- `thread_callables*` for retained callable identities;
- `thread_flow*` for bounded traversal;
- `explanation*` / thread explanation validation for typed predicate checking;
- CAS objects for retained proof closure.

Prefer factoring shared predicate verification and projection logic over introducing a parallel semantic subsystem.

## Success criterion

The feature succeeds when Codeclew can answer a promoted class of evidence questions with fewer total model tokens than a strong `rg`-based workflow because the digital thread removes model-mediated search steps—not because the baseline is weakened or evidence is omitted.