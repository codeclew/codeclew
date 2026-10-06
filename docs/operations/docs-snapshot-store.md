# Docs snapshot data plane (contract)

Scope: the durable service documentation store. This document defines the
capture/consumption boundary that the granular fact index and request-scoped
readers implement. It is the working contract for the docs snapshot pipeline
(docs_snapshot_pipeline.rs) and the production wiring in
`crates/clew/src/documentation/*`.

## Watching a documentation command

Documentation commands emit JSON Lines progress on stderr by default, keeping
stdout as the command's JSON result. Save both separately while reproducing a
problem; for example:

```sh
clew docs work read --root docs --work "$work" --input read-request.json \
  > result.json 2> progress.jsonl
```

Each `codeclew-documentation-progress/1.0` event includes a static `phase`,
`event` (`STARTED`, `HEARTBEAT`, `COMPLETED`, or `FAILED`), `elapsedMs`, process
`pid`, and `spanId` / `parentSpanId`. A running phase emits a heartbeat about
every five seconds. Nested spans identify the active subphase; a heartbeat for
its parent only means the enclosing operation remains active. A heartbeat is
not a percentage estimate or proof that a compiler subprocess is advancing.
An abrupt process kill can leave a start/heartbeat without a terminal event.

`LOAD_RETAINED_SNAPSHOT` and `BUILD_CONTEXT_ROWS` consume saved evidence.
`ACQUIRE_COMPILER_EVIDENCE` and `ENSURE_COMPILER_GENERATION` occur only on the
source-acquisition path. Lock phases distinguish waiting for a writer from
processing evidence. Progress carries no source text, paths, service IDs, or
error details; existing launcher/compiler diagnostics may also appear on stderr,
so consumers should select the progress schema rather than assume every line is
JSON. Failed command details remain in the normal command error output.

Set `CODECLEW_DOCS_PROGRESS=off` to disable these progress messages. Redirected
stderr is supported and does not require a terminal. A closed stderr pipe does
not make the documentation command fail.

## Explicit named snapshot retention

Pin an exact saved documentation snapshot without copying its payloads:

```sh
clew docs snapshot pin --root docs --name release-review --snapshot "$snapshot"
clew docs snapshot list --root docs
clew docs snapshot show --root docs --name release-review
clew docs snapshot unpin --root docs --name release-review
```

The pin marker stores its name, exact immutable handle and versioned reader
contract. Multiple names and snapshots share their existing CAS objects. Repeating
an unchanged pin writes no new payload or marker bytes. Reusing a name for a
different snapshot fails; remove the old mark explicitly before rebinding it.
The commands neither advance `latest-check.json` nor acquire source evidence or
invoke a model. They do not require current documentation declarations to match
historical capture inputs. The documentation repository must still be openable.

`pin` and `show` validate the current typed Check reader data, including each
original composition parent. Missing or corrupt required objects fail explicitly;
an unsuccessful verification leaves an existing marker in place. The report says
`READABLE_NOW_CURRENT_READER` and source freshness `UNVERIFIED`. It does not
revalidate current source code, certify compiler-input reuse, or imply a complete
service analysis where the retained snapshot records gaps. `show` returns the
exact `snapshot` handle for supported consumer commands. Those consumers retain
their own declaration/currentness admission rules: pinning old evidence does not
bypass them.

`list` reads only bounded markers and labels readability `NOT_CHECKED`; it does
not hydrate every snapshot. Names use existing documentation identifier rules.
At most 4096 markers of at most 4096 bytes each are accepted, with an explicit
error rather than clipped results. Corrupt or unsupported markers are reported,
not silently ignored. Marker commands serialize through the documentation writer
lock; they do not establish a global CAS reader/writer lease protocol. Interrupted
`.pin-staging-` files are bounded uncommitted records, excluded from roots and
counted by `list`; they do not block new pins after interrupted-writer recovery.
Other malformed entries still fail explicitly. The total directory enumeration
is capped at 8192 entries, including staging.

`unpin` removes only the named marker, including when its snapshot has become
unreadable. Other marks and every payload remain untouched. Removing an absent
mark is idempotent. No command here evicts history, deletes evidence or scans
unrelated object stores. Supplied diagnostic exports are never a deletion target.

This is a retention-root foundation. Before adding payload reclamation, a
collector must enumerate typed transitive references and all live roots,
including pins, latest evidence, Work, publication/history, portable packages
and in-flight writers/readers. `fact_index::reclaimable` considers one current
map only and is not a safe whole-store deletion plan. Current pin validation
is not a GC closure certificate. Payloads commit in SQLite with full synchronous
transactions before their references are published. Pins use same-directory
staged marker publication with file and parent-directory sync. These mechanisms
support process restart; pin validation does not repair a damaged database or
establish a power-loss guarantee beyond the database and filesystem contracts.

## Saved evidence is the consumer default

`docs check` is still an explicit evidence acquisition command. Its response
now includes `snapshot`, an immutable `sha256:<digest>/<size>` check-manifest
handle. Save that exact value; it remains readable after another check replaces
`latest-check.json`. The manifest references shared evidence objects rather than
copying their payloads. Use the named snapshot pins below to explicitly retain
and retrieve a handle; automatic eviction and evidence GC remain unimplemented.

For example, after `docs check --root docs --service orders`:

```sh
clew docs context --root docs --service orders --snapshot "$snapshot"
clew docs work prepare --root docs --subject service:orders \
  --input request.json --snapshot "$snapshot"
clew docs render --root docs --snapshot "$snapshot"
clew docs render --root docs --refresh --require-complete
```

Work retains the selected snapshot through reads, proposal submission, local
publication and agent-reviewed publication. With no `--snapshot`, ordinary
consumers resolve the latest saved check once and freeze its immutable handle;
missing evidence never causes implicit acquisition. `context`, `work prepare`,
`render`, `changes`, interaction candidates, process inspection and update-queue
work all consume saved evidence. `docs check` and explicit `context --refresh`
remain acquisition boundaries; portable evidence capture is also explicit.
`render --refresh` is the one explicit render acquisition route: it runs the
ordinary current-source check, saves that result, and publishes only after the
fresh evidence passes the same completeness gate. It may run Maven/compiler
analyzers. `--refresh` and `--snapshot` are mutually exclusive. A saved render
never treats the latest pointer or timestamps as current-source authority.

A service Work may use a full multi-service snapshot without another service
capture. It retains that exact frozen Check, including recorded annotation and
review scopes: no current overlays are replayed during selection. Its initial
service context is selected normally, but obligations, global domain facts,
linked sources, handles, influence and accepted revision vectors may still
cover other services. This avoids reacquisition; it is not service-local
hydration or a compact derived snapshot. Capture `check --service` when new
service evidence is actually needed, not merely to narrow an existing snapshot.
A selected service absent from saved evidence fails with explicit guidance.

`changes --snapshot` and `interaction candidates --snapshot` select historical
evidence explicitly. Change dossiers compare saved evidence with the accepted
baseline; they do not discover unobserved live source changes. Use the
observation-only status command for target changes and explicit capture when
new semantic evidence is required.

Consumers verify immutable object integrity and current documentation
declarations. Missing objects, corruption or changed declarations are errors;
the user must explicitly obtain replacement evidence. Source repositories need
not be accessible. Admitted human/external authoring inputs and concurrent
publication guards remain checked separately.

Historical validity does not prove current source freshness. Snapshot renders
mark otherwise-current sections `UNVERIFIED` with
`PINNED_SNAPSHOT_NOT_REVERIFIED`; existing `STALE` states and meaning-review
results remain intact. `--require-complete` rejects unverified freshness.
Snapshot consumers do not change `latest-check.json` or the selected manifest.

Check continuation cursors freeze the original snapshot, selection and freshness
report. Subsequent pages do not recapture, follow latest-check or recompute
freshness against a newer publication. A missing report/snapshot is an explicit
error. Legacy cursors issued before this change cannot be resumed.

Context and changes cursors also bind the resolved immutable snapshot, even
when the first page selected latest implicitly. Keep the returned `snapshot`
and pass it with `--cursor` to resume after latest changes; a cursor cannot cross
snapshots with equal semantic context digests. `context --refresh` cannot be
combined with `--cursor`; refresh once, then page through the returned snapshot.

## Implemented observation-only status refresh

`docs refresh --status-only` no longer captures evidence. It observes declared
local target refs or admitted coordinator selections, without fetching, running
compiler providers, loading latest-check, or regenerating explanations.
It also compares retained declarations and registered human inputs. Identical
authoring-input selections share one fingerprint calculation within a refresh.

Known participant revision or recorded-input changes mark affected content
`STALE`. Missing targets and incomplete declaration scope are explicit unknowns;
matching Git revisions alone yield `UNVERIFIED`, not semantic freshness. An
unrelated coordinator event does not make an independent service stale. A
selected event that no longer matches the current policy remains historical
evidence but provides no current target authority; observation does not fall
back to a different local ref.

Retained prose, source evidence, content revision vectors and meaning-review
results remain unchanged. Prior `STALE` is preserved until an explicit recheck.
Repeated identical observations return `UNCHANGED` without another publication.
Process/view `targetObservation` distinguishes a required recheck from a graph
that has not been rechecked. Existing manual-edit and concurrent-publication
guards still apply. This path reads retained bindings and outputs; it does not
yet establish bounded per-query IO for large publications.

## Shared evidence in saved Work

### Discover captured files without dependency source links

For a service Work with a registered captured `SOURCE_SCOPE`, use this raw
selection with `docs work read --root docs --work "$work" --input sources.json`:

```json
{"query":{"kind":"SOURCE"}}
```

This inventory includes retained `FILE_ONLY` sources such as an explicitly
captured README even when no dependency cites them. It searches only that
service's immutable captured file membership. Continue using the same selection
and returned cursor. Each returned SOURCE row or oversized omission carries its
exact `s` reference; use that returned reference for a separate read or
`docs work read-part`. Do not guess reference numbers. A discovery query cannot
establish that omitted text was read or that a file has parsed semantics.

SOURCE records have no symbol field: omit `symbolContains` or set it to the empty
string. A nonempty filter is rejected; it does not search paths, IDs or source
text. `*` still searches dependency kinds only. Compiler services without a
captured source inventory use declaration `sourceReferences` instead. Scenario
inventory and unregistered source scopes are explicitly refused. Driver
`selectionGuidance.availableKinds` advertises SOURCE only when this Work admits
the inventory.

SOURCE query receipts bind the exact Work, selection and page membership. The
existing conservative Work influence watches the declared SOURCE_SCOPE,
including file membership and content digests, even for an empty query. A later
captured file addition, removal or change can invalidate accepted content;
the original Work still reads its original snapshot. Absence describes only
the captured scope, not unobserved files or runtime behavior.

An otherwise-unreferenced whole `FILE_ONLY` source can support a manual prose
claim through its existing SOURCE_SCOPE inventory pin. SOURCE citation
materialization adds that exact watched scope when no direct dependency cites
the file. The source must retain its complete byte occurrence from offset zero,
line range, service and revision, verified text and occurrence digests, and an
inventory entry with matching `FILE_ONLY` coverage and whole-file text digest.
Partial excerpts, parsed-source inventory entries and mismatched pins cannot use
this fallback. This binds captured text only; it creates no compiler fact or
semantic approval and does not change the historical source or scope records.

### Read one retained SOURCE in bounded parts

When an initial Work page marks a SOURCE as `ITEM_EXCEEDS_WORK_BYTE_BUDGET`,
use its exact `reference` with `docs work read-part`. The command reads only the
immutable snapshot pinned by that Work. It does not follow a newer
`latest-check.json` or acquire source evidence.

Create a request such as:

```json
{
  "schema": "codeclew-documentation-source-part-request/1.0",
  "reference": "s42"
}
```

Then run:

```sh
clew docs work read-part --root docs --work "$work" --input source-part.json
```

Each response repeats the SOURCE metadata without its text, gives the raw UTF-8
byte range and fragment digest, and includes the next cursor. Continue with the
same schema and reference plus that cursor until `nextCursor` is `null`. Each
response is bounded by the Work's fixed `maxBytes`, including serialized
metadata, digests, cursor and the output newline. A SOURCE becomes recorded
proposal evidence only after its explicit receipts cover the complete text;
an empty source also needs its one explicit empty-range receipt. This resolves
only the exact oversized SOURCE omission. Other omitted records and the
ordinary initial-page chain remain required. Manual parts do not bypass the
automatic author check that its actual model request contains the complete
required context. `recordDigest` hashes the canonical complete SOURCE record;
the stored `textDigest` and `evidenceDigest` remain metadata and are not part
digests. A final `nextCursor: null` marks the end of that source, but does not
prove earlier parts were read. Proposal eligibility uses validated gap-free
receipt coverage. If metadata leaves no room for a UTF-8 byte (or the explicit
empty response), the command returns `SOURCE_PART_NO_PROGRESS` and writes no
receipt. Prepare a new Work against the same immutable snapshot with a larger
`maxBytes` budget (up to 49,152); if the complete retained metadata still cannot
fit, source-part reads do not support that record.

The Work read ledger stores only receipt and range digests, not copied source
chunks. A ledger with `sourcePartReceipts` requires a part-aware reader; the
0.11.2 reader rejects this new field. Work without part reads retains its
existing serialized ledger shape. The Work loader still hydrates the full
retained Check; parts do not claim memory use proportional to one response.

### Read one retained operation in bounded parts

When a Work page omits `RETAINED_OPERATION` with
`ITEM_EXCEEDS_WORK_BYTE_BUDGET`, use its exact `kind` and `id` with
`docs work read-retained-part`. Retained operation rows have no SOURCE handle.
The reader uses only the full operation saved in that immutable Work; it never
follows the latest publication, recaptures source, or treats retained prose as
newly verified evidence.

```json
{
  "schema": "codeclew-documentation-retained-part-request/1.0",
  "kind": "RETAINED_OPERATION",
  "id": "reserve"
}
```

```sh
clew docs work read-retained-part --root docs --work "$work" --input retained-part.json
```

The response's `text` is a fragment of the canonical JSON encoding of the
complete retained operation. Concatenate these fragments in `startByte` order
and parse the resulting JSON to obtain every field. Byte ranges are UTF-8
boundaries in that canonical JSON, not offsets into an individual prose field
or the original Work file. Each response binds Work, snapshot, record kind and
ID, complete `recordDigest`, range, `totalRecordBytes`, fragment digest and
receipt digest. The complete serialized response, including its output newline,
fits the Work's fixed `maxBytes`.

Continue with the same schema, kind and ID plus the returned `nextCursor` until
it is `null`. Interrupted reads can resume from their last cursor. Reading an
identical part again retains one receipt and cannot fill missing ranges. The
read ledger stores `retainedPartReceipts` without copied fragment text; this
field is absent when empty, preserving earlier ledgers' serialized shape.
Older readers that do not recognize this additive field cannot read a ledger
after retained-part reads have been recorded.

Only exact validated, contiguous, nonoverlapping coverage resolves the matching
retained-operation omission for manual proposal submission. Every initial Work
page and every other required record remains necessary; oversized declarations
and external inputs still block completeness. A terminal part alone does not
prove earlier delivery. Invalid identities, stale/cross-Work cursors, corrupt
receipts and no-progress ranges are rejected. If even one UTF-8 record byte and
the receipt envelope cannot fit, `RETAINED_PART_NO_PROGRESS` writes no receipt
and advises preparing Work from the same saved snapshot with a larger
`maxBytes`, up to 49,152.

These reads do not establish delivery to automatic author or reviewer model
requests. Their existing packet gates still require actual delivered context
within the model's input cap. Retained operation parts remain authored context,
not SOURCE citations. The Work loader and reader still hydrate the complete
saved operation; bounded output does not imply bounded total memory use.

### Recorded source-selection inputs

New checks retain a versioned `sourceInputs` contract. Its captured Service,
portable-evidence expectation and coordinator target values drive source
selection directly. Changing those files during acquisition cannot substitute
different values under the original input identity. The final declaration
comparison still rejects a persistent change. This is computation over captured
values, not proof that the filesystem stayed unchanged continuously.

The Check manifest references a separate input manifest. Services, interactions,
scenarios, entities, expectations, update policies and target events share
per-record CAS objects. Each note separates its original text/status from
association metadata, so a title or target edit can retain the text object.
Reconstruction verifies reference schemas, object integrity and the canonical
input digest. Missing or corrupt inputs fail without acquisition fallback.
Legacy snapshots with no contract remain readable and do not acquire a
fabricated contract from current files.

New captures also attach entities, protected notes and data-flow views from
these captured values. Target existence and scope membership use the captured
catalogues, including declared services without selected source evidence.
Returning edited files to their original contents cannot conceal different
note text or membership being attached under the original input identity.

This is not a complete native build-input reuse key and does not change native
`NON_CACHEABLE` authority. Input hydration and the reference manifest remain linear in the number
of records; text repeated in differently shaped overlay objects is not deduped
by this representation.

### Explicit declaration recomposition

Check dependency maps use stable map-slot identities, independently of the
whole documentation input digest. An unchanged observation shares the same
membership page across snapshots; changing its full payload updates its bucket.
The enclosing Check still binds source revisions and consumed inputs. A map
root alone does not establish source provenance. This exact-map writer does not
read or update the mutable generic fact index, whose revision semantics remain
unchanged. Current-format snapshot pages share unchanged payloads.
Full-map serialization and hashing still occur, and historical objects are not
automatically reclaimed.

`docs recompose --root DOCS --snapshot ORIGINAL_CAPTURE` creates a new immutable
Check using retained source evidence and current documentation declarations.
Use its returned handle with `docs context --root DOCS --service ID --snapshot
DERIVED` or `docs work prepare --snapshot DERIVED`. It does not run source
analyzers, generate prose, publish documents or advance the latest pointer.

Recomposition requires a current-format original capture with a recorded
consumed-input contract. Obsolete snapshots are rejected. Derived
parents are rejected; use the original capture for each new declaration
revision. All Service records (including titles), portable-evidence expectations,
update policies and targets must remain identical. Documentation title,
entities, notes, interactions and scenario/process/view declarations may change.
Newly requested facts can remain unavailable in retained evidence; recomposition
does not acquire them or claim source freshness.

The original `sourceInputs` contract remains unchanged. A separate composition
manifest binds current granular declaration inputs, the original parent handle,
composer implementation, exact baseline byte receipts, selected complete child
operations, accepted versions and external-input fingerprint scopes. It stores
per-record references rather than another full copy of published bindings or
external prose. Loading also verifies that source references and unresolved
source outcomes match the original parent manifest; retain that manifest with
the derivative. Derived inline Check imports are unsupported.

External input capture deduplicates identical ordered requests before reading
their files. All requests must observe the same protected-note tree; overlapping
paths and associated notes must agree on captured status and content digest.
The stage allows at most 4,096 retained operations/versions, 1,024 input paths
and 64 MiB of captured external body bytes per pass, counting repeated reads
of overlapping paths. Exceeding a bound returns an error without clipping.
Final optimistic comparison rereads declarations and retained inputs under the
documentation write lock before storing a handle. This is computation over
captured values with change detection, not an atomic filesystem snapshot.

Repeated recomposition from the same parent, declarations, retained inputs and
composer returns the same handle and shares existing CAS payloads. It still
performs bounded hydration and validation. Named documentation snapshot pins
are supported below; payload reclamation and full native-input reuse remain
separate lifecycle concerns.

### Work references

New work records use `codeclew-documentation-work-manifest/2.0`: an immutable
`evidenceSnapshot` handle replaces the embedded Check, and typed references to
the handle and influence maps replace their per-Work copies. Equal map payloads
share content-addressed objects even when request, subject, scope, or snapshot
bindings differ. The manifest separately binds those identities, external
inputs, retained prose, obligations, and profile, so sharing a table never
merges Work identities. References validate their schema, digest, size, typed
payload and canonical encoding. The manifest commits only after both objects
are durably stored. An interrupted preparation can leave unreferenced table
objects that a retry can reuse. The documentation CAS has no object collector;
shared and orphaned objects are retained.

The manifest schema and digest are checked before its evidence is opened; an
explicit historical `snapshot` must match `evidenceSnapshot`. Missing or
corrupt tables or evidence fail without source acquisition or latest fallback.
Restore a complete documentation-root backup when a referenced table object is
missing or corrupt. `docs work prepare --root DOCS --subject service:ID --input request.json --snapshot SNAPSHOT` prepares a new format against the same saved evidence.
Older inline Work records are unsupported by this reader and report
`DOCS_WORK_REPREPARE_REQUIRED`. They and their read, answer, and other records
remain unchanged and bound to the old Work ID; new Work has a new identity and
never silently rebinds those records. The 64 MiB Work-record limit applies to
manifest metadata, not to the hydrated Check or the separate maps. Each shared
object remains subject to the cache's existing portable object-size bound.

An agent run reuses its verified Work for initial and expansion pages, avoiding
reopening the same snapshot for each page. Reads in a new process still hydrate
the full Check and the referenced tables. Repeated preparation validates the
saved manifest and both tables against the freshly derived request, scope,
snapshot, input, and profile bindings while reusing the Check already hydrated
for that preparation. Page limits remain independent of the complete
model-request budget.

Check hydration verifies dependency-index pages in one traversal, then decodes
the selected observation payloads. Every referenced page and entry count is
checked before payload decoding; an empty selected scope cannot bypass a
corrupt page. This removes duplicate metadata reads while preserving existing
scope-load validation. It is not selective payload loading or recursive
verification of unrelated payload objects.

Source-syntax captures with an enabled semantic provider do not reuse keyed
composite captures: the syntax key lacks complete provider input authority.
They write `NON_CACHEABLE` with a reason. Pure
syntax retains its cache path; explicit historical snapshot reads remain
available independently of current provider admission.

## Initial model-request preflight

Work pages annotate delivered items with `referenceRoles`: `evidence`,
`operation`, and/or `gap`. Only the item's `reference` is a proposal handle;
record IDs are not aliases. Items without handles have an empty role list.
Scenario pages also expose their special target as `subjectReference`.
These are additive advisory fields, derived from the proposal validator's
eligibility rules and included in page byte limits. A role does not bypass
recorded-read requirements, requested-entrypoint scope, evidence combination
limits, source support, freshness or meaning review. `sourceReferences` are
expansion links and do not prove those sources have been read. Existing gap
references keep their distinct validation rule; they do not require the same
read receipt as claim evidence.

An isolated author run checks the complete serialized initial job envelope,
including the proposal schema, obligations, page records, role metadata and
configured input overhead. It checks fixed overhead first, then each candidate
page, and stops at the first overflow before source revalidation, reservation
or model dispatch. It never sends a prefix in place of required context. The
final dispatch guard still checks repairs, expansions and reviewer requests.

Run reports expose `contextBudget` for the `INITIAL_AUTHOR` stage. Its
`candidateRequestBytes` is a complete request size only when `complete` is true;
otherwise it is a prefix lower bound with the remaining context unread.
`configuredOverheadInputTokens` and `conservativeInputLimit` retain the existing
admission contract. Serialized bytes are not actual token usage or money.

Input-cap failures update the run report without changing the publication.
Other explicit-snapshot failures can publish their generation gap from that
snapshot, without falling back to source capture. Legacy non-snapshot failure
publication retains its existing behavior.

## Opt-in section author contract

Work preparation also accepts `"contextProfile": "declarations-v1"` in its
request for a service's `section-entities`. This is independent of the author
output contract below. Omission preserves the full initial selection and old
Work identity. Unknown profiles and incompatible targets fail before capture.

The profile keeps the requested section, admitted dependency kinds other than
`SYNTAX_DETAIL` and `FLOW`, their exact deduplicated source records, and every
review reason, external input and obligation. Unknown dependency kinds are
preserved. It keeps global domain-entity facts admitted by the original section
selection. A referenced source missing from the retained snapshot is an error;
source text is not fetched or clipped to fit.

An advisory `CONTEXT_PROFILE` row describes selected/deferred membership,
counts and digests. It preserves inventory gaps and source boundaries while
replacing callable arrays with counts and a digest. Other sections retain a
bounded list of identities, required flags and states. This summary has no
evidence handle and cannot support a claim about an unread detail. Explicit
section expansion returns the original full inventory; dependency queries and
handle expansions retain their existing behavior.

Profile deferral is distinct from a required item omitted by a page byte limit.
Every profile member must be delivered before author dispatch or proposal
acceptance; an oversized required source or human input still blocks that
path. Deferred evidence becomes citable only through actual recorded reads.
The profile version is part of immutable Work identity and cursor ownership.
Its selector semantics are fixed: changing membership rules requires a new
profile version; existing Work must not be reinterpreted under new rules. Full
influence and currentness checks remain unchanged, including deferred facts;
this changes transmission, not snapshot hydration or invalidation scope.

For one captured Java HTTP endpoint, Work accepts
`"contextProfile": "endpoint-context-v3"`. The profile supplies the retained
endpoint route, direct request/response FIELD facts, eligible reachable method
sources, and retained FLOW or CALL_RELATION facts. It keeps all selected
reachable method bodies and deduplicates their source text; traversal ends at
the natural evidence frontier and uses visited identities to stop cycles.
Provenance, scope and ambiguity checks remain in force. Missing or unparseable
bodies remain explicit gaps. There is no method-count or aggregate-source-byte
selector cutoff; delivery and provider limits must be reported explicitly rather
than silently dropping selected source evidence.
Legacy same-owner helper and constant discovery is marked
`SOURCE_REFERENCE_CANDIDATE`; it is lexical evidence and does not establish
compiler resolution. DTO class source is not included. The initial page places
FIELD facts and selected SOURCE rows before FLOW and CALL_RELATION facts. The
packet groups references to those facts and reports unavailable, ambiguous, or
truncated paths as gaps.

An explicit `this::method` is a lexical `METHOD_REFERENCE` candidate; it records
a callable reference and does not establish invocation at that source position.

`clew docs work packet --root docs --work WORK_ID` projects the saved rows into a
compact author packet. Each retained source is included once, and method nodes
point into that source; the packet keeps scopes, coverage, candidate authority,
and every recorded limitation code. It states that runtime and serialization
effects remain unknown. This projection does not create a Work read receipt or
an accepted narrative. Add `--audit-output packet-audit.json` to write a
separate verification file with full selected rows, Work handles and digests;
those audit-only rows are not described as delivered to the author.

For the ordinary one-author endpoint draft, use `docs work run --draft`; it
builds this packet and renders the returned answer through the same saved Work.
The complete preparation, author-only configuration and recovery recipe is in
`skills/codeclew/references/service-documentation.md`.

Internal callable Work may opt into `"sourceDataContext": true` in the same
`process-graph-v1` preparation request. `rootDeclaration` selects an exact retained
Java compiler callable. Run `docs work packet --root ROOT --work WORK_ID` to inspect
the resulting `sourceDataContext` and genuine Work citation labels before authoring.
The graph uses only the immutable Work snapshot; it never captures or reads current
source. Omitted and false retain the ordinary Work and packet representation.

This context reuses the native source-call and data-state projection: shared guarded
definition IDs preserve alternatives, opaque transformations and field interference.
Compiler variable identity is distinct from source-syntax transfer and model meaning
review. Actual/formal and return links are declared-target conditional; source-order
normal-completion prerequisites do not prove runtime success, receiver identity,
delivery or incident facts. Meaning remains `UNASSESSED` and runtime `UNKNOWN`.
The source-data digest is separate from examined-source digest and packet digest.
`sourceDataContext.sources` delivers complete exact source records independently
of legacy containing-class aliases. Definition `sourceSpan` keys preserve native
covering source ranges; `citationId` and span `reference` are genuine Work labels.
The addition bound includes these sources, context and all additional citation metadata.
Node-local `dataState.shared` tables intern exact storage, guard and completion sets;
`storageRef`, `guardSetRef` and `completionSetRef` are lossless table references,
not compiler identities or source citation labels. Definitions still reference
prior definitions instead of copying paths. Full variable facts remain in the
immutable audit and are bound by per-node count/digest; unused statement spans
and duplicate variable-fact citation descriptions are not author evidence rows.
The fixed traversal bounds remain depth two, 64 additional bodies and one MiB of
additional source. Complete added context and exact sources must fit 65,536 bytes;
preparation refuses larger selections before any model reservation. This bound is
not a provider token or monetary cap. Existing role preflight still counts the whole
model envelope. Authoring, review, repair and export validate the exact saved context
and source delivery; no implicit publication follows this opt-in.

Internal callable drafts use the separate `process-graph-v1` profile with an
exact scoped `rootDeclaration` and a persisted question from the same saved
snapshot. Their compact author packet includes linked containing-type source
when retained, while marking source candidates as context rather than executed
calls. The full process graph remains in the operator-only audit. Use the same
`docs work run --draft` command and author-only configuration; do not reuse an
endpoint-context Work for this profile.

A new endpoint `process-graph-v1` Work can explicitly select one frozen human
paragraph with `maintainedParagraph` in its preparation request:

```json
{
  "schema": "codeclew-documentation-work-request/1.0",
  "audience": "Maintainers",
  "contextProfile": "process-graph-v1",
  "rootDeclaration": "<exact current endpoint declaration ID>",
  "question": "Explain current source with the selected attributed context.",
  "maintainedParagraph": {
    "bundle": "<exact published bundle ID>",
    "operation": "<discovered endpoint operation ID>",
    "fragment": "<human explanation paragraph ID>"
  }
}
```

Prepare it with `docs work prepare --root docs --subject service:ID --snapshot
SNAPSHOT --input request.json`. Preparation verifies the immutable publication,
its original snapshot and the exact endpoint symbol, declaration and scope.
The selected current Check must still discover that same endpoint operation and
retain the selected root in its dependency membership; removal of its endpoint
route is refused even when the method remains. Preparation freezes the paragraph's text, declared author, effective source snapshot,
source/dependency pins, event anchors and any explicit context migration into
`maintainedContext` in the new Work. A changed logical endpoint identity is
refused; this selector does not map old IDs to new IDs.

For a new endpoint Work, `maintainedFromBundle: {"bundle": "BUNDLE_ID"}` may
replace `maintainedParagraph`. The explicitly selected frozen bundle must have
one uniquely authored paragraph associated with the same discovered current
endpoint, service, scoped declaration, and full callable descriptor. Missing or
ambiguous matches are refused with at most eight bounded candidates; use the
explicit paragraph selector to choose among competing fragments. This does not
search other bundles, select the latest version, or remap changed declarations
or anchors.

The automatic input normalizes to the existing exact paragraph selector before
Work storage, preserving its Work identity and complete packet bytes. Original
text, text author, context editor, and source pins remain intact; changed source
context remains conservatively `STALE`, with meaning `UNASSESSED`. Omitting both
selectors preserves legacy serialization and performs no history lookup. Saved
Work never resolves an automatic selection again.

`docs work packet` and the author request deliver this complete context before
calculating the packet digest and request budget. `CURRENT` means the pinned
root, sources and dependencies match the selected Work; it does not assess the
human prose. Changed source bytes produce `STALE` while retaining the original
attributed text and historical records. The text author and an explicit context
editor remain distinct, and meaning/context review remains `UNASSESSED`.
Historical records are separate from compiler citation labels and current
method bodies. Their presence does not prove a compiler fact or approve a claim.

The inline context limit is eight MiB within the existing 64 MiB Work limit.
Public paragraph text edits retain their existing 8192-byte limit; complete
source context can exceed the ordinary 49,152-byte Work read page. Packet and
author input delivery do not truncate that selected context. An oversized
complete author request fails its input cap before the driver starts; narrow
selection or use a configured supported larger input budget. Omit the selector
(or use `null`) to retain legacy Work and packet bytes. Existing saved jobs are
not augmented, and later packet reads use only the frozen Work and snapshot,
including when the original source checkout is unavailable. This selection does
not automatically edit text, remap anchors or attest to human meaning.

To render a saved structured answer conforming to
`codeclew-operation-answer/1.0`, `codeclew-operation-answer/1.1`, or
`codeclew-operation-answer/1.2`, run
`clew docs work explain --root docs --work WORK_ID --input answer.json --output-dir .codeclew/drafts/WORK_ID`.
The command rebuilds the packet from that saved Work, checks the packet digest
and evidence labels, then atomically replaces each of `answer.json`,
`operation.md`, `index.html` and the packet/audit reference files individually.
New `docs work run --draft` executions require answer 1.2; offline rendering of
saved 1.0 and 1.1 answers remains supported. New operation Work records the
versioned `authoringContract` value
`codeclew-operation-draft-authoring/1.4`, which binds the current answer schema,
generic author policy, and packet projection. Endpoint packets include selected
referenced owner FIELD declarations in `fields`; `constants` remains the
static-and-final subset. Initializer tokens preserve declaration syntax but do
not establish runtime values or initialization timing, and `final` does not
establish deep immutability. A material change to the schema, policy, or packet
requires a new identity and new Work prepared from the same saved snapshot; old
Work identity is never rewritten.

New `process-graph-v1` Work with `sourceDataContext: true` and a non-empty
question defaults to `codeclew-operation-draft-authoring/1.5`. This separate
question-focused policy keeps the complete packet, exact sources, source-data
IR, citations and answer schema 1.2. It asks for the requested answer and only
its necessary prerequisites, decisions, mutations, failures and uncertainties;
it does not require exhaustive documentation of unrelated packet behavior.
Glossary and preparations may be empty when they do not help that question.
The ordinary reviewer still assesses every included answer block and its
material claims against the complete saved packet. Explicit 1.4 remains
available; saved 1.4 inputs and reviews keep their exact policy bytes. Saved
1.5 answers support replay, review, explicit bounded repair and approved export
through the same immutable bindings. This policy does not change model,
transport, timeout or token reservations, or establish a latency guarantee.

To enable grouped retained-context requests, prepare new Work with explicit
`authoringContract: "codeclew-operation-draft-authoring/1.6"`. This opt-in works
with `endpoint-context-v3` or a `process-graph-v1` question. The complete initial
semantic packet retains its source identities, guards, effects and explicit
gaps; no current-source capture is performed during expansion. Default 1.4 and
1.5 Work and their saved results keep their original behavior.

Use `codeclew-documentation-operation-draft-execution/1.1` with `author`,
`budget` and a positive caller-selected `authorCalls`. Each model decision is
either `{"action":"answer","answer":...}` or
`{"action":"expand","selections":[...]}`. A selection uses native references,
exact symbols or a query; one decision may group many references. The host
chunks the existing native read limit and drains content pages and contiguous
source parts. SYMBOL queries return one navigation page; navigation handles
must be selected separately before they become citable evidence. The next
answer binds the new packet digest. Unknown or ambiguous exact symbols return
lookup feedback rather than fabricated evidence.

Independent review uses
`codeclew-documentation-operation-draft-review-execution/1.1` with `reviewer`,
`budget` and positive `reviewerCalls`. It may return a grouped expand action or
`{"action":"review","review":...}`. The inner meaning review uses schema 1.1
and binds `reviewContextDigest`; its additional reads never rewrite the saved
author packet or answer. Both roles reserve their entire configured finite
call budget before execution. Reservations are maxima, not actual provider
usage. Each role receives `roleBudget.configuredCalls` and
`roleBudget.remainingCalls`, including the current decision, so it can return
its terminal answer or review before exhausting the budget. Complete source
receipts and exact saved inputs/results are validated
when recovering or exporting an approved review without a configuration.

Rerun the same command and configuration to recover a saved invocation or
interrupted context delivery. An uncertain invocation is not automatically
dispatched again. After a terminal unsuccessful author run, explicit
`--new-run` permits a fresh attempt with a new finite configuration. This mode
does not yet support `--repair-from-run` or `--repair-from-review`.
For an original terminal uncertain review without a saved verdict or result,
explicit `--retry-from-review FAILED_REVIEW_RUN` starts a replacement reviewer
under a new empty account and finite configuration. Its evidence seed is bound
to the failed checkpoint; the author answer and original accounting stay
unchanged. Reusing an old failed selector after its child finishes is refused;
ordinary replay selects the child. Completed or invalid results and failed
retry children are not replaceable by this selector. Complete interrupted
accounting with the original configuration before starting a fresh attempt.
Inspect the original result and accounting first. A failed invocation remains
retained and no refund or provider cancellation is implied.
Schemas for the new configurations and meaning review have separate `-1.1`
files in `schemas/documentation`; the original 1.0 contracts are unchanged.

Existing 1.3 Work keeps its prior packet and may only replay a validated saved
answer, including after an interrupted output write; a fresh author attempt
requires new 1.4 Work prepared from the same saved snapshot.
The output directory is not replaced as one transaction. If a command fails
between file replacements, rerun it with the same saved Work and answer to
finish the draft; this performs no index capture. The result remains a local
`DRAFT` / `UNREVIEWED`; it does not publish or create a release version.

### Review a saved operation draft

To export an approved saved review without a configuration or another model call:

```sh
clew docs work explain --root docs --work WORK_ID \
  --review-run REVIEW_RUN_ID --output-dir reviewed-answer
```

Select exactly one of `--input` and `--review-run`. Reviewed exports require a
new or empty output directory and preserve the original author export. The host
loads the exact author and reviewer invocation records, selected checkpoints,
complete saved packet and immutable Work snapshot. It rejects missing or corrupt
records, mismatched coverage and forged approval. Mutable exported answers and
latest-run pointers are not authority; this command does not read a driver
configuration, dispatch a model, update reservations, or switch the live index.

The HTML and Markdown show `MODEL REVIEW: APPROVED` and `NOT PUBLISHED`, retain
all reviewer limitations and findings, and include declared models and exact
source, packet, answer and invocation digests in supporting provenance. The
export includes `meaning-review.json` and `review-provenance.json` beside the
answer, packet and source audit. Model approval covers meaning against the saved
packet only. It is not compiler proof, current-source verification or execution
evidence. A missing retained snapshot rejects export even if current source is
available. Saving this local reader does not admit it to the documentation
catalogue or publication history.

To explicitly admit an approved answer to the documentation catalogue and
immutable publication history, first save the exact current baseline:

```sh
clew docs work publication-baseline --root docs > answer-baseline.json
clew docs work publish-answer --root docs --work WORK_ID \
  --review-run REVIEW_RUN_ID --baseline answer-baseline.json
```

The closed baseline is `{"kind":"NONE"}` only for an initial empty publication;
an existing catalogue requires its exact bundle, index digest and bindings
digest. The command checks that baseline again under the publication lock and
returns the next baseline. Stale or concurrent changes reject publication.
Selecting the same already published answer is idempotent.

Publication stores a separate typed reviewed-answer binding and standalone
reader route, with the exact saved answer, packet, source audit, meaning review
and provenance in the immutable bundle manifest. It preserves prior narratives,
answers and historical bundles. Later ordinary render or status updates retain
the reviewed answer artifacts. This does not create a narrative proposal or
change original author/reviewer records, exports or accounting; no model or
source capture is dispatched. Meaning approval remains model review against the
saved snapshot, independently from current narrative sources or source
freshness. The original review provenance retains its pre-publication status;
the separate publication entry and reader report explicit publication. PlantUML
and the source tree are retained without launching an SVG renderer. Catalogue
answer payloads are limited to 64 entries and 64 MiB; this explicit publication
path also bounds copied prior bundle input to 64 MiB. Existing publications
without reviewed answers retain their previous schema and limits.

To review one successful original operation draft without another author call, use:

```sh
clew docs work review-draft --root docs --work WORK_ID \
  --source-run AUTHOR_RUN_ID --config review-execution.json
clew docs work status --root docs --work WORK_ID
clew docs work cancel --root docs --work WORK_ID
```

A terminal `DRAFT_REVIEW_UNCERTAIN` with no durable response keeps its maximum
reservation. Ordinary replay never launches another reviewer. After inspecting
that failed invocation, an explicit replacement can select its exact run:

```sh
clew docs work review-draft --root docs --work WORK_ID \
  --source-run AUTHOR_RUN_ID --retry-from-review FAILED_REVIEW_RUN_ID \
  --config new-review-execution.json
```

Use a new empty reviewer budget account. This starts one reviewer invocation
against the same immutable author answer, complete packet and host coverage;
it never starts an author. The failed review must be the selected latest child
of that exact author and must have a terminal uncertain checkpoint, no verdict
and no saved result. Completed, cancelled, invalid and nonterminal reviews
cannot be replaced by this selector. The original failed records and maximum
accounting remain unchanged; no refund or provider cancellation is implied.
A retry child carries exact prior invocation/checkpoint lineage and remains
unpublished. Once it finishes, reusing the old failed selector is refused;
ordinary replay without the retry flag selects the existing child. This first
slice does not replace a failed retry child or import an external completion.

The closed configuration uses
`codeclew-documentation-operation-draft-review-execution/1.0`, with `reviewer`
and `budget` fields. The reviewer role has the same isolated driver and finite
per-call cap structure as an author role. Use a separate budget account; this
command reserves exactly one reviewer call and zero author, repair, fallback or
expansion calls. Configured token and local cost-unit maxima are coordinator
reservations, not enforced provider billing or monetary cost limits. Missing
usage dimensions retain their maximum reservation and remain unknown.

The command selects the immutable author input and result through the exact
saved report, checkpoint and invocation. It checks the Work snapshot, original
packet and answer digests, and passes the saved author instruction and guide as
untrusted review material. Editable exported `answer.json` is not authoritative.
Review accepts successful original author runs with answer 1.2 and authoring
contract 1.5, 1.4 or 1.3, plus successful semantic-repair children whose complete
saved rejection lineage and author payload validate. Invalid answers and native
validation-repair runs remain ineligible. No author driver is needed for review.

The reviewer returns the closed
`codeclew-operation-draft-meaning-review/1.0` result. The host derives unique JSON
paths covering the title, summary, glossary definitions, all three predicate
claims, recursive steps, preparation summaries and steps, and uncertainties.
The result must acknowledge every exact path and used packet evidence key once.
Those acknowledgments check coverage and identity; they do not establish that
semantic assessment was correct. `APPROVE`, `REJECT` and `NEEDS_EVIDENCE` remain
local, unpublished outcomes. Approval does not create a narrative proposal,
release, publication, or source authority claim.

The review owns a new run report, checkpoint and reservations. Its latest-run
pointer supports the existing progress, status and cancellation commands; the
original author's report, immutable input/result, rendered files and account are
not rewritten. Rerun the exact same command/configuration to replay a saved
result or recover an interrupted result write without a second call. A changed
review configuration is rejected. A dispatched reviewer with no durable response
is never automatically dispatched again; inspect its retained invocation and
maximum accounting. Cancellation and timeout stop the local adapter process;
they do not prove provider cancellation or absence of billing. Invalid reviewer
output is retained without automatic repair, replacement review or publication.

One explicit semantic repair is available for an exact saved `REJECT`:

```sh
./clew docs work run --root <root> --work <work-id> --draft \
  --repair-from-review <review-run-id> --config <repair-author.json>
```

Use the existing author-only execution configuration with a budget account
separate from both the original author and rejecting reviewer. This command
conflicts with `--new-run` and native-invalid `--repair-from-run`. It verifies
the durable review input/result, original author input/result, selected Work,
snapshot, packet, answer and complete review coverage before reserving a fresh
single author call. `APPROVE`, `NEEDS_EVIDENCE`, invalid review results and
reviews of repaired answers are ineligible. No source capture or packet
regeneration occurs.

The repair receives the unchanged saved author payload, complete previous
answer and complete model rejection as untrusted correction feedback. Compiler
citation labels and answer validation remain mandatory. Selected maintained
human context remains complete, attributed and `UNASSESSED`; its `CURRENT` or
`STALE` status describes retained source context rather than semantic truth.
The complete repair request must fit the author cap before reservation; selected
context is never pruned or truncated to fit.

The original author/reviewer records, rendered files and accounts remain
unchanged. Repair output uses a new directory under
`.codeclew/drafts/<work-id>/<repair-run-id>`. Repeating the same explicit selector
and configuration replays its matching child without another author call;
uncertain dispatch does not retry. A later review cannot rewind to that old
rejection. A repaired answer cannot be repaired again in this slice.

A native-valid repair is still an unreviewed, unpublished draft. Explicitly use
`docs work review-draft --source-run <repair-run-id>` for an independent reviewer
call over its exact saved answer, packet and full repair contract. Review
coverage and `APPROVE` remain model judgments, not compiler or execution proof;
human context is not promoted to assessed truth.

The endpoint selector changed in v3, so saved `endpoint-context-v1` and
`endpoint-context-v2` Work fail closed and must be prepared again from their
retained snapshot; source acquisition is not repeated.

The endpoint profile does not infer value propagation or execution order from
call edges. It does not establish wire requiredness, null omission, inherited
field completeness, generic payload types, or annotation activation. Read and
expand the referenced Work items before citing them; a navigation reference in
the packet does not count as a read receipt for its target. The profile is part
of immutable Work identity, and selecting it never acquires fresh source.

Run attempts expose optional `requestBytes`, the complete canonical dispatched
job-envelope size. This covers author, expansion follow-ups, repairs and
reviewer requests. It is not provider token usage or monetary cost. Older
attempts without this field remain readable.

An agent-job configuration may set `"authorOutputContract": "section-summary/1.0"`
for a service Work whose requested entrypoint is `section-entities`. Omission
preserves the generic proposal contract. Unknown contract names and incompatible
Work targets fail before model dispatch. This first contract is deliberately
limited to a declaration section; it is not a general diagram generator.

The controller supplies the fixed target, Work and snapshot identities, a digest
of the delivered pages and a dynamically bound output schema. The author returns
`action: "section"` with a title, summary, evidence handles and optional
uncertainties, or `action: "expand"` with a registered selection. It cannot set
targets, gaps, dataflow, checks or review authority. Unsupported fields are
rejected rather than silently removed. Runtime text limits are UTF-8 byte
limits; schema `maxLength` values alone do not establish runtime admission for
multibyte strings. Whitespace-only text is rejected.

Claim evidence must occur in this particular request's delivered pages and have
a recorded read. Merely listing a source expansion link, or reading a source
concurrently in another process, does not authorize a citation in an earlier
request. Expansion rebuilds the bound schema and complete request budget.
Initial evidence, mandatory obligations and reviewer evidence remain intact;
this output contract does not yet reduce the input context.

The controller adapts the response into the ordinary proposal and runs existing
scope, evidence, freshness and independent meaning-review checks. Repairs and
fallback authors receive the same narrow contract and their previous section.
Run attempts record the emitted contract binding and adapted proposal identity;
the existing job-result records retain raw replies. Invalid author-contract
replies and admission failures do not publish a generation gap or trigger a
replacement capture. Input-cap failures likewise leave publication unchanged.

Structural acceptance is not evidence of semantic quality or lower model cost.
Those require an independent content oracle and measured model usage on real
service inputs.

## Current-format object storage

Documentation roots use SQLite exclusively for immutable payloads. Version 0.10
uses the top-level `codeclew-documentation/2.0` manifest and requires a fresh
documentation root and reindexing. Old roots and serialized
Check, Work, bindings and narrative formats are not migrated or decoded through
fallback readers. Rejection leaves old data untouched; archive it separately if
wanted, then initialize a new root. Do not copy old private state into that root.

Digest, schema and byte-length references identify shared objects. Current-format
snapshots, Work and pins reference the same payloads without copying them.
Payload transactions commit before a root manifest can refer to them. SQLite uses
full synchronous transactions and WAL; database, WAL and SHM are one live unit.
Stop writers and copy the complete root for a filesystem backup.

Git tracks declarations and publications, but excludes `.codeclew` local state.
After cloning a current-format documentation repository, explicitly run
`clew docs init --root DOCS --title EXISTING_TITLE`, then bind its source checkouts
and run `docs check` to acquire new local evidence. Initialization preserves the
tracked declarations and manual files. It only creates local storage when the
entire `.codeclew` directory was absent; it never resets partial or corrupt local
state. A Git clone does not restore private snapshots, Work or pins. Use a complete
filesystem backup when those retained artifacts must survive relocation.

A missing or corrupt selected database fails explicitly. Codeclew does not create
an empty replacement or fall back to loose files. There are no legacy compaction
or migration commands. Documentation history and native runtime caches have
separate lifecycles; compact physical storage does not imply automatic evidence
GC or a disk-usage plateau.

### Remaining implementation boundaries

Consumers now default to saved evidence, but explicit capture can still spend
substantial time obtaining the native project model. Check hydration and runtime
Work still materialize the selected maps. Whole-snapshot service Work retains
broad influence and revision vectors. This patch does not establish bounded
per-query IO, lower native capture cost, complete compiler-input reuse or
evidence GC. Supported declaration-only changes can use explicit recomposition;
changes to source-selection inputs still require matching retained evidence or
an explicit capture. Named pins retain documentation snapshots, not native
compiler sessions or external caches.

The sections below are the **target contract**, not a claim that every listed
command, storage bound or lifecycle mechanism is already implemented. In
particular, `check` acquires on its first page; only continuation pages consume
a frozen report.

## Capture vs. snapshot consumption

- **Capture** is the only operation that runs a compiler/Maven provider. It
  produces immutable evidence (a `snapshot`) pinned to an explicit identity,
  capture time, and input/freshness authority. Capture is never implicit.
- **Consumption** is every read-only command that operates on an already
  captured snapshot: `check`, `context`, `work`, `work read/expand/run`, saved
  `render`, `status`, `history`, `read`, `proposal`, `refresh`. Consumption must
  not launch a capture or trigger a hidden Maven/compiler build. `render
  --refresh` is an explicit capture-and-publish route, not a saved consumer.
- A missing selected snapshot is an explicit error naming the missing snapshot
  identity; it never silently captures.

## Snapshot selector and evidence vector

- A snapshot is addressed by an explicit selector. When a command consumes
  evidence, it uses the selected snapshot's identity, capture time, base
  revision, and freshness authority.
- Pin one evidence vector for a command and its authoring lifecycle: the
  selected snapshot root/version, not a re-captured `latest-check`.
- A snapshot read reports its evidence as *pinned capture evidence*.
  *Newly verified live source freshness* is a separate, explicit
  refresh/capture result. They are never conflated.

## Explicit capture boundaries

- Current-source rendering is available only through the explicit
  `render --refresh` option. It is never the default for a snapshot-consuming
  command, and it cannot be combined with `--snapshot`.
- `check` remains the explicit capture/refresh boundary; consumers that need a
  fresh snapshot must pass an explicit capture/refresh option.

## Storage and integrity (summary)

- Canonical fact/text payloads are stored once by content digest.
- Per-compilation occurrences and snapshot/scope membership are stored in
  bounded, deterministic index pages (cap 256 KiB).
- Publish is atomic: expected previous root/version revalidation under lock
  prevents lost concurrent updates. Old snapshots are immutable and shared
  payloads are retained.
- Reads verify schema, size, digest, and ownership on every open; same-size
  preexisting writes are verified too.

## Limits and accounting

- Per-object reader/writer limit is 128 MiB; required objects above 64 MiB are
  supported via streamed synthetic fixtures, never a sparse binary.
- Integrity checks are never weakened for size. Oversized/corrupt required
  payloads preserve the previously published index/root and readable
  publication.
- Inventory reports category bytes/counts, roots, retained closure, leases,
  owned attempts, and conservative reclaimable estimates. Logical vs allocated
  sizes are reported separately.

### Exact section-review output contract

For `section-summary/1.0`, reviewer requests include the exact output schema and
its digest. The schema binds the current Work, proposal and evidence digest;
coverage arrays contain the complete set of claim and operation ID strings.
Issue evidence uses delivered Work handles, not long source IDs. A reviewer can
return a review or request registered expansion. The controller still validates
closed fields, bindings, complete coverage and verdict consistency; supplying a
schema does not make a model verdict correct or repair malformed output.

### Explicitly select a new paragraph source context

Text edits preserve their effective source snapshot and maps. To move an existing
user-authored paragraph to a current saved Check, submit the separate closed
`explanationContext` instruction through the same manual proposal route:

```json
{
  "kind": "RETAINED_OPERATION",
  "id": "<operation ID>",
  "recordDigest": "<exact canonical retained Operation digest>",
  "target": "explanationContext",
  "fragmentId": "<paragraph ID>",
  "expectedParagraphDigest": "<exact canonical Explanation digest>",
  "expectedContextDigest": "<effective context digest>",
  "contextEditor": "Declared context editor",
  "sourceReferences": ["<current SOURCE Work reference>"],
  "dependencyReferences": ["<current DEPENDENCY Work reference>"],
  "anchors": [{"eventId": "<current event ID>", "expectedEventDigest": "<canonical Event digest>"}]
}
```

The effective context digest hashes the canonical JSON array
`[sourceSnapshot, sourceRefs, dependencyRefs, contextRole, contextMigration]`
from the old paragraph's authorship, using `null` when migration metadata is
absent. The paragraph digest covers the entire old Explanation. Both digests
must match the immutable retained operation read through
`docs work read-retained-part`. Read every selected current SOURCE with
`docs work read-part`, including small sources, until `nextCursor` is null.
Read all selected current DEPENDENCY handles through recorded Work reads.
An inline SOURCE preview or an ENTRYPOINT handle does not establish a full
source read for this instruction.

This first migration route preserves the exact logical source and dependency
sets. It checks compatible source association, dependency kind, provider and
compiler scope. Anchors must cover every existing paragraph event, match its
exact current Event digest, and include all anchored evidence. Current event
fragment bindings must match the Work Check; regenerate stale source-derived
fields before requesting migration. ID remapping, changed scopes, missing
anchors and arbitrary source records are unsupported.

The instruction preserves paragraph text, ID, anchors, detailed-view setting,
text author and original text edit digest. It changes only effective source
snapshot/maps and adds `contextMigration` with a declared editor, instruction
digest, previous snapshot/context digest and `contextReview: UNASSESSED`.
Publish the prepared proposal with `docs proposal publish --proposal ID
--unassessed`. Exact stored manual instructions are required; direct narrative
metadata cannot forge a migration. A text edit and context selection cannot
share a paragraph in one proposal. Later text edits preserve migration metadata
and effective context. Subsequent source changes make that context stale without
silently moving it again. Current context freshness establishes neither semantic
review of text nor review of the selected context.


## Compare a saved answer with an explicit captured Check

`clew docs work answer-context --root DOCS --work WORK_ID --review-run REVIEW_RUN --snapshot NEW_CHECK`
is read-only. Select an exact saved approved answer and a Check produced by an
explicit source check. There is no latest fallback, capture, provider call,
publication, or promotion of the old model verdict. The result preserves the
original question/root/profile and labels meaning
`MODEL_APPROVED_AGAINST_SAVED_PACKET` separately from captured-context freshness.

`CURRENT` means the complete selected packet SOURCE and DEPENDENCY content/pins
match the compared capture. This includes selected records the answer never cited;
it excludes unrelated global Work influence. Revision, source-link and coordinate
metadata can appear in `linkChanges` without staling exact content and stable
compiler bindings. Changed compiler pins are never heuristically remapped to claim
equivalence. Changed selected content/pins report `STALE`. Missing records,
retained-only/unselected service evidence, changed captured service declaration
(including configured repository and source-root origin), unsupported identity/scope
bindings or changed capture coverage report `UNKNOWN`, with explicit missing/change reasons.
The result compares with the supplied Check, not live runtime or a later source
state. Human/imported maintained prose remains unverified and UNASSESSED.

Saved answer/packet/review/provenance files and historical publications stay frozen.
A different question or scope requires new Work and cannot inherit this saved
approval. The comparison creates no new cache, review or publication baseline.

## Find an approved answer by exact request (source candidate)

```sh
./clew docs work find-answer --root DOCS --subject service:ID \
  --input question.json --snapshot SAVED_CHECK
```

This source candidate is separate from release 0.13.15. It searches immutable
Work manifests and every historical approved run, without needing saved Work or
review IDs. Matching uses the complete normalized request: question, audience,
language, root declaration, profile, authoring contract, context bounds and
external-input selectors. Similar question wording does not match.

The first supported shape is a method question using `process-graph-v1`,
authoring contract `codeclew-operation-draft-authoring/1.6`, and
`sourceDataContext: true`. It requires no actual author or reviewer expansions,
repair, maintained context or admitted external inputs, and absent protected
notes. Other retained approvals remain historical; unsupported chains never
become reusable by default.
This first boundary supports native capture. Portable evidence expectations or
`EVIDENCE_PACKAGE` producer authority in either saved or compared service are
explicitly unsupported.

The lookup compares complete selected source/compiler bindings and replays the
initial graph selection, including membership, unavailable targets and derived
source-data context. A new matching callee can invalidate a saved answer even
when its old cited records are unchanged. A read-only protected-notes check
detects newly present notes; the compared Check alone does not freeze all notes
membership. Historical answer bytes, citations, snapshot and model approval
remain unchanged. `CURRENT` describes applicability to those explicit inputs;
it is neither new meaning review nor live runtime verification.
Validated receipt/revision changes can appear as link changes only after the
complete initial semantic replay matches. Any changed delivered source fragment
still invalidates reuse, including unrelated code inside a delivered class.

`FOUND` returns `selected.answer` with its historical review and provenance.
`SELECTION_REQUIRED` returns eligible candidates without ranking them. Save one
candidate's `selection` object and supply `--select selection.json` to choose
explicitly. The closed selector binds the exact request and compared snapshot.
`NO_MATCH` and `NO_REUSABLE_MATCH` give a next action and applicability reasons.
Missing or corrupt bound records fail the search rather than silently becoming
an empty or uniquely matched result. Lookup performs no capture, model call,
Work preparation, export or publication; acquire new evidence separately.
Zero writes refers to durable Work, job, review, publication and source records.
Existing object-store reads and locking may still perform operational filesystem
IO; this command does not introduce a separate read-only storage implementation.
