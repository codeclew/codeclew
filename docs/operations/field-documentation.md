# First useful documentation and selective refresh

This workflow separates an initial source explanation from optional compiler
analysis. Uppercase values below are returned handles or operator-selected values,
not literal values to copy unchanged. Use installed `clew`, or `./clew` when
developing this checkout. See [snapshot storage](docs-snapshot-store.md) for pins
and retention, and [source profiles](source-documentation.md) for evidence limits.

## First useful document without Maven

Use a separate documentation root and one explicitly selected service. Register a small committed source scope containing the worker, relevant helpers and configuration. Example `/work/orders-source.json`:

```json
{
  "schema": "codeclew-documentation-service/1.0",
  "id": "orders",
  "title": "Order worker",
  "repositoryId": "orders",
  "repository": "https://example.invalid/orders",
  "language": "java",
  "profile": "source-syntax",
  "targetRef": "FULL_COMMIT_SHA",
  "source": {
    "roots": ["src/main/java/example/worker", "src/main/resources"],
    "dialect": "17"
  }
}
```

Replace URL, commit and paths with actual values. Omit `modules` for the first capture. Optional `contractFiles` is an explicit array of committed OpenAPI files, independent of these roots.

```sh
clew docs init --root /work/architecture --title 'Order worker documentation'
clew docs service list --root /work/architecture
clew docs service add --root /work/architecture --input /work/orders-source.json --expected-input-digest INPUT_DIGEST
clew docs bind --root /work/architecture --service orders --repo /work/orders
clew docs modules list --root /work/architecture --service orders
clew docs check --root /work/architecture --service orders
clew docs context --root /work/architecture --service orders --snapshot SOURCE_SNAPSHOT --format compact --limit 100
```

Use the latest list/show `inputDigest`, including `sha256:`, for each catalog mutation; save `snapshot` from check. If evidence-selection/update policies were already configured, inspect them first: an admitted external evidence package may be selected ahead of local capture. This example assumes a new root with no such policy.

Select one method from returned evidence. `docs process candidates --root /work/architecture --service orders --snapshot SOURCE_SNAPSHOT --declaration SYMBOL_OBSERVATION_ID` supports an explicit callable root even where automatic discovery is incomplete. A candidate is navigation, not accepted business meaning. Create a process definition using the existing process schema/fixture: exact service/selector, explicit requested scope, participants, trigger/outcomes, bounded `maxDepth`/`maxNodes`, and only declared interactions. Source methods must use the selector values exposed by retained evidence rather than guessed JVM identities.

```sh
clew docs process inspect --root /work/architecture --input /work/process.json
clew docs process list --root /work/architecture
clew docs process put --root /work/architecture --input /work/process.json --expected-input-digest INPUT_DIGEST
clew docs recompose --root /work/architecture --snapshot SOURCE_SNAPSHOT
```

`inspect` consumes latest compatible retained evidence. `put` changes declarations, so use the returned recomposed snapshot for authoring; recomposition executes no analyzer and does not update latest. Do not use default `process prepare` here: it selects latest, which predates the added declaration.

Prepare `/work/request.json` as `{"schema":"codeclew-documentation-work-request/1.0","audience":"Worker maintainers","entrypoint":"process-overview","contextProfile":"process-v1","maxItems":100,"maxBytes":40960}`. Use the ID authored in the process definition:

```sh
clew docs work prepare --root /work/architecture --subject scenario:PROCESS_ID --input /work/request.json --snapshot RECOMPOSED_SNAPSHOT
clew docs work read --root /work/architecture --work WORK_ID --input /work/selection.json
clew docs proposal submit --root /work/architecture --work WORK_ID --input /work/proposal.json
clew docs proposal publish --root /work/architecture --proposal PROPOSAL_ID --unassessed
```

The Work result provides evidence handles; `selection.json` names those handles, and a human/current agent authors the schema-constrained proposal from recorded reads. This is real authoring work, not an automatic consequence of capture. Alternatively, `docs work run --root /work/architecture --work WORK_ID --config /work/execution.json` uses an explicitly configured author/reviewer; do not invent provider credentials or assume it runs free. Local publication remains `UNASSESSED`; configured meaning review is separate.

`proposal publish` and an accepted `work run` already create a frozen publication. Open the bundle returned by that command to read the new explanation. Do not immediately render the pre-author snapshot: an exact snapshot also retains its captured authored baseline, so that render can reproduce the earlier gaps instead of the newly accepted prose.

When another projection is needed, run `docs recompose` from the original source-capture snapshot after publication, then render its newly returned snapshot. Both operations use saved evidence without capture. Keep the frozen publication ID/path. Pin the source parent with `clew docs snapshot pin --root /work/architecture --name orders-source-first --snapshot SOURCE_SNAPSHOT` before changing service configuration.

## Explicit compiler enrichment afterward

Keep the same service ID, `profile: "source-syntax"`, source roots/dialect and fixed commit. Add this object to the complete service JSON, then update through `service show` / `service add` with its current input digest:

```json
"modules": {
  "schema": "codeclew-documentation-modules/1.0",
  "semantic": {
    "module": "javac",
    "enabled": true,
    "profile": "java-17plus-maven-read-only",
    "compilation": ":/main"
  }
}
```

Use the actual qualified profile and compilation selector; writable/AP projects require their own explicit supported profile/admission. Configure the provider only through `modules.semantic`. Then run `clew docs check --root /work/architecture --service orders`. This is synchronous and may run Maven/compiler work for that provider's selected compilation and its build dependencies. It retains compatible saved sibling services; it does not guarantee no reactor work. Ordinary `context`, Work and `render` consume saved evidence; avoid `--refresh` and unscoped `docs check` when no acquisition is intended.

This is a new capture, not an in-place enrichment of the old snapshot. Current code reparses source when semantic execution is enabled, then attaches only unique equal-revision/name/file/line `SEMANTIC_SYMBOL` observations. Source identities remain source-based, and lexical FLOW targets are not upgraded. Provider failure is explicitly recorded as unavailable while source evidence remains readable in this intentionally selected source profile. A native-only failed capture is never silently substituted with syntax evidence.

## Profile changes and historical access

Any service-record change changes its digest and overall catalog input. Enabling the semantic module therefore invalidates ordinary current consumers of the previous check. `Check::retained` enforces current catalog equality even for explicit `--snapshot`; `recompose` rejects service/profile/module/root changes. The old bytes are retained, but do not promise that `context/render/work prepare --snapshot OLD` works under the changed catalog. Existing pins can be checked with `docs snapshot show`; frozen publications remain available through `docs history list/show` and their generated files. Finish and save the source publication before changing the service record; prepared old work also cannot be published against incompatible current declarations.

Changing the top-level profile to native additionally requires removing `source`/source-only modules and uses native identities; there is no automatic mapping of authored source process roots. Prefer the optional module path when source-root continuity is wanted. A fixed commit and equal line ranges still do not map transformed-source semantics automatically.

The first document must state `SYNTAX`, unresolved call targets, lexical ordering and unknown runtime activation. Capture is bounded to 2,048 files, 2 MiB/file and 32 MiB source scope; narrowing scope is explicit, never silent omission. Measure time to retained evidence, time to first useful authored process and answers to reader questions separately from later compiler enrichment. This workflow avoids initial Maven waiting; it does not resolve AP input closure or replace the independent zero-repeat-analysis requirement.

## Recover saved captures after a failed check

A completed service capture may be present even when the final check snapshot was
not saved. Repeating an ordinary check can repeat acquisition: a native capture
marked `NON_CACHEABLE` is historical evidence, not permission to reuse it as a
current source check.

First repair the reported documentation state. If `DOCS_BASELINE_INCOMPLETE`
names a missing generated bundle, restore that bundle from the same root. If the
old publication was intentionally discarded, explicitly move the generated
`docs/index.html` aside outside the documentation root. Preserve any wanted old
publication before doing so. Do not remove the capture cache. A missing publication
cannot be repaired by running a compiler. Checks validate this baseline before
starting source capture.

With the original current-format catalogue and cache still present, explicitly
select the saved capture manifest basenames:

```sh
clew docs snapshot recover --root /work/architecture --capture orders-CAPTURE_KEY.json
clew docs context --root /work/architecture --service orders --snapshot RECOVERED_SNAPSHOT --format compact --limit 100
clew docs snapshot pin --root /work/architecture --name recovered-orders --snapshot RECOVERED_SNAPSHOT
```

Repeat `--capture` for each selected service. Obtain exact filenames from that
root's `.codeclew/cache` directory; the command does not guess the newest capture.
Every manifest must match its current registered service declaration and all
referenced objects must be intact. Duplicate services, unsafe filenames, external
evidence expectations and update policy/target state are refused. Source checkouts
and compiler tools are not needed. The command creates an immutable snapshot,
without updating `latest-check.json`, rewriting keyed captures or rechecking source
freshness. Recovered services remain `RETAINED_SOURCE_NOT_REVERIFIED`; omitted
services are unresolved. Continue Work, rendering and pinning with the explicit
returned snapshot.

Recovery accepts only current formats. It does not import old releases, reconstruct
missing catalogue files, or make unsupported old narratives readable. Restoring a
snapshot from saved captures is distinct from checking whether sources changed.

## Release boundary

Version 0.10 requires a new documentation root and fresh indexing. Old private
state, bindings and narrative formats are not imported or migrated. Keep old
roots separately if needed; initialization does not delete them. Current-format
snapshots, Work and explicit pins continue to share retained objects.

## Publish internal flows and decisions in service pages

Typed visuals are authored from saved evidence and published through the same
Work/proposal path as prose. They appear in the generated service page; a separate
atlas or Mermaid installation is unnecessary. This does not automatically discover
all business processes. Select the process or local decision being explained,
read its implementation and relevant helpers, and state what the selected scope
leaves unresolved.

For service-level internal behavior, prepare a responsibilities section from an
existing compatible snapshot. Save `/work/visual-request.json` as:

```json
{
  "schema": "codeclew-documentation-work-request/1.0",
  "audience": "Service maintainers",
  "entrypoint": "section-responsibilities",
  "maxItems": 100,
  "maxBytes": 49152
}
```

```sh
clew docs work prepare --root /work/architecture --subject service:orders --snapshot SOURCE_SNAPSHOT --input /work/visual-request.json
clew docs work read --root /work/architecture --work WORK_ID --input /work/selection.json
clew docs proposal submit --root /work/architecture --work WORK_ID --input /work/visual-proposal.json
clew docs proposal publish --root /work/architecture --proposal PROPOSAL_ID --unassessed
```

Use the returned section handle as the proposed operation's `entrypoint`.
`selection.json` can contain `{"references":["RETURNED_SOURCE_REF"]}`; use actual
handles, follow pagination, and expand the relevant callable/source records.
Navigation summaries, source aliases and merely present Work handles do not
replace a recorded delivery of evidence. Every semantic visual field cites
supported records actually supplied to this Work. No new check is needed just to
add a diagram to saved evidence.

A proposal keeps schema `codeclew-documentation-proposal/1.0` and places a `visuals`
array inside a proposed operation. A service section can use `steps: []`, its
ordinary evidence-bound `summary`, and the visuals below. This illustrative array
assumes source with a negative-quantity rejection and a nonnegative return. Replace
`BODY_REF`, the labels and the rules with the actual delivered evidence; do not
submit these illustrative claims against unrelated code.

```json
[
  {
    "id": "quantity-flow",
    "kind": "execution-flow",
    "title": "Quantity validation",
    "purpose": {"text": "Explain validation outcomes.", "evidence": ["BODY_REF"]},
    "scope": {"text": "The selected quantity validation method.", "evidence": ["BODY_REF"]},
    "limitations": ["Method behavior only; runtime callers were not verified."],
    "nodes": [
      {"id": "validate", "meaning": {"text": "Check requested quantity", "evidence": ["BODY_REF"]}},
      {"id": "reject", "meaning": {"text": "Throw invalid argument", "evidence": ["BODY_REF"]}},
      {"id": "return", "meaning": {"text": "Return quantity", "evidence": ["BODY_REF"]}}
    ],
    "edges": [
      {"id": "negative", "from": "validate", "to": "reject", "meaning": {"text": "quantity < 0", "evidence": ["BODY_REF"]}},
      {"id": "accepted", "from": "validate", "to": "return", "meaning": {"text": "quantity >= 0", "evidence": ["BODY_REF"]}}
    ]
  },
  {
    "id": "quantity-decision",
    "kind": "decision-table",
    "title": "Choose the validation outcome",
    "purpose": {"text": "Explain the branch at quantity validation.", "evidence": ["BODY_REF"]},
    "scope": {"text": "The same method's ordered branch evaluation.", "evidence": ["BODY_REF"]},
    "limitations": ["This table documents source behavior; it is not an executable DMN model."],
    "parent": {"artifact": "quantity-flow", "node": "validate"},
    "hitPolicy": "FIRST",
    "policyExplanation": {"text": "The negative check terminates with an exception; otherwise evaluation reaches the return.", "evidence": ["BODY_REF"]},
    "rules": [
      {"condition": {"text": "quantity < 0", "evidence": ["BODY_REF"]}, "outcome": {"text": "Throw invalid argument", "evidence": ["BODY_REF"]}},
      {"condition": {"text": "Otherwise", "evidence": ["BODY_REF"]}, "outcome": {"text": "Return quantity", "evidence": ["BODY_REF"]}}
    ]
  }
]
```

Choose the representation from the question being answered:

- `execution-flow` describes supported action order and conditional transitions.
  An edge must explain its condition or ordering; proximity on a diagram is not
  evidence of a call. Preserve loops, early exits and asynchronous boundaries.
- `dependency-map` describes structural relationships. It does not establish
  execution order, invocation conditions or runtime reachability.
- `decision-table` explains conditions and outcomes. A `parent` must reference an
  existing `execution-flow` node in the same operation. If the selected handler
  cannot be located in the overview from retained evidence, omit `parent`, describe
  the local scope and record the missing connection in `limitations`. Do not invent
  a parent merely to connect every card.

Every visual requires an evidence-bound `purpose` and `scope`, plus nonempty
`limitations`. Explain why this selection helps the reader; selected handlers are
not automatically a complete list or an importance ranking. `FIRST` means choose
the first matching rule in row order; it does not order separate tables or prove
that a selected output contains only one action. `UNIQUE` claims mutually exclusive
matching rules. Use `UNKNOWN` when retained evidence cannot establish the policy.
All three require an evidence-bound `policyExplanation`; structural validation
cannot prove mutual exclusivity or that the prose correctly interprets source.

Use a decision's optional evidence-bound `afterSelection` for execution outcomes
that occur after choosing an action, such as serialization or send failure. Keep
those outcomes separate from pre-action selection conditions. A selected action,
a scheduled callback and completed external work are different observations.

The typed format accepts closed data, not imported Mermaid, HTML or script.
Codeclew owns layout and produces local reader assets without a CDN dependency.
Visuals are part of their containing operation: they are versioned, replaced,
retained and checked for freshness atomically with its prose and evidence. They
have no independent acceptance or review status in this release. Publishing with
`--unassessed` does not independently verify either the explanation or the diagram.
A changed participant can make retained content require review without an automatic
model rerun.

The narrow `section-author-v1` job authors a section summary and cannot emit these
visuals. Use general Work reads and a proposal, or a configured process-overview
job for a declared process. Inspect the returned job schema before delegating to
an author. Native publication validates structure, references and recorded reads;
it does not prove semantic correctness, exhaustive internal-flow coverage or
runtime behavior.

When separately prepared services are published sequentially, generated empty
pages for sibling services no longer invalidate their Work. Real authored updates,
including custom gap text, still conflict. Proposal records reference the saved
Work influence, and publication shares identical influence maps across accepted
operations while retaining distinct historical evidence where necessary.

## Select English or Russian documentation

Pass `--language en` or `--language ru` to `docs work prepare`, or set
`documentationLanguage` in the request JSON. Conflicting explicit values fail.
Without an explicit value, Work inherits the current publication language and
uses English for a root with no selected language. The language participates in
immutable Work identity and is recorded on accepted operations.

```sh
clew docs work prepare --root /work/architecture --subject service:orders --snapshot SOURCE_SNAPSHOT --input /work/visual-request.json --language ru
clew docs render --root /work/architecture --snapshot SOURCE_SNAPSHOT --language ru
```

Russian author instructions require ordinary Russian engineering terminology
without unnecessary English loanwords. Preserve class and method names, paths,
fields, API and evidence IDs. Publish through the same proposal workflow.

Rendering selects presentation language, not machine translation. Existing prose
in another or unknown language remains in its accepted version but does not count
as completed target-language content. The reader shows a translation placeholder
and links to an available original version. `translationGaps` is separate from
analysis gaps; it does not trigger capture. Reusing the same source snapshot for
a translated author Work requires no reindexing. Review the translation normally;
`--unassessed` still means the explanation has not passed meaning review.
