# Declared external relationships and sequence boundaries

This projection uses the retained interaction catalogue. It does not discover
external calls, capture external source, or establish runtime behavior. Registered
external participants link to pages in the same generated bundle. The overview,
service and selected process show a directed graph and a text navigation route.
Graph positions establish no execution order. Parallel interactions retain their
IDs even when participant titles are equal.

Register the external service with `docs external put`, and save an interaction
with `external: true`, `externalService: "provider"`, and `to.service: "provider"`.
Use the actual declaration origin: `human`, `imported`, or `agent-proposal`.
Keep the existing HTTPS URL admission. Select the interaction in the process's
`interactions` list. Recompose the original source snapshot, then prepare a fresh
Work for that scenario or the calling service. Read its `DECLARED_INTERACTION`
records through `nextCursor: null` before submitting a proposal.

In proposal 1.0, add only the external actor:

```json
{"participants":[{"id":"provider-actor","label":"Provider","externalService":"provider"}]}
```

`service` and `externalService` are mutually exclusive. Built-in selected services
remain available under their service IDs. A boundary step uses the exact delivered
Work reference in both `interaction` and `meaning.evidence`:

```json
{"kind":"declared-boundary","from":"owned","to":"provider-actor","interaction":"INTERACTION_REF","meaning":{"text":"Declared request; receiver execution is unknown.","evidence":["INTERACTION_REF"]}}
```

This step can use declaration evidence without a fake source citation. Its
explanation also retains that declaration. The operation summary and separate
source-derived steps still require their own source evidence. Mandatory source
conditions and returns remain mandatory; a boundary arrow cannot cover a FLOW.
The runtime materializes these additions as Narrative 1.4. Ordinary Narrative
1.3 input, serialization and resolved `declared` validation stay supported.

An explicitly authored HTTP response uses `declared-boundary-return` with the
same interaction reference and reversed participants. No response is generated
automatically. Kafka has no synchronous return. A separate outgoing reply event
requires its own declared interaction; inbound external discovery is outside
this model. Source-backed caller failures and retries belong in separate steps
with actual supporting source, not in invented external behavior.

Inspect `proposal show`: `NEEDS_REPAIR` blocks publication even if a wrapper exits
zero. After the supported ready status, publish the proposal with its actual
review policy and export using `docs render`. No separate session mutation loop
is required for documentation commands; the supported launcher owns admission.
Never substitute a direct capsule executable or modify private state.

HTML embeds linked SVG nodes and sequence participant links, plus text navigation.
Markdown and Mermaid source retain the identities, direction and declared scope.
Full Pages preserves the graph data. MDX includes the graph and ordinary page
links; diagram-host security may disable embedded Mermaid links, so use those
normal links. External Wiki links remain on the participant's page.

Catalogue changes require recomposition and a fresh Work. New publications use
new retained declarations; old publications and Work are not rewritten. Removing
a relationship removes it from a newly projected graph. Endpoint/source statuses
remain independent: `SOURCE_MATCH` is never promoted to `RESOLVED`, and an
`agent-proposal` remains visibly unverified.

Sequence origin, transport and declaration evidence come from that operation's
saved event binding, including after a catalogue mutation or relationship removal.
They never fall back to the publication's catalogue graph. New explicit HTTP
responses retain their own event binding. A legacy response may use an unambiguous
same-operation interaction binding, visibly labelled as such; without one, its
provenance is explicitly unavailable. A changed catalogue is shown separately
from retained sequence evidence and does not upgrade its original declaration.
