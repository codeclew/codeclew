# Saved entity data-flow views

Use `docs view modules --root <docs>` to read the built-in input, edge, authority,
validation, dependency and renderer contract. Only the registered
`entity-dataflow/1.0` implementation is supported; this is not an executable
plugin interface.

A saved definition has the same `root`, interaction selection and traversal bounds
as a scenario. Submit the complete definition, including top-level `root.service`;
the nested `view` object alone is not a valid `docs view put` input. This example
uses only synthetic service and entity IDs:

```json
{
  "schema": "codeclew-documentation-view/1.0",
  "id": "quantity-lineage",
  "title": "Quantity lineage",
  "summary": "Source-based quantity normalization and explicit unknowns.",
  "root": {
    "service": "orders"
  },
  "interactions": [],
  "maxDepth": 4,
  "maxNodes": 64,
  "view": {
    "module": "entity-dataflow/1.0",
    "inputObjects": [
      "entity:quantity"
    ],
    "services": [
      "orders"
    ],
    "contracts": [],
    "relatedProcesses": [],
    "scope": "Read and normalize the requested quantity; preserve uncertain lineage.",
    "human": {
      "annotations": {},
      "tags": [],
      "metadata": {},
      "layout": {}
    },
    "limitations": [
      "Static source does not establish runtime lineage."
    ]
  }
}
```

Register explicit entity IDs first. Use the current `inputDigest` from
`docs view list` with `docs view put --input <definition.json>
--expected-input-digest <digest>`; every command also requires `--root <docs>`.
Existing authorization to save the view is sufficient. Keep its ID when changing
its title. `docs view prepare --id <id>` uses the normal recorded-read,
proposal, separate meaning-review and publication pipeline. The proposal schema
returned to the author includes `dataflow.nodes` and `dataflow.edges`; use its
`scenario:<view-id>` subject as the entrypoint and leave sequence `steps` empty.

## Find source bodies in this scenario Work

Finish required initial/retained reads first. For additional discovery, query
exact `SYMBOL` records in the **same scenario Work**, with `symbolContains` set
to the complete captured path of the needed mapper/type, for example:

```json
{"query":{"kind":"SYMBOL","symbolContains":"src/main/java/example/OrderMapper.java"}}
```

Use the actual path from saved evidence. Scenario Work does not support the
`SOURCE` inventory query; that inventory belongs to service Work. Do not replace
a targeted lookup with an unfiltered `kind: "*"` drain or prepare another Work.
Inspect returned SYMBOL rows and select their delivered `sourceReferences` in
batches of at most eight; read those exact sources, including matching native
source parts when omitted. SYMBOL metadata alone is not complete source text.

`work_session.py read` discovery queries default to one page. A nonterminal
result is `INCOMPLETE`, never complete delivery. Save and inspect its
`read-collection/manifest.json`; the exact `continuationSelection` preserves the
query and cursor. Use it in the same Work when another page is needed, with an
explicit bounded `--max-pages` override. Exact-reference reads and mandatory
initial, retained and proposal collection keep their existing behavior.

The `work_session.py submit` participant preflight is service-only. For a
scenario/DataFlow proposal, use supported native `docs proposal submit`, save
stdout/stderr/exit status, then use `collect_proposal.py` on its returned proposal
with an explicit page budget. Inspect every diagnostic through a terminal
cursor; an incomplete collection cannot establish native readiness. This route
does not bypass native participant, evidence or meaning validation.

Read opaque Work/proposal IDs, source references, cursors and digests from the
saved JSON into command argument arrays (for example with `json.loads` and
`subprocess.run`). Do not retype or reconstruct them from displayed summaries.

Cite the view definition and source in the summary. Domain node IDs and titles
must match the declared entity; DTO/message/table/field/function representations
need exact source from their named service. Read mapper/body references explicitly.
Every read/transform/write edge needs source evidence. A declared transfer needs
an explicit interaction, matching service endpoints, source and uncertainty.
Name-only matches remain `candidate` / `UNKNOWN` with a stated limitation.
Evidence classifications never grant runtime or review authority.

Keep `view.human` annotations, tags, metadata and layout separate from generated
nodes/edges. Ordinary updates cannot replace those fields; use `--human` only for
an explicitly requested maintainer edit. Protected original notes may target
`view:<id>`. Preserve stale accepted content and unknown edges after changes.
The renderer exports the same graph to SVG, readable text and Mermaid; none of
these representations proves runtime execution or universal data lineage.

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
