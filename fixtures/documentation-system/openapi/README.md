# Independent declared OpenAPI fixture

`api.yaml` tests OpenAPI 3.0.3; the CLI regression also runs a 3.0.0 document.
`types.yaml` is an explicitly registered local reference outside language roots.
The no-tool CLI fixture checks nested constraints, parameter overrides, responses,
security inheritance/override, servers, exact committed source occurrences and
unmatched declaration rendering. Additional mutations exercise cycles, missing
pointers, unregistered files, unsafe paths, network references, malformed YAML,
unsupported 3.1.0, missing registered files, source-route mismatch and contract-only
invalidation of service and process fragment dependencies.

These are static declarations. No OpenAPI runtime validator or deployed service
is qualified by these tests; callbacks remain declarations without source mapping.
