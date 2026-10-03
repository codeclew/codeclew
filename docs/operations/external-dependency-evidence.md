# External Java dependency target evidence

The Java compiler provider emits `DEPENDENCY_TARGET` for a resolved method or
constructor from an admitted classpath artifact. Docs capture retains this as a
separate observation. `CALL_RELATION.targetIdentity` joins to its
`symbolIdentity` only within the same compilation scope. Endpoint context
retains dependency targets reached by exact `CALLS` or `CONSTRUCTS` relations.
`TYPE_USES` becomes a separate `TYPE_RELATION` with a `typeSite`, preserving
external types used by selected declarations and constructors without treating
type references as invocations.

The observation carries the compiler owner, declaration name, JVM descriptor,
declaration kind, modifiers and compiler module. `binaryOrigin.artifact` binds
the exact admitted artifact's logical name, digest, size and kind;
`binaryOrigin.classEntry` identifies its class entry. Logical names are portable
identities, not local artifact locations.

`binaryMetadata` contains only metadata observed inside that artifact. Maven
coordinates come from `META-INF/maven/*/*/pom.properties`; automatic module and
implementation version values come from the JAR manifest. `versionStatus`
distinguishes exact, unavailable and ambiguous metadata. A filename or package
name is not version evidence, and metadata does not identify an artifact beyond
its admitted digest.

`resolution: COMPILER_EXACT` establishes the compile-time declaration and
selected overload. An unattached method records
`sourceStatus: SOURCE_NOT_ATTACHED` and `bodyStatus: BODY_UNAVAILABLE`;
type declarations record `BODY_NOT_APPLICABLE`. The caller's
`callSite.sourceStatus: SOURCE_RETAINED` refers to the caller only.

An admitted source archive at the expected Maven classifier path can supply a
`dependencySource` whose separately compiled declaration identity and modifiers
match the binary target. The matching basis is
`MAVEN_CLASSIFIER_PATH_AND_COMPILER_SIGNATURE`; an adjacent filename does not
prove source-to-bytecode equivalence. Docs
retains its exact declaration snippet as a separate
`EXACT_DEPENDENCY_SOURCE_ARCHIVE` source record. The observation keeps the source
archive authority, entry, full content digest, snippet digest, coordinates and
matching basis, without duplicating text. `SOURCE_ATTACHED` establishes that
binding; an unmatched or unverified attachment keeps its explicit failure
status. `BODY_SOURCE_AVAILABLE` requires a matching concrete method body;
abstract declarations still have `BODY_UNAVAILABLE`.
Optional attachment failures retain a `sourceBoundary` on the target and do
not downgrade exact binary resolution or compiler coverage. Source verification
is bounded to 32 distinct archive entries per compilation; excess entries keep
`SOURCE_ATTACHMENT_UNVERIFIED` with the explicit verification-budget boundary.

No source text is reconstructed from a class file.
`runtimeImplementationStatus: UNRESOLVED` remains separate from binary
resolution and source availability. These facts do not establish an injected
runtime implementation, execution of a dependency body, HTTP route or payload
contract. Endpoint context retains available target source but does not traverse
it as a resolved runtime implementation.

Capture and inspect through the supported launcher:

```sh
clew docs evidence capture --root docs --service orders --output orders-evidence
clew docs evidence inspect --input orders-evidence
clew docs evidence read --input orders-evidence --kind observations --limit 100
```

Inspection verifies integrity without admitting the package. Configure the
ordinary coordinator expectation and use `docs evidence import` to admit it;
the target observation then remains available through saved docs evidence
without the source checkout or JAR. Package integrity does not revalidate the
current dependency or establish complete Maven build authority.

For retained cache-directory exports, follow
[the export recovery runbook](documentation-export-recovery.md). The selected
capture and recovered snapshot preserve the target observation, artifact
binding and availability statuses. Recovery retains source authority as
`RETAINED_SOURCE_NOT_REVERIFIED`; it does not make a non-cacheable Maven capture
reusable. Use supported lifecycle commands rather than editing SQLite or CAS
objects.

The focused regression uses a public synthetic interface with overloaded
methods, constructor injection, a real JAR and the real javac provider. Its
Maven wrapper supplies deterministic goal outputs for admission, so it verifies
compiler target provenance and portable docs retention without claiming a
production Maven build:

```sh
cargo test --locked -p clew --test docs_dependency_targets -- --ignored --test-threads=1
```

JDK 21 and the repository's pinned Rust toolchain are required. The regression
checks that the selected overload survives package admission and portable
snapshot recovery, that caller source remains separate from absent dependency
source, and that saving unchanged evidence reuses its immutable payloads.
