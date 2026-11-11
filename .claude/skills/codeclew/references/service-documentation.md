# Durable service documentation

Use [authored originals and checked human HTML](compose-original.md#frozen-source-to-reviewed-pages).
Reuse an available saved snapshot. This reference covers acquisition, evidence
reads, bounded answer JSON and recovery. There is one document publication route:
authored original/Markdown → compose/update/bind → finalize → packaged renderer.

## Authoring start route

Prepare bounded Work on the selected saved snapshot, read context and complete
source continuations, then author the requested pages. Describe each relevant
entry, interaction, contract, effect and failure. Source inventory and summary
sections do not establish that the requested operations are documented.

```sh
clew docs work prepare --root ROOT --subject service:SERVICE --snapshot SNAPSHOT --input REQUEST_JSON
clew docs work read --root ROOT --work WORK --input SELECTION_JSON
```

Use returned references from the same Work. Follow candidate bodies before calling
evidence unavailable. Continue through the [human-page commands](compose-original.md#original-pages-and-completion).
The [source-to-description example](authoring-example.md) explains this transition.

## Prepared SOURCE answers and maintenance

For a bounded question with an available snapshot or saved answer, start with
[the short prepared SOURCE route](../README.md). Model-backed Work Q&A retains
validated answer, packet and audit JSON; it does not generate document previews.
Use the human-page route when the user requests documentation.

## Build-independent source profile

The current explicit `source-syntax` profile supports Python, Java, Kotlin and
TypeScript with a declared dialect. Inspect the selected launcher's help/version
before using new flags on an installed release. This profile selects syntax authority
explicitly; it does not substitute syntax evidence after compiler admission fails.

Spring wildcard imports can yield `WILDCARD_IMPORT_CANDIDATE` names under the
bounded framework rules. These remain SOURCE candidates, never compiler-resolved
names or proof of bean registration. Explicit import conflicts and known local
shadows block qualification, including same-package types in other selected files.
An unmatched SOURCE contract route carries `SOURCE_ROUTE_ABSENCE_UNPROVEN`; do not
report it as proof of a missing implementation. Other unresolved names, dynamic
paths and unsupported registrations remain unknown. Compact Kotlin class members
can recover a redundant parser terminator only when an existing space or tab before
`}` preserves byte offsets. Without that whitespace, or for other parser errors,
capture remains explicitly partial. A top-level annotated expression that the pinned
Kotlin grammar cannot enumerate as a declaration also makes capture partial.
Original bytes are retained. Do not reformat
a retained fixture to hide a capture gap.

Set `source.roots` and `source.dialect`; omit compiler `compilations`. Include
configuration and helper files in the roots, or record those omitted inputs as
limitations. Committed files come only from the selected revision; ordinary
dirty and untracked files are not included. To include existing generated source,
register each exact path and descriptive `origin` in `source.generatedInputs`.
The collector reads those named existing files without running a generator or
scanning ignored directories. Generated inputs retain their bytes and digests
separately from the Git revision; they may include ignored or untracked files,
but cannot collide with committed paths. Limits are 64 files, 2 MiB per file and
32 MiB in total. Keep `EXPLICIT_GENERATED_SOURCE_TEXT` and the declared origin
visible; the origin is not proof of generator execution. The installed binary and
its pinned grammars are prerequisites, but project dependencies, imports, build scripts, and K2 are not invoked.

```json
{"schema":"codeclew-documentation-service/1.0","id":"orders","title":"Orders","repositoryId":"orders","repository":"https://example.invalid/orders","language":"kotlin","profile":"source-syntax","targetRef":"main","source":{"roots":["src","pom.xml"],"dialect":"1.9"}}
```

`SOURCE_MATCH` means a unique lexical declaration. Preserve `SYNTAX`,
`ORDER_LEXICAL_ONLY`, unresolved targets, parse errors, and declared dialect
boundaries in explanations. Every source-based fragment conservatively watches
its involved service scope, including helpers, configuration, membership changes,
and empty catalogues. Reads outside registered scopes require an expanded scope
or an explicit incomplete-read limitation. Citations alone do not track them.

For optional Kotlin compiler enrichment, add this complete `modules` object to
the existing full SOURCE service record (this example selects Maven):

```json
{
  "schema": "codeclew-documentation-modules/1.0",
  "semantic": {
    "module": "kotlin-k2",
    "enabled": true,
    "profile": "kotlin-jvm-maven-analysis",
    "compilation": ":/main"
  }
}
```

Keep the existing source roots and dialect. For Gradle, select its admitted
analysis profile instead. Read the current `inputDigest` with `docs service list`,
then submit the complete updated service JSON with `docs service add --input
<service.json> --expected-input-digest <inputDigest>`; both commands require
`--root <docs>`. The parent modules schema is required. No top-level compiler
array is needed for a SOURCE service. Provider failure keeps source evidence
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

For an already prepared SOURCE answer, use the
[short maintenance route](#prepared-source-answers-and-maintenance) above. The
setup sequence below is for missing registration/bindings or explicitly requested
acquisition.

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
the selected ref available locally. SOURCE reads that explicit committed ref
without requiring the checkout HEAD to match it. Compiler admission may require
the selected checkout at its target HEAD; follow the chosen profile rather than
applying that requirement to SOURCE. Keep the documentation root separate from every
source checkout. Commands do not change application source.

For recovery, open the existing documentation root and first try its retained
snapshot/context. Keep `.codeclew/`: it contains immutable evidence, Work records,
pins and local bindings, not merely disposable scratch. Existing HTML and portable
bindings can be read without source repositories. New 1.6 `bindings.json` files
are manifests: preserve their entire declared `docs/evidence/objects/` closure
alongside the selected generated bundle and pointer. Inspect all pages of
`clew docs history show --root ROOT --id PUBLICATION --kind bindings-closure`;
copying only the manifest is incomplete. Inline 1.4/1.5 reads remain supported;
older public readers cannot read 1.6. A portable publication does not replace
original protected-paragraph snapshots or Work state in `.codeclew/`.
Preserve complete historical exports and their declared closure. Do not repeat
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
snapshot to context and Work.

For a stopped export with an immutable snapshot but no matching current cache
manifest, select the exact captures embedded in that snapshot explicitly:

```sh
clew docs snapshot recover --root FRESH_ROOT --from-export STOPPED_EXPORT --source-snapshot SNAPSHOT --service orders --service billing
```

This mode imports only selected service definitions and their verified source
closures, without old scenarios, notes, Work or authored narratives. It preserves
the snapshot's capture/cacheability metadata and historical authority. It neither
recaptures sources nor updates latest-check. `--service` and `--capture` are
exclusive; the existing `--capture` mode still rejects a mismatched keyed file.
Do not rewrite a stale cache manifest to make it match. A missing or corrupt
immutable source object remains a blocking error.

Unsupported policy authority or mismatched catalogue input must fail, not trigger
an automatic recapture. Old formats remain unsupported.

After a successful capture, keep its `snapshot` handle. `context` and `work prepare`
consume saved evidence by default; pass `--snapshot` for an exact
saved selection. Do not run another check before authoring merely to
establish a fresh baseline. A selected check retains compatible sibling results
with an explicit retained-source status; it does not reverify their current
checkout bytes. A selected failure or changed service declaration does not fall
back to incompatible old evidence.

Catalogue-only process/interaction changes can be applied with `docs recompose
--root ROOT --snapshot SOURCE_SNAPSHOT` without a build; use the returned snapshot
for Work or context. Recomposition does not update the latest-check pointer. A
service profile/root/module change needs an explicit selected capture and cannot
be silently recomposed. Ordinary consumers also require current catalogue
compatibility when an old snapshot handle is specified. Frozen published pages
remain accessible through history.

To explicitly update the OpenAPI projection of full registered saved files,
use `docs refresh-contracts --root ROOT --snapshot SOURCE_SNAPSHOT --service ID`.
Repeat `--service` for up to eight captured services. The current bounded reader
consumes retained declaration bytes only; no checkout, analyzer, package restore
or network request runs. Missing or partial files fail. Limits are 128 registered
files and 8 MiB per selected service. Use the returned immutable snapshot for
context and Work. Source records, revisions, historical pages and latest
remain unchanged; source and runtime are not reverified. Use the original capture
as parent for every refresh, including after later catalogue edits. Ordinary
recomposition retains the original capture's contract projection.

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

Author the requested contract and process descriptions in originals, then use
[compose/bind/finalize](compose-original.md#original-pages-and-completion).
Keep each arrow's evidence and distinguish local compiler observations from
declared cross-service relationships and unobserved remote outcomes.


## Discover internal processes before authoring

Use `docs process candidates --root ROOT --service ID --snapshot SNAPSHOT` to
list structural candidates from saved evidence. Follow pagination; `--lane
internal` separates internal candidates from declared triggers. An explicit
`--declaration SYMBOL_OBSERVATION_ID` includes a callable even when automatic
flow discovery has a gap. Candidates are not accepted business processes.
Preserve exact scope selectors where one symbol exists in several compilations;
never infer production behavior from a same-named test-scope method.

Select the intended retained declarations explicitly. Define the process, recompose
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


## Find an approved answer before preparing new Work

Codeclew 0.13.17 includes `docs work find-answer`. Use the installed `clew`
launcher and check its version and help; older releases need an update before
using this command. Select an existing saved Check explicitly and supply the exact
subject and original Work request. Normalization applies the Work defaults; it
does not search for similar questions or choose an answer by recency.

For the supported native method case, the request uses `process-graph-v1`, an
exact compiler METHOD declaration, authoring 1.6 and source-data context:

```json
{
  "schema": "codeclew-documentation-work-request/1.0",
  "audience": "Service maintainers",
  "contextProfile": "process-graph-v1",
  "rootDeclaration": "EXACT_SCOPED_CALLABLE_SYMBOL_OBSERVATION_ID",
  "question": "How does this internal service operation decide what to do?",
  "authoringContract": "codeclew-operation-draft-authoring/1.6",
  "sourceDataContext": true,
  "maxItems": 100,
  "maxBytes": 49152
}
```

Use the same request that was approved, including its question, audience,
language and scope choices:

```sh
clew docs work find-answer --root /work/architecture --subject service:orders --snapshot COMPARED_SNAPSHOT --input /work/internal-operation-request-1.6.json
```

`FOUND` returns `selected.answer`, its original review, provenance, packet and
audit. Keep those historical citations and bindings unchanged. If status is
`SELECTION_REQUIRED`, inspect the applicable candidates and save one returned
`candidate.selection` object verbatim, then repeat the exact lookup:

```sh
clew docs work find-answer --root /work/architecture --subject service:orders --snapshot COMPARED_SNAPSHOT --input /work/internal-operation-request-1.6.json --select /work/answer-selection.json
```

A unique `CURRENT` approval is selected automatically. Multiple applicable
approvals require explicit selection; none is ranked by recency or preferred.
`NO_MATCH` means there is no exact historical approval; `NO_REUSABLE_MATCH` means the matching history
does not pass applicability. Inspect each candidate's `applicability` and reasons:
`CURRENT`, `STALE` and `UNKNOWN` are separate from its frozen historical approval.
`CURRENT` compares against the selected saved Check plus a read-only protected
notes membership check. It is not a new meaning review or verification of live
checkout bytes. Lookup makes no captures or model calls and creates no new Work,
job, review or publication records. Existing CAS and locking infrastructure may
still perform operational filesystem IO.

The initial reuse boundary requires absent protected notes, no actual author or
reviewer expansion, no repair, and no maintained, retained or admitted external
context. Portable evidence expectations and `EVIDENCE_PACKAGE` authority are
unsupported. The replay compares selected facts and source text, complete initial
selection membership, negative/ambiguous/frontier results, derived source-data
context and producer/selection/review policy. Valid receipt changes across
revisions can remain applicable when these semantic inputs are unchanged;
unrelated file changes alone need not invalidate the answer. Changed guards,
new matching callees, or newly present notes refuse reuse. Missing or corrupt
bindings are explicit errors, not an empty search. `answer-context` retains its
existing stricter comparison policy. When lookup cannot reuse an answer, acquire
a new Check separately if needed, then explicitly prepare, author and review
new Work; lookup never starts that work automatically.


## Draft one Java endpoint explanation

For a captured Java HTTP endpoint, select `endpoint-context-v3` in the request
before preparing Work. The profile is part of immutable Work identity, so an
existing Work with another profile cannot be switched in place. Prepare a new
Work from the same saved snapshot; this does not capture source again.

```json
{
  "schema": "codeclew-documentation-work-request/1.0",
  "audience": "Service maintainers",
  "entrypoint": "HTTP_ENTRYPOINT_ID",
  "contextProfile": "endpoint-context-v3",
  "maxItems": 100,
  "maxBytes": 49152
}
```

For grouped retained-context decisions, explicitly add
`"authoringContract": "codeclew-operation-draft-authoring/1.6"` to a new endpoint
request or `process-graph-v1` question. Use the separate
`operation-draft-execution-1.1.schema.json` configuration with schema
`codeclew-documentation-operation-draft-execution/1.1`, `author`, `budget` and
positive caller-selected `authorCalls`. Every decision is an answer action or
one expand action grouping native selections. The coordinator chunks native
read limits and drains content pages and source parts from the saved snapshot;
SYMBOL navigation is not citable until full records are delivered. The final
answer binds the updated packet digest. Keep unavailable evidence explicit.

Independent expanding review uses
`operation-draft-review-execution-1.1.schema.json`, schema
`codeclew-documentation-operation-draft-review-execution/1.1`, `reviewer`,
`budget` and positive `reviewerCalls`. Its separate context may expand without
changing the original author packet or answer. Both configurations reserve all
configured calls before execution. Repeating the same command recovers saved
work and never automatically replaces an uncertain invocation. After a terminal
unsuccessful author run, explicit `--new-run` admits a fresh finite attempt.
For an original terminal uncertain review without a saved result, explicit
`--retry-from-review FAILED_REVIEW_RUN` uses a new empty reviewer account and
the exact previously delivered evidence seed. Original accounting and author
bytes are preserved; no refund or provider cancellation is implied. Complete
interrupted accounting by replaying its original configuration first. A
completed result or failed retry child is not replaceable by this selector.
The older defaults and saved results retain their existing policies.


### Opt into typed model IDs with a compatible driver

Codeclew 0.13.17 includes both model-ID modes. Use the installed `clew`
launcher; older releases need an update before selecting a mode. For native
authoring 1.6 and its independent reviewer, set
`"modelRepresentation": "codeclew-model-ids/1.1"` on the selected `author` or
`reviewer` role only when its driver implements that carrier contract. Omit the
field for an existing driver; its canonical stdin format and configuration
serialization remain unchanged. The selected mode is part of driver admission
and configuration identity.

The host retains the unchanged canonical job and exact versioned carrier before
dispatch. The carrier contains `schema`, `canonicalJob` and `preparedModel`;
`schema` matches the selected and retained representation version.
The prepared form includes the model payload, output schema, scoped map and
digest bindings. Validate the canonical job and packet first, then use the
[public Rust API and serializer contract](https://github.com/codeclew/codeclew/blob/main/docs/operations/model-id-serializer.md)
and [executable serializer example](https://github.com/codeclew/codeclew/blob/main/crates/clew/examples/model_ids_serializer.rs).
`clew::documentation::model_ids::forward_model_input` invokes the canonical
validation callback before checking the frozen prepared projection. Forward only
its returned `payload` and `output_schema` to the model. Keep the canonical job,
host map and carrier metadata outside the prompt and strict output schema.
Do not recompute canonical packet digests from aliased content.

Version 1.1 keeps all native evidence presentation in model input. Before alias
projection, it checks each presentation against the native builder using the
canonical pages and source parts. It then omits only the duplicate raw `pages`
and `sourceParts` siblings in `packet.contextDelivery` and `reviewContext`.
Read complete evidence and retained-reference links from the presentation.
Canonical raw arrays, metadata, receipts, citations and host maps remain retained;
delivery metadata and bindings remain in model input. A missing presentation
keeps raw arrays; a mismatched or incomplete presentation refuses preparation.

Version 1.0 remains supported with its original raw-plus-presentation behavior,
map identities and carrier bytes. Recovery validates the saved version; changing
a role's mode cannot migrate an existing run. The public `prepare` API still
prepares version 1.0. Use `prepare_with_version` for explicit version 1.1
preparation, and validate a frozen carrier through `forward_model_input`.

Input admission and reservation still conservatively account for the complete
serialized carrier plus configured transport overhead. A smaller model payload
can require a larger explicit carrier cap and budget. Typed aliases remain
stable as grouped expansion appends to that Work/run/role/version map. Review
uses its own map and read delivery. Source text, semantic symbols, paths, native
evidence references and query selections retain their canonical values.

Return the existing outer Reply with canonical invocation, role and model; use
the prepared typed bindings in its inner answer or review. The host retains the
parsed raw JSON and any adapter failure before fallible Reply validation or alias
decoding. Unknown, wrong-domain or foreign-scope aliases fail closed. Aliases are
decoded before existing semantic answer/review checks, and successful saved
results remain canonical. Resume consumes retained output without another
dispatch, including failed decoding. Parseable JSON from a failed driver cannot
be promoted to success or perform an expansion; malformed JSON remains an
adapter failure. Byte-for-byte stdout retention is not claimed.

The [version 1.0 prepared-input assessment](https://github.com/codeclew/codeclew/blob/e729cd2e6a5f46957255b17ff0f9007a072a2d52/docs/product/validation/model-id-alias-feasibility.md#supported-native-candidate-2026-10-06)
measured two retained first-call jobs through the native API and example:
combined prompt and strict-schema reference text fell from 26,716 to 25,402
`o200k_base` tokens, a 4.9184% reduction including encoding guidance. This is
reference text counting, not measured provider usage or billing, answer quality,
or a general or grouped-expansion savings claim. No model calls were made for
that assessment.

The [version 1.1 compact-delivery assessment](https://github.com/codeclew/codeclew/blob/main/docs/plans/question-results-and-incremental-evidence.md#compact-role-evidence-representation-candidate-2026-10-06)
measured all four actual author and reviewer inputs from one generated grouped
fixture. Combined prompt and strict-schema `o200k_base` reference text counts
fell from 163,106 to 104,051 (36.2065%) including representation guidance.
Within that total, the two expanded calls fell from 113,714 to 69,559 (38.8299%).
The initial author input had no duplicate delivery to omit and increased by
18 reference tokens (0.2547%). This is a
bounded reference-count comparison, not general savings, provider billing or
answer-quality evidence; no model calls were made for the assessment.

For the default single-call path, use the author-only configuration schema
`schemas/documentation/operation-draft-execution.schema.json`.
It contains `schema`, one existing isolated `author` role, and a finite
`budget`; it has no reviewer, fallback or repair settings. The role shape is
the same one used by `agent-job.schema.json`.

```json
{
  "schema": "codeclew-documentation-operation-draft-execution/1.0",
  "author": {
    "adapter": "macos-seatbelt-stdio/1.0",
    "model": "OPERATOR_SELECTED_MODEL",
    "command": ["/absolute/path/to/driver"],
    "runtimeReads": ["/absolute/path/to/driver-runtime"],
    "cap": {
      "maximum": {"inputTokens": 120000, "outputTokens": 8000, "costUnits": 100},
      "overheadInputTokens": 0,
      "timeoutMs": 180000,
      "outputBytes": 32768
    }
  },
  "budget": {
    "account": "orders-operation-drafts",
    "costUnit": "OPERATOR_DEFINED",
    "ceiling": {"inputTokens": 160000, "outputTokens": 10000, "costUnits": 120},
    "stopLoss": {"inputTokens": 140000, "outputTokens": 9000, "costUnits": 110}
  }
}
```

Then prepare from the saved snapshot and run the explicit draft mode:

```sh
clew docs work prepare --root /work/architecture --subject service:orders --snapshot SAVED_SNAPSHOT --input /work/operation-request.json
clew docs work run --root /work/architecture --work WORK_ID --config /operator/operation-draft.json --draft
clew docs work status --root /work/architecture --work WORK_ID
```

The author receives only the compact operation packet, common language and
source-as-untrusted-data instructions, and
`schemas/documentation/operation-answer-1.2.schema.json`. New draft answers use
`codeclew-operation-answer/1.2`; saved 1.0 and 1.1 answers remain available for
immutable inspection. Version 1.2 adds required `glossaryRefs` on claims and steps.
Version 1.1 added shared `preparations` that describe evidence-supported input
transformations, checks, and failures; they remain in 1.2. Steps link to shared
explanations with `preparationRefs`; links do not assert execution or order. The
answer keeps explicit step `from` and `to` values, leaving missing endpoints unknown.
Codeclew validates the returned packet digest and citation labels, then saves
`answer.json`, `reader-packet.json` and `reader-packet-audit.json` under
`.codeclew/drafts/WORK_ID/RUN_ID`. Status is `DRAFT` / `UNREVIEWED`.
The command retains JSON and does not publish a document. One author call
is admitted. A saved answer is reused to restore missing draft files. An invalid
answer is retained without an author repair. A terminal `ANSWER_INVALID` result
may be revalidated only from its digest-bound saved author result after the
selected invocation identity, immutable input, checkpoint and report bindings
match; this makes no new driver dispatch, and an answer that still fails
validation remains invalid. A dispatch without a saved response is
marked uncertain with its maximum reservation retained; rerunning that Work does
not dispatch another call. For a latest terminal invalid answer, an operator
may run `docs work run --draft --repair-from-run RUN_ID` to make one explicit
repair call from the unchanged saved packet, previous answer, and fresh native
validator error. Repeating that command or plain `--draft` resumes the same
repair run; repairs do not chain. New operation Work persists the centrally selected
`authoringContract` value `codeclew-operation-draft-authoring/1.4`, covering the
answer schema, generic author policy, and endpoint field projection. For this
profile, `packet.fields` lists selected referenced owner FIELD declarations
with their exact type, modifiers, annotations, source tokens, and evidence;
`packet.constants` keeps its existing static-and-final subset. Initializer
tokens document declaration syntax, not runtime values or initialization
timing, and `final` does not establish deep immutability. Material changes to
the schema, policy, or packet require a new identity; prepare new Work from the
same saved snapshot to use it. The operator does not edit old Work records or
set an identity to reuse completed results. Existing 1.3 Work keeps its prior
packet and may replay a validated saved answer, including after an interrupted
output write. An explicit repair of a retained invalid answer is the only
additional 1.3 author-call path; ordinary initial generation and `--new-run`
remain blocked. This recipe covers the HTTP endpoint
profile. Use the next recipe for an internal callable whose behavior is not
entered through an HTTP endpoint.


## Draft one internal service process explanation

For an internal operation, choose the exact callable `SYMBOL` observation ID
from the selected service and saved snapshot. Persist the question with that
root before Work identity is calculated. The process profile has no
`entrypoint`; a non-null endpoint selector cannot be combined with it.

```json
{
  "schema": "codeclew-documentation-work-request/1.0",
  "audience": "Service maintainers",
  "contextProfile": "process-graph-v1",
  "rootDeclaration": "EXACT_SCOPED_CALLABLE_SYMBOL_OBSERVATION_ID",
  "question": "How does this internal service operation decide what to do?",
  "maxItems": 100,
  "maxBytes": 49152
}
```

Prepare from the saved snapshot that supplied the exact declaration:

```sh
clew docs work prepare --root /work/architecture --subject service:orders --snapshot SAVED_SNAPSHOT --input /work/internal-operation-request.json
clew docs work run --root /work/architecture --work WORK_ID --config /operator/operation-draft.json --draft
clew docs work status --root /work/architecture --work WORK_ID
```

Use the same author-only configuration shown in the endpoint recipe. The
author packet answers the persisted question from the exact root, retained
method sources, fields, types, provider callsites and explicitly marked source
context candidates. It contains no HTTP endpoint or trigger. A candidate can
identify useful helper source, but it does not establish that a call executed,
which runtime receiver or override was selected, or inter-method order. Missing
or ambiguous context remains a cited limitation for the author to describe.
The output is still `DRAFT` / `UNREVIEWED`, is not a proposal, and is never
published by this command. Saved-result recovery and no-retry handling are the
same as in the endpoint recipe.


### Account for and prune documentation storage

Run `clew docs storage plan --root "$DOCS_ROOT"` first and save the returned
plan outside that root. Review `planDigest`, `plan.removeSqlite`,
`plan.removePortable` and retained reference reasons. Apply only an explicitly
requested cleanup with `clew docs storage prune --root "$DOCS_ROOT"
--expected-plan "$PLAN_DIGEST" --apply`. Without `--apply` it remains a dry run.
Pin any saved snapshot you need that is not already referenced by a publication,
Work or another retained root before pruning; a handle saved only outside the
documentation root is not a discoverable retention root.
A changed root, new reference, active reader/writer, missing retained closure,
symlink or traversal refuses cleanup; review a new plan instead of bypassing it.
The traversal is bounded to 100,000 files/objects and 512 MiB of root and closure
reads. All current/historical publications, snapshots, pins, proposals and Work
remain roots; cleanup does not expire them automatically.

SQLite payload rows and portable publication objects are separate categories.
Portable object files are partition/index/record chunks and shared source text,
not one file per original source file. Their count is not reclaimable bytes.
SQLite row removal releases reusable database space; its file may stay the same
size. Private runtime/CAS storage is separate: use `clew storage gc` there and
never delete CODECLEW_HOME paths manually. Cleanup does not fix export admission.

For a live declaration, inspect `clew docs service remove --root "$DOCS_ROOT"
--id weather` or `clew docs entity remove --root "$DOCS_ROOT" --id city`.
Only an unreferenced record can be removed with `--expected-input-digest` and
`--apply`. Current references block removal and must be changed explicitly.
Frozen publications, proposals, snapshots and Work retain their old declarations;
logical removal neither rewrites history nor implicitly cascades or frees storage.



### Describe an external service without capturing its code

Register authored information with `clew docs external put --root "$DOCS_ROOT"
--input provider.json --expected-input-digest "$INPUT_DIGEST"`:

```json
{"schema":"codeclew-documentation-external-service/1.0","id":"forecast-provider","title":"Forecast Provider","description":"Provides forecast information to Weather.","repository":"https://github.com/owned/forecast-provider","wiki":"https://docs.example.invalid/forecast","declaration":{"origin":"human","rationale":"Explicit provider registry"},"limitations":["Target code and runtime behavior have not been verified."]}
```

Use `docs external list` or `show --id forecast-provider` to inspect it. No
binding, source acquisition, compiler profile or provider request is needed.
An interaction targeting it must set `external: true`,
`externalService: "forecast-provider"` and `to.service: "forecast-provider"`.
The explicit registry reference rejects a missing destination; old interactions
with only `external: true` retain their legacy identity-only behavior.
Captured and external service identities cannot collide. Saved process
`participants` may include registered external IDs, but the process root and any
method/source/page bindings must remain captured-service declarations. External
actors stay declared and are excluded from compiler/SOURCE capture selection.

The next Check freezes this registry with the same captured inputs. Use the
retained registry as declared information in authored originals, with its
provenance and safe repository/wiki links. It is not compiler, SOURCE or runtime
proof, and adding it does not expand capture coverage. Changes to
the registry do not rewrite old publications or automatically refresh snapshots.
Use `docs recompose` to apply declaration changes to a saved capture without
acquiring source again. Review `docs external remove --id forecast-provider`
and apply with the expected input digest only after current references are
removed explicitly; no cascade or historical rewrite is performed.


### Preserve collected external information

Keep external identity/repository/wiki in the registry above. For addresses, read
`clew docs interaction show --root "$DOCS_ROOT" --id "$INTERACTION_ID"`, preserve
the real callSite/selector and edit the full interaction. This is only its fragment:

```json
{"addresses":[{"url":"https://forecast.example.invalid/api","environment":"test","source":"inventory-01; config export; claimed collectedAt=2026-11-10T09:00:00Z"}]}
```

Put collector origin/rationale in existing declaration fields, not invented typed
fields. Save a small UTF-8 report at `external-evidence/inventory-01.json` in the
docs root. This example is an author convention, not a clew schema or collector:

```json
{"collectionId":"inventory-01","collectedAt":"2026-11-10T09:00:00Z","method":"owner-supplied config export","sourceURI":"https://inventory.example.invalid/forecast","environment":"test","revision":null,"artifactSha256":null,"facts":[{"externalService":"forecast-provider","address":"https://forecast.example.invalid/api"}]}
```

Use supplied facts; unknown revision/hash stays null. Hash the actual permitted
artifact bytes, not a different serialization. Claimed collection time is not
clew observation/registration time; a hash establishes identity, not truth.
Use current inputDigest for each put. On WW_CONFLICT reconcile intent/current
record and retry with `--expected-input-digest` from currentInputDigest.

After catalogue edits, recompose the **original capture** and set SNAPSHOT to the
exact returned value. With unchanged declarations reuse the suitable snapshot;
recompose does not repair changed source selection or recapture:

```sh
clew docs recompose --root "$DOCS_ROOT" --snapshot "$ORIGINAL_CAPTURE"
```

Save request.json, then prepare/read the new Work:

```json
{"schema":"codeclew-documentation-work-request/1.0","audience":"Service documentation","externalInputs":["external-evidence/inventory-01.json"],"maxItems":100,"maxBytes":49152}
```

```sh
clew docs work prepare --root "$DOCS_ROOT" --subject "service:$SERVICE" --snapshot "$SNAPSHOT" --input request.json
clew docs work read --root "$DOCS_ROOT" --work "$WORK" --input external-read.json
```

WORK comes from prepare; external-read.json is
`{"references":["RETURNED_EXTERNAL_INPUT_REFERENCE"]}`. Finish delivery/cursors;
unread/omitted is not reviewed. At most 64 UTF-8 files, each 256 KiB; do not split
to evade limits. Work retains text/digest with HUMAN_OR_IMPORTED_UNVERIFIED
authority; that internal text digest is not the artifact's raw-byte SHA256.
Keep report/Work; changed text needs a new Work, not rewritten history.

External refs are not SOURCE cite-handles. SOURCE/SOURCE_NATIVE human helpers do
not establish external-input→human claim/footer compatibility; check that path on
the exact candidate. Do not fake sourceId, elevate authority or strip factual
annotations for a pass. Retain the attributed report and required binding blocker;
registration does not rewrite authored pages. No new provenance engine/collector.


## Retained route and OpenAPI comparison

Use `clew docs contract check --root DOCS --snapshot SNAPSHOT --service ID`
for a read-only bidirectional declaration report. Select services explicitly.
`REPORTED` means the report was produced, not conformance. Preserve candidate
authority, legacy sourceMapping, conditions, scope and UNKNOWN boundaries.
`wireConformance` remains `UNVERIFIED`; a missing captured match never proves a
missing production endpoint. This command does not recapture or require Work
preparation. Read the report summary, then the exact finding/source references.


## Exact complete retained Kotlin files

Fresh Kotlin SOURCE_SYNTAX captures preserve nonempty committed `.kt`/`.kts`
files including package/import headers, comments, whitespace and original bytes.
Use the existing saved Work with this exact selection:

```json
{"query":{"kind":"SOURCE","projection":"WHOLE_FILE","file":"src/main/kotlin/example/Service.kt"}}
```

Submit it through `docs work read --root DOCS --work WORK --input selection.json`.
The result is metadata (`SOURCE_NAVIGATION`) with `wholeFileCapture:
EXACT_RETAINED_FILE` and `fullRecordReference`; it is not body delivery. Read that
reference through ordinary `docs work read-part`, continuing until its cursor is
null. Declarations and default raw SOURCE queries keep their existing behavior.

Old snapshots lacking the complete record refuse with `WHOLE_FILE_NOT_RETAINED`.
No current-checkout/Git fallback or historical snapshot upgrade occurs. Empty or
generated files and Java compiler inputs are not added by this route. Complete
records share the existing 2 MiB per-file, 32 MiB aggregate text and 32768 record
limits; a near-limit capture may explicitly refuse and require narrower roots.
Exact retained bytes establish no compiler resolution or runtime correctness.

For a selected Kotlin computation with retained parent/body/origin links, see the optional [connected computation view](connected-computation.md).

## Documentation language

Author prose in the requested language and select the matching packaged renderer
language. Keep source identifiers and original bytes unchanged.

## Source links for shared documentation

Use the checked human export's [local Sources and service-keyed repository mapping](library-sources.md#checked-sources-footer-for-human-pages).
Preserve manifests, original text and the complete export. Historical native
exports remain readable, but their generators and replay writers are removed.
To update them, explicitly author originals from retained evidence and use the
human pipeline. Do not recapture or rewrite saved objects as a migration shortcut.

## Human HTML with addressable private metadata

Use [source views and guarded page changes](reader-metadata.md). The same
compose/bind/finalize path supports initial pages and updates.
