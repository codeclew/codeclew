# E03: authority-backed MAP_EDGE_WITH_CONTEXT

Date: 2026-08-09
Status: ACCEPT for the narrow contour documented here

## Question

Can Codeclew understand a small but cross-cutting Kotlin change without asking
the model to name files, symbols, source fragments, or edit operations? More
specifically, can it find a value flow from a collection producer to a consumer,
select a compatible context producer and transformer, and compute the conditions
that must remain true after the future edit?

E03 deliberately stops before source mutation. Its output is a checked change
theorem, not a patch.

## Public product path

```bash
clew prove map-edge-with-context \
  --repo /path/to/clean-kotlin-repository \
  --compilation :/main \
  --workflow-symbol com.example.pendingValues \
  --test-symbol 'applies the mapping context to one value' \
  --test-compilation :/test
```

The command returns exactly one decision:

- `BOUND`: one safe binding was found and all required invariants were proved;
- `AMBIGUOUS`: several safe bindings exist and the caller must choose from a
  bounded list;
- `REFUSED`: the available evidence cannot justify the change.

No decision contains replacement text or a source edit. Ambiguous and refused
decisions contain no change graph.

## What is inferred

For the supported contour Codeclew derives all of the following from a clean,
committed checkout and live Kotlin 2.1 compiler evidence:

1. the collection element type `T`;
2. the unique value edge from the workflow parameter to its return;
3. a compatible context producer `() -> C`;
4. a compatible transformer `(T, C) -> T`;
5. a placement strategy that evaluates the context once and maps the eager
   collection edge;
6. a compiler-linked behavioral test that asserts the transformer result and
   consumes the selected context producer;
7. a closed change graph containing fifteen obligations.

The implementation does not choose candidates by repository vocabulary. The
positive test renames all relevant functions and moves both production and test
files to another package path before asking for the proof.

## Computed invariants

Every `BOUND` result contains evidence fingerprints for exactly twelve
invariants:

1. transformed values remain assignable to the consumer type;
2. context is evaluated once;
3. placement dominates every transformed use;
4. element order is preserved;
5. cardinality is preserved;
6. eager/lazy behavior is preserved;
7. effects remain within the supported pure contour;
8. nullability is preserved;
9. the consumer contract is preserved;
10. public ABI is preserved;
11. a compiler-linked behavioral oracle is available;
12. no unsupported boundary is crossed.

These invariants are not caller-provided booleans. The authority rebuilds the
thread through the worker, resolves source anchors against the clean Git tree,
indexes declarations with K2, resolves candidate callables, checks effects,
resolves the test assertion, and reruns the configured Gradle test lifecycle.
Only then does it issue a process-local, non-serializable proof receipt.

## Executable evidence

The focused product tests cover:

| Case | Expected decision | Meaning |
|---|---|---|
| renamed functions and relocated files | `BOUND` | binding is structural and typed, not name/layout-specific |
| two compatible context producers | `AMBIGUOUS` with two choices | the engine does not guess |
| `Sequence<T>` workflow | `REFUSED: UNSUPPORTED_COLLECTION_MODALITY` | laziness is not silently changed |
| transformer with observable output | `REFUSED: UNKNOWN_EFFECTS` | unknown effects are not accepted as pure |
| unrelated test | `REFUSED: MISSING_BEHAVIORAL_ORACLE` | any passing project test is insufficient |
| compatible methods requiring an object receiver | `REFUSED: NO_COMPATIBLE_CONTEXT_AND_TRANSFORMER` | an unbound receiver is not hidden inside a top-level plan |

The `BOUND` result contains twelve applicability/invariant proofs and fifteen
change obligations for the future materializer. It does not claim that a patch
has already satisfied those postconditions. Serialization checks confirm that it contains none of
`sourceText`, `replacement`, `regex`, or `EditIR`.

An end-to-end run of the public binary on an isolated clean Kotlin 2.1 checkout
completed in 20.9 seconds and returned this compact projection of the result:

```json
{
  "status": "BOUND",
  "bindings": {
    "workflowSymbol": "com/acme/valuesAwaitingContext",
    "contextProducerSymbol": "com/acme/mappingContext",
    "transformerSymbol": "com/acme/applyMappingContext",
    "collectionType": "kotlin/collections/List<kotlin/Int>",
    "elementType": "kotlin/Int",
    "contextType": "kotlin/Int",
    "placement": "com/acme/valuesAwaitingContext#FUNCTION_ENTRY",
    "strategy": "KOTLIN_EAGER_LIST_MAP_WITH_CONTEXT_ONCE"
  },
  "invariantCount": 12,
  "obligationCount": 15
}
```

Focused commands:

```bash
cargo test -p clew --test goal_binding \
  map_edge_with_context_binds_renamed_layout_and_computes_every_invariant
cargo test -p clew --test goal_binding \
  map_edge_with_context_returns_bounded_ambiguity_and_structured_refusals
```

## Supported contour and non-claims

The current accepted product contour is intentionally narrow:

- Kotlin 2.1 JVM project with Gradle;
- clean committed checkout;
- direct, linear `List<T>` parameter-to-return value edge;
- one existing non-null `() -> C` producer and `(T, C) -> T` transformer;
- both callables are top-level and have no extension, dispatch, or context
  receiver;
- transformer composed only from the current known-pure Kotlin numeric
  intrinsic allow-list;
- direct `kotlin.test.assertEquals(expected, transformer(value,
  contextProducer()))` oracle.

E03 does not yet prove safe source materialization. It does not support
`Sequence`, `Flow`, callbacks, aliases, branches, loops, reflection, Maven, an
unknown-effect transformer, or a test whose connection to the selected
producer and transformer cannot be compiler-resolved. Those cases must refuse,
not degrade to heuristics.

The result therefore proves a mechanism: Codeclew can turn a small typed intent
into a compact, authority-backed change graph and explicit preservation
conditions without grep-style discovery or a model-authored source plan. It
does not yet prove broad applicability or an end-to-end time/token advantage.

## Next product edge

The next implementation step is to compile this proof into PSI-native edits in
an isolated worktree, then validate the same twelve postconditions against the
candidate before commit. Benchmarking is meaningful only after that product
edge exists; until then E03 measures understanding and refusal quality, not
editing speed.

## Independent verification

The first independent review returned `REJECT` with one executable false
`BOUND`: compatible-looking producer and transformer methods inside a class
were accepted even though the plan did not bind the required object receiver.
The public CLI therefore described a plan the future materializer could not
execute from `FUNCTION_ENTRY`.

The bounded repair requires compiler identities with empty
`containingDeclarations`, `receiverTypes`, and `contextReceiverTypes`. Missing,
null, or non-empty fields fail closed. A focused regression reproduces the
member-only program and now receives
`REFUSED: NO_COMPATIBLE_CONTEXT_AND_TRANSFORMER`.

The same review identified that integer division and remainder may throw;
those operations were removed from the known-pure allow-list and covered by a
unit regression.

The independent delta re-verification verdict is `ACCEPT`. It confirmed that
member, extension, and context-receiver callables cannot enter either candidate
set and found no remaining blocker in this repair. This is acceptance only for
the narrow contour above, not for source materialization, other task families,
or universal applicability.
