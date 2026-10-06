# Question results and incremental evidence

Status: authorized on 2026-10-06; execution started.
Baseline: release `v0.13.14`, main `496284241625ee86fa61d8d5422739601ac92d28`.

## Outcome and ownership

An engineer can ask a source question, inspect its evidence, reuse an unchanged
answer, and update only affected explanations after a source change. Small
questions should avoid loading unrelated evidence. Repeated source analysis and
model input should be reduced where correctness and measured benefit justify it.

The research chat owns task specification, independent acceptance and priority
decisions. The existing coding chat owns implementation, focused verification,
integration and release execution. Keep one bounded implementation slice active.
An implementation report is not an independent acceptance verdict.

This plan authorizes implementation and release within this scope. No additional
approval or attestation layer is required. Publish meaningful accepted increments
as subsequent `0.13.*` releases. Do not issue a release solely to change this plan.

## Starting facts

- Saved-only consumers, immutable snapshots, shared payloads, CAS catalog fixes,
  grouped context requests and independent reviewer expansion already exist.
- Saved-answer context comparison already distinguishes selected evidence changes
  from source-link changes; it does not perform new meaning review or create a
  reusable answer-selection service.
- Current Check/Work reads still hydrate broad maps before selecting context.
- Syntax capture reuse includes revision and service identity. Compiler capture
  reuse requires complete build-input authority; source equality alone is unsafe.
- The accepted five-question comparison preserved semantic packets and supported
  all 18 checked facts. Successful-role CLI input was 348,827 versus 415,099 tokens;
  native role time was 977.426 versus 959.548 seconds. No expansions occurred.
  This establishes neither a general speedup nor benefits of smaller initial
  semantic context. Historical unknown timeout usage remains unknown.

## Work sequence and decision points

### 1. Choose one useful path and establish its current cost

Use an existing reproducible source question and saved evidence. Inspect the
actual current path before adding instrumentation. Record only measurements
needed to choose the first implementation: loaded objects/bytes, elapsed time,
memory where available, delivered model input and repeated native acquisition.

Cover these operations across the subsequent slices:

1. Ask and inspect a useful question about a method or endpoint.
2. Repeat the identical question against unchanged evidence.
3. Change an unrelated method or file.
4. Change a relevant guard, effect, target or declared external contract.
5. Add a newly matching result, testing selection membership and absence claims.

Reuse accepted correctness and capture evidence where behavior is unchanged.
Do not begin by rerunning every paid case or building a general benchmark system.
The first task must end with a concrete implementation candidate, baseline fact
and a narrow acceptance test, rather than a new broad architecture document.

### 2. Read selected evidence before materializing unrelated payloads

Extend the existing immutable index with a scoped evidence reader for the chosen
path. Resolve identities and necessary adjacency first, then load the required
evidence closure. Preserve source bytes, provenance, boundaries and cursor scope.

Compare old and new results on the same snapshot. Check relevant corruption and
missing objects explicitly; never treat unread evidence as proven absence.
Distinguish selected-read validation from an exhaustive store integrity audit.

Accept when the public path returns equivalent useful evidence with materially
less irrelevant IO/memory or lower observed latency. If the current path is
already cheap, narrow or defer this work and advance the more valuable slice.

### 3. Reuse question results with precise dependencies

Build on Work, saved answers and answer-context. Add the smallest user-facing
path that finds an exact reusable answer and explains why it is reusable or stale.
Bind reuse to question/scope, selected evidence, relevant declarations and notes,
producer/selection policy and applicable review policy. Preserve historical
authorship and review; reuse is not new review or runtime verification.

Track dependencies on selectors, inventory membership and negative results in
addition to returned facts. A new endpoint or possible callee must invalidate
answers whose completeness depends on that selection. Do not infer equivalence
from similar question wording or matching cited lines alone.

Expose a task-oriented facade over existing lifecycle commands. Return a readable
answer, its evidence and next action. Keep expensive acquisition explicit and
visible, without adding routine human approval prompts. Keep publication and
source mutation as separate actions. A common query result contract should grow
from this working slice; do not rewrite every thread/docs model up front.

Accept when identical input reuses a result without an avoidable model call,
unrelated changes preserve valid results, and relevant/membership changes report
the right invalidation. Update only affected explanation units where supported.

### 4. Reuse syntax extraction by content and producer

Separate reusable syntax payloads from occurrences at a path/revision/scope.
Use complete syntax inputs: content, language/dialect, producer and any additional
inputs the extractor actually reads. Bind reused results to current occurrences
without inventing stable identity after unsupported moves or scope changes.

First qualify two revisions with one changed file: only necessary syntax work
repeats, results equal a full extraction, and source links remain exact. Include
duplicate content at different paths and a changed producer/dialect boundary.

Compiler reuse is a separate conditional extension. Proceed only when a complete
fingerprint of the compilation, ordered dependencies, toolchain, settings,
processors and generated inputs can be established with meaningful benefit.
Do not weaken existing non-cacheable boundaries to obtain a faster benchmark.

### 5. Give each model role one sufficient evidence representation

The user additionally proposed shortening identifiers in model context. Evaluate
this as an early, bounded candidate in this slice: use compact packet-local
aliases with an exact host-side mapping to canonical identifiers. Measure actual
model-input token reduction on existing representative prompts, including any
mapping or protocol overhead. Preserve canonical stored identities and evidence
binding. Resolve citations and expansion requests through the mapping; reject
unknown, ambiguous or wrong-packet aliases. Keep aliases stable across grouped
expansions of one run and qualify independent reviewer access. Avoid merely
truncating hashes or replacing text inside source code. If the current driver
already shortens identities, measure the remaining opportunity before changing it.

Separate retained audit data from actual model-facing input. Reuse the existing
compact presentation and grouped-read machinery. Preserve one clear source copy,
resolvable citations, necessary guards/effects and evidence limitations. Retain
raw pages and receipts in the controller's archive with checked delivery binding.

Inspect the actual driver/provider prompt: redundant native envelopes do not by
themselves prove redundant provider tokens. Start with one real expansion case.
Keep independent reviewer access to uncited source and additional evidence.

Then evaluate task-specific initial selection and semantic grouped expansions.
Do not impose arbitrary new byte/node/call limits or optimize initial size alone.
Count all author, expansion, reviewer and failed-call inputs and elapsed time.
Unify divergent role-selection contracts only where it removes measured user or
maintenance cost; preserve role-specific outputs and recovery contracts.

## Benefit-based prioritization

The order after the first baseline may change. Prefer demonstrated user value
and avoided work over completing every proposed abstraction. For each slice,
record: current problem, smallest change, observed benefit, correctness limits,
and one disposition: continue, accept, narrow, defer or reject.

An unchanged answer, simpler task completion, removed blocker or confirmed
measurement is progress. Follow AGENTS.md's 30-minute/100-call rule when no such
progress occurs. Do not keep extending infrastructure because a direction was
listed here. No universal performance percentage is an acceptance substitute.

## Verification and release cadence

- During iteration, use behavior-focused existing tests and necessary regression
  coverage. Reuse successful checks until a material change warrants repetition.
- Measure comparable inputs and source/model policies. Distinguish cold capture,
  retained reads, model time and end-to-end task time; bytes are not token counts.
- Before publication, complete relevant worker/contract checks, repository
  privacy checks and the full CI gate for the integrated revision.
- After independent acceptance of a meaningful increment, publish the next
  unused `0.13.*` release through the existing release process. Confirm release
  assets, supported architecture qualification, official installation and a
  focused installed-launcher exercise of changed behavior.
- Preserve existing language capabilities, including C# Roslyn read-only preview.
  Do not repeat unchanged qualification arms merely to update a report.

## Final documentation and repository cleanup

### Build-artifact housekeeping during the goal

The user additionally authorized periodic removal of unnecessary Codeclew build
artifacts. Check owned build-output sizes after substantial build/test batches
and before release work. Coordinate with the coding chat so an active build or
its current review inputs are not removed. Prefer obsolete, reproducible output
from completed versions/checkouts; retain the active incremental build while it
still saves useful work. Record reclaimed disk space briefly. Use supported
lifecycle commands for managed runtime/cache state, never edit private CAS or
CODECLEW_HOME objects directly. This housekeeping excludes source, user files,
required evidence, installed active releases and unrelated projects.

### Final tracked-content cleanup

Document new public behavior with the accepted installed release. Update the
conceptual architecture, task guides, examples, limits and navigation together.
Retain source-versus-narrative authority and measured-versus-proposed distinctions.

Inventory tracked old experiments, generated artifacts and version-specific
examples. Remove superseded material from the current repository tree when it
has no active test, supported workflow, current reproduction or necessary
historical-reference role. Update references and preserve useful old public URLs
with an accurate redirect or archive notice where appropriate.

Keep essential regression fixtures and current reproducibility evidence. Retain
the minimum decision/results summary needed to explain accepted or rejected
directions. Git history preserves removed tracked material; do not rewrite it.
Do not delete user work, private caches, research outside this repository or
published GitHub release assets as part of this cleanup. Review the removal diff
and validate remaining links, site output, tests and privacy before publication.

## Completion

The goal is complete when selected improvements have independent acceptance and
published releases, low-value directions have explicit dispositions, current
documentation reflects the delivered behavior, and the authorized cleanup has
been reviewed and published. A running CI, implementation-only verdict, or plan
document alone is not completion.

## Execution record

| Slice | Status | Result / next decision |
| --- | --- | --- |
| Baseline and first scoped read | Accepted and shipped in 0.13.15 | Independent acceptance covers `30c837d`: public retained `docs context --dependency` preserves the exact 75-item method result on the same saved snapshot; per-page object fetches 1,996 to 105 and fetched bytes 5,142,771 to 2,648,886. Source objects still read in full; selected validation is explicit. Full local and exact-release hosted CI passed; official installed verification preserves all 75 items and both pages. |
| Precise question-result reuse | Implementation accepted; release pending | Accepted `c15fecd`: exact request discovery, complete retained-history checks and native method preparation replay; separate from release 0.13.15. |
| Content-keyed syntax reuse | Implementation accepted; release pending | Independently accepted `1db703a`: same-service/path exact-byte extraction reuse with complete producer admission and current source receipts. Two-revision full evidence/Check equality and 21.863% measured incremental native benefit; 16 final syntax tests including nine cache regressions passed. Compiler reuse is outside this slice. |
| Role evidence representation | Compact native 1.1 independently accepted; release pending | Checked native presentation replaces only duplicate raw model-facing pages/source parts. Actual grouped author/reviewer delivery, canonical approval, both-version recovery and frozen 1.0 compatibility passed. The complete four-call generated Java fixture decreases paired prompt/schema reference counts 163,106 to 104,051 (36.2065%); expanded calls decrease 38.8299%. Initial author without duplication grows by 18 reference tokens. Initial semantic selection remains deferred. |
| Short opaque model IDs | Implementation independently accepted; release pending | Accepted `e729cd2`: explicit host-prepared native author/reviewer carrier, stable typed expansion maps, durable raw JSON/failure recovery and supported public serializer. Actual first-call prompts plus strict schemas decrease reference `o200k_base` text counts by 4.9184% including protocol. The earlier 10.26% remains a historical prospective upper bound, not delivered benefit. See the [assessment and actual measurement](../product/validation/model-id-alias-feasibility.md#supported-native-candidate-2026-10-06). |
| Incremental releases | 0.13.15 published and installed; 0.13.16 aborted; 0.13.17 final gates pending | The 0.13.15 local/hosted gates, 14 published assets and official installed retained-context equivalence remain recorded below. Integrated 0.13.16 local CI passed on `e2542efd33f193eb9e7f6e126b3f31f535fd66ce` in 4,173.985 seconds, including real C# qualification and a RELEASE usability smoke. The 0.13.16 tag remains immutable without assets after Linux test-helper lint failures. Version 0.13.17 carries the narrow fix; exact release-revision hosted gates, publication and official installed verification follow. |
| Documentation and cleanup | Cleanup accepted and published; final product documentation prepared for release | Cleanup `f570dba` (integrated as `f622083`) removes three generated historical reports and pins public methodology links. README, practical workflow, snapshot-store reference, mirrored portable guide and public task page describe the accepted candidates without changing historical example qualification. Current guides and installation pins target 0.13.17; historical source and renderer receipts retain their original versions. |

### First scoped-read candidate (2026-10-06)

The saved `clew-public-workflow` question drills into the syntax-captured
`Main.kt processRequest` declaration. The old service capture hydrates 928
observations, and the Check map hydrates 931; the useful method closure contains
34 observation payloads totaling 36,833 bytes. The public dependency drill now
validates the complete Check membership inventory and decodes only requested
observations and relevant same-symbol flow candidates, filtering their
compilation scopes before delivery. Sources, input declarations, and provenance
remain unchanged. Unread capture observation indexes, contracts, and unrelated
payloads are not integrity-audited; the response declares this limitation.

Instrumented public-dispatch reads on the same immutable snapshot returned all
75 items across the same two cursor pages, exactly preserving source text,
citations, boundaries, membership and existing authority fields. Equal page
boundaries are a fact for this question, not a universal promise: validation
metadata counts toward the existing stdout budget. The general contract preserves
the complete item sequence and snapshot/selector cursor binding. Each page
fetched 1,996 objects / 5,142,771 bytes before and 105 objects / 2,648,886 bytes
after. These are successful object fetches, including repeated fetches, not
physical disk IO. Same-process debug timings were 1,049.83 / 1,011.65 ms before
and 824.70 / 822.47 ms after. They are warm local observations, not an installed
release or end-to-end model speedup. The installed 0.13.14 baseline separately
completed its two launcher-inclusive pages in 275.28 / 266.69 ms; its release
profile and launcher differ from the debug measurement, so those times are not
compared against the candidate.

Disposition: accepted implementation at `30c837d`, release pending. The research
chat independently accepted the bounded behavior and measurements. Full CI and
official installed verification remain gates for the integrated release. Do not widen this
slice into Work, symbol/endpoint adjacency, source-fragment storage, question
reuse, model aliases or a general benchmark controller. Relevant payload and
source corruption, missing inputs, inventory consistency, service/scope
selection, and page budgets have regression coverage. No paid model calls or
unchanged compiler qualification arms ran.

Housekeeping completed by the research chat reclaimed 3,041,956 KiB (about
2.90 GiB) from obsolete build outputs in completed checkouts. The active coding
checkout's incremental `target` was preserved; source, CAS, evidence and the
installed release were unchanged.

### Model ID representation assessment (2026-10-06)

The [separate feasibility assessment](../product/validation/model-id-alias-feasibility.md)
records actual retained prompt and strict-schema sizes, reference-token counts,
protocol overhead and the canonical-versus-model representation seam. Defer
implementation from the immediate scoped-read release; retain aliases as a
promising follow-up rather than rejecting them on byte savings or old-driver
refusal. The latter only rejects changing the canonical packet before validation.
No alias behavior is included in `30c837d` or installed `0.13.14`. No paid model
calls, repeated scoped-read measurements or full CI ran for this assessment.

### Release qualification and housekeeping (2026-10-06)

Release `0.13.15` was published on 2026-10-06 at 07:12 UTC. Full
`scripts/ci-verify.sh` passed on
`b3eb735`, including the actual C# fixture, required Java/Kotlin checks and
RELEASE-mode usability smoke. No functional correction was needed. The final
metadata-only adjustment links the site installer to the latest published
release, avoiding advertising an unpublished version during platform builds.
Hosted [CI](https://github.com/codeclew/codeclew/actions/runs/37423184253)
and [release qualification](https://github.com/codeclew/codeclew/actions/runs/37423190012)
passed on the exact tagged commit
`bad1e0ca8bd804f2bce61a8a7ec93ed9a9bcc1ba`. The
[published release](https://github.com/codeclew/codeclew/releases/tag/v0.13.15)
contains all 14 expected assets: six platform/profile archives, their checksums,
and the installer with its checksum. Published checksum files match GitHub asset
digests. The official installer downloaded and verified the macOS arm64 core
archive; the other five archives were qualified by the release workflow, not
independently downloaded in the installed-launcher check.

The official site installer activated `clew 0.13.15` in RELEASE mode. Its installed
manifest binds the exact tagged source revision and tree
`04a1b8ee553dc8d4931df503d12b5a2e2f64970a`. On the original saved public question,
the installed launcher returned the same 75 canonical items in two pages (40 and
35), preserving existing metadata, source citations, scope, boundaries and cursor
scope. The additional selected-validation metadata remains explicit. This was a
correctness check, not a comparative performance benchmark; no capture, refresh
or model call was needed. Existing C# Roslyn capability remains present.

During this gate, eight obsolete pre-run clew library/metadata/test/CLI outputs
were removed, reclaiming about 1.39 GiB while preserving the active target and
current outputs. The research chat separately verified and removed an ignored,
inactive `0.13.1` checkout's debug output, reclaiming 27,800,948,736 bytes
(about 25.9 GiB). Current CI, source, private CAS, evidence and installed releases
were preserved. This is build housekeeping, not the final tracked-content cleanup.

### Exact approved-answer discovery candidate (2026-10-06)

The source candidate adds `docs work find-answer` for an exact normalized request,
subject and explicitly selected Check. It searches every retained approved run,
preserves historical answer/review/provenance, requires explicit selection among
multiple applicable approvals and explains stale or unsupported context. Missing
report/checkpoint-history bindings or a missing Work cannot silently become a
complete empty or unique search. No database, source acquisition, model call or
publication path was added.

The first supported boundary is native method `process-graph-v1`, authoring 1.6,
source-data context, no actual role expansions/repair/maintained or external
context, and absent protected notes. Initial preparation replay compares stable
semantic membership, graph/frontier/negative results, derived source-data and
producer/selection policy. Notes get a separate read-only membership probe.
Portable expectations and `EVIDENCE_PACKAGE` authority explicitly remain
unsupported. Complete verified replay can accept valid source/compiler receipt
changes across revisions; existing `answer-context` keeps its strict policy.
Changed delivered source content and newly matching callees remain invalidating.

Seventeen focused tests passed, including genuine deterministic local approvals,
historical ambiguity, missing nonlatest/latest reports, missing Work with both
matching and nonmatching requests, corruption, new notes, changed guard/callee,
new-revision receipt equivalence through the production comparison wrapper,
producer-policy refusal and actual expansion refusal. These are synthetic host
correctness fixtures, not new compiler qualification or model-quality evidence.

One actual retained `prepare-null-boundaries` lookup found its original 1.6
zero-expansion approval. Answer, meaning review, provenance, packet and audit
equaled the prior retained export, and all 351 documentation-root files remained
unchanged across the fresh CLI process. It took 4.228 seconds on the warm
unoptimized source CLI, including cargo launch overhead, with zero captures and
model invocations. The historical author/reviewer pair's roughly 148 seconds and
45,505 successful-role CLI input tokens are context for this saved result only,
not a general latency, billing or pricing comparison. No paid pilot was rerun.

Zero writes describes durable Work/job/review/publication/source mutations;
existing CAS/locking infrastructure may perform operational filesystem IO.
The research chat independently accepted `c15fecd937c6795d378a18fe97aefd50e193aeec`
in this scope after reviewing the final corrections, test log and retained
exercise. The next release and agreed integrated final gate remain pending.
This implementation is not included in the frozen 0.13.15 tag, and no new full CI
gate was run for it. Next, measure the actual syntax path before deciding whether
per-file extraction reuse merits implementation; short model IDs stay queued.


### Syntax extraction cost and bounded decision (2026-10-06)

A temporary test-only harness measured actual committed-source extraction at
public source revision `fd82a763fb5b38083fc3f298aee5ee116c906e21` (`v0.13.7`).
Each corpus used one warmup and five measured warm samples, an optimized native
build, a fresh isolated CAS and direct capture bypassing whole-capture reuse.
The final observation-to-dependency Check assembly was included with service-only
inputs; no interactions, scenarios or semantic compiler provider were configured.
Temporary instrumentation was removed after preserving the private raw results.

| Median phase (ms) | Public workflow scope | Kotlin worker source scope |
| --- | ---: | ---: |
| Git acquisition and CAS write | 36.285 | 35.801 |
| CAS read | 3.230 | 2.491 |
| Parse | 2.476 | 50.031 |
| Root syntax fingerprint | 1.434 | 30.008 |
| Extraction including nested fingerprints | 26.301 | 818.269 |
| Nested fingerprints, subset of extraction | 8.398 | 200.922 |
| File-only extraction | 6.287 | 0.110 |
| Finalize and verify evidence | 9.671 | 131.564 |
| Check assembly | 1.481 | 35.023 |
| Total capture and Check assembly | 87.431 | 1,104.181 |

The public workflow scope contains ten files (1,130,647 bytes), with one supported
Kotlin file (15,266 bytes), 928 observations and 936 sources. The worker scope is
`workers/kotlin/src/main`: 24 files (483,650 bytes), including 21 Kotlin files
(468,583 bytes), 16,954 observations and 16,956 sources. Evidence identities and
counts were deterministic across all samples. Worker coverage is `PARTIAL`:
`FirFactsPlugin.kt` retains a parse-error boundary, which must remain uncached.

The single supported file has low absolute extraction cost. The worker scope
spends about 898 ms in parsing, root fingerprinting and extraction; this supports
one bounded same-service, same-path, exact-byte reuse candidate across revisions.
It is an eligible-phase upper bound, not a measured cache gain. Actual cache IO,
current occurrence reconstruction, uncached parse-error work, scope acquisition,
verification and Check assembly still contribute to total time. Accept only if
the candidate preserves complete current evidence and delivers useful measured
net benefit against forced full extraction on the same build and revision.

This profile excludes initial Git resolution, CAS opening, compiler capture,
whole CLI startup, cold IO and model time. Nested fingerprint timings overlap
extraction and must not be added again. Service-only Check assembly is a lower
bound for richer documentation. No general documentation or model speedup follows
from these measurements. No paid call or repeated compiler qualification ran.


### Accepted tracked-artifact cleanup (2026-10-06)

The independently accepted cleanup removes exactly three generated historical
files (435,397 bytes): the multi-service plan PDF, the Kotlin evidence-study PDF,
and the marketing sample JSON. Their bytes were verified against archived public
revision `f0b874f5f8e0d6e5b96a2f5f65592c7c4e07cb0a` before removal. Public methodology
links now use that immutable revision; affected PDF and sample URLs return HTTP
200. The benchmark README preserves the sample's date, source revision, five
concerns, 16 successful checks, 784 ms warm time, 160 lexical matches, one retained
release receipt and the fresh `RESOURCE_LIMIT` attempt that produced no facts.

The source PDF generator, planning Markdown and DOT, public evidence JSON and all
Q1 material remain. Site checks passed (16 tests), as did English, JSON syntax,
diff and privacy checks. The documentation-only cleanup was published as
`f570dba` and integrated as `f622083`; it required no runtime build or repeated
release qualification. This closes the agreed artifact-removal scope. Final
product documentation for subsequent accepted improvements remains unfinished.


### Measured syntax-file reuse candidate (2026-10-06)

The candidate reuses successful syntax extraction for the same service and path
only when exact bytes, language/dialect and the complete bundled producer match.
Source occurrences are rebuilt with the current revision, snapshot, blob, ranges
and URL. Scope acquisition and membership, file-only handling, annotations and
aggregate source/fact budgets remain active. Parse-error trees and failed
extractions are not cached; corrupted cache bindings fail explicitly. Whole
capture admission also includes the producer identity, preventing older keyed
captures from bypassing this policy. No compiler-result reuse is added.

A private isolated Git fixture copied the real 24-file worker scope from public
`v0.13.7`. Revision A was `4ac63dfc987f129e7a1d4e70baffe16ce854dd46`; revision B,
`64b988cd8bc12298357261a285b4e3d978831513`, adds one 52-byte marker declaration to
`CompilerCfgLabels.kt`. One optimized build measured one warmup and five paired
B captures, alternating cached/full order. Each pair used a fresh documentation
root seeded from A outside timing, so every cached B capture had 19 hits and two
parses: the changed file and the persistent `FirFactsPlugin.kt` parse error.
Forced full extraction parsed all 21 supported files at the same B revision.

| Median measured phase (ms) | Cached B | Forced full B |
| --- | ---: | ---: |
| Capture, including cache IO, replay and changed-file writes | 834.439 | 1,071.716 |
| Check assembly and source-input binding | 29.894 | 35.151 |
| Total | 865.107 | 1,107.166 |

The measured total reduction is 242.058 ms (21.863%; 1.280x). Cache envelope/CAS
IO and deserialization take 114.236 ms; validation and current-source replay take
531.945 ms. These are subsets of capture time and must not be added again.
All five pairs preserve the complete `ServiceEvidence` and full serialized Check,
including current sources and all boundaries; identity digests remain stable.
The result retains `PARTIAL` coverage, 16,955 observations, 16,957 sources and 339
entrypoints. This is observed incremental native benefit, not a general CLI,
compiler, cold IO, model-time or documentation-task speedup.

Initial A cache seeding took about 2.27 seconds including documentation-root
initialization, versus about 1.1 seconds for the measured full B capture. Those
are different operations, so this reports startup cost rather than a paired cold
benchmark. Git fixture setup, revision validation, CAS opening, producer
initialization, A seeding, equality assertions and report writes are excluded
from measured B totals. Actual service-only Check assembly is included; richer
interactions/scenarios or compiler-provider merges are not configured.

Temporary profiling hooks were removed. After measurement, producer admission
also gained the pinned Rust toolchain input, whose Unicode classification can
affect declaration identity. The measured and final builds use the same pin;
this changes admission keys without changing the measured extraction/replay
algorithm. Final native regression checks qualify the corrected producer binding.
The research root independently accepted final commit `1db703a` after reviewing
the complete paired measurements, producer closure and final checks: 16 syntax
tests (including nine cache regressions), formatting, diff, English and privacy.
The nine focused cache tests also passed after the final selector correction and
repository formatting. Integrated release qualification remains pending; this
behavior is not part of released `0.13.15`.


### Supported model ID representation candidate (2026-10-06)

The source candidate now provides an explicit `codeclew-model-ids/1.0` role
opt-in for native method authoring 1.6 and independent review. It retains the
unchanged canonical request plus an immutable prepared model payload, strict
schema and typed Work/run/role map before dispatch. Compatible drivers validate
canonical input first and use the public pure serializer API/example to forward
only the model payload and schema. Legacy role bytes and driver identity remain
unchanged when the mode is absent.

Grouped expansion preserves old aliases; reviewer scope remains independent.
The host retains parsed outer JSON and adapter failure before fallible Reply or
identity decoding. Unknown or foreign aliases fail closed, and recovery after
map-head publication or result-save interruption never repeats an already
received dispatch. Saved successful answers/reviews retain canonical identity.
Malformed JSON remains an adapter failure rather than a byte-level output record.

The [actual two-job measurement](../product/validation/model-id-alias-feasibility.md#supported-native-candidate-2026-10-06)
reduces combined prompt and strict-schema reference tokens from 26,716 to 25,402
(4.9184%, including protocol overhead), versus the earlier prospective 10.26%
upper bound. Canonical input and protected source/semantic/evidence/query leaves
are exact. Whole carriers are 80,371/118,031 bytes with 2,759/3,908-byte host-only
maps; host admission/reservation still charges these complete carriers although
they are excluded from model input. This is reference `o200k_base` text counting,
not provider usage, billing, quality or a grouped-expansion savings claim.

All six codec tests and four native host scenarios passed across the initial
focused run and one corrected grouped-review fixture rerun. Two legacy tests
passed separately, and the public serializer example compiled and returned the
exact prepared payload/schema for both retained jobs. No provider traffic, paid
model call or broad qualification run was needed. Full CI and the next release
remain pending; released `0.13.15` does not contain this candidate.

The research root independently accepted implementation and measured bounded
benefit at final commit `e729cd2`. The final formatting, English, privacy and diff
checks passed before commit; their private logs retain the
`codeclew-model-id-final-{fmt,english,privacy,diff}-20261006` names. The public
serializer example was compiled and exercised on both actual retained carriers.
The private `native-candidate/validation-summary.json` distinguishes the initial
nine passes from the independently corrected grouped-review rerun, and retains
the two separate legacy compatibility logs. No unchanged successful tests were
repeated to create a new acceptance layer. Proceed with the agreed final product
documentation and one coherent full-CI/release gate.


### Actual grouped role delivery and selection disposition (2026-10-06)

The existing deterministic native grouped fixture was run once more solely to
retain its previously unavailable actual dispatched inputs. Optional test-only
artifact output saved both initial and expanded author/reviewer carriers and
`forward_model_input` results; no production behavior, role cap or model pilot
was changed. The same focused test passed with four actual loopback-observed
driver dispatches and a canonical approved saved answer/review. The public
serializer example independently returned the exact expanded payload/schema
for both roles. No remote provider or paid model call was made.

| Actual delivered model form | Payload bytes | Native schema bytes | Complete carrier bytes | Registered pages / source parts |
| --- | ---: | ---: | ---: | --- |
| Author initial | 23,117 | 5,250 | 52,592 | No additional grouped delivery |
| Author expanded | 154,471 | 5,250 | 339,235 | 13 / 6 |
| Reviewer initial | 167,398 | 3,782 | 365,091 | Author packet retains 13 / 6; independent reviewer delivery 0 / 0 |
| Reviewer expanded | 296,910 | 3,781 | 640,512 | Author packet retains 13 / 6; independent reviewer delivery 13 / 6 |

All pages/source parts retain their matching receipt counts. Expanded author
and reviewer representations preserve all 249 and 459 checked source, semantic,
evidence and selection leaf values, respectively. Maps preserve the exact
initial prefix: the author map grows from one to 55 identities; the reviewer
map stays at 62 identities while independently requested evidence is delivered.
Complete instructions and schemas remain in model input; raw native pages and
receipts remain in the retained host carrier. No evidence was dropped to obtain
these sizes. This generated Java fixture establishes actual native delivery
shape and recovery/identity correctness, not a production workload, provider
prompt observation or grouped token, latency, billing or quality saving.

Defer task-specific initial selection and broader role-selection contract
unification. Evaluate the observed raw/presentation duplicate delivery separately. The accepted five-question comparison had
no expansions and establishes no net benefit or quality preservation for those
changes. The expanded reviewer carries both historical author delivery and its
independent review delivery; those separate authorities cannot simply be removed
because their sizes overlap. Within each role, however, raw delivery and
presentation contain exact model-facing source-part duplicates. A narrow review
is evaluating whether those raw copies can be excluded solely from the model
projection while keeping canonical archives, presentation, schema and citations.
No new arbitrary context/call caps, paid pilot or protocol rewrite is justified.
The initial inspection is recorded below as its own evidence stage. The
subsequent compact candidate closes that specific duplication deliverable.
Initial semantic selection and broader role-contract unification remain deferred.


### Compact role evidence representation candidate (2026-10-06)

Representation `codeclew-model-ids/1.1` closes the observed duplicate-delivery
problem through the existing opt-in driver path. Version 1.0 and public `prepare`
retain their original behavior. New explicit `prepare_with_version` and saved
version selection preserve frozen 1.0 carriers rather than reinterpreting them.
Carrier, map, scope tag, map head, selected role mode and saved result binding
agree on the selected version; raw-result record format remains unchanged.

For 1.1, the pure host codec checks each complete canonical native delivery's
presentation against existing `job_context::present(raw_pages, raw_source_parts)`
before projecting identities. After that proof and the existing typed alias walk,
only raw `pages` and `sourceParts` siblings are removed from the model form in
`packet.contextDelivery` and `reviewContext`. Complete presentation, callable
metadata, retained-reference links, receipts, citations and delivery bindings
remain. A missing presentation retains raw data; incomplete/changed presentation
fails. Canonical jobs, input archives, saved answers and reviewer authority remain
unchanged. There is no cross-role evidence sharing or initial-selection change.

The native builder preserves duplicate coverage rows through `retainedAt` and
can project source/text/token/event references. Literal raw-page equality would
therefore reject valid actual deliveries. Reusing the checked existing builder
proves the complete representation without introducing a new reconstruction
engine or weakening receipt validators. Opted-in fixture consumers read
presentation; canonical and version 1.0 consumers retain their previous paths.

The same existing grouped fixture produced actual 1.1 author/reviewer calls,
complete 13-page/six-source-part delivery per role and a canonical approved
saved answer/review. For comparison, those exact canonical jobs and Work/run/role
scopes were prepared through both versioned APIs with stable first-call map
prefixes and passed through the public serializer. The 1.1 projections equal
the actual retained dispatch records exactly. The 1.0 paired view is prepared
from those same jobs; it is not claimed to be a second dispatch of the same
immutable invocation. Independent 1.0 grouped/recovery tests also passed.

The frozen prompt formatter and strict-provider schema projection were identical
on both sides. Counts include complete protocol guidance and schema, with maps
excluded from model input. Reference encoding remains `o200k_base` from
`tiktoken` 0.14.0, excluding chat framing and provider schema wrappers.

| Actual input stage | 1.0 prompt bytes / reference tokens | 1.1 prompt bytes / reference tokens | Strict schema bytes (both) | 1.0 / 1.1 strict-schema reference tokens | Combined reference change |
| --- | ---: | ---: | ---: | ---: | ---: |
| Author initial | 23,608 / 5,483 | 23,783 / 5,505 | 5,688 | 1,583 / 1,579 | 7,066 to 7,084 (+18) |
| Author expanded | 154,962 / 37,691 | 95,562 / 23,239 | 5,688 | 1,583 / 1,579 | 39,274 to 24,818 (-36.8081%) |
| Reviewer initial | 167,891 / 41,154 | 108,464 / 26,284 | 3,711 | 1,172 / 1,124 | 42,326 to 27,408 (-35.2455%) |
| Reviewer expanded | 297,403 / 73,268 | 178,430 / 43,617 | 3,710 | 1,172 / 1,124 | 74,440 to 44,741 (-39.8966%) |

The complete four-call fixture context decreases from **163,106 to 104,051
reference text tokens (36.2065%)**. This accounts for both initial and expanded
author and reviewer inputs. The two expanded calls alone decrease from
113,714 to 69,559 (38.8299%). The author initial call has no duplicate delivery
to remove and grows by 18 reference tokens (0.2547%) from guidance and version-scoped alias differences; this overhead is
reported rather than omitted. Native model input pairs decrease from 159,749 to
100,349 bytes for expanded author and from 300,719 to 181,746 for expanded
reviewer. Complete carriers decrease from 339,235 to 279,835 and 640,512 to
521,539 bytes, respectively. Their 7,743/8,627-byte maps stay outside model
input while conservative host admission/reservation still accounts for the
complete carrier and configured overhead.

This generated Java fixture establishes complete native delivery and an observed
reference-text reduction, not general workload performance, provider usage,
billing or model quality. No paid or remote-provider calls, new context/call
caps or qualification pilot were added. The previous 4.9184% two-real-first-call
measurement belongs to the accepted 1.0 ID projection; the earlier prospective
10.26% remains historical. Neither is substituted for this expanded-delivery
comparison.

The final focused gate passed **16 tests: 11 codec and five host test names**,
including parameterized unknown-alias and head/raw crash scenarios for both
versions. Six genuine pre-change 1.0 carriers passed the new public serializer;
the two original real-work outputs remain byte-identical. Complete pre-change
recovery-record chains were not retained, so no historical full-chain replay is
claimed. Existing 1.0 recovery tests and production read-only record validators
qualify current recovery; newly captured chain artifacts are labeled current
compatibility captures. Temporary measurement examples were removed. Integrated
full local CI subsequently passed on `e2542ef`; publication and official installed
verification proceed with 0.13.17 after the aborted 0.13.16 attempt recorded below.

The research root independently accepted the 1.1 implementation, focused gate,
frozen-carrier compatibility and paired model-facing benefit after material
delta review. No further efficiency scope is scheduled before publication.
Task-specific initial semantic selection, cross-role sharing and broader role
contract unification remain explicitly deferred. Finish the current product
documentation and the final 0.13.17 hosted qualification and installed checks.


### Integrated 0.13.16 local qualification (2026-10-06)

One coherent `scripts/ci-verify.sh` run passed on clean source commit
`e2542efd33f193eb9e7f6e126b3f31f535fd66ce`, tree
`02dabad0c5c504556a137ebba79d975c77496b63`, in 4,173.985 seconds.
This covers the real SDK 10 C# fixture, 616 documentation library tests
(six separate scenarios ignored there), native Java/Kotlin contracts, retained
source and publication/recovery scenarios, CLI checks, privacy checks and a
`runtimeMode: RELEASE` usability smoke. It is source/test-build qualification,
not an official installed 0.13.16 receipt.

The earlier coherent attempt stopped at eight Clippy style findings on
`17649cf`. Commit `e2542ef` fixes only borrows, conditional formatting and test
helper placement. Failed-attempt logs remain retained privately. Accepted
behavior and measurements were not changed or rerun as a new benchmark.

After that successful run, the prepared README, operations, three identical
portable-guide copies and site wording are promoted from upcoming/source
candidates to the included 0.13.16 behavior. Legacy anchors and the original
0.13.11 source/authoring and separate 0.13.14 renderer receipts remain intact.
This follow-up changes documentation only; relevant documentation, site and
skill-package checks apply before the final release commit. The final exact
release revision still requires hosted gates, published assets and official
installed checks of exact lookup, current syntax receipts and both 1.1 roles.
The installed role smoke uses local deterministic fixtures without provider
calls, expansion, publication or a review-quality claim. Expanded compaction
and frozen-1.0 compatibility remain the accepted source evidence above.


### Aborted 0.13.16 tag and 0.13.17 continuation (2026-10-06)

The immutable `v0.13.16` tag points at merge commit
`d69ed3b3273422b2c9239284865b4c412c41adc8`, whose tree exactly matches the
qualified feature branch after documentation promotion. Its RELEASE mutation
pilot passed 6/6 (Rust 3/3, Python 3/3). The hosted
[Linux CI job](https://github.com/codeclew/codeclew/actions/runs/37447748129/job/112216555094)
then failed at Clippy because 15 test helpers were compiled on Linux although
only macOS tests call them. The
[release attempt](https://github.com/codeclew/codeclew/actions/runs/37447759517)
was cancelled before assets were published. The tag is preserved without
release assets; it must not be used as an installation target.

Commit `96b360ac610eedc3e78ed8fbff60dcc28ddfc2a5` changes exactly 15 test-only
`cfg(test)` guards to `cfg(all(test, target_os = "macos"))`. It changes no
function body or production algorithm and adds no warning suppression.
The original local full-CI receipt above remains bound to its original source.
Current guides, installation pins and release notes move to 0.13.17, retaining
the 0.13.16 public section anchor for existing links. Focused source and
documentation checks, a RELEASE mutation pilot, exact-head Linux/macOS hosted
gates and platform release qualification are required before publication.
Official installed lookup, current syntax receipts and independent 1.1 role
smokes then verify the published package. These local deterministic role
fixtures make no paid provider calls or review-quality claim.


### Kotlin model configuration cache and safe failure guidance (2026-10-06)

Before tagging 0.13.17, a bounded paired synthetic Gradle reproduction established
that native `help` succeeds with configuration cache enabled, the injected model
task fails under cache restrictions, and the same model request succeeds with
`--no-configuration-cache`. The production model command now supplies that flag
without changing the caller's project properties or ordinary build preference.
The native project-dependency extraction regression uses configuration cache
enabled and the production command builder; it verifies metadata extraction
without claiming a target compilation. The command-plan test and shared worker
compilation for Kotlin 2.1/2.3 passed.

The Rust host also retains eight existing `BUILD_*` categories through a fixed
static guidance allowlist for `UNSUPPORTED_PROJECT_CONFIGURATION`. Unknown or
malformed prefixes and other error codes keep their generic guidance. Three
focused tests cover all eight categories, private-shaped suffixes, malformed
inputs and wrong error codes; formatting passed. Raw worker text is never
forwarded as guidance. These bounded changes were independently reviewed and
accepted before integration. They do not establish the cause or resolution of
a separate reported project/version combination or universal composite-build
support.

The earlier 0.13.17 candidate `dd28cf7d288d15c7703dfd4847d28b193ae0d643`
retains its own hosted CI binding and must not qualify this new production
delta. The final exact release head requires fresh hosted Linux/macOS gates,
platform qualification and official installed checks. A fourth installed
regression will capture a small Kotlin compiler project with configuration
cache enabled and verify ordinary HTML rendering from the saved Check. Native
linked MDX/HTML projection remains Java-only; this regression must not claim
Kotlin native MDX support.
