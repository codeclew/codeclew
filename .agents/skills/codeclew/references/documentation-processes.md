# Explicit maintained process definition

Use this JSON shape with `docs process put --input <file>` after the user has
requested a saved process. Use registered service IDs, exact supported selectors,
existing declared interaction IDs and explicit domain entity IDs. The metadata
records the requested scope; it does not verify business outcomes.

```json
{
  "schema": "codeclew-documentation-process/1.0",
  "id": "quantity",
  "title": "Quantity handling",
  "summary": "An explicitly requested quantity process.",
  "root": {
    "service": "orders",
    "selector": {
      "language": "java",
      "owner": "Orders",
      "name": "reserve",
      "parameterTypes": ["int"]
    }
  },
  "interactions": [],
  "maxDepth": 4,
  "maxNodes": 64,
  "process": {
    "scope": "Quantity handling within the declared services.",
    "participants": ["orders"],
    "objects": [],
    "trigger": "A caller requests a quantity.",
    "outcomes": ["Return a normalized quantity or preserve an evidenced failure."],
    "linkedSubviews": []
  }
}
```

`objects` contains `entity:<id>` references. `linkedSubviews` contains stable
process or legacy scenario IDs; a missing child is retained as a visible gap.
For example, a larger saved process can link to `quantity` through that field.
Do not declare an unproven execution order merely because child views are listed.
The definition is human-requested metadata. The generated overview and detailed
flow are separate reviewed interpretations with retained evidence.

`docs process inspect --input <file>` explores a candidate without saving it.
For saved work, get the current `inputDigest` from `docs process list`, pass it as
`--expected-input-digest` to `put`, then use `show --id <id>` or
`prepare --id <id> --overview`. All commands also require `--root <docs>`.
The normal author/reviewer execution reads recorded evidence and publishes the
accepted summary; it cannot rewrite a saved definition or original human note.
