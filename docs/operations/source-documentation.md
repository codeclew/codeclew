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

## Declare OpenAPI contracts independently

Set `contractFiles` on the service record to an explicit list such as
`["api/openapi.yaml", "api/types.yaml"]`. These committed files are captured
independently of language roots and compiler availability. The `openapi` module
exposes its tested versions through `docs modules show --id openapi`: 3.0.0 and
3.0.3. Other versions remain unsupported with a named gap and retained input.
Reference-only registered files do not need an `openapi` header.

The reader preserves nested schemas and constraints, inherited/overridden
parameters, responses, security schemes, and server declarations. Relative file
references resolve only within the explicit registration; network references,
missing files, cycles and unresolved pointers remain visible gaps. No network
fetch, schema instance validation or runtime enforcement is performed. Callbacks
are retained inside their declaring operation but are not mapped to source routes.
Reference siblings are flagged rather than silently treated as merged schemas.
Capture is bounded to 128 files, 2 MiB each, 16 MiB total, 100,000 expanded values
and 32 levels. Excessive expansion fails with an explicit diagnostic.

Every declared operation appears in the service contract data, even without a
matching source endpoint. The HTML navigation also shows unmatched declarations.
A unique method/path match is only a source-route comparison; it does not prove
payload compatibility or deployed behavior. Changing a registered contract or
reference input invalidates dependent service/process claims, including when
language source roots are unchanged.

## Author one explanation, render several views

```sh
clew docs context --root /work/architecture --service orders --symbol example.Reservations.reserve --format compact --limit 100
clew docs context --root /work/architecture --service orders --source RETURNED_SOURCE_ID --format raw
clew docs context --root /work/architecture --service orders --dependency RETURNED_DEPENDENCY_ID --format raw
clew docs render --root /work/architecture --input /work/orders-narrative.json
```

A qualified name must select exactly one declaration; overloads need an exact
returned identity. Source entrypoints retain callable identities, including unannotated helpers.
Java/Kotlin annotations with fully qualified names or explicit imports also
supply Spring declaration triggers through shared Rust rules. These are source
declarations with runtime-registration gaps. Compiler profiles retain their
existing endpoint discovery. Use explicit gaps for
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
clew docs check --root /work/architecture --service orders
clew docs refresh --root /work/architecture --status-only
clew docs changes --root /work/architecture --limit 100
clew docs changes --root /work/architecture --fragment RETURNED_FRAGMENT_ID --limit 100
```

Default `docs render` publishes valid operations despite unrelated unavailable
repositories or rejected inputs. A failed operation update retains its previous
text, exact snippets and source revision; another operation on the same page may
use newer evidence. Inspect `updateFailures` and operation states. Selective
`docs check --service` explicitly marks unselected services as not checked.
`--require-complete` retains the strict, non-publishing diagnostic behavior when
any section is stale, unavailable, rejected or explicitly incomplete.

`docs refresh --status-only` publishes a new immutable status snapshot without
calling an agent. It retains the old explanation and exact source snippets,
shows content versus target revisions, and marks unavailable inputs locally.
Meaning review remains unassessed for legacy/direct Narrative content. A status
refresh cannot remove a prior review obligation or overwrite edited generated
files. HTML, Markdown and diagram exports carry the same operation status.

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

Inspect producer capabilities without running a build:

```sh
clew docs modules list --root /work/architecture --service orders
clew docs modules show --root /work/architecture --id kotlin-k2
```

Results distinguish registered producers from project admission, expose input/
output schemas and implementation digests, and report project/analyzer versions.
Kotlin workers require JDK 21 independently of the project's JVM target; configure
`CODECLEW_WORKER_JAVA_HOME` separately from the project's `JAVA_HOME`. A Java 17
project target does not require the analyzer to run on Java 17. Project plugins,
options and compiler compatibility still pass the existing admission checks.

The versioned `modules` field belongs at the service record's top level:

```json
{
  "modules": {
    "schema": "codeclew-documentation-modules/1.0",
    "semantic": {
      "module": "javac",
      "enabled": true,
      "profile": "java-17plus-maven-read-only",
      "compilation": ":/main"
    }
  }
}
```

Use `kotlin-k2` with an existing Kotlin analysis profile for Kotlin. Omit `semantic`
or set `{"module":"javac","enabled":false}` to keep only source evidence.
The source profile remains enabled. Module configuration cannot contain commands
or select arbitrary adapters from repository files. Wrong-language selections,
unknown fields and simultaneous explicit/legacy settings are rejected.

Legacy `source.semantic` remains supported when `modules` is absent:

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

Module implementation/rule digests, selection and availability participate in the
conservative service scope. Provider loss preserves source readability and marks
relevant dependent evidence changed; rule changes can require review without a
source commit. A module name never promotes syntax or declared facts to runtime
proof. The shared Spring module consumes compiler facts and separately qualified source
annotation facts. Its authority follows the input; source spelling never becomes
compiler resolution. See the [tested compatibility matrix](../../fixtures/documentation-system/spring/README.md).

## Standard service sections and domain identities

Registration exposes five stable section roots: overview, responsibilities,
domain entities, ingress contracts and egress contracts. They are immediately
inspectable, even before source binding. `docs render` publishes the common page
structure with explicit gaps, including registered services whose source capture
is unavailable. Author one section through the same bounded author/reviewer path:

```sh
./clew docs section list --root /path/to/docs --service orders
./clew docs section show --root /path/to/docs --service orders --id section-overview
./clew docs section prepare --root /path/to/docs --service orders --id section-overview
./clew docs work run --root /path/to/docs --work WORK_ID --config /path/to/execution.json
```

Section preparation does not invoke a model. A section proposal uses its supplied
`SECTION` reference in the existing `operations[].entrypoint` field, an
exact-source-supported `summary`, empty `steps` and no sequence contracts. This
compatibility mapping preserves old operation IDs; sections are never inserted
into source entrypoint inventory. Older narratives may omit section gaps, which
the publisher supplies. Partial section failure retains accepted siblings.
Internal source callable records are distinct from discovered public boundaries;
selected modules, dynamic registration and runtime activation limits remain
visible. Forty-endpoint services use the same navigation as small services.

Domain entities live in `catalog/entities` with explicit stable IDs. Inspect the
[entity schema](../../schemas/documentation/entity.schema.json), then register a
record with `docs entity put --root /path/to/docs --input entity.json
--expected-input-digest DIGEST`. `docs entity list` returns the current digest.
Use `--human` for an explicit maintainer declaration. Without that flag, writes
cannot add, replace or remove human relationship records. Sandboxed author and
reviewer jobs have no catalogue write capability. Agent-proposed ownership is
labelled separately and does not become a human ownership declaration.

Relations name `created`, `changed`, `read`, `stored-copy` or `owned`, with origin,
confidence, rationale and optional source dependencies and implementation
representations. Classes, DTOs and table names are representations, not domain
IDs. Duplicate titles remain separate candidates; renaming a title preserves the
ID. Links require explicit existing entity IDs and are never rebound by name.
Entity and contract dependencies propagate through document bindings. Missing
declared evidence remains an explicit gap. These records express declared or
proposed domain interpretation, not runtime ownership proof.

## Protected notes and separate assessments

Human notes are bounded UTF-8 Markdown under `notes/`, outside generated files.
Imports preserve all original bytes, including frontmatter, line endings, tags,
formatting and embedded text. The reader shows escaped original text, so embedded
HTML and instructions cannot become executable page content. Associations live
in `catalog/notes`; they never rewrite the original. See the
[note association schema](../../schemas/documentation/note-association.schema.json).

```sh
./clew docs note list --root /path/to/docs
./clew docs note inspect --root /path/to/docs --path notes/existing.md
./clew docs note import --root /path/to/docs --input association.json \
  --source /path/to/original.md --expected-input-digest DIGEST
./clew docs note associate --root /path/to/docs --input association.json \
  --expected-input-digest DIGEST --expected-note-digest NOTE_DIGEST
./clew docs note prepare --root /path/to/docs --id policy
./clew docs work run --root /path/to/docs --work WORK_ID --config execution.json
./clew docs note remove --root /path/to/docs --id policy --expected-input-digest DIGEST
```

Import requires a new destination and note ID. To associate an existing file,
use the original digest returned by `note inspect`; after association `note list`
returns that digest too. A rename
requires an explicit association update using the same ID. A missing old path
stays absent until that update; matching titles never cause a rebind. Removal
deletes only the association. Local import source paths are not persisted.

Targets explicitly name `service:ID`, `service:ID/section-overview` (or another
standard section), `entity:ID`, or `scenario:ID`. They must exist when associated.
The assessment service is explicit and may differ from a related object's
service. Classification (`fact`, `historical-context`, `policy`, `intention`,
`opinion`, `mixed`), declared period, tags and arbitrary bounded metadata retain
human/imported authority. A fact classification is not automatic verification.

Optional assessments use a supplied `NOTE` work reference as the proposal root,
an evidence-bound summary and separate assessment metadata. Outcomes are
`CONSISTENT`, `CONTRADICTED`, `HISTORICAL` or `UNKNOWN`; proposed corrections are
separate claims with their own evidence. Without source evidence, the assessment
must remain `UNKNOWN` with a limitation and no correction. Historical conclusions
must name `revision:FULL_SHA` matching their captured source. Calendar periods,
intentions and undocumented history need additional evidence or an explicit
unknown; current code cannot establish them. Deterministic checks bind inputs and
structure; the isolated reviewer assesses meaning and cannot grant human authority.

Author/reviewer jobs cannot write original notes or association records. Concurrent
text or association edits invalidate prepared work; later changes mark accepted
assessments stale. Status-only refresh keeps the original publication snapshot
and flags changed note inputs. New generation can display the new original beside
a stale retained assessment; their digests and provenance remain separate.
HTML navigation/search and Markdown/JSON exports include related notes. Evidence
tracking covers captured text, associations and recorded inputs, not every implicit
claim in arbitrary prose. Limits are 128 associations, 256 KiB per note and the
existing bounded work-read budget; an oversized required read remains a named gap
and cannot be silently accepted. This is not a general Markdown execution engine.


## Saved process definitions

Use `docs process inspect --root <docs> --input <process.json>` for a transient
source interpretation: it saves no process, work request or publication.
Explicitly requested maintained processes use
[the process schema](../../schemas/documentation/process.schema.json) and
[the quantity fixture](../../fixtures/documentation-system/processes/quantity.json).
The definition records requested scope and outcomes, not an assertion that they
are implemented. Its `process` metadata names registered participants, optional
`entity:<id>` objects, a trigger, outcomes and optional `linkedSubviews` IDs.

```sh
./clew docs process list --root <docs>
./clew docs process put --root <docs> --input <process.json> --expected-input-digest <digest>
./clew docs process show --root <docs> --id <process-id>
./clew docs process prepare --root <docs> --id <process-id>
./clew docs process prepare --root <docs> --id <process-id> --overview
```

Keep an existing ID when changing a title or definition. `put` uses the current
catalogue input digest and saves only the explicit definition in `scenarios/`.
Legacy scenario files remain supported. These commands never save chat history.
Agent execution cannot rewrite the saved definition. A maintainer may explicitly
update it with `put`; the existing authorization to maintain it is sufficient.

The overview is separately reviewed through the existing work/proposal pipeline.
A proposal uses `scenario:<process-id>` as its entrypoint reference; an overview
request maps that to `process-overview`. Source flow and the overview keep
independent accepted versions. Linked child explanations are admitted only when
their accepted claim, definition and source influence are current. Every parent
fragment tracks the child version and transitive dependencies. Changing a child
accepted explanation marks retained parents stale in the same publication.
Status-only refresh retains old prose and clearly marks changed child captures.

Each detailed scenario retains its eight-service composition limit, depth at most
16 and node count at most 512. A definition has at most 32 direct child links;
composition traverses at most 64 linked definitions and 16 levels, with named
cycle, missing-child and budget gaps. The existing scenario work capture retains
its conservative registered-service influence boundary. No source-only declared
link becomes a
compiler-resolved or observed runtime transfer. Unavailable participants and
unaccepted child prose remain visible gaps. The reader and Markdown export
separate requested metadata, reviewed overview, detailed flow and linked views.

## Saved entity data-flow views

`docs view modules --root <docs>` exposes the built-in versioned view contract:
input objects, representation/edge kinds, authority, dependency derivation,
validation, renderer identity and limitations. The first implementation is
`entity-dataflow/1.0`; arbitrary scripts or executable plugin definitions are
not accepted. A new view implementation must provide these contracts explicitly.

Use [the saved-view schema](../../schemas/documentation/view.schema.json) and
[the quantity-flow definition](../../fixtures/documentation-system/dataflow/quantity-flow.json).
Saved views reuse the bounded scenario selection and stable document identity.
Their `view.inputObjects` are existing `entity:<id>` domain IDs, distinct from
DTO, message, table, field and function representations. `view.services` explicitly
selects at most eight services for graph claims; source capture retains the existing
conservative scenario work boundary. Optional `contracts` reference exact observed
contract IDs, and `relatedProcesses` reuse current accepted component explanations.

```sh
./clew docs view modules --root <docs>
./clew docs view list --root <docs>
./clew docs view put --root <docs> --input <view.json> --expected-input-digest <digest>
./clew docs view show --root <docs> --id <view-id>
./clew docs view prepare --root <docs> --id <view-id>
```

The proposal uses `scenario:<view-id>` as its entrypoint, an empty `steps` array
and a typed `dataflow` graph. Its summary cites the returned `VIEW_DEFINITION`
reference plus source evidence. Read the supplied domain inputs and explicitly
expand mapper/body references and exact sources before citing them. Domain nodes
use the exact declared entity ID/title; other representations cite source from
their named service. Read/transform/write claims need local source evidence.
A transfer requires an explicit human/imported interaction with matching service
endpoints, exact source and stated uncertainty. Name-only correspondence remains
`candidate` / `UNKNOWN`; it cannot be relabeled as an established transformation.

The graph, node and edge claims are bound to the saved definition, view module,
source/contract scopes and accepted related component versions, and receive the
existing separate meaning review. A view is static interpretation, not a runtime
trace, universal taint analysis, or a proof of field/wire compatibility. Declared
transfers retain that limitation. Failed updates preserve prior graph content with
non-current status. Mapper changes propagate to dependent graphs, processes and
contract rows; independent scopes remain reusable where their recorded evidence
permits. New relevant declared interactions also invalidate a previously captured
view scope, including negative/candidate conclusions.

`view.human` owns annotations, tags, metadata and optional node layout positions.
Generated graphs are separate accepted content and do not rewrite this material.
`view put` preserves existing human fields by default; explicit maintainer edits
use `--human` and the current catalogue digest. Concurrent human edits invalidate
older work. Protected notes may associate with `view:<id>` and remain separate
from generated graph claims. The reader provides a source-linked SVG, accessible
node/edge text, Markdown and Mermaid exports using the same accepted graph and
per-operation freshness state. Human layout is optional; unmatched layout IDs
are retained in metadata rather than silently deleted.
# Portable capture and remote support

`clew docs evidence report --root <docs> --service <id> --output <new-directory>`
collects a versioned source-free diagnostic report. `docs evidence inspect --input
<directory>` reads it offline. To include the selected application index and
retained source, explicitly use `docs evidence capture` or `report --include-index`.
The recipient can page through source, observations, entrypoints and contracts
without the checkout or compiler. Package text is data and cannot configure tools.

Central import uses an operator-controlled expectation binding the registered
origin/project, configuration digest, exact source revision, trusted manifest
digest and increasing sequence. Use `docs evidence expect` with the current
catalogue input digest, then `docs evidence import`. Mismatches, missing parts and
older results cannot replace the selected artifact; a missing expected result or
captured producer failure remains a local gap. The original report records the
failure code and safe worker metadata. Imported explanations use the existing
author/reviewer and publication pipeline.

See the complete [portable evidence and support workflow](../../skills/codeclew/references/documentation-evidence.md)
for commands, expectation shape, retention, privacy boundaries and format limits.
The default report distinguishes declared Java/Kotlin versions from producer
admission and the Kotlin worker's Java 21 requirement. Exact application and
compiler inputs absent from the optional index remain an explicit diagnostic limit.
