# E01: typed semantic goal, change graph and proof

Date: 2026-08-09

Outcome: `SUCCESS`

Independent verdict: `ACCEPT` after two fail-closed proof corrections

## Product result

`semantic-goal/0.2` makes the model-owned request a closed typed constraint
set. For the first `MAP_EDGE_WITH_CONTEXT` family it contains only the family,
base revision, the exact semantic primitive set and the oracle policy. Source
text, substitutions, regexes, graph identifiers, symbols and inferred Kotlin
types are not accepted model inputs.

The product binder consumes worker/kernel evidence and returns one of three
states:

- `BOUND` with three distinct role-labelled bindings and a first-class
  `change-graph/0.2`;
- `AMBIGUOUS` with bounded candidate choices and no bindings or obligations;
- `REFUSED` with no bindings or obligations when a required invariant cannot
  be proven.

A `BOUND` proof is accepted only against the same composite snapshot and a
complete semantic kernel. All candidate and predicate records must be current,
sound and source-provenanced. The graph has an exact obligation count for all
13 primitives, explicit dependencies and evidence for type compatibility,
single evaluation, mapping, preservation, ABI, behavioral oracle and absence
of unsupported boundaries. Any current `Unknown` record makes binding fail
closed.

The three `BindUnique` obligations are labelled as context producer,
transformer and value edge. Their symbols must be distinct, preventing one
valid fact plus filler obligations from being replayed as three semantic
roles. Absence of a reported boundary is not evidence: the boundary obligation
requires its own current, sound `NoUnsupportedBoundary` kernel fact tied to all
three bindings.

## Migration and safety boundary

The former `semantic-goal/0.1` shape has a narrow compatibility decoder.
Legacy type strings are discarded because type binding is now worker-owned.
Only an empty `businessChoices` map is accepted; an old request containing a
source replacement or any other free-form choice is rejected. Current and
legacy wire schemas deny unknown fields.

E01 deliberately performs no source mutation. It separates intent, semantic
binding and proof from materialization, so a later editor cannot treat an
ambiguous goal as a patch plan.

## Verification evidence

Focused executable checks cover:

- a complete unique binding and change graph;
- ambiguity and all fail-closed preservation/oracle/snapshot paths;
- missing, stale, conservative and semantically mismatched kernel records;
- every typed primitive being mandatory and consumed;
- legacy migration and rejection of free-form choices;
- rejection of `sourceText`, `replacement`, `regex`, `EditIR` and `graphId`;
- forged role aliases, duplicate roles and removal of boundary proof evidence;
- a current hidden `Unknown` inside nominally complete coverage.

Command: `cargo test -p clew semantic_goal --lib`.

## Independent gate history

The first independent pass rejected the candidate despite 12 passing tests. It
found two real proof attacks: `MustRefuseOnBoundary` relied on an empty result
field rather than positive kernel evidence, and three generic `BindUnique`
obligations could be relabelled or aliased. Both defects were fixed in product
code and encoded as counterexample tests. The delta pass then found that
bind-time validation rejected a hidden current `Unknown`, while replay of an
already issued proof did not. The shared invariant was added to replay
validation, and the independent final delta pass accepted the result.

## Honest limit and next edge

This is one typed family and a proof/binder boundary, not universal semantic
editing. It does not yet establish family-relative `COMPLETE_FOR`, five-family
applicability, source materialization, correctness on hidden tasks, or a time
or token win over default and AST-index modes. Those claims remain closed by
E02 and later benchmark nodes. The meaningful result here is narrower: a model
can express one change intent without emitting code/navigation identities, and
the product can turn it into an evidence-bound semantic obligation graph or
fail closed.

## Gate efficiency

The independent gate used one source review and one focused 12-test run to find
two product defects. Repair added two semantic invariants and focused mutation
tests; no controller, service or benchmark harness was introduced.
