# K2 immutable-local value extraction

The Kotlin 2.4.10 worker emits `k2-value-projection/0.1` on source functions
whose exact JVM descriptor and source owner are retained. The shared Rust
`kotlin_values::project` API validates the closed projection and converts it
to `portable-value-model/0.1`; `thread_values::analyze` calculates nullness on
modeled normal completion. This is a compiler bridge, not an authenticated
Kotlin public query. `session values` still reports the Kotlin backend as
unqualified. Java retained queries remain independently available.

## Supported operations

- Parameters are `INPUT` with unknown nullness, including parameters with a
  default expression. This profile does not turn Kotlin non-null types or
  caller-omitted defaults into fixed incoming values.
- Null and non-null FIR literals supply literal nullness. Boolean `false` is
  non-null; it is not a null-coalescing fallback condition.
- Initialized local `val` definitions in a linear body followed by one final
  return are `COPY` transfers. The expression-bodied return is supported too.
- Resolved parameter/local reads join through the exact FIR symbol object.
  Definitions and occurrences have distinct anchored IDs; shadowed names and
  repeated reads do not join by spelling.
- FIR Elvis expressions emit `COALESCE(left, right)` with the left definition
  as their guard dependency. They test null only. Logical OR, truthiness,
  general branches and path refinement are not inferred.

Smart casts are unwrapped to their original resolved occurrence. Their type
refinement does not create a new definition or a nullness guarantee. Reads of
fields, calls, safe calls and unsupported expression shapes remain anchored
`UNKNOWN` transfers. Mutable locals, statement writes, non-linear body shapes,
missing returns and extraction budget exhaustion collapse to one explicit
`KOTLIN_VALUE_BODY_OR_BUDGET_UNSUPPORTED` result. Nested callbacks are not
treated as immediately executed code.

The bridge limits a function to 128 nodes/statements and 64 expression levels.
Raw and normalized projections have a 32 KiB contract limit. A missing exact
owner or usable source mapping leaves the attachment unavailable; it cannot
become an empty successful analysis. Every model retains
`KOTLIN_VALUE_EFFECT_EXCEPTION_MODEL_UNAVAILABLE`. Normal return nullness does
not establish that a return is reached, a write commits, or a message arrives.

## Source and authority

The worker normalizes compiler UTF-16 offsets to the original UTF-8 source
bytes, preserving CRLF and Unicode source identity. Nodes carry both coordinate
domains and original line bounds. The owner, file digest, exact occurrence
ranges, operands and guards travel with the projection. The Rust file-fact
adapter rejects malformed attachments and foreign owner/file/digest bindings.
Old file facts without the optional attachment remain readable.

Structural validation of a JSON projection is not authentication of a compiler
run. The caller must verify the retained generation, compiler configuration,
source CAS closure and owner before publishing compiler authority. The solver
keeps `DECLARED_MODEL_EQUATIONS`, its binding-verification frontier, and
`programSemanticClosure=false`. No CLI accepts arbitrary projections as
authenticated evidence.

Only the 2.4.10 emitter is qualified by this slice. The shared normalization
code compiles with the 2.1.21 worker, but that worker does not emit the projection
and gains no value capability from this compatibility check.

## Focused verification

`KotlinValueFactsTest` invokes the actual 2.4.10 compiler plugin and normalizes
its output through the production documentation source adapter. It checks
deterministic output against the retained fixture, overloads, parameter/local
shadowing, repeated reads, original Unicode/CRLF ranges and explicit budget
fallback. The Rust tests consume that same native golden output, verify source
coordinates and run the portable solver. Java and Kotlin native fixtures share
the expected nullness cases in `fixtures/value-nullness-cases.json`.

```sh
./gradlew :workers:kotlin:test --tests dev.semanticthread.worker.KotlinValueFactsTest
cargo test --locked -p clew --lib kotlin_values::tests:: -- --test-threads=1
```

These checks qualify the bounded compiler projection, not arbitrary build-model
discovery, retained Kotlin public queries, general SSA, heap effects or full CI.
