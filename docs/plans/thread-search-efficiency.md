# Thread search efficiency and criterion-specific structural reachability

Status: experimental implementation; not merge-qualified.
Base: `bb2c7b732682fc56c5ea598e508d957fec780575` (main, inspected 2026-10-13).
Branch: `research/thread-search-efficiency`.
Scope: documentation process extraction. No provider calls or authored pages.

## Current algorithm and limits

`process_graph::collect_bounded` already builds service/scope/symbol indices and
uses a deduplicated BFS. It follows a unique same-scope callable target;
ambiguous ownership, absent targets, foreign scopes, unavailable source and
budgets remain explicit frontiers. This is a retained association graph. An
exact symbol match in it is not compiler-qualified argument binding or runtime
routing. The forward envelope retains branches, exceptions and unrelated
methods conservatively.

`process_extract::packet` previously resolved each outcome by scanning every
method's event membership and then repeatedly scanned every call edge until
the union of outcome ancestors stabilized. In forward edge order, a chain may
need one pass per level: O(VE log V) for the reverse annotation. Several outcomes
were combined into one boolean, so a consumer could not tell which outcome a
method belonged to. This did not establish a semantic data/effect slice.

Other paths are separate: `thread_flow` traverses admitted compiler callable
facts, `thread_question` builds retained evidence selectors and summaries, and
`thread_value_calls` adds narrowly qualified value-call summaries. This change
optimizes the documentation extraction path only; it does not replace them.

## Research and choice

| Primary source | Relevant finding | Application here |
| --- | --- | --- |
| [Horwitz, Reps, Binkley: Interprocedural Slicing Using Dependence Graphs](https://research.cs.wisc.edu/wpis/abstracts/toplas90.abs.html) | SDGs and call summaries support slicing across procedure boundaries | Keep the distinction between call reachability and dependence; do not present the current graph as an SDG |
| [Reps, Sagiv, Horwitz: Interprocedural Dataflow Analysis via Graph Reachability](https://research.cs.wisc.edu/wpis/papers/diku-tr94-14.pdf) | IFDS uses finite distributive facts and realizable call/return paths | Use finite outcome bits and monotone propagation; implementing IFDS requires additional qualified transfer and call/return structure |
| [Horwitz, Reps, Sagiv: Demand Interprocedural Dataflow Analysis](https://research.cs.wisc.edu/wpis/papers/fse95a.pdf) | Demand analysis focuses on selected facts and can reuse useful intermediate results | Seed reverse propagation from requested outcomes; leave unqualified facts and unrelated forward evidence visible |

The implementation is an ordinary structural reachability worklist, inspired by
these scheduling principles. It is not an IFDS solver or a context-sensitive
interprocedural slice. SCC compression and persistent caches were deferred:
current bounded extraction admits at most 64 methods and 32 outcomes, and index
setup can already dominate shallow queries. A new cache would also need exact
snapshot, scope, provider and implementation binding.

## Implemented slice

- Index retained event ownership and presence once; require one event and one
  owner for an outcome seed.
- Build reverse adjacency over retained method endpoints. Propagate a 32-bit
  outcome mask with a queue, requeuing only on changed masks. Cycles terminate;
  simultaneous pending updates are coalesced. Unknown endpoints are excluded.
- For a single outcome, try at most two full scans. Return on convergence or
  full envelope coverage; otherwise continue through the indexed worklist.
- Expose `outcomes[].structuralMethodIds` and
  `methods[].structuralOutcomeEventIds`. Retain `onStructuralOutcomePath` as the
  union, keep semantic closure UNKNOWN, and keep every forward-envelope method.
- Advance the policy to `retained-process-envelope/0.2` and bind the new kernel
  into the implementation digest. Packet schema remains 0.1 with additive
  fields. The existing reader exposes the outcome lists in provenance.

For indexed propagation, each node's mask gains at most K bits. Edge processing
is O(KE) after O((V+E) log V) indexing, with K <= 32; a single-outcome worklist
visits each predecessor edge once. The shallow warm-up adds at most two scans
and membership tests. Source hydration, graph collection and JSON serialization
are outside this kernel and may dominate total latency.

## Observed checks and timing

Standalone tests compile the actual production kernel via
`scripts/benchmark_process_reachability.rs`. Rust 1.91.1, Linux x86_64, edition
2024, `-D warnings`; benchmark additionally uses `-O`. Pinned Rust 1.92 is
unavailable here: the toolchain download endpoint timed out. These checks do not
qualify the entire clew crate or its integration.

Three kernel tests passed. Differential checks cover all 512 directed
three-node graphs with self-loops and all 16 pairs of optional outcome seeds,
plus all four single seeds: 10,240 comparisons. Additional assertions cover
cycles, diamonds, duplicate edges, missing retained endpoints, disconnected
methods, all 32 bits and empty queries. The packet-level two-outcome/budget-cut
regression was added but has NOT run in this environment.

Benchmark: 11 samples of 100 executions, median nanoseconds per execution;
setup/index costs inside the kernel are included, compiler startup and retained
capture are excluded. No timing thresholds are CI assertions. Equality with an
independent full-scan reference is checked before each measured shape.

| 64 methods, one outcome | Full-scan reference | New kernel | Ratio |
| --- | ---: | ---: | ---: |
| Chain, 63 edges | 109.723 us | 10.381 us | 10.570x faster |
| Star, 63 edges | 0.757 us | 0.914 us | 0.828x; slower |
| Dense DAG, 2016 edges | 192.625 us | 74.549 us | 2.584x faster |

Raw measurements: [kernel-benchmark.csv](evidence/thread-search-efficiency-20261013/kernel-benchmark.csv).
For 32 outcomes the reference runs a separate scan for each criterion to produce
comparable per-outcome masks. Those ratios are NOT a speedup over the old union
annotation, which answered a less specific question. Single-outcome reference
uses the old scan strategy but includes conversion to the comparable mask map.
No end-to-end performance, private-service replay, semantic precision/recall or
compiler call-resolution improvement is claimed. The precision gain is separate
structural membership per requested outcome rather than an undifferentiated
union. Warm-up reduced a larger initial star regression; the remaining shallow
regression is reported and needs production timing before adoption.

## Reproduction and integration

```sh
rustc --edition 2024 -D warnings --test scripts/benchmark_process_reachability.rs -o /tmp/clew-reachability-tests
/tmp/clew-reachability-tests
rustc --edition 2024 -D warnings -O scripts/benchmark_process_reachability.rs -o /tmp/clew-reachability-bench
/tmp/clew-reachability-bench
cargo fmt --all --check
cargo test --locked -p clew --lib documentation::process_reachability -- --test-threads=1
cargo test --locked -p clew --lib documentation::process_extract_tests -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
./scripts/ci-verify.sh
```

Use pinned Rust 1.92 for integration. English and privacy worktree checks and
whitespace validation passed on the experiment. No full native crate build or
CI run was performed. The branch must remain experimental until packet tests and the
full gate pass. No changes to the release branch are part of this experiment.

## Next measurable slices

1. Measure total `docs process extract` latency separately for retained hydrate,
   graph collection, reverse annotation and packet serialization on public source
   fixtures. Include absent outcomes, real shallow graphs, chains and budget cuts.
   Compare warm/cold runs; report medians and tail latency before choosing a cache.
2. Qualify event-to-declaration ownership and callsite argument/receiver bindings
   from retained compiler facts. Build a labeled public corpus with overloads,
   inherited/interface dispatch, recursion, aliases and exception paths. Measure
   false links, missed links and explicit unknowns separately; never improve a
   score by removing unknown frontier cases from the denominator silently.
3. Evaluate a demand dependence solver only after finite transfer rules and
   matched call/return sites are qualified. Keep summary edges keyed by exact
   compilation/revision and distinguish alias, exception and framework gaps.
4. Evaluate criterion-prioritized budget expansion without deleting uncertain
   branches. Report outcome recall under equal source/event/method budgets and
   preserve explicit cuts. Do not replace absence with an irrelevance claim.
