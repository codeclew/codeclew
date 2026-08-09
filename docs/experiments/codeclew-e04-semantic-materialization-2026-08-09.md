# E04 product checkpoint: authority-backed semantic materialization

Date: 2026-08-09
Status: ACCEPT for the narrow contour documented here
Scope: one narrow Kotlin 2.1 / Gradle `MAP_EDGE_WITH_CONTEXT` family

This is the product-first E04 materialization checkpoint that follows the E03
probe. It adds executable evidence to the editing branch, but it does **not**
mark the frozen cumulative-graph node `E04 — Blind three-mode goal-binding
experiment` complete. That withheld benchmark remains future work.

## Product result

Codeclew can now take the live, process-local E03 proof receipt and execute:

```bash
clew apply map-edge-with-context \
  --repo /path/to/clean-kotlin-repository-root \
  --compilation :/main \
  --workflow-symbol com.example.valuesAwaitingContext \
  --test-symbol 'applies the mapping context to one value' \
  --test-compilation :/test \
  --target-ref refs/heads/main
```

For the accepted fixture the command:

1. rebuilds and verifies the Kotlin 2.1 semantic thread;
2. binds the context producer, transformer and collection edge;
3. proves the twelve E03 invariants and fifteen change obligations;
4. compiles the opaque receipt into one typed `MAP_EDGE_WITH_CONTEXT`
   operation with an empty Kotlin replacement string;
5. constructs the new Kotlin declaration with PSI inside the version-pinned
   worker;
6. resolves the generated calls with K2, checks the unchanged signature and
   rejects new diagnostics or effects;
7. applies the candidate in a detached worktree;
8. runs the configured Gradle compile/tests;
9. creates one commit and advances the target ref with compare-and-swap.

The model does not supply source text, regex, file names, import edits, local
names, placement code or a repository recipe. The worker derives those details
from the authority-owned bindings.

## Concrete change

The accepted fixture begins with:

```kotlin
fun valuesAwaitingContext(values: List<Int>): List<Int> = values
```

The worker materializes:

```kotlin
fun valuesAwaitingContext(values: List<Int>): List<Int> {
    val __codeclewContext = com.acme.mappingContext()
    return values.map { __codeclewValue ->
        com.acme.applyMappingContext(__codeclewValue, __codeclewContext)
    }
}
```

The actual PSI output is kept on one line inside the lambda, but has the same
structure. A hidden acceptance test added only after the semantic commit checks
that `valuesAwaitingContext(listOf(4, 5)) == listOf(6, 7)` and reruns the Gradle
test lifecycle.

## Authority and refusal boundary

The typed operation is not a new general-purpose edit escape hatch:

- public generic preview/commit rejects `MAP_EDGE_WITH_CONTEXT`;
- only a live receipt stored by the same `EvidenceAuthority` session can compile
  and commit the operation;
- a copied JSON proof cannot be replayed;
- textual replacement must be empty;
- `AMBIGUOUS` and `REFUSED` return decisions without a transaction or mutation;
- a dirty checkout invalidates the receipt;
- the target ref must still equal the exact proof revision;
- any moved or divergent target requires a new E03 proof.

The exact-revision rule is intentional. The generic transaction replay only
rebuilds the workflow thread, while E03 also depends on the context producer,
transformer effect proof and compiler-linked behavioral test. Until those
dependencies become a goal-wide replay bundle, silently rebasing an authority
proof would be unsound.

## Materialization implementation boundary

The headless Kotlin worker cannot use IntelliJ editor/POM indentation services.
It therefore constructs a complete `KtNamedFunction` with `KtPsiFactory`, takes
that parsed declaration's text, and replaces the uniquely anchored owner range.
It does not use regex or model-authored source. The candidate is parsed again
and analyzed with K2 before leaving the worker.

The current materializer deliberately refuses annotations, modifiers, type
parameters, receiver functions, suspend functions, nullable types, lazy
collections, indirect/aliased value flow and non-root Git project paths. This
keeps the implementation claim aligned with executable evidence.

## Executable evidence

Focused gates:

```bash
cargo test -p clew \
  generic_preview_cannot_apply_an_authority_semantic_operation --lib

cargo test -p clew --test goal_materialization \
  clew_apply_materializes_the_proved_change_as_one_verified_commit

cargo test -p clew --test goal_materialization \
  clew_apply_leaves_no_commit_or_source_change_when_not_bound

cargo test -p clew --test goal_materialization \
  an_authority_receipt_cannot_overwrite_a_newer_worktree
```

Observed focused results before final verification:

| Gate | Result |
|---|---|
| generic authority bypass | passed; forged semantic operation rejected before worker apply |
| positive public apply | passed; one Kotlin file, one commit, hidden behavior test green |
| ambiguous/refused apply | passed; no HEAD or worktree change |
| dirty and divergent target | passed; `STALE_REQUIRES_RESLICE`, both refs unchanged |

The first full `scripts/verify.sh` run reached the integration phase after 116
passing unit/CLI checks, then exposed a pre-existing fixture setup assumption:
the authority test required a new commit even when copied fixture bytes already
matched HEAD. The helper now uses an explicit empty fixture commit; its focused
test passes. The subsequent clean `scripts/verify.sh` run completed with
`{"schema":"semantic-verification/0.1","status":"PASSED"}`, including all
three new materialization scenarios, the authority and goal-binding tests,
concurrency matrix, Kotlin 2.1 worker, Maven contour and semantic corpus.

## Independent verification history

The first independent E04 review returned `REJECT` with a real public-CLI
counterexample. A proof issued on `main` could target a different branch where
the same transformer signature had acquired `println`; workflow-only replay
then committed the change while incorrectly retaining the old purity claim.

The repair forbids authority-semantic replay across revisions. The commit path
now checks the target ref against the proof revision before preview or
materialization. The regression constructs the divergent side-effecting target
and confirms that Codeclew returns `STALE_REQUIRES_RESLICE` without advancing
it. The independent delta rerun returned `ACCEPT`: the exact effectful-target
attack now returns `STALE_REQUIRES_RESLICE`, leaves both target and checked-out
revision unchanged, and CAS still protects movement after the revision check.

## What this proves — and what it does not

This checkpoint proves a complete mechanism for one structural family:

```text
typed intent
  -> live evidence and bounded binding
  -> invariant/change-graph proof
  -> worker-owned semantic operation
  -> Kotlin candidate
  -> compile/tests
  -> atomic commit
```

It does not prove broad applicability, multi-family materialization, Maven
support, serialized/remote proof capabilities, a general PSI refactoring
engine, or a time/token win over default and AST-index modes. Those remain
withheld-corpus and paired-benchmark questions. The meaningful advance is that
Codeclew can now both understand and safely perform one non-local Kotlin change
without a grep workflow or a model-authored textual patch.
