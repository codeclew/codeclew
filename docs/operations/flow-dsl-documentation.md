# Static flow DSL documentation

`docs dsl render` interprets a bounded Java flow DSL from an original retained
`docs check` snapshot. It emits a typed projection, MDX 3, standalone HTML, a
local stylesheet, a manifest, and a retained source appendix. Rendering runs no
compiler, build, provider, model, or network request. Snapshot loader integrity
and admission errors are preserved.

```sh
./clew docs dsl render --root /path/to/documentation \
  --snapshot ORIGINAL_SNAPSHOT --service example \
  --profile fixtures/flow-dsl/queue-profile.json --output /path/to/new-output
```

The documentation repository must already contain the selected service and
snapshot. Capture synthetic examples with `docs init`, `docs service add`,
`docs bind`, and `docs check` using the `source-syntax` Java profile. The public
CLI integration test `flow_dsl_static_render_uses_original_supported_snapshot`
exercises that path for both examples in `fixtures/flow-dsl`. The Java fixture
illustrates source syntax; it is not an executable framework simulator.

The output directory may contain unrelated files. A collision with any generated
filename rejects rendering before writing files. Use a fresh directory for each
render. The renderer does not overwrite user material or release a publication.

## Declared profile

Profiles use the closed `codeclew-flow-dsl-profile/1.0` JSON schema. The family
is `builder-chain` or `operation-queue`. The framework and optional engine
version are supplied declarations; an absent engine version stays unresolved.
The profile chooses exact source file, qualified owner, and method bindings for
construction, optional factory and registry, plus a context owner/file.
Same-name overloaded source methods are rejected as ambiguous. An optional
`compilerSymbol` and `scope` prefer an exact retained compiler declaration;
otherwise the adapter requires retained syntax declarations and labels that
fallback. Compiler declaration binding does not upgrade DSL interpretation to
runtime or compiler-proven behavior.

Builder profiles bind an exact builder receiver/method and root variable.
Supported construction uses a direct method-reference group assigned to that
root, followed by `.then(...)` groups rooted at the same variable. A group of
callbacks does not prove parallel execution. Queue profiles bind a constructor
type, initial argument offset, and registry receiver; `.put(Operation.KEY,
Context::method)` establishes an unordered lookup binding. Constructor arguments
establish declared initial queue order, not dequeue/delivery policy. A selected
factory's constant task-type return establishes a declared type binding, not
runtime registration or trigger activation.

`decisionMethods` selects context methods whose returns or `selectionCalls` may
contain operation constants, `.name()` constants, `List.of`, `Arrays.asList`,
`Collections.singletonList`, or `Collections.emptyList`. Local `if` and `else`
paths stay attached to selected next operations. Condition helper semantics stay
unresolved. Assignments and explicitly classified calls are effects; they are
never promoted to conditions. Other helper calls retain their own unresolved
records. A `this::method` belongs to the construction owner. Selecting a
different context owner leaves wrapper delegation unresolved; name equality does
not bind that delegation.

Effects bind source receiver and method and declare one of `mapping`,
`state-change`, `persistence-call`, or `opaque-external-call`. Optional
`compilerTarget` narrows matching when an exact retained compiler call relation
exists for the same source occurrence and scope. Effect classification and
reader labels are authored profile content. A persistence call does not establish
a database engine or committed row. An outbound ODM or other decision request is
an opaque external boundary; the adapter does not infer local rules, Rete,
Drools, destination configuration, delivery, or successful response semantics.

## Reading and limits

One typed projection drives both formats. The static overview uses at most seven
editorial groups; long callback chains are grouped and details remain reachable
through native disclosures and canonical anchors. Short flows keep their actual
size. Category order is a reading aid, not an execution sequence. Source references
retain the original service revision, source authority, text digest, and line
range. Text is escaped as inert content in HTML and MDX. HTML requires no
JavaScript, external font, API, or network dependency.

Loops, switches, lambdas, anonymous/local classes, short-circuit expressions,
conditional construction, unsupported callbacks, and unsupported operation
expressions produce local gaps. The adapter conservatively marks a trailing
block after a conditional exit as control-dependent rather than reporting it as
unconditional behavior. It does not infer engine activation, exception/retry
policy, queue merging, ordering across categories, repeat rules, runtime
completion, or incident causality. Mixed service/revision sources and missing or
ambiguous method bindings reject the projection. No scenario evaluator or
universal rule-engine support is included.

For focused verification:

```sh
cargo test --locked -p clew --lib documentation::flow_dsl::tests -- --test-threads=1
cargo test --locked -p clew --test managed_cli flow_dsl_static_render_uses_original_supported_snapshot -- --test-threads=1
```

Set `CODECLEW_FLOW_DSL_TEST_OUTPUT` to an existing parent directory to keep the
integration test's synthetic `builder` and `queue` bundles for layout and MDX
compilation review. This test-only artifact hook is not a snapshot cache API.
