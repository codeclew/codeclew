# Source-bound authoring example

This schematic example shows the record shape. Replace every uppercase
placeholder with an ID or digest returned by `docs context`; these placeholders
are deliberately not executable evidence. The service fixture in the product
repository (`fixtures/durable-docs`) and its CLI regression provide a complete
executable two-service example, including rejected orders and stock shortages.

Suppose retained source says `if (quantity <= 0) return "invalid";` followed by
`return reserve(request);`. The diagram must preserve that early return. The
summary describes the outcome in plain language. Ordered explanation paragraphs
explain the business meaning of the diagram and link directly to its steps; exact
implementation details remain available in the source inspector. A service operation ID is its exact catalogue entrypoint
ID; a scenario operation ID is its registered scenario ID.

```json
{
  "schema": "codeclew-documentation-narrative/1.1",
  "subject": "service:orders",
  "contextDigest": "CURRENT_CONTEXT_DIGEST",
  "operations": [{
    "id": "ENTRYPOINT_ID",
    "title": "Check out an order",
    "summary": {
      "id": "summary",
      "text": "Reject invalid quantities and request a reservation for valid orders.",
      "dependencyIds": ["METHOD_DEPENDENCY"],
      "sourceIds": ["METHOD_SOURCE"]
    },
    "participants": [
      {"id": "client", "label": "Client"},
      {"id": "orders", "label": "Orders", "service": "orders"}
    ],
    "events": [
      {"id": "guard", "kind": "alt", "text": "Quantity is zero or negative", "dependencyIds": ["IF_DEPENDENCY"], "sourceIds": ["IF_SOURCE"]},
      {"id": "reject", "kind": "return", "text": "Return invalid quantity", "from": "orders", "to": "client", "dependencyIds": ["REJECT_DEPENDENCY"], "sourceIds": ["REJECT_SOURCE"]},
      {"id": "valid", "kind": "else", "text": "Quantity is positive", "dependencyIds": ["IF_DEPENDENCY"], "sourceIds": ["IF_SOURCE"]},
      {"id": "reserve", "kind": "message", "text": "Request a reservation", "from": "orders", "to": "orders", "dependencyIds": ["CALL_DEPENDENCY"], "sourceIds": ["CALL_SOURCE"]},
      {"id": "result", "kind": "return", "text": "Return the reservation result", "from": "orders", "to": "client", "dependencyIds": ["RETURN_DEPENDENCY"], "sourceIds": ["RETURN_SOURCE"]},
      {"id": "end", "kind": "end", "text": "", "dependencyIds": ["IF_DEPENDENCY"], "sourceIds": ["IF_SOURCE"]}
    ],
    "explanation": [
      {"id": "eligibility", "text": "An order must request a positive quantity. A zero or negative quantity is rejected immediately, so it does not consume inventory or create a reservation.", "eventIds": ["guard", "reject"], "dependencyIds": ["IF_DEPENDENCY", "REJECT_DEPENDENCY"], "sourceIds": ["IF_SOURCE", "REJECT_SOURCE"]},
      {"id": "reservation", "text": "For a valid quantity, the service requests a reservation and returns its result to the customer. This step determines whether inventory can be held for the order; acceptance of the request alone does not mean that stock has been reserved.", "eventIds": ["valid", "reserve", "result"], "dependencyIds": ["IF_DEPENDENCY", "CALL_DEPENDENCY", "RETURN_DEPENDENCY"], "sourceIds": ["IF_SOURCE", "CALL_SOURCE", "RETURN_SOURCE"]}
    ],
    "findings": [],
    "boundaries": ["This service view stops at the reservation client; the registered checkout scenario connects both services."]
  }],
  "gaps": {}
}
```

Allowed event kinds are `message`, `return`, `note`, `alt`, `else`, `loop`, `opt`, `end`,
and `declared`. For a cross-service request use `kind: declared`, participants
with the two registered service IDs, and `interaction: reserve-inventory`.
Bind the edge to the declaration and the returned call/receiver dependencies.
A corresponding HTTP reverse return also names that interaction. Kafka delivery
has no synchronous return; declare a separate reply-event interaction. Nest receiver
conditions within the caller's successful branch. Do not omit failures just
because the named scenario focuses on success.

A source gap is an entry in `gaps` keyed by the exact entrypoint ID with an
explanation of what must become available. Empty prose, an unbound diagram,
or an endpoint catalogue row does not satisfy full-scope documentation.
