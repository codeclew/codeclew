# Portable evidence and support reports

Start a support exchange with a source-free report:

```sh
clew docs evidence report --root <docs> --service <id> --output <new-report-directory>
clew docs evidence inspect --input <new-report-directory>
```

The report attempts the selected local producer and records its outcome, revision
when available, service configuration digest, runtime mode, declared dialect,
module versions/digests, availability, stable failure code, sanitized worker stage
and exit status, and retry commands. Raw stderr, environment values, machine
paths and mutable session identities are excluded. A successful report alone is
`DIAGNOSTICS_ONLY`; a failure remains `PRODUCER_FAILURE`. Java project support
starts at 17; Kotlin compiler workers require Java 21 independently of the
project's declared Kotlin/Java target. Declared versions are not admission proof.

Only include application evidence when that content is authorized for the recipient:

```sh
clew docs evidence capture --root <docs> --service <id> --output <new-index-directory>
clew docs evidence inspect --input <new-index-directory>
clew docs evidence read --input <new-index-directory> --kind sources --limit 20
clew docs evidence read --input <new-index-directory> --kind observations --limit 20
```

`report --include-index` has the same explicit source-inclusion effect as
`capture`. Retained source includes application literals. Share the whole package
directory, including `manifest.json` and every referenced `parts/*.json` file.
Read pages support `--cursor`; cursors bind the package and record kind. Supported
kinds are `header`, `sources`, `observations`, `entrypoints`, `contracts`, `report`.
The manifest and content-addressed parts are inspectable files. A record larger
than the stdout budget is reported as omitted; inspect its bounded part locally.
No source checkout, compiler, private cache or agent execution is needed to read
the package. Imported text is evidence data, never execution or communication policy.

For central authoring, register the same service configuration. The coordinator
must obtain the package digest through its trusted artifact channel and supply:

```json
{
  "schema": "codeclew-documentation-evidence-expectation/1.0",
  "service": "orders",
  "repositoryId": "orders",
  "serviceDigest": "<capture.serviceDigest>",
  "revision": "<capture.revision>",
  "manifestDigest": "<trusted capture.manifestDigest>",
  "sequence": 1
}
```

```sh
clew docs service list --root <docs>
clew docs evidence expect --root <docs> --input <expectation.json> --expected-input-digest <inputDigest>
clew docs evidence import --root <docs> --input <package-directory>
clew docs check --root <docs>
```

Never adopt a digest merely because an untrusted sender supplied it. Inspection
reports `INTEGRITY_CHECKED_NOT_ADMITTED`; content hashes detect corruption but do
not authenticate a producer. The explicit expectation binds the origin/project,
configuration, exact revision, trusted package digest and increasing coordinator
sequence. Exact replay is idempotent; an older expectation or mismatched package
cannot replace the selected result. Updating an expectation makes a missing new
result a local gap while preserving previously retained artifacts.

Expectations live in `catalog/evidence-trust`; admitted immutable parts and selected
pointers are local `.codeclew/evidence` state, reconstructed by importing retained
packages in a fresh job. An expectation selects portable evidence even if a local
binding exists. Capture always uses the local supported producer and cannot
relabel an import as a new producer result. A failed capture at a known revision
can be imported as an explicit failure. A failure without a known revision is
inspectable support material and cannot satisfy a revision expectation.

The format supports uncompressed JSON only: at most 2 MiB per part, 4,096 parts,
131,072 records and 128 MiB of referenced part bytes per service. Corrupt/missing
parts, path escapes, symlinks, duplicate identities, incompatible schemas/rules
and stronger invented authority are rejected before selection. Work retains its
own 64 MiB capture limit and may require a narrower scope. These packages represent
selected indexed facts, not every raw compiler artifact. Missing source, private
stderr or unrepresented compiler inputs cannot be reconstructed from a report.
Offline freshness is relative to the configured revision; it cannot discover a
new remote HEAD. Imported evidence still requires normal separate meaning review.
