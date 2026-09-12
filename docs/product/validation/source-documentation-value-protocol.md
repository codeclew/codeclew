# Source documentation: value and availability evaluation

Date: 2026-09-12. Status: frozen engineering comparison in progress.
Product baseline: published Codeclew 0.7.1, source revision
`4430b5af1e82b0bf89b7eca988517c1802108b0c`.

## Outcome

Evaluate the cost and quality of obtaining enough evidence to write useful
documentation. Independently qualify documentation availability for Python,
Java without a working build, and Kotlin 1.9 without K2. The final report must
list available documentation operations, tested behavior, examples, and limits.
Token reduction and documentation availability are different success criteria.

The implementation order follows the [syntax-first plan](../../plans/codeclew-syntax-first-documentation-plan.md)
and the [token-efficiency plan](../../plans/codeclew-token-efficiency-plan.md).
The first stage informs scope; it does not authorize a planner without evidence.

## Existing evidence retained

Two earlier self-hosting source-context comparisons are relevant diagnostics:

- At `56d1211`, the three-criteria calibration found that manually preloaded
  original source could support accepted answers at lower token cost. Source
  selection was manual, two questions had already been consumed, and a
  packet-first answer on the longer question omitted an important fact.
- At `55260fc`, two new questions using automatic initial context accepted all
  four answers, but cumulative input plus output rose from 277,356 to 344,945
  tokens (24.4%). This candidate remained optional and was not in 0.7.1.

These observations are historical evidence, not fresh measurements for this
task. They argue against promoting broad call-adjacency expansion as a token
optimization. They do not establish that documentation without a compiler is
unhelpful. The retained records are `docs/plans/token-value-three-criteria.md`
at `56d1211` and `docs/plans/automatic-source-context-results.md` at `55260fc`.
Local branches preserve the original records; their availability on GitHub is
not assumed.

## Frozen initial tasks

The comparison uses the repository's synthetic Java orders/inventory fixtures
and Kotlin 1.9.25 warehouse-import fixture. A second, unannotated `receive`
declaration is added before freezing the Kotlin fixture to exercise ambiguous
name lookup. Every source repository is committed before preparation. This is
a small documentation engineering corpus, not an unseen population benchmark.

| Task | Stratum | Required documentation facts |
|---|---|---|
| E1 | Exact lexical lookup | Kafka topic/key, listener topic/group, delivery boundary |
| E2 | Single-repository behavior | HTTP route, negative/zero/positive quantity paths, return messages, processing boundary |
| E3 | Ambiguous symbol | Both `receive` owners/types/bodies, listener annotation, ambiguity and runtime limits |
| E4 | Cross-file path | Checkout guard, client request construction, configuration, response propagation, network limits |
| E5 | Cross-repository declared path | Quantity 0/100/101 outcomes, declared HTTP handoff, memory state change, deployment/delivery/persistence limits |

Acceptance statements and task obligations are frozen before model runs. Models
receive the same question and obligation IDs. They never receive the private
acceptance answers. Parent source review scores each obligation and critical
omission; grading is not blinded or independent.

## Arms and accounting

All arms use `gpt-6-astra`, high reasoning, fresh conversations, the same source
revisions, a 300-second deadline, and native source-reading tools. They batch
independent reads, avoid rereading supplied evidence, and do not edit source or
run tests/builds. The first pass has one repetition per task. Arm order rotates
between native/current/oracle and oracle/current/native across tasks.

- **Native:** `rg` and bounded source reads, including engineer declarations.
- **Current:** installed RELEASE 0.7.1 documentation context first; native
  continuation is allowed for gaps. Services are already bound and checked.
  Matching documentation instructions count as model input.
- **Oracle:** manually selected original source files with line numbers in the
  initial prompt, without answer text. Native continuation is still available.

Retain actual cumulative model input, cached input, output, elapsed time,
command count, failed commands, reported tool-output bytes and per-round usage
when available. Cached input is a subset of input. Total logical tokens equal
input plus output; do not add cached or reasoning subsets again. Missing usage
is unavailable, never zero. Token costs are not inferred from file/output bytes.
Reconcile per-round accounting with the client's completed-turn totals before
using it for conclusions. Do not call command count a model-round count.

Release installation and compiler/source preparation have separate elapsed-time
records and zero preparation model calls. All initial prompt tokens count in
model input. The current arm is a warm retained-context measurement; cold
preparation cannot be described as free or omitted from lifecycle discussion.
Failed calls, fallback and transport problems remain in the records. An arm is
not rerun to replace an unfavorable accepted or failed answer.

Zero preparation calls describes the deterministic assembly/capture scripts.
Experiment design and manual oracle source selection were performed by the
parent assistant outside measured arms; that effort is not separately metered.
An oracle ratio therefore measures downstream feasibility, not the total cost
of an automatic source-selection product or the cost of this research task.

One preflight attempt with Codex CLI 0.147.0 was rejected before any model answer
or tool call because that client did not support the selected model. Its record
is retained with unavailable token usage. The corrected protocol pins the
installed app's Codex CLI 0.153.4 for all measured arms and preserves the original
protocol and failure. This is an infrastructure correction before measurement,
not an excluded unfavorable answer.

## Decision boundary

Report per-task quality together with cost. A smaller answer that misses a
required fact is not a successful token optimization. One repetition of five
related fixture questions cannot establish a population-level 30% saving or
qualify a planner. An oracle win is a feasibility signal for a task class, not
an automatic product result. M4-M6 remain deferred until repeatable independent
evidence justifies them. M3 should first reduce avoidable context delivery while
preserving source and material boundaries.

## Documentation qualification after the first stage

Use the same shared documentation model, retained source and publication path.
Require useful declarations, lexical calls and control structure for valid
Python, Java and Kotlin 1.9 fixtures with no compiler/build execution in the
baseline. `FILE_ONLY` for valid supported Kotlin fixtures is insufficient.

Exercise source relocation, literal/docstring/helper/configuration changes,
added/deleted scope members, previously empty lookups, ambiguous correspondence,
parse errors and semantic-provider loss/restoration. Preserve exact syntax
authority without inventing resolved calls, runtime order or arbitrary prose
correctness. Changes to a declared source scope must not silently produce a
false-current explanation. Retain prior bundles on failed generation and protect
manual outputs. Publish views from one accepted explanation version.

The report must distinguish released 0.7.1 behavior from the new local candidate,
static inspection from executed checks, and demonstrated fixture coverage from
unqualified language/framework behavior. Private source, raw model logs and
machine paths remain outside the repository; public reports use aggregate
results and synthetic examples only.
