# Run retained documentation with fixed gates

This bounded runner uses the ordinary source-read, original/compose, citation,
component, renderer and export helpers. It never captures, refreshes or publishes.
The calling agent fills a job and a short launcher; it does not generate validators.
A source helper treatment is experimental until included in an official package.

Create a job matching [docs-run-job](../schemas/docs-run-job.schema.json). Pin the
supported clew launcher, exact saved snapshot, selected services, language, current
skill and unchanged packaged renderer. Install that renderer's locked dependencies
using its documented npm command. Configure the existing local Codex CLI and exact
authorized model/effort; no new credentials or hosted account are required.
When the admitted clew state lives outside the author workspace, set author.stateRoot
to that existing state directory so supported native lock/receipt writes are allowed.
This grants no permission to inspect/edit private cache objects directly. The launcher is only:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/docs_run.py" prepare --job /absolute/job.json --run-dir /absolute/new-run
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/docs_run.py" handoff --run-dir /absolute/new-run --page current.md
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/docs_run.py" run --run-dir /absolute/new-run
```

Prepare records the complete paginated native trigger/file inventory before any
author starts. A retained executable file also has a discovery obligation: source
syntax can register an imperative listener that native trigger detection misses.
The same author must report additional starts or an evidenced no-additions result;
a script cannot establish that this judgment is true. Discovered starts remain
explicit authored syntax, not compiler or runtime registration evidence.

Prepare also serializes the existing retained-source helpers into a source input
kit before the author starts: a small service/requirement index, directly addressed
verbatim region views and genuine ready citation handles. Raw packets, coordinates,
digests and authority remain in sidecars. The source-union helper combines only
compatible agreeing occurrences; missing coordinates or conflicting overlaps stay
separate. Every original citation reconstructs its exact original bytes. A compiler
Work uses its returned symbol/source references and retained-resource query, not an
implicit switch to a source profile. Partial coverage and unavailable resources
remain explicit. No previous authored pages or reviewer prose enter this kit.

The source-input preparation profile has a 300-second wall limit, 128 files and
8 MiB of readable views/indexes, plus the existing 128 MiB/4096-file artifact
closure limits. Native reads run serially and timed-out helper process groups stop.
Exceeding a limit blocks preparation; it never silently truncates the source or
inventory. The author opens relevant individual views rather than replaying whole
bulky fixtures, lockfiles or raw packet metadata. Additional evidence still uses
the supported retained-read route. Frozen prepared manifests are included by the
runner automatically; authors must not copy them into their extra read directory.

Discovery reads must reconstruct complete native Git blob bytes using genuine
retained SOURCE occurrences and the existing agreeing source union. Syntax/input
fingerprints are not text digests. Use ordinary read-files for languages without
whole-file helper support; an existing full-file SOURCE is sufficient. Preserve
all handles and authority. A gap, unavailable occurrence or unmatched file object
blocks discovery; never pad gaps, read source privately or recapture as a shortcut.

Readable views live under inputs/author-source; native packets, receipts and full
binding metadata live under inputs/source-proof. Both are frozen together.

Prepare creates unfinished pages, entry anchors and navigation from the inventory,
using the existing original and sequence formats. This structure contains no
semantic explanation. The caller prepares a handoff for the current page, then
explicitly runs that one page in a fresh inference session with the same model.
The handoff reuses complete admitted review-context source views and includes the
draft, own notes, pending decisions and full multi-service scope/source index.
Required uncited input stays discoverable; completed-page bodies and conversation
logs are absent. Add --source-view SERVICE/VIEW for explicit complete selections;
required views cannot be omitted. One run returns PAGE_COMPLETED or local failure,
never launches the next page or final promotion. Repeat with the next page. A
run without a handoff refuses before author launch; old history is never silently
resumed. Repeating a consumed run returns its recorded outcome. Explicit
--renew-context prepares another turn; it never launches one by itself. All
contexts and repairs retain the original aggregate time/token budget and attempts.

The author edits persisted workspace/pages/PAGE.md.json files and calls
`save-pages --run-dir RUN --input FILE` to merge only those pages. Managed anchors
and standalone anchor blocks are separated safely from adjacent Markdown. Call
`check --run-dir RUN --page PAGE.md` (repeat --page for a related group) before
advancing. It uses the final parser, renderer and source-binding rules, reports
the affected page/fragment/field, preserves other pages and reads, and keeps the
last valid preview after a failed edit. Pending cross-page targets remain explicit
failures until their real anchors are saved. Unit checks and `check --scaffold`
never mean READY; unfinished pages block final verification.

The runner builds its prompt from the request, admitted inputs and versioned
contract. The author also fills draft/navigation.json, routes.json for
multiple services, retained citation reads under draft/reads, and the bidirectional
[self-review protocol](../schemas/docs-run-review.schema.json). Required entry
pages and handler identities are fixed; additional discovered entry accounts cannot
replace them. The runner uses the real Markdown lexer and packaged sequence
validator, then compose/bind/finalize/export and source membership checks.
Author-written PASS reports are ignored. No gate command or skip list comes from
the job. Native declaration component tickets, when supplied, keep their own cells,
coverage and authority through the existing validators.

After writing annotated originals, the author can run `review-template --run-dir
RUN` and save its JSON as draft/self-review.json. This read-only template command
works during the active author invocation; it performs no gate or promotion.
It checks frozen input hashes without native commands or receipt writes. Runtime
identity and parser checks remain mandatory in runner verification and promotion.
The compact 1.1 review maps stable requirement IDs to explicit semantic answers,
and maps page/fragment IDs to explicit claim dispositions. Pages, duplicate claim
evidence and default entry/discovery cite candidates are derived from immutable
inventory and annotated originals. Original source choices and explicit evidence
overrides stay intact. Overrides accept real cite handles, never raw SOURCE IDs;
ambiguous or foreign evidence is not guessed. Same-page #anchor syntax normalizes
to its local anchor; cross-page references refuse. Noncanonical sections/diagrams,
semantic dispositions, unknowns and discovered starts remain the author's work.
The template leaves those answers null and cannot establish semantic completion.
The full canonical 1.0 review remains supported, with the same evidence guards.

Repairs receive the current page's machine failures and unchanged source reads in
an explicit handoff, subject to the job's repair limit. Each attempt, actual
thread identity and failed draft stays in the run. No interrupted, nonzero or
over-budget attempt is silently retried or made complete through a context reset.
The primary token budget is cumulative uncached input plus output:
`input_tokens - cached_input_tokens + output_tokens`. Preserve raw input, cached
input, output and available reasoning-output counters separately; reasoning is
already included in output and is never added again. Total input/output footprint
remains separate from the budget and is not a dollar-cost estimate. A missing cache
counter makes the primary metric explicitly unavailable; invalid counters refuse
completion. Known usage supplies a lower bound that can prove an over-budget
failure; it cannot prove a within-budget pass. Unavailable primary usage blocks
READY and another author invocation, preserving the draft for diagnosis.
Set maxModelTokens to null when no token cutoff was requested: measured usage
remains mandatory, including every same-author stage and repair. Numeric cutoffs
remain supported. wallSeconds is an explicit operational anti-hang guard (up to
7200 seconds), separate from quality and efficiency acceptance.
Counters arrive at turn completion, so a token limit is checked
then and prevents readiness/further calls; it is not a provider billing guarantee
or an enforceable token ceiling inside one CLI turn. Actual wall timeout is enforced.
Missing counters remain unavailable; bytes are never presented as tokens or cost.

Codex CLI terminal usage is cumulative within the same thread across resumes.
The runner preserves every raw report, derives turn deltas within that thread and
counts its latest total once. Different threads sum. Missing thread attribution
across multiple reports, missing required counters, resets and inconsistent deltas
cannot establish exact cost or pass a budget gate. A single historical report is
still usable without subtraction.

For a proven false stop from the previous cumulative-counter summation bug, a
trusted caller may use `recover-accounting --run-dir RUN --previous-skill OLD_SKILL`.
It validates the old admission and saved CLI history, refuses genuine overspend,
expired original time or changed drafts, and saves the original state/pins before
admitting the reviewed successor skill. It performs no author call and preserves
the recorded historical thread, source, prompt, raw history, start time and all
original limits. Prepare the current page with `handoff --run-dir RUN --page
PAGE.md`, then explicitly invoke `run --run-dir RUN` for that page in a fresh
inference context. The caller chooses each subsequent page; no automatic advance
follows recovery. After all page decisions and finish-review are complete, invoke
`verify --run-dir RUN` and `promote --run-dir RUN --output-dir NEW_LOCAL_DIRECTORY`.
Record the mixed helper and context lineage rather than claiming an unchanged
treatment.

New prepared workspaces pin AGENTS.md, a project-local prepared codeclew skill
profile and the short ./docs save-pages/check/review-template entry. This keeps
the admitted workflow discoverable after compaction, without restarting generic
admission or acquisition. Changes to these managed files refuse frozen checks.
The pinned per-process skills.config disables the generic codeclew name and
enables this exact project profile; user-global configuration stays unchanged.
Save/check stdout is compact by default; complete receipts remain in the workspace
and --full returns the original receipt. Failure diagnostics retain their actual
code, source/parser details and affected page/fragment/field.

READY means the fixed finite mechanical profile and recorded self-review passed.
It does not mean semantic truth, complete runtime behavior or universal coverage.
A real source handle can accompany a false claim. Author visual review is recorded
as REVIEWED or UNAVAILABLE; independent visual/source acceptance remains separate.
Do not claim an unavailable browser stage as completed author work.

`verify --run-dir RUN` repeats the fixed checks without a model. `promote --run-dir
RUN --output-dir NEW_LOCAL_DIRECTORY` rechecks the accepted digest and gates before
atomically copying the exact accepted output. Failed or partial drafts never replace
the previous ready pointer. Keep private inputs, prompts, attempts and diagnostics
local. The human serving payload is the accepted html directory, not its receipts
or local paths. Public deployment requires the user's authorization and independent
acceptance of that exact payload.

This is an ownership contract, not a same-UID security sandbox. An embedding
publisher must protect the runner, its pinned inputs/check state and accepted
outputs from the author, and invoke the fixed promotion consumer. An agent that can
rewrite trusted code/state or deploy around the consumer can bypass this boundary;
no forged-report impossibility is claimed. Debug saves observable inputs, commands,
results and decisions only, never hidden model reasoning.


## Caller-owned successor after terminal work

To continue the same model/task and unchanged retained work with newly admitted tooling,
prepare a separate successor. This command never launches the author:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/docs_run.py" prepare-successor --job /absolute/successor-job.json --previous-run /absolute/terminal-failed-run --previous-skill /absolute/previous-pinned-skill --run-dir /absolute/new-successor
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/docs_run.py" handoff --run-dir /absolute/new-successor --page current.md
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/docs_run.py" run --run-dir /absolute/new-successor --debug
```

Only the run command launches the separately authorized author. Preparation
rejects a live/nonterminal predecessor, changed request/inventory/source content or
native bindings, changed runtime, and unverifiable historical counters. It admits
the new helper/profile/renderer through ordinary prepare, copies rather than aliases
authored work, and preserves prior attempts and the failed report. Do not edit old
pins or use accounting recovery for a different failure.

For native READY that the caller rejects on independent quality review, add
`--revise-ready` to prepare-successor. This explicit caller request validates and
preserves the original READY report, output, pins, ready directory and exact cost;
it records a separate quality-revision decision without marking the parent FAILED.
The reason is a fixed generic decision, not evaluator findings for the author.
Preparation still makes no model call. Launch requires separate caller admission.

For a genuine terminal `AUTHOR_BUDGET_EXCEEDED` without `report.json`, add
`--revise-budget-stopped` instead. This explicitly accepts the preserved result as
quality rejected and prepares a separate continuation; it does not recover or
rewrite the exhausted run. All inherited turns must be completed with exit 0 and
exact measured history. The runner verifies that use in the predecessor's own
budget scope exceeds its original limit, subtracting an existing inherited
baseline when present. It preserves the terminal status, old limit, authored
text, source bindings and all cumulative costs. The receipt records native report
absence; no READY/FAILED report is fabricated. Live, incomplete, unmeasured or
unproven stops refuse. This mode and `--revise-ready` are mutually exclusive.

For prepare-successor, the job budget is the explicit NEW_ATTEMPTS_ONLY allowance:
each explicit page handoff gets a fresh inference session with the same model.
At most the declared machine-repair rounds are allowed. The immutable history
keeps every context identity; existing per-thread counters are summed once, never
reset. Original exhausted limits and all raw counters remain in
inputs/successor.json and history; reports show inherited, continuation and total
usage. Thread-cumulative counters are not summed twice. Preparation imports no
READY state or semantic approval. The new review must compare source and text in
both directions, reconcile the full question and reader layout under the generic
updated contract, and pass the complete final gate before promotion. Unchanged
content can be retained. After explicit quality rejection, current page decisions
are required even for unchanged text; the current page cannot complete with its
own pending decisions. Ordinary valid recorded assessments remain reusable.
Available unread evidence and evaluator answers must not replace the author's
independent review.
