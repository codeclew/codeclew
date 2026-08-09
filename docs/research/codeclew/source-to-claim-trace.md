# R01 source-to-claim trace

Frozen against `source-manifest.json` and the cumulative plan digest
`83933d98913af3c4b016f674f73b76af3cfe4db190e30294ebb469d6d6cd6f93`.
This is a planning/provenance artifact, not execution evidence.

## Evidence and gate rules

- `S0` is the approved technical baseline. `S1` and `S5` contain research,
  literature summaries, inferences, hypotheses and design decisions. `S2` is
  historical planning evidence; `S3` proves planning coverage only. `S4` is a
  research request, not evidence that an answer is true.
- No claim in this file can by itself pass a product or experiment gate. A gate
  requires the destination node's measured packet and independent receipt.
- `UNVERIFIED` bibliography items and all opaque `turn…` citations are retained
  for provenance only and are never gate-eligible.
- `SOURCE_GROUNDED` means “faithfully represented from an approved source”, not
  “empirically proven”. `HYPOTHESIS` and `DESIGN_DECISION` remain falsifiable.
- `UNKNOWN_NOT_RUN` must be emitted downstream when a destination did not run;
  absence is never interpreted as success.

## Canonical source claims (source → claim → gap/hypothesis/destination)

| Claim | Approved source location | Evidence class | Frozen assertion/boundary | Owner hypotheses | Open gap / falsifier | Destination nodes | Gate use |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SC-001` | S0 §§2–3, 35–36 | `SOURCE_GROUNDED` baseline | Source/build artifacts remain authoritative; Kotlin Gradle and single-module Maven are the supported baseline, with fail-closed transaction safety. | H01, H02, H06 | `GP-001`, `GP-002`, `GP-006` | R03, K01, K02, E05, E06, GK, GES, Q03, GF | Baseline input only; requires clean-checkout verification. |
| `SC-002` | S1 §§13–16, 20 | `RESEARCH_DECISION` | Universal semantic editing must proceed corpus-first, then proof/refusal binding, then PSI materialization and paired evaluation. | H04, H05, H06 | `GP-004`, `GP-005`, `GP-006` | D01, D02, E01–E07, GE1, GES, GE2, GF | Not a gate result. |
| `SC-003` | S1 §§7, 9–10 | `DESIGN_DECISION` | Completeness is family-relative (`COMPLETE_FOR`); unknown oracle/boundary must produce refusal, never global completeness. | H04, H06 | `GP-004`, `GP-006` | D02, E01, E02, E03, E04, GE1, GF | Requires withheld proof/refusal evidence. |
| `SC-004` | S1 §§6, 9–10 | `DESIGN_DECISION` | Typed constraints, obligations and certified strategies are preferred to repository recipes or model-authored textual patches. | H04, H06 | `GP-004`, `GP-006` | E01, E02, E03, E04, E05, GE1, GF | Requires binder/materializer evidence. |
| `SC-005` | S1 §12; S0 §§18–25 | `SOURCE_GROUNDED` + `DESIGN_DECISION` | Supported materialization must be PSI-native, preserve protected bindings/types/effects and use goal-wide CAS/recovery. | H06 | `GP-006` | E05, E06, GES, GF | Requires executable safety tests. |
| `SC-006` | S1 §11 | `DESIGN_DECISION` | Test-oracle ownership is explicit; absent business/external oracle is a refusal or external obligation, not self-confirming proof. | H12, H06 | `GP-012`, `GP-006` | D01, E01, E06, Q01, X00–X04, GF | Requires hidden mutant/routing evidence. |
| `SC-007` | S4 §§4, 7–8; S5 §§3, 146–295 | `DESIGN_DECISION` | The semantic model is a disposable, query-oriented, lossy projection and must not become a second implementation. | H01 | `GP-001` | R03, K01, GK, GF | Anti-duplication test required. |
| `SC-008` | S4 §§8–9, 19; S5 §§297–438 | `HYPOTHESIS` | Stable cross-snapshot identity, provenance, validity and dependency invalidation can prevent silent retargeting/stale use. | H02 | `GP-002` | K02, K03, M03, Q02, X02–X04, GF | Measured identity/invalidation evidence required. |
| `SC-009` | S4 §5; S5 §§179–239 | `HYPOTHESIS` | L0–L5 projections may bound model-visible context without losing required closure. | H03 | `GP-003` | K04, E07, X02, X03, X04, GF | Context/closure measurement required. |
| `SC-010` | S4 §§12–14; S5 §§619–892 | `HYPOTHESIS` | Semantic MVCC and typed conflict detectors may detect supported conflicts before Git/build. | H07 | `GP-007` | D03, M01, M02, M03, M04, M05, M06, GM, Q02, X02–X04, GF | Supported-class FN/FP/uplift measurement required. |
| `SC-011` | S4 §§10–14; S5 §§525–892 | `HYPOTHESIS` | Coordination should preserve parallelism and add less than the frozen overhead on independent tasks. | H08 | `GP-008` | M01–M06, GM, X02–X04, GF | Paired overhead/correctness evidence required. |
| `SC-012` | S4 §§25–26; S5 §§1254–1383 | `HYPOTHESIS` | Codeclew may reduce integration time, human interventions and tokens on conflict-heavy work while preserving correctness. | H09 | `GP-009` | D03, E07, M05, M06, X00–X04, GF | Preregistered crossover evidence required. |
| `SC-013` | S4 §§11–14; S5 §§674–812 | `DESIGN_DECISION` + `HYPOTHESIS` | Structured operations should resolve most supported coordination episodes; free dialogue must end in a typed artifact. | H10 | `GP-010` | M02, M05, M06, GM, X02–X04, GF | Episode-level measurement required. |
| `SC-014` | S4 §§9, 19; S5 §§352–488 | `HYPOTHESIS` | Dependency-tracked incremental views are plausible, but rebuild may be cheaper for large impact sets. | H11 | `GP-011` | K03, M03, R02, X02–X04, GF | Latency/freshness/state measurement required. |
| `SC-015` | S4 §15; S5 §§892–980 | `HYPOTHESIS` | Typed test-evidence links and calibrated criticality/routing may reduce selected-test cost without omission. | H12 | `GP-012` | Q01, X00, X01, X02, X03, X04, GF | Only X04 may decide H12. |
| `SC-016` | S4 §7; S5 §§473–523 | `DESIGN_DECISION` | Hybrid storage is an option, not a default; SQLite remains until a preregistered measured benefit justifies added services. | H13 | `GP-013` | R03, K03, X02–X04, GF | ADR must follow measurements. |
| `SC-017` | S4 §17; S5 §§1030–1097, Q25–Q26 | `BOUNDED_DESIGN_DECISION` | Cross-language threads/contracts/states/effects are an untested specification scaffold, never a universal transpiler; equivalence is bounded to declared observables. | H14 | `GP-014` | R01, GF, Z01 | R01 prose cannot pass H14; only optional post-GO Z01 measures it. |
| `SC-018` | S5 §§3, 19–55, Q32 | `RESEARCH_VERDICT` | `GO_BUILD_SEMANTIC_COORDINATION_MVP` at confidence 0.72 is provenance for a testable program, not proven product value. | H01–H14 | `GP-001`–`GP-014` | R01, K01, Q03, GF0, GF | Never a gate substitute. |
| `SC-019` | S2 T00–T23; S3 §§42–146 | `PLANNING_VERIFICATION` | S3 found S2 planning coverage complete after fixes, but explicitly did not prove implementation or baseline superiority. | H04, H05, H06, H12 | `GP-004`, `GP-005`, `GP-006`, `GP-012` | R01, R02, D01, D02, E01–E07, GE1, GES, GE2, X00–X04, GF | Coverage evidence only. |
| `SC-020` | S5 §§3, 57–124 | `LITERATURE` + `INFERENCE` | Long context helps one-shot search/summarization but does not by itself provide durable identity, provenance, invalidation or concurrency control. | H02, H03, H08 | `GP-002`, `GP-003`, `GP-008`, `GP-015` | K02, K03, K04, M01, X03, GF | Background only; opaque citations cannot support a gate. |

Range notation such as `E01–E07` is an inclusive enumeration, not an implicit
wildcard. The reverse index below expands every destination family.

## Gap register embedded in this trace (gap → claim/hypothesis/destination)

| Gap | Unknown or falsifier to close | Claims | Hypotheses | Evidence owners |
| --- | --- | --- | --- | --- |
| `GP-001` | Orphan facts, non-rebuildable state, or enough duplicated transition logic to recreate the product. | SC-001, SC-007, SC-018 | H01 | R03, K01, GK, GF0, GF |
| `GP-002` | Silent retargeting, ambiguous publication, or stale fact used as fresh. | SC-001, SC-008, SC-020 | H02 | K02, K03, M03, Q02, X02–X04, GF0, GF |
| `GP-003` | Context scales with LOC, exceeds cap, or closure/provenance is truncated. | SC-009, SC-020 | H03 | K04, E07, X02–X04, GF0, GF |
| `GP-004` | Low applicability, wrong binding, decoy sensitivity, fallback search, or false completeness. | SC-002, SC-003, SC-004, SC-019 | H04 | D01, D02, E01–E04, GE1, M05, X00–X04, GF0, GF |
| `GP-005` | Editing correctness is inferior or cost/token thresholds and confidence bounds are missed. | SC-002, SC-019 | H05 | R02, D02, E07, GE2, X02–X04, GF0, GF |
| `GP-006` | Textual recipe remains necessary, must-refuse publishes, false commit occurs, or CAS/recovery fails. | SC-001–SC-006, SC-019 | H06 | D02, E01–E06, GES, Q03, X02–X04, GF0, GF |
| `GP-007` | Supported conflict FN, FP above threshold, or detection uplift below threshold. | SC-010 | H07 | D03, M02, M04, M06, GM, Q02, X02–X04, GF0, GF |
| `GP-008` | Independent-task overhead/CI exceeds threshold, work serializes, or correctness regresses. | SC-011, SC-020 | H08 | R02, M01–M06, GM, Q03, X02–X04, GF0, GF |
| `GP-009` | Conflict-heavy integration/human/token benefit is absent or correctness/safety regresses. | SC-012 | H09 | R02, D03, E07, M05, M06, X00–X04, GF0, GF |
| `GP-010` | Too many resolutions remain free-text-only or fail to materialize a typed decision. | SC-013 | H10 | R02, M02, M05, M06, GM, Q03, X02–X04, GF0, GF |
| `GP-011` | Update/notification p95 misses 5 s, stale active facts remain, or state/operations cost dominates. | SC-014 | H11 | R02, K03, M03, X02–X04, GF0, GF |
| `GP-012` | Supported test routing misses, mutation survives, self-confirming oracle appears, or score is presented as probability. | SC-006, SC-015, SC-019 | H12 | R02, D01, E06, Q01, X00–X04, GF0, GF |
| `GP-013` | Added graph/OWL service lacks a measured ≥25% benefit or exceeds operations/state budget. | SC-016 | H13 | R02, R03, K03, X02–X04, GF0, GF |
| `GP-014` | Scaffold overclaims arbitrary behavioral/concurrency equivalence or behaves as a transpiler. | SC-017 | H14 | R01, X03, GF0, GF, Z01 |
| `GP-015` | S5's 46 opaque `turn…` citations lack recoverable title→primary-locator mapping. | SC-020 | H02, H03, H08 | R01, X03, GF; future bibliography refresh only |
| `GP-016` | S4/S5 bytes are digest-bound at external absolute paths but are not archived in the repository. | SC-007–SC-018 | H01–H14 | R01, GF; any relocation requires a new approved source tuple |

## Hypothesis reverse map (hypothesis → source claims/gaps/destinations)

| Hypothesis | Claims | Gaps | Destination/decision path |
| --- | --- | --- | --- |
| `H01` | SC-001, SC-007, SC-018 | GP-001, GP-016 | R03 → K01 → GK → GF |
| `H02` | SC-001, SC-008, SC-018, SC-020 | GP-002, GP-015, GP-016 | K02/K03/M03 → X02/X03/X04 → GF |
| `H03` | SC-009, SC-018, SC-020 | GP-003, GP-015, GP-016 | K04/E07 → X02/X03/X04 → GF |
| `H04` | SC-002, SC-003, SC-004, SC-018, SC-019 | GP-004, GP-016 | D01/D02 → E01–E04 → GE1 → GF |
| `H05` | SC-002, SC-018, SC-019 | GP-005, GP-016 | E07 → GE2 → X02/X03/X04 → GF |
| `H06` | SC-001–SC-006, SC-018, SC-019 | GP-006, GP-016 | E01–E06 → GES → GF |
| `H07` | SC-010, SC-018 | GP-007, GP-016 | D03/M04/M06 → GM → X02/X03/X04 → GF |
| `H08` | SC-011, SC-018, SC-020 | GP-008, GP-015, GP-016 | M01–M06 → GM → X02/X03/X04 → GF |
| `H09` | SC-012, SC-018 | GP-009, GP-016 | D03/M05/M06 → X02/X03/X04 → GF |
| `H10` | SC-013, SC-018 | GP-010, GP-016 | M02/M05/M06 → GM → X02/X03/X04 → GF |
| `H11` | SC-014, SC-018 | GP-011, GP-016 | R02/K03/M03 → X02/X03/X04 → GF |
| `H12` | SC-006, SC-015, SC-018, SC-019 | GP-012, GP-016 | X00 → Q01 → X01 → X02 → X03 → X04 → GF |
| `H13` | SC-016, SC-018 | GP-013, GP-016 | R03/K03 → X02/X03/X04 → GF |
| `H14` | SC-017, SC-018 | GP-014, GP-016 | R01 → GF; optional post-GO Z01 |

## S4 mandatory questions (question → claim/gap/hypothesis/destination)

| ID | Required question | Claims / gap | Hypotheses | Evidence destination |
| --- | --- | --- | --- | --- |
| `Q01` | What is the minimum durable semantic model? | SC-007 / GP-001 | H01 | R03, K01, GK, GF |
| `Q02` | How is duplication of the source program prevented? | SC-007 / GP-001 | H01 | K01, GK, GF |
| `Q03` | Where is the boundary between fact model and alternative program IR? | SC-007 / GP-001 | H01 | R03, K01, GK, GF |
| `Q04` | Which levels belong to the understanding pyramid? | SC-009 / GP-003 | H03 | K04, GF |
| `Q05` | How do horizontal architecture layers relate to vertical threads? | SC-009 / GP-003 | H03 | K04, GF |
| `Q06` | Which thread types are required? | SC-009 / GP-003 | H03 | K04, GF |
| `Q07` | What is authoritative source of truth? | SC-001, SC-007 / GP-001 | H01, H06 | R03, M01, E06, GF |
| `Q08` | Which storage technology fits best? | SC-016 / GP-013 | H13 | R03, X03, GF |
| `Q09` | Are OWL/RDF needed, and for which part? | SC-016 / GP-013 | H13 | R03, X03, GF |
| `Q10` | How are uncertainty and incompleteness represented? | SC-003, SC-007 / GP-001, GP-004 | H01, H04 | K01, K04, E02, M04, GF |
| `Q11` | How is the model updated incrementally? | SC-008, SC-014 / GP-002, GP-011 | H02, H11 | K03, M03, GF |
| `Q12` | What is stored in an agent session? | SC-010, SC-011 / GP-008 | H08 | M01, GF |
| `Q13` | What is semantic scope? | SC-010, SC-013 / GP-007, GP-010 | H07, H10 | M01, M02, M05, GF |
| `Q14` | When are claims, leases, locks or optimistic MVCC used? | SC-010, SC-013 / GP-007, GP-010 | H07, H10 | M01, M02, M05, GF |
| `Q15` | Which semantic conflicts can be detected reliably? | SC-010 / GP-007 | H07 | D03, M04, GM, GF |
| `Q16` | Which conflicts inherently need a human or model? | SC-010, SC-013 / GP-007, GP-010 | H07, H10 | D03, M04, GM, GF |
| `Q17` | How should agents negotiate? | SC-013 / GP-010 | H10 | M02, M05, GF |
| `Q18` | Is free agent dialogue needed, or is protocol enough? | SC-013 / GP-010 | H10 | M02, M05, X03, GF |
| `Q19` | How are parent agent and subagents coordinated? | SC-013 / GP-010 | H10 | M02, M05, X03, GF |
| `Q20` | How are long refactoring transactions supported? | SC-008, SC-010 / GP-002, GP-007 | H02, H07 | Q02, GF |
| `Q21` | Can semantic criticality be computed automatically? | SC-015 / GP-012 | H12 | Q01, X04, GF |
| `Q22` | How are threads connected to tests? | SC-015 / GP-012 | H12 | Q01, E06, X04, GF |
| `Q23` | Which test obligations can be generated? | SC-006, SC-015 / GP-012 | H12 | Q01, E06, X04, GF |
| `Q24` | How is the model used for a large refactor? | SC-008, SC-010 / GP-002, GP-007 | H02, H07 | Q02, GF |
| `Q25` | How is it used for migration between languages? | SC-017 / GP-014 | H14 | R01, GF, Z01 |
| `Q26` | Which migration properties can be proved? | SC-017 / GP-014 | H14 | R01, GF, Z01 |
| `Q27` | What does large LLM context make redundant? | SC-009, SC-020 / GP-003, GP-015 | H03 | K04, E07, X03, GF |
| `Q28` | What can large context not replace? | SC-008, SC-020 / GP-002, GP-015 | H02, H03 | K04, E07, X03, GF |
| `Q29` | When does Codeclew lose to Git and `rg`? | SC-002, SC-012 / GP-005, GP-009 | H05, H09 | E07, M06, X03, GF |
| `Q30` | When does semantic coordination pay off? | SC-012 / GP-009 | H09 | E07, M06, X03, GF |
| `Q31` | Which MVP has maximum information gain? | SC-007, SC-010, SC-012 / GP-001, GP-007, GP-009 | H01, H07, H09 | K01, M06, GF |
| `Q32` | What is the final research verdict? | SC-018 / GP-001–GP-014 | H01–H14 | R01, GF0, GF |

## S4/S5 required deliverables (deliverable → claim/gap/destination)

| ID | Required deliverable | Claims / gaps | Destination and honest terminal disposition |
| --- | --- | --- | --- |
| `D01` | Landscape review | SC-018, SC-020 / GP-015 | R01 archive/trace; GF exact status |
| `D02` | Strong critique | SC-018 / GP-001–GP-014 | R01 gap/falsifier trace; GF exact status |
| `D03` | Semantic pyramid RFC | SC-009 / GP-003 | K04; GF |
| `D04` | Formal semantic fact model | SC-007 / GP-001 | K01; GF |
| `D05` | Thread taxonomy | SC-009 / GP-003 | K04; GF |
| `D06` | Agent session model | SC-011 / GP-008 | M01; GF |
| `D07` | Semantic scope and claims model | SC-010, SC-013 / GP-007, GP-010 | M02; GF |
| `D08` | Conflict taxonomy and detection matrix | SC-010 / GP-007 | M04, D03; GF |
| `D09` | Coordination protocol specification | SC-013 / GP-010 | M02, M03; GF |
| `D10` | Streaming and incremental architecture | SC-014 / GP-011 | K03, M03; GF |
| `D11` | Storage alternatives matrix | SC-016 / GP-013 | R03 measured ADR; GF |
| `D12` | Human interaction and visualization concept | SC-018 / GP-008, GP-010 | R01 synthetic prototype, Q03 validation; GF |
| `D13` | Criticality and test-evidence model | SC-015 / GP-012 | Q01; GF |
| `D14` | Refactoring workflow specification | SC-008, SC-010 / GP-002, GP-007 | Q02; GF |
| `D15` | Cross-language migration model | SC-017 / GP-014 | R01 bounded scaffold; GF; optional Z01 |
| `D16` | Security and governance model | SC-001 / GP-006 | Q03; GF |
| `D17` | MVP architecture RFC | SC-007–SC-016 / GP-001–GP-013 | R03, K01–K04, M01–M05, GK, GM; GF |
| `D18` | Benchmark corpus specification | SC-002, SC-012 / GP-004, GP-007–GP-009 | D01–D03, X00, X01; GF |
| `D19` | Evaluation protocol | SC-002, SC-012 / GP-005, GP-007–GP-012 | R02, M05, X02, X03; GF |
| `D20` | Risk register | SC-018 / GP-001–GP-016 | R01 plus node falsifiers; GF |
| `D21` | Decision with confidence and falsifiers | SC-018 / GP-001–GP-014 | GE1, GE2, GM, GF0, GF |
| `D22` | First five implementation commits if verdict permits | SC-018 / GP-001–GP-014 | K01, D01, D02, D03, K02 after approval/gates; GF |

## Historical S2 obligations (old task → claims/gaps/new destinations)

| Old task | Frozen obligation | Claims / gaps | New destination(s) |
| --- | --- | --- | --- |
| `T00` | Gap register and decision contract | SC-018, SC-019 / GP-001–GP-016 | R01, gates, GF |
| `T01` | Telemetry and run schemas | SC-019 / GP-005, GP-008–GP-013 | R02, M05, GF |
| `T02` | Deterministic corpus generator | SC-002 / GP-004 | D01, GF |
| `T03` | Hidden manifest and oracle isolation | SC-002, SC-006 / GP-004, GP-012 | D01, X00, X01, GF |
| `T04` | Structural variation and decoys | SC-002, SC-003 / GP-004 | D01, D02, GF |
| `T05` | Three data-flow families | SC-002 / GP-004 | D02, GF |
| `T06` | Persistence and lifecycle families | SC-002, SC-006 / GP-004, GP-006, GP-012 | D02, E02, E05, GF |
| `T07` | Freeze corpus and target population | SC-002 / GP-004, GP-005 | D02, X00, X01, GF |
| `T08` | Goal/Obligation/Proof schemas | SC-003, SC-004 / GP-004, GP-006 | K01, E01, GF |
| `T09` | Goal-wide and multi-root evidence | SC-003, SC-005 / GP-004, GP-006 | E02, E06, M03, GF |
| `T10` | Obligation closure and `COMPLETE_FOR` | SC-003 / GP-004 | E02, GF |
| `T11` | Binding primitives | SC-004 / GP-004 | E01, E02, GF |
| `T12` | `MAP_EDGE_WITH_CONTEXT` binder | SC-003, SC-004 / GP-004 | E03, GF |
| `T13` | Existing typed-field proof adapter | SC-004 / GP-004, GP-006 | E03, GF |
| `T14` | Blind binder experiment | SC-002–SC-004 / GP-004 | E04, GE1, GF |
| `T15` | PSI protocol | SC-005 / GP-006 | E05, GF |
| `T16` | Kotlin PSI operations | SC-005 / GP-006 | E05, GF |
| `T17` | Oracle/mutation policy | SC-006, SC-015 / GP-006, GP-012 | E06, Q01, GF |
| `T18` | One proven family materialization | SC-002, SC-005 / GP-006 | E05, E06, GF |
| `T19` | Concurrency/recovery/full validation | SC-005, SC-008 / GP-002, GP-006 | E06, GES, K03, M01, M02, M03, Q02, GF |
| `T20` | Full end-to-end harness | SC-002, SC-012 / GP-005, GP-009 | E07, M05, GF |
| `T21` | Paired comparative series | SC-002, SC-012 / GP-005, GP-007–GP-010 | E07, M06, X02, GF |
| `T22` | Statistics and blind audit | SC-012, SC-015 / GP-005, GP-007–GP-013 | X03, X04, GF |
| `T23` | Final product/architecture decision | SC-018 / GP-001–GP-014 | GE2, GM, GF0, GF |

## Destination reverse index (destination → source claims/gaps/mandatory IDs)

This is the reverse direction for every producer named above. A range is
inclusive and all IDs in the range are mandatory.

| Destination | Claims / gaps consumed | Mandatory questions | Deliverables | Historical tasks |
| --- | --- | --- | --- | --- |
| `R01` | SC-017–SC-019; GP-014–GP-016 | Q25, Q26, Q32 | D01, D02, D12, D15, D20 | T00 |
| `R02` | SC-014, SC-019; GP-005, GP-008–GP-013 | — | D19 | T01 |
| `R03` | SC-001, SC-007, SC-016; GP-001, GP-013 | Q01, Q03, Q07–Q09 | D11, D17 | — |
| `K01` | SC-001, SC-007, SC-018; GP-001 | Q01–Q03, Q10, Q31 | D04, D17, D22 | T08 |
| `K02` | SC-001, SC-008, SC-020; GP-002 | — | D17, D22 | — |
| `K03` | SC-008, SC-014, SC-016, SC-020; GP-002, GP-011, GP-013 | Q11 | D10, D17 | T19 |
| `K04` | SC-009, SC-020; GP-003 | Q04–Q06, Q10, Q27, Q28 | D03, D05, D17 | — |
| `GK` | SC-001, SC-007; GP-001 | Q01–Q03 | D17 | — |
| `D01` | SC-002, SC-006, SC-019; GP-004, GP-012 | — | D18, D22 | T02–T04 |
| `D02` | SC-002, SC-003, SC-019; GP-004–GP-006 | — | D18, D22 | T04–T07 |
| `D03` | SC-010, SC-012; GP-007, GP-009 | Q15, Q16 | D08, D18, D22 | — |
| `E01` | SC-002–SC-004, SC-006, SC-019; GP-004, GP-006 | — | — | T08, T11 |
| `E02` | SC-002–SC-004, SC-019; GP-004, GP-006 | Q10 | — | T06, T09–T11 |
| `E03` | SC-002–SC-004, SC-019; GP-004, GP-006 | — | — | T12, T13 |
| `E04` | SC-002–SC-004, SC-019; GP-004, GP-006 | — | — | T14 |
| `E05` | SC-001, SC-002, SC-004, SC-005, SC-019; GP-006 | — | — | T06, T15, T16, T18 |
| `E06` | SC-001, SC-002, SC-005, SC-006, SC-019; GP-006, GP-012 | Q07, Q22, Q23 | — | T09, T17–T19 |
| `E07` | SC-002, SC-009, SC-012, SC-019; GP-003, GP-005, GP-009 | Q27–Q30 | — | T20, T21 |
| `GE1` | SC-002–SC-004, SC-019; GP-004 | — | D21 | T14 |
| `GES` | SC-001, SC-002, SC-005, SC-019; GP-006 | — | — | T19 |
| `GE2` | SC-002, SC-019; GP-005 | — | D21 | T23 |
| `M01` | SC-010, SC-011, SC-020; GP-008 | Q07, Q12–Q14 | D06, D17 | T19 |
| `M02` | SC-010, SC-011, SC-013; GP-007, GP-008, GP-010 | Q13, Q14, Q17–Q19 | D07, D09, D17 | T19 |
| `M03` | SC-008, SC-010, SC-011, SC-014; GP-002, GP-008, GP-011 | Q11 | D09, D10, D17 | T09, T19 |
| `M04` | SC-010, SC-011; GP-007, GP-008 | Q10, Q15, Q16 | D08, D17 | — |
| `M05` | SC-010–SC-013; GP-004, GP-008–GP-010 | Q13, Q14, Q17–Q19 | D17, D19 | T01, T20 |
| `M06` | SC-010–SC-013; GP-007–GP-010 | Q29–Q31 | — | T21 |
| `GM` | SC-010, SC-011, SC-013; GP-007, GP-008, GP-010 | Q15, Q16 | D17, D21 | T23 |
| `Q01` | SC-006, SC-015; GP-012 | Q21–Q23 | D13 | T17 |
| `Q02` | SC-008, SC-010; GP-002, GP-007 | Q20, Q24 | D14 | T19 |
| `Q03` | SC-001, SC-018; GP-006, GP-008, GP-010 | — | D12, D16 | — |
| `X00` | SC-006, SC-012, SC-015, SC-019; GP-004, GP-009, GP-012 | — | D18 | T03, T07 |
| `X01` | SC-006, SC-012, SC-015, SC-019; GP-004, GP-009, GP-012 | — | D18 | T03, T07 |
| `X02` | SC-006, SC-008–SC-016, SC-019; GP-002–GP-013 | — | D19 | T21 |
| `X03` | SC-006, SC-008–SC-016, SC-019, SC-020; GP-002–GP-015 | Q08, Q09, Q18, Q19, Q27–Q30 | D19 | T22 |
| `X04` | SC-006, SC-008–SC-016, SC-019; GP-002–GP-013 | Q21–Q23 | — | T22 |
| `GF0` | SC-018; GP-001–GP-014 | Q32 | D21 | T23 |
| `GF` | SC-001–SC-020; GP-001–GP-016 | Q01–Q32 | D01–D22 | T00–T23 |
| `Z01` | SC-017; GP-014 | Q25, Q26 | D15 | — |

## Bibliography linkage and unsupported-gate audit

- Primary-locator candidates are locked in `bibliography-lock.json` as
  `B01`–`B18`; only entries explicitly marked `RESOLVED_PRIMARY` are treated as
  resolved, and their downloaded bytes are hashed when retrievable.
- `B03`–`B05`, `B16`, `B19`–`B20`, and every opaque citation listed in
  `opaqueCitations` are `UNVERIFIED` and `gateEligible: false`.
- The canonical claims above intentionally do not cite an opaque identifier as
  sole support. `SC-020` retains the literature inference but routes its open
  provenance problem to `GP-015` and remains background-only.
- Therefore there is no path `UNVERIFIED bibliography → gate PASS`. The only
  path to a hypothesis decision is a measured destination packet plus an
  independent receipt; missing evidence yields `UNKNOWN_NOT_RUN` or an
  inconclusive/stop branch.

## Mechanical coverage receipt

Expected exact identifier sets:

```text
questions:    Q01..Q32  (32)
deliverables: D01..D22  (22)
old tasks:    T00..T23  (24)
hypotheses:   H01..H14  (14)
claims:       SC-001..SC-020 (20)
gaps:         GP-001..GP-016 (16)
```

Forward coverage is provided by the four identifier tables; reverse coverage
is provided by the hypothesis, gap and destination indexes. Any future checker
must expand inclusive ranges and fail on a missing, extra, duplicated or
unparseable mandatory identifier.
