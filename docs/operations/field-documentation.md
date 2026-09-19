# First useful documentation and selective refresh

This workflow separates an initial source explanation from optional compiler
analysis. Uppercase values below are returned handles or operator-selected values,
not literal values to copy unchanged. Use installed `clew`, or `./clew` when
developing this checkout. See [snapshot storage](docs-snapshot-store.md) for pins
and retention, and [source profiles](source-documentation.md) for evidence limits.

## First useful document without Maven

Use a separate documentation root and one explicitly selected service. Register a small committed source scope containing the worker, relevant helpers and configuration. Example `/work/orders-source.json`:

```json
{
  "schema": "codeclew-documentation-service/1.0",
  "id": "orders",
  "title": "Order worker",
  "repositoryId": "orders",
  "repository": "https://example.invalid/orders",
  "language": "java",
  "profile": "source-syntax",
  "targetRef": "FULL_COMMIT_SHA",
  "source": {
    "roots": ["src/main/java/example/worker", "src/main/resources"],
    "dialect": "17"
  }
}
```

Replace URL, commit and paths with actual values. Omit `modules` for the first capture. Optional `contractFiles` is an explicit array of committed OpenAPI files, independent of these roots.

```sh
clew docs init --root /work/architecture --title 'Order worker documentation'
clew docs service list --root /work/architecture
clew docs service add --root /work/architecture --input /work/orders-source.json --expected-input-digest INPUT_DIGEST
clew docs bind --root /work/architecture --service orders --repo /work/orders
clew docs modules list --root /work/architecture --service orders
clew docs check --root /work/architecture --service orders
clew docs context --root /work/architecture --service orders --snapshot SOURCE_SNAPSHOT --format compact --limit 100
```

Use the latest list/show `inputDigest`, including `sha256:`, for each catalog mutation; save `snapshot` from check. If evidence-selection/update policies were already configured, inspect them first: an admitted external evidence package may be selected ahead of local capture. This example assumes a new root with no such policy.

Select one method from returned evidence. `docs process candidates --root /work/architecture --service orders --snapshot SOURCE_SNAPSHOT --declaration SYMBOL_OBSERVATION_ID` supports an explicit callable root even where automatic discovery is incomplete. A candidate is navigation, not accepted business meaning. Create a process definition using the existing process schema/fixture: exact service/selector, explicit requested scope, participants, trigger/outcomes, bounded `maxDepth`/`maxNodes`, and only declared interactions. Source methods must use the selector values exposed by retained evidence rather than guessed JVM identities.

```sh
clew docs process inspect --root /work/architecture --input /work/process.json
clew docs process list --root /work/architecture
clew docs process put --root /work/architecture --input /work/process.json --expected-input-digest INPUT_DIGEST
clew docs recompose --root /work/architecture --snapshot SOURCE_SNAPSHOT
```

`inspect` consumes latest compatible retained evidence. `put` changes declarations, so use the returned recomposed snapshot for authoring; recomposition executes no analyzer and does not update latest. Do not use default `process prepare` here: it selects latest, which predates the added declaration.

Prepare `/work/request.json` as `{"schema":"codeclew-documentation-work-request/1.0","audience":"Worker maintainers","entrypoint":"process-overview","contextProfile":"process-v1","maxItems":100,"maxBytes":40960}`. Use the ID authored in the process definition:

```sh
clew docs work prepare --root /work/architecture --subject scenario:PROCESS_ID --input /work/request.json --snapshot RECOMPOSED_SNAPSHOT
clew docs work read --root /work/architecture --work WORK_ID --input /work/selection.json
clew docs proposal submit --root /work/architecture --work WORK_ID --input /work/proposal.json
clew docs proposal publish --root /work/architecture --proposal PROPOSAL_ID --unassessed
```

The Work result provides evidence handles; `selection.json` names those handles, and a human/current agent authors the schema-constrained proposal from recorded reads. This is real authoring work, not an automatic consequence of capture. Alternatively, `docs work run --root /work/architecture --work WORK_ID --config /work/execution.json` uses an explicitly configured author/reviewer; do not invent provider credentials or assume it runs free. Local publication remains `UNASSESSED`; configured meaning review is separate.

If a saved-evidence render is needed, `clew docs render --root /work/architecture --snapshot RECOMPOSED_SNAPSHOT` uses it without capture. Keep the returned frozen publication ID/path. Pin the source parent with `clew docs snapshot pin --root /work/architecture --name orders-source-first --snapshot SOURCE_SNAPSHOT` before changing service configuration.

## Explicit compiler enrichment afterward

Keep the same service ID, `profile: "source-syntax"`, source roots/dialect and fixed commit. Add this object to the complete service JSON, then update through `service show` / `service add` with its current input digest:

```json
"modules": {
  "schema": "codeclew-documentation-modules/1.0",
  "semantic": {
    "module": "javac",
    "enabled": true,
    "profile": "java-17plus-maven-read-only",
    "compilation": ":/main"
  }
}
```

Use the actual qualified profile and compilation selector; writable/AP projects require their own explicit supported profile/admission. Configure the provider only through `modules.semantic`. Then run `clew docs check --root /work/architecture --service orders`. This is synchronous and may run Maven/compiler work for that provider's selected compilation and its build dependencies. It retains compatible saved sibling services; it does not guarantee no reactor work. Ordinary `context`, Work and `render` consume saved evidence; avoid `--refresh` and unscoped `docs check` when no acquisition is intended.

This is a new capture, not an in-place enrichment of the old snapshot. Current code reparses source when semantic execution is enabled, then attaches only unique equal-revision/name/file/line `SEMANTIC_SYMBOL` observations. Source identities remain source-based, and lexical FLOW targets are not upgraded. Provider failure is explicitly recorded as unavailable while source evidence remains readable in this intentionally selected source profile. A native-only failed capture is never silently substituted with syntax evidence.

## Profile changes and historical access

Any service-record change changes its digest and overall catalog input. Enabling the semantic module therefore invalidates ordinary current consumers of the previous check. `Check::retained` enforces current catalog equality even for explicit `--snapshot`; `recompose` rejects service/profile/module/root changes. The old bytes are retained, but do not promise that `context/render/work prepare --snapshot OLD` works under the changed catalog. Existing pins can be checked with `docs snapshot show`; frozen publications remain available through `docs history list/show` and their generated files. Finish and save the source publication before changing the service record; prepared old work also cannot be published against incompatible current declarations.

Changing the top-level profile to native additionally requires removing `source`/source-only modules and uses native identities; there is no automatic mapping of authored source process roots. Prefer the optional module path when source-root continuity is wanted. A fixed commit and equal line ranges still do not map transformed-source semantics automatically.

The first document must state `SYNTAX`, unresolved call targets, lexical ordering and unknown runtime activation. Capture is bounded to 2,048 files, 2 MiB/file and 32 MiB source scope; narrowing scope is explicit, never silent omission. Measure time to retained evidence, time to first useful authored process and answers to reader questions separately from later compiler enrichment. This workflow avoids initial Maven waiting; it does not resolve AP input closure or replace the independent zero-repeat-analysis requirement.

## Release boundary

Version 0.10 requires a new documentation root and fresh indexing. Old private
state, bindings and narrative formats are not imported or migrated. Keep old
roots separately if needed; initialization does not delete them. Current-format
snapshots, Work and explicit pins continue to share retained objects.
