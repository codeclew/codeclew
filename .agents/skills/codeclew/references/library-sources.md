# Independent Kotlin library source

Use this bounded path when a service A calls a separately captured Kotlin/JVM
library B. The first supported case is Kotlin 2.4, Gradle and independent Git
repositories. B is an in-process library source scope, never a deployed service
or an HTTP participant. Keep the normal service/thread workflow; do not replace
it with a standalone name search or copy B's source into A.

## Prepare independent evidence

Capture B through the normal compiler documentation lifecycle before attachment:
`kotlin-jvm-gradle-analysis`, explicit `compilations`, a registered repository
identity and an existing named `targetRef`. Retain its original `docs check`
snapshot. Capture A separately with that profile or `source-syntax`.

Inspect exact saved inputs without source acquisition:

```sh
clew docs library inspect --root B_DOCS --snapshot B_SNAPSHOT --service B
clew docs library inspect --root A_DOCS --snapshot A_SNAPSHOT --service A
```

B inspection supplies its service declaration, revision and publication
coordinates for the selected compilation. A compiler inspection supplies the
actual resolved artifact rows: SHA256, size, classifier, variant and classpath
slot. Copy the chosen row's `artifact` value into the input below. Do not infer
ownership from a version, FQN, classpath order or compatible JVM signature.
Compiler continuation additionally requires unique classfile ownership for the
exact called method in the selected A classpath; shadowed owners stay boundaries.

For example, an owned `Policy.decide` library can retain an internal helper and
Kafka send. A selected `Service.handle` thread must show only the B methods
reached from its call. Another B method publishing an unrelated topic does not
become part of that thread.

## Declare the association

Prepare a JSON input using inspected identities, replacing the placeholders:

```json
{
  "schema": "codeclew-documentation-library-binding-input/1.0",
  "id": "policy-library",
  "consumer": "application",
  "consumerCompilation": ":/main",
  "artifact": "REPLACE_WITH_THE_INSPECTED_ARTIFACT_OBJECT",
  "library": {
    "service": "policy-library",
    "repositoryId": "policy-library",
    "repository": "https://example.invalid/owned/policy-library.git",
    "revision": "REPLACE_WITH_B_CAPTURE_REVISION",
    "compilation": ":/main"
  },
  "authority": "DECLARED_SOURCE_BINDING",
  "rationale": "The owner explicitly associates this immutable B source capture with this artifact; binary/source equivalence is not established."
}
```

Register the explicit association, then derive a new snapshot from the original
A capture. Neither command opens B's checkout or captures it again:

```sh
clew docs library attach --root A_DOCS --snapshot A_SNAPSHOT --library-root B_DOCS --library-snapshot B_SNAPSHOT --input library-binding.json --expected-input-digest A_INPUT_DIGEST
clew docs recompose --root A_DOCS --snapshot ORIGINAL_A_CAPTURE
```

Use the returned composed snapshot explicitly for Work, threads and rendering.
`recompose` preserves the convenience latest pointer; an older snapshot remains
historical. `CHECKED` with provider `PARTIAL` evidence can return exit code 3;
inspect the structured status and unresolved services, retain the limitations,
and do not recapture solely to obtain exit code 0.

The association imports B's immutable capture into A's evidence closure. Its
foreign snapshot identity is provenance, not a requirement to retain B's whole
documentation repository. A failed admission leaves the current A capture
readable. Changed binding inputs require recomposition before current reads.

## SOURCE to compiler continuation

For SOURCE A, set `consumerCompilation` to `""` and add `consumerRevision` with
A's exact saved source revision. `artifact` can contain declared coordinates
without a binary hash:

```json
{
  "coordinates": {
    "group": "org.codeclew.owned",
    "name": "policy-library",
    "version": "1.0.0",
    "classifier": null,
    "extension": "jar"
  }
}
```

Optional SHA256, size and variant remain declarations for SOURCE A. They do not
establish A's resolved dependency. No A build is run by this path. Pinning A's
revision prevents an older declaration from silently surviving source or
build-file changes; review and replace it explicitly after a revision change.

A straightforward Kotlin call such as `Policy.decide(amount)` with an explicit
`import owned.library.Policy` can continue as a candidate when B retains one
matching owner/name/arity. File-wide receiver shadowing, multiple overloads,
wildcards, aliases and unsupported receiver expressions are not guessed. The
A call stays source syntax, the cross-repository edge stays declared/candidate,
and B's internals retain their own compiler evidence.

## Read, explain and render

Prepare service Work on the new snapshot. Read its `THREAD_INVENTORY`, then
`THREAD_CONTEXT` with the exact A declaration ID; follow the existing bounded
pager and read receipts. Library scopes are marked `LIBRARY_SOURCE` and
`REACHED_CALLABLES_ONLY_NOT_WHOLE_LIBRARY_COVERAGE`. Read `LIBRARY_SOURCE_SCOPE`
when detailed artifact/repository provenance is needed. Source snippets carry a
compact binding pointer; JVM/Gradle metadata is not repeated beside every body.

The same retained continuation is available to `docs process graph` and normal
`docs render --snapshot COMPOSED_SNAPSHOT`. Service egress includes a compact
candidate section with collapsed method bodies and evidence details. Citations
keep B's original file, range and revision; remote source export uses B's
repository mapping. Branches, local returns and recognized outgoing calls are
retained static evidence, not proof of runtime dispatch, delivery or persistence.
Use the normal proposal/review path for an accepted process explanation; no paid
full-service author run is necessary to demonstrate this connection.

### Checked Sources footer for human pages

`compose-original` names the [human-page route](compose-original.md#original-pages-and-completion),
not a `clew docs` command. Its existing command is
`python3 -I -S "$CODECLEW_SKILL_DIR/scripts/reader_export_check.py" finalize`;
add `--source-footers` as shown in that route. Follow the
[checked local footer example](../examples/markdown-renderer/README.md#checked-local-sources-footer)
for the required compose root, citation manifests, renderer argv and output path.
Finalize binds once; a render-only Sources update uses the existing checked index
and `reader_sources.py`, without another bind. `materialize` is an internal
function, not a `reader_sources.py` subcommand.

Footers select each page's bound claims, not every field of every library DTO.
The packaged renderer opens local verbatim excerpts; Git mappings are optional
and do not make this a remote-only export. Keep excerpt-containing HTML private
when its sources are private. The human bind/finalize `--repositories` file maps
source service IDs to `{repository,urlTemplate}` with placeholders `revision`,
`path`, `startLine`, `endLine`. Native `--source-map` instead uses the
`codeclew-source-repositories/1.0` wrapper keyed by immutable repository IDs;
these files are not interchangeable. A URL or an attachment is not read evidence.

### Library reads and citation-helper boundaries

A reached library callable does not establish all DTO fields or a complete wire
schema. Follow exact returned SOURCE references for the definitions behind the
claims you author. Preserve both the Work owner and the original library source
identity; an imported A-scoped SOURCE ID is not an original B Work reference.

Native thread/part delivery and acceptance by the human citation helper are
separate checks. The helper's SOURCE batch path currently requires source revision
to equal the Work service COVERAGE revision. An imported library row can retain
B's revision under A's Work scope and fail `SOURCE_BATCH_RECORD_CHANGED` despite
successful native delivery. This limitation does not establish that B changed,
and does not establish the cause of `SOURCE_BATCH_ROW_IDENTITY_CHANGED`.
The latter rejects row kind, id/reference pairing, duplicates or unsupported
omissions before checking record service/revision/digests. A smaller batch or
ordinary read/read-part is not a guaranteed citation-helper bypass.

If an independent retained B root/Work is available, use its own service,
snapshot and returned SOURCE IDs/references through
[read-sources](compose-original.md#retained-source-references), then supply the
resulting B manifests alongside A manifests to the common page bind/finalize.
Preserve B's file, range, revision and root ownership; do not relabel its evidence
as A, edit proof packets or infer a replacement repository from a URL. This route
requires actual B evidence; attached provenance alone is insufficient. A
SOURCE_NATIVE manifest also requires its actual retained evidenceSet and owning
root, never a synthetic packet reconstructed from quotes.

For an unresolved batch error, retain one sanitized helper/native argv, input
JSON, exit code/error and page metadata: Work, snapshot, subject,
selectionProjection, requested id/reference, returned kind/id/reference/reason,
and record versus COVERAGE service/revision. Source text and credentials are not
needed. Generic input/output-unavailable errors also need the exact input file
and root/Work arguments; they do not by themselves prove a missing library DTO.

## Failure, freshness and removal

- A resolved artifact hash/variant mismatch: inspect the new A capture, explicitly
  select the corresponding B capture and replace the association.
- B publication/version, repository, revision or compilation mismatch: select the
  matching independent original compiler capture. Keep the A binary boundary.
- Same version but republished bytes: the old compiler association cannot match
  the new SHA. Different B source at the same declared version remains an explicit
  weaker binding; matching names and signatures never prove equivalence.
- Missing imported evidence or ambiguous ownership/target: report the retained
  boundary and its repair. Do not choose another body or silently recapture B.
- An existing SOURCE association on a newer A revision: review and pin a new
  explicit declaration, then recompose. Do not infer build resolution.

Inspect current declarations with `docs library list --root A_DOCS`. Remove one
with the current input digest, then recompose the original A capture:

```sh
clew docs library detach --root A_DOCS --id policy-library --expected-input-digest CURRENT_INPUT_DIGEST
clew docs recompose --root A_DOCS --snapshot ORIGINAL_A_CAPTURE
```

Current derived claims disappear; historical snapshots and Work keep their
pinned B capture. Repeated reads and portable closure reuse that retained
material and do not assert that B's current checkout has been reverified.
