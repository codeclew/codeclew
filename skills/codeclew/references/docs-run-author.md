# Prepared author: meaning first, mechanical work delegated

The caller has admitted this runtime and retained scope. One author/context works
through related-page units. Keep the complete request and all source requirements.
A successor may instead assign one standalone full-docset review of preserved work;
then follow that full assignment without replaying service-unit authoring.
Finish the assigned unit or standalone review and end the turn; do not start runners, other models,
capture, refresh, publication, or source-repository/private-store exploration.
The workspace profile survives compaction: use it rather than restarting the
installed generic workflow. Proof receipts are for targeted diagnostics.

## Normal working loop

1. Read consequential source views through service indexes and the `Source views`
   paths printed by `show`. `views.json` indexes covering handles, not every native
   evidence candidate; never derive a view number from a cite. Inspect
   saved content with `./docs show --page PAGE.md [--fragment ID]`. Use `show
   --entries [--service SERVICE]` or `show --routes` for IDs, then `--entry ID` or
   `--route ID` for one object. Invalid IDs list choices; plain fragments need no
   ID. Follow the printed continuation for remaining characters, or use `--limit
   0` for all. Available-but-unread source remains unfinished work. Extract meaning
   from consequential source blocks directly into reader-ready canonical fragments:
   guards/branches, state before calls, success/failure/skip state, parse/default
   variants, outputs and proven order. Write each shared explanation once with
   genuine source bindings. Compare source with the canonical fragment as you
   extract it; existing prose is not the checklist of what behavior exists.
   When changing a summary, branch or arrow, compare the source block, canonical
   fragment and affected excerpt locally in both directions. Use `show --fragment`
   and the printed source views; do not copy whole sources or reread unchanged pages.
   Private scratch notes remain optional via `review-page --text-file NOTES.md
   --evidence HANDLE`. Do not write a second full private semantic body merely
   to restate reader-ready fragments. Never publish scratch or review metadata.
2. Write ordinary Markdown in a text file; save with `./docs put --page PAGE.md
   --fragment CLAIM --text-file FILE --evidence HANDLE` (repeat genuine evidence).
   Quotes/backslashes stay literal; the helper merges, serializes and anchors.
   `put`/`sequence` return `fragmentTarget`, a stable reader URL independent of
   heading text. Copy this URL for prose, participant and step destinations.
   `show --page PAGE.md --fragment CLAIM` also prints the prepared reader target.
   Use the returned `routeTarget` object directly for a route occurrence target.
   A known origin still uses its registered start from `show --entry ID`.
   To address a preserved claim, use `./docs page-info --page PAGE.md --fragment
   CLAIM`; it adds only managed structure, keeping claim text/evidence intact.
   Existing heading links remain valid. A bare `#CLAIM` remains a heading/HTML
   anchor reference, not a guessed citation ID. Do not write managed anchor HTML.
   Addressed fragments must be complete Markdown blocks; inline legacy claims
   are kept intact and require a deliberate block edit rather than guessed layout.
   The admitted renderer checks insertion context and the real HTML target before
   saving. Literal/raw/container interiors are refused; `show` does not advertise
   an address that fails this read-only check.
   For a semantic sequence JSON file use `./docs sequence --page PAGE.md --fragment
   FLOW --input FILE --evidence HANDLE`; the helper owns the fence. On an entry
   page, participant `handler` receives its registered ID, target and initial HTTP
   label. Supply all other participants, order, conditions, labels and outcomes.
   Replace the unfinished diagram; a stub is not accepted content.
3. Run `./docs check --page PAGE.md` (repeat page for a group). It uses the actual
   parser/renderer/source rules. Repair only named page/fragment/fields from the
   compact result; full receipts stay on disk and the last valid preview remains.
   After PASS, do not reread checker dumps or regenerate the docset.
4. Before recording each new/changed claim, compare source to its visible claim/step and each back to source, including
   predicates, helper calls, before/after effects, failures, response variants,
   defaults and completion boundaries. Check prose and diagrams. Get assessments
   with `./docs review-page --page PAGE.md > assessment.json`, fill claim and
   requirement dispositions, then record with `./docs review-page --page PAGE.md
   --input assessment.json`. Empty/null is not approval. Changed page hashes
   invalidate their reviews; unchanged reviews persist.

For a SERVICE review, set `sourceNotes` to `{"references": [{"page":
"SERVICE.md", "claim": "phase"}]}` after explicitly comparing each referenced
canonical reader fragment with its consequential source blocks. Choose existing
service-owned fragments with genuine evidence; this path needs no second text body.
Evidence and fragment digests are recorded by the helper. Missing, foreign,
unbacked or stale references fail; recompare changed fragments and submit fresh
page/claim references. Explicit null or malformed legacy notes are never replaced
by inference. Valid legacy private notes remain supported. Decision-only updates
reuse retained canonical references or saved own notes when `sourceNotes` is omitted.
Claim, requirement and topic dispositions remain explicit. These structural checks
record the author's review; they do not verify semantic truth.

Use `./docs register-entry --requirement DISCOVERY_ID --kind HTTP|KAFKA|SCHEDULED|OTHER
--page ENTRY.md --title "Reader action" --evidence HANDLE` for a genuine discovered
start (HTTP also requires method/path). The helper derives the local ID, handler
address, stub and navigation from this declaration once. Each entry has its own
page; link shared behavior instead of merging unrelated entries. To repair a wrong
evidence handle, repeat `./docs register-entry --requirement ID --page PAGE.md
--evidence HANDLE`; existing kind/title/identity are reused. Content and IDs stay intact; affected review is
invalidated explicitly. Do not edit managed entries.json. Registration is
an authored syntax finding, not compiler/runtime binding or semantic approval.
Review the parent discovery requirement and each discovered disposition explicitly.

For owned data pages use `./docs page-info --page PAGE.md --service SERVICE
--title "Reader title" --data`; the helper merges ownership/navigation.
For a data section inside an existing service overview, add `--fragment ID` to
that command. It preserves the source-backed fragment and targets its canonical
address. Data-topic review must reference that same section; page top does not
close an in-overview data topic. A dedicated data page may use its page target.
Shared process narratives/catalog remain unowned. Save reader titles/contents yourself.
For an existing start use `./docs page-info --page ENTRY.md --service SERVICE
--title "Reader action" --summary "Source-backed consequence"`. Identity, kind and
target stay fixed; literal starter instructions and kind-only titles block check.
The old `save-pages --input FILE` representation remains supported for targeted
advanced edits; no hand-written site generator is necessary in the normal path.

In the final related unit, finish shared data/process pages and necessary
cross-service reconciliation. Every canonical process narrative needs a meaningful
diagram retaining its own start, causal progression, actual reply/completion
boundary and consequential branches. Link detailed shared phases for external
effects, broker handoffs and outcomes instead of copying their downstream steps
into every narrative. A compact diagram plus those linked phases must cover the
complete causal path; unrelated starts keep distinct landings and diagrams.
Preserve route scope and search ownership. Review new/changed pages; reuse the
recorded semantic work on unchanged pages. Reopen dependencies when a changed
claim affects their meaning; hashes cannot prove cross-service consistency.
Do not repeat whole-site authoring or blanket reread all previous page JSON.
Each SERVICE review has a `topics` map, with exactly the inventory topic names.
For each, supply `{disposition: "EXPLAINED", page: "data.md", claim: "fields"}`
(valid JSON uses quoted keys). Refer to an actual service-owned, evidence-backed
reader fragment, not the aggregate clew-scope anchor. Register a grouped data page
or section's page in Data navigation. If retained evidence bounds an absence,
use EXPLICIT_UNKNOWN, the reader fragment explaining that boundary, and `reason`.
Evidence and fragment digests are derived; changed references require explicit
review again. A reference, table, note or recorded disposition does not prove
semantic completeness. Source-review references or notes and topic closure support your source-first audit;
the caller still independently assesses meaning and reader usefulness.
Use `./docs finish-review --input FILE` with the explicit global decisions
`{unresolved: [...], visual: {status: "UNAVAILABLE", reason: "..."}}` (valid JSON).
Only record REVIEWED after actual browser inspection. Requirement `disposition`
is EXPLAINED or EXPLICIT_UNKNOWN. A DISCOVERY requirement has a separate
`discoveryDisposition`: explicitly choose NO_ADDITIONAL_STARTS after reviewing the
full retained file and declaring an empty finding list, or ADDITIONAL_STARTS for
its nonempty declared findings. EXPLAINED is never a discoveryDisposition. An
empty registry does not establish that the source has no additional starts.
`review-page`, `check` and `finish-review` report every invalid discovery field
with its requirement/page, expected values and declared finding count. Repair only
those assessments with review-page, preserving other assessments and content.
Use `./docs --help` for the supported commands and this distinction. `review-template` remains
available to inspect all outstanding assessments, never to auto-approve them.
The caller still runs the full source/coverage/route/navigation/export gate.
UNIT_CHECKED, exit0 or a generated scaffold never means READY.

## Reader and semantic quality

Lead with purpose, inputs and consequences. Explain steps in order with errors
beside the step that fails; put technical details in disclosure. Field tables
retain exact code names, meaning, type/requiredness, units and evidenced aliases.
Group representations only when their mapping is evidenced. Keep proof metadata
out of reader prose and preserve source disclosure.

Treat a declared schema constraint, producer transformation and consumer predicate
as separate assertions. A constraint in one layer does not prove another enforces
it. Distinguish an absent input/default branch from a present malformed value:
follow parsing exceptions and local catches rather than assuming a fallback.
Distinguish application execution from external effects. A method called inside
a transaction is not a database operation; unfold its consequential file/network/
broker/database effects with correctly owned participants and ordering. Include
pre-operation guards and the state written on each terminal path. Do not infer
retry, rollback, acknowledgement, durability or collection order without evidence.

A real citation is not semantic approval; inspect consequential helpers and
settings. Use genuine explicit unknowns when evidence is unavailable, not to hide
available unread source. Additional needed evidence follows the supported
compose-original/thread-preparation acquisition contract, saved in draft/reads.
No old generated prose, evaluator answers or correcting second author are inputs.

## Consequential boundaries and readable process coverage

Before recording a semantic assessment, audit each consequential operation across
its boundary: state written before the call/handoff, successful state written
after it, and state retained on failure, skipped work, duplicate/no-op paths and
returns. Check absent/default and present/malformed paths separately. An early
return does not imply that independent later work in another handler is skipped.
Use source-to-text and text-to-source checks for these facts in prose and arrows.

Keep each HTTP method/path, operation identity and completion-order fact identical
across its entry page, process arrows and catalog declaration. Compare exact
source identifiers rather than renaming copied operations. A synchronous response
and independent asynchronous work have distinct completion boundaries. A response
that does not wait for background work proves no response-before-background order.

Give the reader a compact overview thread with links to readable phases and
technical details. Keep detailed conditions, side effects and failure paths in
those source-backed phases rather than one giant all-steps diagram. The overview
must still make the actual end-to-end progression and completion boundary clear;
links are not permission to drop consequential work or mark unread code unknown.
Existing put/sequence/page-info/route commands suffice for this organization.
Use readable field tables with exact code names and code-formatted literal values.
Explain consequential unavailable behavior once in reader-facing terms; keep
capture/proof metadata in source details rather than repeating it in the narrative.

Preserve every independent known local start in the catalog, even if starts share
explanation or detailed diagrams. One route's origin must not stand for another
trigger. The existing registry requires a distinct canonical narrative page per
route: use concise per-start landing explanations linked to shared phases rather
than duplicating their details or silently merging starts. Keep unknown activation
explicit; do not create a runtime start from an unbound source declaration.

## Sequence representation

Use ordinary `clew-sequence` JSON semantic rows, without writing the fence:

```json
{"title":"Reader operation","participants":[{"id":"caller","title":"Caller"},{"id":"handler","title":"Application"}],"steps":[{"from":"caller","to":"handler","label":"Input","kind":"call"},{"from":"handler","to":"caller","label":"Actual response","kind":"return"}]}
```

This is a shape example, not evidence about any operation. Participants accept
`id,title,target?`; steps accept `from,to,label,kind,target?,condition?`. Kinds are
`call`, `return`, `async` (starts without waiting), `await`, `note` (local step).
On non-entry process pages choose actual participants/targets yourself. Conditions
and valid structure do not prove semantic truth. Targets use flat page.md#anchor.
The helper binds both the diagram and text trace to your supplied source handles.

For multiple services, `./docs route --input FILE` merges one `{route: ROW, externalActors?: [...], catalog?: "processes.html"}` update. Supply catalog on the first call, after creating its real source-backed page with put. Missing catalog targets are explicit prerequisites, not implied scaffolds. Both `.md` and `.html` catalog inputs are stored canonically as `.html`. The helper owns top-level schema, revision, local docset (`docs`/`v1`), service inventory and serialization; it preserves other routes. In ROW, route id defaults to its narrative filename and version to `v1`; destinations may omit the local docset and use `.md` pages. Service actors may omit local docset. Supply actual semantic actor/occurrence/branch/outcome/coverage values using this existing route representation:

The normal input contains no top-level schema, revision, docsets, service inventory
or baseURL: the helper constructs them. Do not copy a full routes.json envelope
into this update. `catalog` selects an actual unowned catalog page on the first
call; it is not a request to assemble catalog metadata. A destination is
`{page:"flat.md",anchor?}` (local docset is automatic); anchors omit `#`.

- External actor: `{id,title}`. Actor: `{kind:"service",docset,service}` or
  `{kind:"external",id}`. IDs are ASCII letters/digits plus `. _ -`.
- ROW: `{title,narrative:DEST,origin,occurrences,branches,outcomes,coverage}`;
  `id` and `version` are optional stable overrides, not required author bookkeeping.
  Each route has a unique canonical unowned narrative page.
- Occurrence: `{id,actor,role,target:DEST}`. A known origin is
  `{status:"known",occurrence,kind,title,summary}` naming a service occurrence
  whose target and kind match an actual navigation start. Trigger kind is
  `http,kafka,scheduled,background,manual,other`, not inventory names such as
  HTTP_ENDPOINT. Local structure is checked before route persistence and final
  rendering; failures name stable route/outcome IDs and exact fields. Repair
  metadata with `./docs route --input FILE`, not by rewriting narrative pages. Otherwise use
  `{status:"unknown",reason}`; unknown participation does not mean no match.
- Branch: `{id,title,condition,target:DEST}`. Outcome:
  `{id,branch,kind,title,target:DEST,internal,end}`. Outcome kinds:
  `normal,error,handoff,unknown`. `internal` is
  `{coverage:"complete-for-this-outcome"|"partial"|"unknown",occurrences:[IDs]}`,
  excluding the origin and terminal occurrence. A known end is
  `{status:"known",ownerOccurrence,boundary,recipient}`; boundary is
  `return,publish,handoff,stop,other`. Recipient is an actor or
  `{kind:"unknown",reason}` or `{kind:"none"}`. An unknown end is
  `{status:"unknown",reason}`. Preserve outcome-specific ends and failures.
- Coverage: `{outcomeInventory:"complete-for-declared-scope"|"partial"|"unknown",scope,reference:DEST}`.
  Completion describes the declared source scope, never runtime proof.

For advanced canonical identity/catalog features, open only the relevant
`examples/markdown-renderer/README.md` section or `docset-routes.mjs` validator.


Preserve originals, reads, failed checks and full provenance. Structural membership
is not compiler/runtime or semantic proof. Never remove scope to obtain PASS.
