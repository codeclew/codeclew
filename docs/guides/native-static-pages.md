# Native source documentation pages

`docs pages render` consumes an explicit immutable `docs check` snapshot and
exact retained declaration IDs. It produces linked offline HTML and inert MDX 3
from one typed source projection. It runs no analyzer, recapture or latest-pointer
fallback. Ordinary source development uses `./clew`.

First register and bind the Java service, then capture native compiler evidence:

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
    "question": "Which guards can leave the gateway call unreached?"
  }
]
```

```sh
./clew docs pages render --root /path/to/docs \
  --snapshot sha256:IDENTITY/SIZE --input selection.json --output /path/to/pages
```

The input is a JSON array of 1–128 selections. Each selection has a safe `id` unique ignoring ASCII case, `service`, exact `endpointDeclaration` and `workerDeclaration` IDs.
`wiringDeclaration` and `question` are optional. The question is displayed as
reader context; it supplies no answer, semantic labels, effects or evidence.
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

`manifest.json` uses `codeclew-native-pages-static-manifest/1.0`, binds the saved
snapshot, projection and selector identities, supplies a shared semantic content
digest for each format pair, and hashes every generated content file. The
manifest does not hash itself. The CLI returns
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
source after the checkout or latest pointer becomes unavailable. This slice
produces composed static documentation; maintained authored-content editing is a
separate consumer.

The compiler-backed fixture check is
`cargo test --locked -p clew --test docs_static_pages -- --ignored --test-threads=1`.
It requires the repository JDK and verifies fresh source capture, two renamed
scenarios, source mutation, relative links, hashes, format parity and offline
reuse of the original snapshot. Its Maven wrapper models compiler acquisition,
not a full application build.
