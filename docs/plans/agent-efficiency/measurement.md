# Offline measurement contract and evaluation plan

The analyzer in `scripts/agent_efficiency_report.py` is standard-library-only and
never calls an agent, provider, network, compiler or Codeclew. Its strict validator
defines `codeclew-agent-efficiency-run/1.0`; it consumes one complete run per JSONL
line and emits a descriptive paired report. It does not establish provenance of
submitted numbers, compute billing, qualify a release or automatically declare a
winner. Test values are synthetic and are never benchmark evidence.

## Four arms, one task contract

- `bare`: strong agent with rg, bounded source reads, native edits and native tests.
- `clew-current`: pinned current Codeclew and its actual short-path routing.
- `clew-compact`: same authority/validation guarantees, proposed edit-context path.
- `oracle-context`: expert-selected sufficient evidence, with no solution/patch
  leakage. Diagnostic ceiling, not a production implementation or fair acquisition
  cost win; report preparation cost separately.

Run the analyzer pairwise with exactly two arms per input. Preserve the entire run,
including failed calls, retries, fallback and setup. A failed attempt is not removed
or replaced by a later successful attempt. A retry within a run contributes to the
same totals; an independent repetition receives a new repeat number.

Freeze task prompts, repository revisions, model/effort, host harness, test oracle,
runtime/skill versions and guarantees. Use clean independent worktrees and managed
state per arm. Randomize arm order; keep cold and warm runs separate. Start with
public fixtures and deterministic replay, then a separately authorized agent pilot.
No new paid run is authorized by these documents.

Task strata: exact lookup/small edit; unknown-location edit; ambiguous overload;
cross-file effects and failure handling; cross-service contract; no relevant target;
prepared SOURCE unchanged maintenance; prepared SOURCE changed dependency.
Include large retained dependencies and budget exhaustion. Ground truth should
contain expected relevant source and tests plus prohibited false exact claims.
Quality review must be blind to arm where practical. Same passing test count is
insufficient if a critical dependency or error path is omitted.

## Run fields (all required)

| Field | Meaning |
| --- | --- |
| `schema` | Literal `codeclew-agent-efficiency-run/1.0`. |
| `experiment`, `corpus` | Frozen experiment and task-corpus identifiers. |
| `model`, `effort`, `harness` | Exact execution settings/versions, not model marketing families. |
| `guarantees` | Shared test/authority/publication contract identifier. Different guarantees are separate comparison groups. |
| `cache` | `cold` or `warm`; declare what was primed in the experiment manifest. |
| `stratum` | Task category; do not average unrelated categories silently. |
| `evidenceKind` | `observed` or `synthetic`; synthetic values never pool with measured runs. |
| `task`, `taskDigest`, `fixtureDigest`, `repeat` | Pair identity: prompt digest, frozen repository-set digest and nonnegative repeat integer. Use SHA-256 values in observed records, opaque fixture labels only in synthetic tests. |
| `arm` | Exact arm identifier passed to the analyzer. |
| `implementationRevision`, `skillDigest` | Per-arm implementation and skill authority. These may differ across arms; summaries expose all values. |
| `outcome` | `pass`, `fail` or `unresolved`, according to the frozen quality oracle. |
| `criticalMisses`, `falseExactClaims` | Nonnegative counts from independent evidence review. Passing tests with either count nonzero is not a quality pass. |
| `metrics` | Object with every metric below. Unknown values are null, never zero. |

The validator enforces shape and arithmetic, not whether hashes point to real
artifacts. Keep the raw trajectory, tokenizer/provider usage receipts, quality
review and environment manifest locally. Do not include private source or prompts
in public summaries. Implementation revisions and model names alone are not a
complete experiment manifest.

## Metrics

| Metric | Accounting definition |
| --- | --- |
| `elapsedMs` | End-to-end monotonic wall duration, including setup appropriate to the cache class. |
| `firstJustifiedEditMs` | Offset from task start to first edit supported by relevant evidence, judged from the trace; null for analysis-only/no edit. |
| `verifiedPatchMs` | Offset to final qualifying patch validation; null without a qualifying patch. |
| `inputTokens` | Sum of all model-call input usage, including cached input. |
| `cachedInputTokens`, `uncachedInputTokens` | Disjoint input components; total equals their sum. Never add cached input to a total that already includes it. |
| `outputTokens` | Provider-reported output usage, with the provider's reasoning-token convention documented. |
| `toolCalls`, `failedToolCalls`, `modelRounds` | Totals over the entire run, including retries and unsuccessful exploration. Define one round as one model request. |
| `toolOutputBytes` | UTF-8 bytes actually delivered as tool results to the model. |
| `sourceBytes` | Subset of delivered tool bytes containing source text. |
| `repeatedSourceBytes` | Re-delivered source bytes with identical repository revision/file/range identity; adapter must define overlap handling. |
| `indexMs`, `validationMs` | Instrumented stage durations. May overlap other work; do not sum them to infer wall time. |

Counts must be integers; durations finite nonnegative numbers. Byte counts are not
tokens. Provider billing/cached-prefix savings require actual usage plus a pinned
price table and are intentionally outside this analyzer. Do not present inferred
cache cost as an observed bill.

For attribution, retain per-round events locally: start/end, phase, command kind,
delivered bytes by instruction/source/facts/authority/limits/error category and
provider usage. The included code aggregates normalized runs; **the host trajectory
adapter and trustworthy per-round instrumentation remain implementation work**.

## Interpreting reports

Comparisons are partitioned by experiment, corpus, model, effort, harness,
guarantees, cold/warm class, stratum and synthetic/observed evidence. Task digests,
fixture digests and repetitions must match. Missing counterparts are listed and
`allTasksPaired` is false. No unmatched task contributes to paired ratios.

Reports show all-run quality counts, per-task quality losses, and metrics both for
all paired runs and for pairs where both arms passed quality review. Fast failures
remain counted; their short duration must not be sold as a speedup. Unknown metrics
stay unknown. Zero baseline produces no ratio. Check `knownPairs`, `ratioPairs`
and `complete` before comparing medians. Ratios are candidate/baseline; below one
means less of that metric, not necessarily better overall.

Predeclare a pilot target: for selected editing strata, median fresh input and
model rounds at least 30% lower than current Clew, with no paired quality loss,
critical miss or false exact claim and no material wall-time regression. This is
a proposed engineering gate, not a scientific forecast. Also compare against
bare; improving current Clew alone does not prove user value. Report per-task
results and uncertainty; five anecdotes cannot establish general superiority.
The analyzer provides descriptive statistics only. A powered evaluation and
confidence intervals remain work for an authorized benchmark, not unit tests.
