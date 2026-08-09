# E02 second approach: authority-bound completeness

Date: 2026-08-09

Outcome: `SUCCESS + NARROW_BINDER_FAMILIES`

Independent verdict: `ACCEPT` for one deliberately narrow contour:

- Kotlin 2.1;
- Gradle;
- clean committed Git checkout;
- one local `ProducerTransformConsumer` family;
- a direct `kotlin.test.assertEquals(expected, productionCall(...))` oracle;
- process-local, non-serializable capability receipts.

This result does not complete the five-family E02 plan. It closes the
architectural falsifier from the first attempt and provides a safe foundation
on which the remaining binders can be evaluated.

## What changed from the rejected approach

The first implementation accepted a caller-created packet as evidence. A
caller could invent source anchors, graph edges and a validation record which
agreed with one another, then receive `COMPLETE_FOR` without any corresponding
program.

The second implementation introduces `EvidenceAuthority`. The caller may
propose a semantic thread and a test, but cannot mint the handles required to
authorize completeness. The authority itself:

1. requires the checkout to be clean and bound to the claimed Git `HEAD`;
2. rebuilds the proposed Thread IR through the live version-selected Kotlin
   worker and requires canonical equality;
3. verifies source ranges, exact bytes, hashes and `SOURCE_NODE` read facts;
4. derives the structural binding from compiler evidence instead of accepting
   role labels from the caller;
5. resolves a behavioral test with K2 and proves that its assertion consumes
   the exact production callable;
6. reruns Gradle tests, requires the exact linked JUnit testcase in the fresh
   report and binds the report hash to the exact thread/test receipt sets;
7. issues opaque, random-session, map-backed Rust capabilities which cannot be
   serialized or replayed in another authority session;
8. rechecks the Git revision and clean checkout whenever the completed theorem
   is recognized.

## Demonstrated positive path

The Kotlin 2.1 fixture contains this production flow:

```kotlin
fun transformAndConsume(input: Int): Int {
    val transformed = input * 2
    return transformed
}
```

and this independent behavioral oracle:

```kotlin
@Test
fun `transforms the produced value before consumption`() {
    assertEquals(8, transformAndConsume(4))
}
```

The live integration test obtains a theorem with the worker-derived binding:

```text
producer    param:0     (PARAMETER input)
transformer fir:9       (immutable val transformed)
consumer    fir:11      (RETURN transformed)
```

The binder requires both def-use edges, the return edge and the compiler call
result feeding the immutable definition. The test receipt requires a
compiler-resolved JUnit annotation, exactly one call to the same production
compiler symbol, and that call as the `actual` argument of
`kotlin.test.assertEquals` with an `expected` argument.

## Demonstrated refusal paths

| Attack or unsupported input | Result |
| --- | --- |
| Claimed `HEAD`, but production source exists only in the dirty worktree | `STALE_REQUIRES_RESLICE` |
| Fabricated graph edge in proposed Thread IR | `STALE_REQUIRES_RESLICE` |
| Existing test which does not call/assert the bound production symbol | `INCOMPLETE_SEMANTIC_ANALYSIS` |
| Production or test source changes after receipt issuance | `STALE_REQUIRES_RESLICE` |
| Validation receipt is reused with another thread or test set | `PRECONDITION_FAILED` |
| Receipt is used in another authority session | `PRECONDITION_FAILED` / not recognized |
| Duplicate semantic evidence with different receipt IDs | refused as non-independent evidence |
| Forged testcase text inside XML CDATA output | ignored; cannot satisfy the oracle |
| Maven project | `UNSUPPORTED_PROJECT_CONFIGURATION` |

## Product changes

- New process-local evidence authority and capability types in
  `crates/sthread/src/evidence_authority.rs`.
- Kotlin FIR graph nodes now retain the compiler-issued owner callable symbol.
- Kotlin variable declarations correctly export defines/uses for both entry
  and exit FIR declaration node shapes.
- `resolve symbol` and `cfg` accept an explicit compilation, allowing test
  source-set evidence to be inspected without filesystem search.
- Authority validation can force a fresh Gradle test execution with
  `--rerun-tasks`.
- Kotlin 2.1 fixture now exercises a real production data flow and a real
  behavioral test.

No source materializer or production transform was added in this node.

## Verification evidence

Focused producer verification:

```text
cargo test -p sthread evidence_authority --lib -- --nocapture
2 passed

cargo test -p sthread --test evidence_authority -- --nocapture
1 passed (live Kotlin 2.1 worker + Gradle validation)
```

Independent delta verification repeated those focused checks, inspected the
safe public API and found no executable false `COMPLETE_FOR`. Its verdict was
`ACCEPT` for `SUCCESS + NARROW_BINDER_FAMILIES`.

Repository regression:

```text
cargo test --workspace --quiet
PASS (all unit and integration targets)
```

`scripts/verify.sh` completed its Kotlin worker and fixture stages, then
stopped at `cargo clippy -- -D warnings` on pre-existing Rust 1.92 lint debt in
unrelated modules (`semantic-corpus`, `index`, `model`, `task_context`,
`task_plan`, and `thread_projection`). Those warnings were not broadened into
this product change; the complete workspace test suite was run separately and
passed.

## Honest boundary and next implication

This proves an architectural mechanism, not universal applicability. It does
not prove:

- the other four preregistered D02 families;
- Maven freshness;
- remote or serialized receipt transport;
- framework, lifecycle, persistence, coroutine or serialization closure;
- PSI-native source materialization;
- token, time or grep-free benchmark wins.

The next binder must reuse this authority boundary. It may expand the
supported family set only when its source, semantic relations and behavioral
oracle are independently attributable; otherwise it must refuse.
