# Process envelope fixture benchmark

## Scope and ground truth

The executable fixture is
`documentation::process_extract_tests::deterministic_benchmark_records_full_read_and_bounded_envelope_bytes`.
It declares three methods: a root calling a helper and an external target, the
helper calling the root, and one unrelated method. The exact retained graph has
two reachable methods and eight events, including IF, ELSE and THROW. A selected
helper RETURN is found only when both methods fit the budget. Provider labels
vary across JAVAC, K2_FIR and SYNTAX_ONLY; these are synthetic contract fixtures,
not compiler-worker qualification.

## Observations

| Baseline | Retained source-object bytes | Outcome coverage |
| --- | ---: | --- |
| Read all three fixture sources | 49 | Reference ground truth; human reading not timed |
| Full retained callable envelope | 30 | Selected outcome found |
| One-method budget | 14 | Outcome UNKNOWN with explicit METHOD_BUDGET |

These are actual byte counts in the small fixture, not measured model-token or
monetary savings. There are zero model calls. Worker duplication is zero because
there is one packet consumer; no parallel-agent comparison was run. Wall time of
manual reading, an LLM baseline, calibration against human reviewers and runtime
performance are not measured. The bounded packet cannot be called better merely
because it is smaller: it loses outcome coverage and reports that loss.

## Pass/fail assertions

- Full expansion retains both cycle members, the exception and unresolved external
  target; it does not claim delivered messages or program semantic closure.
- Each method/event/source-byte budget yields a BUDGET frontier. Missing outcomes
  remain UNKNOWN, not absent or unreachable.
- Ambiguous same-scope targets cannot become unique through aggregation.
- Forged root dependency or changed source digest fails closed.
- Repeated inputs yield identical packets; snapshot or configuration changes alter
  freshness identity. Provider labels do not bypass the shared contract.
- Explicit branches require guard citations and a valid default shape. Effect
  citations cannot escape the retained method.
- Reader tests keep alternatives separate, source links navigable and untrusted
  labels inert. DOM tests do not establish browser geometry.

The supplied private review case is used outside the repository for native replay
and exact source preservation. No private case text, identities or paths belong in
these fixtures. Broader human/model benchmarks require a separately measured
corpus and budget; no paid runs are part of this slice.
