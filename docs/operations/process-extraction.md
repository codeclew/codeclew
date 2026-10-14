# Bounded process extraction

`clew docs process extract --root ROOT --snapshot HANDLE --input QUERY.json
--output PACKET.json` reads a retained immutable Check and writes a query packet.
It does not launch a compiler, model, target program or service request. It does
not save a process declaration or automatically accept business meaning.

```json
{
  "purpose": "PROCESS",
  "service": "example",
  "rootDeclaration": "example:symbol:exact-retained-id",
  "configurationScope": "UNSPECIFIED",
  "outcomeEventIds": [],
  "budgets": {"maxMethods": 32, "maxEvents": 256, "maxSourceBytes": 262144}
}
```

Root selection is by an exact scoped callable observation, not an identifier
search. Purposes are PROCESS, PROBLEMS or CHARACTERISTICS. Outcomes are exact
retained FLOW IDs, not natural-language labels. `configurationScope` records the
requested analysis context; its text is not runtime configuration evidence.

## Algorithm and authority

The extractor reuses the portable documentation process graph's service/scope
association policy. It indexes retained callable and FLOW observations, expands
known local calls in stable breadth-first order, and preserves every retained
event for each included method. Recursion reaches a finite visited-set fixed
point. Ambiguous ownership, external calls, provider limits and unavailable
events remain frontiers. A unique retained local association is not proof of
dynamic dispatch or a cross-service relationship.

Before expanding a method, the collector checks method, event and distinct
source-object byte budgets. A cut retains the method ID and required expansion
as a BUDGET frontier; it does not certify irrelevance. Source revision and text
digests are checked. Output is limited to 1 MiB, input observations to 131,072,
methods to 64, events to 4,096 and selected source-object payloads to 1 MiB.
Sources stay in the Check; packets contain bounded references rather than source
bodies. Byte counts sum distinct source object payloads; overlapping source spans
are not deduplicated as physical code bytes.

Outcome queries compute a backward fixed point over retained call membership.
Policy `retained-process-envelope/0.2` uses a reverse adjacency worklist with
one bit per requested outcome (at most 32). A single-outcome query tries at most
two full scans before indexing; an exhausted or fully covered envelope returns
early. This preserves shallow-query costs while avoiding repeated global scans
on deep chains. Event ownership and event presence are indexed once.

Each outcome carries sorted `structuralMethodIds`; each method carries
`structuralOutcomeEventIds` in query order. The existing
`onStructuralOutcomePath` remains the union for older consumers. These additive
fields separate requested outcomes; they do not add a dependence model. Missing,
unbound or budget-cut outcomes have no structural members. Edges with missing
retained endpoints never create new members. The implementation digest now also
binds `process_reachability.rs`; old packets remain historical under their old
policy, and regenerated packets receive new digests.

This annotation is a structural outcome path, not a data/control/effect chop or
path-feasibility result. Methods outside that path are retained conservatively.
Missing outcomes are `NOT_FOUND_UNKNOWN`, including outcomes outside a budget
cut. `programSemanticClosure` is always false.

The packet records snapshot, input/context/service/root fact digests, extractor,
runtime mode, configuration scope, query budgets, policy/schema and an
implementation digest that also includes the graph association policy. Repeating
unchanged inputs yields the same packet digest. The managed immutable evidence
cache is reused; this profile does not add a second packet cache or an automatic
watcher. Expand by repeating the same snapshot/query with increased budgets.

There is one interpretation task with no model calls. Independent semantic cuts
have not been established; duplicating this envelope across agents would duplicate
input. Future parallel work needs independently supported interface summaries and
integration checks before its savings can be measured.

## Use in a saved process

`process.extraction` may contain the same query. Native rendering checks its
service and exact root against the captured saved-process selector and attaches
`processExtraction`. The reader shows retained decisions, return/throw events,
explicit outcomes and extraction frontiers. Source links bind the selected Check.
Markdown retains the packet and its provenance.

An authored `process.thread` remains the explanation; extraction supplements it.
Business names and selected effect labels require a separate proposal and review.
The source-bound thread presentation currently admits Kotlin/Java complete method
spans; the extraction packet consumes the common documentation observation model,
without assuming FIR. Synthetic Java, Kotlin and syntax-provider fixtures test
that contract; they do not qualify the corresponding compiler workers.

Runtime I/O counts, latency bounds, strict state machines, alias-sensitive effects
and business violations remain unknown in this profile. Program-wide absence of
errors cannot be inferred from this packet.
