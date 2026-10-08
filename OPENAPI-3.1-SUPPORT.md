# Request: support OpenAPI 3.1.x service contracts

Historical issue report translated from the original Russian note. Version
allowlists and family gates below are recorded proposals, superseded by the
subsequent user requirement to attempt bounded reading through OpenAPI 3.x.

## Problem (observed behavior)

A service declares `src/main/resources/openapi/openapi.yml` with
`openapi: 3.1.2`. During `docs render`/`docs check`, it is not interpreted:

- The service shows the boundary
  `UNSUPPORTED_CONTRACT_VERSION:src/main/resources/openapi/openapi.yml`.
- No `CONTRACT_OPERATION` observations are created; `contract-scope`
  (`CONTRACT_SCOPE`) has no contracts.
- Render returns `contracts:[]` and zero operations declared in OpenAPI even
  though the contract contains operations.

The user cannot see why: the contract is silently ignored.

## Root cause

`crates/clew/src/documentation/contracts.rs` imposed a strict version allowlist:

```rust
pub const TESTED_VERSIONS: &[&str] = &["3.0.0", "3.0.3"];
```

In that implementation:

- `capture`/`import` (approximately lines 145-160) compared `document["openapi"]`
  with `TESTED_VERSIONS`. On a mismatch it emitted
  `UNSUPPORTED_CONTRACT_VERSION` and **did not add** the contract to
  `evidence.contracts`.
- `enrich` (approximately lines 299-306) used
  `if !document["openapi"].as_str().is_some_and(|v| TESTED_VERSIONS.contains(&v)) { continue; }`.
  It did not generate operations for unsupported versions.

`crates/clew/src/documentation/modules.rs:228` obtained the version list from
the same constant:

```rust
openapi["testedVersions"] = json!(super::contracts::TESTED_VERSIONS);
```

The report therefore proposed updating this single source.

## Actual contract reported

`motor-deal-service`: `openapi: 3.1.2`, 5343 lines, standard structure
(`info`, `servers`, `security`, `tags`, `paths`, `components`). The reported
contract used **none** of the listed 3.1-specific features (`webhooks`,
`type: null`, `unevaluatedProperties`, `patternProperties`). The report described
it as compatible with 3.0.3 apart from its version string. `examples:` was
present and is also valid in 3.0.3. This preserves the original observation;
it is not a claim of complete specification validation.

## Original requested fixes

### Minimum proposal: add the specific version

At the then-current `contracts.rs:17`:

```rust
pub const TESTED_VERSIONS: &[&str] = &["3.0.0", "3.0.3", "3.1.0", "3.1.1", "3.1.2"];
```

### Preferred original proposal: support the 3.1.x family

Replace the exact list with major/minor compatibility, for example a
`supported_openapi_version(&str)` helper admitting `3.0.x` and `3.1.x`, and use
it in both `capture`/`import` and `enrich`. This would avoid adding each new
3.1 patch separately.

Preserve explicit boundaries for 3.1-specific features:

- `webhooks`, `type: null`, `2020-12`, etc. differ structurally from 3.0. Such
  cases must retain explicit limits rather than disappearing silently.
- The existing Resolver handles `$ref` and fragments. For the reported
  contract, 3.1-compatible path/parameter/security parsing requires no new
  resolver logic.

## Suggested diagnostic improvement

Make a version-related rejection explicit in render/check output, for example
`UNSUPPORTED_CONTRACT_VERSION:<path> (openapi=<ver>; supported=3.0.x, 3.1.x)`.
Show it as the reason for `contracts:[]`, rather than hiding it in general status.
This is the original suggested diagnostic, not the current attempt-first policy.

## Acceptance criteria

- Capturing the reported `motor-deal-service` contract with `openapi: 3.1.2`
  adds it to `evidence.contracts`, and enrich creates a `CONTRACT_OPERATION`
  for each HTTP operation under `paths`.
- Render returns a nonzero OpenAPI-operation count.
- OpenAPI 3.0.x contracts do not regress.
- Contracts using 3.1-specific features retain explicit limits instead of
  disappearing silently.
