# nav query: source-bound visual walkthrough

The public page is [How nav query works](https://codeclew.github.io/codeclew/nav-query.html).
Its primary artifact is a selectable static flow; every node and arrow opens a
claim with exact source fragments. The downloadable
[Mermaid graph](../../site/diagrams/nav-query.mmd),
[SVG](../../site/diagrams/nav-query.svg), and
[public evidence](../../site/evidence/nav-query.json) describe the same slice.

## Authority and scope

- Analyzed revision: `64819fdb1b64a66b98cbbafcd0a68322ef818006`.
- Extractor: installed Codeclew `0.3.1`, RELEASE, `rust-syntax`.
- Targets: `cargo:crates/clew/Cargo.toml#clew#bin#clew` and
  `cargo:crates/clew/Cargo.toml#clew#lib#clew`.
- Entry: `Command::Nav / NavCommand::Query` in `main.rs`, followed into
  `nav_query`, admission/context assembly and the navigation decision.
- Nine agent-authored claims, eight graph nodes, nine graph arrows and 21
  exact source fragments. The optional reference-follow claim is available in
  the inspector without implying that it runs on the default path.

This is **syntax-backed documentation**, not a compiler-resolved Rust call graph
or runtime trace. The agent interprets the static links. Every graph edge is
labelled `AGENT_INFERRED_STATIC_FLOW` in the machine-readable artifact. Unknown
name resolution and cfg/macro expansion remain explicit. Readiness internals,
individual adapters, snapshot/cache internals and cleanup recovery are outside
the slice. The documentation does not claim that every error path is drawn.

The real example returned one candidate with
`SUPPORTED / UNIQUE_EXACT_IDENTIFIER_FULL_COVERAGE`; context certainty remained
`UNSURE`, coverage `QUERY_COMPLETE`, and status `CONDITIONAL_TASK`. This is a
single observed example, not a quality or performance benchmark. The initial
analysis used the clean committed snapshot. A later independent example used
`--committed` after documentation edits; those edits were excluded.

## Reproduce extraction

Use a fresh clone so this recipe does not affect an existing developer checkout:

```sh
git clone https://github.com/codeclew/codeclew.git codeclew-doc-demo
cd codeclew-doc-demo
git switch -c docs-example 64819fdb1b64a66b98cbbafcd0a68322ef818006
clew --version
clew doctor repository --repo "$PWD"
```

The CLI needs a local branch or tag for admission; a bare commit SHA is not an
accepted target-ref. The branch above pins the intended source revision.

For the small demonstrated query:

```sh
clew nav query --repo "$PWD" --target-ref docs-example \
  --language rust --profile rust-syntax \
  --compilation 'cargo:crates/clew/Cargo.toml#clew#bin#clew' \
  --term nav_query --decision-identifier nav_query --source
```

For the documentation extraction, open both relevant targets and keep the
returned `sessionId`, `contextId`, `baseRevision` and `evidenceDigest`. Save raw
responses in caller-owned private files, outside the publication tree.

```sh
clew context open --repo "$PWD" --target-ref docs-example \
  --language rust --profile rust-syntax --operation analysis \
  --compilation 'cargo:crates/clew/Cargo.toml#clew#bin#clew' \
  --compilation 'cargo:crates/clew/Cargo.toml#clew#lib#clew' \
  --intent 'Document nav query from its CLI entrypoint with source-bound visual flow' \
  --term NavQueryArgs --term nav_query
```

Then use the current returned handles for these bounded operations:

1. Select `nav_query` with `nav expand --term nav_query
   --file crates/clew/src/main.rs --source`.
2. From its retained candidate, follow `admit_and_open_context`,
   `clew::navigation::query_with_decision_identifier`, and
   `clew::navigation::agent_card` together using repeated `--reference`.
   Retain the child context from `navigation.referenceFollow`.
3. Select the now-observed `assemble_with_decision_identifier` in
   `crates/clew/src/navigation.rs` with `--term`, `--file`, and `--source`.
4. Follow its retained `navigation_decision_authority` reference with `--source`.
5. Select `run` in `crates/clew/src/main.rs` with `--term`, `--file`, and
   `--source`; retain only the `NavCommand::Query` dispatch arm for publication.

Every expand call also takes `--session <returned-session-id>` and
`--from <newest-context-id>`. Do not reuse an old context when a follow operation
returns a child. Returned exact source proves the declaration text; following a
retained reference does not prove Rust name resolution.

Close each completed session through `clew session close --session <id>`.
Retain evidence while it is needed; use only supported session GC afterward.

## Author and verify the publication

The agent records one claim per user-visible decision or outcome. A claim's
source fragments are exact subsets of returned windows, never paraphrased code.
The public JSON deliberately contains selected Codeclew source and provenance;
it is not a dump of private sessions or runtime state.

`fileDigest` is the Codeclew CAS digest of the full source blob, including its
schema domain. `textDigest` is the SHA-256 of the displayed UTF-8 fragment with
LF separators and no trailing newline. These digests bind bytes; neither is
proof of the narrative's semantic correctness.

The renderer does not invoke an agent or re-extract code. It verifies the
public fragments against the pinned Git revision and generates the diagram and
claim panels from `site/evidence/nav-query.json`:

```sh
python3 -I -S scripts/build_cli_documentation.py
python3 -I -S scripts/build_cli_documentation.py --check
```

Regeneration requires Graphviz `dot`; validation needs only Python and Git.
The checked-in SVG renders without a third-party script or runtime network
dependency. The page supports node/arrow selection, keyboard navigation,
claim deep links, an evidence detail mode and a text fallback without JavaScript.
CI and the Pages workflow validate the source/graph bindings before publication.

This document is pinned, not automatically fresh. To update it, select a new
revision, extract fresh evidence through Codeclew, review affected claims and
regenerate the artifacts. A successful byte check at the old revision is not a
semantic freshness check against current main.
