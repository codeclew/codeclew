# Agent efficiency: implementation handoff

Date: 2026-10-17. Base: `b77635448eaab2202c26d38e21e1cec5fe291c29` (main,
0.13.34). Status: implementation task plus offline measurement tooling; no new
agent benchmark, provider authoring, runtime qualification or product optimization
is claimed by this branch.

## Options and current evidence

Read [the decision study](options.md) before choosing an implementation slice.
It incorporates a completed eight-chat Weather experiment and an offline replay
through this branch's analyzer. Prepared SOURCE maintenance and unknown-location
editing remain separate strata. The proposed compact/edit paths have not been
implemented or evaluated by that study.

## Start here, Codex

Implement a bounded **task-to-edit-context** path using existing Codeclew admission,
retrieval, source, graph and mutation primitives. Reduce the model's search and
orchestration decisions while preserving evidence authority and patch quality.
First instrument the current path; then ship one small, testable projection slice.
Do not begin with a new graph engine, provider call or full repository summary.

Read this file, [the experiment contract](measurement.md), and the relevant source
functions below. Read [research](research.md) when choosing a design tradeoff.
This is a continuation of [the existing staged plan](../codeclew-token-efficiency-plan.md),
not a second planner initiative. Respect repository AGENTS.md. No paid runs,
additional authoring, release, public private-source fixtures or messages to other
chats are authorized by this handoff. Development preference: Sol-6.1 High; no Astra.

## Reconcile the baseline before coding

The previous analysis inspected `26dc594223c69b660b4cd83a9ce866bc755ad0c9`.
Main has since merged PR #30. Do not implement these improvements again:

| Existing mechanism at the base | Reuse / remaining limitation |
| --- | --- |
| `skills/codeclew/SKILL.md`: 42 lines, 2,267 UTF-8 bytes | Short dispatch already exists. Managed guide remains 613 lines / 36,384 bytes and should only be loaded for its workflow. These are bytes, not tokens. |
| `skills/codeclew/README.md`, `scripts/prepared_source.py` under that skill | Exact-selector prepared SOURCE route, bounded 12,000-byte view, local continuations, saved packets and maintenance already exist. This is not discovery of an unknown edit location. |
| `documentation/prepared_evidence.rs::{retain,compare}` | Existing immutable evidence and registered dependency comparison. UNCHANGED is limited to registered scope, not semantic/runtime equivalence. |
| `navigation.rs::{agent_card,expand_delta_with_decision_identifier}` | Compact cards and candidate deltas already exist. Measure remaining metadata/source repetition before changing them. |
| `context_v2.rs::create_with_selector`, `generation_service.rs` | Parent expansion loads an existing generation. Do not claim every expansion rebuilds it. |
| `task_run_v2.rs::{prepare,public_candidate_status,publish}` | Existing plan, validation, candidate and publication checks. Keep final atomic freshness and CAS checks. |
| Managed guide: `--intent` does not affect retrieval | Generic navigation still asks the model to select lexical terms and evidence needs. This is a candidate cost, not measured causality. |

Rust paths above are relative to `crates/clew/src/`. Source references are pinned
by the base SHA; recheck these symbols if rebasing. Prepared SOURCE tests live in
`scripts/test_prepared_source.py`; skill copies must stay identical.

## Implement in reviewable slices

1. **Measurement adapter.** Normalize an existing local trajectory into the run
   contract in measurement.md. Preserve failures, retries, cold setup and provider
   token accounting. Use the included analyzer; do not guess tokens from bytes.
   Separate already-prepared SOURCE maintenance from discovery/edit tasks.
2. **Bounded edit-context projection.** Extend an existing navigation/context
   projection rather than creating another capture store. Given an admitted
   session, task terms or exact selector and output budget, return selected edit
   candidates, exact source ranges, material callers/contracts, available relevant
   tests, and explicit unresolved/omitted items with engine-owned drill handles.
   Select lexical lookup for exact names; use existing relations for bounded
   expansion. No LLM, embedding service or trained compressor in this slice.
3. **One-action acquisition.** Put deterministic orchestration inside the engine:
   reuse admission and generation, select/deduplicate ranges, assemble the bounded
   response. Avoid requiring the model to copy source IDs and authority fields
   between multiple discovery calls. Keep full artifacts locally accessible.
4. **Evaluate before broadening.** Exercise the deterministic fixtures below and
   compare recorded trajectories. The oracle-context arm is diagnostic: if it
   cannot outperform a strong bare-agent baseline, redirect that task stratum
   instead of building a more elaborate planner. Paid evaluation needs separate
   authorization; offline checks can proceed now.

Do not change the meaning of ABSTAIN merely to shorten a run. If provisional
exploration is needed, implement an explicit exploration state with tests proving
that candidates cannot become decision authority through navigation alone.
Mutation receipt reuse, learned ranking and host history masking are later slices.

## Projection contract to implement

This is a design contract, not an already supported CLI/schema:

- Bind the response to session, repository revisions, generation and selected
  authority. Reuse opaque handles; no model-invented identities.
- Return candidates with selection reasons, source spans, source authority and
  evidence refs. Ranking is a hypothesis, not call-target or runtime proof.
- Deduplicate source by revision/file/range; preserve complete predicates, failure
  paths and enclosing control flow needed for the requested edit. Excerpts are
  explicitly partial and have bounded continuation handles.
- Include relevant known tests and why they are selected. Missing test relations
  remain unresolved; a guessed test is not verified coverage.
- Budget response bytes independently from model tokens. Preserve authority,
  boundaries, omissions and next actions even under a tiny budget. If required
  metadata cannot fit, return a typed budget error; do not emit false completeness.
- Deterministic selection and serialization for identical inputs; supported
  additive versioning and unchanged legacy command behavior. Requesting a compact
  view must never mutate retained source or silently change retrieval semantics.
- Report engine-stage durations separately from model/tool wall time. A CLI can
  reduce future output; it cannot evict old outputs from a host model's context.

## Acceptance and regression matrix

| Case | Required result |
| --- | --- |
| Exact symbol / small edit | Bounded source plus relevant existing test; no forced broad graph walk. |
| Unknown location / cross-file dependency | Selected candidates and necessary dependency source; no false exact target. |
| Two overloads or ambiguous receiver | Ambiguity survives projection; caller branch is not chosen by name alone. |
| Error path / conditional effect | Guard and failure continuation survive pruning; save is not asserted as commit. |
| Stale revision / changed callee | Existing freshness/delta machinery detects registered changes; unchanged handler is insufficient. |
| No relevant target / unavailable evidence | Explicit unresolved state and minimal next acquisition; no empty-success packet. |
| Tiny budget / Unicode / large dependency | Byte bound enforced, omissions visible, continuation reconstructs source exactly. |
| Repeat expansion / prepared maintenance | No unchanged source replay unless requested; no redundant agent-level hash/Git checks for engine-covered evidence. |
| Mutation | Same CAS, validation, approval and publication guarantees as baseline. |

Add meaningful tests at the existing projection/CLI boundary. Reuse native fixtures
where possible; public synthetic fixtures must say SYNTHETIC and cannot establish
compiler or runtime correctness. Include historical semantic pitfalls only as
small generic regression inputs: assigning IN_PROGRESS overwrites any prior
status; isNullOrBlank includes whitespace. Do not reopen entity-page authoring.

## Commands and deliverables

This branch's executable code is measurement tooling, not the new product path:

```sh
python3 -I -S scripts/test_agent_efficiency_report.py
python3 -I -S scripts/agent_efficiency_report.py /absolute/runs.jsonl --baseline bare --candidate clew-compact
python3 -I -S scripts/test_prepared_source.py
python3 -I -S scripts/test_agent_skill.py
python3 -I -S scripts/check_english_content.py
python3 -I -S scripts/check_repository_privacy.py --worktree
```

For the implementation, use the pinned toolchain, `cargo fmt --all --check`, focused
Rust projection tests and a CLI integration regression. Follow AGENTS.md for the
merge-ready gate. Report exactly which checks ran. Return a commit/PR, a concise
before/after command transcript, the unchanged authority guarantees, and measured
per-task results or an explicit measurement blocker. Do not label static tests a
token/time win. Do not merge unrelated experimental branches automatically.

Related optional experiments: `research/thread-search-efficiency-420c034`
(`57a957b048a158e4e512f66a2a15b236e9b62449`) and `feat/thread-call-bindings`
(PR #24). Their kernel/call-target improvements are separate from proving
end-to-end agent efficiency; inspect current status before reusing them.

## Handoff verification

On 2026-10-17: accounting regressions 9/9, existing prepared SOURCE tests 9/9,
skill tests 6/6; English content, worktree privacy, relative Markdown links and
staged diff whitespace checks passed. Tests use synthetic accounting inputs and
local fixtures; no new agent trajectories or provider calls were run. No Rust
product code changed, and no Rust/native/runtime/CI qualification is claimed.
The trajectory adapter, product edit-context projection and measured comparison
are explicitly left for implementation. Publication metadata is separate from
the passing worktree-content privacy check.
