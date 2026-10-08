# Addresses and contracts at outgoing service calls

The HTML reader displays declared destinations beside a linked message step in
an ordinary service operation. Expand **Contract** at that step to read the
saved outgoing OpenAPI operation, its request parameters/body, responses and
server declarations. This does not switch to the caller's incoming API contract.
Addresses are inert text: rendering neither contacts the destination nor opens
an external URL. User declarations never establish runtime routing or delivery.

## Portable input

Use the existing `clew docs interaction put` command. An interaction may contain
up to 16 `addresses`, each with `url`, `source` and an optional `environment`.
An omitted environment is shown as unspecified. URLs must use
HTTP(S), contain no credentials or whitespace, and fit within 2048 bytes.
`source` records where the user obtained the address; it is not fetched.
`declaration.origin` and `declaration.rationale` retain the binding's provenance.
Address environments are displayed individually; the reader does not select a
production address automatically or resolve configuration placeholders.

For a service without a local checkout, set `external: true` and specify only
its identity in `to.service`. No service registration, repository binding or
analysis of the destination is required. A receiver selector or receiver
callSite is rejected in this mode. The destination remains an incomplete source
endpoint; this is intentional and does not block displaying user-provided data.

```json
{
  "schema": "codeclew-documentation-interaction/1.0",
  "id": "reserve-external",
  "title": "Reserve external inventory",
  "external": true,
  "from": {
    "service": "orders",
    "selector": {
      "language": "java",
      "owner": "Orders",
      "name": "reserve",
      "parameterTypes": ["int"]
    },
    "callSite": { "observation": "REPLACE_WITH_CAPTURED_FLOW_CALL_ID" }
  },
  "to": { "service": "external-inventory" },
  "transport": {
    "kind": "http",
    "method": "POST",
    "path": "/external-reservations"
  },
  "declaration": {
    "origin": "human",
    "rationale": "The service owner supplied this destination and API declaration."
  },
  "addresses": [{
    "url": "https://inventory.example.invalid/v1",
    "environment": "staging",
    "source": "Deployment inventory provided by the service owner"
  }],
  "applicability": { "environments": ["staging"] },
  "contractReference": "REPLACE_WITH_SAVED_CONTRACT_OPERATION_ID"
}
```

Replace both ID placeholders with actual records returned by the selected
snapshot's `docs context`. The caller selector must identify one declaration.
`callSite.observation` selects an exact captured `FLOW` whose kind is `CALL`
within that declaration, including syntax-backed calls with unresolved targets.
It cannot select a different caller, a contract or a guessed target. This proves
which source location the user selected, not which remote service executes.
Compiler-backed callers may instead use the existing `callSite.target` and
optional zero-based `ordinal`. Do not combine those with `observation`.
An ambiguous or missing call site is not attached to an arbitrary message step.
Existing declared cross-service arrow validation remains unchanged.

## Reuse a saved outgoing contract

Set `contractReference` to the exact `CONTRACT_OPERATION` observation ID from
saved evidence. A path, an operation name or a source-file name is not a binding.
If the receiver's existing source endpoint already has one exactly linked saved
OpenAPI operation, that operation can be reused without an explicit reference.
Multiple or missing receiver contracts require an explicit reference; similar
names and matching paths are never sufficient to choose one.

For an external service without a checkout, place the API file supplied by its
owner in the **caller repository** and list that repository-relative file in the
caller's existing `contractFiles` configuration. Capture the caller explicitly
with `docs check --service orders`; this imports the supplied declaration and
its source without checking out the external service. OpenAPI server declarations
remain contract data, not verified runtime destinations. Source-derived interface
contracts are not substituted for an outgoing OpenAPI declaration.

```sh
clew docs context --root /work/architecture --service orders \
  --snapshot CAPTURE_SNAPSHOT --limit 100
# Follow the returned cursor if needed. Inspect the exact FLOW and contract IDs.
clew docs interaction list --root /work/architecture
clew docs interaction put --root /work/architecture --input interaction.json \
  --expected-input-digest INPUT_DIGEST_FROM_LIST
clew docs recompose --root /work/architecture --snapshot CAPTURE_SNAPSHOT
```

Adding or changing only the interaction uses `docs recompose` from the original
capture. It does not recapture sources. Use its returned snapshot and context
for the ordinary narrative/proposal workflow, then render that accepted operation.
A partially checked declaration may return exit code 3 while still providing a
saved recomposed snapshot; inspect its explicit checks and boundaries.

## Retention and gaps

Addresses, provenance and contracts stay with the accepted operation's saved
projection. Rendering retained prose after a declaration changes preserves the
old details and marks them as changed. Updating a declaration never silently
replaces an accepted operation's address or contract. Accept updated operation
content against the new context to display the new binding. Older publications
without outgoing-call metadata remain readable and acquire no inferred links.

An absent address is shown explicitly. An unavailable exact contract reference
or multiple receiver contracts produce a gap with a path to set
`contractReference`. Multiple interactions selecting the same step are displayed
as separate declarations with an ambiguity notice. No destination is selected
by the reader. The current caller's incoming contract remains a separate tab.
