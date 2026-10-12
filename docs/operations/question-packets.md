# Question packets over retained threads

`clew thread question --thread THREAD --request request.json` returns a bounded,
read-only packet over an existing `thread flow`. It loads and verifies the
retained flow and its parent callable facts. It starts no compiler, model,
target program, or network request, and publishes no new managed root.

This is a conservative callable-envelope profile, not a universal program
slice. Presentation methods and semantic relevance have different granularity.
The packet refers to the existing source/CFG/support objects; it does not copy
method bodies into each question. Its lifetime and source authority depend on
the retained parent. Saving the JSON output does not retain that parent.

## Request

Use the exact `flowId` and `sliceRef.digest` from the retained flow result.
Criteria use callable identities, relation fact IDs, or local CFG region/node
IDs. Labels and purpose are declared questions; a label such as "Kafka delivery"
cannot turn a call fact into delivery evidence.

Relation selectors resolve both graph-edge supports and boundary-only supports.
Multiple retained representations of the same selected fact are returned as
evidence locations, not treated as separate runtime calls. Presentation gives
matched owners priority, then the root and the remaining canonical envelope.
Boundary-only supports do not always project an exact owner identity; when
that identity is unavailable, presentation falls back to the root/envelope
rather than guessing ownership from source coordinates or method names.

```json
{
  "schema": "codeclew-question-request/0.1",
  "flowId": "thread-flow:sha256:<parent-id>",
  "flowDigest": "sha256:<parent-CAS-digest>",
  "purpose": "CHARACTERISTICS",
  "criteria": [
    {
      "id": "selected-call",
      "label": "Inspect the selected external-call occurrence",
      "selector": {"kind": "RELATION_FACT", "factId": "<retained-use-fact-id>"}
    }
  ],
  "maxMethods": 32,
  "localOrderQueries": []
}
```

Other selectors are `{"kind":"CALLABLE","memberAlias":"member", "symbolIdentity":"<exact identity>"}`
and `{"kind":"CFG_NODE","regionId":"<retained region>","nodeId":3}`.
Purpose is `PROBLEMS`, `CHARACTERISTICS`, or `PROCESS`. It does not change facts
or upgrade authority. Request and result are limited to 64 KiB each; there are
at most 32 criteria, 32 local order queries and 128 presentation methods.
Unknown JSON fields, duplicate keys, duplicate criterion IDs, unsafe text and
foreign flow bindings are rejected.

## Completion and uncertainty

`extraction` carries the parent fact-set coverage, certainty and obligations.
`traversal` carries the flow traversal status. `presentationComplete` states
only whether the method presentation budget included the retained callable
envelope. A flow can be `COMPLETE` while semantic boundaries remain.

`FOUND_IN_RETAINED_FLOW` establishes a selector match, not complete discovery
across an unbounded program. Missing and ambiguous selectors yield explicit
unknown results. Every criterion has `semanticClosure: UNKNOWN` in this first
profile: general value, alias, framework and effect transfer models are not
qualified. No packet proves absence of bugs or absent runtime behavior.

All parent boundaries are carried into `frontier`, conservatively affecting
every criterion. Presentation omissions are explicit budget cuts, never
irrelevance certificates. All retained relation and control-region IDs remain
referenceable; selected method bodies remain in the parent evidence. Criteria,
references and boundaries have canonical ordering. The packet ID binds the
request, exact parent CAS reference, policy, coverage and result. The library
can verify a packet by deterministic reconstruction from the verified parent.

## Local CFG observations

An optional query has `id`, `regionId`, `before` and `after`. The result reports
`mayReachInActiveCfg` and `strictlyDominatesInActiveCfg`. Active topology excludes
explicit DEAD nodes/edges, and includes loops and exceptional transitions.
Dominance means every active entry-to-after graph path visits before; it does
not establish a feasible execution, transaction order, or per-iteration order.
Missing/dead/unreachable nodes produce unknown observations. Identical nodes
do not strictly dominate themselves for this query.

Existing explanation `OrderedBefore` retains its original graph-reachability
meaning. The new observations do not strengthen that predicate or interpret
CFG labels as business conditions.

## Next qualification slices

`callableEnvelopeSummaries` expose each presented method's outgoing relations,
boundary targets, CFG region references and exact source CAS references. These
are structural interfaces, not value/transaction/I/O contracts. They inherit
the packet's global frontier. They cannot replace an unknown callee transfer.

`interpretationPlan` deduplicates source CAS references, reports unique source
bytes (not token counts), and selects one shared worker because this profile
has no qualified independent relevance cuts. It plans at most two model calls
and one expansion round, but makes zero calls itself. Zero duplicated source
bytes describes the proposed shared input, not measured model savings. A full
source CAS object may contain more than one relevant method; consumers should
use the parent anchors for bounded source delivery. Source-object byte totals
are therefore upper estimates of referenced object bytes, not model tokens.

Local graph observations are usable now. General reaching definitions,
immutable-local/parameter/default transfer, matched interprocedural contexts,
heap state, qualified exception/termination summaries and physical I/O costs
remain outside this profile. Existing narrow FIR value relations must be
qualified individually before extending semantic closure. Declared transport
ports remain boundaries until routing/configuration/contract evidence is added.

Interpretation can reuse the existing `thread explain` protocol against the
same parent. Model labels remain inferred; structural validators check their
support without proving arbitrary prose. No paid experiment is part of this
command. Scheduler experiments should compare equal coverage and preserve
cross-question frontier obligations before evaluating latency or token cost.
