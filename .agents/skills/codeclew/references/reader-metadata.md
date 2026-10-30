# Human HTML and exact private metadata

Use this explicit profile when sharing human documentation without automatic
source payloads, audit panels, binding manifests or machine sidecars. Ordinary
`docs render` and `docs pages render` remain evidence-bearing export routes.
This profile does not translate, assess or acquire source.

Prepare the original immutable Work from an explicit saved snapshot using the
normal Work route. Export an exact native service section from its frozen bundle:

```json
{"schema":"codeclew-reader-export-request/1.0","title":"Service overview","selection":{"kind":"NATIVE","bundle":"EXACT_FROZEN_BUNDLE","operations":["section-overview"]}}
```

```sh
clew docs reader export --root DOCS --work ORIGINAL_WORK --input export.json --output NEW_DIRECTORY
```

Legacy native `boundaries` and visual `limitations` mix business conditions with
analysis caveats. If any are present, `selection.boundaryRoles` must classify every
entry exactly once. Each role identifies the selected `operation`, an exact
`target`, a SHA-256 `textDigest` over the original UTF-8 text, and `audience`:

```json
{"operation":"section-overview","target":{"field":"OPERATION","index":0},"textDigest":"sha256:EXACT_TEXT_HASH","audience":"ANALYSIS_CAVEAT"}
```

Targets are `OPERATION` with `index`, or `CONTRACT`/`VISUAL` with the existing
contract/visual `id` and `index` in its boundaries/limitations array. The frozen
bundle remains the authority for the original text. Use `READER_CONDITION` for
substantive behavior constraints; use `ANALYSIS_CAVEAT` for evidence/analysis
qualification. No keywords or model decide these roles. Missing, duplicate,
foreign or stale roles refuse export. This projection cannot replace the text.
Human HTML receives only the explicitly classified reader conditions. The full
original entries and role choices remain private, pageable `NATIVE_BOUNDARY`
items from `docs reader query`, including analysis caveats. The classification is
caller-owned presentation policy, not semantic approval. Historical exports and
queries retain their exact original receipts.

Native operation identities are explicit; a heading is not a selector. The
selected publication's original Source versions must belong to the admitted
Work. Missing or ambiguous original versions refuse export; no current Source
is substituted. The HTML preserves substantive prose, contracts, conditions,
uncertainty, tables and interactive diagrams. It contains only opaque section
lookup references for private machine metadata, and no automatic source archive.
Existing authored links and quotations remain authored reader content.

For registered external Markdown or a protected note, use its captured original
and explicit UTF-8 byte ranges. `bindings` names typed binding slots already
registered on that note. Optionally supply an existing authored-sources map to
bridge its claim IDs to exact original native Work/SOURCE handles:

```json
{"schema":"codeclew-reader-export-request/1.0","title":"Operations","selection":{"kind":"NOTE","id":"runbook","map":"/owned/previous/sources.json","sections":[{"id":"refresh","title":"Refresh","startByte":6590,"endByte":7524,"bindings":[]}]}}
```

Those example ranges are not universal. Copy the exact range/ID from the current
validated map. Without a map, select registered binding slots explicitly. A map
must match the captured Markdown bytes and native source identities. Registration
or a source map is mechanical provenance, not proof of prose meaning.

The complete surrounding original prose is preserved; only selected blocks get
source-lookup addresses. Sections must be disjoint whole Markdown blocks. If a
note also contains authored provenance blocks that should stay private, supply
an explicit `publicRanges` array of `{startByte,endByte}` intervals in the NOTE
selection. The intervals are an allowlist over original UTF-8 bytes, not guessed
heading slugs or automatic prose redaction. Selected claims must lie inside it.
The complete original and map remain private. Missing metadata for unselected
prose does not imply evidence coverage for that prose.

Use the returned opaque reference and the actual copied HTML bytes:

```sh
clew docs reader query --root ORIGINATING_DOCS --reference RETURNED_REFERENCE --file COPY/index.html --max-items 20 --max-bytes 12288
```

Follow `nextCursor` with exactly the same selection and budgets until null.
`SOURCE_BINDING` rows carry existing Work `read-part` requests; follow those
bounded native reads only when source text is needed. Metadata lookup does not
hydrate source bodies. A cursor from another version, section or budget refuses.
Changed HTML, a missing original Work/receipt or an unrelated registry refuses;
there is no latest fallback. A copied HTML file works against its originating
registry. Private metadata remains in the existing Work read ledger/object
store and is retained by normal documentation storage planning/pruning.

`--compare-snapshot NEW_SAVED_CHECK` separately compares retained source versions
and captured note text. It never replaces the original bindings or asserts that
the Check matches a running service or the current checkout. Without it, status
is explicitly `NOT_COMPARED`.

## Guarded addressed Markdown repair

A mapped NOTE section returns `repairSupported: true` and
`repairAdapter: "reader_metadata.py"` because the adapter is bundled with this
version. Unmapped notes and native operation sections return false. This is
adapter availability, not a promise that a new paragraph passes its guards.

Use the exact copied HTML/reference, original caller-trusted READ manifests and
an explicit `{old,new}` replacement. The adapter queries that section, recovers
its original Markdown/map through `docs reader repair-base`, and invokes the
existing citation and rendered-paragraph guards:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/reader_metadata.py" --clew clew --root DOCS --reference RETURNED_REFERENCE --file COPY/index.html --replacement replacement.json --manifest OWNED_READ/citations.json --renderer-command renderer.json --output-dir NEW_DIRECTORY
```

`renderer.json` is a caller-trusted JSON argv array with separate `{input}` and
`{output}` arguments. The renderer reads `index.md` from input and writes
`index.html` under one visible `main` in output. It executes as the caller; its
arbitrary filesystem/network behavior is not sandboxed by this helper. Structural
HTML checks do not establish computed CSS visibility.

A manifest must retain its original discovery input, READ packet, receipt index,
raw native page/part streams and summary. The read-only validator checks exact
Work/service/snapshot, declaration or SOURCE bindings, stream hashes, native
page metadata, contiguous complete UTF-8 parts and canonical record digests.
Rehashed arbitrary `sources.json` alone is insufficient. Inputs must already be
caller-trusted; internal checksums do not authenticate a producer.

The binder first recompiles previous claims against the same closure. Only
explicit whole paragraphs may change; existing inline code and link structure,
bound source links, and unrelated Markdown/rendered paragraphs are guarded.
Hidden/comment/append/sidecar substitutions refuse. Original Markdown, map,
patch, renderer commands and rendered references remain in the new output.
There is no source acquisition, model call, automatic publication, entailment
check or runtime verification.

Recheck the complete preserved repair through the normal script entry:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/author_citations.py" check-repair --repair-directory NEW_DIRECTORY/repair --output NEW_CHECK.json
```

For an existing bound Markdown directory without a reader address, use the same
`author_citations.py repair` command with explicit `--base`, `--patch`, repeated
`--manifest`, `--renderer-command` and `--output-dir`. `bind` compiles supplied
claims against an existing valid READ closure; it does not discover or acquire
source. `docs reader repair-base` alone remains an exact original-data export.

## Read an exact retained source view

When writing or repairing prose from an existing caller-trusted READ manifest,
use `inspect` rather than decoding the packet's internal JSON structure:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/author_citations.py" inspect --manifest OWNED_READ/citations.json --file services/example/Handler.kt
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/author_citations.py" inspect --manifest OWNED_READ/citations.json --file services/example/Handler.kt --start-line 32 --end-line 48
```

Use the exact retained path and original one-based inclusive lines. One invocation
selects one file; use separate calls for different files. Repeated `--file`,
unknown or ambiguous paths, incomplete/inverted/out-of-bounds ranges and invalid
proof refuse with nonzero exit; there is no wildcard, clamping or source fallback.
Omit all selection flags for the unchanged full `inspect` output.

Every selection first validates the complete original native proof. Output keeps
the existing author-read `records`/`texts` style and original cite handle, with
only the selected source body. `records[].source` and `sourceRecordDigest` retain
the original identity, revision, evidence lines and digest; `location` and `view`
describe the displayed lines, preserving their UTF-8 text and LF/CRLF endings.
A range is a view of that citation, not new evidence or whole-file coverage.
Inspection performs no native commands, changes no evidence and does not assess
meaning, current runtime behavior or publication approval.

For several exact views from one retained manifest, the optional grouped route
shares source headers and repeated protocol metadata. Save a JSON array:

```json
[{"file":"services/example/Handler.kt","startLine":32,"endLine":48},{"file":"services/example/Handler.kt","startLine":70,"endLine":81},{"file":"services/example/Config.kt"}]
```

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/author_citations.py" inspect-grouped --manifest OWNED_READ/citations.json --request views.json > grouped.json
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/author_citations.py" display-grouped --input grouped.json --against RECEIVED_CONTEXT.txt --prior-complete
```

Use the shown `--against` flags when that context is known to have arrived intact.
For unknown/incomplete prior delivery omit them for full text; `--reread` also
shows full text when deliberately revisiting source. No read-once rule applies.

`views` preserves zero-based request order and maps each request to a group,
fragment and original `cite`. The group gives service/file/revision/authority,
original `sourceLines` and citation-specific role/body status/scope/limitations;
each fragment gives original inclusive line bounds and verbatim text. Only full
validated source-identity equality permits grouping. Identical ranges share one
fragment; overlapping ranges remain verbatim separate fragments, and disjoint
fragments never imply adjacency. No text is summarized or semantically removed.
Use returned `views[].cite` with the original manifest for ordinary document-span
binding/check/export, never group/fragment IDs as evidence handles. Full digests,
occurrences and source metadata remain accessible through legacy inspect/maps.
For opt-in references to already delivered exact bodies, see the
[compact display recipe](compose-original.md#frozen-source-to-reviewed-pages).
All original limitations, coverage warnings, discovery completeness and failures
are retained. Incomplete/corrupt READ proof refuses before successful output.

Both range bounds are required together. An optional `cite` per request may
disambiguate an exact path using an already returned original handle; otherwise
ambiguous files refuse. Unknown fields/files/handles, duplicate JSON keys and
invalid ranges refuse the entire batch, with no partial success or fallback.
Do not combine grouped requests with legacy single-view flags. Requests are
limited to 256 entries/1 MiB JSON and 16 MiB unique fragment UTF-8 bytes. Multiple
manifests use separate calls; grouped output is a view, not a new proof identity
or approval. Character/byte reduction does not establish model-token savings.

## Bind already-authored pages by location

For compose-original pages, use the separate [initial/update route](compose-original.md).
The instructions below are for manual selectors on existing Markdown.

Review the complete reader draft against the full request and retained source;
fix material gaps before final locations, exact span binding and rendering.
Keep source handles while drafting; no separate review report is required.
`page_citations.py` extracts selected text from page/digest/position/handle
requests. Binding validates the full retained READ proof, not semantic truth.

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/page_citations.py" locations --markdown-root authored --page guide.md --page api.md --output locations.json
```

The saved location index supplies exact page digests and line/byte positions
without reader prose. Copy its page digest into a compact request:

```json
{"schema":"codeclew-page-claims/1.0","pages":[{"page":"guide.md","markdownDigest":"RETURNED_GUIDE_DIGEST","claims":[{"id":"purpose","startLine":3,"endLine":6,"evidence":["RETURNED_HANDLE"]}]},{"page":"api.md","markdownDigest":"RETURNED_API_DIGEST","claims":[{"id":"purpose","startByte":20,"endByte":80,"evidence":["RETURNED_HANDLE"]}]}]}
```

Lines are one-based and inclusive, including original line endings. Alternatively
choose a half-open UTF-8 byte range. Do not combine both selectors. Empty spans,
stale page digests, split UTF-8/CRLF boundaries and ambiguous locations refuse;
there is no shorter-substring fallback. A rare overlapping literal occurrence
that the existing single-page map cannot represent also refuses explicitly.
Table and list spans are supported. The same local claim ID may appear on two
pages; it must remain unique within each page.

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/page_citations.py" bind --markdown-root authored --request selections.json --manifest OWNED_READ/citations.json --output-dir bound
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/page_citations.py" check --index bound/index.json --manifest OWNED_READ/citations.json
```

Use a fresh `locations --output` file after each edit and a fresh `bind
--output-dir` path; do not pre-create the binding directory, even when empty.
Existing outputs are preserved and refused. For an update, keep the previous
package and use another output path as shown below.

Repeat `--manifest` for all required retained proof packages. Optional
`--repositories` uses the existing explicit link-mapping format. Each page gets
the unchanged `codeclew-authored-sources/1.0` map in `maps/`, complete reader bytes
in `markdown/`, and body-free selectors. `index.json` binds relative page paths,
Markdown/map/selection digests and page-local IDs. It has no synthetic global
Markdown digest. `check` reconstructs the maps through current native proof;
matching caller-supplied checksums alone is insufficient. All Markdown bytes are
preserved except existing explicit `cite:` link expansion; its original input
is retained separately. A renderer continues to read Markdown and needs no map
adapter or new source schema. Only selected spans have mechanical claim coverage.
Continue rendering the complete authored root. The package's Markdown copies
retain the explicitly selected pages for checking; the authored root is unchanged.

For a one-page update, edit that Markdown, refresh its location digest, and supply
only that page in `update.json`:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/page_citations.py" bind --markdown-root authored --request update.json --manifest OWNED_READ/citations.json --previous-index bound/index.json --output-dir updated
```

Unspecified pages are revalidated and their Markdown/maps copied unchanged.
An unspecified page with changed input refuses. Page-local `remove` lists must
name exactly retired claim IDs; `replace` lists name evidence/source rebindings,
as in ordinary `bind`. No page is silently removed. Every page keeps at least
one selected claim. Mapping changes require fresh selections for affected pages.
CLI output gives counts/paths, with details retained locally. The helper admits
up to 64 portable relative `.md` paths, 4,096 total claims, 2 MiB per page and
16 MiB total input/selected span bytes; it refuses exceeding these bounds.

## Verify a deterministic flat HTML export

Use `reader_export_check.py` after binding and rendering a flat set of pages.
It runs the existing full native page-map check, compares every outer authored
Markdown file to its checked original input, and verifies the export against a
fresh rendering of the complete checked `markdown/` copies. Existing citation
link expansion belongs to those checked copies; nothing rewrites the authored
files or the supplied export. Keep the inputs stable throughout verification.
Export-check already includes full page-map proof reconstruction; avoid a
duplicate equivalent native check on unchanged inputs.

Provide the same explicit caller-trusted renderer and options used for export.
The JSON command is an argv array with separate `{input}` and `{output}` arguments,
using the existing addressed-repair renderer convention, for example:

```json
["node","/absolute/path/to/existing-renderer.mjs","{input}","{output}","Documentation"]
```

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/reader_export_check.py" --manifest OWNED_READ/citations.json --index bound/index.json --authored-root authored --rendered-root exported --renderer-command renderer.json --output-dir NEW_CHECK
```

Repeat `--manifest` for the original proof packages. `NEW_CHECK` must not exist
and must be outside the authored, package, exported and READ directories. It
retains `result.json`, `native-check.json`, the renderer argv/stdout/stderr/exit
receipt and the expected input/output copies. Failures return nonzero and retain
their report when a fresh check directory was created; existing reports are never
overwritten. Check the actual exit status as well as `EXPORT_CHECKED`.

This bounded contract accepts at most 64 root-level `.md` pages and matching
root-level `.html` outputs only (2 MiB per HTML file, 16 MiB total HTML). Nested
metadata in the authored root is ignored; its root-level Markdown inventory must
match the checked index exactly. The exported root contains only those HTML files
(Finder `.DS_Store` is ignored). Local assets, nested/root-relative links, `<base>`
and `srcset` are unsupported and refuse. Relative HTML links and decoded fragments
must resolve to an exported page and an actual unique HTML `id` or named anchor.
Duplicate heading text is fine when the renderer gives it distinct IDs. External
HTTP(S), protocol-relative, mail and telephone URL availability is not checked.

The comparison is exact full HTML bytes, without Markdown regexes or text
normalization that could erase numbers, operators, code or table cells. The
renderer must produce deterministic, location-independent HTML; timestamps,
temporary paths or other output differences refuse. Its argv executes as the
caller, without a sandbox. `EXPORT_CHECKED` means equivalence to that trusted
renderer plus native binding/input consistency and static link targets. It does
not independently establish the renderer's faithfulness, browser/CSS visibility,
semantic completeness, fresh service behavior or publication approval. Preserve
any required visual gate and necessary iteration: after material edits or failed
checks, refresh affected locations/bindings, render and check again as needed.
Use the supported previous-index repair path above. Generic export/link
verification also benefits workflows without Clew; no token saving follows from
a mechanical pass.
