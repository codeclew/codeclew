# Prepared author: meaning first, mechanical work delegated

The caller has admitted this runtime and retained scope. One author/context works
through related-page units. Keep the complete request and all source requirements.
Finish the assigned unit and end the turn; do not start runners, other models,
capture, refresh, publication, or source-repository/private-store exploration.
The workspace profile survives compaction: use it rather than restarting the
installed generic workflow. Proof receipts are for targeted diagnostics.

## Normal working loop

1. Read consequential prepared views through each service index. Real cite handles
   accompany the code. Available-but-unread evidence is unfinished work. Preserve
   source coverage and runtime/registration uncertainty.
2. Write a small reader explanation as ordinary Markdown in a local text file.
   Save it with `./docs put --page PAGE.md --fragment CLAIM --text-file FILE
   --evidence HANDLE` (repeat evidence for each genuine handle). The helper merges
   one claim, serializes its envelope, and supplies block separators and anchors.
   Quotes/backslashes are literal; do not write a JSON string around the prose.
3. Write a semantic sequence object in a local JSON file and use `./docs sequence
   --page PAGE.md --fragment FLOW --input FILE --evidence HANDLE`. The helper owns
   the fence and serialization. On an entry page, use participant ID `handler`;
   its registered ID, target and initial HTTP label are supplied automatically.
   Other participants, ordering, conditions, labels and outcomes are your work.
   Replace the unfinished entry diagram; an initial stub is not accepted content.
4. Run `./docs check --page PAGE.md` (repeat page for a related group). It uses the
   actual parser/renderer/source rules. Read compact success; full receipts stay
   on disk. On failure use the named page/fragment/field and expected/actual values
   to repair only that content. Last valid preview remains available. Do not
   reread checker input dumps after PASS or regenerate the whole docset.
5. Compare source to all visible claims and steps, then every claim/step back to
   source. Include predicates, consequential helper calls, before/after effects,
   failures, response variants, defaults and completion boundaries. A correct
   sentence does not excuse an incorrect diagram. Record this page's decisions:
   `./docs review-page --page PAGE.md > assessment.json`, fill its actual claim
   and requirement dispositions, then `./docs review-page --page PAGE.md --input
   assessment.json`. Empty/null dispositions are not approvals. Reviews are tied
   to page hashes: changing content invalidates its review; other reviews persist.

Use `./docs register-entry --requirement DISCOVERY_ID --kind HTTP|KAFKA|SCHEDULED|OTHER
--page ENTRY.md --title "Reader action" --evidence HANDLE` for a genuine discovered
start (HTTP also requires method/path). The helper derives the local ID, handler
address, stub and navigation from this declaration once. Each entry has its own
page; link shared behavior instead of merging unrelated entries. Registration is
an authored syntax finding, not compiler/runtime binding or semantic approval.
Review the parent discovery requirement and each discovered disposition explicitly.

For owned data pages use `./docs page-info --page PAGE.md --service SERVICE
--title "Reader title" --data`; the helper merges ownership/navigation. Shared
process narratives/catalog remain unowned. Save reader titles/contents yourself.
The old `save-pages --input FILE` representation remains supported for targeted
advanced edits; no hand-written site generator is necessary in the normal path.

In the final related unit, finish shared data/process pages and necessary
cross-service reconciliation. Every canonical process narrative needs its own
full-chain diagram, including actual request replies, external effects, broker
handoff and consequential outcomes; local entry diagrams do not replace it.
Preserve route scope and search ownership. Review new/changed pages; reuse the
recorded semantic work on unchanged pages. Reopen dependencies when a changed
claim affects their meaning; hashes cannot prove cross-service consistency.
Do not repeat whole-site authoring or blanket reread all previous page JSON.
Use `./docs finish-review --input FILE` with the explicit global decisions
`{unresolved: [...], visual: {status: "UNAVAILABLE", reason: "..."}}` (valid JSON).
Only record REVIEWED after actual browser inspection. `review-template` remains
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

For multiple services, `./docs route --input FILE` merges one `{route: ROW, externalActors?: [...], catalog?: "processes.html"}` update. Supply catalog on the first call. The helper owns top-level schema, revision, local docset (`docs`/`v1`), service inventory and serialization; it preserves other routes. In ROW, route id defaults to its narrative filename and version to `v1`; destinations may omit the local docset and use `.md` pages. Service actors may omit local docset. Supply actual semantic actor/occurrence/branch/outcome/coverage values using this existing route representation:

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


Preserve originals, reads, failed checks and full provenance. Structural membership
is not compiler/runtime or semantic proof. Never remove scope to obtain PASS.
