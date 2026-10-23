# Inline declarations and service field tables

The source preview supports an optional `interfaceBindings` array in the existing
`--data-model` plan for `./clew docs pages render`. A binding names an admitted
contract and one exact retained interface card:

```json
{
  "contract": "forecast-updated",
  "subject": "scenario:dashboard-forecast-updated-consumer",
  "operation": "dashboard-forecast-updated-consumer",
  "interfaceContract": "claim-1fa6ad108a2637e192d5"
}
```

The contract's existing service, source ID, text digest and JSON Pointer bind the
retained declaration. A missing card or contract, duplicate card binding, changed
source digest or unavailable branch rejects the export. Titles and prose never
select a schema. Older plans remain valid without the optional array.

The Contract reader presents the bound schema inline, including objects, arrays,
items, fields, parent-relative presence, nullability, descriptions and constraints.
One primary declaration link leads to its exact source. Original code-derived
checks remain separate and unchanged, with detailed source bindings on demand.
An unbound card is explicitly a source-derived account, not a declared schema.
References, composition and conditional rules are retained without expansion or
rule merging; any display frontier is visible beside the exact retained schema.
This is a reading projection, not a validator or evidence of runtime conformance.

Service `section-entities` tables use only the existing explicit model
`processLinks` associations to that service and section. They reuse the model
field table, complete comments, authored readable names and canonical model
links. Original service descriptions and compact exact evidence identities stay
available on demand. The projection does not invent domain identity or ownership.

Local presentation aliases may route a historical `/generated/<publication>/`
HTML URL to a current renderer for the same retained publication. Preserve query
and fragment, label the renderer and provide an explicit historical option.
Retained JSON/source files and original export artifacts must remain unchanged.
