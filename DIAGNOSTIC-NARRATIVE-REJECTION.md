# Request: improve narrative-rejection diagnostics

Historical issue report translated from the original Russian note. The examples,
observations and proposed remedies below describe the reported implementation;
subsequent fixes use bounded safe diagnostics rather than exposing raw input.

## Problem (observed behavior)

During `clew docs render --input narrative.json`, the entire narrative was
rejected, but the CLI reported this indirectly and concealed the root cause.

**User-visible symptom:** the supplied narrative was not applied: old section
content remained in the documentation, and the output `inputDigest` did not
change. The direct sign was `input-0` in `updateFailures` with this value:

```json
{
  "nextAction": "documentation input violates its closed JSON/YAML schema",
  "reason": "INVALID_INPUT"
}
```

## Root cause

The `Command::Render` branch in `crates/clew/src/documentation/cli.rs` reads each
input through `store::read::<Narrative>(path, store::MAX_RECORD)`. On any parse
error, it records **only** the generic `error.message` from `store::read` in
`failures`; it **does not include** the narrative in `narratives`.

The `store::read` function (then at `crates/clew/src/documentation/store.rs:84`):

```rust
serde_yaml_ng::from_slice(&data)
    .map_err(|_| invalid("documentation input violates its closed JSON/YAML schema"))
```

Serialization-error details (`serde_yaml_ng::Error`) are discarded by
`map_err(|_| ...)`. The user sees the generic schema violation but does not learn:

1. **Which field is unknown or extra.** `struct Operation` and `struct Narrative`
   in `crates/clew/src/documentation/model.rs` have `deny_unknown_fields`, so any
   extra field (for example, `interaction` or `operationId`) rejects the entire
   narrative during parsing.
2. **Which operation (index/ID) is problematic.** The error is not localized.
3. **That the narrative was rejected in full** and render silently reused the
   previous retained/pinned narrative.

### Concrete reproducible case

Fields `"operationId"` and `"interaction"` were added to operations in narrative
1.3. They are absent from `struct Operation` in `model.rs`, whose structure uses
`#[serde(deny_unknown_fields)]`. Consequently, **the whole narrative** was
rejected because of just two extra fields in three operations.

## Requested fixes

### 1. Preserve and display the serialization error

In `store::read` in `crates/clew/src/documentation/store.rs`, replace the generic
mapping shown above with a detailed reason from `serde_yaml_ng::Error` or
`serde_json`:

- Invalid/unknown fields (`unknown field`), including the field name.
- The operation/line number where the error occurred, when available.

The original minimum proposal was to include the serializer's
`error.to_string()` in `invalid(...)` instead of discarding it. This records the
proposal; the implemented fix must still avoid exposing private input values.

### 2. Localize the problem to an operation

`deny_unknown_fields` on Narrative/Operation is all-or-nothing. That policy is
acceptable, but rejection must identify the offending operation by ID and/or
its index in `operations`. Consider:

- A separate operation-validation pass with a readable message such as
  `operations[5] (id="..."): unknown field "interaction"`.
- A separate diagnostic block in `updateFailures`, beyond a generic `input-0`.

### 3. Explicitly warn about retained narrative use

When some or all incoming narratives are rejected and publication proceeds
using a previous retained/pinned narrative, explicitly state that the incoming
narrative was not applied. Previously, the only indirect signs were a stable
`inputDigest` and old section content, which misled the reader.

## Relevant code

- `crates/clew/src/documentation/cli.rs`: `Command::Render`, failure collection
  and `input-{index}`.
- `crates/clew/src/documentation/store.rs`: `read`, masking parse errors.
- `crates/clew/src/documentation/model.rs`: Narrative/Operation fields and
  `deny_unknown_fields`.
- `crates/clew/src/documentation/render.rs`: `publish_language_with_mode` and
  `publish_internal`, including how failures affect publication selection.

## Acceptance criteria

- Rendering a narrative with an extra field (`interaction`, `operationId`, etc.)
  reports the **field name** and **operation ID/index**.
- The CLI explicitly warns when the incoming narrative is rejected and the
  previous narrative is published instead.
- Valid narratives retain their existing behavior without regressions.
