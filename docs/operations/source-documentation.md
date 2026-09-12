# Documentation from committed source without a build

Status: development source; not an already published 0.7.1 capability.

The `source-syntax` profile uses the existing documentation catalogue, narrative,
binding, and renderer models. Python, Java, and Kotlin sources can be documented
when imports, libraries, Maven/Gradle configuration, or K2 are unavailable.
The installed application contains pinned Tree-sitter grammars. Baseline analysis
executes Git plumbing, never project code or a compiler. Building Codeclew itself
from source still requires its normal development toolchain.

## Register a source scope

Initialize a separate documentation repository and adapt
`examples/service-source.json`. Use the checkout's credential-free origin URL.

```sh
clew docs init --root /work/architecture --title 'Reservation architecture'
clew docs service list --root /work/architecture
clew docs service add --root /work/architecture --input /work/orders-source.json --expected-input-digest RETURNED_INPUT_DIGEST
clew docs bind --root /work/architecture --service orders --repo /work/orders
clew docs check --root /work/architecture
clew docs context --root /work/architecture --service orders --format compact --limit 100
```

Example service record:

```json
{
  "schema": "codeclew-documentation-service/1.0",
  "id": "orders",
  "title": "Order reservations",
  "repositoryId": "orders",
  "repository": "https://example.invalid/orders",
  "language": "kotlin",
  "profile": "source-syntax",
  "targetRef": "main",
  "source": {"roots": ["src", "pom.xml"], "dialect": "1.9"}
}
```

Use `python` or `java` with their declared dialect for those languages. `roots`
selects literal Git paths; `.` selects the whole committed tree. Include relevant
configuration and helpers. Out-of-scope data and arbitrary external agent reads
are not automatically observed. Expand the registered scope before relying on
them, or explicitly report incomplete influence tracking. Dirty and untracked
working files are excluded; use a committed revision. Binding does not change
source files or require the checkout to move to the selected revision.

Capture allows at most 2,048 files, 2 MiB per file, 32 MiB of input and retained
source text, and 32,768 observations/source fragments. Parser traversal and time
are bounded. Unsupported textual files retain exact text with `FILE_ONLY`
authority boundaries; binary inputs retain inventory digests. Symlinks, missing
roots, empty scopes, and recovered parse errors remain explicit gaps. Exceeding
a budget fails the capture rather than silently dropping the remaining files.

## Author one explanation, render several views

```sh
clew docs context --root /work/architecture --service orders --symbol example.Reservations.reserve --format compact --limit 100
clew docs context --root /work/architecture --service orders --source RETURNED_SOURCE_ID --format raw
clew docs context --root /work/architecture --service orders --dependency RETURNED_DEPENDENCY_ID --format raw
clew docs render --root /work/architecture --input /work/orders-narrative.json
```

A qualified name must select exactly one declaration; overloads need an exact
returned identity. Source entrypoints are callable declarations, including
unannotated helpers, rather than confirmed framework endpoints. Compiler
profiles retain their existing endpoint discovery. Use explicit gaps for
callables whose behavior is not yet authored. Follow `nextCursor` and inspect
`omitted`; compact output is a projection of the same selected evidence, not a
new context planner. `SOURCE_ALIAS` names a covering source fragment and provides
an exact drill-down reference. Source ranges and flow records remain available.

Write narrative schema 1.3 using the
[authoring example](../../skills/codeclew/references/authoring-example.md).
Explain inputs, decisions, state changes, and outcomes. Bind statements and
diagram elements to source/observation IDs. The renderer checks references and
requires relevant branch/return coverage; it cannot prove arbitrary prose true.
A unique source selector returns `SOURCE_MATCH`, never compiler `RESOLVED`.
Lexical events retain parent ordinals, syntax kinds, and exact snippets. They do
not establish runtime order, resolved call targets, overload selection, inferred
types, extension dispatch, or coroutine scheduling. Engineer-declared HTTP/Kafka
handoffs remain declarations when their linkage cannot be resolved.

The same accepted explanation produces the existing offline HTML, Markdown,
Mermaid, detailed sequence, and optional overview/state abstraction. Manual prose
and existing generated-output edits remain protected. `--require-complete`
rejects missing authored operations and partial source capture.

## Check and selectively review changes

```sh
clew docs check --root /work/architecture
clew docs changes --root /work/architecture --limit 100
clew docs changes --root /work/architecture --fragment RETURNED_FRAGMENT_ID --limit 100
```

A logical source ID is separate from its immutable revision/blob/byte occurrence.
Whitespace relocation preserves unique callable bindings and updates links.
Token/tree fingerprints preserve literals, comments, docstrings, and Python
nesting. Renames, changed signatures, duplicate declarations, or deleted roots
are missing/ambiguous correspondence, not guessed matches.

Every source-based fragment conservatively depends on the whole declared scope
of each involved service. This catches changed helpers/configuration and added or
deleted files, including formerly empty catalogues. It can over-invalidate within
a service; it is not minimal semantic influence analysis. Independent services
remain independent. Partial source capture is unresolved for freshness.

The change dossier retains the old claim, affected view ID, changed observations,
before/after source text, and references to more scope context. Older bundles
without retained text explicitly report unavailable history. Pages and individual
items have byte limits; omissions carry their identity and reason. No model runs
inside this command. Supply a reviewed narrative with the new `contextDigest`
when needed; unchanged explanations can be rebound during rendering. All views
are written into one immutable bundle before its index pointer changes. A failed
render leaves the prior pointer and manual content intact.

## Optional compiler enrichment

Add this field inside `source` when semantic evidence is wanted:

```json
{"semantic":{"profile":"java-17plus-maven-read-only","compilation":":/main"}}
```

Kotlin uses its existing `kotlin-jvm-maven-analysis` or Gradle analysis profile.
This explicit option invokes the normal admitted compiler workflow. Facts attach
to source roots only at a unique equal-revision file/name/line-range match.
Unmapped or synthetic facts remain gaps. The source IDs and narrative subject do
not change when a provider is restored. Provider loss removes semantic facts and
requires review of their dependants, while syntax remains readable. Source calls
are not automatically promoted to compiler-resolved graph edges.

Kotlin 1.9 source parsing needs no K2. Optional semantic analysis still records the
packaged analyzer's language/API 2.0 compatibility boundary; it is not native 1.9
execution. Enrichment currently uses the broad source watch, so reduced review
fan-out has not been established.

## Reproduce focused acceptance checks

```sh
cargo test --locked -p clew --lib documentation::syntax::tests:: -- --test-threads=1
cargo test --locked -p clew --test managed_cli durable_source_documentation_without_build_tools_rebinds_and_preserves_publication -- --exact --test-threads=1
cargo test --locked -p clew --test managed_cli durable_source_documentation_java_enrichment_recovers_on_the_same_source_roots -- --exact --ignored --test-threads=1
```

The fixtures are in [durable-docs-source](../../fixtures/durable-docs-source/README.md).
The initial [token-economics measurement](../product/validation/source-documentation-value-first-stage.md)
compares released Codeclew, native tools, and manually prepared oracle context.
It does not measure this implementation's model-token savings. Byte reductions
from compact output must not be reported as measured model-token or cost savings.

The subsequent [implementation qualification and paired projection study](../product/validation/source-documentation-qualification.md)
records executed availability checks and the limited observed compact-token effect.
