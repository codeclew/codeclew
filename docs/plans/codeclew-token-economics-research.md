# Codeclew token economics: research note

Date: 2026-09-10
Status: research proposal; historical evidence plus static analysis, not a new v0.7.1 agent benchmark.

## Problem

Codeclew owns compiler-backed identities, source bindings, immutable snapshots, cross-repository threads and typed evidence, but historical agent qualification did not consistently reduce token use versus a strong default workflow based on `rg` and bounded reads.

The preserved v0.2.5 Q1 result completed all five task classes in both arms, but recorded 533,589 noncached input tokens for Codeclew versus 171,685 for Default (3.108x total). An earlier fixed navigation case did show a smaller win: 41,076 versus 46,550 tokens (11.8% reduction). Therefore the observed problem is lack of robust, material savings rather than impossibility of any savings.

Some historical waste has already been fixed. In particular, current `nav expand` is delta-based, so cumulative expansion output should not be treated as a current missing feature.

## Main hypothesis

Codeclew currently improves the authority of retrieval more than it changes the algorithm by which an agent solves a retrieval problem.

`query_v2` still begins from normalized terms, exact-name postings and aliases, while `context_v2` ranks bounded facts. The agent skill explicitly requires the agent to create a private evidence checklist; `--intent` is provenance and does not change retrieval. Consequently the agent still performs much of the expensive loop:

1. decompose the task into claims;
2. invent search terms;
3. choose candidates;
4. request source or relations;
5. interpret boundaries;
6. decide whether evidence is sufficient;
7. formulate the next query.

The graph therefore often supplies better evidence without eliminating enough LLM-mediated decisions.

## Existing architectural leverage

The repository already contains most primitives needed for a different model:

- compiler-backed declaration identities and source authority;
- bounded navigation and exact source windows;
- multi-repository thread composition;
- `thread callables` and `thread flow` traversal;
- typed claim validation in `thread explain`, including predicates such as `CALL_EXISTS`, `CONSTRUCTS`, `BRANCH_EXISTS`, `ORDERED_BEFORE` and `REACHABLE_STATIC_PATH`;
- CAS-backed immutable evidence and semantic-zoom rendering.

The opportunity is to compose these primitives inside Codeclew instead of exposing every intermediate decision to the model.

## Theoretical model

For repository snapshot `R`, analysis scope `Ω`, and task `q`, let `O(q)={o1,...,ok}` be the required evidence obligations. Let `V(R,Ω,E,oi)` verify whether evidence set `E` proves obligation `oi` within the declared authority boundary.

The retrieval objective should be:

```text
E* = argmin_E tokens(render(E))
```

subject to:

```text
for every required oi:
    V(R, Ω, E, oi) = true
```

plus preservation of material boundaries/verification obligations and the source needed for edits.

This is different from top-k relevance: the objective is the cheapest sufficient proof package.

For an agent session, optimize total logical model traffic rather than only tool-output bytes:

```text
T(q) = Σ rounds (uncached_input + cached_input + output)
```

If the initial conversation has `S` tokens and round `i` appends `xi`, then without history removal:

```text
Σ input = N*S + Σ(i=1..N-1) (N-i)*xi
```

Thus eliminating an LLM-mediated round can be more valuable than compressing one tool response: early output is repeatedly carried by later rounds.

## Economic routing condition

Let `p` be the probability that a short semantic route is sufficient, `s` its savings when successful, `r` the extra cost of an insufficient attempt before fallback, and `h` unavoidable Codeclew overhead. Expected savings are:

```text
p*s - (1-p)*r - h
```

Codeclew-first is favorable when:

```text
p > (r+h)/(s+r)
```

This argues for a router rather than forcing semantic retrieval for every request. Exact unique identifiers, literals and known small files can legitimately remain lexical paths.

## Proposed architecture: sufficient-evidence compiler

Introduce a typed Evidence Goal above existing facts/flows. The target interaction becomes:

```text
agent -> typed evidence goals
      -> Codeclew planner
      -> indexes / facts / flow traversal
      -> predicate verifier
      -> minimal sufficient evidence package
      -> agent
```

Reuse the existing `thread explain` validation rules as planner terminal conditions where possible.

Separate discovery from proof:

```text
DISCOVERY -> PROVISIONAL_BINDING -> VERIFIED_CLAIM
```

A provisional binding may be traversed internally to test a hypothesis but cannot authorize a task-level exact claim. This permits investigation when the user did not already provide the correct symbol name without weakening the evidence contract.

The agent projection should normally contain only goal verdicts, authority, required source windows, critical boundaries/obligations and stable drill-down references. Full proof closure remains in CAS.

## Bounded optimality

For a first implementation, construct a finite set of verified evidence packages `Pj`, each covering obligations `Cj` at measured projection cost `cj`. With additive costs, choose:

```text
min Σ cj*xj
subject to every required obligation being covered
```

For small obligation sets this can be solved exactly with bitmask dynamic programming or branch-and-bound. The planner can report an optimality certificate within the finite candidate-package space when its lower and upper bounds meet. This does not claim global optimality across all possible representations.

Do not assume greedy set-cover guarantees without proof: evidence has complementarity (for example, two edges may only prove a path together).

## Falsifying experiment

Before implementing a broad planner, run five arms on the same frozen tasks:

1. strong Default: `rg` plus bounded reads/scripts;
2. current Codeclew;
3. projection-only: current evidence with a minimized agent projection;
4. oracle sufficient package: expert-selected evidence delivered in one step;
5. automatic planner (after implementation).

The oracle arm is diagnostic, not a product result or proven lower bound.

Interpretation:

- oracle does not materially beat Default: retrieval planning alone is unlikely to solve this task class;
- oracle wins but planner does not: localization/planning is the bottleneck;
- projection-only nearly reaches oracle: optimize protocol/rendering first;
- results vary strongly by task class: implement routing.

## Measurement rules

Do not give Codeclew an exact symbol absent from the original task. Do not force Default to read whole files when bounded reads suffice. Include failed attempts and fallback costs. Separate navigation from stronger mutation/publication guarantees. Record total logical input/output, cached and uncached input, monetary cost, task success, critical misses, false exact claims and LLM-mediated round count.

A proposed product gate is at least 30% token reduction on a predeclared task distribution with quality non-inferiority, expressed statistically as an upper confidence bound below 0.70 for the aggregate Codeclew/Default token ratio. The 30% value is a product target, not a derived constant.

## Conclusion

The central product metric should become **cost to sufficient verified evidence**, not number of indexed facts, files avoided, or bounded response size. Codeclew's digital thread creates a defensible advantage only when it removes reasoning/search steps from the LLM path. The next work should test that hypothesis with an oracle package and then implement one narrow goal-driven path before expanding the semantic surface.