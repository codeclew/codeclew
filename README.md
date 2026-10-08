# Codeclew — managed semantic context and changes

Codeclew prepares bounded source and compiler context for agents to explain
code, maintain source-linked service documentation, trace interactions across
repositories, compare saved edits, and prepare managed changes for explicit
publication.
Use the installed `clew` launcher for public releases or `./clew` from a pinned
checkout for source development; direct capsule binaries are unsupported.

## Choose a task

| If you want to… | Start here |
| --- | --- |
| Install Codeclew and connect an agent | [Install](#install-on-macos-linux-or-windows-through-wsl2) and [install the agent skill](#install-the-agent-skill) |
| Navigate code and check repository support | [Practical code navigation](#practical-code-navigation) and [operational admission](#operational-admission) |
| Create or refresh service documentation | [Durable service documentation](#durable-service-documentation) and the [service workflow](skills/codeclew/references/service-documentation.md) |
| Export linked pages from selected retained source | [Native source pages](docs/guides/native-static-pages.md) |
| Ask and review a question about retained Java source | [Draft review and explicit publication](docs/operations/docs-snapshot-store.md#review-a-saved-operation-draft) and [compare a saved answer with a new Check](docs/operations/docs-snapshot-store.md#compare-a-saved-answer-with-an-explicit-captured-check) |
| Find a previously approved method answer after a source update | [Capture, question, review and exact reuse](docs/operations/field-documentation.md#capture-question-review-and-exact-reuse) and [discovery limits](docs/operations/docs-snapshot-store.md#find-an-approved-answer-by-exact-request-source-candidate) |
| Try the internal Java process draft | [historical v0.13.1 example and limits](#historical-v0131-example-and-limits), [reproduction recipe](docs/operations/task-promotion-draft.md), and [live sample reader](https://codeclew.github.io/codeclew/examples/task-promotion/index.html) |
| Compare saved local edits | [Analysis with local edits](#analysis-with-local-edits) |
| Save diagnostics for a failed Check or command | [Support bundles](docs/operations/support-diagnostics.md) |
| Prepare a managed change | [Workflow](#workflow) and [conditional evidence](#conditional-evidence) |

<a id="current-release-v01315"></a>

## Current release: v0.13.20

Version 0.13.20 saves a diagnostic bundle from an existing Check without another
source capture: `clew support collect --root DOCS --output NEWDIR`. Use
`clew --diagnostics NEWDIR COMMAND ...` to retain diagnostics from the next
invocation, including failures before the core starts. Original output and exit
status are preserved; private bounded terminal, compiler-worker and Maven logs
require `--include-private-logs`. See the [release notes](docs/releases/v0.13.20.md)
and [diagnostic guide](docs/operations/support-diagnostics.md) for scope and limits.

Core now includes the sole Kotlin 2.4.10 analyzer for supported Kotlin 1.9–2.4
projects, including 2.3.0. Separate Kotlin 2.3 packages are retired; see the
[installation guide](#install-on-macos-linux-or-windows-through-wsl2) for migration
from an older optional package. C# Roslyn remains included in the public archives.

### Included from v0.13.18

Version 0.13.18 renders linked offline HTML and MDX from selected retained Java,
Kotlin, C#, TypeScript and Rust declarations. New Kotlin compiler evidence can
explain guarded data transformations and source-declared shared queue wiring
through the common Java/Kotlin transfer engine. Endpoint exclusion and
reinclusion preserve saved analysis and retained explanations. See the
[v0.13.18 release notes](docs/releases/v0.13.18.md) and
[native retained-source guide](docs/guides/native-retained-source.md).

A single installed core passed a bounded public Pipeline fixture on native
Kotlin 1.9.25, 2.1.21, 2.3.0 and 2.4.10 projects. Project compiler settings and
analyzer identity remain separate; partial coverage and the explicit Kotlin 1.9
analysis upgrade boundary stay visible. This does not establish native compiler
semantic equivalence or broader mutation support.

<a id="accepted-source-candidates-for-v01316"></a>

### Included from v0.13.17

Kotlin/Gradle metadata capture now works with configuration cache enabled.
Known build failure categories keep safe, specific recovery guidance.


`docs work find-answer` searches historical approved method answers by the exact
normalized request and subject, then checks applicability against an explicitly
saved Check. Multiple eligible approvals require explicit selection. Historical
approval stays frozen; `CURRENT` is a separate evidence comparison. Follow the
[complete retained-evidence route](docs/operations/field-documentation.md#capture-question-review-and-exact-reuse)
for preparation, independent review, reuse and updates.

Syntax extraction can reuse successful results for the same service/path,
exact bytes, dialect and complete producer while rebuilding current source
receipts. One bounded worker corpus measured 21.863% incremental native
capture/Check savings; initial cache seeding has a cost. This is not a general
CLI or compiler speedup. See the [capture-cost boundary](docs/operations/field-documentation.md#incremental-capture-cost-source-candidate).

For compatible new drivers, the [opt-in model serializer](docs/operations/model-id-serializer.md)
recommends `codeclew-model-ids/1.1`; frozen 1.0 carriers remain supported unchanged.
Version 1.1 checks the native presentation against canonical pages/source parts
and omits only their duplicate raw siblings from model input. Receipts, citations,
presentation and canonical archives stay intact. Across a generated grouped-read
fixture's four initial/expanded role contexts, paired 1.0/1.1 model forms for the
same canonical jobs and scopes measured **163,106 to 104,051 reference text
tokens: 36.2065% fewer**, including protocol/schema overhead. The expanded pair
alone fell 38.8299%; the initial author grew by 18 tokens (0.2547%). This is not a
general, quality or billing saving, or repeated dispatch of the same invocation.
See the [bounded compact measurement](docs/plans/question-results-and-incremental-evidence.md#compact-role-evidence-representation-candidate-2026-10-06).
The earlier 4.9184% result remains the separate 1.0 measurement of two retained
first calls against canonical input. Maps and canonical jobs stay outside model
input in both versions.

### Included from v0.13.15

Retained `docs context --dependency` reads the selected evidence closure before
materializing unrelated observation payloads. The response explicitly identifies
selected validation and preserves source, citations, boundaries and cursor scope.
Other selectors and source objects keep their existing full reads.
Service documentation retains the neutral **SERVICE** label and separate source,
review and runtime status. See the [v0.13.15 release notes](docs/releases/v0.13.15.md).

C# reading and analysis through Roslyn remains included, with the read-only
profile and .NET 10+ host SDK requirement introduced in 0.13.11.
The [public task guides](https://codeclew.github.io/codeclew/) include pinned,
executed 0.13.11 examples and their explicit evidence limits.

### Included from v0.13.11

Create a first source-linked overview with the executable starter included by
`clew docs init`. It registers an explicit committed source scope, captures it,
resolves native Work references and assembles complete source reads for your
current agent. Saved-snapshot questions need no recapture or model driver.
See the [first-document recipe](docs/operations/field-documentation.md#executable-first-overview-and-source-questions).

New operation Work can opt into authoring contract 1.6 and execution schema 1.1.
An author or independent reviewer can request a group of native references,
exact symbols or queries before its next decision. Complete pages and source
parts come from the retained snapshot. Reviewer reads form a separate evidence
context; they do not rewrite the saved author answer. Both roles receive their
remaining caller-selected finite call budget. Existing defaults are unchanged.
See the [configuration and recovery guide](docs/operations/docs-snapshot-store.md#opt-in-section-author-contract).

Replay resumes interrupted evidence delivery and settles unfinished terminal
accounting under the original configuration without repeating a model call.
An explicit uncertain-review retry preserves the failed invocation and uses a
new empty account. Untouched generated Help and Runbooks navigation updates on
the next publication while preserving edited guides and historical bundles.

Manual publication remains `UNASSESSED`; independent model review requires
explicit drivers and finite configuration. The complete initial semantic packet
is preserved; this feature alone does not establish a token saving. See the
[v0.13.11 release notes](docs/releases/v0.13.11.md).

### Historical v0.13.1 example and limits

The v0.13.1 release introduced an experimental draft path that makes one
configured author call for one compiler-backed internal Java method. The public
example explains `TaskStateTransitions.promote(Task, Instant)` in the `:/main`
Maven compilation.
The configured stdio author path requires macOS Seatbelt isolation; Linux keeps
source capture and the external structured-answer renderer, but does not qualify
author execution for this example. See the [v0.13.1 release notes](docs/releases/v0.13.1.md)
and [reproduction recipe](docs/operations/task-promotion-draft.md).

The draft writes local `answer.json`, `operation.md`, and `index.html` views.
Its authored decision tree can render as pseudocode, while a separate source-local
process-flow SVG is projected from retained source syntax. Neither view is a
runtime trace. The Work remains `DRAFT` / `UNREVIEWED`; this path creates
no proposal and publishes nothing. The checked-in
[sample reader](fixtures/documentation-internal-process/documentation/sample/index.html)
is also available as a [live page](https://codeclew.github.io/codeclew/examples/task-promotion/index.html).
It received a positive source-fitness review and browser acceptance for this one
method and rendered sample. That does not qualify the full workflow or establish
runtime dispatch, persistence, or deployed behavior. GitHub's file view does not
render the checked-in HTML; its exact scope is recorded in [sample provenance](fixtures/documentation-internal-process/documentation/sample/provenance.json).

## Install on macOS, Linux, or Windows through WSL2

The current public release ships prebuilt bundles for Apple Silicon and Intel Macs, and
Linux x86_64 (including Windows x64 through WSL2):

```bash
curl -fsSL https://codeclew.github.io/codeclew/install.sh | CODECLEW_VERSION=v0.13.20 sh
```

This pins the version documented above. Omit `CODECLEW_VERSION` to install the
latest published release.

On Windows, run this command in your WSL2 Linux shell. Native Windows shells
(PowerShell, Git Bash, and MSYS2) are not supported. Linux requires glibc 2.35+
and Python 3.11+; Ubuntu 24.04 is a suitable WSL2 distribution. Git and curl must
also be installed inside WSL, with JDK 21 for Kotlin project analysis. Keep the
installation, `CODECLEW_HOME`, and repositories in the Linux filesystem (for
example, below `~`), rather than `/mnt/c`. See Microsoft's
[WSL filesystem guidance](https://learn.microsoft.com/en-us/windows/wsl/filesystems).
Linux ARM64 and musl distributions are not included in this release matrix.

The installer resolves `latest` to one immutable version, downloads that exact
GitHub Release asset and checksum, verifies SHA-256, installs it below
`~/.local/share/codeclew`, and atomically links `clew` into `~/.local/bin`. It
never compiles Codeclew on the target machine. Run
`clew doctor attach --human` after installation and `clew upgrade` to install a
newer release. The source launcher `./clew` remains the supported development
entrypoint and is updated through Git, not `clew upgrade`.

If GitHub downloads return 403, manually download `install.sh`,
`install.sh.sha256`, the archive for your operating system and architecture, and its matching
`.sha256` file from one Codeclew Release into one directory. Install those local
bytes without network access by pinning their release tag:

```bash
CODECLEW_VERSION=v0.13.20 CODECLEW_ASSET_DIR="$PWD" /bin/sh ./install.sh
```

For WSL2, select `codeclew-linux-x86_64.tar.gz` and its `.sha256` file from a
release that includes Linux assets, then pin that release's version.

Local mode performs the same checksum, embedded-version, profile, and runtime
verification as the online installer. It refuses `latest`, relative asset
directories, and symlinked archive or checksum inputs. To upgrade without
GitHub access, put the newer archive and checksum in a local directory and run
the same command with its newer explicit tag. Keep the same
`CODECLEW_INSTALL_ROOT` and `CODECLEW_BIN_DIR` (or omit both to keep their
defaults); the installer adds the new version side by side and atomically
switches the `clew` launcher. It does not modify `CODECLEW_HOME`:

```bash
CODECLEW_VERSION=vNEWER \
CODECLEW_ASSET_DIR=/absolute/path/to/downloads \
/bin/sh /absolute/path/to/install.sh
clew --version
```

Core contains the sole Kotlin 2.4.10 analyzer for supported Kotlin 1.9–2.4
projects, including Kotlin 2.3.0. Project compiler, language and API versions
remain separate from the analyzer identity. Compiler-plugin, `-X` and Kotlin
1.9 language-upgrade boundaries remain explicit; this does not imply native
compiler equivalence, mutation qualification or support for Kotlin 2.5+.
Language pack commands and explicit `CODECLEW_PACKS=kotlin23` requests are retired.

If an older release has the optional Kotlin 2.3 profile installed, first switch
that **old release** to its existing core archive, then upgrade:

```bash
clew pack remove kotlin23
clew upgrade
```

This transition uses the older launcher's supported commands. Its old updater
otherwise preserves `kotlin23` and requests a removed asset; new updater code
cannot change those old installed bytes. Alternatively install current core
directly with the public installer and the pack setting cleared:

```bash
curl -fsSL https://codeclew.github.io/codeclew/install.sh | CODECLEW_VERSION=v0.13.20 CODECLEW_PACKS= sh
```

## Install the agent skill

Install the bundled, version-matched Codeclew skill for both Codex and Claude:

```bash
clew skill install
```

Use `--agent codex` or `--agent claude` for one personal installation, or
`--project /absolute/path/to/repository` to install discovery copies in one
project. Agent Skills-compatible tools with another discovery root can use
`--destination /absolute/path/to/skills`. The command is idempotent and refuses
to replace a different existing skill unless `--force` is explicit.

The portable source package is public at
[codeclew/codeclew-skill](https://github.com/codeclew/codeclew-skill). It uses
the open Agent Skills format; `skills/codeclew` in this repository is the exact
package bundled with releases.

## Start JVM analysis with your agent

After `clew skill install`, ask the agent to analyze the repository or document
its Spring computation roots. The skill uses bounded repository discovery to
select an available analysis profile and exact compilation:

```bash
clew doctor repository --repo /absolute/path/to/repository
```

The agent carries the returned `targetRef`, `profileId` and compilation into
`clew context open` or `clew nav query`. You do not need to choose an engine pack
for ordinary Kotlin/Java baseline analysis. The tool preserves the project's
compiler version and reports any analyzer differences; baseline access does not
enable mutation. See the support contract below for qualified capabilities and
[the architecture guide](https://codeclew.github.io/codeclew/architecture.html)
for the data flow and extension boundaries.

Language extraction is separate from framework interpretation.
`clew capabilities` lists installed analysis modules and their compiler/JVM
requirements. Java 17+ and the installed Kotlin engines emit portable annotation
facts; one Rust Spring module derives entrypoints with separate input and policy
provenance. Kotlin workers run on JDK 21 while project Maven/Gradle builds retain
their own JDK. Set `CODECLEW_WORKER_JAVA_HOME` to select the analyzer JDK;
`JAVA_HOME` continues to select the project build environment. Maven's reported
build JVM and Gradle's selected toolchain determine the analysis JDK classes.
Unknown cross-engine semantic flags are rejected with their option names and
compiler context. For Kotlin 1.9 projects, analysis explicitly ignores
`-Xannotation-default-target=param-property`; native project arguments are kept.

## Durable service documentation

`clew docs` maintains documentation in a separate architecture repository. The
native offline reader brings together a service overview, purpose and
responsibilities, domain information, incoming and outgoing interactions,
internal processes, source evidence and publication history. The agent authors
explanations from recorded evidence; Codeclew validates their bindings and
updates working pages through the stable `docs/index.html` entry point to the
current internal bundle while tracking which explanations need rechecking when
their inputs change. Only `docs render --publish` adds an entry to released
history. Retained content bundles continue to support evidence and
optimistic-baseline checks.

### Internal flows and decisions

Service pages support typed visuals:

- Execution-flow diagrams explain selected actions, ordering and branch guards.
- Dependency maps show structural relationships without implying execution order.
- Decision tables describe conditions and outcomes and can link to a particular
  node in the same operation's flow diagram. Policies such as `FIRST` include an
  explanation; they do not establish ordering between separate tables.

Purpose, scope and semantic claims cite evidence delivered to the author. Visuals
share their owning operation's accepted version, source bindings, freshness and
meaning-review status. Updating prose cannot silently erase existing diagrams;
replacing or removing them must be explicit. Old versions remain in history.
Rendering is local and does not require a CDN, a Mermaid installation or an
online model service. Tables document decisions; they are not executable DMN.

The overview shows authored service sections before detailed diagrams, and source
records open from the relevant explanation. Missing content, partial discovery
and unresolved library or runtime behavior remain visible. An empty inventory
is not evidence that the service has no internal processes or outgoing calls.

### Process flows and lifecycle states

Process roots are addressable handles in Work. A lifecycle view can use an
indented tree deepened from retained transformed source, or a PlantUML activity
diagram built from compiler FLOW events. The source-syntax path uses bounded
heuristics for supported constructs; unsupported or ambiguous syntax stays
conservative and visible as a limitation. It does not claim compiler resolution
or runtime behavior. Lifecycle state diagrams come from explicitly authored
`scenarios/<id>-states.yaml` declarations. Every supplied evidence reference
must resolve uniquely. Without explicit references, an exact full symbol or a
unique method name may resolve the operation. This checks identity, not whether
the declared transition is semantically correct or occurs at runtime.

PlantUML is optional. Install `plantuml` on `PATH`, or set `PLANTUML_JAR` with
Java available to produce SVG. The sealed Codeclew runtime does not bundle or
guarantee PlantUML. The flow tree and stored `.puml` source remain readable when
no renderer is installed; SVG generation is opportunistic.

### Start from saved evidence

Initialize a documentation root, register services using the generated examples,
and select the analysis appropriate to each repository:

```bash
clew docs init --root /work/architecture
```

The [packaged workflow](skills/codeclew/references/service-documentation.md)
covers registration, analysis, authoring, publication and history. For a concrete
source-bound visual proposal, follow
[publishing internal flows and decisions](docs/operations/field-documentation.md#publish-internal-flows-and-decisions-in-service-pages).
Generic Work/proposal authoring and configured process-overview jobs support
these visuals. The narrow `section-author-v1` contract remains summary-only.

For linked offline HTML and MDX pages derived directly from retained Java
endpoint, worker and wiring declarations, use
[native source documentation pages](docs/guides/native-static-pages.md).
The selector supplies exact roots; conditions, source expressions and local
uncertainty come from the saved evidence.

Set `"expandSourceCalls": true` on each selected process to follow exact compiler
call occurrences to selected child endpoint pages in the same service and
compilation scope, or to retained helper bodies. Expansion stops at depth two,
64 additional bodies or 1 MiB of additional source text. The separate
`examinedSourceDigest` and reverse membership describe only the examined source:
a child helper change can identify both examining parents, while unexamined
targets remain frontiers. Existing publication and file hashes keep their meaning.

### Maintain an attributed paragraph's source context

The manual Work/proposal route can edit an exact retained paragraph with
`explanationText`, or separately select its effective source context with
`explanationContext`. Context selection requires complete retained-operation and
current source reads, matching paragraph/context digests, compatible logical
source and dependency references, and exact current event anchors. Regenerate
stale source-derived fields first; changed scopes and ID remapping are unsupported.

Context selection preserves the paragraph text, ID and declared text author. Its
declared context editor, previous snapshot and context digest record the change.
Publish the prepared manual proposal with `--unassessed`. Later text edits keep
the selected context; a later source change can mark it stale without silently
rebinding it. Older publications keep their original text and source bytes.
Native page selectors can include an exact frozen publication's paragraph beside
current generated content, with separate source links and visible local freshness.
Read the [manual context-selection contract](docs/operations/docs-snapshot-store.md#explicitly-select-a-new-paragraph-source-context).

Use `docs check` when source acquisition or freshness checking is needed.
Ordinary context, Work and rendering consume compatible saved evidence; adding
an explanation or diagram does not require another indexing pass. Editing a
process-state sidecar changes captured documentation inputs; run
`docs recompose --snapshot ORIGINAL_CAPTURE` to refresh that declaration snapshot
offline, then render the returned snapshot. Recomposition preserves the original
source capture and does not reindex. Section Work
starts with a bounded orientation packet, and additional evidence is read on
demand instead of requiring the author to page through the whole dependency
inventory. To render an existing snapshot explicitly:

```bash
clew docs render --root /work/architecture --snapshot SAVED_SNAPSHOT
```

Use the actual snapshot reference returned by Codeclew. If a previous check
stopped before saving its final snapshot, `docs snapshot recover` can assemble
one from intact current-format service captures without the original checkout
or compiler. Recovery does not reverify source freshness. See
[saved-capture recovery](docs/operations/field-documentation.md#recover-saved-captures-after-a-failed-check)
for the required declarations and capture manifests.

The default render, proposal publication and successful Work jobs update working
documentation and the stable `docs/index.html` entry point. Add `--publish` to
`docs render` only when the render should also add an entry to released history.
Internal immutable bundles remain retained for evidence and optimistic-baseline
checks; cleanup is not automatic.

Publications share identical retained influence maps while keeping the evidence
needed by older accepted versions. An HTML reader upgrade alone does not require
reindexing compatible saved analysis. Old private formats are not migrated;
see the [release boundary](docs/operations/field-documentation.md#release-boundary).

### Choose the documentation language

Use `--language en` or `--language ru` when preparing author Work. The same
choice is available as `documentationLanguage` in the Work request JSON;
conflicting values are rejected. Without an explicit choice, new Work inherits
the publication language, or English if none has been selected.

```bash
clew docs work prepare --root /work/architecture --subject service:orders --snapshot SAVED_SNAPSHOT --input /work/request.json --language ru
clew docs render --root /work/architecture --snapshot SAVED_SNAPSHOT --language ru
```

The language guides the author and the native reader, including flow and decision
table captions. Russian output should use ordinary Russian engineering terms;
code, API and evidence identifiers remain unchanged. Language changes authoring
and publication identity, not source analysis. A render command does not translate
existing prose: a section in another or unknown language shows a translation gap
and, when available, a link to its original publication. Old snapshots and accepted
versions remain intact. Language metadata is not proof of translation quality;
meaning review and unassessed publication remain separate.

### Analysis choices and limits

The `source-syntax` profile supports Python, Java and Kotlin 1.9 source scopes
without running a project build or K2. It records lexical declarations and
control structure while preserving unresolved targets. Optional compiler
enrichment attaches to the same source objects. Java 17+ and supported Kotlin/JVM
Maven/Gradle analysis provide stronger compiler evidence where available.
Compiler differences, callback scheduling and external-library frontiers remain
explicit boundaries. See
[build-independent documentation](docs/operations/source-documentation.md).

Capture alone does not produce a complete business explanation. Selecting useful
process boundaries and interpreting source still require an author. Local agents
can submit proposals and publish with `--unassessed`; a model API is optional,
and meaning review is separate from source freshness. Source citations and
structural validation do not prove that a model's explanation is correct.
Codeclew does not guarantee complete business-flow discovery, deployed behavior,
or a universal generator plugin SDK.

Offline help and product runbooks are available at `docs/help.html` and
`docs/runbooks.html`; service-specific procedures must come from service sources
and notes. The [two-service fixture](fixtures/durable-docs/README.md) demonstrates
guards, contracts and recovery, and the
[Kotlin fixture](fixtures/durable-docs-kotlin/README.md) exercises compiler-backed
HTTP/Kafka extraction. Project builds still depend on the target environment's
configured caches and mirrors. For the internal-method draft introduced in
v0.13.1, see its [release notes](docs/releases/v0.13.1.md), [reproduction recipe](docs/operations/task-promotion-draft.md),
and [sample reader](fixtures/documentation-internal-process/documentation/sample/index.html).

## Source-build requirements

For agent-assisted source development, start with [AGENTS.md](AGENTS.md).
It defines the ordinary development workflow, focused verification, and the
boundary between editing Codeclew and using its installed-release skill.

- macOS or Linux
- Python 3.11+
- Node.js 22 for the development/CI reader tests (not needed to open generated documentation)
- Git
- JDK 21
- .NET 10+ SDK for C# analysis, full CI and release qualification; otherwise the source bootstrap omits the optional C# worker
- the Rust toolchain pinned by `rust-toolchain.toml`
- Cargo on `PATH` for Rust repositories
- Maven on `PATH` only for Maven projects without `./mvnw`

The sole Kotlin 2.4.10 analyzer is packaged into the immutable core runtime
capsule. A cold source runtime start may build the capsule. A warm runtime invocation
verifies and reuses it without running Cargo, Rustc, Gradle, or Maven. The
workers execute directly from that sealed capsule under its shared lease; the
runtime warm path does not copy their distributions. Project analysis may still
invoke the project wrapper under the model-cache policy described below.

### Analysis with local edits

Use `--working-tree` on `doctor repository`, `doctor task`, `context open` or
`nav query` to analyze saved Kotlin or Rust edits. Codeclew captures tracked
and non-ignored untracked files into immutable managed storage. A staged
version and a later unstaged version remain separate provenance; analysis
reads the saved working-tree bytes. The session reports its source selection,
base commit, captured snapshot, profile and selected compilations.

```bash
./clew nav query --repo /absolute/repository --target-ref main \
  --language kotlin --profile kotlin-jvm-gradle-analysis --compilation :/main \
  --working-tree --term calculatePrice --source
```

Capture includes repository inputs outside the selected compilation so the
project model can resolve its inputs; semantic conclusions cover only the
selected compilations. Ignored untracked outputs, legacy managed state and
unsaved editor buffers are excluded. Conflicted indexes and links are rejected.
Limits are 32,768 paths, 16 MiB per file, 256 MiB across indexed and saved bytes,
16 MiB / 15 seconds per Git inventory pass, and 4,096 bytes / 128 components
per path. Two complete content passes and
HEAD/index/inventory checks establish observed stability, not an atomic
filesystem transaction. Detected drift fails with `INPUT_MUTATED`; overflow
fails with `RESOURCE_LIMIT`. Neither outcome substitutes committed HEAD.

After opening, keep editing normally. `change check-freshness --session <id>`
reports `LIVE_CHANGED` separately from retained evidence validity. The retained
context still reads its original snapshot. Refresh by opening a new session;
close and collect finished sessions with `session close` and `session gc`.
Working-tree sessions reject mutation preparation and publication.

To analyze committed HEAD while excluding local edits, use `--committed`.
The two source flags conflict. Existing defaults and clean-target mutation
requirements remain in effect. The user checkout, index and refs are preserved.

Compare saved Kotlin/Gradle or Rust edits against pinned HEAD with the same
repository, ref, language, profile and compilation flags:

```sh
clew change inspect --repo <repo> --target-ref <branch> --language kotlin \
  --profile kotlin-jvm-gradle-analysis --compilation :/main --working-tree
clew change show --comparison <comparison-id>
clew change graph --comparison <comparison-id>
clew change render --comparison <comparison-id> --output <new-report.html>
clew change check-freshness --comparison <comparison-id>
clew change forget --comparison <comparison-id>
```

The retained report separates exact text changes, declaration correspondence,
projected shape changes and unavailable analysis. It includes both model input
bindings, explicit coverage limits and before/after source anchors. Inspect
collects its temporary sessions; the report stays readable until forgotten.
Tests are not executed by inspection.

### Developing Codeclew itself

The installed agent skill governs use of Codeclew on a target repository.
Contributions to Codeclew itself use this checkout's native workflow: inspect
and edit the source, use the pinned `./clew` launcher where needed, and run the
relevant Rust, worker or packaging checks. Consumer task admission is not a
prerequisite for maintainer edits. Preserve unrelated work and use an isolated
worktree when appropriate; a consumer `CLEAN_TARGET_WORKTREE` result does not
require cleaning the contributor's checkout or stopping implementation.


## Practical code navigation

`nav query` is the shortest fact-backed path from a search term to code. It
performs task admission, opens the managed session and returns at most three
fact-bound decision cards with exact one-line source previews in one command:

```bash
./clew nav query \
  --repo . \
  --target-ref main \
  --language rust \
  --profile rust-syntax \
  --compilation 'cargo:crates/clew/Cargo.toml#clew#lib#clew' \
  --term fair_fact_selection \
  --decision-identifier fair_fact_selection \
  --source
```

The response retains `sessionId`, `contextId`, and the full evidence digest.
Every response also contains
`codeclew-navigation-decision-authority/1.0`. A candidate becomes the selected
code identity only when the caller explicitly supplies its task-derived exact
name or identity with `--decision-identifier`, exactly one retained declaration
matches it, boundary-safe declaration evidence covers every normalized query
term, and the underlying query is not truncated. With `--source`, that
`SUPPORTED/UNIQUE_EXACT_IDENTIFIER_FULL_COVERAGE` candidate receives exact
retained source and declaration-to-window bindings. Generic search terms alone,
unmatched terms, underlying query truncation, ambiguous identities, or partial
local coverage produce `ABSTAIN`: cards remain discovery evidence,
`decisionSource` is unavailable and reference follow is disabled. A structured
refinement is executable only once and only when the task supplies a new exact
identifier absent from the current query; otherwise it returns
`STOP_UNRESOLVED` and forbids replaying the same request. The legacy
`truncated` flag combines underlying query truncation with the three-card
presentation cap; `queryCoverageTruncated` and `candidateListTruncated` separate
those cases. Candidate uniqueness is evaluated across all retained declaration
facts, so list truncation alone does not authorize or veto a decision. Candidate
and stdout caps remain unchanged. The complete immutable evidence remains bound
by `evidenceDigest`; omit `--source` when names and locations are enough.
Select one to three cards in one call to receive each complete retained fact
and its single exact source window without retransmitting every alternative:

```bash
./clew nav expand \
  --session session:... \
  --from context:sha256:... \
  --candidate c:0123456789abcdef \
  --candidate c:fedcba9876543210 \
  --source \
  --facet callers
```

If the decision cards are insufficient, add one discriminative identifier with
`--term rank_fact_evidence` instead. Term selection and candidate selection are
mutually exclusive.

When an exact identifier and repository-relative file are already established,
expand and select them atomically:

```bash
./clew nav expand \
  --session session:... \
  --from context:sha256:... \
  --term rank_fact_evidence \
  --file crates/clew/src/context_v2.rs \
  --source
```

This succeeds only for one exact declaration. No match and same-file overloads
remain typed `SYMBOL_NOT_FOUND` or `AMBIGUOUS_SYMBOL` results.

When the task already supplies a non-identifier literal, `nav locate` searches
only an explicit set of files in the session's immutable source snapshot. The
request is a caller-owned mode-0600 JSON file whose paths are sorted and unique:

```json
{"schema":"codeclew-source-locate-request/1.0","literal":"local-trace.json","paths":["launchpad-web/src/main/kotlin/example/ReplayRoutes.kt"],"maxMatches":3}
```

```bash
./clew nav locate \
  --session session:... \
  --from context:sha256:... \
  --request /absolute/private/source-locate.json
```

The result contains exact non-overlapping UTF-8 byte coordinates, the request
digest, and snapshot authority, but no source text. Missing files, unsafe paths,
symlinks, and oversized scopes fail closed. If the match limit is exceeded,
the count is returned without leaking a partial coordinate set. This is lexical
source navigation; it does not claim compiler resolution or a semantic relation.

For source-navigation-only tasks that cannot open a language context, the same
request can be resolved directly from a clean repository's pinned Git commit:

```bash
./clew nav locate \
  --repo /canonical/absolute/repository \
  --target-ref main \
  --request /absolute/private/source-locate.json
```

This direct mode requires the canonical Git root, no staged or tracked worktree
changes, and a branch resolving to the same commit as `HEAD`. It reads regular blobs from
that commit rather than live worktree files, rejects symlinks and submodules,
and returns `codeclew-source-locate-result/1.1`. Its source authority exposes
the pinned commit/tree and only digests for repository/branch identity; it
creates no session and makes no compiler-backed claim. The session/context mode remains
`codeclew-source-locate-result/1.0` and retains its lifecycle admission lock.

`nav expand` returns a delta against its exact `parentContextId`, not another
copy of the cumulative candidate list. Apply `candidateDelta.upserts` and
`candidateDelta.removals` to the retained parent candidates; an empty upsert
array means that no candidate card changed, not that the child context has no
candidates. Then order the reconstructed cards by `candidateOrder`; ranking
can change even when card bytes do not. `unchangedCount` reports the cards
deliberately omitted from stdout. The child `contextId` and `evidenceDigest` still bind the complete
immutable context in managed CAS. Candidate detail returns the exact selected
payload and only the overlapping retained source window; it fails closed when
there is no exact line mapping.

`--intent` is optional provenance and never changes retrieval or ranking.
On candidate detail, `--facet callers`, `--facet callees`, and `--facet tests`
return only direct, identity-bound relation facts for that selected symbol. A
missing relation is reported as `UNSUPPORTED`; a non-empty bounded subset is `PARTIAL`.
Syntax-only namesakes are never promoted to resolved edges. There is no
`--all`: narrow or expand the context when the bounded response reports
truncation.

The strict compiler-backed mutation contour is Kotlin 2.4.10, Gradle,
`PROJECT_NATIVE`, and one exact compilation. Baseline Kotlin 2.3.0 projects,
including Maven, use the core 2.4.10 analyzer for read-only context and
documentation. Original project compiler/language/API identity remains retained;
this does not establish exact project-compiler semantics or a mutation/publish
claim. The existing `kotlin-2.3.0-maven-single` read-only profile also requires
core 2.4.10 and reports conditional analysis authority. Kotlin 2.1, multiple
compilations, Android/KMP and `EXTERNAL` remain unqualified until they have
their own acceptance tests. Rust and Python are operationally `PILOT_READY` for
conditional mutation, with the weaker evidence boundaries described below.

Java 17+ has a compiler-backed read-only preview for project-native Gradle and
Maven builds. Open a session with `--language java` and an exact `:/main`,
`:/test`, `:module/main`, or `:module/test` compilation. Codeclew uses the JDK
Compiler API to return resolved declarations, JVM descriptors, annotations,
calls and type-use relations with source anchors. A clean Gradle fixture has
passed the public `session open -> context create` path with `COMPLETE/VERIFIED`
evidence; the same fact contour is qualified on both Gradle and Maven fixtures.
The Spring catalogue derives annotation-declared entrypoints from resolved
compiler facts. Generated source declarations and Java mutation remain outside
this profile. Unsupported toolchains and unresolved compiler diagnostics
remain typed boundaries instead of exact claims.

For Java/Maven, choose a private settings file in caller-local `codeclew.yaml`:

```yaml
version: 1
maven:
  settings: ../private/settings.xml
```

This file never needs a preparation commit for Java analysis: untracked, staged,
and locally modified tracked configurations are accepted. Its path is resolved
relative to the repository root. `--maven-settings /path/to/settings.xml` on
`nav query`, `context open`, `session open`, or `doctor task` overrides this
selection; a relative CLI path uses the caller's current directory. Without
either setting, Maven keeps its native settings selection. Codeclew validates
the YAML, binds explicit settings bytes to the session, and keeps their path and
contents private. Reopen the session after changing the selected settings.

An existing `mvnw` with a supported sh/bash shebang runs through its interpreter
when its executable bit is absent, without chmod or a commit. Maven compilation,
source generation, and classpath extraction run in the managed workspace; module
selection uses the native reactor without requiring `mvn install`. Compiled
generated types can resolve references from original source objects, while
`JAVA_GENERATED_DECLARATIONS_NOT_INDEXED` marks the generated declarations omitted
from the source index. Use exact source returned by `nav query --source` or
`nav expand --source` to document retained objects, and preserve this boundary.

TypeScript 5 and JavaScript use the project-local TypeScript compiler through an
exact `tsconfig:<repo-relative-json>` authority. Open them with `--language
typescript` or `--language javascript`; Codeclew never downloads a compiler or
uses a global `typescript` module. TypeScript and `checkJs` JavaScript expose
compiler-resolved declarations, shapes, imports, calls and type uses.
JavaScript without `checkJs` is explicitly either declaration-typed and
conditional, or syntax-only with `SYNTAX_OBSERVED` facts and `<unchecked>`
shapes. Project references, mixed-language files, missing dependencies, `any`
types and unresolved calls remain named boundaries. Both profiles are read-only
previews and reject candidate generation.

C# uses a Roslyn worker included in the published macOS and Linux archives.
Analyzing a project requires a .NET 10+ SDK (`dotnet`) on `PATH`; the SDK is
not bundled. Source bootstrap builds this optional worker only when that SDK
is available; otherwise `clew capabilities` omits `csharp-roslyn` and C#
sessions report `INSTALL_CSHARP_COMPONENT`. Open a
session with `--language csharp`, profile `csharp-dotnet-msbuild-read-only`, and
an exact `csproj:<path>[@<tfm>]` or `sln:<path>` (`.sln`/`.slnx`) selector. The
worker runs the project's own SDK design-time `Compile` target in-process and
builds Roslyn compilations from the exact csc command lines. It never restores
packages: run `dotnet restore` first. Unrestored projects, compiled files
outside the repository, source-generator output, and generated sources (such
as `Connected Services` proxies) remain named boundaries. MSBuild output is
redirected to private scratch, and a changed `obj/` or `bin/` entry fails the
analysis. Facts include declarations with typed attributes, calls,
constructions, overrides, implemented interface members and bounded
documentation flow. `clew entrypoints` and `clew docs` derive ASP.NET Core MVC
attribute routes through the `aspnetcore` package. Custom controller discovery,
conventional routing, minimal APIs and API-version URL formats stay explicit
boundaries. The profile is read-only and rejects candidate generation.

Where `dotnet` cannot reach NuGet through its own TLS stack, populate a local
folder feed with `scripts/fetch_dotnet_worker_packages.py --feed DIR`. Then
bootstrap with `CODECLEW_DOTNET_PACKAGE_SOURCE=DIR`. The lock file's content
hashes still bind every package.

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

The repository-owned Rust/Python qualification runs three independent changes
per language and requires idempotent prepare/publish, exact writes, native tests
and managed GC. Run it with
`python3 -I -S scripts/language_mutation_pilot.py`; a passing profile is ready
for limited team use, while its evidence remains `PARTIAL/UNSURE`.

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
readiness. When auditing an unfamiliar repository, first discover its bounded
research contours:

```bash
./clew capabilities --human
./clew doctor repository --repo /absolute/path/to/repository --human
```

The repository report identifies the checked-out ref, detected languages,
supported profiles, and exact compilation selectors. Its
`READY_FOR_TASK_DOCTOR` contours are candidates, not task admission. Choose the
contour that matches the requested work and confirm it with the exact task
doctor arguments:

```bash
./clew doctor task \
  --repo /absolute/path/to/repository \
  --target-ref refs/heads/feature \
  --language python \
  --profile python-syntax \
  --compilation 'python:.#.' \
  --operation analysis \
  --human
```

The support matrix is also retained as
[`crates/clew/support-matrix.json`](crates/clew/support-matrix.json). Automation
uses the canonical JSON emitted when `--human` is omitted and must inspect the
doctor's status and required checks. A doctor command can return a valid
`ACTION_REQUIRED`, `PARTIALLY_READY`, or `UNSUPPORTED` report without treating
that report as a CLI failure. Repository discovery contains ref and relative
selector identity, but never source or an absolute repository path; keep its raw
JSON local.

Before prepare and publish, verify that the session still owns the checked-out
target authority:

```bash
./clew change check-freshness --session session:...
```

Repository/task admission, direct source locate, and target freshness ignore
untracked files, including local plans and notes. Analysis remains bound to the
selected Git commit: untracked files are not included in its source snapshot.
Staged additions and changes to tracked files still block these cleanliness
checks, except caller-local `codeclew.yaml` during Java analysis as described
above. Publication preserves unrelated untracked files and rejects candidate
paths that would overwrite them; managed candidate/source integrity and cleanup
checks still account for untracked outputs.

`DIRTY` preserves developer work and requires a human decision. `STALE`
requires a new session/context/plan; Codeclew never rebases or replays the old
plan automatically. For incident sharing, keep raw JSON local in a mode-0600
file and export only the allowlist summary:

```bash
./clew support summarize --input /absolute/private/path/result.json
```

The converter also accepts `docs check` results and their pages, keeping only
failure counts, typed codes, and safe worker process details. Worker transport
failures retain a private stderr tail of at most 64 KiB, with the file location
and observed exit status in the diagnostic evidence. See the
[worker crash runbook](docs/operations/p0-runbook.md#worker-crash) for capture
limits and the fixed JVM memory settings.

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

Managed CAS space is accounted separately from session/worktree cleanup:

```bash
./clew storage gc          # reachability dry-run; changes nothing
./clew storage gc --apply  # reclaim exactly the reported unreachable bytes
```

`session close` preserves the session's evidence roots. `session gc` is the
explicit retention boundary: after its hash-chained lifecycle reaches
`GARBAGE_COLLECTED`, that session and its exact verified runs no longer keep CAS
objects alive. Physical bytes are still removed only by the separate
`storage gc --apply` command; evidence also referenced by a mission, workspace,
thread, repository, or another retained root remains reachable.

GC follows every retained repository, session, mission, workspace, thread,
run, and generation metadata CAS reference transitively. Repository source
trees, candidate worktrees, and the compiler store are opaque derived payloads,
not metadata roots; GC never parses project JSON inside them. In-progress CAS
work holds the shared world lease, so an exclusive collector cannot race an
unpublished attempt. It removes a pack only when every member is unreachable;
mixed packs remain intact. An
exclusive CAS lease delays physical deletion while any Codeclew reader or
writer is active.

The dry-run reports `collectionBlocked: true` and proposes zero reclaimable
bytes when retained metadata names an object that is physically absent. Apply
then fails closed; corrupt bytes at an existing location remain
`STATE_CORRUPT`. Catalog snapshots are compact maintenance over the durable
append-only journal. An oversized snapshot is deferred while a bounded journal
allows REMOVE records to shrink the catalog; new ADD records stop before
publication if that recovery bound is exhausted.

### Durable development missions

A mission binds one canonical `codeclew-change-spec/1.0` to one through eight
already-open sessions. The spec gives requirements, non-goals, acceptance
criteria, and documentation policy stable IDs. Keep it in a private mode-0600
file and record existing immutable context, plan, and run authorities instead
of copying their mutable output into prose:

```bash
./clew mission open --session session:... --spec /absolute/private/change-spec.json
./clew mission record \
  --mission mission:... \
  --session session:... \
  --context context:sha256:... \
  --plan plan:sha256:... \
  --run run:...
./clew mission inspect --mission mission:...
./clew mission status --mission mission:...
./clew mission close --mission mission:...
```

Mission events are append-only, content-bound, bounded, and path-free on
stdout. A failed or conditional run remains failed or conditional in the
mission record; mission status never upgrades its certainty or substitutes for
the existing publication checks.

Once the mission has a context/plan/run binding, the agent may submit a typed
`codeclew-development-record-input/1.0`. Claims can be `EXACT`, `OBSERVED`,
`DECLARED`, `CONDITIONAL`, or `UNSURE`; Codeclew resolves JSON pointers against
the bound immutable context evidence, resolves operation IDs against the bound
plan, and resolves validation against the bound run. Missing requirement,
acceptance, changed-file, or documentation links remain explicit obligations:

```json
{
  "claims": [
    {
      "acceptanceCriterionIds": ["A1"],
      "certainty": "UNSURE",
      "documentation": [],
      "evidence": [],
      "id": "C1",
      "obligations": ["Verify the scenario before treating the claim as exact"],
      "operations": [],
      "requirementIds": ["R1"],
      "text": "The scenario remains compatible",
      "validationSessionIds": []
    }
  ],
  "schema": "codeclew-development-record-input/1.0"
}
```

Keep the canonical compact JSON input mode `0600`, then create and review the
immutable record:

```bash
./clew mission develop --mission mission:... --record /absolute/private/record.json
./clew mission dossier --mission mission:... --record development-record:sha256:... --format markdown
./clew mission dossier --mission mission:... --record development-record:sha256:... --format dot
./clew mission dossier --mission mission:... --record development-record:sha256:... --node claim:...
```

The full dossier and DOT graph are deterministic projections. Selecting a node
returns that node and its own evidence only; it never substitutes one shared
graph-level evidence block. If a retained context, plan, or run authority later
becomes unavailable or changes, only claims that depend on it are downgraded
and aggregate readiness becomes `CONDITIONAL`.

### Mission-bound local workspaces

A workspace is the durable two-to-four-repository development boundary above a
mission. It resolves one explicit private catalog to exact session authorities,
declared dependency edges, and the mission's immutable ChangeSpec identity. It
does not discover repositories, clone anything, infer topology, or build a
combined index.

The canonical compact catalog is a mode-0600 file. Every catalog member must be
an open session already bound by the mission, every mission member must appear
exactly once, and members must refer to distinct repositories:

```json
{"edges":[{"id":"api-client","relation":"depends-on","source":"api","target":"client"}],"members":[{"alias":"api","sessionId":"session:..."},{"alias":"client","sessionId":"session:..."}],"missionId":"mission:...","schema":"codeclew-workspace-catalog-input/1.0"}
```

```bash
./clew workspace open --catalog /absolute/private/workspace-catalog.json
./clew workspace inspect --workspace workspace:...
./clew workspace context \
  --workspace workspace:... \
  --intent 'compare the selected service boundaries' \
  --term Service \
  --term Repository

./clew workspace prepare \
  --workspace workspace:... \
  --request /absolute/private/workspace-prepare.json
./clew workspace observe \
  --workspace workspace:... \
  --request /absolute/private/scenario-observation.json \
  --evidence /absolute/private/scenario-raw-evidence.bin
./clew workspace publish \
  --workspace workspace:... \
  --request /absolute/private/workspace-publication.json
./clew workspace recover \
  --workspace workspace:... \
  --publication workspace-publication:sha256:...
./clew workspace close --workspace workspace:...
```

The mode-0600 prepare request binds one existing context and immutable plan per
member:

```json
{"members":[{"alias":"api","contextId":"context:sha256:...","planId":"plan:sha256:..."},{"alias":"client","contextId":"context:sha256:...","planId":"plan:sha256:..."}],"schema":"codeclew-workspace-prepare-input/1.0"}
```

`workspace prepare` starts or attaches to each deterministic task run and emits
an immutable `PREPARED_ALL` `AfterWorkspace` only after every candidate has
passed its own validation. It never updates a target ref. Declared edges bind
both exact candidate OIDs for later combined checks without replacing member
authority or promoting catalog certainty. An identical repeat returns the same
retained authority without starting task runners.

`workspace observe` adds provider-neutral runtime evidence without promoting it
to compiler or contract certainty. Its canonical request binds the exact
`AfterWorkspace`, provider/action/config digests, time window, aggregate status,
and checks. Every check digest must match the exact private raw evidence file;
stdout contains only the path-free CAS reference and certainty axes. Identical
input is idempotent, while a genuinely new observation has a new content
identity.

`workspace publish` seals every member, its reviewed prepared digest,
conditional obligation acknowledgements, the publication order, and
`ROLL_FORWARD_ONLY` policy before the first ref update. The append-only ledger
can stop at `RECOVERY_REQUIRED` after a partial publication, but never rolls a
published candidate back. `workspace recover` resumes only the sealed suffix.
Both commands are idempotent at `PUBLISHED_ALL`. This local saga is intentionally
not advertised as a cross-repository atomic transaction, and JVM multi-service
mutation remains gated by three independent qualification cases.

Member and edge order cannot change the workspace identity; changing an alias,
edge, session, repository revision, mission, or ChangeSpec does. Context reuses
the existing globally bounded multi-repository composition engine, so each fact
retains its member/session/revision/evidence authority and no mega-generation is
created. Catalog edges are only `DECLARED_CATALOG`: compiler shape, artifact
ownership, contract verification, and observed runtime remain independent
`UNKNOWN` axes until a later authority proves them. Closing a workspace closes
only its private analysis view; it never closes, mutates, publishes, or collects
a member session.

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

When the selected pair contains one Kotlin and one Java member, the same command
selects the bounded JVM Navigation v1 projection instead. It resolves only
compiler-emitted declarations and references under the consumer's exact
classpath authority. Exact descriptors win directly; a descriptor-less Kotlin
callable family wins only when the provider has exactly one candidate. Overloads,
missing generated/local declarations, Kotlin nullability, artifact ownership,
and binary compatibility remain explicit boundaries or obligations. This
qualified contour joins independent repositories through built artifacts; a
joint Kotlin/Java source compilation is not yet supported.

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

For syntax-backed Rust and Python facts, an exact matched declaration includes
its complete declaration body when that body fits the existing 32 KiB source
budget. Larger declarations retain the bounded line-window fallback. This
improves access to inner assertions and literals without changing
`PARTIAL/UNSURE` certainty or claiming name resolution.

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
missions/
workspaces/
threads/
runs/
objects/
  sha256/
  packs-v3/
  catalog-v1/
    snapshots/
    records/
locks/
tmp/
quarantine/
```

Opening a session also creates Codeclew-owned Git worktree administration under
the repository's Git common directory and may create source or candidate
worktrees inside managed state. Sandboxes and benchmark harnesses must allow
those writes, including for read-only analysis. This authority does not permit
implicit edits to the user's checkout or target ref: candidates change only in
an admitted mutation workflow, and the target ref changes only through explicit
`change publish`.

The pack catalog is not rebuilt on every request. One append-only immutable
record publishes each pack addition or removal. Every 64 records Codeclew
atomically advances an immutable snapshot, then deletes only the records and
older snapshots covered by the new head. A reader uses its in-memory catalog,
loads a snapshot once per process, and consults the bounded journal tail only
on a digest miss or before publication. Pack/object bytes are still checked
against the requested CAS digest when read. Once a pack is durably present in
the catalog, its redundant migration index and verification receipt are
removed; an interrupted pre-catalog publication keeps them for the one-time
bootstrap/recovery scan.

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
not prerequisites for the supported Kotlin 2.4 publish path. Historical Kotlin
2.3.0 qualification used the now-retired exact 2.3 analyzer. Its recorded
admission, capsule, fact-translation and read-only context results remain
historical evidence; they do not qualify the current core 2.4.10 analyzer's
semantics or extend the publish contour. Current CI separately verifies that a
baseline 2.3 project opens once with core while retaining its declared compiler,
language and API versions, and that retired analyzer startup fails explicitly.

The CLI writes canonical JSON to stdout and diagnostics to stderr. The system is
fail-closed for stale authorities, ambiguous anchors, unsupported project
models, dirty checked-out publication targets, and recovery uncertainty.
Each cold generation also retains a private `dag-report.json` with total work,
critical path, observed parallelism, and the number of sealed compiler streams.

## Spring computation-root catalogue

Once a Kotlin or Java analysis session has produced its sealed generation,
enumerate HTTP endpoints, Kafka listeners and scheduled jobs directly:

```bash
clew entrypoints --session <session-id> --limit 100
```

Repeat `--session <session-id>` for additional selected sessions. A bound
multi-repository thread can supply its members instead:

```bash
clew entrypoints --thread <thread-id> --limit 100
```

Session selection and thread selection are mutually exclusive. Results contain
`catalogueDigest`, `total`, `entries`, per-compilation `scopes` and `nextCursor`.
When the cursor is non-null, repeat the same selection with
`--cursor <returned-cursor>` until all pages are consumed. `--limit` accepts
1–100; the output byte budget may return fewer entries. A cursor is valid only
for its original immutable catalogue.

Each root retains a callable identity, source anchor, annotation binding and
fact/generation evidence. Use those identities for subsequent context and
thread analysis. Annotation identities and attributes come from K2 or the Java
Compiler API; Spring interpretation uses stable annotation names rather than a
Boot patch-version check. Coverage boundaries remain visible, including missing
extraction, unresolved expressions and runtime activation. An empty page is not
proof that every runtime trigger has been discovered. Transport names alone do
not prove HTTP/Kafka handoffs across repositories.

See [Spring acceptance scope](docs/plans/spring-entrypoints.md) and the
[modular JVM rollout plan](docs/plans/modular-jvm-roadmap.md).

## Repository map

- `crates/clew`: Rust core, supervisor, sessions, indexes, and CLI
- `workers/kotlin*`: version-pinned Kotlin compiler workers
- `bootstrap`: isolated content-addressed runtime bootstrap
- `schemas`: typed worker protocol
- `fixtures`: executable Kotlin corpus
- `scripts`: CI, demo, and benchmark entrypoints
- `docs`: architecture and experiment history

Licensed under the [Apache License 2.0](LICENSE).
