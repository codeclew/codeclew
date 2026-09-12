# Saved entity data-flow views

Use `docs view modules --root <docs>` to read the built-in input, edge, authority,
validation, dependency and renderer contract. Only the registered
`entity-dataflow/1.0` implementation is supported; this is not an executable
plugin interface.

A saved definition has the same `root`, interaction selection and traversal bounds
as a scenario. Use schema `codeclew-documentation-view/1.0`, an explicit stable
`id`, `title`, `summary`, and this `view` object:

```json
{
  "module": "entity-dataflow/1.0",
  "inputObjects": ["entity:quantity"],
  "services": ["orders"],
  "contracts": [],
  "relatedProcesses": [],
  "scope": "Read and normalize the requested quantity; preserve uncertain lineage.",
  "human": {
    "annotations": {},
    "tags": [],
    "metadata": {},
    "layout": {}
  },
  "limitations": ["Static source does not establish runtime lineage."]
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
