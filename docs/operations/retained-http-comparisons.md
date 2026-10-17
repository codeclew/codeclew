# Retained HTTP comparison evidence

Native full Pages can display recorded HTTP checks beside an exact source/OpenAPI
route pair. This is an optional evidence overlay; accepted narratives, source
bindings, review states and original views remain unchanged.

```sh
./clew docs pages render --root /path/to/docs-root \
  --snapshot 'sha256:EXACT_DIGEST/BYTES' --publication EXACT_PUBLICATION \
  --reading-plan /path/to/reading-plans.json \
  --contract-comparison /path/to/comparison-input.json \
  --output /path/to/new-output
```

The reading plan is optional. The comparison requires a full publication and an
exact immutable snapshot; it does not capture source, execute HTTP requests or
rerun a JSON Schema validator. It never implies exhaustive conformance or deployed
runtime verification.

## Input and admission

`codeclew-http-comparison/1.0` is a JSON object with these fields:

- `publication`, `snapshot`, `service`, `revision`: exact selected identities.
- `scope`, `limitations`: bounded statements of the observed environment,
  substitutions, untested branches and interpretation limits.
- `artifacts`: an object of identifier to `{text, digest}` records. Each digest is
  the SHA-256 of the exact UTF-8 text, prefixed with `sha256:`. Retain the raw HTTP
  observations, external validator report, source copies and harness here.
- `probeArtifact`, `assessmentArtifact`: identifiers of JSON artifacts. The probe
  has `cases`; the assessment has `validator` and `results`.
- `sources`: selected source bindings with `sourceId`, `revision`, `textDigest`
  and `artifact`. The artifact contains the original file. Pages compares the
  selected span's exact captured bytes within the declared lines. Leading
  indentation before the first token and a final newline may lie outside the
  compiler span. Exact artifact byte offsets and both original strings remain
  retained; no captured source whitespace is normalized.
- `entries`: records with `entrypoint`, `contractId`, `contractDigest`, `method`,
  `path` and `coverage`. Include an entry with no cases to disclose an inspected
  but unexercised HTTP path.

The input is limited to 1 MiB, 16 artifacts of at most 256 KiB each, 16 selected
source bindings, 64 entries and 128 HTTP cases. Unknown input envelope fields,
repeated identities, missing evidence and mismatched bindings fail closed.

Each probe case has an `id`, `request` (`method`, `path`, `body`, `contentType`)
and `response` (`status`, `contentType`, `body`). Each assessment result binds the
same `id` to `contractId`, `contractDigest`, `method`, `route`, `observedStatus`,
`observedMedia`, `statusDeclared`, `mediaAssessment`, `requestAssessment` and
`bodyAssessment`. Schema comparisons also name `comparedSchemaMedia`; optional
`requestErrors` and `bodyErrors` retain the external validator's diagnostics.

Allowed request assessments are `within-schema`, `outside-schema`,
`undeclared-media`, `no-body-declared` and `not-checked`. Body assessments are
`matches-selected-schema`, `violates-selected-schema`, `not-specified` and
`not-checked`. Media assessments are `declared`, `undeclared-media` and
`not-specified`.

Pages checks recorded method/path/status/media against the selected operation.
Response status selection follows exact status, status range, then default;
response media accepts exact declarations and matching media ranges. Schema
assessments remain explicitly external. A missing media declaration is distinct
from a body-schema difference, and a body without a declared schema is not a
schema failure. Accepting an out-of-contract request can indicate broader
acceptance; it does not establish failure for a contract-valid client.

## Retained output and authority

The service JSON and inert HTML payload receive `contractComparison`. The export
also contains `http-contract-comparison.json`, including exact source records,
raw artifacts, their digests, cases, external assessments and typed findings.
The native Pages manifest covers the exported file bytes.

`MATCHES_PINNED_SELECTED_SNAPSHOT` means the supplied evidence bindings match the
selected immutable capture. It is separate from
`EXTERNAL_RECORDED_LOCAL_PROBE` authority and
`RETAINED_EXTERNAL_ASSESSMENT_NOT_REEXECUTED` schema validation. External harness
files outside selected source records have integrity checks; they do not acquire
compiler or snapshot-source authority. A digest identifies retained evidence; it
does not authenticate an external author or prove that a probe ran.

A renderer refuses to apply findings to a different selected snapshot, revision,
route owner or contract digest. To use another capture, prepare a new comparison
with evidence for that capture. Do not relabel an old probe as current.
