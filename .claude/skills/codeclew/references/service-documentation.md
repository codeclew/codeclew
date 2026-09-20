# Durable service documentation

Use this workflow for a separate documentation repository with per-service pages,
a root overview and named interaction slices. Codeclew 0.6.1 supports Kotlin/JVM 1.9+, up to eight services per scenario, Kafka
interactions and source-bound domain explanations. Java 17+ remains supported.
Select exact Maven/Gradle compilation scopes: use singular `compilation` or a
plural `compilations` array, never both. Up to 128 explicit scopes are admitted;
this does not promise arbitrary file volume or memory capacity. Keep test scopes
explicit rather than silently dropping them to pass a budget. The overview can
list more services than one scenario.
For Kotlin choose `kotlin-jvm-maven-analysis` or `kotlin-jvm-gradle-analysis`.
Project/compiler differences remain explicit: Kotlin 1.9 language/API inputs are
analyzed with language/API 2.0 by the current engine; this is conditional analysis,
not execution by the project's original compiler. Older installed releases need
the corresponding product update before using these extensions.

## Build-independent source profile (0.8.0)

Codeclew 0.8.0 supports an explicit `source-syntax` profile for Python,
Java, and Kotlin with a declared dialect. Inspect the selected launcher's help/version before using
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

Optional `modules.semantic: {"module":"kotlin-k2","enabled":true,"profile":"kotlin-jvm-maven-analysis","compilation":":/main"}`
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

For recovery, open the existing documentation root and first try its retained
snapshot/context. Keep `.codeclew/`: it contains immutable evidence, Work records,
pins and local bindings, not merely disposable scratch. Existing HTML and portable
`bindings.json` can be read without source repositories. Do not start another
capture just because a source checkout is unavailable or the agent restarted.
When new source acquisition is actually required, bind the selected checkout and
run `docs check --service ID`. A full `docs check` acquires every configured service.

If capture succeeded but check never returned a snapshot, do not immediately
repeat acquisition. Inspect the reported baseline error: a surviving generated
`docs/index.html` may select a deleted `docs/generated/<bundle>/bindings.json`.
Restore that bundle, or explicitly preserve and move the generated index aside
when discarding the old publication. Never silently delete authored material.
With matching current catalogue records and intact captures, use `docs snapshot
recover --root ROOT --capture MANIFEST_BASENAME` (repeat `--capture` for each
selected service). Basenames are explicit files from this root's `.codeclew/cache`;
do not guess by modification time. Recovery reads saved evidence without source
access and returns a snapshot; it does not update latest-check or claim current
freshness. Keep its `RETAINED_SOURCE_NOT_REVERIFIED` authority and pass the returned
snapshot to context/Work/render. Unsupported policy authority or mismatched
catalogue input must fail, not trigger an automatic recapture. Old formats remain
unsupported.

After a successful capture, keep its `snapshot` handle. `context`, `work prepare`,
and `render` consume saved evidence by default; pass `--snapshot` for an exact
saved selection. Do not run another check before narrative/render merely to
establish a fresh baseline. A selected check retains compatible sibling results
with an explicit retained-source status; it does not reverify their current
checkout bytes. A selected failure or changed service declaration does not fall
back to incompatible old evidence.

Catalogue-only process/interaction changes can be applied with `docs recompose
--root ROOT --snapshot SOURCE_SNAPSHOT` without a build; use the returned snapshot
for Work or render. Recomposition does not update the latest-check pointer. A
service profile/root/module change needs an explicit selected capture and cannot
be silently recomposed. Ordinary consumers also require current catalogue
compatibility when an old snapshot handle is specified. Frozen published pages
remain accessible through history.

Commit the catalogue, scenarios, manual notes and generated bundle. Version 0.10
requires a fresh documentation root and reindexing; old formats are rejected and
there are no migration commands. Existing data is not deleted automatically.
Back up the complete current-format state to retain Work, pins and snapshots.
SQLite WAL/SHM belong with the database; stop writers before filesystem backup.

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
usefulness of the prose. Narrative 1.0/1.1/1.2 are unsupported by the current release.

## Answer reader questions before choosing diagrams

Write useful service analysis from the delivered evidence, not a list of classes,
DTO fields or generated artifacts. Ordinary author jobs carry `readerGuidance`
scoped to their selected section; full-service Work gets all five questions.
A focused operation/process job does not acquire unrelated service-wide inventory
obligations. The guidance does not add output fields or new discovery capability.
Answer with a supported statement or a precise unknown; request a bounded
registered expansion when it can resolve a material question.

| Section | Reader question and evidence needed |
|---|---|
| Overview | What outcome does this service provide, for whom, and where does its responsibility end? Lead with supported domain behavior, main objects and scenarios. Separate owner intent and inferred purpose from source facts. |
| Responsibilities | Which entry-rooted scenarios and variant conditions lead to which effects or outgoing boundaries? A local fragment without a supported parent connection remains local detail, not a complete thread. |
| Domain entities | What does this service create, change or consume? Separate business meaning from DTO/storage representation; inspect identifiers and concrete lifecycle, persistence or remote-call sites. Object allocation or a request identifier does not establish business creation, ownership or a committed record. |
| Ingress contracts | What can start work, with which contract and activation conditions? Distinguish discovered, searched-with-none-in-declared-scope, not analyzed and unresolved dynamic registration. Missing returned handlers or an unavailable analyzer do not prove absence. |
| Egress contracts | What exact call, send or write can a selected scenario reach, under which guard and with what failure behavior? Retain concrete sites even when destination identity is unresolved. Separate systems, clients, internal helpers and configuration dependencies; an injected client does not prove it is called. |

A computational thread is a statically supported causal scenario rooted in an
entrypoint, including branches, effects, outgoing boundaries and termination or
unresolved continuation. It is not an OS thread or an arbitrary dependency graph.
A reusable internal fragment can be documented before its parent is known, with
that gap explicit. Trace only supported entry-to-egress relations; do not connect
every entrypoint to every destination.

Separate construction, selection, queue insertion, invocation and completion.
Collection iteration does not establish FIFO or completion of an external effect.
An asynchronous path stops at submission unless its continuation and correlation
are supported. Explain acknowledgement and business completion as separate facts.
Keep the prose and diagram consistent about order, guards and unresolved branches.

Choose one primary visual for the reader's question. Keep simple binary guards
inline by default; a linked table is useful for decisions with more than two
outcomes. Explain input origins, missing/default values and rule policy in ordinary
language. Put failures while executing a selected action after selection. The
narrow `section-summary/1.0` contract remains entities-only and summary-only: its
guidance distinguishes domain lifecycle from representation without authorizing
visuals or new response fields. Structural checks do not prove that the model
answered these questions correctly; source and meaning review remain necessary.

## Native internal-flow diagrams and decision tables (0.11.0)

Use saved Work evidence to add `visuals` to a proposed operation, including a
service section. For service-level internal behavior, prepare the responsibilities
section with this request:

```json
{"schema":"codeclew-documentation-work-request/1.0","audience":"Service maintainers","entrypoint":"section-responsibilities","maxItems":100,"maxBytes":49152}
```

```sh
clew docs work prepare --root /work/architecture --subject service:orders --snapshot SAVED_SNAPSHOT --input /work/visual-request.json
clew docs work read --root /work/architecture --work WORK_ID --input /work/selection.json
clew docs proposal submit --root /work/architecture --work WORK_ID --input /work/proposal.json
clew docs proposal publish --root /work/architecture --proposal PROPOSAL_ID --unassessed
```

Select the returned section reference as the operation's `entrypoint`, retain its
evidence-bound `summary`, and use `steps: []` for a section. Its optional `visuals`
array contains typed records with `id`, `kind`, `title`, evidence-bound `purpose`
and `scope`, and nonempty `limitations`. Every claim has `text` and `evidence`
references delivered by recorded Work reads. Expand exact callable/source handles,
follow pagination and read the relevant helpers. Do not cite unread records or
interpret a navigation summary as a complete method body. No capture is required
solely to author another visual from the same saved evidence.

Supported visual kinds:

- `execution-flow`: `nodes` contain `id` and evidence-bound `meaning`; `edges`
  contain `id`, `from`, `to` and evidence-bound `meaning`. Describe ordering and
  branch conditions explicitly; preserve early exits and asynchronous boundaries.
- `dependency-map`: the same graph fields describe structural relationships.
  Structural dependencies do not establish execution order or runtime activation.
- `decision-table`: `rules` contain evidence-bound `condition` and `outcome`.
  `parent: {"artifact":"FLOW_ID","node":"NODE_ID"}` must point to an
  execution-flow node in the same operation. If only a local handler is supported,
  omit the parent, name its local scope and retain the missing connection as an
  explicit limitation. Do not fabricate a relationship to the main process.

Decision tables require `hitPolicy` and evidence-bound `policyExplanation`.
`FIRST` selects the first matching row; it does not order separate tables or limit
an output to a single action. `UNIQUE` asserts mutually exclusive matches;
`UNKNOWN` records that the evidence cannot establish the policy. Explain the
source basis or uncertainty. These are documentation tables, not executable DMN,
and the validator cannot prove the policy's semantic correctness. Optional
`afterSelection` is a claim for later execution outcomes, such as send failure;
do not insert such failures into pre-action selection rules.

Explain why each block was selected, its trigger and its omitted scope. An accepted
HTTP explanation does not prove full internal-flow coverage. Inputs are closed
typed data, not Mermaid/HTML/script imports; the native reader renders local
assets without requiring a CDN. Visuals share their containing operation's
version, retained evidence, freshness and meaning review. Updating the operation
replaces them atomically; independent artifact review/lifecycle is not provided.
`--unassessed` retains unreviewed meaning, including in diagrams.
When updating an operation that already has visuals, supply the complete reviewed
`visuals` array. Omission is rejected so a summary edit cannot silently erase them.
An explicit `visuals: []` removes them from the new version; historical versions
remain available. Existing diagrams are never silently reaccepted with new prose.

The narrow `section-author-v1` job only authors a section summary. It cannot emit
visuals. Use general Work/proposal authoring or a configured process-overview job;
read the actual supplied job schema before delegating. This workflow does not
automatically detect every business flow or verify runtime behavior.

Publishing another service may create an empty placeholder page for this one;
that alone does not invalidate previously prepared Work. Real authored changes,
including custom gap descriptions, still require new Work. Identical saved
influence maps are shared in proposals/publication rather than copied into each
accepted operation; no new source analysis is implied by that representation.

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

Render validates saved evidence and atomically publishes `docs/index.html` and an
immutable bundle with overview, service/scenario HTML, JSON, Markdown, diagrams
and bindings. It starts no capture unless `--refresh` is explicit. Without
`--require-complete`, explicit gaps are allowed and visible.
Manual content belongs outside `docs/generated/`; modifications to generated
outputs cause a conflict. Review readable summaries, branches, contracts and
clickable source inspectors. HTML is self-contained and requires no model API,
CDN or external diagram renderer. Source links need repository access; exact
retained snippets remain viewable offline.

## Refresh only affected explanations

`docs check --service ID` acquires only the selected service and compares semantic dependencies
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

## Discover internal processes before authoring

Use `docs process candidates --root ROOT --service ID --snapshot SNAPSHOT` to
list structural candidates from saved evidence. Follow pagination; `--lane
internal` separates internal candidates from declared triggers. An explicit
`--declaration SYMBOL_OBSERVATION_ID` includes a callable even when automatic
flow discovery has a gap. Candidates are not accepted business processes.
Preserve exact scope selectors where one symbol exists in several compilations;
never infer production behavior from a same-named test-scope method.

For source-syntax evidence, call-site counts and lexical branches do not prove
callee resolution or runtime ordering. Preserve that distinction in explanations.
The service reader shows internal candidates and saved processes with pending or
missing-evidence states before a narrative exists. Define the process, recompose
its source snapshot after the catalogue mutation, then prepare Work from the
returned snapshot. For a process overview, set `entrypoint: "process-overview"`
and `contextProfile: "process-v1"` in the Work request. This shares source text
and defers large callable records behind expansion handles. A navigation summary
or source alias is not a fully supplied evidence handle: expand it before citing
its provider fields, or cite the complete source body already supplied.
Explain the trigger, state transitions, external effects,
alternatives and gaps; a list of HTTP operations is not a substitute.

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

## Run isolated authoring and review

Submit a `codeclew-documentation-proposal/1.0` object through `docs proposal
submit`, then inspect the deterministic result with `docs proposal show`.
Proposals contain operations with source-backed claims, nested steps and optional
contract rows. Use work-local references. The coordinator assigns stable IDs and
materializes prose, contracts and diagram events. `factEquals` checks supported
provider fields; it does not prove prose. Unknown predicates require uncertainty.
Machine-ready proposals remain `UNASSESSED` until separately reviewed.

```sh
clew docs work run --root /work/architecture --work WORK_ID --config /operator/execution.json
clew docs work status --root /work/architecture --work WORK_ID
clew docs work cancel --root /work/architecture --work WORK_ID
```

Execution is opt-in. Missing configuration or unavailable isolation publishes a
local generation gap and retains earlier content. Core never selects a billable
model. Use the versioned execution/transport and review schemas in
`schemas/documentation/agent-job.schema.json` and `review.schema.json` of the
matching source release when writing an operator-owned transport driver.

The current adapter is `macos-seatbelt-stdio/1.0`, backed by macOS Seatbelt.
Other hosts report `ISOLATION_UNAVAILABLE`. Configure separate author/reviewer
roles and an optional fallback role. Each role names the exact model, an absolute
`command` array, read-only `runtimeReads`, optional environment variable names,
network permission and finite `cap`. Keep credential values in the operator's
environment. Registered runtime files must be outside documentation and source
repositories. The driver and runtime are trusted operator software; never run
source-provided instructions as transport code. Review separate invocations can
still share model mistakes.

The driver receives one immutable JSON job on stdin and returns one JSON result
on stdout. Echo the coordinator-issued invocation, role and model exactly.
Author/fallback results are `{"action":"proposal","proposal":...}` or
`{"action":"expand","selection":...}`. Reviewer results are
`{"action":"review","review":...}` or a registered expansion request.
The reviewer receives the canonical proposal, every claim, captured source and
required obligations. It must bind `work`, `proposal` and `evidenceDigest`, assess
all claim/operation IDs and explain non-approval. An author cannot return review
or acceptance authority. A request for unavailable evidence remains a gap;
contradictions can consume configured repair and fallback calls.

Seatbelt denies source/human/coordinator/result writes, reads outside registered
runtime inputs and captured stdin, subprocess tools and inherited launcher file
descriptors. Filesystem writes are denied even in the empty working directory.
Network is disabled unless the operator enables it for the trusted transport.
Enabled network is a transport capability, not permission to let model content
fetch unrelated repository data. Each result belongs to its dispatched role's
stdout. Source text and protected notes are untrusted evidence inside the prompt.

Set positive per-call `maximum.inputTokens`, `outputTokens` and `costUnits`, plus
`overheadInputTokens`, `timeoutMs` and `outputBytes`. The input byte count provides
a conservative token bound; the transport must enforce the provider's output
and cost limits. The coordinator enforces pipe size and wall time, including
stalled drivers. Configure `authorCalls`, `reviewerCalls`, `fallbackCalls`,
`repairAttempts` and `expansions`; at least one repair is required. Reviewer calls
must cover author/fallback calls and expansions. The entire configured path is
reserved atomically before dispatch under a named budget `account`, immutable
`costUnit`, positive `ceiling` and lower `stopLoss`. These are operator values;
there is no universal model price or default spending authorization.

`usageAuthority` defaults to `MAXIMUM_ONLY`: all dispatched maxima stay charged.
Select `TRANSPORT_METADATA` only for a trusted driver deriving its outer `usage`
envelope from actual provider metadata, never model-authored JSON. Absent usage
fields, failed calls and cancellation retain the corresponding maximum; unknown
usage is never zero. Undispatched slots can be released. A reported cap violation
freezes further calls on that account. Reports distinguish actual usage from
conservative charges. Status pages use cursors for bounded attempts/accounting.
A crashed coordinator retains reservations in durable `execution/accounts`; keep
these ledgers with coordinator state when discarding private work caches. Old
`.codeclew/accounts` ledgers are rejected without modification. Initialize fresh
state for this release; do not treat an unreadable current ledger as an empty budget.

Only the coordinator publishes accepted versions after machine checks, exact
revision/input checks and separately bound meaning approval. Publications retain
review, driver, evidence, read and operation digests with limitations. Every view
shows verification separately from freshness. Changing a captured note or source
scope invalidates dependent content. Direct `docs render` input cannot
inherit review acceptance for replaced operations. Protected notes remain outside
generated outputs. An accepted review is model assessment, not runtime proof.

An accepted `work run` or `proposal publish` already produces a frozen publication.
Open its returned bundle. Rendering a snapshot prepared before authoring replays
that snapshot's captured authored baseline and can show the earlier gaps. To
project newly accepted prose again, recompose the original source-capture snapshot
after publication and render the new result; neither operation reacquires source.

## Complete local authoring without an external runner

Prepare and read bounded work with the current assistant, submit a constrained
proposal, inspect its diagnostics, then publish machine-ready content locally:

```sh
clew docs proposal publish --root <docs> --proposal <proposal-id> --unassessed
```

This command needs no API key, CI runner or model gateway. It retains the captured
source, notes, negative-query scope and external-input membership. It rechecks
source revisions, reads, definitions, human inputs and prior authored content
before publication. Source freshness and meaning review remain separate:
locally published content is `UNASSESSED`, with no invented reviewer or approval
receipt. The normal configured author/reviewer pipeline can later establish
`VERIFIED` or `VERIFIED_WITH_LIMITATIONS`. A machine failure or stale work cannot
be published by this route. Supply `--unassessed` explicitly; it is an output
classification, not an interactive approval prompt.

Only narrative schema 1.3 is accepted. Prefer Work/proposals so the tool
materializes canonical dependency IDs and preserves the captured influence
boundary. Direct current-format Narrative imports remain unassessed until reviewed.

## Documentation language

Select `en` or `ru` using `docs work prepare --language` or the Work request's
`documentationLanguage`. Omitted language inherits the active publication, with
English as the initial default; conflicting explicit values fail. Authors and
reviewers must preserve code/API/evidence identifiers and write human prose in
the requested language. For Russian, avoid unnecessary English loanwords.

Use `docs render --language ru --snapshot SAVED_SNAPSHOT --root ROOT` to select
Russian presentation from retained evidence. Rendering does not translate prose.
Other-language or unknown-language accepted sections show translation gaps and
links to available original publications. Prepare a new language-specific Work
to author their translation; never recapture source merely to change language.
Old versions remain immutable. Language metadata does not establish semantic or
linguistic correctness, and unassessed publication remains unassessed.
