# Codeclew — managed semantic context and changes

Codeclew builds bounded compiler-backed context, validates an edit plan in an
isolated candidate worktree, and publishes the resulting commit explicitly.
Use the installed `clew` launcher for public releases or `./clew` from a pinned
checkout for source development; direct capsule binaries are unsupported.

## Install on macOS

The public pilot ships prebuilt bundles for Apple Silicon and Intel Macs:

```bash
curl -fsSL https://codeclew.github.io/codeclew/install.sh | sh
```

The installer resolves `latest` to one immutable version, downloads that exact
GitHub Release asset and checksum, verifies SHA-256, installs it below
`~/.local/share/codeclew`, and atomically links `clew` into `~/.local/bin`. It
never compiles Codeclew on the target machine. Run `clew doctor --human` after
installation and `clew upgrade` to install a newer release. The source launcher
`./clew` remains the supported development entrypoint and is updated through Git,
not `clew upgrade`. Existing
installations older than v0.1.3 need the one-line installer once more to acquire
the updater; their later updates use `clew upgrade`.

The default `core` profile contains Kotlin 2.4.10. Kotlin 2.3.0 remains an
optional read-only preview and is downloaded only when requested:

```bash
clew pack install kotlin23
clew pack list
clew pack remove kotlin23
```

## Source-build requirements

- macOS or Linux
- Python 3.11+
- Git
- JDK 21
- the Rust toolchain pinned by `rust-toolchain.toml`
- Cargo on `PATH` for Rust repositories
- Maven on `PATH` only for Maven projects without `./mvnw`

The exact Kotlin 2.4.10 worker is packaged into the default immutable runtime
capsule; the optional `kotlin23` profile adds the exact Kotlin 2.3.0 worker. A
cold source runtime start may build the capsule. A warm runtime invocation
verifies and reuses it without running Cargo, Rustc, Gradle, or Maven. The
workers execute directly from that sealed capsule under its shared lease; the
runtime warm path does not copy their distributions. Project analysis may still
invoke the project wrapper under the model-cache policy described below.

The strict compiler-backed mutation contour is Kotlin 2.4.10, Gradle,
`PROJECT_NATIVE`, and one exact compilation. Exact Kotlin 2.3.0 with Maven is a
read-only compiler-backed context preview: it has real-project context
acceptance, but no mutation or publish claim. Kotlin 2.1, multiple
compilations, Android/KMP and `EXTERNAL` remain unqualified until they have
their own acceptance tests. Rust and Python also support conditional mutation,
with the weaker evidence boundaries described below.

Rust has a bounded syntax contour. Open it with `--language rust`
and an exact target selector such as
`cargo:crates/clew/Cargo.toml#clew#lib#clew`. The repository must have a regular
root `Cargo.lock`; live `cargo metadata --no-deps` authority and all paths are
normalized before entering CAS or stdout. The preview follows only unambiguous
snapshot-backed `mod name;` files from the selected target and exposes bounded
declaration occurrences with exact syntax ranges. It deliberately emits no
resolved references or call edges: cfg, derive/procedural macros, custom module
paths, parse failures, ambiguity, and resource caps remain explicit boundaries,
so the generation is `PARTIAL/UNSURE`. A Rust change plan must use only Cargo
validation. Codeclew may prepare and publish the isolated candidate only as
`READY_TO_PUBLISH_CONDITIONAL` / `PUBLISHED_CONDITIONAL`, after the caller
reviews the diff, sees successful native validation, and explicitly
acknowledges every cfg/macro and name-resolution obligation. This never upgrades
syntax evidence to compiler-backed certainty.

Python has a generic bounded syntax contour. It parses only selected
tracked UTF-8 `.py` blobs directly from the immutable session base commit with
the pinned in-process Tree-sitter grammar. It creates no source checkout and
does not start Python, import project modules, activate a virtualenv, install
dependencies, run Git hooks/filters/fsmonitor, or read unrelated blobs such as
`.env`. The target index and dirty, deleted or untracked worktree state are
outside the session authority. Select an explicit import root and source root,
for example:

```bash
./clew session open \
  --repo /path/to/python-repository \
  --target-ref refs/heads/main \
  --language python \
  --compilation 'python:.#src'
```

The source root must equal the import root or be its descendant. Analysis
returns bounded source, declaration, import, decorator-name and syntactic
call-name facts with exact ranges. It never claims runtime import, type, call,
decorator or framework resolution, so its authority is always
`PARTIAL/UNSURE`. A Python change plan must validate with
`python3 -m <safe.module> ...`; inline code and shell launchers are rejected.
Codeclew may publish only conditionally after the native module succeeds and
the caller acknowledges every runtime-import/type and dynamic-execution
obligation. The Python environment remains project-owned: activate or place its
virtual environment on `PATH` before `change prepare`.

This revision is pilot-ready, not general availability. Team use follows the
[Kotlin 2.4 pilot runbook](docs/pilot/README.md); a signed prebuilt release is a
separate decision after 20 recorded in-contour cases meet its numerical gate.

## Operational admission

Before opening a session, inspect the runtime-bound support contract and host
readiness. For a terminal-friendly report, run:

```bash
./clew capabilities --human
./clew doctor --repo /absolute/path/to/repository --target-ref refs/heads/feature --human
```

The support matrix is also retained as
[`crates/clew/support-matrix.json`](crates/clew/support-matrix.json). Automation
uses the canonical JSON emitted when `--human` is omitted and must inspect the
doctor's status and required checks. The command can return a valid
`ACTION_REQUIRED` report without treating that report as a CLI failure.

Before prepare and publish, verify that the session still owns the checked-out
target authority:

```bash
./clew change check-freshness --session session:...
```

`DIRTY` preserves developer work and requires a human decision. `STALE`
requires a new session/context/plan; Codeclew never rebases or replays the old
plan automatically. For incident sharing, keep raw JSON local in a mode-0600
file and export only the allowlist summary:

```bash
./clew support summarize --input /absolute/private/path/result.json
```

Installation, Codex/Claude skills, frequent-update recovery, privacy rules,
language extension, and multi-repository operations are documented in the
[P0 operations runbook](docs/operations/p0-runbook.md).

## Workflow

```bash
./clew change open \
  --repo /path/to/clean-kotlin-repository \
  --target-ref main \
  --language kotlin \
  --compilation :app/main \
  --intent 'describe the requested change' \
  --term ImportantSymbol \
  --term ImportantBehavior

./clew change prepare \
  --session session:... \
  --context context:sha256:... \
  --plan edit-plan.json

./clew change status --run run:...
./clew change publish --session session:... --run run:...
./clew session close --session session:...
./clew session gc --session session:...
```

`change open` atomically returns the session and its bounded initial context.
`change prepare` validates the immutable plan and idempotently starts its
isolated candidate run. The low-level `session`, `context`, `plan`, and
`task-run` commands remain an advanced protocol for expansion, cancellation,
relocation, and diagnostics; they are not required for the happy path.

### Read-only multi-service threads

An immutable thread can bind two to eight already-open local sessions and
return one bounded, path-free context across repositories and languages. Every
fact and source row retains its member, service, session, language, compilation,
context, and evidence authority. This is analysis composition only: thread
contexts are explicitly rejected by plan validation, task runs, and
publication.

All member sessions must have been opened through the currently active runtime
capsule. A thread neither owns nor changes them, so the same repository may
contribute separate Kotlin, Python, or Rust analysis units and a session may be
used by more than one thread.

```bash
./clew thread open \
  --member provider=session:... \
  --member consumer=session:... \
  --service-alias provider=orders \
  --service-alias consumer=checkout

./clew thread context \
  --thread thread:... \
  --intent 'trace normalization across services' \
  --term normalize \
  --term Service

./clew thread callables \
  --thread thread:... \
  --context thread-context:sha256:... \
  --task-id inspect-normalization \
  --pair-id orders-checkout \
  --provider provider \
  --consumer consumer \
  --term normalize \
  --term Service

./clew thread flow \
  --thread thread:... \
  --fact-set thread-callables:sha256:... \
  --pair-id orders-checkout \
  --member provider \
  --root-kind full-symbol \
  --root 'callable:sample/Service.normalize#jvm:(Ljava/lang/String;)Ljava/lang/String;' \
  --direction downstream \
  --max-depth 4

./clew thread explain \
  --thread thread:... \
  --flow thread-flow:sha256:... \
  --claims claims.json

./clew thread render \
  --thread thread:... \
  --explanation thread-explanation:sha256:... \
  --detail summary \
  --format markdown

./clew thread explanation-status \
  --thread thread:... \
  --explanation thread-explanation:sha256:... \
  --against-thread thread:... \
  --against-fact-set thread-callables:sha256:... \
  --against-flow thread-flow:sha256:... \
  --member-correspondence provider=provider \
  --member-correspondence consumer=consumer

./clew thread impact \
  --thread thread:... \
  --fact-set thread-callables:sha256:... \
  --pair-id orders-checkout \
  --subject-kind callable-family \
  --subject sample/Service.normalize

./clew thread validate \
  --before-thread thread:... \
  --before-impact thread-impact:sha256:... \
  --after-thread thread:... \
  --after-impact thread-impact:sha256:... \
  --member-correspondence provider=provider \
  --member-correspondence consumer=consumer \
  --coverage change-coverage.json

./clew thread close --thread thread:...
./clew thread gc --thread thread:...
```

The composite uses one global 4,096-fact, 32-window/256-KiB source, 1-MiB
evidence, and 64-KiB stdout budget, with deterministic round-robin selection
across member and compilation lanes. It never upgrades a member's certainty or
drops its verification obligations. Member contexts remain independently
addressed if another member fails, but no composite is published. Thread `gc`
is a terminal metadata transition; it does not delete member sessions or claim
physical CAS reclamation, and retained thread-context root records remain
readable.

`thread callables` is the Kotlin Descriptor Navigation v1 projection. It reads
already-sealed K2 generations and writes an immutable, thread-owned
declaration/use/boundary fact set plus a dedicated query index; it never starts
a compiler or build tool. A descriptor is exact only when its own compiler row
is complete and no named boundary matches that member and callable. An
unidentified boundary keeps the aggregate result `PARTIAL/UNSURE` and remains
an explicit verification obligation, but it does not erase an independently
proved descriptor shape. Cross-repository relationships remain
`DECLARED_TOPOLOGY`: exact shape evidence is not service ownership, routing, or
compatibility evidence.

Experimental `thread flow` consumes only that retained fact-set closure and
starts no compiler, build, repository discovery, or target process. It accepts
one exact `FULL_SYMBOL` root in one selected-pair member and deterministically
walks downstream `UseFact` relations with frozen depth, node, edge, boundary,
stdout, and retained-closure budgets. Nodes and edges are qualified by member,
service, and repository namespace; their support retains session, compilation,
generation, fact-shard, payload, and immutable repository-relative source
authority. This prevents identical CallableIds in two repositories from
colliding. Missing, ambiguous, unresolved, out-of-pair, and truncated
transitions remain visible boundaries with verification obligations. Cycles
are represented once through stable nodes and edges rather than expanded
paths.

Pair flow is not whole-thread navigation. Traversal is confined to the exact
provider/consumer pair named by `--pair-id`; a relation into a third member is
reported as `TARGET_OUTSIDE_SELECTED_PAIR`. In v1, a cross-repository relation
may become only a `DECLARED_TOPOLOGY` handoff from the declared consumer to a
boundary node for the provider. It includes
`VERIFY_RUNTIME_COMPONENT_HANDOFF` and is never described as an observed HTTP,
Kafka, database, or runtime call. `UNBOUND` creates no handoff edge. A supplied
`VERIFIED_SAME_SNAPSHOT_COMPILATION_DEPENDENCY` cross-repository relation is
also rejected as `UNSUPPORTED_EXACT_PAIR_DEPENDENCY`: support for proving that
relationship is deliberately outside this version. Member lanes are selected
fairly so one large repository cannot silently displace the other from a
bounded projection. Because v1 relation facts do not carry a pair ID, a
consumer participating in multiple declared pairs produces
`AMBIGUOUS_DECLARED_TOPOLOGY_HANDOFF` instead of guessing a provider.

Absent compiler CFG evidence, the graph is an `UNORDERED_STATIC_RELATION`:
source offsets are evidence locations, not proof that independent calls happen
before or after one another.

Control-flow order is claimed only when the selected sealed generation also
contains a canonical `local-cfg/0.1` payload for the exact full-symbol owner and
the relation's `cfgNodeIds` all exist in that graph. The normalized contract
uses stable roles (`DECISION`, `RETURN`, `THROW`, `LOOP_CONDITION`, and others)
and transition kinds (`TRUE`, `FALSE`, `WHEN_CASE`, `EXCEPTION`, `LOOP_BACK`,
and others), not Kotlin-version-specific FIR class names. Loops remain graph
back-edges and exceptional/return edges do not continue along a fabricated
happy path. If the graph is absent, partial, ambiguous, or not linked to the
relation, the result is `orderAuthority=UNKNOWN` with
`VERIFY_CONTROL_FLOW_ORDER`; numeric CFG IDs and source offsets alone are never
treated as before/after evidence. The currently bundled worker generations do
not yet emit this standalone normalized payload, so existing retained
generations intentionally take that fail-closed path.

`thread explain` accepts a canonical, closed
`codeclew-explanation-claim-input/0.1` document authored by an agent. The agent
may provide only a local ID, `en`/`ru` narrative text, a typed predicate, flow
support refs, and relevant boundary refs; input fields such as `authority`,
`status`, or a digest are rejected. Core validates `CALL_EXISTS`, `CONSTRUCTS`,
`BRANCH_EXISTS`, `ORDERED_BEFORE`, `REACHABLE_STATIC_PATH`, and
`NARRATIVE_SUMMARY` against the immutable FlowSlice. `COMPONENT_HANDOFF` is
accepted only for a member-qualified cross-repository edge whose target is the
declared topology boundary; its authority is exactly `DECLARED`, never
compiler- or runtime-proven. Core computes the authority cap
(`COMPILER_PROVEN`, `STATIC_DERIVED`, `DECLARED`, `AGENT_INFERRED`, or
`UNKNOWN`), and publishes one content-addressed bundle. A missing mandatory
support, invented ref, contradictory endpoint, omitted relevant boundary,
truncated path, or attempted authority promotion fails closed. Narrative text
is never semantic authority: even supported narrative is capped at
`AGENT_INFERRED`, while boundary-only narrative is `UNKNOWN`.

`thread render` selects one of five deterministic views from that same bundle:
`SUMMARY` keeps outcomes and unknown claims, `SCENARIO` adds branches/order/path
claims and CFG regions, `TECHNICAL` adds symbols and relation edges, `EVIDENCE`
adds claim support/boundary closure, and `COMPILER` adds the bounded fact,
payload, shard, provenance, and source-anchor references already retained by
the flow. Rendering never rereads a mutable repository and never invokes an
agent, compiler, build tool, or target process. JSON is the structured
projection; Markdown is returned in the `content` field of a bounded result and
uses stable anchors for claim → node/edge/region → fact expansion. Both formats
carry the same explanation ID, flow ID, semantic digest, truncation flag,
boundaries, authorities, and obligations. Optional sections are filled in a
fair round-robin across pair members until 64 KiB; Markdown groups technical
nodes by service/member and labels declared cross-service edges explicitly.
Critical boundaries and obligations are never silently truncated, and the
render fails if they alone exceed the budget.

An explanation bundle is immutable and remains byte-for-byte reproducible for
its original thread snapshot. `thread explanation-status` compares it with an
explicit new thread/fact-set/flow binding chain; it never rewrites the bundle
or Markdown. Exact member correspondence must preserve repository namespace
and Kotlin profile authority. Root and evidence are matched by full symbol,
declaration shape, relation identity, and normalized CFG structure, while file
and line offsets are ignored as semantic change signals. The retained report
is `CURRENT`, `PARTIALLY_STALE`, `STALE`, or `UNRESOLVED` and lists only
affected claim IDs, their old refs, observed new candidates, reasons, and a
regeneration obligation. Regeneration is a separate agent action. Both old and
new CAS closures are retained in the report; repeated status reads start no
compiler, build, repository scan, target process, or agent.

### Save-product explanation walkthrough

The executable acceptance fixture under `fixtures/kotlin-explanation` contains
a product service and a separate outbox worker. The product method performs a
duplicate check, calls `ProductRepository.save`, calls `OutboxRepository.save`,
and carries a local annotation named `Transactional`. That annotation is
deliberately not framework authority: the walkthrough proves both call edges
from K2 facts, but reports transaction atomicity as `UNKNOWN` with
`VERIFY_CONTROL_FLOW_ORDER` rather than inferring Spring or database behavior.

Run the complete scenario with:

```bash
python3 scripts/test_explanation_smoke.py
python3 scripts/explanation-smoke.py
```

The second command creates private temporary Git repositories and Codeclew
state, validates both Gradle projects, and performs this chain:

```text
Kotlin sessions -> multi-service thread -> bounded context
  -> callable fact set -> exact saveProduct flow
  -> agent claim template bound to exact flow refs
  -> immutable explanation bundle -> five semantic-zoom renders
  -> compiler fact/CAS/source-anchor drilldown
  -> offset-only freshness -> controlled relation-change freshness
```

`thread flow` returns the complete retained slice when it fits 64 KiB. For a
larger slice it returns a root-centric `claimBinding` projection containing
exact direct node/edge IDs, support fact IDs, and relevant boundary IDs; the
full slice remains in CAS and is what `thread explain` validates. The smoke
does not trust selector text after binding: ambiguous or absent roots, edges,
boundaries, source anchors, or authorities fail the run.

The expected authority split is:

- product repository save: `COMPILER_PROVEN`;
- outbox repository save: `COMPILER_PROVEN`;
- high-level narrative: at most `AGENT_INFERRED`;
- transaction atomicity: `UNKNOWN` until a separate framework/runtime
  authority proves it.

The smoke then uses managed `change open/prepare/publish` operations rather
than editing Kotlin directly. Inserting a blank line must produce `CURRENT`;
removing the outbox save must produce `PARTIALLY_STALE` with both affected and
unaffected claims. Its final JSON includes cold generation, warm retained-read,
render, and freshness timings. These are diagnostic measurements, not fixed
performance thresholds; identity reuse and the absence of extra generations
on retained operations are the acceptance conditions.

`thread impact` consumes that immutable fact set without rebuilding it. The
single command accepts an exact full symbol (with `--member`), a raw CallableId
family, or a navigation token. Its bounded output includes both declared pair
members, projected declaration shapes, relevant uses and boundaries, every
verification obligation, and exact repository-relative source anchors. Source
text is not copied into the impact authority; each anchor is bound to a source
CAS digest and byte range so an agent can verify the local code without
confusing a snippet with semantic authority. Findings may truncate to a fair
`UNSURE` prefix, while obligations never truncate. No compiler, build tool,
repository discovery, or target process runs on this path.

`thread validate` compares two retained `CALLABLE_FAMILY` impacts for the same
repositories and Kotlin profile. It reports compiler-projected `KCD_*`
changes, both selected pair members, and unresolved before/after/comparison
obligations; unrelated members of either containing thread are not claimed as
covered. These are observations to verify, never compatibility or breakage
verdicts.
An empty coverage document returns `INCOMPLETE` with every required stable
target ID. A closed document may acknowledge each target with only an inert
`ACTION` or `EXTERNAL_WORK` tracking ID:

```json
{"entries":[{"handling":{"id":"verify-relationship","kind":"EXTERNAL_WORK"},"requiredCategories":["RELATIONSHIP_AUTHORITY"],"targetId":"sha256:..."}],"schema":"codeclew-kotlin-change-coverage-document/1.0"}
```

Complete acknowledgement returns `VALIDATED_CONDITIONAL`; current
`DECLARED_TOPOLOGY` authority can never produce an unconditional green status.
Unknown, duplicate, stale, category-mismatched, executable, path-shaped, or
shell-shaped entries fail before publication. The result binds both threads,
fact sets, impacts, runtime manifest, rules, correspondence, document, and the
full two-sided CAS closure. Repeating it reads retained state only and starts
no Git, compiler, build tool, or target process.

Terms may be exact identities or natural identifier components. The query
index retains the full identifier and language-neutral camel-case/snake-case
aliases, so `Maven` can discover `MavenProjectModel` without changing the exact
request authority. Component discovery remains `UNSURE` until exact semantic
evidence is selected and its reported obligations are checked. Prefer a few
distinctive terms over broad words such as `model` or `test`, which may produce
an intentionally truncated conditional context.

Each normalized term retains at most 1024 deterministic fact references in the
query index. A term with greater fan-out is recorded as overflow authority and
every result that uses it reports `truncated=true`; refine that request with a
class, callable, or file identity before treating the context as complete. This
keeps generic vocabulary from turning the bounded routing index into a second
copy of the repository fact store.

`--compilation` is mandatory and names an exact analysis authority: use
`:/main` for a Kotlin root project, `:module/main` for a Gradle subproject, or
`python:<import-root>#<source-root>` for Python syntax analysis. Repeat the
option to select up to 64 compilations. Session authority
sorts and deduplicates the set and accepts only `:/sourceSet` or
`:module[:nested]/sourceSet`; option-like and path-traversal values are rejected
before any build tool starts. Codeclew never guesses a root compilation because
that would defer a deterministic configuration error from session admission to
context creation.

Multi-compilation generation captures the repository snapshot once and admits
independent compiler lanes from the detected CPU and memory budget. By default
the lane count is host-adaptive and capped at 16. Reproducible measurements may
pin it with `--generation-jobs N`; the selected job count is session authority
and cannot exceed current admission.

`change prepare` writes a durable `CREATED` record before detaching. Repeating
the same request attaches to the same content-addressed run. Preparation may
compile, test, and build a staged repository index, but it never changes the
session's target ref. Only `change publish` may fast-forward the ref.

Interrupted pre-commit work can be continued with the advanced `task-run
resume`. A committed candidate is never reset automatically: use `change
recover` with its session and run IDs to reconcile it. `session close` requires no live or unresolved run;
`session abort` is the explicit cancellation terminal. If the repository moved,
use `session relocate --session ... --repo ...`. GC deletes only worktrees proven
to be Codeclew-owned and refuses every non-empty candidate without an exact
receipt, including with `--force`.

Context stdout is bounded to 64 KiB. It contains the edit-ready projection and
content IDs; full evidence remains in private managed state. Plans are bounded
to 1 MiB, 256 operations, 256 files, and a 256 KiB expected write set.

Opaque per-file `semanticFacts` arrays are normalized before hashing and
replaced in public/query file facts by a fact count and digest. Other normalized
file metadata and declarations remain present, while granular compiler
descriptors and relations remain queryable. This prevents private operational
paths and oversized opaque payloads from dominating context selection; recall
inside the omitted array is deliberately not claimed.

## State and build authority

All mutable Codeclew state lives under private `CODECLEW_HOME` (by default the
user cache directory):

```text
runtimes/
repos/
sessions/
threads/
runs/
locks/
tmp/
quarantine/
```

Codeclew does not discover, read, import, update, or delete `.semantic-thread`.
Old receipts, indexes, and runs are inert. Absolute repository paths exist only
in private `0600` locator files and are never emitted in stdout or evidence.

`PROJECT_NATIVE` uses the project's wrapper and ordinary user build
environment. Model caching is `NON_CACHEABLE` unless the session explicitly
selects a tracked `codeclew.model-cache.json` or sealed external authority.
The sealed external contour remains fail-closed. Within an open session,
context creation reuses its immutable generation. For every new `NON_CACHEABLE`
session, Codeclew extracts and compares the live project model; it reuses a
previous generation only when the repository snapshot, derived model manifest,
and compiler-store authority are all exact matches. The worker still opens the
project, but skips `IndexFiles`. Any mismatch returns to the fail-closed
full/delta planner.

A tracked cache manifest must exactly match `HEAD`, be canonical JSON plus one
newline, and explicitly list the authorized compilations:

```json
{"compilations":[":/main"],"schema":"codeclew-model-cache-policy/2.0"}
```

Select it with `session open --model-cache tracked-manifest`. Sealed external
state requires a RELEASE capsule, a private absolute directory with its signed
manifest/seed, and both `--model-cache sealed-external` and
`--external-build-state /private/path`.

## Conditional evidence

When evidence is useful but not deterministic, a conditional decision may carry
explicit publication obligations. Such a run may compile, test, and index a
candidate. If both context and candidate remain within the supported contour,
it reaches `READY_TO_PUBLISH_CONDITIONAL`; incomplete or unsupported evidence
still terminates as `VALIDATED_CONDITIONAL` and cannot be published.

Conditional publication is fail-closed by default. The caller must pass
`--allow-conditional` and acknowledge every qualified `approvalId` reported by
`change status`. The durable approval binds the session, run, context evidence,
plan, candidate commit and snapshot, exact changed files, bounded diff and
successful validation evidence. The result is `PUBLISHED_CONDITIONAL` and its
certainty remains `UNSURE`; acknowledgement never upgrades evidence to
`VERIFIED`.

```bash
./clew change status --run run:...
./clew change publish \
  --session session:... \
  --run run:... \
  --allow-conditional \
  --prepared-authority-digest sha256:... \
  --acknowledge-obligation context:sha256:...
```

## Verification

Ordinary development and GitHub CI use the same repository-owned entrypoint:

```bash
./scripts/ci-verify.sh
```

The stabilization controller and its receipts remain optional research/release
evidence. They do not gate product development or CI. Cold/multi-compilation
performance, BTA24, self-hosting and agent comparisons are qualification work,
not prerequisites for the supported Kotlin 2.4 publish path. Kotlin 2.3.0
qualification covers exact worker admission, capsule identity, privacy-safe
fact translation, incremental receipt construction, and read-only context on a
representative Maven repository; it does not extend the publish contour.

The CLI writes canonical JSON to stdout and diagnostics to stderr. The system is
fail-closed for stale authorities, ambiguous anchors, unsupported project
models, dirty checked-out publication targets, and recovery uncertainty.
Each cold generation also retains a private `dag-report.json` with total work,
critical path, observed parallelism, and the number of sealed compiler streams.

## Repository map

- `crates/clew`: Rust core, supervisor, sessions, indexes, and CLI
- `workers/kotlin*`: version-pinned Kotlin compiler workers
- `bootstrap`: isolated content-addressed runtime bootstrap
- `schemas`: typed worker protocol
- `fixtures`: executable Kotlin corpus
- `scripts`: CI, demo, and benchmark entrypoints
- `docs`: architecture and experiment history

Licensed under the [Apache License 2.0](LICENSE).
