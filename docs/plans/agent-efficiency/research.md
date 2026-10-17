# Research supporting the efficiency slice

Reviewed 2026-10-17. This is a focused engineering synthesis, not a systematic
review or a measured prediction for Codeclew. Primary sources below distinguish
localization, final patch quality, model tokens, latency and billed cost.

| Primary source | Finding relevant to this task | Design implication and limit |
| --- | --- | --- |
| [SWE-agent, NeurIPS 2024](https://proceedings.nips.cc/paper_files/paper/2024/hash/5a7c947568c1b1328ccc5230172e1e7c-Abstract-Conference.html) | The agent-computer interface affects software-engineering performance. | Optimize the sequence of decisions and tool feedback, not only retrieval speed. Different harnesses limit direct numerical transfer. |
| [Agentless, paper v2](https://arxiv.org/html/2407.01489v2) | Structured localization, repair and validation can compete with complex agent loops. | Prefer bounded deterministic orchestration where it replaces repeated model decisions. This does not prove a fixed pipeline wins on every task. |
| [RepoGraph, paper v1](https://arxiv.org/html/2410.14684v1) | Repository graph context helps repair; in the early Agentless comparison, quality improves while input usage/cost also increases. | A graph is a retrieval aid, not evidence of savings. Version-specific table results should not be attributed to the final version. |
| [LocAgent, ACL 2025](https://aclanthology.org/2025.acl-long.426/) | Graph-guided multi-hop localization uses repository structure effectively. | Reuse existing symbols and relationships. Reported cost advantages also involve model choices/fine-tuning; do not attribute them solely to graph retrieval. |
| [OrcaLoca, ICML 2025](https://proceedings.mlr.press/v267/yu25x.html) | Priority scheduling, action decomposition and pruning improve localization. | Rank expansions by task relevance and stop at a sufficient context. Localization accuracy is not final patch success. |
| [Evaluating AGENTS.md, v3, 2026-09-29](https://arxiv.org/abs/2602.11988v3) | Context instructions increase average inference cost by over 20%, without general success improvement. | Shorten required behavior, not just word count. Instructions can induce extra exploration/testing. This is not an instruction to ignore repository rules. |
| [The Complexity Trap, v2](https://arxiv.org/html/2508.21433v2) | Simple observation masking competes with model-generated summaries in the tested SWE-agent settings. | Keep exact evidence retrievable and avoid unnecessary summary-model calls. History masking belongs to the host harness; results vary by model. Workshop evidence, not a universal guarantee. |
| [SWE-Pruner, v4, 2026-05-07](https://arxiv.org/abs/2601.16746v4) | Task-conditioned line selection reduces context with a trained 0.6B skimmer. | Preserve task-relevant source and guards. Its training/inference costs and semantic omission risk do not justify adding a compressor in the first slice. |
| [Agent Retrieval Bench, v1, 2026-07-27](https://arxiv.org/abs/2607.24882v1) | Retrieval winners vary by task/budget; natural no-gold cases expose abstention calibration gaps. | Include exact lookup, test selection, edit impact, ambiguity and no-target controls. File exposure/recall does not establish comprehension or a valid patch. Preprint. |
| [Beyond Token Savings, v1, 2026-09-26](https://arxiv.org/abs/2609.32961v1) | Across roughly 35,000 runs, fewer tokens can accompany longer execution; some Terminal-Bench policies use about one-third the tokens but take 20-80% longer. | Measure time, tokens and paired task quality separately. Cost includes estimated cache assumptions, not a measured Codeclew/provider bill. Preprint. |

[FastContext](https://arxiv.org/abs/2606.14066) is withdrawn; exclude its claimed
savings from decision evidence. Do not combine percentages across different
benchmarks, models or accounting conventions.

## Repository evidence versus hypotheses

The frozen [Q1 v0.2.5 result](../evidence/q1-v0.2.5-warm-result.json) has five warm
tasks and reuses frozen default-arm runs. In ONE-REPO-CHANGE, default reports
72 seconds / 5 tool calls / 28,726 noncached input tokens; Codeclew reports
286 seconds / 36 calls / 77,406 tokens. All five task outcomes pass. This motivates
investigation but cannot quantify current 0.13.34 overhead: behavior, guarantees
and setup differ, and cold setup was excluded.

Current source confirms compact navigation, retained generation reuse and prepared
SOURCE comparison already exist. Therefore plausible remaining costs are model
decisions for discovery, broad context selection, repeated evidence presentation,
long managed instructions and redundant workflow checks. Attribution requires
trajectories; source inspection alone cannot rank their measured contribution.

The strongest falsifiable hypothesis is that an engine-selected sufficient edit
context can replace several model-mediated searches. Compare with an expert-built
context arm before investing in a general planner. If search itself is only 10%
of wall time, eliminating it entirely yields at most 1/0.9 = 1.11x speedup; faster
graph traversal cannot solve the remaining 90%.

## Priority after the new prepared SOURCE path

1. Account for total task cost and split prepared maintenance from unknown-location
   editing. Avoid crediting already merged improvements to a new implementation.
2. Produce one bounded context with edit sites, necessary dependencies, available
   tests and unresolved evidence, reusing existing engines.
3. Reduce repeated payload and model bookkeeping while preserving authority and
   necessary guards, exception paths and effect ordering.
4. Only then consider provisional exploration, validation-receipt reuse, learned
   retrieval or host observation masking, each with a separate correctness gate.

The earlier [token economics research](../codeclew-token-economics-research.md)
and [staged plan](../codeclew-token-efficiency-plan.md) remain useful background.
Their dated proposals are not evidence that a feature is absent today.
