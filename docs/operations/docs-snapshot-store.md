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

New work records use `codeclew-documentation-work-manifest/1.0`: an immutable
`evidenceSnapshot` handle replaces the embedded Check. Requests, authoring
inputs, retained prose, handles, influence and obligations remain bound to the
work identity. The manifest schema and digest are checked before its evidence
is opened; an explicit historical `snapshot` must match `evidenceSnapshot`.
Missing or corrupt evidence fails without source acquisition or latest fallback.

Legacy inline `codeclew-documentation-work/1.0` records keep their original
identity and remain readable. Legacy Work with no pinned `snapshot` cannot
continue through proposal validation or start an agent run: it reports
`LEGACY_WORK_REQUIRES_REPREPARE`. Prepare new Work from a saved snapshot; merely
having an `evidenceSnapshot` field does not silently grant historical authority
to older records. Existing accepted run status remains readable. The 64 MiB Work-record limit now applies to the
manifest metadata, not to the hydrated Check. This does not remove bounds on
individual immutable evidence objects or on authoring metadata.

An agent run reuses its verified Work for initial and expansion pages, avoiding
reopening the same snapshot for each page. Reads in a new process still hydrate
the full Check. Handles/influence scale with the selected analysis; they are
not yet a constant-size query result. Page limits remain independent of the
complete model-request budget.

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
