# Efficiency options and a bounded next decision

Date: 2026-10-17. Research base: `e34d2143a255c711eef91d9f6ab1b2d41d61dc01`.
Status: design alternatives, observed prepared-SOURCE evidence and offline
accounting. No new product path, new agent run, release or performance qualification.

## Recommendation

Do not promote Codeclew-first as a universal agent policy. Use the existing
prepared-evidence route for retained answers that need its provenance, and preserve
native exact lookup as a first-class route. Investigate one bounded edit-context
projection for tasks whose expensive part is locating and connecting code. Do not
build a general planner until an oracle-context comparison demonstrates headroom
for that editing stratum.

The next implementation slice should be a deterministic projection over existing
retrieval/admission, preceded by trace instrumentation. A small prepared-maintenance
interface improvement is independently worthwhile but cannot justify the editing
planner. Host history masking and learned retrieval are separate experiments.

## What the new Weather result establishes

Eight fresh GPT-6.1 Sol High chats, four stages per arm, same fixture and quality
oracle. Installation, service registration and capture/indexing were excluded.
Answer-time admission, native evidence registration, comparison and failures were
included. Both arms passed all four answers on their first attempt; no repair was
used. This was analysis and maintenance, not editing or patch validation.

| All four stages | Native | Codeclew candidate |
| --- | ---: | ---: |
| Wall seconds | 134.75 | 173.94 |
| Total tokens, input plus output | 482,039 | 747,620 |
| Cached input tokens | 398,848 | 595,840 |
| Uncached input tokens | 77,213 | 143,566 |
| Output tokens | 5,978 | 8,214 |
| Model requests | 20 | 26 |
| Nested terminal commands | 27 | 33 |
| Quality | 4/4 | 4/4 |

The three maintenance stages cost 96.29 versus 123.47 seconds and 379,901 versus
542,596 total tokens. Codeclew still costs 1.28x time and 1.43x total tokens there.

The previous Codeclew run cost 219.31 seconds and 1,147,441 tokens including one
50.33-second / 415,102-token citation repair. Current totals improve by 20.7% and
34.8%, but uncached input increases by 16.9%. Excluding that old repair solely as a
diagnostic gives 168.98 seconds / 732,339 tokens versus 173.94 / 747,620 now. Normal
first-attempt acceleration has not been demonstrated. The old run also had a
remaining native completeness omission; historical quality was not identical.

Candidate `824d4fb335e8b1ddeaf60977ea29619aa8be4d8d` includes the old pinned-SOURCE
drill fix on top of this branch's `b7763544` base. It is a local packaged candidate,
not a public release. Both citation bindings and historical offline reads passed
independent acceptance. The measured title-change answer used ready citations
without a historical drill; its absence is useful, not a measurement of drill speed.
Do not credit PR #30 or this candidate work to a future edit-context implementation.

### Attribution supported by the trajectories

- Bootstrap Git checks decreased from seven to two invocations. One changed-stage
  agent still ran status and HEAD before reading the skill. An instruction inside
  a file cannot control a decision made before that file is read.
- There were no redundant Git checks of covered evidence. Five initial Git reads
  acquired previously uncovered C# and were subsequently registered. They remain
  allowed; eliminating all Git would weaken the baseline and coverage.
- Each maintenance stage read previous packet metadata before compare. Compare
  already accepts the retained packet. This is a candidate avoidable model step;
  its isolated time/token contribution has not been measured.
- A changed-stage agent requested `--max-sources 0`, received an invalid-bounds
  error, then repeated with one source and exposed 19,187 characters. This is one
  bounded recovery opportunity, not evidence that all broad reads are unnecessary.
- The first Codeclew answer included four receiver files, including the optional
  publisher; native inspected a smaller receiver scope. Both met the same rubric.
  Extra scope is a behavioral confound, not a mandatory Codeclew cost.
- The four default projections regenerated offline were 10,346, 11,560, 3,290 and
  11,909 bytes (without the trailing newline). The unchanged response already
  contains no SOURCE text. No exact duplicated reason objects were found between
  its reason list and exposed change handles in these projections. Do not propose
  those already-absent duplicates as measured savings.

See [normalized runs](evidence/weather-observed.jsonl),
[paired offline report](evidence/weather-offline-report.json),
[receipt hashes and limits](evidence/normalization-receipts.json), and
[payload audit](evidence/payload-audit.json). Exact source/repeated-source byte
counts remain null. The hash receipts identify retained local raw evidence without
publishing its paths, prompts or session contents. This is one journey, not a
statistically powered evaluation; no additional provider run was performed here.

## Alternative paths

| Option | Concrete change | What it could remove | Main risk / limit | Decision |
| --- | --- | --- | --- | --- |
| A. Instruction-only cleanup | Put the prepared route before generic bootstrap; document direct compare and valid metadata bounds | Some early Git and pre-compare reads | Fresh agents can act before loading it; longer rules can add cost | Small maintenance follow-up, not the main product strategy |
| B. Prepared maintenance view | Existing compare returns a self-contained decision view with answer-relevant deltas, both citations, limits and opaque continuation | Packet bookkeeping and optional projection widening | Removing material coverage or binding data to meet a size target | First narrow usability slice; retain the full packet and existing contract |
| C. Bounded edit-context | Compose admitted lookup, exact source and existing relation facets into one acquisition | Several model-mediated discovery/expand/choose steps | Wrong candidate or omitted guard/callee/test; no editing trajectory yet proves savings | Preferred product hypothesis after an oracle gate for editing |
| D. Native-first routing | Exact literal/name tasks use native lookup; existing prepared evidence uses B; unresolved semantic tasks return a scoped next action | Always-paid admission/instruction overhead on easy or unsupported tasks | Routing error; native fallback must not inherit managed authority | Keep as an explicit route, with guarantees declared at entry |
| E. Host observation masking | Host replaces consumed tool bodies with retained handles and small state summaries | Repeated historical input to later model requests | Host integration, cache effects and lost needed observations | Separate host experiment; not solvable by the CLI alone |
| F. Learned retrieval/compression | Rank or prune task context with an additional model | Potentially irrelevant source in hard discovery tasks | New latency, training/deployment cost, semantic omissions | Defer; current evidence does not justify it |

A and B concern the measured prepared-source stratum. C concerns unknown-location
editing; Weather does not evaluate it. D chooses between routes, rather than
silently replacing a managed contract midway. E and F change a different component
and must not be mixed into C's first comparison.

## B: smallest independently useful maintenance slice

Keep `retain-source` / `compare-source` and their opaque evidence set. The ordinary
happy path is read the short entry, compare the prior packet directly against the
exact target, then update or retain the answer. Reading old packet metadata is
optional diagnosis, not preparation.

Proposed additive view semantics, not an implemented schema:

1. A single authority block names exact baseline/target bindings, registered scope,
   syntax/compiler/native boundaries, integrity status and unresolved inputs.
2. A decision block reports CHANGED/UNCHANGED/UNKNOWN and selected-source status.
3. Changes contain each necessary before/after value and containing-source citation
   once. Unchanged prose need not be replayed; updated citation bindings must still
   be available. Return the actual required deltas, not only opaque digests.
4. One retained-packet handle plus typed continuation identifies omitted material.
   Keep the full original packet local. A tiny budget cannot erase a limit or turn
   partial evidence into an unchanged/completeness claim.

Do not change the successful immutable admission, add a new cache, or remove final
mutation freshness checks. Decide whether a metadata-only mode accepts zero sources
or exposes a named mode; whichever is chosen must reject malformed packets and
preserve limits. Avoid requiring the agent to infer numeric flag constraints.

Acceptance: unchanged target has no forced source replay; changed dependency with
stable handler is still changed/unknown; missing native input is unknown; both
citation spans survive a title-only delta; tiny budget fails explicitly. Use exact
identity controls and Unicode source continuations. Observe the whole answer run
before claiming savings. Shortening 3,290 bytes alone is not a token-economics result.

## C: bounded edit-context, using current seams

The inspected code already provides:

| Existing seam at this branch | Reuse and boundary |
| --- | --- |
| `context_v2.rs:665`, `create_with_selector` | Existing generation loaded for parent expansion; initial acquisition ensures it. No assumption that every expand rebuilds an index. |
| `context_v2.rs:1916`, `bounded_projection` | Existing 54 KiB projection, facts then sources. Add a task view; do not duplicate the source store or silently alter legacy ordering. |
| `navigation.rs:59`, `agent_card` | Compact authority-aware result exists. Preserve its required authority and next-action fields. |
| `navigation.rs:368`, `expand_delta_with_decision_identifier` | Candidate deltas already retain parent/session identity. Reuse rather than create another delta protocol. |
| `navigation.rs:2097`, `relation_facet` | Existing callers/callees/tests from bounded relation facts. No TEST relation means unsupported, not evidence that tests do not exist. |
| `task_run_v2.rs:299,1186`, prepare/publish | Existing validation and final compare-and-swap remain mandatory for that managed route. |

Input is an admitted session, an exact selector or explicit lexical task terms,
optional existing context handle, and a response budget. Keep the first prototype
deterministic. `intent` text currently describes the task; it is not a semantic
retrieval planner. Do not claim arbitrary natural-language task understanding.

Acquisition order:

1. Resolve exact names using current indexed lookup. Ambiguity is a result, not a
   reason to choose a receiver by name. Retain identities for all exposed candidates.
2. Select candidate source spans and necessary enclosing predicates/failure paths.
   A contiguous range may be larger than a few matching lines. Do not prune guards
   just to meet the budget; expose a continuation or explicit insufficient context.
3. Add existing direct callers/callees/contracts and available test relations under
   a shared bound. Begin with direct relations; recursive arbitrary traversal is
   outside this slice. Lexical test suggestions remain discovery, not coverage.
4. Deduplicate by repository, revision, file and overlapping ranges before
   serialization. Emit one source body and references from the relevant facts.
5. Return edit candidates, reasons, source, test candidates, authority, unresolved
   items and exact next handles in one response. Persist full artifacts separately.

The existing agent-card validator forbids a decision SOURCE for ABSTAIN. Do not
smuggle provisional candidates into that field or relabel them SUPPORTED. For a
first slice, keep discovery previews distinct from decision evidence and leave
ABSTAIN unchanged. A new exploration state is a separate versioned change with
negative tests; it is not needed merely to serialize existing candidates together.

Prefer an additive view on an existing navigation/context boundary. A convenient
CLI spelling is secondary; a new top-level command alone does not eliminate model
choices. There must be one supported acquisition that selects/reuses internal
handles, not a skill recipe asking the model to chain the old commands itself.

## D: an explicit economic routing decision

Let H be always-paid route overhead; S savings when the supported route succeeds;
p the probability of that case in a declared task stratum; and F extra cost of a
failed Codeclew attempt before native continuation. Relative expected cost is:

`H - p*S + (1-p)*F`. Break-even requires `p > (H+F)/(S+F)` when `S+F > 0`.

Evaluate this separately for time, total tokens and uncached input; they are not
interchangeable currencies. If S is nonpositive, improving p cannot manufacture
savings. No p, H, S or F has been estimated for unknown-location editing by Weather.

Choose from explicit input facts first: supplied exact snapshot/packet, exact
literal/config task, admitted relation-capable session, or unsupported authority.
Do not add a classifier model in the first router. If the requested managed
workflow is unavailable, report that boundary; native exploration cannot silently
stand in for managed publication or compiler evidence.

## Evaluation and implementation sequence

1. Keep the offline adapter outside product code until its provenance and counting
   unit are agreed. Normalize existing trajectories without provider execution.
   Record both host tool envelopes and nested commands: this run has 38 versus 60,
   with 46 model requests. Do not call 60 commands 60 model decisions.
2. For B, use the already measured packet fixtures and deterministic regressions.
   Compare projection-only output before changing admission/retrieval. The next
   quality experiment, if separately authorized, must include every failed call.
3. For C, define at least one task each for unknown location, ambiguous overload,
   changed callee, conditional failure, relevant-test absence and no target. Use
   existing language fixtures. Have an expert build sufficient context without a
   solution or patch; keep its preparation cost separate.
4. Run bare/current/oracle only after a specific pilot is authorized. If the oracle
   does not beat bare with equal quality, stop C for that stratum. A positive oracle
   is only a ceiling; the implemented acquisition still pays retrieval overhead.
5. Implement the smallest projection that closes the demonstrated gap, then compare
   bare/current/compact with identical authority, tasks and validation. No new
   planner, embedding service, history masking or receipt reuse in the same slice.

The handoff's proposed 30% uncached-input/model-round improvement over current Clew
remains a hypothesis. Cache hit variation can move uncached input independently of
algorithmic work; retain total input and fixed cache classification as guardrails.
Also require the bare comparison. Better than old Clew alone is not product value.
No new paid pilot or implementation is authorized by this document itself.

## Concrete handoff and stop criteria

Coding owns product changes; research owns independent acceptance and experiments.
First deliver instrumentation of acquisition/projection phases and B's bounded
view or C's projection-only fixture slice, explicitly stating which stratum it
serves. Attach unchanged authority tests and a before/after transcript. Do not
combine both as a single claimed speedup.

Reject promotion if a guard, receiver ambiguity, critical dependency, required test
boundary or final CAS protection disappears. Stop economic expansion when a strong
native baseline matches the oracle for that stratum, or when extra recovery cost
cancels ordinary-path savings. Preserve successful functionality even if its value
is auditability rather than speed, and describe that value without a savings claim.
