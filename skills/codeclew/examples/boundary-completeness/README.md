# Boundary, domain and DataFlow exercise

This is synthetic owned source, not a private service example. Freeze this file,
the source and evaluator expectations before a model run. The interface contract
supplied to an executor declares VersionApi as GET `/active-version`; its body
and failure policy are not captured. Logical storage has no known vendor. Queue
send has no observed broker acknowledgement or consumer reply. Inference has an
unknown SDK transport. Never infer those missing facts from the interface names.

Executor request: document `reserve`, register its logical participants and
whole-service outgoing boundaries, distinguish required and optional inference,
declare the business Reservation concept separately from request representation,
and save a distinct typed DataFlow view. Read the source. Use the same request
with `debug=true`; task decisions must match. The required author is fresh Luna
Max. An independent reviewer receives the source and the frozen oracle, not just
the candidate's own inventory.

Evaluator expectations (author-generated, fixed before behavioral execution):

- Show external VersionApi call and returned nullable value. The local default
  belongs on the local service lane. No remote 404/timeout policy is evidenced.
- Show Store save only under `save`, with normal local API completion separated
  from commit/durability. Preserve uncaught failures; do not invent a catch.
- Queue submission belongs under `notifyQueue`, in background. No synchronous consumer
  reply, observed broker ack or guaranteed completion before return.
- SDK inference is submitted on both paths; only `requiredInference` joins it.
  A synchronous return on the optional path is an error.
- Reservation is the business identity/value under discussion; the source only
  supports its ID and quantity. It establishes no booking/payment lifecycle or
  exclusive service ownership. ReservationRequest is a transport representation.
- DataFlow is an independently linked `entity-dataflow/1.0` view of input,
  construction and effects, with explicit read/transform/write/transfer edges.
  A dependency graph or a renamed sequence is insufficient.

Negative controls: delete identity+registry+arrows together; replace the HTTP arrow
with a service self-call; return local fallback from VersionApi; claim storage
commit; add queue consumer reply; join optional inference; remove required join;
accept ReservationRequest as the domain overview; relabel dependency topology as
DataFlow. Each must fail its applicable mechanical or semantic criterion.

`boundary_coverage.py` can check reviewed IDs/endpoints/paired arrows and native
typed graphs. Its PASS does not evaluate these source semantics. Preserve first
model attempts and manual semantic verdicts separately. Until those executions
exist, model behavior and normal/debug equivalence are `NOT_RUN`.
