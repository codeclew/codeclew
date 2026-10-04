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

A first check can return exit code 3 with `MISSING_BASELINE` even after saving a
`CHECKED` source snapshot: no explanation has been published yet. Inspect the
returned producer status and unresolved evidence. Reuse that exact snapshot when
capture succeeded; do not repeat acquisition just to clear the missing baseline.

Start with a service overview before defining a process. Save `/work/request.json`:

```json
{"schema":"codeclew-documentation-work-request/1.0","audience":"Worker maintainers seeking their first source explanation","entrypoint":"section-overview","maxItems":100,"maxBytes":40960}
```

```sh
clew docs work prepare --root /work/architecture --subject service:orders --input /work/request.json --snapshot SOURCE_SNAPSHOT
clew docs work read --root /work/architecture --work WORK_ID --input /work/sources.json
```

The preparation returns `WORK_ID`, the overview operation reference and recorded
context. Read every initial context page using its returned cursor. For the
source-syntax profile, `/work/sources.json` can be
`{"query":{"kind":"SOURCE"}}`. The query returns registered SOURCE references,
including otherwise-unreferenced whole files. Select the actual references for
the explanation; never guess `s` ordinals. Read selected records through
`{"references":["RETURNED_SOURCE_REFERENCE"]}`. If a record is omitted for size,
use [complete SOURCE part reads](docs-snapshot-store.md#read-one-retained-source-in-bounded-parts)
until `nextCursor` is null. Pagination changes delivery size, not source authority
or the requirement to finish reading a cited file. Complete FILE_ONLY text can
support an explanation of those exact bytes; it is not parsed compiler evidence.

Ask your current agent to explain the selected service from those recorded reads
and write a native proposal. For the first overview, the shape is:

```json
{
  "schema": "codeclew-documentation-proposal/1.0",
  "operations": [{
    "entrypoint": "RETURNED_OVERVIEW_REFERENCE",
    "title": "SOURCE_SUPPORTED_TITLE",
    "summary": {"text": "EXPLANATION_FROM_RECORDED_SOURCE", "evidence": ["RETURNED_SOURCE_REFERENCE"]},
    "steps": []
  }]
}
```

Replace every uppercase placeholder with the actual returned reference or an
explanation supported by the supplied text. The summary is plain prose; describe
missing facts explicitly. This authoring step is required: capture alone does not
write an explanation. The first overview can leave other sections as visible gaps.

```sh
clew docs proposal submit --root /work/architecture --work WORK_ID --input /work/proposal.json
clew docs proposal publish --root /work/architecture --proposal PROPOSAL_ID --unassessed
clew docs recompose --root /work/architecture --snapshot SOURCE_SNAPSHOT
clew docs render --root /work/architecture --snapshot RECOMPOSED_SNAPSHOT
```

Submit must return `READY_FOR_REVIEW` or `READY_WITH_LIMITATIONS`, with no structural
diagnostics, before local publication. Open the returned HTML output beneath
`/work/architecture/docs/`. `--unassessed` keeps meaning review explicitly
`UNASSESSED`; it does not certify the explanation. Recomposition after publication
includes the new narrative using the saved capture. It runs no analyzer. A saved
process definition, protected note or configured provider is optional for this
first result.

## Add a process explanation after the first overview

Select one method from returned evidence. `docs process candidates --root /work/architecture --service orders --snapshot SOURCE_SNAPSHOT --declaration SYMBOL_OBSERVATION_ID` supports an explicit callable root even where automatic discovery is incomplete. A candidate is navigation, not accepted business meaning. Create a process definition using the existing process schema/fixture: exact service/selector, explicit requested scope, participants, trigger/outcomes, bounded `maxDepth`/`maxNodes`, and only declared interactions. Source methods must use the selector values exposed by retained evidence rather than guessed JVM identities.

To inspect the complete retained call/evidence graph for one exact method without
authoring a bounded process definition, save a graph artifact from the same
snapshot:

```sh
clew docs process graph --root /work/architecture --service orders --declaration SYMBOL_OBSERVATION_ID --snapshot SOURCE_SNAPSHOT --output /work/orders-process-graph.json
```

The snapshot is required and this command does not capture source, save a
process definition, or run an author. Every retained FLOW event stays in its
original ordinal order; repeated callsites remain separate links, while each
resolved method body appears once and cycles point back to the visited method.
The artifact keeps complete retained source bodies and records missing or
ambiguous evidence as frontiers. Its overview may project an isolated direct
field getter/setter to a READ/WRITE item, but the full methods, callsites,
observations, and sources remain available in the same artifact. The
`knownReachableCollection` status describes exhaustion of uniquely resolved
same-scope links; `runtimeGraphCompleteness` remains `NOT_ESTABLISHED`.
Resource or write errors fail the command instead of producing a truncated
successful artifact.

```sh
clew docs recompose --root /work/architecture --snapshot SOURCE_SNAPSHOT
clew docs process inspect --root /work/architecture --input /work/process.json --snapshot ENTITY_SNAPSHOT
clew docs process list --root /work/architecture
clew docs process put --root /work/architecture --input /work/process.json --expected-input-digest INPUT_DIGEST
clew docs recompose --root /work/architecture --snapshot SOURCE_SNAPSHOT
```

The first recomposition returns `ENTITY_SNAPSHOT`, including entities written after capture; use it for inspection. The second returns `RECOMPOSED_SNAPSHOT`, including the process. Recomposition executes no analyzer and does not update latest. Pass `--snapshot RECOMPOSED_SNAPSHOT` to `docs process prepare --id PROCESS_ID --overview` or to `docs work prepare`. The default still selects latest, which predates the added declaration.

Prepare `/work/request.json` as `{"schema":"codeclew-documentation-work-request/1.0","audience":"Worker maintainers","entrypoint":"process-overview","contextProfile":"process-v1","maxItems":100,"maxBytes":40960}`. Use the ID authored in the process definition:

```sh
clew docs work prepare --root /work/architecture --subject scenario:PROCESS_ID --input /work/request.json --snapshot RECOMPOSED_SNAPSHOT
clew docs work read --root /work/architecture --work WORK_ID --input /work/selection.json
clew docs proposal submit --root /work/architecture --work WORK_ID --input /work/proposal.json
clew docs proposal publish --root /work/architecture --proposal PROPOSAL_ID --unassessed
```

The Work result includes `PROCESS_ROOT` handles for the process overview and process-specific behavior. Use the overview handle as a proposal operation target when `entrypoint` is `process-overview`; prepare another Work without `entrypoint` to author the process-specific root. The handles have `operation`/`gap` roles but not `evidence`: cite source and dependency handles delivered by recorded reads. `selection.json` names those handles, and a human/current agent authors the schema-constrained proposal from recorded reads. This is real authoring work, not an automatic consequence of capture. Alternatively, `docs work run --root /work/architecture --work WORK_ID --config /work/execution.json` uses an explicitly configured author/reviewer; do not invent provider credentials or assume it runs free. Local publication remains `UNASSESSED`; configured meaning review is separate.

For one captured Java HTTP endpoint, prepare Work with
`contextProfile: "endpoint-context-v3"` in its request before calculating Work
identity, then use `docs work run --root /work/architecture --work WORK_ID
--config /operator/operation-draft.json --draft`. This path calls one configured
author with the compact operation packet and writes `DRAFT` / `UNREVIEWED`
`answer.json`, `operation.md` and `index.html` beneath
`/work/architecture/.codeclew/drafts/WORK_ID`. The author-only config schema is
`codeclew-documentation-operation-draft-execution/1.0`; it contains `author`
and `budget`, with no reviewer, fallback or repair settings. The draft is not a
proposal and is never published by this command. See the Codeclew skill's
service-documentation reference for the preparation and config examples.

For an internal service callable, use `contextProfile: "process-graph-v1"`, an
exact scoped callable `rootDeclaration` observation ID from the selected saved
snapshot, and a persisted natural-language `question`; omit `entrypoint`. Run
the same `docs work run --draft` command with that Work and the author-only
configuration above. The packet contains retained internal method context and
explicitly labeled source candidates without inventing an HTTP endpoint or
claiming runtime dispatch. Draft status, saved-answer replay, and nonpublication
behavior are the same as for the endpoint profile.

To start from an already saved process definition, use the same immutable
snapshot selection through the process command:

```sh
clew docs process prepare --root /work/architecture --id PROCESS_ID --question "Explain the selected operation and its boundaries." --language en --snapshot SOURCE_SNAPSHOT
```

This prepares `scenario:PROCESS_ID` with `process-graph-v1`; `--language` accepts
`en` or `ru`, and `--question` cannot be combined with `--overview`. The chosen
snapshot must contain the frozen saved-process selection and one retained
callable matching its exact service, owner, and selector scope. Missing or
ambiguous matches report candidates and stop before authoring; use supported
`docs process put` followed by `docs recompose --snapshot ORIGINAL_CAPTURE` to
change the selected root, then prepare new Work from the recomposed snapshot.
The author packet carries the frozen title, summary, trigger, desired outcomes,
and declared continuations as user-requested intent. Desired outcomes are not
source-proven postconditions, and a declared interaction does not prove an
executed cross-service call. This route traverses retained graph evidence
independently of the saved process's `maxDepth` and `maxNodes` composition
limits; it does not recapture source or establish runtime dispatch, execution
order, external effects, endpoint exposure, or successful completion.
Previously prepared Work remains bound to its original process definition and
snapshot.

`proposal publish` and an accepted generic `work run` without `--draft` create an immutable internal bundle and update the working `docs/index.html` pointer (the frozen publication). The operation draft path does not. This does not automatically release a version. Open the bundle returned by a publishing command to read the new explanation. Do not immediately render the pre-author snapshot: an exact snapshot also retains its captured authored baseline, so that render can reproduce the earlier gaps instead of the newly accepted prose.

### Diagnose a draft run

Documentation commands write JSON progress events to stderr and keep their result
JSON on stdout. Save stderr to inspect the current stage and its duration. Each
event contains `timestampUnixMs`, monotonic `elapsedMs`, a `spanId` and its
`parentSpanId`. Long stages emit a heartbeat every five seconds.

Operation drafts distinguish packet preparation, author execution, driver
admission and startup, request delivery, response waiting, answer validation and
rendering, and output writes. `SEND_AGENT_REQUEST` completes when the full input
has been delivered; `WAIT_AGENT_DRIVER_RESPONSE` measures the subsequent wait
for the isolated driver, including any bridge, CLI and provider work. It does not
identify a provider queue or inference stage. Failed author stages emit `FAILED`
even when the command successfully saves a `DRAFT_UNCERTAIN` report. The logs
contain fixed stage names and timings, without source text or model reasoning.
Set `CODECLEW_DOCS_PROGRESS=off` to suppress these progress events.

### Recover a Work run

For a generic author/reviewer `docs work run`, retry the **same Work with the same execution configuration**. Intact, validated saved role results are reused. A dispatched call with no saved result may be retried within that run's original finite allowance; when provider usage is unknown, accounting keeps the maximum reservation. This is recovery behavior, not an exactly-once or provider-billing guarantee.

For `docs work run --draft`, a saved raw answer is reused to render or restore
the local draft without another author call. An invalid answer is retained with
its usage and receives no hidden repair call. A terminal `ANSWER_INVALID` result
may be revalidated only from its digest-bound saved author result after the
selected invocation identity, immutable input, checkpoint and report bindings
match; this makes no new driver dispatch, and an answer that still fails
validation remains invalid. A dispatch with no durable result is
reported as `DRAFT_UNCERTAIN`, retaining the maximum reservation; repeating the
command does not redrive it. Inspect the retained report and provider state;
then, when another attempt is intended, run
`clew docs work run --root <docs-root> --work <work-id> --config <draft.json> --draft --new-run`.
This creates one fresh run on the same saved Work and accepts only a terminal
unsuccessful draft: `DRAFT_UNCERTAIN` / `DISPATCH_UNCERTAIN`,
`DRAFT_INVALID_ANSWER` / `ANSWER_INVALID`, `DRAFT_CANCELLED` / `CANCELLED`, or
`DRAFT_FAILED` / `FAILED`. Its selected checkpoint, original config digest,
snapshot, packet digest, and execution mode must still match the retained run.
The old report, checkpoint, cancellation marker, and maximum reservation remain
available for audit; the new run gets its own run identity and reservation.
The new config and admitted driver may be corrected for this explicit fresh
attempt, subject to the selected budget account's immutable ceiling. Successful
drafts and nonterminal runs are not eligible. Without `--new-run`, normal replay
still reuses the selected run and refuses config or driver mismatches; a generic
run cannot be resumed in draft mode or the reverse.

For a terminal `DRAFT_INVALID_ANSWER` only, an operator may select that latest
retained run with `--repair-from-run <run-id>`. The new run reuses its exact
saved packet and answer, adds the fresh native validator error, and makes one
author call. Repeating the same repair command or plain `--draft` resumes that
repair run without another reservation or dispatch. A repair run cannot be
selected for another repair. This explicit path supports retained authoring
contract 1.3 Work; ordinary 1.3 generation and `--new-run` remain blocked.

Configuration, driver, evidence, read-ledger or publication mismatches return an explicit `RECOVERY_*` refusal. Inspect the retained report and current documentation state; do not delete saved run data, change configuration to force a retry, or repeat capture as a recovery shortcut. An accepted retry whose intended publication is still current can finish without another model call, bundle or history entry. If a later unrelated publication has changed the current output, recovery refuses conservatively and does not roll it back. Cancellation applies to its run only. A terminal run with recorded attempts is not an implicit fresh run or reviewer-only retry; retain and inspect its report and results before deciding what work to prepare next.

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
