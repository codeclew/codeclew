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

## Recover original note data

For an exported note carrying a source map, `docs reader repair-base --root DOCS
--reference RETURNED_REFERENCE --file COPY/index.html --output NEW_DIRECTORY`
exports the exact original Markdown/map for caller review. It does not perform
an addressed repair or validate a new paragraph. The optional automated guarded
paragraph adapter is not included in this scope. Native operation content uses
its returned historical selector and is not an external Markdown paragraph.
