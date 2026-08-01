# Zero-grep graph-driven task edit

Date: 2026-08-01

## Outcome

The supported Kotlin/JVM workflow becomes:

```text
task intent
  -> one bounded task-context
  -> one generic role-complete structured Edit IR
  -> one atomic SThread task commit
  -> clean detached-worktree compile and tests
```

The benchmark agent must not use external source search or broad file reads.
SThread, rather than `apply_patch`, applies every production and test-source
change. The model follows emitted graph roles and provides compact exact
substitutions without repository-specific knowledge in the global worker or
skill. SThread owns target selection, source assembly, imports, syntax checks,
write scope, clean validation, Git commit, and evidence.

## Why the old pack failed

`agent-context` selected declarations by symbol-name matches, scanned lines for
textual references, and returned prefixes of test files. It did not consume the
existing K2 resolved calls or Thread IR. A class-name hit therefore returned the
head of `ProductService`, not the relevant `archive` member. `COMPLETE` described
only stdout truncation, not task-relative semantic closure.

Per-operation K2 validation is also the wrong unit for a multi-file contract
change: changing an interface, producer, repository projection, and callers is
temporarily inconsistent until the whole edit set exists.

## Task context

The existing `agent-context` command becomes a backwards-compatible alias for
a task-relative pack. It accepts free-text intent in addition to symbol hints.
The syntax index only discovers candidates. Ranked member functions are then
resolved with K2 and projected through their resolved call facts and local
graph.

The bounded stdout contains:

- exact member-level edit surfaces with source and stable targets;
- compact resolved-call edges with argument/parameter types;
- transitively relevant declarations and static contracts;
- anchored test-function snippets;
- behavioral and data-access invariants inferred from the closure;
- a structured edit skeleton and clean validation plan.

Full index, resolution, graph, anchors, and omitted evidence remain in the
evidence artifact. `COMPLETE_TASK` is permitted only when every mandatory
surface fits the byte budget; otherwise the pack is `PARTIAL_TASK` and names
the missing boundary.

## Atomic task edit

Edit IR gains task-level source operations:

- `REPLACE_DECLARATION` for signature/contract changes;
- `REWRITE_DECLARATION` for one or more exact substitutions inside an anchored
  declaration, with explicit occurrence preconditions;
- `CREATE_FILE` for a new Kotlin source or test;
- the existing expression, function-body, and import operations.

Declaration and file operations are syntax-checked while candidates are
assembled but semantic validation is deferred until the complete candidate set
exists. `tx commit` writes the set only into a detached worktree, runs the
configured clean compile and tests once, commits it, and updates the target ref
with compare-and-swap. A failure leaves the target unchanged.

The earlier hard-coded archive recipe was removed: it made the benchmark fast
by embedding task knowledge in the global worker and therefore was not a fair
general solution. Repository-owned recipes may be designed separately for
genuinely recurring local automation, but the eligible path requires none.

The speed optimization and correctness model are one semantic closure, one
role-complete plan, one multi-file candidate and one clean validation.

## Benchmark gate

A run is eligible only if:

- fresh hidden acceptance passes;
- no external `rg`, `grep`, `sed`, `find`, source-file shell reads, or direct
  production `apply_patch` occur in the benchmark rollout;
- SThread structured edit creates the committed source changes;
- context stdout is at most 16 KiB;
- commit time is below 171 seconds, tool calls below 21, noncached tokens below
  72,925, and raw tokens below 1,099,997.

The aspirational next margin is stricter: at most 150 seconds, 16 calls, 40,000
noncached tokens, and 400,000 raw tokens. One run proves the concrete result
only; it does not establish a general statistical claim.
