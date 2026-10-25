# Source-bound authoring example

Start with proposal input, not a materialized Narrative. Replace each uppercase
reference with one returned and read in the current Work. The fixture in
`fixtures/durable-docs` and its CLI regression provide an executable example;
these placeholders are not executable evidence.

## Choose the existing authoring route

Resolve the installed `clew` and the active skill folder once. Scripts below live
beside that folder's `SKILL.md`; a repository copy can differ from the installed
package. Check the selected CLI version and helper availability. `clew upgrade`
updates the runtime, while `clew skill install` installs its bundled skill; use
`clew skill install --help` for the chosen agent/destination. Do not substitute a
source runtime or a private converter when a released capability is unavailable.

Use the existing [Work start route](service-documentation.md#authoring-start-route)
for the requested public operations; a section summary is not their completion.
For a complete thread, continue through [thread preparation](thread-preparation.md).
Its `THREAD_CONTEXT` path is Java/Spring-specific; Kotlin and other saved Work use
the [SYMBOL-to-body continuation](thread-preparation.md#saved-work-continuations-without-java-thread-context).
An unresolved call edge does not make a delivered candidate body unavailable.

| Requested output | Supported route | Completion boundary |
| --- | --- | --- |
| One operation draft with a configured isolated driver | `docs work run --root ROOT --work WORK --draft --config CONFIG` | Existing finite author calls and expanding authoring contract; no publication |
| Independent review of that saved draft | `docs work review-draft` with the installed command's configuration contract | A separate review result; preserve failed author/reviewer attempts |
| Human/assistant-authored local proposal | Recorded Work reads, `docs proposal submit`, then paginated `docs proposal show` | All required pages/parts and native diagnostics must be inspected |
| Local explanation from a validated operation answer | `docs work explain --root ROOT --work WORK --input ANSWER --output-dir NEW_DIRECTORY` | Local draft only; does not publish or attest runtime behavior |
| Prepared SOURCE Q&A/maintenance | [Prepared SOURCE helper](../README.md) | `scripts/prepared_source.py` is not a Work/proposal orchestrator |

Native draft, review and explanation commands require an eligible Work profile
in the installed release. A valid manual SOURCE Work does not establish that
eligibility. Preserve a profile rejection and use the supported manual proposal
route; do not relabel its evidence or pass a Proposal as an operation answer.

For a submitted manual Proposal requiring independent meaning review, use the
[bounded semantic handoff](semantic-repair.md). It carries exact candidate and
Work identities, source navigation, addressable issues and a finite repair;
full local assembly reuses submit/collect. External reviewer verdicts remain
separate from native meaning authority. Native READY is not semantic approval.

For native execution use the caller's selected model, driver and finite budget,
not a new model loop. The expanding coordinator drains content pages and SOURCE
parts. An oversized non-SOURCE record fails with `NEEDS_EVIDENCE`; do not describe
that run as complete or silently drop the record. Manual recorded reads can use
the supported dependency/retained part commands described below. `docs work
packet` and its audit output do not record Work reads.

For a complete thread, save one short source-to-sequence sketch in the assigned
output root **before** choosing nested steps: containing statements in source
order, callback braces, early/post-call guards, named exits and output-field
source arrows. Add actual read references and proposed step locations as you
build the draft. Walk an early exit, a post-call missing result, and a normally returning call
that skips its callback through both sketch and tree; then account for delivered helper failures
and response mappings. A contradiction changes the tree or requires a targeted
body read, not just a disclaimer. The [owned executable exercise](../examples/thread-order/README.md)
shows the short method and concrete counterexamples. Keep this review aid outside
the native Proposal; it is not a new response schema or meaning approval.

Before declaring evidence unavailable, check the proposed gap against already
delivered references and declarations. Preserve an unresolved-dispatch boundary,
but follow its returned candidate bodies, interface/default methods and material
effects with bounded reads. A navigation lead is unfinished acquisition, not
proof that its behavior is unavailable. For each relevant call, record actual
arguments, forwarded/default values and whether the call is inside a callback or
follows its return; a mode from one call does not apply to the next call.

When native grammar rejects a node, correct that exact node and compare the
structural diff before resubmitting. Keep every unaffected branch, condition,
return, effect and evidence binding. Moving branches into one prose claim
is not a grammar repair. Rewalk the affected path and one unchanged sibling;
the owned thread-order exercise supplies concrete counterexamples.

Before final submit, compare the draft with the actual request and supplied
evidence in one short pass:

- Keep every requested field and identity distinction, including exact supplied
  names, types, nullability and nesting. Distinguish correlation identifiers from
  result/business identifiers and entities from their request/response/event
  representations. Preserve an explicit gap for anything not established.
- Check each requested branch's condition, positive/negative outcome and return
  placement. Retain each claim's specific evidence; broader entrypoint or
  declaration references do not replace it.
- Compare the requested thread scope with delivered stage, guard, return,
  data-effect and result facts. Keep still-unread evidence tasks explicit; do not
  finish a full-thread request as a controller-only account while supported local
  continuation remains available.
- Before submit or a FLOW-related repair, map every delivered mandatory FLOW
  requirement to the exact nested step and its evidence handle. Keep IF conditions
  and each RETURN at their actual depth; a broader SOURCE or method handle does
  not substitute for a required FLOW handle. Compare against the complete
  delivered requirement set before repairing one fail-fast diagnostic. If a
  required record is omitted or unread, finish its supported reads first.
- State supported actions and outcomes alongside their limits. Distinguish
  initiation, acceptance/enqueueing and completion only where evidence establishes
  them. Preserve the supplied public-target scope separately from internal helpers,
  and report described targets and remaining gaps. Unknown whole-repository
  inventory does not erase a known captured scope or count.
- Keep summary text as nonblank plain prose of at most 2048 UTF-8 bytes,
  without backticks or `<`; put Markdown detail in explanation claims. Keep one
  assigned run root, including its complete subtree, and derive all input paths
  from it. For example, `CLEW_AUTHOR_OUTPUT_ROOT=/absolute/assigned/run-03` and
  `--input "$CLEW_AUTHOR_OUTPUT_ROOT/draft/proposal.json"` refer to that same run.
- Validate the final saved artifact, after its last edit. For JSON, run
  `python3 -I -S <skill-folder>/scripts/check_json.py FINAL.json` to reject syntax
  errors, duplicate keys and `NaN`/`Infinity` literals. This syntax check defines no
  business schema and performs no native or meaning review. Then use the actual
  native validator for the selected answer/proposal contract. A self-reported
  correction is not a successful validation. Preserve the first failed output.

Inspect process exit codes and structured native outcomes before reporting
completion. `NEEDS_REPAIR` blocks publication; `PARTIAL` retains its gaps. There
is no universal publication receipt. Inspect the exact bundle identity and the
route's supported baseline/receipt where supplied. Exact reviewed `publish-answer`
idempotency depends on its current publication baseline; do not generalize it to
other writes or rerun an uncertain model invocation.
Report publication success, coverage, source freshness and meaning review
separately. A confirmed publication can succeed with `PARTIAL` coverage and
`UNASSESSED` meaning; those limits do not cancel its confirmed bundle/result.
A supplied historical command transcript supports a conclusion about that
recorded run, not a claim that the current agent executed it again. Lead the
reader summary with supported behavior and outcomes, then their evidence limits.

Normal request: "Document this selected operation from its saved Work."
`debug=true` is an instruction convention for the same request and commands,
not a `clew` flag or a reason to make extra model calls. Save the sanitized
request, active skill/helper versions, selected Work/snapshot, original failed
and final artifacts, commands, native statuses, validation results and available
receipts in a new private directory outside the skill. Record concrete coverage
gaps and decision reasons, without private reasoning or invented authority.
Write `debug.json` with exactly these six top-level keys and no others:
`status` (`PASS`, `FAIL`, `BLOCKED`, or `NOT_RUN`), `summary`, `artifacts`,
`decisions`, `checks`, and `limitations`. Store receipts as files listed in
`artifacts`, with their result or absence described in `checks`/`limitations`;
do not add receipt-related top-level keys. Artifact paths are relative files
that actually exist in that directory. A decision records
`decision`, `basis`, `evidence`, and `assumptions`; a check records `name`,
`status`, `detail`, and `evidence`. Evidence names listed artifacts. A required
failed, blocked or unrun check prevents overall PASS. JSON validity and native
structure checks do not make semantic quality PASS.

For an already grounded exact RETURN citation gap, use the bounded
[evidence-only repair route](evidence-repair.md). It preserves the full proposal
and runs the native whole-candidate gate; meaning changes stay on the author/reviewer route.

## Proposal input for docs proposal submit

The host gets subject, context and immutable evidence from `--work WORK_ID`.
The `entrypoint` below is that Work's authorable reference. Claims use Work-local
`evidence`, and sequence `steps` are nested. The corresponding installed-version
contract is bundled as [proposal.schema.json](../schemas/proposal.schema.json).
The [contract index](../schemas/README.md) identifies its matching public runtime.
These are discoverable contracts, not a replacement validator: submit through
the selected native Work and inspect its diagnostics.

A manual service-section proposal can add an `explanation` array of source-bound
claims to its summary (up to 64 in this carrier). Section roots do not accept
sequence steps, participants or contracts. The isolated role's
[section-author.schema.json](../schemas/section-author.schema.json) is a different,
summary-only response contract; do not impose it on the manual proposal route or
pass a manual Proposal as that role response. Preserve each paragraph's specific
evidence and unresolved distinctions instead of compressing the whole section
into an oversized summary.

```json
{
  "schema": "codeclew-documentation-proposal/1.0",
  "operations": [{
    "entrypoint": "ENTRYPOINT_REF",
    "title": "Check out an order",
    "summary": {"text": "Reject invalid quantities; otherwise request a reservation and return its result.", "evidence": ["METHOD_REF"]},
    "steps": [{
      "kind": "alt",
      "meaning": {"text": "Quantity is zero or negative", "evidence": ["IF_REF"]},
      "children": [{"kind": "return", "from": "SERVICE_ID", "to": "caller", "meaning": {"text": "Return invalid quantity", "evidence": ["REJECT_REF"]}}],
      "otherwise": [
        {"kind": "message", "from": "SERVICE_ID", "to": "SERVICE_ID", "meaning": {"text": "Delegate to reserve(request)", "evidence": ["CALL_REF"]}},
        {"kind": "return", "from": "SERVICE_ID", "to": "caller", "meaning": {"text": "Return the reservation result", "evidence": ["RETURN_REF"]}}
      ]
    }],
    "contracts": [{"title": "Quantity input", "kind": "payload", "rows": [{"label": "quantity validation", "description": {"text": "A value at or below zero is rejected before delegation.", "evidence": ["IF_REF", "REJECT_REF"]}}]}]
  }],
  "uncertainties": ["The selected body does not establish the downstream reservation algorithm or wire schema."]
}
```

Use the selected service's exact ID for `SERVICE_ID` in proposal step `from`/`to`.
The host supplies `caller` and selected services automatically; it materializes
service participants as `service-<id>`. Proposal `participants` declares only
additional actors backed by the selected evidence. Do not redeclare an implicit
actor to choose its label, copy participant IDs from another Work, or put the
materialized `service-<id>` alias into a proposal step that expects the service ID.

Only `alt`, `opt` and `loop` steps may contain `children`; only `alt` may contain
`otherwise`. Other step kinds must leave both arrays empty. When preparing a
compact model packet, include these rules and the relevant
[interaction rules](#declared-interactions) below, rather than an arbitrary
prefix of this reference. The JSON carrier schema alone does not enforce every
native condition. Preserve the full recorded evidence delivery separately.

The example is complete only for its stated small source body. Real Work may
require other conditions, returns and evidence. Machine acceptance checks
structure and coverage, not whether the business explanation is true.

## Materialized Narrative output, not proposal input

The host assigns stable IDs and materializes Narrative 1.3 (1.4 for explicit external boundaries). Do not construct
this output by renaming proposal fields or submit it as proposal input:

| Proposal input | Materialized Narrative |
| --- | --- |
| `entrypoint`: current Work reference | operation `id`: canonical target identity |
| claims with `evidence` references | claims with bound `dependencyIds` and `sourceIds` |
| nested `steps`, `children`, `otherwise` | ordered diagram `events` with assigned IDs |
| `contracts` and row `description` claims | `interfaceContracts` and source-bound rows |
| optional additional `participants` | implicit caller/services plus additional actors |
| host-bound `--work` | `subject` and `contextDigest` |

For retained source `if (quantity <= 0) return "invalid"; return reserve(request);`,
keep the rejecting return in one branch and delegation/result in the other.
Source evidence of delegation does not establish the downstream algorithm.

### Preserve a retained flow when preparing a new proposal

There is no supported lossy Narrative-to-proposal field-renaming converter.
Read the retained operation and submit the closed proposal shape above through
the current Work. Keep the original input separately. For a mechanical adapter,
require an exact tree comparison before submission:

- `alt`, `opt`, and `loop` each open their own stack frame. `end` closes only
  the innermost frame. An `else` belongs to the currently open `alt`; closing a
  nested `opt` must not close its outer `alt`.
- Preserve conditions, sibling order, nested `children`, `otherwise`, and every
  return at its original depth. A return following a group's `end` stays outside
  that group. Never turn the negative branch into an unconditional action.
- Reject unknown kinds, unmatched `end`, misplaced `else`, and unclosed groups
  with the input position. Do not repair malformed streams by guessing. Proposal
  steps contain nested groups, without explicit `else` or `end` events; the host
  generates those events during materialization.

Use separate typed maps for catalogue identities and Work references. An exact
source ID maps only to a delivered `SOURCE` record with that ID in this Work;
an interaction ID maps only to its delivered `DECLARED_INTERACTION` record.
If input already contains a Work reference such as `s2`, validate its kind and
record and keep it unchanged. Require exactly one match. Missing, ambiguous,
foreign, or unread evidence stops conversion of that claim with an explicit gap;
never filter it out or substitute the operation's broader entrypoint reference.
Retaining a factual assertion requires retaining its specific evidence. Declaration
evidence supports a declared boundary, not source reachability, guards, order,
receiver execution, or delivery.

### Guard local Work inputs and resume the same Work

Use `scripts/work_session.py` with one absolute caller-owned `--input-root`.
Every relative input, saved Work page and fresh output resolves from that root,
independently of cwd. A missing or foreign path stops locally before any native
call; correct the path rather than preparing new Work. Existing outputs are never
overwritten. The helper archives exact input bytes and raw native failures first.

```sh
python3 -I -S <skill-folder>/scripts/work_session.py read --launcher /installed/clew --root DOCS_ROOT --input-root OUTPUT_ROOT --work WORK_ID --input selections/helper.json --output reads/helper-01
python3 -I -S <skill-folder>/scripts/work_session.py submit --launcher /installed/clew --root DOCS_ROOT --input-root OUTPUT_ROOT --work WORK_ID --work-page work-initial/page-0001.stdout --input draft-002.json --output submit-002
```

`read` uses native Selection JSON unchanged, including the original query and
cursor. A resumed cursor is labeled `TAIL_RECORDED`, not full selection delivery.
`submit` accepts the existing native Proposal, checks every additional participant
against a saved page from that service Work, submits once and collects every
proposal page. After native feedback, edit a new proposal file explicitly and
repeat `submit` with the **same Work** and a new output. An idempotent `prepare`
does not prevent resubmission. The helper never edits branches, prepares Work or
publishes; `NATIVE_RESULT_RECORDED` can still mean `NEEDS_REPAIR`.

`caller` and the service ID are predefined step actors. Leave them out of
`participants`; use them in step `from`/`to`. Declare only additional participants,
for example `{"id":"provider","label":"Provider","externalService":"registered-provider"}`
when that external service is registered in the Work. A preflight is not native
schema or meaning acceptance; external declarations are never auto-deleted.

Search already archived Work evidence before claiming a fact is unavailable:

```sh
python3 -I -S <skill-folder>/scripts/work_evidence.py --input-root OUTPUT_ROOT --work WORK_ID --collection work-initial --collection reads/helper-01/read-collection --query ACCESS_ROLE --max-bytes 1024
```

Pass `--page` for individual saved native Work pages or SOURCE parts. This offline
view accepts Work-page schemas, not prepared SOURCE docs-pages. It returns exact
references, available source locations/revisions/authority, origin pointers and
bounded text. Follow view `nextCursor`/`nextOffset` or native part continuation;
inspect omissions and missing source windows. SYMBOL records and navigation are
separate from SOURCE text. A negative search can establish absence only in an
explicit complete selected source, not the service. Archive presence and returned
view ranges do not establish delivery to a model or that the model read them;
those states remain `UNATTESTED` without external delivery evidence.

### Finish reads and verify publication

The initial `docs work prepare` page uses the default selection `{}`. Its
continuation input is only the exact cursor, without `entrypoint`, `references`,
`symbols` or `query`:

```json
{"cursor":"EXACT_CURSOR_FROM_PREPARE"}
```

```sh
clew docs work read --root ROOT --work WORK_ID --input initial-next.json
```

Alternatively, finish that initial selection with the bounded bundled helper:

```sh
python3 -I -S <skill-folder>/scripts/collect_work_initial.py --launcher /installed/clew --root ROOT --work WORK_ID --output NEW_PRIVATE_DIRECTORY
```

It replays `{}` then sends cursor-only continuations, recording native read
receipts. It shares the proposal collector's bounded transport and failure
handling. It never prepares another Work, adds selectors, expands dependencies,
authors or publishes. `COMPLETE` means only that initial/default pages were fully
delivered; it does not complete downstream evidence acquisition or the thread.
Raw responses and each selection/command remain local. Omitted records leave an
explicit incomplete result; use the matching native part route below, then let
native validation check recorded coverage. The helper does not resolve parts.

Start a filtered query/reference selection without the initial cursor. Continue
that new selection with its own cursor and the identical query/references/symbols;
never mix cursors from different selections. A Work cursor is not an entrypoint.

For each required selection, keep the selector unchanged and follow `nextCursor`
until it is explicitly null. This includes `DECLARED_INTERACTION` queries. An
empty page can still have a continuation. For `omitted` records, use the matching
supported part command: `docs work read-part` for SOURCE,
`docs work read-dependency-part` for DEPENDENCY, and
`docs work read-retained-part` for retained content. Drain each part continuation
and keep its exact kind, ID/reference, Work, snapshot, ranges, digests, and receipts.
A repeated cursor, exhausted local loop bound, malformed/schema-invalid response,
failed subprocess, or missing terminal page is an incomplete read, never success.
Use a new private temporary directory per attempt and retain failed command results.

For the manual route, collect all saved proposal result pages with the bundled
read-only helper. It issues only `docs proposal show`; it never authors, submits,
rebinds, retries failed commands or publishes:

```sh
python3 -I -S <skill-folder>/scripts/collect_proposal.py --launcher /installed/clew --root ROOT --proposal PROPOSAL_ID --output NEW_PRIVATE_DIRECTORY
```

`manifest.json` separates collection `COMPLETE`/`INCOMPLETE` from native status,
meaning review and diagnostic counts. `items.json` preserves ordered full rows;
`diagnostics.json`, `omitted.json`, and each raw stdout/stderr and command result
remain local. It verifies the same proposal, Work, read/narrative digests and
status across pages and requires an explicit terminal null cursor and exact row
count. An empty nonterminal page requires continuation. Repeated cursors/items,
omitted records, changed identities, process/JSON failures or a helper limit leave
the collection incomplete. Inspect those saved results; do not filter failed rows
or call them a complete Narrative. Exit zero means delivery completed, including
when native status is `NEEDS_REPAIR`; it never means semantic approval. Default
helper bounds are 1,000 pages and 120 seconds per command; reaching them stops
without automatic retry. A new attempt requires a new output directory.

Check each prepare/read/submit/publish command's exit code and structured status.
A submitted proposal is not a publication. After `docs proposal publish
--unassessed`, require the supported render result's schema, bundle identity,
expected proposal ID, and meaningReview classification. Confirm that
`docs/index.html` selects that same bundle and its `bindings.json` and
`publication.json` exist. Where the supported coordinator supplies a publication
receipt, verify its bundle and recorded hashes against those final files. A
missing/mismatched identity or receipt leaves publication unconfirmed. `PARTIAL`
must retain its gaps and update failures; never report it as complete. Neither a
zero exit code nor printed text such as "published" establishes semantic approval
or runtime correctness.

### Declared interactions

Materialized event kinds are `message`, `return`, `note`, `alt`, `else`, `loop`, `opt`, `end`,
and `declared`. A resolved cross-service request uses `kind: declared` and
participants with the two captured service IDs. Proposal input must use the
interaction reference delivered by a recorded read in that same immutable Work:

```json
{"kind":"declared","from":"orders","to":"inventory","interaction":"INTERACTION_REF","meaning":{"text":"Request a reservation.","evidence":["INTERACTION_REF","CALL_REF","RECEIVER_REF"]}}
```

For example, if the read delivers reference `d141` for dependency
`interaction:reserve-inventory`, replace `INTERACTION_REF` with `d141`. The host
then writes `interaction: reserve-inventory` in the materialized event. Neither
the catalogue ID `reserve-inventory` nor dependency ID
`interaction:reserve-inventory` is a proposal reference. References such as `d141`
are local to their Work; the same spelling in another Work can name different
evidence. Read and verify the returned record rather than copying a handle.

In service Work, query `DECLARED_INTERACTION` to obtain declarations whose
`from.service` or `to.service` is the selected service. Their visibility is
boundary evidence, not permission to read another service's implementation or
draw an executable arrow. A declared arrow requires scenario Work with an
actually composed transition, `RESOLVED` endpoints and call site, and an admitted
declaration. `SOURCE_MATCH` and `AMBIGUOUS` do not satisfy this requirement.
Bind the edge to the declaration and the returned call/receiver dependencies.
A corresponding HTTP reverse return also names that interaction. Kafka delivery
has no synchronous return; declare a separate reply-event interaction. Nest receiver
conditions within the caller's successful branch. Do not omit failures just
because the named scenario focuses on success.

## External HTTP, Kafka and logical boundaries

For complete service/thread results, first follow [boundary and domain closure](boundary-completeness.md). Do not use the conservative narrative-only repair path to conceal missing participants or catalogue structure.

An `external: true` interaction has no captured receiver. Source-observed caller
behavior stays in source-bound steps; it never proves receiver execution or
runtime delivery. For an explicit declared arrow, register the destination using
`docs external put`, set the interaction's `externalService` and `to.service` to
that registered ID, select it in the process, and recompose. Prepare a fresh Work
and read its `DECLARED_INTERACTION` evidence. Declare an additional participant
with `externalService` instead of `service`:

```json
{"participants":[{"id":"provider","label":"External provider","externalService":"REGISTERED_EXTERNAL_ID"}],"steps":[{"kind":"declared-boundary","from":"SERVICE_ID","to":"provider","interaction":"INTERACTION_REF","meaning":{"text":"Declared request; receiver execution is not captured.","evidence":["INTERACTION_REF"]}}]}
```

This is an operation fragment, not a complete proposal. Both reference fields use
the same delivered Work handle, never a raw catalogue ID. Boundary meanings may
cite that declaration without source bytes; ordinary source-derived messages,
summary and mandatory FLOW branches keep their existing evidence requirements.
The host materializes Narrative 1.4 and labels boundary arrows as declared with
runtime unverified; endpoint status and origin remain unchanged. A service Work
can declare only its outgoing boundaries; a scenario must explicitly select them.
Unknown destinations, mismatched direction and foreign/undelivered handles fail.

Use `declared-boundary-return` only for a separately authored HTTP response, with
reversed participants and the same interaction. No response is automatic. Kafka
synchronous return is rejected; a separately declared outgoing reply event needs
its own interaction. Inbound external discovery is outside this model. A plain
`declared` step still requires the older resolved composed-transition contract.
The graph and sequence participants link to the same bundle's external pages;
use the adjacent text links if a diagram host disables embedded navigation.

When the destination cannot yet be identified, the following is an explicitly incomplete source-only fragment. It does not satisfy complete-thread or service-egress acceptance. Register a stable logical destination and explicit boundary arrows when the available evidence supports them. A source-only fragment has this shape:

```json
{
  "schema":"codeclew-documentation-proposal/1.0",
  "operations":[{
    "entrypoint":"ENTRYPOINT_REF",
    "title":"Request an external reservation",
    "summary":{"text":"Delegate the reservation request and return the client result.","evidence":["METHOD_REF"]},
    "steps":[
      {"kind":"note","from":"SERVICE_ID","meaning":{"text":"Call the external client; receiver execution is not captured.","evidence":["CALL_REF","INTERACTION_REF"],"uncertainty":"Receiver implementation and runtime delivery are unknown."}},
      {"kind":"return","from":"SERVICE_ID","to":"caller","meaning":{"text":"Return the client result.","evidence":["RETURN_REF"]}}
    ],
    "contracts":[{"title":"Caller request","kind":"payload","rows":[{"label":"Request argument","description":{"text":"The caller forwards the request argument to the client.","evidence":["METHOD_REF"]}}]}]
  }],
  "uncertainties":["The external receiver and its failures are not captured; runtime delivery is UNKNOWN."]
}
```

Replace all references with delivered evidence and `SERVICE_ID` with the selected
service. Add any actual branches, failures and returns required by its source.
For Kafka, describe the source-observed publish and the caller's own return;
that return is not a synchronous consumer reply. A source-derived contract must
separate observed arguments from engineer-declared transport facts. If an entire
operation cannot be explained, use its Work-keyed proposal `gaps` entry instead
of proposing that same operation. A partial receiver limitation belongs in
`uncertainty`/`uncertainties`; an operation and a gap for its target conflict.

A proposal gap is keyed by its current Work reference; the host materializes
Narrative `gaps` under the canonical entrypoint ID, with an
explanation of what must become available. Empty prose, an unbound diagram,
or an endpoint catalogue row does not satisfy full-scope documentation.

The overview should describe business decisions, not repeat every source step.
For a direct Narrative 1.3 import, mark implementation-only explanation paragraphs
with `detail: true`. This is a Narrative field, not a proposal Claim field. Their step coverage and evidence remain validated, but the UI
keeps them inside the detailed view. Keep important failures and asynchronous
boundaries in the short default overview. The renderer folds reference lists and source commentary. For a direct Narrative 1.3 import, add
a source-bound `overviewDiagram` with at most 12 nodes; its grid, node and edge
fields are defined in the service-documentation reference. A large retained
event list must never become the reader-facing diagram.

Use proposal `contracts` (materialized as `interfaceContracts`) for source-derived
HTTP, Kafka and payload facts when
OpenAPI is absent or incomplete. Proposal rows contain `label` and a `description` claim with Work-local
evidence; the host assigns row IDs and materializes their values/bindings. Include the real transport and complete
in-scope payload shape, not just the single rule in this schematic example.
The renderer labels these cards as source-derived; they never become a published
OpenAPI contract merely because they have source references.
