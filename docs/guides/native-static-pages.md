# Native source documentation pages

`docs pages render` consumes an explicit immutable `docs check` snapshot and
exact retained declaration IDs. It produces linked offline HTML and inert MDX 3
from one typed source projection. It runs no analyzer, recapture or latest-pointer
fallback. Ordinary source development uses `./clew`.

First register and bind the Java service. To include an attributed operational
instruction, import it through the protected note lifecycle before capture. Use
`docs note list --root /path/to/docs` to obtain the current `inputDigest` and put
this association in `association.json`:

```json
{
  "schema": "codeclew-documentation-note-association/1.0",
  "id": "on-call",
  "title": "Gateway retry instructions",
  "service": "example",
  "path": "notes/on-call.md",
  "targets": ["service:example/section-egress"],
  "classification": "policy",
  "period": "2026 onward; maintainer review required",
  "metadata": {"author": "Example service maintainer"}
}
```

```sh
./clew docs note import --root /path/to/docs --input association.json \
  --source /path/to/on-call-original.md --expected-input-digest CURRENT_INPUT_DIGEST
./clew docs note inspect --root /path/to/docs --path notes/on-call.md
```

Import preserves the bounded UTF-8 original text, including Unicode and CRLF.
`docs note associate` updates its association with explicit input and original
note digest checks; `docs note remove` removes the association while preserving
the original. List and inspect remain the supported ways to inspect live notes.

Then capture native compiler evidence and the note inputs:

```sh
./clew docs check --root /path/to/docs --service example
./clew docs context --root /path/to/docs --snapshot sha256:IDENTITY/SIZE \
  --service example
```

Use the returned snapshot and exact callable observation IDs in `selection.json`:

```json
[
  {
    "id": "delivery",
    "service": "example",
    "endpointDeclaration": "EXACT_RETAINED_ENDPOINT_ID",
    "workerDeclaration": "EXACT_RETAINED_WORKER_ID",
    "wiringDeclaration": "EXACT_RETAINED_WIRING_ID",
    "question": "Which guards can leave the gateway call unreached?",
    "noteIds": ["on-call"]
  }
]
```

```sh
./clew docs pages render --root /path/to/docs \
  --snapshot sha256:IDENTITY/SIZE --input selection.json --output /path/to/pages
```

The input is a JSON array of 1–128 selections. Each selection has a safe `id` unique ignoring ASCII case, `service`, exact `endpointDeclaration` and `workerDeclaration` IDs.
`wiringDeclaration`, `question` and `noteIds` are optional. The question is displayed as
reader context; it supplies no answer, semantic labels, effects or evidence.
`noteIds` is an explicit ordered list of up to 128 unique protected note IDs.
Omit it or use an empty array when no note should be included; both retain the
legacy selector identity. Included notes require a captured original, consistent
identity and digests, and a declared `metadata.author` string that is nonblank and
at most 512 UTF-8 bytes. A selected note must explicitly target
`service:SELECTED_SERVICE` or a supported section target such as
`service:SELECTED_SERVICE/section-egress`.
The note's assessment `service` alone does not establish this relationship. This
consumer accepts only service/section targets; entity, scenario and view target
forms need a later typed expansion. Missing, unrelated, unattributed, unavailable
or inconsistent selected notes fail before output creation.

Callable selectors identify a source boundary; an HTTP route is established only
when retained native endpoint evidence separately supplies that route.

The new or empty output directory receives `index.html`/`index.mdx`, five paired
views per selection (`overview`, `endpoint`, `worker`, `fields-state`,
`diagnostic`), one shared `sources.html`/`sources.mdx` appendix, `style.css`,
`projection.json` and `manifest.json`. Existing files, including symlinks, are
refused. HTML requires no JavaScript or network dependency. MDX contains escaped
text and native inert JSX, with no imports or executable expressions. Relative
links use the corresponding output format.

`projection.json` uses `codeclew-native-page-projection/1.0`: input and context
digests retain their original Check meaning, while `selectionDigest` hashes the
exact ordered selector array. Pages preserve full selected source, observations,
recursive statements, condition paths, calls, state rows, diagnostics, citation
ranges and limitations. Citations bind original service, revision, file, line and
byte ranges, source text/evidence digests and authority. Original source URLs
remain available; dependency archive sources without URLs link to the retained
local appendix. These anchors never point at a recaptured checkout.

Opted-in notes appear in a separate **Operational instructions** section on the
process overview. The other four views link to it. Exact captured text is escaped
inside an inert native `pre` element, alongside declared author, classification,
period and note/version/content identities. `humanInstructions` in the full
projection preserves these fields and the complete captured association. Each
record has authority `HUMAN_OR_IMPORTED_UNVERIFIED` and source-claim status
`UNASSESSED`. A `NOTE_ASSOCIATION` observation proves captured metadata, not the
note's source semantics; export neither interprets nor validates its claims.

A note's `contentDigest` follows the existing note convention: it hashes the
canonical serialized text string, rather than the raw file bytes.
`associationDigest` binds the captured association; `versionDigest` hashes the
ordered pair `(associationDigest, contentDigest)`. Native source regeneration
can change source-derived conclusions while retaining the note's text, declared
author and identities. Export resolves only the selected Check's pinned note
inputs and never rereads live notes or runs note snapshot capture.

`manifest.json` uses `codeclew-native-pages-static-manifest/1.0`, binds the saved
snapshot, projection and selector identities, supplies a shared semantic content
digest for each format pair, and hashes every generated content file. The
manifest does not hash itself. When notes are included, `selectedNotes` records
each included note's overview page ID, process ID, HTML/MDX filenames, note ID,
version digest, content digest and association digest. Static bundles are immutable output directories; these
inclusion records do not create documentation history publications. The CLI returns
`codeclew-native-pages-render/1.0` with status `RENDERED`, the immutable snapshot,
digests and the generated file list.

Shared queue identity requires an explicit local allocation, exact compiler call
relations and constructor assignments to the selected fields. Equal variable
names or types establish no handoff. Branches preserve exact expressions and
false alternatives. Unsupported syntax and missing/ambiguous provenance remain
local gaps. DEP-01 metadata identifies an external target and source availability;
it establishes no implementation behavior. Offer success, scheduling, runtime
activation, business delivery and external success require runtime evidence.
Diagnostics describe source-derived possible reasons and inspection points;
they do not diagnose an observed incident.

After editing a guard or transformation, capture a new snapshot explicitly and
render to a new directory. Old snapshots still render offline from retained
source after the checkout or latest pointer becomes unavailable. Selected notes
also retain their captured text and association after the live association is
edited or removed. Render a newer capture explicitly to adopt newer note inputs;
existing bundle files are never rewritten. Maintained authored-content editing
is a separate consumer.

The compiler-backed fixture check is
`cargo test --locked -p clew --test docs_static_pages -- --ignored --test-threads=1`.
It requires the repository JDK and verifies fresh source capture, two renamed
scenarios, source mutation, explicit attributed protected note inclusion,
Unicode/CRLF and inert-text preservation, relative links, hashes, format parity
and offline reuse after a public live association edit and removal. Its Maven
wrapper models compiler acquisition, not a full application build.
