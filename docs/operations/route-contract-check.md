# Compare retained routes and OpenAPI operations

Use the installed launcher and an explicit snapshot from `clew docs check`:

```sh
clew docs contract check --root ./documentation --snapshot "$SNAPSHOT" --service orders
```

Repeat `--service` to select several captured services. The command reads the
whole selected retained capture, independently of any unfinished Work/context
page. It never captures source, follows `latest`, fetches external references,
changes declarations or updates the saved snapshot. Historical snapshots remain
readable after current declarations change. Missing/corrupt evidence or an absent
selected service is an error.

The JSON report uses `codeclew-route-contract-report/1.0`. Exit 0 and `REPORTED`
mean that comparison completed, including reports containing differences and
unknowns. They do not mean that the API conforms. `summary` and `counts` provide
a short reading entrypoint; counts include both directions, rather than a count
of distinct runtime endpoints.

Each `CONTRACT_TO_SOURCE` finding carries the operation's legacy `sourceMapping`
unchanged, a JSON Pointer within its declared OpenAPI file, retained dependency
identity/digest, source references, reader selection and boundaries. Associations
carry the exact entrypoint kind, trigger authority, conditions, dependencies and
source references. Each `SOURCE_TO_CONTRACT` finding carries the same declaration
and matching contract references. Service, revision and immutable snapshot bind
each finding; the service's `sourceReferences` table contains exact file/line and
text/evidence digests without copying full source text.

`pointerRole` distinguishes a direct operation from a path item requiring
reference resolution. `logicalOperationPointer` identifies the compared operation;
`jsonPointer` is null if its retained declaration location is unavailable. A
resolved operation is never given an invented exact pointer into another file.

| Status | Meaning |
| --- | --- |
| `DECLARATION_MATCH` | A retained declaration has the same supported method/path. |
| `CANDIDATE_MATCH` | A source/syntax or otherwise unproven registration is associated; it keeps its original authority. |
| `DECLARED_ROUTE_ONLY` | An observed source declaration has no matching operation in the selected retained contracts. |
| `CONTRACT_ONLY_UNPROVEN` | No comparable captured source route matches the operation; implementation absence is unproven. |
| `AMBIGUOUS` | Distinct registrations or contracts match; conditions are retained and are not evaluated. |
| `UNSUPPORTED_INCOMPLETE` | A route's method/path semantics cannot be compared safely. |

`samePathDeclarations`, `samePathContracts` and `methodDifferenceObserved` expose
method discrepancies independently of the absence frontier. Positive declaration
differences are retained even when other evidence is incomplete. A conditional
controller is never counted as an extra runtime endpoint.

Methods are case-normalized. Literal paths compare exactly, including trailing
slashes. Simple whole-segment `{id}` and `{key}` templates normalize to `{}`;
regex templates, wildcards, placeholders and unsupported path syntax remain
unknown. Structured `sourceRoutes` retain correlated method/path pairs. Multiple
methods and paths without such correlation remain unknown rather than forming
an invented cross-product. An unspecified method is not guessed from OpenAPI.

Scope records, coverage, producer boundaries and unresolved services accompany
the report. `scopeClosure` stays `UNPROVEN`, even if capture coverage is complete.
Production activation, route conditions, servlet/gateway prefixes and request or
response wire bindings are not established by this comparison. Every finding
keeps `wireConformance: UNVERIFIED`: a matched route does not validate query
requirements, serializers, response fields or status codes. External/recursive
reference boundaries remain visible. No whole-service or runtime PASS follows.
