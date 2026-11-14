# Prepared author: save, check, repair

The caller has admitted the runtime and retained sources. Keep this running task
on that runtime. One author/context works through related-page units; the complete
request remains required. Finish only the assigned unit, then end the turn so the
runner can check its cost and resume you for the next unit. Do not start a runner,
other models, capture, refresh, publish, or read source repositories/private stores.
The local skill profile and AGENTS.md select this prepared workflow; retain it
after context compaction rather than restarting the global generic workflow.

## Short working loop

1. Read the request, assigned unit and its source indexes. Open consequential code,
   contracts/settings and dependencies through the prepared readable views. Real
   cite handles are printed with the code. `inputs/source-proof` contains full
   proof/receipt sidecars for targeted diagnostics, not ordinary author context.
   Available-but-unread evidence is unfinished work; preserve actual source limits.
2. Edit one persisted `workspace/pages/PAGE.md.json` at a time. Each file already
   uses the ordinary original format with one page. Replace the unfinished draft
   placeholder with a reader explanation and evidence-bound diagram/field table.
   Extend the prepared entry sequence through actual work and outcomes. The first
   handler identity/address is fixed. Keep the original request's data/process scope.
3. Save with `./docs save-pages --input pages/PAGE.md.json`. This merges
   only those pages into `draft/original.json`; other pages/reads remain intact.
   It supplies managed `handler`/`clew-scope` anchors. Do not type those anchors in
   authored fragments or use headings that duplicate them. For additional anchors,
   use a dedicated plain `{text:"<a id=\"name\"></a>"}` fragment: save-pages owns
   its block separators. Keep a sequence fence within one bound fragment.
4. You ARE allowed and expected to run `./docs check --page PAGE.md`
   (repeat `--page` for a related group). It binds selected claims through the real
   source rules and uses the same final parser/renderer and actual HTML link checks.
   Read its compact result. Fix the named saved page/fragment/field, save and repeat.
   Check output and the last valid preview live under `workspace/checks`; a failed
   preview never replaces `last-valid-preview.json`. Unit checks are not READY.
   Success prints a compact summary; full receipts remain available on disk or
   with `--full` for targeted diagnostics. Do not reread assembled sequences.json
   after PASS; compare actual source with the relevant saved page and diagram.
5. Compare source to every explanation and diagram, then every visible claim back
   to source. Repair the same saved unit. Check predicates/precedence, before/after
   effects, failures, response/error variants, defaults and completion boundaries.
   A correct sentence does not excuse a wrong label. Then finish this unit's turn.

Create extra page files using the same `{pages:[{page,fragments}]}` shape and save
with the same command. Update ordinary navigation for added owned data pages.
Shared process/catalog pages stay unowned. Do not write a whole-site generator.
If a target belongs to an unfinished page, check reports `pendingTargets`; it does
not certify the link. Save the target's real anchor in its pending page file, or
finish the related page in the same group, then repeat check. Pending content stays
explicit and still blocks final readiness. Never invent an explanation or cite.
If a small script helps, save it before execution and repair that file locally.
Do not retransmit every page for a local failure.

The `complete-docset` unit finishes shared process/routes/data navigation, resolves
pending cross-page targets and runs the full bidirectional review. Only then call
`./docs review-template` and fill its actual assessments in self-review.json.
The caller retains the full source-binding/export/coverage gate and promotion.
A scaffold, UNIT_CHECKED, exit0 or DONE never means the complete site is READY.

## Reader and evidence quality

Lead with reader purpose, inputs and consequences. Use ordered work and errors
beside the step that fails; readable field tables preserve exact code identifiers,
meaning, type/requiredness, units and evidenced aliases. Group representations only
when their mapping is evidenced. Explain each material distinction once and link.
Keep proof labels, hashes and review protocols in metadata or source disclosure.

Follow consequential helpers, predicates, configuration and resource notes.
Distinguish local adapters from remote participants, local acceptance/handoff from
downstream completion, and source declarations from runtime registration. State
conditions on every affected sequence step. Do not invent waiting, rollback,
retry, acknowledgement, durability or collection order. A condition string is not
machine proof. A genuine citation is not semantic approval. If evidence is really
unavailable, state its consequence; do not relabel available unread source unknown.
Additional missing evidence uses the supported acquisition section of
`compose-original.md`/`thread-preparation.md`, saved under `draft/reads`.

## Files and compact contracts

`original.json` is an object with `pages`. Each page is
`{"page":"name.md","fragments":[...]}`. Use flat Markdown filenames. Headings,
anchors and navigation-only fragments use `{"text":"..."}`. A substantive claim
uses `{"id":"local-id","text":"...","evidence":["REAL_HANDLE"]}`; both annotation
fields are required together. IDs are unique within a page. Markdown fragments
concatenate literally, so retain newlines. Every page needs at least one meaningful
source-bound claim; fold a pure navigation hub into an explained overview instead
of inventing evidence for links. Extra process/data pages are allowed.

The saved entry page receives its managed `handler` anchor automatically. Its bound Markdown fence
named `clew-sequence` containing JSON:

```json
{"title":"Operation","participants":[{"id":"caller","title":"Caller"},{"id":"INVENTORY_HANDLER_ID","title":"Handler","target":"REQUIRED.md#handler"}],"steps":[{"from":"caller","to":"INVENTORY_HANDLER_ID","label":"METHOD /exact-path","kind":"call","target":"REQUIRED.md#handler"},{"from":"INVENTORY_HANDLER_ID","to":"caller","label":"Actual response","kind":"return"}]}
```

This example illustrates the contract, not an operation's behavior. Expand actual
work, branches and errors from source. Participants accept `id,title,target?`;
steps accept `from,to,label,kind,target?,condition?`. Kinds are `call`, `return`,
`async` (start without waiting), `await`, `note` (local step). Targets use flat
`page.md#anchor`. The first step addresses the inventory handler and its target;
HTTP labels are exactly inventory METHOD + space + PATH. The renderer derives
the diagram and text trace from these same rows; structural validity is not truth.

`navigation.json` is already started for you. Contract:

```json
{"schema":"codeclew-markdown-navigation/1","home":{"title":"Guide","target":"overview.md"},"services":[{"id":"SERVICE","title":"Service","overview":"overview.md","pages":["overview.md","entry.md"],"starts":[{"title":"Reader action","target":"entry.md#handler","kind":"http","summary":"What it does"}],"data":[{"title":"Object","target":"data.md"}]}],"routes":[]}
```

Kinds: `http,kafka,scheduled,background,manual,other`. Every service destination
must be one of its uniquely owned pages. Add actual data pages to `pages` and
`data`. Keep shared process narratives and the process catalog unowned by a
service, and keep legacy navigation `routes` empty when using `routes.json`.

For multiple services write `routes.json` using this field contract:

- Top: `schema:"codeclew-cross-docset-routes/1.1"`, `revision` (ASCII ID),
  `catalog` (local process-catalog destination), `docsets`, `externalActors`,
  `routes`. A destination is `{docset,page:"flat.html",anchor?}`; anchors omit `#`.
- Docset: `{id,title,version,baseURL,services:[{id,title,overview:DEST}]}`.
  Choose stable docset/version IDs (for example `docs`/`v1`) and an explicit HTTP(S)
  directory baseURL. The local service inventory must match navigation.
- External actor: `{id,title}`. Actor: `{kind:"service",docset,service}` or
  `{kind:"external",id}`. IDs are ASCII letters/digits plus `. _ -`.
- Route: `{id,version,title,narrative:DEST,origin,occurrences,branches,outcomes,coverage}`.
  Each route has a unique canonical unowned narrative page.
- Occurrence: `{id,actor,role,target:DEST}`. A known origin is
  `{status:"known",occurrence,kind,title,summary}` naming a service occurrence
  whose target and kind match an actual navigation start. Otherwise use
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

## Final same-author review

Use the generated compact review shell after the complete-docset originals exist.
It derives IDs, default sections and duplicate evidence bookkeeping, never answers.
Fill every requirement and claim disposition from actual comparison in both
directions. Read full required discovery views and record additional starts or an
evidenced no-additions judgment. Preserve unknown/runtime boundaries. Do not remove
requirements, citations or meaningful distinctions to obtain a check pass.

Record only observed browser inspection; otherwise use UNAVAILABLE with a reason.
Root reader/visual acceptance is separate. Keep originals, reads, failed local
checks and full provenance. No previous prose, rubric or critic answers are inputs.
