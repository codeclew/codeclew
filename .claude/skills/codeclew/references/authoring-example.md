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
and `declared`. For a cross-service request use `kind: declared`, participants
with the two registered service IDs, and `interaction: reserve-inventory`.
Bind the edge to the declaration and the returned call/receiver dependencies.
A corresponding HTTP reverse return also names that interaction. Kafka delivery
has no synchronous return; declare a separate reply-event interaction. Nest receiver
conditions within the caller's successful branch. Do not omit failures just
because the named scenario focuses on success.

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
