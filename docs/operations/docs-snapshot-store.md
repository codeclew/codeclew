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
`READABLE_NOW_CURRENT_READER` and source freshness `UNVERIFIED`. The separate
`checkStatus` reports `CHECKED` or `UNRESOLVED`; a readable snapshot can contain
failed source acquisition. `unresolved` preserves the recorded reason and next
action, while `services[ID].entrypoints` counts captured entrypoints. The
`manifestBytes` field and the snapshot handle's `/size` suffix count serialized
manifest bytes, not entrypoints. Selecting a service whose capture failed reports
that snapshot's recorded cause; a service omitted from capture is identified as
`SERVICE_NOT_SELECTED`. Repair the cause and explicitly run
`docs check --service ID`, or select a known snapshot containing the service. This does not
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

`docs check` is an explicit evidence acquisition command. Its response includes
an immutable `sha256:<digest>/<size>` snapshot handle. Save that exact value;
it remains readable after another check replaces the latest-check pointer.
Named snapshot pins retain handles explicitly.

```sh
clew docs context --root docs --service orders --snapshot "$snapshot"
clew docs work prepare --root docs --subject service:orders \
  --input request.json --snapshot "$snapshot"
```

Work, context, changes and process reads consume saved evidence without implicit
acquisition. A selected service can reuse a full multi-service snapshot. The
snapshot remains immutable; narrowing Work does not imply service-local hydration.
Read cursors bind the snapshot and selector. Resume with the returned snapshot;
never substitute a newer latest-check pointer.

Consumers verify immutable objects and applicable declaration bindings. Missing
or corrupt required evidence is an error, not permission to recapture. Historical
validity does not prove current-source freshness. Native document rendering and
status-page refresh commands are retired. New documents follow the
[authored-originals route](../../skills/codeclew/references/compose-original.md#frozen-source-to-reviewed-pages).

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

`docs refresh-contracts --root DOCS --snapshot ORIGINAL_CAPTURE --service ID`
explicitly reads registered OpenAPI files from their full saved source records
with the current bounded reader. It returns a new immutable derived snapshot;
pass that handle to context, Work and render. Repeat `--service` for up to eight
captured services. Each selected service must have 1..128 registered contract
files totalling at most 8 MiB. Missing or partial saved files fail without reading
the checkout, running an analyzer, restoring packages or fetching references.

Contract refresh preserves the original source records, revision and capture
authority. It changes contract projections and contract-reader diagnostics only;
it does not establish current source or runtime verification. Historical snapshots,
publications and the latest-check pointer stay unchanged. Catalogue compatibility
and the original-capture parent rules above also apply. After editing process or
interaction declarations, run refresh again from the original capture to keep the
refreshed contracts in the next derived snapshot; ordinary recomposition preserves
the original capture's contract projection. Existing derived snapshots continue to
read their saved projection rather than being silently reinterpreted.

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

<a id="exact-per-file-syntax-reuse-source-candidate"></a>

### Exact per-file syntax reuse

Version 0.13.17 includes a second reuse boundary inside syntax capture. Successful extraction is keyed by service, relative path, exact
file bytes, language/dialect and the complete bundled producer, including its
parser inputs and pinned Rust toolchain. Equal content at a different path or in
another service does not reuse that result. Whole-capture admission also binds
the producer, so an older keyed capture cannot bypass its changed identity.

Replay rebuilds Source occurrences against the current revision, snapshot, blob,
ranges and URL. It preserves inventory, FILE_ONLY sources, annotations, coverage
and overlapping aggregate source/fact budgets; a hit does not admit a larger
scope. Failed extraction and parse-error trees are never stored in the per-file
cache; they require parsing again when a later capture runs per-file extraction
rather than reusing a complete keyed capture. Corrupt stored bindings fail
closed. This is syntax extraction
reuse, not compiler-result reuse or a source-currentness certificate.

The [bounded capture-cost measurement](field-documentation.md#incremental-capture-cost-source-candidate)
found 21.863% incremental native capture/Check savings on one worker corpus after
cache seeding, with complete evidence/Check equality and retained partial
coverage. Startup, changed-file parsing and IO still cost time. It is not a
general CLI, compiler or documentation-task speedup. Exact per-file syntax reuse
is included in v0.13.17.

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

## Bounded answer authoring and review

Use `docs work run --draft` for configured finite Q&A and
`docs work review-draft` to review a saved answer. See the
[current execution and recovery contract](../../skills/codeclew/references/service-documentation.md).
The retained outputs are answer, packet and audit JSON, under the individual run.
Existing exports are never rewritten. Failed calls, durable identities and usage
accounting remain part of the record.

These operations do not publish HTML, Markdown or diagrams. The native section
coordinator, answer exporter and reviewed-answer publisher are retired. To make
a document, read the saved Work and author originals using the single checked
publication route. Do not change old Work identities or delete failed attempts.

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

<a id="find-an-approved-answer-by-exact-request-source-candidate"></a>

## Find an approved answer by exact request

```sh
clew docs work find-answer --root DOCS --subject service:ID \
  --input question.json --snapshot SAVED_CHECK
```

Exact approved-answer discovery is included in v0.13.17. It searches immutable
Work manifests and every historical approved run, without needing saved Work or
review IDs. Matching uses the complete normalized request: question, audience,
language, root declaration, profile, authoring contract, context bounds and
external-input selectors. Similar question wording does not match.
Keep the original request file and exact subject. The compared Check is a
separate required immutable handle, not a latest pointer or an implicit capture.
Request normalization resolves the same preparation defaults; changing audience,
language, bounds or selectors can change the match even when question text stays
equal. For a complete executable sequence, see the [method-answer route](field-documentation.md#capture-question-review-and-exact-reuse).

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

`FOUND` returns `selected.answer` with its historical review and provenance; one
`CURRENT` candidate is selected directly.
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

To choose among eligible approvals, copy one candidate's complete `selection`
object into a JSON file and rerun:

```sh
clew docs work find-answer --root DOCS --subject service:ID \
  --input question.json --snapshot SAVED_CHECK --select answer-selection.json
```

The selector freezes subject, request digest, compared Check, Work and review run.
It cannot carry a choice over to another request or Check. Unsupported historical
approvals remain visible with `UNKNOWN` applicability; explicit selection cannot
make them current. Search validates all known historical report/Work bindings,
including history outside the matching request, before claiming completeness.

Locate the corresponding candidate by matching its `selection` to
`selected.selection`. Inspect that candidate's `selectedEvidence.linkChanges`
for validated source receipt or presentation movement and
`initialPreparationReplay` for the selection
comparison. Read current source records using the explicit compared Check;
historical answer citations, exports and published reader links stay frozen.
`CURRENT` allows using that historical answer with a separately stated current
applicability result. It creates no new approval, source capture or publication.
For changed or unknown inputs, prepare/review a replacement against explicitly
acquired evidence and use the existing [publication baseline and admission](#review-a-saved-operation-draft)
when a catalogue update is intended. Generic service narratives continue through
their Work/proposal path.

<a id="compatible-model-id-representation-source-candidate"></a>

### Compatible model ID representation

Version 0.13.17 includes the frozen
`modelRepresentation: "codeclew-model-ids/1.0"` option for native 1.6 method
authoring and independent review, and recommends compact
`modelRepresentation: "codeclew-model-ids/1.1"` for compatible new drivers;
existing 1.0 representations remain supported unchanged. The
[serializer contract and public Rust example](model-id-serializer.md)
require unchanged canonical validation before forwarding only the frozen model
payload and strict schema. Typed maps remain host/driver metadata; expansion
extends a role's map without renumbering, and the reviewer has an independent
scope. Canonical saved answers/reviews remain compatible with discovery, but an
actual expansion still falls outside the initial exact-reuse boundary above.

Before typed alias projection, 1.1 checks the complete delivery presentation
against the unchanged native presenter over canonical pages and source parts,
including its duplicate-row retention links. It removes only raw `pages` and
`sourceParts` siblings under the model payload's `packet.contextDelivery` and
`reviewContext`. Complete presentation, receipts, citations, bindings and the
canonical raw archive remain. `savedAuthorContract` is outside this removal
scope. A missing presentation keeps raw delivery; a mismatched or incomplete
presentation fails closed. Drivers must consume the presentation when raw
siblings are absent. Frozen carriers and recovery records validate by their
saved version; selecting 1.1 cannot rewrite or migrate an existing 1.0 run.

The [paired compact measurement](../plans/question-results-and-incremental-evidence.md#compact-role-evidence-representation-candidate-2026-10-06)
uses a generated grouped-read fixture and supported 1.0/1.1 model forms for the
same canonical jobs and scopes. Across all four initial/expanded role contexts,
combined prompt plus strict-schema reference counts were **163,106 to 104,051:
36.2065% fewer than 1.0**, including actual protocol overhead. The expanded pair
alone fell from 113,714 to 69,559 (38.8299%): author counts were 39,274 to 24,818
and reviewer counts were 74,440 to 44,741. The initial author grew by 18 tokens
(0.2547%). This comparison did not repeat dispatch for an invocation and does not
compare compact input to a canonical baseline. These are reference text measurements, not
general-corpus savings, provider usage/billing, quality or verified delivery.

The earlier [two-retained-first-call measurement](../product/validation/model-id-alias-feasibility.md#supported-native-candidate-2026-10-06)
remains a separate 1.0 result: 4.9184% fewer combined reference tokens against
canonical input. Complete carrier bytes, including canonical input and host-only
map, still participate in conservative admission and reservation for both modes.
Both modes are included in v0.13.17. Historical v0.13.15 contains neither mode.
