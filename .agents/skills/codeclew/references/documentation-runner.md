# Run retained documentation with fixed gates

This bounded runner uses the ordinary source-read, original/compose, citation,
component, renderer and export helpers. It never captures, refreshes or publishes.
The calling agent fills a job and a short launcher; it does not generate validators.
A source helper treatment is experimental until included in an official package.

For routine updates to existing documentation, use the [prior checked export
route](compose-original.md#update-an-existing-checked-export). It preserves the
original, bound index and citation manifests and applies the requested edits.
Use a fresh run for a new documentation set.

Create a job matching [docs-run-job](../schemas/docs-run-job.schema.json). Pin the
supported clew launcher, exact saved snapshot, selected services, language, current
skill and unchanged packaged renderer. Install that renderer's locked dependencies
using its documented npm command. Configure the existing local Codex CLI and exact
authorized model/effort; no new credentials or hosted account are required.
Set the job's time and repair limits, for example
`"budget": {"wallSeconds": 900, "maxRepairs": 1}`.
When the admitted clew state lives outside the author workspace, set author.stateRoot
to that existing state directory so supported native lock/receipt writes are allowed.
This grants no permission to inspect/edit private cache objects directly. The launcher is only:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/docs_run.py" prepare --job /absolute/job.json --run-dir /absolute/new-run
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/docs_run.py" run --run-dir /absolute/new-run
```

Prepare records the complete paginated native trigger/file inventory before any
author starts. A retained executable file also has a discovery obligation: source
syntax can register an imperative listener that native trigger detection misses.
Even with zero native triggers, prepare retains the service overview and executable
files for discovery. The author must report additional starts or an evidenced no-additions result;
a script cannot establish that this judgment is true. Discovered starts remain
explicit authored syntax, not compiler or runtime registration evidence. If the
required overview and entry pages exceed the compose limit, prepare reports the
count and how to split the output before reading source bodies.

Prepare also serializes the existing retained-source helpers into a source input
kit before the author starts: a small service/requirement index, directly addressed
verbatim region views and genuine ready citation handles. Raw packets, coordinates,
digests and authority remain in sidecars. The source-union helper combines only
compatible agreeing occurrences; missing coordinates or conflicting overlaps stay
separate. Every original citation reconstructs its exact original bytes. A compiler
Work uses its returned symbol/source references and retained-resource query, not an
implicit switch to a source profile. Partial coverage and unavailable resources
remain explicit. No previous authored pages or reviewer prose enter this kit.

The source-input preparation profile has a 300-second wall limit, 128 eagerly read files and
8 MiB of readable views/indexes, plus the existing 128 MiB/4096-file artifact
closure limits. Native reads run serially and timed-out helper process groups stop.
Exceeding a limit blocks preparation; it never silently truncates the source or
inventory. The author opens relevant individual views rather than replaying whole
raw packet metadata. The index lists retained FILE_ONLY resources and points to
the saved Work and metadata for an exact read when the request needs them. This
keeps resource bodies outside the eager read count and preserves their file-level
coverage. Frozen prepared manifests are included by the
runner automatically; authors must not copy them into their extra read directory.

Discovery reads must reconstruct complete native Git blob bytes using genuine
retained SOURCE occurrences and the existing agreeing source union. Syntax/input
fingerprints are not text digests. Use ordinary read-files for languages without
whole-file helper support; an existing full-file SOURCE is sufficient. Preserve
all handles and authority. A gap, unavailable occurrence or unmatched file object
blocks discovery; never pad gaps, read source privately or recapture as a shortcut.

## Prepared evidence propositions

Set `"evidenceMode": "PREPARED_FACTS"` to use the packaged deterministic
source projectors before authoring. The default is `RETAINED_SOURCE`.
Each entry or discovery file can have scoped propositions in the required
retained source views the author already reads. A short section before the code
shows facts and evidence boundaries with their conditions, authority, uncertainty
and ordinary citation handles. On a shared view the opening section samples
across scopes. If it fills up, the view lists the number of remaining items and
shows them in a named section after the verbatim source. Inline authoring places
facts and boundaries inside the matching entry and file records with short cite
aliases. Both modes show narrower witness line ranges when available. The
`prepared-facts.json` reviewer sidecar retains full witness coordinates, digests
and preparation coverage.

Preparation also looks for background entries in supported retained source.
C# discovery binds the framework `AddHostedService<T>()` call to a retained
`BackgroundService.ExecuteAsync` override. TypeScript and JavaScript discovery
binds a supported `setInterval` call or KafkaJS `eachMessage` registration to
the selected callback, including a uniquely resolved relative import. Existing
Spring scheduled and listener entries use the ordinary captured entry roots.
A discovered background entry has its own starter page, sequence template,
registration and callback source view, and review requirement.

The same views can cite across selected services when retained calls and
registrations share an HTTP route or message topic. Storage links use matching
retained table operands and can include a registered reader, publication and
subscription. Conditions and explicit unknowns accompany these source links.
A matching route carries no deployment guarantee, and matching topics or SQL
tables leave delivery, database identity, row lineage and durability open.
Cross-service source anchors identify the owning service and revision. Their
citation manifests are relative to the service index of the annotated view.

The author checks propositions against retained source and fills gaps using the
same Work. An unavailable adapter, unsupported source shape, missing operations
or unbound propositions leave the ordinary source views available. Prepared facts
do not establish runtime execution, deployment, registration, or completeness.

Preparation has a separate bounded window for the optional projectors. Java
uses an available JDK 21 or later. The TypeScript projector uses the companion
locked dependencies supplied with the installed skill. The managed projector
uses an available .NET SDK 10 or later. Install optional dependencies outside
the skill directory so the release package retains its identity. For TypeScript,
copy `scripts/prepared_facts/package.json` and its lockfile into an isolated
dependency directory and prepare with that module search path:

```sh
mkdir -p "$CODECLEW_FACT_DEPS"
cp "$CODECLEW_SKILL_DIR/scripts/prepared_facts/package.json" "$CODECLEW_FACT_DEPS/"
cp "$CODECLEW_SKILL_DIR/scripts/prepared_facts/package-lock.json" "$CODECLEW_FACT_DEPS/"
npm ci --ignore-scripts --prefix "$CODECLEW_FACT_DEPS"
NODE_PATH="$CODECLEW_FACT_DEPS/node_modules" python3 -I -S "$CODECLEW_SKILL_DIR/scripts/docs_run.py" prepare --job /absolute/job.json --run-dir /absolute/new-run
```

The projectors first use exact retained bytes. When a parser needs a full file
and Work only has fragments, preparation can use the immutable Git commit in
the locally bound checkout verified by `clew docs service show --local-binding`.
This read uses the captured revision, with no checkout or network access. A
working-tree capture uses its retained originals. C# Roslyn projection
requires the original Work compilation unit, checked against its captured
object and raw content digest. TypeScript cross-method relationships remain
open. Exact Work citations still gate every proposition. A narrower witness has to match
the bytes and coordinates of an admitted Work citation. The ordinary runner
checks and renderer still apply to every final claim.

## Author delivery

Prepare returns the author mode and the reason it was selected. A single service
with proven complete executable file bodies, at most four files and three native
HTTP entries, no deferred resources or declaration components, at most 24 cite
aliases, at most 12 KiB of source, and at most 20 KiB of inline prompt uses the
structured inline mode. Discovered source starts can be added in the response,
including for a service with zero native entries. Compiler-only or larger inputs
and cases with resources to retrieve use the workspace authoring path.

The inline author receives the request, verbatim file bytes, source-level entry
inventory, real citation aliases and the [response contract](../schemas/docs-run-inline-author.schema.json).
The user-selected Codex CLI, model, reasoning effort and reported counters remain
the same. Shell, web and extension tooling are disabled for this turn. The runner
builds the ordinary originals, navigation, sequences and self-review from the
response. The author decides claim wording, citations, outcomes, effects, unknowns,
discovery and the absence of a browser review. The same provenance, source and
rendered export checks apply.

The raw inline response is saved with its attempt. A machine rejection can be
repaired in the same session and scope. If the author explicitly gives a reason
for needing more retained evidence or a richer layout, the next permitted repair
supplies the workspace prompt. Both modes share the time and repair limits and
the same cumulative token accounting.

### Workspace authoring

The runner provides a short author prompt and starter original/navigation files.
The starter assigns required page names and anchors, links the overview to entry
pages, and supplies each declared HTTP method/path and first call in its diagram.
Keep each first HTTP call directed to the supplied handler target. Start its label
with the exact method and path. A space after the path can introduce a short
description of the same request.
The author copies these files, explains the selected behavior with real citation
handles from the source indexes, completes the remaining sequence steps, and adds
any newly discovered starts. Every required page needs a cited explanation or a
source-bounded unknown. For multiple services, also write draft/routes.json.
Place any additional citation reads under draft/reads and complete the bidirectional
[self-review protocol](../schemas/docs-run-review.schema.json). Required entry
pages and handler identities are fixed; additional discovered entry accounts cannot
replace them. The runner uses the real Markdown lexer and packaged sequence
validator, then compose/bind/finalize/export and source membership checks.
Native SOURCE membership uses bounded file metadata and record identity. If
compiler context omits the requested SOURCE for size, the runner reloads the
complete record identity from a bounded native Work part. The saved read still
proves the complete source body, owner, revision and range.
A metadata match alone cannot replace that proof. Native failures retain bounded
codes and transport details without source bodies or raw logs.
Author-written PASS reports are ignored. No gate command or skip list comes from
the job. Native declaration component tickets, when supplied, keep their own cells,
coverage and authority through the existing validators.

After writing the draft, run `review-template --run-dir RUN` and save its JSON
as `draft/self-review.json` when the command succeeds. For multiple services,
the command checks `routes.json` against the authored pages and `navigation.json`.
Give each catalog route its own narrative page. All anchors on a page share the
same canonical owner. Keep that page out of service page lists and legacy route
entries in `navigation.json`. A known local origin names a service occurrence
whose kind, page and anchor match a declared start for its service. Keep the
origin and each outcome's terminal occurrence out of that outcome's internal list.
When the command reports a conflict, use its JSON paths and destinations to edit
the draft and rerun it before completing the review.

The template command works during the active author invocation and checks frozen
inputs without native commands or receipt writes. Verification repeats the route
declaration check before binding sources. The final renderer checks anchor
existence and uniqueness in the rendered pages. Verification and promotion probe
the admitted runtime version and Markdown parser.
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

At most two repairs receive exact machine failures and retain the same author
session and unchanged source reads. Each attempt and failed draft stays in the run.
The runner enforces the wall time and repair allowance. Author token usage is
recorded without a runner token cap. The deprecated `budget.maxModelTokens`
field accepts a legacy integer or null for compatibility. Omit it in new jobs.

Terminal usage counters are cumulative within one thread. Resumed attempts count
only their deltas, and separate threads are summed once. Raw reports remain intact.
The primary usage metric is `input_tokens - cached_input_tokens + output_tokens`.
The existing `budgetTokens` field reports that measured usage and
`knownBudgetTokensLowerBound` reports an available lower bound. Reasoning is
already included in output and is never added again. Total input/output footprint
remains separate from the primary usage metric and is not a dollar-cost estimate.
Missing or ambiguous thread identity, changed resume identity and invalid or
regressing counters refuse completion. Missing counters stay unavailable and
prevent readiness and another author invocation.

Frozen runs retain the exact runner, package and input pins from preparation.
Changing the package requires a fresh preparation for a new run. A historical
author stopped by the former token cap is still a stopped attempt and never
automatically resumes.

READY means the fixed finite mechanical profile and recorded self-review passed.
It does not mean semantic truth, complete runtime behavior or universal coverage.
For the initial direct statements of a self-contained retained Kotlin entry
function, prepare can check a unique single-line guard of the form
`if (identifier comparison INTEGER) return INTEGER` (also with inline braces).
The prepared source index lists each such local literal return check. The author names a
supported fragment in the review template's optional `returnClaims` map. It
must cite the listed entry source or a verified occurrence from the same
captured file that contains the declaration byte for byte. Write the guard and
result as successive inline code spans in the same top-level Markdown paragraph. The runner checks the actual
renderable claim against the retained integer and keeps the authored text intact.
This check covers the statement when reached. Branch reachability, arbitrary
prose, other expressions/languages and older unbound exports need independent
meaning review. The candidate scan skips statement text with block comments,
multiline or interpolated strings, and ambiguous matching predicates. A bare
`EXPORT_CHECKED` reports technical export only. Author visual review is recorded
as REVIEWED or UNAVAILABLE. Independent visual/source acceptance remains separate.
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
