# Durable service documentation

Use this workflow for a separate documentation repository with per-service pages,
a root overview and named interaction slices. Codeclew 0.6.1 supports Kotlin/JVM 1.9+, up to eight services per scenario, Kafka
interactions and source-bound domain explanations. Java 17+ remains supported.
Use one Maven/Gradle compilation per service. The overview can list more services.
For Kotlin choose `kotlin-jvm-maven-analysis` or `kotlin-jvm-gradle-analysis`.
Project/compiler differences remain explicit: Kotlin 1.9 language/API inputs are
analyzed with language/API 2.0 by the current engine; this is conditional analysis,
not execution by the project's original compiler. Older installed releases need
the corresponding product update before using these extensions.

## Build-independent source profile (development source)

The development source supports an explicit `source-syntax` profile for Python,
Java, and Kotlin 1.9. Inspect the selected launcher's help/version before using
new flags on an installed release. This profile selects syntax authority
explicitly; it does not substitute syntax evidence after compiler admission fails.
Set `source.roots` and `source.dialect`; `compilation` is unnecessary. Include
configuration and helper files in the roots, or record those omitted inputs as
limitations. Only the selected committed revision is read. Dirty files are not
included. The installed binary and its pinned grammars are prerequisites, but
project dependencies, imports, build scripts, and K2 are not invoked.

```json
{"schema":"codeclew-documentation-service/1.0","id":"orders","title":"Orders","repositoryId":"orders","repository":"https://example.invalid/orders","language":"kotlin","profile":"source-syntax","targetRef":"main","source":{"roots":["src","pom.xml"],"dialect":"1.9"}}
```

`SOURCE_MATCH` means a unique lexical declaration. Preserve `SYNTAX`,
`ORDER_LEXICAL_ONLY`, unresolved targets, parse errors, and declared dialect
boundaries in explanations. Every source-based fragment conservatively watches
its involved service scope, including helpers, configuration, membership changes,
and empty catalogues. Reads outside registered scopes require an expanded scope
or an explicit incomplete-read limitation. Citations alone do not track them.

Optional `source.semantic: {"profile":"kotlin-jvm-maven-analysis","compilation":":/main"}`
requests the existing compiler provider. Provider failure keeps source evidence
readable and invalidates retained semantic dependencies. Enrichment attaches
only uniquely mapped equal-revision file/range facts; it does not convert syntax
calls to resolved edges. Kotlin language/analyzer differences remain visible.

Use `docs context --format compact` to remove duplicate event payloads and
repeated nested snippets. `SOURCE_ALIAS.coveredBy` points to retained text; follow
pagination and preserve all authority/coverage records. A known method can be
selected directly with `--symbol example.Reservations.reserve`; overloads require
an exact returned identity. `--source ID` and `--dependency ID` provide focused
raw drill-down within a selected service. `docs changes --root ...` rebuilds the
check and returns old claims, invalidation reasons, before/after sources, and
supporting context references. `--fragment ID` narrows this review package.
It never publishes or silently marks a claim reviewed.

## Start or recover

Resolve the installed `clew` launcher once. `clew docs` owns source admission,
compiler capture and cleanup; do not open a second navigation/session workflow.
If admission fails, retain the reported gap and next action. Never silently
substitute raw source analysis for compiler-backed evidence.

```sh
clew docs init --root /work/architecture --title 'System architecture'
clew docs service list --root /work/architecture
clew docs service add --root /work/architecture --input /work/service.json --expected-input-digest RETURNED_INPUT_DIGEST
clew docs bind --root /work/architecture --service orders --repo /work/orders
clew docs interaction put --root /work/architecture --input /work/interaction.json --expected-input-digest CURRENT_INPUT_DIGEST
clew docs check --root /work/architecture
clew docs context --root /work/architecture --service orders --limit 100
```

`init` writes editable examples under `examples/`; adapt their repository,
selector, profile and compilation before registering them. Set the repository
identity to its credential-free Git remote, and bind an existing checkout with
the selected target ref at HEAD. Keep the documentation root separate from every
source checkout. Commands do not change application source.

For cold recovery, read `codeclew-docs.yaml`, `catalog/services/*.json`,
`catalog/interactions/*.json` and `scenarios/*.yaml`, bind the relocated checkouts,
then run `docs check`. Commit the catalogue, scenarios, manual notes and generated
bundle to the documentation repository. `.codeclew/` holds ignored local paths
and disposable compiler cache; it is not needed for recovery. Published
`docs/generated/<bundle>/bindings.json` retains narratives and their dependencies.
`docs context` returns retained operations as authoring input, explicitly without
re-verifying their interpretation.

Service context without `--entrypoint` is the catalogue. Follow every
`nextCursor` with the same selection and `--cursor`, then request one exact
returned entrypoint ID at a time:

```sh
clew docs context --root /work/architecture --service orders --entrypoint RETURNED_ID --limit 100
clew docs context --root /work/architecture --scenario checkout --limit 100
```

Follow all pages needed for the selected operation. `omitted` means an item
exceeds the stdout budget, not that it is absent. Narrow the operation or report
an actionable gap. Context normally reuses the latest check; `--refresh` rebuilds
source evidence. Retained context is not a claim about current checkout bytes.

To inspect a DTO or message type named by a handler, request its exact compiler
identity or fully qualified declaration name in the same documentation workflow:

```sh
clew docs context --root /work/architecture --service orders --symbol example.OrderRequest --limit 100
```

Repeat `--symbol` for up to eight known declarations. This selects retained
declaration source; it does not infer runtime serialization or wire compatibility.

## Declare and explain

Interactions record engineer assertions. `origin` is `human`, `imported`, or
`agent-proposal`; proposals do not establish a traversable cross-service edge.
Use `interaction candidates --input ...` to inspect exact selectors before
saving an ambiguous declaration. Preserve origin, rationale, applicability,
stable IDs, and incomplete selectors rather than inventing missing identities.
`--expected-input-digest` on add/put prevents lost updates when editing existing
records; `interaction remove` requires it and refuses referenced declarations.
Scenario YAML remains manually editable and selects explicit interaction IDs.

A matching route is not runtime proof. Keep declaration origin, resolved caller
and receiver, call site, HTTP method/path, destination configuration key,
contract status, environment and runtime as separate certainty axes. The first
HTTP adapter recognizes resolved Spring RestTemplate literal method/path calls
and `@Value` destination keys. Unknown clients, dynamic routes, external calls,
unsupported control flow and runtime configuration remain explicit boundaries.

For a Kafka link use `transport: {"kind":"kafka","topic":"stock-import"}` and
an explicit compiler-returned publisher `callSite.target` when the topic is
indirect. Caller/receiver topic checks stay separate from the declaration. Literal
Spring Kafka topics can be checked; properties and custom publishers can remain
unresolved. A Kafka reply is a separate event interaction, never an HTTP-style
return. Outbox enqueue and a later scheduled publisher are separate entrypoints:
do not invent a call between them. The eight-service limit counts distinct
services, not individual clients or database actors; a diagram allows up to 24
participants so eight services can still show external actors.

Write the explanation from returned source, then submit a closed JSON Narrative
using [the authoring example](authoring-example.md). Every discovered entrypoint
needs an operation with its contract or an explicit actionable gap. For full-service
requests, do not silently stop after one representative endpoint. Include the
supported guards, alternate outcomes, loops, failures and state changes. Branch
markers must balance; every selected source condition and return needs a bound
corresponding event. A count check cannot establish semantic fidelity: read the
predicate and outcome, and keep their actual nesting/order in the narrative.
Do not convert unsupported branches into a linear happy path.

Use narrative schema `codeclew-documentation-narrative/1.3`. Write for a developer
or analyst who needs to use the service: what starts the operation, what data it
accepts, what changes, what comes back, and which failures need handling. Keep
`summary` to one or two sentences. The default `explanation` should usually fit
in three to six short paragraphs, organized by business decisions rather than
one paragraph per method. A small operation may need only one paragraph. Do not
inflate prose to match compiler traversal depth or repeat identical explanations
for each callback. Keep material rejection conditions, partial success, retries,
asynchrony and idempotency visible in this short overview.

Keep the complete bound branch structure in `events` as evidence. Author a
separate `overviewDiagram` for the reader: usually 4–9 nodes, never more than 12
nodes or 20 connections. Show the trigger, meaningful actions, material decisions
and outcomes. Do not draw every call, callback, loop iteration or return, and do
not use a collapsed giant diagram as a substitute for a readable diagram. If the
scenario needs more nodes, split it into named subscenarios before rendering.

Each node has `id`, brief `text` (up to 84 characters), `participant` (an existing
participant ID), `column` (0..3), `row` (0..2), and 1..8 retained non-end `eventIds`.
Use unique grid positions. Each edge has `id`, `from`, `to`, brief `text`, and
1..8 retained `eventIds`. Label the conditions on alternate paths. Cross-service
edges must retain the existing declared transition with matching participants.
Node and edge evidence participates in freshness checks. The diagram describes
source-interpreted business flow, not a runtime trace. Do not infer chronological
ordering across an asynchronous boundary without evidence.

Mark implementation-only explanation paragraphs with `detail: true`. Detailed
prose and exact source remain available on demand; the full event traversal is
retained as structured evidence rather than rendered as an unbounded diagram.
HTML, Markdown and Mermaid exports all use the same bounded overview. Review the
actual rendered diagram at a normal desktop size; inspect labels, crossings and
conditional paths, not only the node count. Preserve rejection, retries, partial
success and asynchronous boundaries while shortening the visual explanation.

Each paragraph has `id`, `text`, `eventIds`, `dependencyIds` and `sourceIds`.
Every non-`end` diagram event must be covered by an overview or detail paragraph,
and paragraphs retain the evidence of every referenced event. One paragraph can
cover many steps. The validator checks references and coverage, not the truth or
usefulness of the prose. Narrative 1.0/1.1/1.2 remain readable for existing bundles.

## Reader comprehension and source checks

Each overview must answer concrete decisions without requiring a source-code
inspection: what identifies a duplicate, what wins an equal-time conflict,
which state changes on each outcome, when an acknowledgement is absent, and
what asynchronous completion actually means. Use a compact result or state table
in the relevant interface contract when those outcomes are easier to compare.
Keep contracts complete, including nested payloads, but disclose payload details
only when the reader selects them. Do not repeat prose for multiple callbacks;
merge its evidence IDs while respecting each record's bounds.

Trace calculated outputs through the actual constructor, mapper and query used
by the selected path. A formula's existence does not prove that its inputs were
loaded. Distinguish constructor defaults from persisted values, current-row
comparison from historical deduplication, an accepted update from a changed
numeric value, and initiating a send from awaiting its successful completion.
Put a limitation that changes an output's meaning next to that output.

For an independently reviewed documentation request, give a fresh reader only
the generated pages and reader guide. Ask it to explain selected services and
answer concrete normal, duplicate, stale, rejection, retry and partial-success
cases. Require page references, missing information and excess-detail findings.
Correct unsupported claims against retained source, then repeat with a fresh
reader after material edits until no blocking comprehension finding remains.
Report the reviewed service/question scope; a pass does not prove runtime
behavior or coverage of functions explicitly outside that scope.

The reader's primary source action opens all snippets bound to the paragraph;
it does not require following a long list of opaque step numbers. `eventIds`
remain necessary for machine-checked coverage and source inspection.

## Interface contracts

Do not equate an empty OpenAPI tab with the absence of a contract. For each
in-scope boundary, document the actual input and output from a published schema
or from retained handler, DTO, serializer and publisher source. Use
`interfaceContracts` for source-derived descriptions, with kind `http`, `kafka`
or `payload`; every row keeps exact dependency/source bindings. They are agent
interpretations and remain distinct from declared OpenAPI evidence.

- HTTP: method/path, path/query/header parameters, body fields and nested types,
  observed validation, successful status/body and material error/status cases.
  Keep authentication declarations separate from proven runtime enforcement.
- Kafka: topic or topic template, producer/consumer, payload and nested fields,
  message key/headers, time and correlation semantics, acknowledgement, retry,
  duplicate/stale handling and outgoing events. An application ACK event is
  separate from the consumer's offset acknowledgement.
- Payloads: type, nullability, source defaults, enums and explicit constraints.
  Kotlin non-null types alone do not prove that a JSON field is required; record
  constructor and Jackson behavior separately, or leave wire requiredness
  unresolved. Distinguish an incoming total quantity from an outgoing available
  quantity and preserve units when the source establishes them.

Before rendering, compare the selected ingress/egress list against the contract
cards. Include each necessary boundary or name the exact missing contract fact;
do not claim complete contracts from counts alone. Preserve external boundaries
without expanding the task to unrelated services. Avoid repeating a whole
payload schema in the overview; put it in an expandable contract card. A useful
page lets the reader find inputs, outcomes and failure handling without opening
the implementation diagram.

Kotlin `DEFERRED` callback blocks must stay inside an `opt` group. Their source
can be explained, but invocation, count and scheduling are not established by
merely passing a lambda. `TRY` becomes `alt`; catches become `else`; preserve
`FINALLY`, `BREAK` and `CONTINUE` as explicit notes. Keep callbacks and outbox
boundaries visible instead of presenting a synchronous happy path.

Source/dependency IDs must come from the current context. Every arrow/node keeps
its own source binding; cross-service arrows also name the declared interaction.
Local calls are compiler observations; cross-service transitions remain declared;
narratives are agent-inferred. Versioned OpenAPI 3.0 files supply contract facts,
including local references and constraints, separately from code behavior.

```sh
clew docs render --root /work/architecture --input /work/orders-narrative.json --input /work/inventory-narrative.json --input /work/checkout-narrative.json --require-complete
```

Render rechecks source before atomically publishing `docs/index.html` and an
immutable bundle with overview, service/scenario HTML, JSON, Markdown, diagrams
and bindings. Without `--require-complete`, explicit gaps are allowed and visible.
Manual content belongs outside `docs/generated/`; modifications to generated
outputs cause a conflict. Review readable summaries, branches, contracts and
clickable source inspectors. HTML is self-contained and requires no model API,
CDN or external diagram renderer. Source links need repository access; exact
retained snippets remain viewable offline.

## Refresh only affected explanations

`docs check` rebuilds current source evidence and compares semantic dependencies
with the portable baseline. Exit 0 means CURRENT; 4 means PARTIALLY_STALE/STALE;
3 means UNRESOLVED, including a missing baseline or source binding. Other invalid
inputs and conflicts use the normal CLI error codes. Large reports have
`items` and `nextCursor`; retain the top-level freshness status on every page.

Read `affected`, `unaffected`, `linkChanges` and coverage/catalogue changes.
Line movement alone refreshes source links without requiring prose changes.
Changed methods, routes, contracts, callees or declarations identify dependent
fragments. Re-author affected operations against the new context digest, keeping
unaffected text and engineer declarations. Rendering refuses stale retained
narratives until reviewed replacements are supplied. A digest detects changes;
it does not prove an agent's explanation. Missing history/source stays
UNRESOLVED. No hosted LLM or embedded API key is part of this workflow.

## Prepare immutable author work

For recorded authoring, prepare one service or saved scenario with a closed
`codeclew-documentation-work-request/1.0` JSON object containing `audience`,
optional `entrypoint`, `maxItems` (1–100) and `maxBytes` (2048–49152):

```sh
clew docs work prepare --root /work/architecture --subject service:orders --input /work/request.json
clew docs work read --root /work/architecture --work WORK_ID --input /work/selection.json
clew docs work expand --root /work/architecture --work WORK_ID --input /work/selection.json
```

Use returned work-local references such as `e1`, `d3`, and `s2` in selection
`references`. Select one entrypoint or up to eight evidence references. Exact
`symbols` or a `query` with `kind` and optional `symbolContains` are alternative
selectors. `kind: "*"` queries all captured dependency kinds. Carry `nextCursor`
into the same selection to read further pages. Unknown references and cursors
from another work or selection are rejected. Read and expand have identical
recording semantics. Explicit `omitted` records identify evidence that cannot fit;
prepare work with a larger allowed byte budget or retain a missing-evidence gap.
Do not silently shorten a fact to make it fit.

The work preserves revisions, provider authority, conservative dependency scopes,
retained explanations and protected `notes/` inputs. Optional `externalInputs`
registers bounded UTF-8 files relative to the documentation root. Read these from
the captured package; citations to outside files do not register prompt reads.
Declare any other reads with `untrackedReads: true`; this is sticky for the work
and makes influence coverage incomplete. Recorded reads alone do not attest that
an external author was isolated. Empty query results and their enclosing source
scopes are recorded so later matching files can invalidate acceptance. A work
read never replaces its source evidence with a newer check. Follow owner/helper,
flow, type and configuration references where required; preserve unresolved
boundary obligations. No agent may promote syntax or imported claims to runtime
proof. Human input is retained verbatim with its separate authority.
