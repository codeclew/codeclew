# Compose-original pages: initial and update

## Frozen source to reviewed pages

When the caller supplies a frozen root/service/snapshot and requests human pages,
use one installed CODECLEW_SKILL_DIR and the admitted CLEW launcher. Keep a running
experiment on its admitted runtime. Native Work/proposal deliverables, capture,
registration and bounded SOURCE Q&A keep their existing routes in SKILL.md.

Source facts MUST come through supported APIs. Use the supplied frozen root;
do not clone it again. Failure is not permission to bypass clew with shell/file
tools on target source, GitHub/web/raw URLs or private-store/cache access, unless
the user explicitly authorizes that fallback. This is agent guidance, not
OS/filesystem sandbox enforcement. Ordinary product implementation follows its
repository workflow. Read saved helper artifacts through the supported inspector.

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/evidence_packet.py" discover-files --clew CLEW --root ROOT --service SERVICE --snapshot SNAPSHOT --prepare-work --output-dir DISCOVERY
```

Inspect returned file metadata and coverage, then write an explicit selection
array `[{"file":"RETURNED_PATH"}]` covering the requested task. No fixed file
allowlist or semantic pruning is implied. The helper drains native pages; failed
reads and unresolved/omitted coverage remain visible. For routine inspection
against context already received intact, use the existing repeat-reference option:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/evidence_packet.py" read-files --clew CLEW --root ROOT --service SERVICE --snapshot SNAPSHOT --packet DISCOVERY/packet.json --selection FILES_JSON --whole-files --output-format metadata --output-dir READ
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/author_citations.py" inspect-grouped --manifest READ/citations.json --request VIEWS_JSON > grouped.json
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/author_citations.py" display-grouped --input grouped.json --against RECEIVED_CONTEXT.txt --prior-complete
```

Metadata delivery saves the full packet, source bodies, original handles and
receipts without displaying bodies twice. VIEWS_JSON is an author-selected array
of `{"file":"RETURNED_PATH"}` or exact `startLine`/`endLine`, optionally `cite` for
an ambiguous file. It returns verbatim original views, not new evidence. Missing
whole-file proof refuses; no fallback or inferred whole-file coverage. Existing
declaration read, source-union, reuse-read and full inspect remain available.

For contracts/configuration exposed by a native dependency's `sourceReferences`,
read the returned reference through `docs work read` to obtain its exact SOURCE
ID. It need not be a SYMBOL or belong to the file-inventory profile. Select
`[{"sourceId":"RETURNED_SOURCE_ID","sourceReference":"RETURNED_REFERENCE"}]`:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/evidence_packet.py" read-sources --clew CLEW --root ROOT --service SERVICE --snapshot SNAPSHOT --work WORK --selection SOURCES_JSON --output-dir READ
```

This route re-reads native reference pages and verifies complete source parts,
including explicit size-omission recovery, then writes ordinary `citations.json`
for the same inspect/compose/bind/finalize commands. It accepts IDs/references,
never imported context JSON; Work/snapshot/service/revision must match. Original
authority, occurrence and file/line metadata remain intact. Selection completeness
does not enumerate every source or prove whole-file capture, compiler semantics
or runtime conformance. `--output-format metadata` and `--source-union` keep their
existing meanings; prior-packet reuse is not part of this route.

For a supplied engine-retained `SOURCE_NATIVE` packet, import its registered
bytes without recapturing the repository or converting them into Work handles:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/author_citations.py" import-native --input PREPARED_PACKET_JSON --clew CLEW --root DOCS --output-dir NATIVE_READ
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/author_citations.py" inspect-grouped --manifest NATIVE_READ/citations.json --request VIEWS_JSON --clew CLEW --root DOCS > grouped.json
```

Import emits compact file/citation metadata; inspect only needed files or exact
line ranges, then use `display-grouped --input grouped.json --clew CLEW --root DOCS`
for plain source. The full prepared context is retained locally and not replayed
to the author. Use the real returned cite handles in original fragments.
Compose stays unchanged. Bind/check/finalize/export accept both Work and native
manifests on the same page; repeat `--manifest` and explicitly pass the trusted
`--clew CLEW --root DOCS` when any native manifest is present:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/reader_export_check.py" finalize --compose-root COMPOSE --manifest WORK_READ/citations.json --manifest NATIVE_READ/citations.json --renderer-command RENDERER_JSON --clew CLEW --root DOCS --output-dir FINAL
```

Every consuming process reloads the native evidenceSet through supported
same-snapshot `docs evidence compare-source --include-context` and checks exact
native records, not saved receipts or edited JSON. The supplied launcher/root
are the caller's trust anchor; manifests never choose an executable. Missing
engine objects or mismatched bytes, identity, ticket, repository configuration
or revision refuse. Validated reloads may be shared within one process only.
The current service definition must still match the snapshot's analysis
configuration; a changed definition refuses rather than guessing a repository.

Native sidecars preserve `SOURCE_NATIVE`, bytes-only authority, binding service,
repository, revision, Git blob/range, text digest and evidenceSet. They contain
no invented Work/sourceReference. A compiler file-comparison `UNKNOWN` can
coexist with exact registered native bytes; the full comparison remains in the
sidecar and is not an unchanged-behavior or compiler-completeness verdict.
One binding service is supported per native manifest; use separate supplied
packets for other services. Native source-union is unsupported. Explicit link
presentation may use the same bound repository, never reassign its identity.
No import or checked export establishes semantic truth, execution or publication
approval, and human prose remains separate from these metadata sidecars.

`display-grouped` rechecks the saved result against its original manifest and
shows verbatim code with file/line/raw-citation headings and source gaps; the full
JSON/proof stays unchanged. No display script or JSON-schema investigation is
needed. When that delivery premise is known, use `--against` as above rather than
displaying the same source again merely to recover its location or handle. Repeat
`--against` for separate intact deliveries. If prior delivery is unknown or
incomplete, omit both flags: default output is complete. Only identity-linked
fenced bodies match; changed/partial/unknown/truncated prior content keeps the
full body. This is your display premise, not proof a model read or understood it.
Use `--reread` for full text even with `--against`. Keep enclosing output limits
sufficient; optional prior references do not make incomplete delivery complete.

## Complete delivery and retained IDs

Deliver intact source groups; keep verbose evidence identities and resource inventories
in sidecars. Size the enclosing response, including nested output, labels and
JSON escaping, against its actual budget; a child limit does not enlarge its
parent. Safe batching is allowed. Keep bodies/lines intact when they fit; split
only oversized results at meaningful boundaries with explicit continuation.
Save grouped results unchanged; show decoded verbatim code with short file/line/
cite headings. Use ordinary output tools, not a new transport or fixed byte cap.

Read a supplied compact range map before a routine source fetch. Distinguish
available, prepared and actually delivered content. Mark a context group received
only after all its parts are visibly complete; a truncation marker leaves the
missing portion open. Recover the exact missing part/range. File existence,
helper success or an accurate sidecar alone does not prove complete delivery.
Short author notes suffice; no delivery database is needed.

Use `inspect-grouped` for additional ranges or an unresolved question, and also
when deliberately revisiting evidence. Identical ranges may share a returned
fragment; distinct file/revision/authority identities remain separate. Do not
fetch a whole source again merely to find a cite or location already in the
lookup map. Intentional rereads and author-local materialization remain allowed;
there is no single-read rule or semantic restriction on available resources.

Write page-to-source choices with stable page-local IDs and the exact returned
handles in `evidence`. Existing compose/bind/check helpers expand immutable
provenance: do not type its unchanged source records or print an entire generated
`sources.json` for routine validation. Keep full artifacts inspectable; check a
specific association through its stable ID when needed. Already authored Markdown
can use the existing manual selector route without rewriting it into fragments.

Full reader scope, consequential source distinctions and the final complete
source-to-document/document-to-source self-review remain required. Use the
existing full finalize/export/link/binding checks after that review. A material
page/source/renderer change requires the relevant check again; retaining IDs or
reading smaller carriers is not a substitute for review or final verification.

Use the full reader request to choose evidence. Follow referenced configuration,
contracts, resources and helpers when their contents change the explanation:
a retained file is not a read file. Use supported reads and retain their manifests.

In those inventories, distinguish bulk example payloads from descriptive resource
metadata. Read relevant notes about data origin, defaults, units or interpretation
before explaining that data or behavior. Small explanatory metadata can matter
even when large examples need only bounded inspection. Use descriptions and
references to choose; size alone does not establish relevance. Available-but-unread
resources are not understood evidence; reading every bulk example is not required.

When first explaining behavior, data roles and consequences, preserve source
conditions and boundaries that change what the reader should understand: what
results, when an action happens or is skipped, and what failure changes. Keep
confirmed errors, retries and asynchronous or completion boundaries distinct;
do not collapse outcomes into a generic statement. Explain each material
distinction once, linking to it elsewhere; keep incidental implementation
mechanics out of reader text.

Do not replace requested explanations with machine evidence or the author report,
or unknown/not-established inventories. Never use prior prose as source evidence.
Preserve the original clew bindings, revision/file/ranges, handles, failures and
SOURCE/compiler/runtime distinctions; extend evidence when explanations expand.
Packet consistency does not authenticate a bundle or prove completeness/truth.
Keep consequential uncertainty in the explanation and full provenance for agents,
without repetitive disclaimers or reviewer checklists in human pages.
When the retained source contradicts the draft, rewrite the contradicted visible
claim before READY; adding a generic caveat does not repair the claim.

## Original pages and completion

For preselected factual schemas/storage declarations that must survive authoring,
use [mandatory retained declaration components](declaration-components.md).
Their typed placement and independently pinned ticket/digest are separate from
the ordinary authored fragments below; never transcribe their cell values.

Use this route for author-local linear, disjoint text/evidence fragments:

```json
{"pages":[{"page":"guide.md","fragments":[{"text":"# Guide\n\n"},{"text":"[Next page](domain.md)\n\n"},{"id":"effect","text":"Explanation.\n","evidence":["RETURNED_HANDLE"]}]}]}
```

Plain headings and navigation use only `{text}`. Substantive bound fragments use
`{id,text,evidence}`: `id` identifies a claim and `evidence` lists its returned
handles. Including either `id` or `evidence` requires both, even for a Markdown
link. Changing Markdown syntax does not repair a missing annotation field.
`UNPAIRED_COMPOSE_ANNOTATION` reports the page, zero-based `fragmentIndex`, JSON
pointer and missing field. Choose the appropriate fragment form explicitly;
the helper never removes annotations or invents evidence.

For updates, read current result paths for the author root, original, authored
root, checked index, manifests, renderer, export and receipts. Resolve relative
paths under their recorded root; no journal dump is needed. Keep all current
pages and stable page-local IDs. Reconcile human Markdown edits into the original
first; never replace them from a stale copy.

Before drafting, choose a reader structure appropriate to the question and saved
evidence. Keep concise pages concise; a short overview needs no mandatory template.

- Overview: purpose, responsibilities, inputs/outputs and useful next pages.
- Domain data: authored GFM tables for comparable named fields, one conceptual
  value per row. The Field column contains the exact evidenced source/code
  identifier, in code formatting and original casing. Put translations and business
  labels in the meaning/use column, not in place of identifiers. Keep evidenced
  code/DTO/storage aliases together (for example, `place_id` → API `id` in one
  cell), without duplicate rows. If an identifier is unavailable, explain that
  limit in prose instead of guessing a name. Explain evidenced meaning, units,
  time interpretation, nullability and use; a name alone is not a definition.
- Processes: trigger/input, ordered work, consequential branches and outcomes.
  Show what changes or is sent, when a branch happens or is skipped, and what
  failure, retry, completion or commit/acknowledgement changes. Use titled branches
  and concise outcome comparisons where useful, without a universal stage count.
- HTTP/integrations: direction, endpoint/topic, payload/result and important
  errors/timeouts. Distinguish declared contracts from visible runtime behavior.
- Presentation: use meaningful headings, short paragraphs, numbered ordered work,
  bullets for parallel alternatives and tables for comparable items. Long pages
  benefit from local contents links. Keep primary behavior and key failures visible;
  disclosures can hold detailed schemas/configuration. Avoid dense tables that
  split identifiers into arbitrary letter fragments; combine related labels or
  choose a simpler structure. Do not impose character caps or forced tables.

Fragments concatenate literally. Author explicit paragraph boundaries between
independent prose blocks; keep adjacent table rows and retained components intact.
Never add blank lines between every fragment, guess structure from arbitrary
paragraphs or transcribe retained component values. Preserve every claim's source
associations through the existing compose/bind/export workflow.

The original is the authoritative input. Generate Markdown with the supported
compose command; do not maintain a separate final Markdown draft. Review the
actual compose/render output. A table/list-to-prose transition needs an explicit
block boundary; adjacent table rows do not. For example, the last row ends with
two newlines before the next prose fragment:

```json
{"text":"| Field | Meaning |\n| --- | --- |\n"}
{"id":"identity","text":"| `id` | Event identity. |\n\n","evidence":["RETURNED_HANDLE"]}
{"id":"commit","text":"Commit precedes acknowledgement.\n","evidence":["RETURNED_HANDLE"]}
```

Before finalization/READY, self-check request-to-source-to-reader meaning using
the existing entry/body/resource inventory and your draft: can a reader locate
what arrives, what changes, what is returned/sent and what happens on failure
without decoding a long block? Check claims against source; preserve qualifications
that change behavior and correct omissions/contradictions without inventing missing
facts or duplicating aliases. A heading or citation alone does not establish meaning.
You perform this review; no extra model, critic stage, semantic lint or separate
report is needed. Keep proof metadata and review bookkeeping outside reader prose.
In debug work, retain observable inputs, outputs and check evidence outside published
pages; do not request or publish hidden reasoning. Intentional rereads remain allowed;
smaller input is not proof of completeness.

When saved connected metadata is available, consult its optional
[structural self-review](connected-computation.md#saved-structural-self-review)
after drafting. Revisit referenced source and correct omitted meaning before READY;
keep the technical review out of reader pages. The full question and resource
inventory still control scope; review rows do not certify completeness.

Write original/edits as literal UTF-8 data with the runtime's existing file/edit
tool (for example apply_patch); let its argument serializer carry the data.
When a working original/edits file already exists, patch its changed portions
instead of resending the whole file. Preserve untouched fields, IDs/evidence and
base/version, and earlier accepted artifacts. Rewrite fully when needed; keep the
same review and compose/update/finalize checks.
Do not embed prose in executable JS/Python/shell templates or change code ticks
and readable content to escape a carrier. Preserve Unicode, quotes, newlines and
interpolation-like text, for example this JSON string value:

```json
{"text":"π🙂 `code` ${literal} \"quote\"\n"}
```

Use a nonreserved shell variable such as `clew_result_code`, never `status`.
Edit explicit text/IDs/handles; gaps stay unbound. Retired claim IDs need page-level
`remove`, changed evidence/source bindings need `replace`, beside `fragments`.
Preserve old packages and failed receipts. Nested/overlapping claims or external
edits outside this representation use the [manual selector route](reader-metadata.md#bind-already-authored-pages-by-location).

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/page_citations.py" compose --input ORIGINAL --output-dir NEW
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/reader_export_check.py" finalize --compose-root NEW --manifest READ/citations.json --renderer-command RENDERER_JSON --output-dir FINAL
```

For updates add `--previous-index OLD_INDEX` to finalize. Repeat `--manifest` for
all old/new proof packages; retain explicit repository mappings (`--repositories`
when needed). Never merge/edit private proof objects. NEW and FINAL must be
absent with existing parents; FINAL must be outside compose/proof/previous-index
roots. Do not precreate stage directories.

Finalize checks the original/request against all authored pages, copies them,
runs existing bind, writes `sources.json` to the returned index, renders all
checked bound Markdown including citation link expansion, then runs full
export-check/native proof reconstruction. Inputs remain in place; unreconciled
Markdown changes fail. RENDERER_JSON is unchanged caller-trusted argv JSON with
separate `{input}`/`{output}` arguments, executed with caller permissions. The
renderer must be deterministic and independent of input/output locations.
The optional [portable Markdown renderer example](../examples/markdown-renderer/README.md)
supports the Codeclew shell, readable page titles, authored tables and a retained
neutral comparison option. Copy it to your
own tools directory, install its pinned dependency there, and supply the explicit
command JSON; the helper does not silently choose or install a renderer.

Require exit zero, `CHECKED_PAGES_FINALIZED` and `exportStatus: EXPORT_CHECKED`.
The result gives root, original/compose/authored/rendered/index/sources, manifests,
renderer, receipts, check and result paths; resolve relative paths under `root`.
Use it to report completion and continue required visual gates. Further help
discovery or duplicate unchanged native/export checks are unnecessary. Full
bind/render/export argv/stdout/stderr/exit receipts and check artifacts stay
local, including failures; inspect the reported stage/receipts on error. Normal
diagnostics and necessary repairs/rechecks remain available.

Completion is mechanical: semanticTruthAssessed stays false; full semantic/source
and visual review remain required. Nothing is published. For separate bind/render/
check operations use the manual route above. Bounds: 32 MiB original, 16,384
fragments, 64 pages, 2 MiB/page, 16 MiB total, 1–1,024 claims/page, 4,096 total.
Export is flat Markdown/HTML with an optional byte-checked `favicon.svg` referenced
by `<link rel="icon" type="image/svg+xml" href="favicon.svg">`. Other local assets,
nested/root-relative links, base URLs and srcset remain unsupported. Compose
itself checks no evidence.

## Addressed original edits

Use `update-original` instead of compose to avoid repeating old prose. Save
`originalSHA256` from compose/update; for an existing original compute its raw-file
SHA-256. Read needed fragments locally. All `start`/`delete` positions address the
same base page's zero-based `fragments` array:

```json
{"baseSHA256":"64_HEX_DIGITS","edits":[{"page":"guide.md","start":1,"delete":1,"fragments":[{"text":"Revised explanation.\n","id":"effect","evidence":["RETURNED_HANDLE"]}]}]}
```

`delete:0` inserts; empty `fragments` deletes an unbound heading/gap. Supply new
text and explicit IDs/evidence only. Existing IDs/handles must survive unchanged;
use direct original edits with explicit page metadata for removals/rebindings.
Untouched pages/fragments and remove/replace are retained. Stale versions,
duplicate/overlapping targets and invalid originals fail closed.

If `ORIGINAL_BINDINGS_CHANGED` reports new necessary evidence, keep both the new
text and its handles. The error identifies changed page/claim IDs and old/new
handles, and supplies explicit page `replace`/`remove` metadata. Patch the current
original directly, retaining all other pages/fragments; an affected page may be:

```json
{"page":"guide.md","replace":["effect"],"fragments":[{"id":"effect","text":"Expanded explanation.\n","evidence":["RETURNED_ORIGINAL_HANDLE","RETURNED_NEW_HANDLE"]}]}
```

Then run compose and finalize with `--previous-index PREVIOUS_CHECKED_INDEX` and
every required `--manifest`. `replace` names existing claim IDs whose evidence
changes; `remove` is only for intentionally retired claims. Do not remove a newly
required handle to satisfy a text-only update. Unknown/foreign handles still fail
binding validation. Text-only edits continue through `update-original` below.

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/page_citations.py" update-original --input ORIGINAL --edits EDITS_JSON --output-dir NEW
```

The fresh atomic compose package includes local `edits.json`/`update.json`.
Stdout gives the new version/paths and at most 16 changed locations; the receipt
retains all. Preserve failures and reconcile external Markdown edits first: this
updates only the specified original. Continue full review and finalize above.
