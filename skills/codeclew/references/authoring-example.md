# Source-bound authoring example

Start with proposal input, not a materialized Narrative. Replace each uppercase
reference with one returned and read in the current Work. The fixture in
`fixtures/durable-docs` and its CLI regression provide an executable example;
these placeholders are not executable evidence.

## Proposal input for docs proposal submit

The host gets subject, context and immutable evidence from `--work WORK_ID`.
The `entrypoint` below is that Work's authorable reference. Claims use Work-local
`evidence`, and sequence `steps` are nested. The corresponding installed-version
contract is `schemas/documentation/proposal.schema.json` in the product source.

```json
{
  "schema": "codeclew-documentation-proposal/1.0",
  "operations": [{
    "entrypoint": "ENTRYPOINT_REF",
    "title": "Check out an order",
    "summary": {"text": "Reject invalid quantities; otherwise request a reservation and return its result.", "evidence": ["METHOD_REF"]},
    "steps": [{
      "kind": "alt",
      "meaning": {"text": "Quantity is zero or negative", "evidence": ["IF_REF"]},
      "children": [{"kind": "return", "from": "SERVICE_ID", "to": "caller", "meaning": {"text": "Return invalid quantity", "evidence": ["REJECT_REF"]}}],
      "otherwise": [
        {"kind": "message", "from": "SERVICE_ID", "to": "SERVICE_ID", "meaning": {"text": "Delegate to reserve(request)", "evidence": ["CALL_REF"]}},
        {"kind": "return", "from": "SERVICE_ID", "to": "caller", "meaning": {"text": "Return the reservation result", "evidence": ["RETURN_REF"]}}
      ]
    }],
    "contracts": [{"title": "Quantity input", "kind": "payload", "rows": [{"label": "quantity validation", "description": {"text": "A value at or below zero is rejected before delegation.", "evidence": ["IF_REF", "REJECT_REF"]}}]}]
  }],
  "uncertainties": ["The selected body does not establish the downstream reservation algorithm or wire schema."]
}
```

Use the selected service's exact ID for `SERVICE_ID` in proposal step `from`/`to`.
The host supplies `caller` and selected services automatically; it materializes
service participants as `service-<id>`. Proposal `participants` declares only
additional actors backed by the selected evidence. Do not redeclare an implicit
actor to choose its label, copy participant IDs from another Work, or put the
materialized `service-<id>` alias into a proposal step that expects the service ID.

The example is complete only for its stated small source body. Real Work may
require other conditions, returns and evidence. Machine acceptance checks
structure and coverage, not whether the business explanation is true.

## Materialized Narrative output, not proposal input

The host assigns stable IDs and materializes Narrative 1.3. Do not construct
this output by renaming proposal fields or submit it as proposal input:

| Proposal input | Materialized Narrative |
| --- | --- |
| `entrypoint`: current Work reference | operation `id`: canonical target identity |
| claims with `evidence` references | claims with bound `dependencyIds` and `sourceIds` |
| nested `steps`, `children`, `otherwise` | ordered diagram `events` with assigned IDs |
| `contracts` and row `description` claims | `interfaceContracts` and source-bound rows |
| optional additional `participants` | implicit caller/services plus additional actors |
| host-bound `--work` | `subject` and `contextDigest` |

For retained source `if (quantity <= 0) return "invalid"; return reserve(request);`,
keep the rejecting return in one branch and delegation/result in the other.
Source evidence of delegation does not establish the downstream algorithm.

Materialized event kinds are `message`, `return`, `note`, `alt`, `else`, `loop`, `opt`, `end`,
and `declared`. A resolved cross-service request uses `kind: declared` and
participants with the two captured service IDs. Proposal input must use the
interaction reference delivered by a recorded read in that same immutable Work:

```json
{"kind":"declared","from":"orders","to":"inventory","interaction":"INTERACTION_REF","meaning":{"text":"Request a reservation.","evidence":["INTERACTION_REF","CALL_REF","RECEIVER_REF"]}}
```

For example, if the read delivers reference `d141` for dependency
`interaction:reserve-inventory`, replace `INTERACTION_REF` with `d141`. The host
then writes `interaction: reserve-inventory` in the materialized event. Neither
the catalogue ID `reserve-inventory` nor dependency ID
`interaction:reserve-inventory` is a proposal reference. References such as `d141`
are local to their Work; the same spelling in another Work can name different
evidence. Read and verify the returned record rather than copying a handle.

In service Work, query `DECLARED_INTERACTION` to obtain declarations whose
`from.service` or `to.service` is the selected service. Their visibility is
boundary evidence, not permission to read another service's implementation or
draw an executable arrow. A declared arrow requires scenario Work with an
actually composed transition, `RESOLVED` endpoints and call site, and an admitted
declaration. `SOURCE_MATCH` and `AMBIGUOUS` do not satisfy this requirement.
Bind the edge to the declaration and the returned call/receiver dependencies.
A corresponding HTTP reverse return also names that interaction. Kafka delivery
has no synchronous return; declare a separate reply-event interaction. Nest receiver
conditions within the caller's successful branch. Do not omit failures just
because the named scenario focuses on success.

## External HTTP and Kafka boundaries

An `external: true` interaction has no captured receiver. Keep its request or
publish behavior on the caller's lifeline using a source-bound `note`, and
describe the available contract and receiver limitation. Do not add the external
service as a captured participant or disguise a service transition as an actor
without a service. Do not claim receiver execution, delivery, retries, failure
handling, or a synchronous reply unless the selected source establishes that
specific caller behavior. Runtime remains `UNKNOWN`.

For a small body that delegates to an HTTP client, the proposal shape is:

```json
{
  "schema":"codeclew-documentation-proposal/1.0",
  "operations":[{
    "entrypoint":"ENTRYPOINT_REF",
    "title":"Request an external reservation",
    "summary":{"text":"Delegate the reservation request and return the client result.","evidence":["METHOD_REF"]},
    "steps":[
      {"kind":"note","from":"SERVICE_ID","meaning":{"text":"Call the external client; receiver execution is not captured.","evidence":["CALL_REF","INTERACTION_REF"],"uncertainty":"Receiver implementation and runtime delivery are unknown."}},
      {"kind":"return","from":"SERVICE_ID","to":"caller","meaning":{"text":"Return the client result.","evidence":["RETURN_REF"]}}
    ],
    "contracts":[{"title":"Caller request","kind":"payload","rows":[{"label":"Request argument","description":{"text":"The caller forwards the request argument to the client.","evidence":["METHOD_REF"]}}]}]
  }],
  "uncertainties":["The external receiver and its failures are not captured; runtime delivery is UNKNOWN."]
}
```

Replace all references with delivered evidence and `SERVICE_ID` with the selected
service. Add any actual branches, failures and returns required by its source.
For Kafka, describe the source-observed publish and the caller's own return;
that return is not a synchronous consumer reply. A source-derived contract must
separate observed arguments from engineer-declared transport facts. If an entire
operation cannot be explained, use its Work-keyed proposal `gaps` entry instead
of proposing that same operation. A partial receiver limitation belongs in
`uncertainty`/`uncertainties`; an operation and a gap for its target conflict.

A proposal gap is keyed by its current Work reference; the host materializes
Narrative `gaps` under the canonical entrypoint ID, with an
explanation of what must become available. Empty prose, an unbound diagram,
or an endpoint catalogue row does not satisfy full-scope documentation.

The overview should describe business decisions, not repeat every source step.
For a direct Narrative 1.3 import, mark implementation-only explanation paragraphs
with `detail: true`. This is a Narrative field, not a proposal Claim field. Their step coverage and evidence remain validated, but the UI
keeps them inside the detailed view. Keep important failures and asynchronous
boundaries in the short default overview. The renderer folds reference lists and source commentary. For a direct Narrative 1.3 import, add
a source-bound `overviewDiagram` with at most 12 nodes; its grid, node and edge
fields are defined in the service-documentation reference. A large retained
event list must never become the reader-facing diagram.

Use proposal `contracts` (materialized as `interfaceContracts`) for source-derived
HTTP, Kafka and payload facts when
OpenAPI is absent or incomplete. Proposal rows contain `label` and a `description` claim with Work-local
evidence; the host assigns row IDs and materializes their values/bindings. Include the real transport and complete
in-scope payload shape, not just the single rule in this schematic example.
The renderer labels these cards as source-derived; they never become a published
OpenAPI contract merely because they have source references.
