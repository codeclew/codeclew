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
| Baseline and first scoped read | Accepted implementation; release pending | Independent acceptance covers `30c837d`: public retained `docs context --dependency` preserves the exact 75-item method result on the same saved snapshot; per-page object fetches 1,996 to 105 and fetched bytes 5,142,771 to 2,648,886. Source objects still read in full; selected validation is explicit. Full CI and official installed verification remain release gates. |
| Precise question-result reuse | Pending | Build on existing answer-context after first cost/result findings. |
| Content-keyed syntax reuse | Pending | Proceed after a bounded baseline; compiler reuse conditional. |
| Role evidence representation | Pending | Inspect an actual expansion prompt before claiming token savings. |
| Short opaque model IDs | Implementation deferred; promising measured opportunity | Two retained role prompts plus strict schemas show a 10.26% combined reduction in reference `o200k_base` text tokens for a prospective representation. This is not target-model usage or a qualified codec. Host-prepared projection can preserve canonical driver validation without a duplex protocol, but durable representation identity, expansion stability, typed decoding and supported driver forwarding need a separate implementation slice. See the [bounded assessment](../product/validation/model-id-alias-feasibility.md). |
| Incremental releases | Pending | Next version follows the current release registry. |
| Documentation and cleanup | Pending | Final inventory and published removal diff required. |

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
