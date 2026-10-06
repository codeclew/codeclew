# Compatible model ID serializer

An operation author using `codeclew-operation-draft-authoring/1.6`, or its
independent reviewer, can opt into `"modelRepresentation":
"codeclew-model-ids/1.1"` in its role configuration. This compact source candidate
targets v0.13.16; installed v0.13.15 has neither model-ID mode. Version 1.0
remains supported with its original frozen representation. Omit this option for an
existing driver that expects the canonical job on stdin. The mode participates
in the admitted configuration identity.

The host retains the unchanged canonical job and an exact prepared model form
before dispatch. An opted-in driver receives this versioned carrier:

```json
{
  "schema": "codeclew-model-ids/1.1",
  "canonicalJob": { "...": "unchanged canonical job" },
  "preparedModel": { "...": "frozen payload, schema, scope, map and bindings" }
}
```

Validate `canonicalJob` with the driver's existing canonical request and packet
checks first. Then call the public Rust
`clew::documentation::model_ids::forward_model_input` API with the canonical job,
the deserialized `Prepared` value, and that validation callback. The API invokes
the callback before checking the complete frozen projection against the
canonical job. It returns `ModelInput { payload, output_schema }`.

Forward only those two returned values through the existing model serializer.
The host map, canonical job, carrier and scope metadata are driver metadata;
they must not be added to the model prompt or strict output schema. Keep the
prepared instruction and strict schema. Never recalculate a canonical packet
digest from aliased content.

Version 1.1 keeps complete evidence in each native delivery's `presentation`.
Before alias projection, the host checks that presentation against the existing
native builder using all canonical pages and source parts. It then excludes only
the duplicate raw `pages` and `sourceParts` siblings in `packet.contextDelivery`
and `reviewContext` from model input. Read the presentation's complete pages,
source parts and retained-reference links. Receipts, citations, delivery bindings
and canonical raw arrays remain retained. A missing presentation keeps raw data;
a mismatched or incomplete presentation fails instead of losing evidence.

Version 1.0 retains its original complete raw-plus-presentation model payload,
map identity and carrier bytes. Frozen records validate and recover by their
saved version; changing a role's selected mode cannot migrate an existing run.
For pure preparation, `prepare` retains version 1.0 behavior; use
`prepare_with_version` with the explicit selected version for version 1.1.
`forward_model_input` validates the saved version. No default silently rewrites
old carriers.

Input admission checks the complete serialized carrier, including the canonical
job, prepared payload and schema, and host-only map, plus configured input
overhead. This conservative byte bound can require a higher explicit role cap
and corresponding reservation budget. The driver forwards only `ModelInput` to
the model; carrier admission bytes are not measured provider tokens or billing.

The [Rust serializer example](../../crates/clew/examples/model_ids_serializer.rs)
reads the carrier from stdin and writes precisely those two model input values
as JSON. It checks canonical envelope identity and packet content binding;
integrating drivers must retain their existing policy, budget and contract
checks in the callback. It makes no provider call and does not emit a final
agent Reply. For local source development:

```sh
cargo run --locked -p clew --example model_ids_serializer < carrier.json
```

Return the existing `codeclew-documentation-agent-result/1.0` outer Reply with
its canonical invocation, role and model. The inner answer or review copies the
typed aliases delivered in the model payload and strict schema. The host retains
the parsed outer JSON value before validating the Reply or decoding those binding fields. Unknown,
wrong-domain and foreign-scope aliases fail closed. Existing semantic answer and
review validation receives the decoded canonical result.

Aliases are typed and scoped to one Work, run, role and encoding version. A
grouped expansion extends that role's map without renumbering existing entries.
The reviewer has its own scope and registered read delivery. Source text, code,
semantic symbols, paths, native evidence references and query text retain their
original values. Aliases are representation identities, not new evidence or
publication authority.

Immutable invocation records retain the canonical input, exact prepared carrier,
map and parsed wire output. This wire record does not preserve byte-for-byte
stdout; malformed JSON remains an adapter `MALFORMED_DRIVER_OUTPUT` failure.
Saved successful answers and reviews remain
canonical. Recovery consumes an already retained output, including an invalid
alias response, without a second dispatch. A prepared carrier establishes the
admitted driver's forwarding contract; it does not independently observe what a
remote provider received or establish answer quality, token use or billing.

A parseable response accompanied by an adapter failure retains that failure.
Recovery must not treat valid JSON from a failed driver as a successful role
result or perform its requested expansion.
