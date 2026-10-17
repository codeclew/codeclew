# Prepared SOURCE answers

Use this entry for a supplied docs root, service, exact selector and snapshot or
saved answer packet. Do not load the full managed-code or service guide. Use
absolute task paths and resolve the supported installed/candidate `clew` once.
For supplied prepared snapshots, start with the packet or engine command below;
no preliminary Git identity, status or hash checks are needed. For maintenance,
pass the saved full packet straight to `compare` below. Do not prepare by reading
its metadata or the helper implementation. Reading your saved answer and this
short entry remains useful; engine/helper packet validation is internal.

The engine admits immutable snapshot objects and their source/configuration
bindings. `PINNED_SNAPSHOT_NOT_REVERIFIED` means no live checkout recheck; it does
not invalidate an admitted immutable revision. Integrity, target freshness and
syntax/compiler authority remain separate. No documentation publication is needed.

## First answer

Read an already supplied engine-retained packet locally. Otherwise fetch once
against the supplied exact snapshot. The helper invokes
`docs evidence retain-source`, keeps the full packet private and emits a bounded
view. Its `evidenceSet` is an opaque engine-owned reference; carry the saved packet
forward. Do not rewrite its SHA, digests or history in prose or hand-edit it.

```sh
python3 -I -S SKILL/scripts/prepared_source.py fetch --clew CLEW --root DOCS --snapshot SNAPSHOT --service SERVICE --symbol EXACT_ID --output ANSWER_PACKET.json
```

If the answer uses code outside the captured language/scope, first discover the
necessary files once and explicitly register them with `--native-inputs ABS_JSON`.
The JSON is an array such as:

```json
[{"service":"dashboard","files":["services/places/PlacesController.cs","services/places/PlacesRepository.cs","services/places/Program.cs"],"membership":["services/places"]}]
```

Here `service` names the captured repository/revision binding, not an inferred
semantic owner. Native files are read at that immutable revision. Registration
tracks exact bytes for listed files and names within membership prefixes. It
provides no C# semantic analysis, deployment proof or implicit dependency closure.
Register every native file used by the answer; unregistered references remain
`UNKNOWN_OUT_OF_SCOPE`. Initial native discovery/read outside coverage is allowed.

## Maintenance

Start with one `compare` using the supplied packet and exact target snapshot.
Use the same managed docs root containing the prepared baseline and target
snapshots. Preparation may register and capture multiple exact revisions there;
carry only the arm's own answer/packet artifacts between stages. Do not copy or
edit private cache state to transfer evidence. A missing retained object is an
explicit error, never an unchanged verdict.

```sh
python3 -I -S SKILL/scripts/prepared_source.py compare --clew CLEW --root DOCS --input PREVIOUS_PACKET.json --snapshot TARGET_SNAPSHOT --output CURRENT_PACKET.json
```

Default compare stdout is a self-contained bounded maintenance view: exact
baseline/target bindings, typed decision, before/after values and windows, citation
updates, changed SOURCE only, coverage and unresolved inputs. Unchanged SOURCE
bodies are not replayed. The full current packet stays at `--output` for the next
stage; do not compute, transcribe or rewrite its SHA or opaque evidence set.
No preliminary packet `read` or optional widening is needed for the ordinary path.

The engine compares the registered dependency closure, source/configuration,
membership, generated inputs and explicit native references. It preserves
unresolved analysis limits. Read the typed result:

- `UNCHANGED`: reuse the answer within its previous scope and limits. Add only a
  short checked/unchanged annotation; use returned binding updates for citations.
- `CHANGED`: inspect the engine's concrete reasons, changed-file window and target
  SOURCE. Update affected clauses. A conservative watched-file change is not a
  claim of changed runtime behavior; an unrelated page-title change can leave the
  selected function unchanged. Check the title delta in the returned evidence.
  `changedWindow.beforeCitation` and `afterCitation` already bind each retained
  source span to its revision, file, lines, authority and URL (when configured).
  Use those citations for the window; they identify the retained source containing
  it, not a claim that the projection is a complete source. No second baseline
  context lookup is needed. Stop acquiring evidence once the answer and citations
  are supported within the reported scope.
- `UNKNOWN`: name the missing input/scope. Do not claim the old answer is verified
  unchanged or suppress unresolved evidence. Acquire only the missing evidence.

After successful engine admission/compare, **do not duplicate covered-evidence
checks with `git show`, `git diff`, `git rev-parse`, manual source hashes or
catalogue comparison**. Git/native reads remain allowed for initial acquisition
outside registered coverage or diagnosis of a concrete contradiction. Engine
internal Git access implements the admitted contract; agents do not repeat it.
Equal handler text and unresolved call targets do not prove full dependencies or
unchanged behavior. Preserve the original semantic and runtime boundaries.

For authorized recapture, `fetch --refresh` replaces `--snapshot`; these are
alternatives. It refreshes only the selected registered service at its declared
ref. If the ref must change, preserve the existing service-record compare-and-swap
workflow. Prepared target snapshots need no installation, registration, check or
refresh during answering. Refreshing a caller does not reverify receivers unless
its registered native inputs are also acquired at the correct repository revision.

## Bounded evidence, full packet local

Use the recipes here; reading the helper implementation is not a preparation step.
Read saved packets locally without another CLI call:

```sh
python3 -I -S SKILL/scripts/prepared_source.py read --input PACKET.json
```

Default stdout is at most 16,384 UTF-8 bytes for maintenance and 12,000 for SOURCE views. Fetch/drill views select up to three
SOURCE records and four handles; saved comparison packets automatically use the
maintenance view. Authority, coverage, engine comparison/bindings, omitted evidence and
cursors remain visible. Full packets and private stderr logs are never replaced.
Do not dump, pretty-print or truncate raw packets as a substitute for this view.
The local packet fingerprint is not an additional freshness verdict.

`textDelivery.complete=false` identifies an excerpt. Continue one exact SOURCE
locally with `--source ID --text-offset N`; `--text-chars`, `--max-sources`,
`--max-handles` and `--max-bytes` bound the view. `--text-chars 0 --max-handles 0`
shows metadata only. `--metadata-only` or `--max-sources 0` are also supported;
these omit SOURCE bodies while keeping changed values/windows, citations and limits.
SOURCE_ALIAS bodies and unchanged normalized catalogues stay local. A maintenance
`LOCAL_MAINTENANCE_RECORDS` continuation uses `read --input PACKET.json
--record-offset N` locally; `LOCAL_SOURCE_TEXT` uses the exact source/offset below.
A `LOCAL_PACKET_RECORD` uses `read --input PACKET.json --record /items/N`.
A missing source can provide a `PINNED_SOURCE_DRILL` with exact snapshot/service/ID.
Limits are never discarded to fit: a tiny budget returns an explicit error with
no evidence. Continuations/omissions do not establish completeness.
Changed windows carry their own before/after citation metadata even when their
source IDs are absent from the selected SOURCE rows. Do not use `read --source`
for such absent rows just to recover a citation. Changed windows are explicitly
projections; complete SOURCE retains its own
bindings. Only when required captured SOURCE text is missing, drill its exact ID
at the required snapshot (including an old snapshot after catalogue advancement):

```sh
python3 -I -S SKILL/scripts/prepared_source.py drill --clew CLEW --root DOCS --snapshot SNAPSHOT --service SERVICE --source SOURCE_ID --output SOURCE_PACKET.json
```

The engine admits the immutable snapshot and captured configuration, not the
current catalogue or checkout. Integrity failures still reject the read. The
helper retains the raw packet privately and bounds stdout just like `read`.
Stop after the required answer and citations are supported; do not fetch the old
source merely to repeat a ready `beforeCitation`. `compare --include-context`
retains current SOURCE in the full packet
also for unchanged inputs; request an exact source only when it is needed. A helper budget error delivers no evidence; inspect
its retained log or request a bounded larger view instead of suppressing limits.

Report an answer delta or short unchanged annotation, not a replay of prior
answers and error history. Evidence is private unless transfer to a specific
recipient is authorized; the bounded projection is not a public-safe report.
