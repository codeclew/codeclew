# Multi-compilation documentation

How one documented service selects several explicit compilation scopes from a
single repository, and how outbound HTTP egress is attributed. This guide uses
synthetic names; the same rules apply to any real repository. It is an
operational reference, not an authorization to run customer builds or touch
customer caches.

## Selecting multiple compilation scopes

A documented service declaration selects build-authoritative module/source-set
compilations. Each selector names a Maven reactor module (or the repository
root) and a source set, and a service may list several:

```json
{
  "schema": "codeclew-documentation-service/1.0",
  "id": "orders",
  "language": "java",
  "profile": "java-17plus-maven-read-only",
  "compilations": [":/web:main", ":/flow:main", ":/common:main"],
  "targetRef": "main"
}
```

Rules:

- A selector is a build-authoritative module/source-set (Maven `main`/`test`),
  not a directory glob. Per-compilation classpath, JDK, processors, source
  state, and dependency authority are retained.
- A legacy singular `compilation` (e.g. `:/main`) normalizes to a one-element
  set. Declaring both `compilation` and `compilations` is rejected.
- Empty selections, duplicate selectors, and more than 128 selectors are typed
  validation errors.
- Test scopes are opt-in and never become production entrypoints unless
  explicitly selected by a suitable view.

## Scope-aware evidence

When more than one scope is selected, each captured fact carries its admitting
compilation scope. Projection retains scope-distinct observations and source
records instead of last-write-wins overwriting, so the same symbol or source
path under different source sets or classpath authority is never silently
collapsed. A symbol admitted under incompatible scope candidates surfaces an
explicit `SCOPE_AMBIGUOUS` boundary; identical candidates across scopes are not
ambiguous. Byte-identical payloads are stored once while retaining every
contextual membership.

Cross-module calls are resolved through the admitted classpath/dependency
relationships: a web body delegating to a flow body that uses a common body
exposes the matching admitted bodies and their egress facts. Callback,
reflection, runtime DI, third-party engine execution, and egress reachability
are never invented simply because a library body is in scope.

## Outbound HTTP egress guidance

Spring `RestClient` fluent chains are detected from their resolved method
authority. A standard executed chain yields exactly one usable egress:

```java
client.post().uri("/orders/{id}", id).retrieve().body(String.class);
// adapter: SPRING_REST_CLIENT_URI/1.0, method: POST, path: /orders/{id}
```

- A relative literal path (or URI template) is kept as the path; a literal
  absolute URL is split so its authority never masquerades as a route path. URI
  templates remain templates, never exact concrete routes.
- `method(HttpMethod.CONSTANT)` resolves the verb statically.
- A configured client or an unexecuted request specification (for example
  `client.post()` without a resolvable `uri`) produces no egress.
- `RestTemplate` literal-suffix extraction (with `@Value` destination config
  keys) is preserved. Unrelated fluent lookalikes named `get`/`post`/`uri` are
  not confused with a Spring client.

## Indexed membership and atomic deltas

Current and historical membership is kept in a versioned paged index over the
immutable object store. Updating or removing one fact rewrites only the affected
index page and the small current root, not every payload. Removal is logical:
the payload objects remain for other memberships and historical snapshots.
`reclaimable` reports unreferenced payloads and bytes read-only; no physical
deletion, pack compaction, or expiry is performed by this run.

## Scale evidence

Enabled tests in `crates/clew/tests/multisource_evidence_regressions.rs` build a
synthetic web/flow/common reactor with >=1,300 distinct generated source records
and tens of thousands of fact memberships. They assert deterministic storage
behavior (unique-payload storage, one-page point lookups, bounded single-fact
deltas, observable logical removal) and a real JDK 21 compiler-to-docs RestClient
pass. Compiler behavior and storage scale are measured separately.

## Boundaries

Same-repository/revision and supported service language/profile are the initial
scope. External source repositories, arbitrary custom Maven source sets, and
mixed-language projects are explicit boundaries, not inferred support. A common
library is a source scope, not a new deployed service. Real-customer rollout is a
separate explicit task; this run does not run its Maven build or touch its
caches.