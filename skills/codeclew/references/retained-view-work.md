# Prepare a typed view from retained evidence

Use this path after entity, process, interaction or view declarations changed and
`docs recompose` returned a new snapshot. It also works when the original source
checkout is unavailable. It uses the supported generic Work command; no private
state edit, source recapture or native runtime upgrade is needed.

In the admitted 0.13.52-derived candidate and the existing view convenience
command, `docs view prepare --id VIEW_ID` reads only `latest` and has no snapshot
selector. Recomposition deliberately does not move `latest`. Repeating the view
shortcut can therefore return `STALE_REQUIRES_RESLICE`, or `STATE_CORRUPT` when a
recovered root has no latest pointer. The returned recomposed snapshot is still
usable through `docs work prepare --snapshot`.

1. Save the entity and view declarations through their supported commands.
2. Run `docs recompose --root DOCS_ROOT --snapshot ORIGINAL_CAPTURE`. Preserve
   its exact returned `snapshot` and `inputDigest`; `CHECKED` with partial source
   coverage can have exit 3. Do not change catalogue inputs between recomposition
   and preparation. If they changed, recompose again from the original capture.
3. Save this caller-owned request file, with the same defaults as the view
   convenience command:

```json
{
  "schema": "codeclew-documentation-work-request/1.0",
  "audience": "Domain and service maintainers",
  "entrypoint": "entity-dataflow",
  "maxItems": 20,
  "maxBytes": 40960
}
```

4. Prepare the saved view against the explicit returned snapshot:

```sh
clew docs work prepare --root DOCS_ROOT --subject scenario:VIEW_ID \
  --snapshot EXACT_RECOMPOSED_SNAPSHOT --input view-work-request.json
```

Use the existing saved view's exact ID. The Work response must name that scenario
subject and selected snapshot. Continue its recorded reads and pagination;
preparation alone does not establish source reading, complete evidence delivery
or meaning approval. No fresh `docs check` is required for this path.

The request's `entrypoint: "entity-dataflow"` selects the typed-view Work root.
For the subsequent Proposal, use the delivered `scenario:VIEW_ID` subject as its
operation entrypoint, author `dataflow.nodes/edges` and leave sequence steps empty,
as described in [typed DataFlow views](documentation-views.md). Keep source
interpretation, declared entity relations and runtime behavior distinct.

## Domain node identity is a semantic validator invariant

The Proposal's [dataflow node schema](../schemas/proposal.schema.json) describes
the fields; native validation adds these exact constraints for `kind: "domain"`:

- `entity` is a fully qualified declared identity, such as `entity:reservation`,
  selected in the view's `inputObjects`.
- `representation` must equal that same fully qualified identity exactly. A
  display title such as `Reservation` is invalid in this field for domain nodes.
- `meaning.text` must equal the registered entity's exact `title`; it is the
  domain label here, not an expanded business description.
- `meaning.evidence` must include this Work's delivered `DOMAIN_ENTITY` reference
  for that exact entity. Native materialization binds it to the dependency ID;
  a raw `entity:reservation` string is not a Work evidence handle.
- `id` is a graph-local, unique valid ID used by edges. It need not equal the
  declared entity ID. `service` must belong to the saved view's service scope.

For a registered `{ "id": "reservation", "title": "Reservation", ... }`, the
following [owned Proposal node fragment](../examples/domain-entities/domain-node-fragment.json)
is a fragment, not a complete Proposal:

```json
{
  "id": "reservation-domain",
  "entity": "entity:reservation",
  "kind": "domain",
  "service": "orders",
  "representation": "entity:reservation",
  "meaning": {
    "text": "Reservation",
    "evidence": ["EXACT_WORK_DOMAIN_ENTITY_REFERENCE"]
  }
}
```

Replace only the placeholder evidence handle and the actual registered
entity/title/service values from the selected Work. Preserve business detail in
the entity description, summary or supported edges/representation claims; do not
lengthen the domain node label beyond its declared title. DTO, field, message,
table and function nodes use their actual representation text and source from
the named service. They are not domain nodes merely because a type name matches.

For non-domain nodes, `meaning.text` is the visible SVG label. Use a short,
distinctive value, representation or operation name, such as `Requested quantity`,
`Store.save call port`, or `Response DTO`. Paragraph-length labels can be clipped
and hide the node's identity even when native validation succeeds. Keep detailed
guards, mappings and outcome qualifications in edge meanings, summary and bound
evidence. Preserve those facts and source references during caption-only edits;
domain labels still use the exact declared title. No new caption field or
renderer/layout change is implied.

The canonical [dataflow graph schema](../schemas/dataflow.schema.json) describes
materialized output with `definitionDigest`, `moduleDigest` and bound fragments.
Do not submit that carrier as a Proposal. A Proposal supplies `dataflow.nodes`
and `dataflow.edges` inside its operation; the native host creates those digests
and materialized fragment identities.

## Local call-port flow and external lineage

A local source DataFlow can show request construction, response parsing, business
values, conditional store/publisher call inputs and caller response DTOs. For an
actual captured client or SDK call, a function/DTO/message node can represent its
caller-side port or payload. Keep its `service` equal to the captured caller and
label it explicitly as a local call port, with the actual payload, guard and
source evidence. The call receiver's implementation is not thereby captured.

This is a useful, supported local data-flow view. It can satisfy a requested
local source-flow scope while remote receiver lineage remains unknown. The
separate sequence must still expose registered external participants and required
request/outcome arrows; service egress must still include every selected boundary.
Local port nodes do not replace those mandatory views or their calls.

The admitted module's `view.services` contains only registered source-captured
services. Every node's `service` must belong to that scope; a registered external
destination cannot be used as a node service or added as a captured service
without actual captured evidence. The current `transfer` / `DECLARED_TRANSFER`
validator also excludes `agent-proposal` interactions. A declaration inferred
from source must keep its true origin.

There is no supported remote endpoint-node carrier in this admitted typed module.
State this limit when cross-boundary receiver transfer/lineage is explicitly
requested. Do not invent captured services, relabel origin or claim a local port
is a remote endpoint to obtain READY. Full external receiver lineage would need a
scoped native extension and additional evidence; it is not established by an HTTP
call or a passing local typed graph. Preserve required external boundaries in the
sequence and egress regardless of the DataFlow scope.

This generic preparation is the supported retained-snapshot equivalent of the
view shortcut; it does not update the convenience latest pointer or change
accepted content. Preserve the original failed calls and continue the existing
author/reviewer history. A guided continuation is not a new blind first pass.

A non-domain representation (`field`, `function`, `message`, `dto` or `table`)
may omit `entity` when no domain association is established. Explicit JSON null
normalizes to absence. Keep exact source/service evidence and the actual
representation; technical correlation context is not a business identity. Never
choose an arbitrary registered entity to satisfy the schema, or register a
technical value as a domain concept for that purpose. If a domain association is
supported, use its exact declared ID; domain nodes still require that ID, exact
declared title/representation and entity evidence. Older runtimes that reject
absence expose a contract gap: retain it and stop that publication instead of
dropping real lineage or inventing a domain. Independent mapping review remains
required even for a native-ready graph.
