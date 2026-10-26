# Authoring contracts

These self-contained JSON schemas are byte-for-byte copies of the corresponding
Codeclew source contracts. They are portable documentation, not a runtime upgrade
or a new validation engine; use the contract of the admitted runtime.

| File | Use |
| --- | --- |
| [proposal.schema.json](proposal.schema.json) | Manual `docs proposal submit` input, including service-section `explanation` claims |
| [section-author.schema.json](section-author.schema.json) | Bound isolated section role's summary or expansion response; its actual job further restricts delivered evidence handles |
| [entity.schema.json](entity.schema.json) | Closed `docs entity put` catalogue record; see the [complete shape and owned examples](../references/domain-entities.md) |
| [dataflow.schema.json](dataflow.schema.json) | Canonical materialized graph, not Proposal input; see [retained preparation and exact domain node invariants](../references/retained-view-work.md) |

These schemas contain no remote `$ref` dependencies. The native validator also enforces Work
identity, read coverage, claim evidence, root-specific shapes, UTF-8 limits and
required FLOW coverage. A generic schema or syntax check cannot establish those
facts or meaning review. Use the contract selected by the installed command and
its actual job; do not interchange a Proposal, operation answer, role response or
materialized Narrative.

The optional task-oriented reader extensions also ship `external-service.schema.json`
and `data-model-plan.schema.json`. Keep their exact native copies in every skill mirror.
