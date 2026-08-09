# Codeclew E04-R1: product entrypoint, harness repair and S0 baseline detail

Date: 2026-08-09

Status: planning after `E04-S0 = INFRA_ERROR / NO_DECISION`

## 1. Why the next product operation is a generic typed-goal binder

The proposal is not primarily about adding another CLI command. It establishes
one stable product boundary:

```text
task intent + repository snapshot
  -> BOUND | AMBIGUOUS | REFUSED
  -> typed roles, obligations, provenance and proof boundaries
```

The first transport is a stable CLI/tool boundary:

```text
clew bind --repo . --goal goal.json
```

The same Rust operation can later be exposed as an MCP/tool call without
changing its semantics or benchmark contract.

`goal.json` is model-owned only where business intent is irreducible. It does
not contain worker graph IDs, source ranges, hashes, patches, hidden family
labels or repository recipes. It declares typed variables and generic semantic
constraints, for example:

```json
{
  "schema": "semantic-goal/0.3",
  "baseRevision": "...",
  "variables": [
    {"id": "values", "domain": "VALUE_EDGE", "hints": ["itemsAwaitingContext"]},
    {"id": "context", "domain": "CALLABLE", "hints": ["mappingContext"]},
    {"id": "transform", "domain": "CALLABLE", "hints": ["applyMappingContext"]}
  ],
  "constraints": [
    {"kind": "BIND_UNIQUE", "args": ["values"]},
    {"kind": "BIND_UNIQUE", "args": ["context"]},
    {"kind": "BIND_UNIQUE", "args": ["transform"]},
    {"kind": "TYPE_ASSIGNABLE", "args": ["values", "transform"]},
    {"kind": "INTRODUCE_ONCE", "args": ["context"]},
    {"kind": "MAP_VALUE_EDGE", "args": ["values", "transform", "context"]},
    {"kind": "PRESERVE_ORDER", "args": ["values"]},
    {"kind": "PRESERVE_CARDINALITY", "args": ["values"]},
    {"kind": "MUST_REFUSE_ON_BOUNDARY", "args": ["values", "transform", "context"]}
  ]
}
```

The production binder has no `family` field and no `match family` dispatch.
Benchmark families are labels used only by the hidden evaluator. A new task is
supported when its goal composes from the same domains, relations and proof
rules. A genuinely new semantic boundary may add one reusable fact provider or
predicate, but never a task/repository-specific source recipe.

The caller-owned constraint list is not the proof closure. Every operator has a
worker-owned specification:

```text
ConstraintOpSpec {
  arity
  operandDomains
  requiredEvidenceRelations
  mandatoryClosure
  refusalOnUnknown
}
```

The worker type-checks the goal and expands `mandatoryClosure` to a fixed point.
For example, `MAP_VALUE_EDGE` adds type compatibility, placement,
effects/nullability/order/cardinality/laziness/consumer preservation, boundary
closure and the selected oracle policy even if the caller omitted them. A proof
is complete only when its obligations exactly equal the normalized closure,
with no missing or extra items.

This is preferred to the available alternatives for the following reasons:

| Alternative | What it improves | Why it does not close the product gap |
|---|---|---|
| Better prompt with exact current CLI syntax | Avoids bad flags and subcommands | The model still selects roots, family and test, then composes several partial outputs. The benchmark measures CLI choreography rather than task binding. |
| A generic skill around `agent-context`, `projection` and `prove` | Hides some invocation details | The skill either leaves semantic assembly to the model or becomes a family/repository recipe. Neither gives a worker-owned proof. |
| Improve only `agent-context` | Better bounded navigation | It returns evidence surfaces, not the exact typed goal, ambiguity set or must-refuse decision. Model-owned planning and transcript replay remain. |
| Add more transform macros | Fast on known shapes | It risks recipe proliferation and does not define a common goal/proof boundary. |
| Permit grep fallback | Makes more runs finish | It directly falsifies the no-grep hypothesis and hides applicability failure. |
| Add one hardcoded binder per benchmark family | Broad nominal coverage | It turns evaluation labels into production recipes and cannot generalize to unseen compositions. |
| Put all source into a larger model context | Simplifies tooling | It recreates the default workflow and loses repository-size-independent context. |

The first executable composition may use `MAP_EDGE_WITH_CONTEXT` as a cheap
probe, but it is not an E04 candidate by itself. Before E04-R1, at least five
preregistered D02 families must be expressible by the same family-neutral goal
schema and proof kernel without adding five production dispatch branches.
Binder-only `EXTERNAL_SPEC` may be accepted as a stated intended behavior, but
it must not be promoted to a materialization/test proof. This preserves the
later safety gate.

The reason this can reduce tokens is structural: one model call provides only
the irreducible task intent; one worker call owns discovery, candidate
enumeration, binding and proof/refusal. The output size depends on the change
obligations, not repository size or textual patch size.

## 2. S0 default and ast-index aggregate statistics

The following measurements are native provider telemetry. `raw` means
`input + output`; `noncached` means `input - cached input + output`.

| Metric | default | ast-index |
|---|---:|---:|
| Runs | 42 | 42 |
| Stored accepted | 15 | 0 |
| Aggregate wall time | 909.140 s | 851.600 s |
| Median / mean wall time | 21.907 / 21.646 s | 14.304 / 20.276 s |
| Raw tokens | 1,949,742 | 2,139,492 |
| Median raw tokens/run | 52,740 | 26,004.5 |
| Noncached tokens | 261,166 | 297,572 |
| Median noncached tokens/run | 5,649.5 | 4,302 |
| Output tokens | 28,798 | 24,146 |
| Action calls | 68 | 65 |
| Median action calls/run | 2 | 0 |

These numbers do not establish an AST speed/token win. Thirty AST runs made no
AST call and usually refused directly from the overly revealing task text;
twelve used ast-index. All 42 were then invalidated by the broken shell-wrapper
audit.

### Why default results were not accepted

Ignoring the seven false audit flags, the 42 outputs divide as follows:

| Strict semantic result | Count |
|---|---:|
| Exact hidden answer | 18 |
| Only undefined oracle-class choice differed | 9 |
| Wrong outcome status | 6 |
| Wrong refusal code | 4 |
| Wrong binding plus oracle | 2 |
| Wrong ambiguity choices | 2 |
| Wrong binding only | 1 |

Three of the 18 exact answers also received the false
`SOURCE_EDIT_ATTEMPT` flag from `2>/dev/null`; therefore the stored judge
accepted only 15. If the undefined oracle class is excluded from correctness,
27/42 outputs match the remaining hidden semantics. This is diagnostic only,
not a replacement gate score.

### Why ast-index results were not accepted

All 42 have the invalid `AST_INDEX_NOT_USED` audit flag. Content-only analysis,
ignoring that broken audit, gives:

| Strict semantic result | Count |
|---|---:|
| Exact hidden answer | 11 |
| Wrong outcome status | 17 |
| Only undefined oracle-class choice differed | 5 |
| Wrong refusal code | 4 |
| Wrong binding | 1 |
| Wrong binding and evidence | 1 |
| Wrong ambiguity choices | 1 |
| Wrong family | 1 |
| Wrong binding, oracle and evidence | 1 |

Of the eleven exact outputs, five actually used ast-index and six answered from
the public wording. Among the twelve tool-using runs, five were exact and seven
were semantically wrong. Therefore S0 cannot isolate the value of the index.

## 3. Per-task default and ast-index statistics

Case notation is `family/variant/build`: `P`, `A`, `R` are positive,
ambiguous and must-refuse; `G`, `M` are Gradle and Maven. Token cells are
`noncached/raw`. `exact` means exact hidden semantic content before the broken
audit. `audit-redir` is the false default write flag. Every AST row has
`audit-broken`; `used` means a raw event actually invoked ast-index.

| task | case | default s | default tokens | default outcome | AST s | AST tokens | AST outcome |
|---|---|---:|---:|---|---:|---:|---|
| 08af693fd34e6435 | PERS/P/G | 21.0 | 19,152/35,280 | bindings 2/3 | 13.5 | 17,464/17,464 | not-used; audit-broken; bindings 2/3 |
| 12a3c39747655c9d | PTC/P/M | 21.0 | 19,883/53,163 | oracle DERIVED≠EXTERNAL_SPEC | 11.9 | 17,490/17,490 | not-used; audit-broken; oracle mismatch |
| 18be565ba8df4969 | CONF/R/M | 25.2 | 7,764/72,276 | wrong refusal code | 12.9 | 4,420/34,628 | not-used; audit-broken; wrong refusal code |
| 1f6b46219d2ca03f | PERS/P/M | 22.3 | 5,041/35,249 | bindings 2/3; oracle mismatch | 14.9 | 4,333/34,541 | not-used; audit-broken; REFUSED≠BOUND |
| 22d9a3cc527c59eb | DTO/P/G | 19.2 | 4,976/35,184 | oracle mismatch | 37.9 | 9,080/92,792 | used; audit-broken; oracle mismatch |
| 2af86598c18a711f | TYPE/P/M | 25.6 | 5,170/52,530 | oracle mismatch | 39.4 | 25,149/131,133 | used; audit-broken; oracle mismatch |
| 2ee07cebbc526c12 | CONF/A/G | 22.0 | 5,318/52,678 | exact | 12.7 | 3,263/17,343 | not-used; audit-broken; REFUSED≠AMBIGUOUS |
| 31df8b366d918aa9 | TYPE/P/G | 19.2 | 4,994/35,202 | oracle mismatch | 50.0 | 12,123/151,387 | used; audit-broken; oracle mismatch |
| 360da6aa6cec80cb | CONF/R/G | 23.2 | 5,817/53,177 | wrong refusal code | 18.8 | 4,463/34,671 | not-used; audit-broken; wrong refusal code |
| 386a6e716964ca9e | ERR/P/M | 17.4 | 6,059/52,395 | REFUSED≠BOUND | 18.9 | 4,691/34,899 | not-used; audit-broken; bindings 0/3 |
| 398102f73666854f | TYPE/A/G | 24.9 | 6,082/53,442 | exact | 16.9 | 3,593/17,673 | not-used; audit-broken; choices 0/2 |
| 405ab87752fbeb24 | ERR/R/G | 11.2 | 3,179/17,259 | exact | 9.3 | 3,221/17,301 | not-used; audit-broken; exact |
| 441f32d72c319c08 | CONF/P/G | 39.7 | 6,792/54,152 | oracle mismatch | 14.9 | 3,226/17,306 | not-used; audit-broken; REFUSED≠BOUND |
| 4fcd13216e169853 | ERR/A/G | 25.7 | 5,969/53,329 | choices 0/2 | 12.6 | 3,352/17,432 | not-used; audit-broken; REFUSED≠AMBIGUOUS |
| 53e3c5a1aef8ada7 | PERS/R/G | 13.0 | 4,570/34,778 | wrong refusal code | 14.2 | 3,139/17,219 | not-used; audit-broken; wrong refusal code |
| 56399303aa2c25f4 | TYPE/A/M | 25.9 | 6,419/53,779 | exact | 10.7 | 3,291/17,371 | not-used; audit-broken; REFUSED≠AMBIGUOUS |
| 58cf08a936d1b61c | ERR/A/M | 10.4 | 3,042/17,122 | REFUSED≠AMBIGUOUS | 7.8 | 3,104/17,184 | not-used; audit-broken; REFUSED≠AMBIGUOUS |
| 58d71265a54d9202 | CONF/P/M | 21.4 | 5,757/53,117 | audit-redir; oracle mismatch | 19.7 | 3,347/17,427 | not-used; audit-broken; REFUSED≠BOUND |
| 6c04cac6446ae81c | CONF/A/M | 20.1 | 5,294/52,654 | exact | 40.4 | 11,425/148,641 | used; audit-broken; exact |
| 6e5552c7a3854dde | PTC/A/M | 21.9 | 5,442/52,802 | audit-redir; exact | 14.6 | 18,461/34,589 | not-used; audit-broken; REFUSED≠AMBIGUOUS |
| 72371f001b5da6f1 | PTC/R/M | 21.5 | 5,315/52,675 | BOUND≠REFUSED | 14.4 | 2,971/17,051 | not-used; audit-broken; wrong family/refusal |
| 846dfcc0490cbb06 | ERR/P/G | 22.7 | 6,307/53,667 | audit-redir; REFUSED≠BOUND | 36.1 | 8,595/92,307 | used; audit-broken; bindings 0/3; oracle/evidence |
| 88364893311ba42a | DTO/P/M | 17.0 | 4,776/34,984 | oracle mismatch | 9.4 | 4,128/34,336 | not-used; audit-broken; REFUSED≠BOUND |
| 8acae0072eaec973 | TYPE/R/M | 22.2 | 6,212/52,548 | exact | 9.2 | 3,184/17,264 | not-used; audit-broken; exact |
| 943fa8fe1ecf0fa0 | DTO/A/G | 31.3 | 7,549/72,061 | exact | 32.8 | 11,752/112,616 | used; audit-broken; exact |
| 9d1ed064171a6806 | TEST/A/G | 33.5 | 7,236/71,748 | exact | 8.0 | 3,125/17,205 | not-used; audit-broken; REFUSED≠AMBIGUOUS |
| 9f3217a85c70825d | DTO/A/M | 23.8 | 6,022/53,382 | exact | 33.0 | 13,755/112,571 | used; audit-broken; exact |
| a4f6afb6f967c069 | TEST/A/M | 35.4 | 8,217/72,729 | exact | 10.6 | 3,242/17,322 | not-used; audit-broken; REFUSED≠AMBIGUOUS |
| b7e2aa36b687484f | PTC/A/G | 18.8 | 4,920/35,128 | exact | 42.3 | 10,793/112,681 | used; audit-broken; exact |
| be775d25c1431f08 | TEST/P/G | 26.4 | 6,118/53,478 | oracle mismatch | 52.5 | 13,496/176,056 | used; audit-broken; oracle mismatch |
| bf1f2be4f430c3ad | ERR/R/M | 7.6 | 3,022/17,102 | exact | 25.5 | 7,945/107,785 | used; audit-broken; exact |
| c312541f943f162d | TEST/P/M | 28.1 | 5,799/53,159 | audit-redir; bindings 2/3; oracle | 13.0 | 3,289/17,369 | not-used; audit-broken; REFUSED≠BOUND |
| d7507bacd30bfbd1 | PERS/R/M | 8.5 | 3,068/17,148 | wrong refusal code | 29.9 | 10,827/110,667 | used; audit-broken; wrong refusal code |
| dc24bcfab988de27 | DTO/R/G | 10.8 | 3,081/17,161 | exact | 8.1 | 3,114/17,194 | not-used; audit-broken; exact |
| e5c8f78daea0ac81 | TYPE/R/G | 22.8 | 5,738/53,098 | audit-redir; exact | 17.2 | 4,443/34,651 | not-used; audit-broken; exact |
| e849dfcb747db927 | PTC/P/G | 19.4 | 5,551/52,911 | audit-redir; oracle mismatch | 11.3 | 4,385/34,593 | not-used; audit-broken; REFUSED≠BOUND |
| e954da1d729e2d56 | PTC/R/G | 30.3 | 7,367/71,879 | BOUND≠REFUSED | 10.6 | 3,315/17,395 | not-used; audit-broken; BOUND≠REFUSED |
| ea7cde05df087087 | TEST/R/M | 32.9 | 11,495/72,935 | audit-redir; exact | 53.8 | 12,943/115,855 | used; audit-broken; BOUND≠REFUSED |
| ef53f0383d68eb06 | TEST/R/G | 20.7 | 5,561/52,921 | exact | 11.7 | 3,133/17,213 | not-used; audit-broken; exact |
| f1f3c77ae2446c41 | DTO/R/M | 10.8 | 3,150/17,230 | exact | 8.5 | 3,122/17,202 | not-used; audit-broken; exact |
| f45c40d69afacba7 | PERS/A/G | 24.0 | 4,927/35,135 | choices 0/2 | 12.3 | 4,271/34,479 | not-used; audit-broken; REFUSED≠AMBIGUOUS |
| f81e87e8d5253906 | PERS/A/M | 10.5 | 3,015/17,095 | REFUSED≠AMBIGUOUS | 9.5 | 3,109/17,189 | not-used; audit-broken; REFUSED≠AMBIGUOUS |

## 4. Harness changes before E04-R1

### R1. Generic product entrypoint and proof kernel

Replace the current single-family `SemanticGoal` validation branch with a
family-neutral `semantic-goal/0.3` language of typed variables, constraints and
policies. `clew bind --goal` owns root discovery, candidate enumeration, FQN
normalization, exact variable binding, obligation discharge, bounded ambiguity
and typed refusal. It returns a stable JSON schema and never edits source.

The proof kernel must:

- dispatch only on individual registered operators/domains and derive their
  mandatory closure to a fixed point; it must never dispatch on a whole-goal
  signature, family label, variable ID or hint;
- require proof obligations to equal that worker-derived closure exactly, so a
  caller cannot weaken a goal by omitting oracle, preservation or boundary
  constraints;
- require every variable and constraint exactly once in the proof receipt;
- tie every binding and relation to current, sound, source-provenanced compiler
  evidence from the exact clean Git snapshot;
- reject current `Unknown`, unsupported boundaries, stale evidence, missing
  oracle ownership, extra obligations and relabelled/filler facts;
- return no Change Graph for `AMBIGUOUS` or `REFUSED`;
- keep model hints non-authoritative: name matches can select candidates but
  cannot exclude compiler candidates, issue regex/source search or discharge
  semantic constraints;
- expose an opaque authority receipt that cannot be serialized, forged or
  replayed across session/revision boundaries.

The CLI returns only a canonical serializable `BoundSummary`. It cannot
authorize apply/materialization. Only the live authority session owns the
opaque capability; apply must reuse it or rebuild and revalidate the full
closure. `AMBIGUOUS` and `REFUSED` contain neither a capability nor a Change
Graph.

Exit criteria:

- one command on clean committed Gradle and supported Maven fixtures returns a
  valid result or an explicit supported-contour refusal;
- positive, two-candidate, missing-evidence, stale, forged and must-refuse
  fixtures pass for each implemented semantic composition;
- at least five D02 families compile to the same public constraint language and
  kernel, with no family enum/dispatch in the product path;
- alpha-renaming variables, reordering constraints and deleting hints preserve
  the candidate set, status and proof fingerprint;
- one test-only unseen composition binds through existing operators without a
  production-code change;
- no corpus-generator dependency or D02 family/task vocabulary exists in the
  worker, CLI, skill or product prompt;
- no workflow/test symbol beyond model-owned goal hints is required from the
  caller;
- result contains no source patch or repository-specific recipe;
- an independent source/test review finds no false `BOUND` and confirms that
  removing family labels does not change binding behavior.

### R2. Audit real Codex events

- Canonically unwrap only exact `/bin/zsh -lc` and `/bin/zsh -c` envelopes.
- Parse and audit the inner argv; reject compound commands and second readers.
- Record cwd, executable, argv, exit status and output digest separately.
- Perform AST provenance checks before temporary repository teardown.
- Treat shell redirection separately from source mutation; the canonical source
  digest and file-change events remain the write authority.
- Add fixtures copied from actual S0 events, not synthetic bare commands.

Exit criteria: all 126 retained S0 events are reclassified as expected by an
independent test oracle; the known wrapped-command, deleted-repo and
`2>/dev/null` counterexamples pass.

### R3. Hermetic execution environment

- Create a clean committed Git repository for every isolated run.
- Give each run content-addressed, writable Gradle and Maven cache overlays.
- Pin and digest wrapper distributions, dependency caches, JDK, Kotlin,
  ast-index and Codeclew binaries.
- Use `--repo .`; never expose the controller tree or host repository.
- Keep network disabled after cache preparation.
- Distinguish valid product `UNSUPPORTED_*` results from infrastructure errors.

Exit criteria: a no-model preflight opens every generated repository, builds
the AST index, invokes the Codeclew entrypoint, and receives either a valid
schema or a preregistered product refusal—never permission/path/no-HEAD errors.

### R4. Prompt and judge protocol

- Define every role, oracle class and refusal code in the frozen common prompt.
- Score family, bindings, obligations, oracle ownership and evidence as
  separate dimensions; do not collapse infrastructure invalidity into semantic
  zero.
- Remove wording that directly reveals hidden outcome or all bound roles.
- Keep task intent natural; decoys and hidden obligations remain controller
  owned.
- Require identical output schema and semantic rubric in all three arms.

Exit criteria: two blind annotators independently agree on family, outcome,
roles, obligations, oracle and refusal code before any arm runs.

### R5. Cheap preflight and canaries

Before final seed derivation:

1. Run all no-model environment/audit checks: zero model tokens. This includes
   public positive/ambiguous/refuse suites for every claimed composition on
   Gradle and every claimed Maven cell; missing/extra constraint, wrong
   arity/domain, forged evidence, relabelled edge, stale/cross-session receipt,
   current Unknown, partial/truncated coverage, alpha-renaming, reordered
   constraints, hint deletion and unseen-composition mutations.
2. Run one disposable canary per arm: three model calls total.
3. Require native telemetry, unchanged source digest, correct tool recognition,
   at least one successful non-help semantic/navigation result, valid output
   schema and correct known canary judgment.
4. Enforce a combined canary ceiling of 45,000 noncached tokens and 12 action
   calls. Crossing the ceiling stops before corpus materialization.

Canaries are infrastructure/protocol tests, not benchmark evidence.

Before R6, a machine-readable coverage guard must prove that the frozen
supported contour can reach GE1: at least five semantic compositions and at
least 9 of the 14 positive family×build cells are executable. Exact ambiguity
enumeration is required for every claimed cell, and the full preregistered
must-refuse denominator must classify correctly. Five Gradle-only cells are
only 35.7% and therefore cannot open R6.

### R6. Freeze and new withheld series

- Freeze product revision, runner, prompt, schema, tools, cache snapshot and
  audit fixtures only after R1–R5 pass.
- Register a new immutable domain `codeclew-e04-r1` plus a preregistered series
  nonce.
- Derive new seeds and commitments after the freeze. Never reuse S0 tasks for a
  decision-bearing result.
- Materialize 42 fresh tasks; complete blind annotation and independent freeze
  verification before opening any arm.
- Run a zero-token preflight across all 42 fresh agent repositories: clean Git
  HEAD, source digest, build model, AST index, non-help Codeclew result, cache
  availability and controller isolation must all pass before model arms.

### R7. Live-run circuit breaker

The full matrix remains 126 runs with all failures retained. The first two
complete task triplets are preregistered to span Gradle and Maven. Before
continuing past them, automatically stop the series on
infrastructure-only conditions:

- missing native telemetry;
- source mutation;
- runner/schema failure;
- required specialized tool never recognized;
- zero successful non-help tool executions in a specialized arm;
- cache/path/no-HEAD error;
- broken controller/public commitment.

Each specialized arm must also have at least one successful non-help tool
execution in those triplets.

Do not stop or repair based on semantic correctness. Per-run ceiling: one model
turn, at most eight action calls, 32 KiB model-visible context and 1 KiB goal.
Any overage is retained as a failed run.

### R8. Judge, independent audit and graph transition

Open controllers only after 126 immutable packets are closed. Publish semantic
scores separately from infrastructure validity and token/time metrics. An
independent agent recomputes every metric and verifies fallback policy from raw
events. Only an accepted E04-R1 receipt can evaluate `GE1`.

## 5. Exact repeat procedure

1. Implement and independently verify R1.
2. Implement R2–R4 and replay them against S0 only as diagnostics.
3. Pass R5 with at most three model calls.
4. Commit and freeze all inputs; record their digests.
5. Derive the new R1 seed domain and materialize fresh agent/controller trees.
6. Obtain two blind annotations and resolve nomenclature before arms.
7. Run the matrix in randomized paired order with the R7 circuit breaker.
8. Close 126 packets, then run the hidden judge.
9. Independently recompute metrics and apply GE1 exactly once.
10. Commit the human report, machine summary and content-addressed raw evidence
    manifest; preserve S0 separately as `INFRA_ERROR`.

This order spends product effort before benchmark scale, and limits the next
harness failure to three canary calls instead of another 126-run series.

## 6. Execution checkpoint: 2026-08-10

Status: `PAUSE / REJECT`. The stop condition fired during zero-token focused
testing. No canary, freeze, fresh seed derivation or model run was started.

Accepted progress:

- the corrected R1 plan, including worker-owned fixed-point closure, exact
  proof equality, anti-recipe invariants, the 9/14 coverage guard and the
  preflight/circuit breaker, received an independent `ACCEPT`;
- real Codex shell-envelope fixtures now demonstrate how S0 audit must parse
  commands and safe stderr redirection;
- the diagnostic harness self-test passes without model calls.

Rejected product prototype:

- every executable `ValueFlow` goal is still converted to the old
  `SemanticGoal::map_edge_with_context` recipe;
- the request still requires one caller-selected target root and one
  caller-selected oracle symbol/compilation;
- proof roles remain hardcoded to `contextProducer`, `transformer` and
  `valueEdge`;
- a goal containing only `BIND_UNIQUE` can reach the PTC binder and receive
  three role obligations that the goal never declared, because exact
  variable/arity correspondence is absent;
- tests cover renamed PTC rather than variables, operand domains, unseen
  compositions or the five-family/9-of-14 contour.

The product prototype is therefore retained only as an uncommitted rejected
experiment and must not be described as generic product progress.

Rejected preflight state:

- `project inspect --repo .` defaults to root `:/main`/`compileKotlin`, while
  multi-module generated repositories contain Kotlin in an included
  `:unit-*` module; Codeclew returns `::compileKotlin` not found;
- populated `.e04-state`/`.semantic-thread` directories live inside the Git
  checkout and are not ignored, so authority can reject the repository as
  dirty after tools run;
- consequently the harness delta is not yet safe to freeze or commit as a
  complete R1 runner.

Exact resume point:

1. Introduce typed variables and explicit operator operands/arity/domains.
2. Make mandatory closure and proof equality variable-aware; demonstrate that
   bare `BIND_UNIQUE` cannot synthesize undeclared roles.
3. Remove the `ValueFlow -> map_edge_with_context` conversion and hardcoded PTC
   proof roles; pass one unseen-composition test without product-code changes.
4. Move tool state outside the committed checkout or explicitly exclude it
   from authority cleanliness without excluding source/config inputs.
5. Make preflight enumerate/select the actual Kotlin compilation for each
   project/module and recheck Git cleanliness after every tool.
6. Re-run only zero-token focused tests and independent review. Canaries remain
   forbidden until all six corrections pass.
