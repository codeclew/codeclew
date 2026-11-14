# Prepared documentation author

Use this entry point only when `docs_run.py` has prepared retained source views,
an inventory and an output directory. The runner owns source preparation and
compose/bind/render/export. Write the requested explanation and review its meaning;
do not repeat discovery, copy machine proofs or run those gates yourself.

## Read and explain

Read the original request, compact inventory, service indexes and consequential
source views. Views contain original code and real citation handles. Requirement
IDs, required page names and handler IDs are fixed; use them directly. All source
and proof sidecars remain available. Open a raw sidecar only to investigate a
specific missing association; printing its complete registry adds no understanding.
The inventory is a minimum source-level checklist, not the whole reader request.

Explain purpose and reading order; every requested entry and its interaction;
domain objects and individual fields; request/response/error/message shapes;
external operations, configuration keys/defaults, branches, effects and failures.
Follow consequential helpers, constants, predicates, contracts and resource notes.
Small data-origin/unit/default notes can matter even when bulk examples do not.
Available-but-unread evidence is unfinished work. If something genuinely remains
unavailable, explain the specific consequence without claiming runtime proof.

Lead with what the reader can do and what changes. Use ordered actions, errors
beside the failing step and concise field tables. Preserve exact source field
identifiers in code formatting, meanings, types, requiredness, units and aliases;
combine representations only when their mapping is evidenced. Describe each
important distinction once and link to it. Keep technical proof labels and review
protocols in metadata/the Sources disclosure. Do not use a caveat to excuse a
visible claim contradicted by source.

For HTTP, explain each method/path through validation, meaningful helpers,
conditional effects, response and errors. Start its sequence at that exact
handler. Distinguish local adapters from remote participants. A normal call does
not prove waiting; a sender call does not prove broker acknowledgement. State
conditions on each affected step, including skip/error paths. Distinguish local
acceptance, background handoff and downstream completion; do not invent rollback,
retry, durability, registration or iteration order. Link populated local process,
domain, contract and integration explanations. DataFlow is needed only if asked.

Acquire additional missing retained evidence only through supported skill APIs;
start with the relevant section of `compose-original.md` or
`thread-preparation.md`. Preserve snapshot, coverage, original authority and saved
manifests under `draft/reads`. Do not read target repositories/private caches,
capture again, consult prior prose or reviewer answers, or launch other models.

## Files and compact contracts

`original.json` is an object with `pages`. Each page is
`{"page":"name.md","fragments":[...]}`. Use flat Markdown filenames. Headings,
anchors and navigation-only fragments use `{"text":"..."}`. A substantive claim
uses `{"id":"local-id","text":"...","evidence":["REAL_HANDLE"]}`; both annotation
fields are required together. IDs are unique within a page. Markdown fragments
concatenate literally, so retain newlines. Every page needs at least one meaningful
source-bound claim; fold a pure navigation hub into an explained overview instead
of inventing evidence for links. Extra process/data pages are allowed.

Each required entry page has `<a id="handler"></a>` and a bound Markdown fence
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

## Review and finish

Use the prompt's `review-template` command after originals exist. It derives IDs
and duplicate claim-evidence bookkeeping, never semantic answers. Fill every
required assessment from your actual review. Compare request/input to pages and
every visible claim/sequence condition back to source; a correct sentence does
not excuse a contradictory diagram label. Preserve conjunctions, negations,
thresholds, precedence, failure effects and completion boundaries. Reconcile
entry inventory, handler sequences and integration participants, then repair the
actual originals. Do not manufacture citations, remove requirements or substitute
unknowns for available evidence to satisfy a machine check.

Record only observed visual inspection; use UNAVAILABLE with a reason otherwise.
The runner performs final mechanical gates. Their PASS does not assess meaning,
and your exit code or DONE is not READY. Keep originals and provenance intact.
