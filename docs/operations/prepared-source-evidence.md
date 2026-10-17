# Prepared SOURCE evidence without publication

`docs evidence retain-source` admits an exact immutable documentation snapshot,
resolves one SOURCE selector through the existing context view and expands its
registered dependency scope with `bindings::expand_dependencies`. It retains the
scope in the existing content-addressed documentation store. No generated docs or
publication baseline is created or required.

```sh
clew docs evidence retain-source --root /absolute/docs --snapshot SHA256_HANDLE \
  --service caller --symbol EXACT_SELECTOR --native-inputs /absolute/native.json
clew docs evidence compare-source --root /absolute/docs --input /absolute/old-packet.json \
  --snapshot TARGET_HANDLE
```

Both commands return `codeclew-docs-page/1.0` packets suitable for the bounded
prepared SOURCE helper. Retain supports explicit `--refresh` instead of
`--snapshot`; comparison never captures source or follows latest. The engine owns
the `evidenceSet` reference and its history. Keep the full stdout artifact; pass
it to comparison instead of reconstructing hashes or revisions in prose.

## Registered scope and decisions

Comparison re-admits both snapshot object closures, validates the retained
selector/configuration binding and uses the existing dependency-delta primitive.
Analysis configuration is separate from target ref, source-link presentation and
service title. Selected source byte changes, new closure membership, watched
source inventory, generated inputs and explicit native references remain tracked.

The serialized verdict is an enum:

- `UNCHANGED`: registered inputs match. This does not establish complete
  dependencies, runtime equivalence or correctness of arbitrary answer prose.
- `CHANGED`: a registered configuration, observation, source byte span, native
  file or membership changed. Conservative source-scope changes identify the
  affected file; meaning impact is not inferred.
- `UNKNOWN`: a registered dependency/source is unavailable, the target selector
  cannot be resolved or a native input has an unresolved acquisition limit.
  Missing/corrupt immutable objects fail admission instead of returning evidence.

`selectedSourceBytes` is separate from the overall decision. An unchanged handler
cannot hide a changed callee, configuration, generated input or watched membership.
An unrelated HTML title may change a conservative file scope while the selected
function stays identical. The packet supplies a labeled changed-character window
with `beforeCitation` and `afterCitation`: the retained source ID, revision, file,
line span, authority and configured URL for each side. These spans describe the
retained sources containing the window, not only its changed characters. They are
available even if those SOURCE rows are outside the selected context packet; no
additional baseline context lookup is required for citation. A window is not a
new exact source. If no retained file excerpt exists, no window or citation is
invented; the file change remains reported.

`SOURCE_BINDING_UPDATE` rows supply current engine-generated citation metadata,
including native-file links. They do not imply changed runtime behavior. Full
SOURCE records retain their exact stored text and original authority; aliases and
helper excerpts remain separate projections. Existing coverage, unresolved edges
and runtime limits are preserved, never promoted by a successful byte comparison.

## Native file registration

The optional JSON array registers literal committed files and membership prefixes:

```json
[{"service":"caller","files":["receiver/Controller.cs","receiver/Repository.cs","compose.yaml"],"membership":["receiver"]}]
```

`service` selects a captured repository/revision binding; it does not infer a
semantic owner. Native reads use that immutable Git revision, not worktree bytes.
The engine uses existing Git blob readers and immutable storage. Native byte
records use `SOURCE_NATIVE`; they are not promoted to captured semantic SOURCE. Cached native
objects can be re-admitted at the same revision without the source checkout.
Comparison at another revision acquires the registered inputs through the bound
repository; no agent-level Git/hash recheck is needed.

Registration establishes bytes for listed files and names under explicit
membership prefixes. It does not establish C# semantics, resolve transitive
native dependencies, interpret deployment config, or cover unregistered inputs.
Register every external/native input used by the answer. Initial discovery and
reading outside coverage remain allowed; repeat evidence verification belongs to
the engine. Missing, nonregular, oversized or non-UTF-8 text inputs retain explicit
limits. No unresolved input is silently treated as an empty successful scope.

Bounds: eight scopes, 32 files and 16 membership prefixes per scope, 512 tree
members per scope, 256 KiB per native file and 2 MiB of total registered native
bytes. Paths must be literal safe repository-relative paths. No full Git archive
or repository history is stored in the evidence set.

## Prepared multi-stage runs

Use one managed docs root and prepare all required immutable snapshots there with
supported registration, bind and check commands. Pin snapshots if the surrounding
retention workflow requires it. Carry each arm's own packet and answer files
between stages. Do not copy/edit private cache records to fabricate portability.
A packet references its managed immutable set; a missing set in another root is an
explicit error. Source acquisition and evidence comparison remain separate steps,
so answer-time registration/admission/compare can be measured independently.

The canonical short agent entry is [the skill README](../../skills/codeclew/README.md).
After engine admission/compare, agents must not repeat Git/digest/catalogue checks
for already covered inputs. A local packet fingerprint identifies the helper's
artifact only; it is not an additional source freshness verdict.

## Exact historical SOURCE drill

When source text is actually missing, use `docs context --root DOCS --service ID
--snapshot SNAPSHOT --source SOURCE_ID --limit 20`, or the portable helper's
`drill` recipe. Only an explicit snapshot plus exact SOURCE IDs selects this
historical route. It shares retain/compare's full immutable snapshot admission,
including object integrity and the captured source-input configuration binding.
It preserves original revision, file, line span, source authority and coverage;
no live checkout or current catalogue match is claimed. Current publication
narratives are not mixed into a historical source drill.

`evidenceValidation.mode=PINNED_SOURCE_SNAPSHOT_CLOSURE` identifies this route.
Missing/corrupt objects and unknown or cross-service source IDs fail closed.
Ordinary context without an exact SOURCE selector still checks the current
catalogue and may require recapture or recomposition. Output budgets, omitted
items and cursors are unchanged. A successful historical read establishes no
current freshness, semantic completeness or runtime equivalence.

## One-compare maintenance view

Pass the previous full packet directly to the helper's `compare` command. It makes
one supported `compare-source` call; packet reads, hashes and immutable admission
are internal. Neither a preliminary agent metadata read nor transcription of an
opaque evidence reference is part of the prepared path.

The `codeclew-prepared-maintenance-view/1.0` projection keeps baseline/target
snapshot, revision and configuration bindings, actual dependency values, changed
source windows, citation updates, coverage, unresolved inputs and all limits.
Unchanged SOURCE bodies are omitted. The full packet is retained privately for
subsequent comparisons and exact local continuation. The previous old-SOURCE
snapshot drill remains available and does not compare against today's catalogue.

`--metadata-only` and `--max-sources 0` omit SOURCE bodies without omitting changed
values, citations or limits. Large change collections have a typed local
`LOCAL_MAINTENANCE_RECORDS` continuation (`read --record-offset N`); SOURCE text
uses the existing Unicode character offsets. A view that cannot fit mandatory
metadata and one change fails explicitly; it must not hide limits or imply that
partial evidence is unchanged. No model-cost improvement is established by this
projection alone.
