# Java and Kotlin token-value experiment

Status: in progress. Baseline: Codeclew 0.7.0 (`b64f1aa`).

## Accepted outcome

Measure actual agent token use on practical Java and Kotlin analysis and change
tasks, add a bounded Java mutation path using existing candidate/validation/
publication machinery, and compare the resulting task outcomes. Kotlin remains
an equal part of the evaluation. Independent packs and new language engines
are outside this work.

## Sequence

- [x] Freeze eight tasks (four per language), source revisions and acceptance
  checks before measured execution; separate development and held-out cases.
- [x] Verify the installed 0.7.0 launcher and run Default versus Default plus
  Codeclew in fresh agent sessions with identical model and reasoning settings.
- [ ] Implement bounded Java mutation and any narrowly measured common
  usability correction; preserve current Kotlin guarantees.
- [ ] Run meaningful rejection, validation and publication regressions, then the
  complete code CI gate.
- [ ] Repeat affected measured tasks with the candidate, evaluate held-out tasks,
  and report benefits, failures and limits without claiming unmeasured savings.

## Measurement contract

Use two analysis and two change tasks per language. Prefer existing real
repositories; label fixtures and injected defects separately. Freeze acceptance
facts or behavioral tests before execution and keep them outside agent roots.
A fresh Default agent has native discovery/edit/test tools. A fresh treatment
agent additionally has Codeclew and its matching skill; product failures and
any explicitly allowed native continuation count toward the whole task. Failed
managed admission is never represented as successful managed evidence or writes.
Do not force Default to read whole files or force unnecessary Codeclew commands.

Record actual model input, cached input, output and total tokens, wall time,
failed tools, accepted outcomes, critical omissions and patch validation. Keep
cold admission/indexing and warm reuse distinct. Include retries and unsuccessful
attempts; do not infer model tokens from output bytes. Target at least a threefold
reduction in median tokens per accepted task without reduced success or quality.
This is a small engineering comparison, not a population-level benchmark claim.
Raw private prompts, source, outputs and oracles stay outside the public repo.

## Implementation boundary

Reuse immutable source bindings, exact file edits, isolated candidates, native
build validation and compare-and-swap publication. Java admission must bind the
actual supported JDK/build profile; no blanket mutation flag for read-only
profiles. Kotlin compatibility analysis does not acquire mutation authority.
Keep the model-facing path compact and avoid new protocol discovery or repeated
full-state output. No target production repository is modified during trials.

## Development observations (2026-09-09)

The first two paired analysis tasks used more tokens with installed 0.7.0:
Java 387,254 versus 261,565; Kotlin 442,927 versus 175,624. These are actual
cumulative input plus output tokens, including cached input, not output-byte
estimates. They are individual engineering observations, not aggregate results.

Java test-compilation analysis failed because inherited annotation facts of
83–85 KiB exceeded the 64 KiB transport limit. Kotlin found the exact function
but decision selection ignored its compiler callable name and returned
NO_EXACT_IDENTIFIER. Native continuation completed the tasks; managed success
must therefore be reported separately from overall acceptance.

A skill-only development trial retained the 0.7.0 runtime, moved detailed
workflow instructions behind scoped references, and reduced the main skill
from 4,548 to 1,042 words. Its Kotlin task used 404,342 tokens: about 9% below
the previous treatment, still above Default. This does not establish the goal.

The next bounded candidate fixes compiler-name selection, preserves ambiguous
overload rejection, and preserves the existing per-fact limits. The measured Java transport fix is
kept outside the Kotlin candidate for later work. Java mutation remains
conditional on demonstrated value; adding a write profile alone would not
resolve the observed analysis and instruction costs. No release is published
as part of this experiment. Raw runs and behavioral checks remain private.

## Current priority

The user selected token savings on Kotlin first, with Java work afterwards.
Four initial Kotlin cases are now diagnostic, not held-out validation. All four
returned an unavailable decision source in installed 0.7.0. Cumulative tokens
were 1,128,919 for Default and 2,792,501 for Default plus Codeclew (2.47x).
Cached input accounts for 97% of the increase; uncached input plus output rose
from 133,079 to 183,605. Per-command token usage is unavailable, so command
counts and output sizes must not be presented as exact token attribution.

Before further measured comparison, both arms need the repository-required
Git identity and private per-run log paths; sequential JVM compilation avoids
shared compiler-daemon heap contention. These setup corrections must also be
applied to a new Default run. The next acceptance target is useful source and
computation-chain evidence replacing native discovery, rather than an
additional successful admission response. Kotlin name selection is fixed and
regression-tested; initial exact-name lookup and query-coverage coupling remain
under investigation. Java mutation is deferred.

The first source candidate also retains complete files up to 8 KiB for initial
source navigation, reusing the existing reference-follow source policy. Larger
files retain bounded declaration windows and the shared source budget. A
regression verifies that a helper beyond the declaration window is delivered
for a small file without widening the large-file policy. This candidate is
a local engineering probe; it does not yet fix broad-query truncation.
