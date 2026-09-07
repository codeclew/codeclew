# Durable documentation acceptance fixture

Two independent Java 17 Maven services exercise checkout and inventory
reservation. Both use Spring HTTP entrypoints. Orders uses RestTemplate and the
`inventory.base-url` configuration key. The versioned OpenAPI contracts include
quantity constraints. `architecture/` contains portable engineer declarations;
repository locators are intentionally synthetic and have no external service.

The CLI regression copies these sources into temporary Git repositories, runs
native Maven/javac through admitted Codeclew capture, and authors complete
fixture narratives with guards and returns. It checks source preservation,
repeat rendering, a changed receiver route, affected diagram/contract fragments,
and successful recovery after relocating both repositories with a fresh home.
It also rejects an explanation that omits a source condition. The authoring
input is deterministic fixture data, not an evaluation of a live model.

From the product source checkout, export the initial current pages for visual
review by choosing a new absolute output directory:

```sh
CODECLEW_DOCS_EXAMPLE_OUTPUT=/tmp/codeclew-docs-example cargo test --locked -p clew --test managed_cli durable_documentation_cli_recovers_and_reports_route_fragments -- --exact --test-threads=1
```

Open `docs/index.html` inside that output. It links both service pages and the
checkout scenario. Select a sequence step to inspect its exact source; the
contract and findings tabs keep interface facts and explanation separate. The
export contains only the synthetic documentation bundle and its three narrative
inputs, with no private runtime state. A second run needs a different output
path. External source URLs use `example.invalid`; the embedded source inspector
works offline.

For real use, install the public release and follow the packaged
[service documentation workflow](../../skills/codeclew/references/service-documentation.md).
