# Automatic javac value extraction

The first S2b bridge is Java, not FIR. Ordinary `JavaAdapterV2` analyzer runs now
attach an optional `valueProjection` to source METHOD declarations with bodies.
`java_values::project_index(index, exact_owner)` validates that projection and
converts it into the existing language-neutral `portable-value-model/0.1`.
The caller can pass that model to `thread_values::analyze` with target `result`.
There is no new public CLI, cross-service traversal or model invocation.

## Qualified scope

`javac-value-projection/0.1` supports a deliberately closed profile:

- Ordinary parameters: reference inputs have unknown nullness; primitive inputs
  have non-null modeled normal values. An annotation does not prove non-nullness.
- A straight method body consisting of initialized local declarations and one
  final value-return statement. Locals need not be written `final`, but the
  entire body must contain no assignment, compound assignment or increment.
- Null and non-null Java literals, parentheses and direct parameter/local copies.
- `x == null ? fallback : x`, or `null == x ? fallback : x`, when both occurrences
  of `x` resolve to the same javac Element and the input definition is retained.
  This emits null `COALESCE`, not a business-value default assertion.

Definition and use matching uses resolved Elements. Distinct declarations and
occurrences have distinct anchored IDs; matching source names or source order
is not a reaching-definition algorithm. The profile does not infer heap aliases.
A write inside another local's initializer invalidates the whole immutable-body
profile. Unknown fields, calls, allocations, casts and general expressions emit
an anchored unknown transfer. Loops, branches, try/finally, statement effects,
missing value returns and other body shapes emit whole-body unknown.

Preflight scanning, expression depth, node count and serialized bytes have
separate bounds. A profile budget failure collapses to an explicit unknown
projection rather than publishing clipped equations. When the additional field
would exceed the enclosing declaration's fact budget, it is removed with a
compiler boundary; the bridge then reports projection unavailable. Existing
source/annotation/declaration facts retain their independent meaning.

## Evidence and authority

Every projected node carries its exact enclosing file, compiler UTF-16 offsets,
UTF-8 byte offsets and original line bounds. Validation rejects another owner,
foreign files, clipped/outside anchors, open operations, missing result roots,
dangling references and invalid guards. It checks closed-profile shape, not
whether arbitrary serialized input was genuinely emitted by a compiler.

The bridge requires one exact source-membership binding for the selected file.
It binds all source digests, the compiler index, project model and bridge policy.
Evidence IDs reference the canonical enclosing declaration fact plus node ID;
they are not separate guessed source bodies. Parent compiler boundaries become
global model boundaries. Effect and exception modeling is explicitly unavailable.

The analyzer source participates in adapter/index digests. Changing extraction
invalidates stale cached compiler authority. Old readers may reject the optional
field under their closed schema; new readers accept old facts, but the value
bridge reports missing/stale projections instead of an empty successful result.
The authenticated `session values` loader verifies saved compiler-run bindings;
see [retained value queries](retained-value-queries.md). Structural index parsing
alone does not assert compiler-run authenticity.
The solver still returns `DECLARED_MODEL_EQUATIONS` and semantic closure false.

## Verification

The native qualification fixture launches the actual JDK source-mode compiler
analyzer with exact input files, empty classpath/sourcepath and annotation
processing disabled. It checks repeated output, parameter/local dependency,
null-default, nullable copies, literal null/non-null, UTF-16 versus UTF-8 offsets,
unknown fields/calls, hidden writes and budget/control fallbacks. A separate
oversized-owner subcase checks the emitter boundary and expected rejection by
the existing JVM annotation contract; successful emission is not index admission.
Its project
model is an explicitly admitted fixture, not a claimed native Gradle/Maven
model-discovery run. Normal unit tests exercise projection binding validation
without starting a compiler. Native qualification is an explicit ignored test:

```sh
cargo test --locked -p clew --lib java_values::tests -- --test-threads=1
cargo test --locked -p clew --lib java_values::tests::native_javac_extracts_values_and_fails_closed_on_writes_control_calls_and_budget -- --ignored --test-threads=1
```

This qualification establishes the bounded Java bridge. Kotlin/FIR, C#/Roslyn,
TypeScript/JavaScript and syntax adapters do not gain this capability merely by
having a compiler registry entry or using the same portable solver. The next
bridge must independently prove definition/operand/evidence mappings against
shared fixtures. General SSA, path refinement, interprocedural summaries,
exceptions, field/alias transfers and process publication integration remain next slices.
