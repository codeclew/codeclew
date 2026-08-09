# Codeclew gate-efficiency log

Every accepted or terminal node must append one row before its outgoing edge is
used. Missing native token telemetry is recorded as `UNAVAILABLE`, never
estimated from bytes.

| Node | Evidence gained | Gate value | Native tokens | Tool/rework signal | Decision | Change to next execution |
| --- | --- | --- | --- | --- | --- | --- |
| `A00` | Read-only current-session view contains explicit user approval (`item-1163`, reinforced by `item-1182` and `item-1195`); exact message hashes are recorded in the approval bundle. | A human stop before implementation is useful. RSA host attestation was unavailable and did not protect any in-scope research result. | `UNAVAILABLE` | High rework: an attempted RSA replacement produced no research evidence and was abandoned on explicit user clarification. | `SIMPLIFY` | Future A00 checks: one session read, exact `USER` message record, plan digest check, then continue. No cryptographic identity project inside Codeclew research. |
| `R01` | Exact S0–S5 provenance; H01–H14/gap/claim/Q/D/T trace; gate-ineligible unresolved literature; bounded cross-language and evidence-view scaffolds. | The independent audit caught stale-plan/false-HEAD provenance and asymmetric trace links. Targeted retry proved both repairs and preserved four prior PASS checks. | `UNAVAILABLE` | Producers: 44 calls. Verifier: 33 + 9 calls; retry wait ~15 s versus ~41 s initial. Team total 86 exceeds frozen ceiling 60. | `KEEP/NARROW`, node `BUDGET_EXCEEDED` | Reuse exact mechanical checks; never repeat full audit after a localized repair. The approved DAG closes R02/R03 and sends the exhausted foundation result to GK. |
| `GK` | Exact exhausted set `{R01}`, retained primary/secondary causes, deterministic `INCONCLUSIVE_FOUNDATION`, proof that implementation stays closed. | The gate prevents a budget-exhausted foundation from silently unlocking R02/R03 while preserving useful R01 facts. | `UNAVAILABLE` | Producer 4 + verifier 4 = 8 calls, below ceiling 20; verifier wait ~0.5 s. | `KEEP`, terminal-only | Reuse this compact set/digest/edge check at GF0; do not reread R01 content or add repair work. |
| `GF0/GF` | Exact terminal mapping; H01–H14; answers 1–32; deliverables 1–22; bounded final claim and remaining unknowns. | Fresh audit caught a real status-enum mismatch that would break downstream interpretation; all semantic claims, refs, browser paths and terminal causes otherwise passed. | `UNAVAILABLE` | Initial audit 26 calls/~29.8 s; enum-only retry 2 calls/~0.2 s. | `KEEP/NARROW` | Preserve one fresh terminal audit, but retries may check only changed machine invariants. Do not rerun browser or evidence traversal after enum-only repair. |

## Per-node review contract

For each next node record:

1. accepted evidence delta and which hypothesis/gap it changes;
2. native input/cached/output/noncached tokens when available;
3. wall time, tool calls, retries, duplicated reads and discarded artifacts;
4. concrete failure prevented by the gate, if any;
5. verdict `KEEP`, `SIMPLIFY`, `REMOVE`, `NARROW`, or `STOP`;
6. one executable change applied to the following node.

A gate that adds no new fact, catches no real defect, and only restates an
already accepted predecessor must be simplified or removed before the next
wave. Safety/correctness gates may remain expensive only with a documented
counterexample they prevent.
