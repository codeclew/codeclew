# Durable service documentation

Use this workflow for a separate documentation repository with per-service pages,
a root overview and named interaction slices. It is available in Codeclew 0.5.0
for Java 17+ Maven/Gradle read-only profiles, one compilation per service. Each
scenario connects at most two services. The overview can list more services.

## Start or recover

Resolve the installed `clew` launcher once. `clew docs` owns source admission,
compiler capture and cleanup; do not open a second navigation/session workflow.
If admission fails, retain the reported gap and next action. Never silently
substitute raw source analysis for compiler-backed evidence.

```sh
clew docs init --root /work/architecture --title 'System architecture'
clew docs service add --root /work/architecture --input /work/service.json
clew docs bind --root /work/architecture --service orders --repo /work/orders
clew docs interaction put --root /work/architecture --input /work/interaction.json
clew docs check --root /work/architecture
clew docs context --root /work/architecture --service orders --limit 100
```

`init` writes editable examples under `examples/`; adapt their repository,
selector, profile and compilation before registering them. Set the repository
identity to its credential-free Git remote, and bind an existing checkout with
the selected target ref at HEAD. Keep the documentation root separate from every
source checkout. Commands do not change application source.

For cold recovery, read `codeclew-docs.yaml`, `catalog/services/*.json`,
`catalog/interactions/*.json` and `scenarios/*.yaml`, bind the relocated checkouts,
then run `docs check`. Commit the catalogue, scenarios, manual notes and generated
bundle to the documentation repository. `.codeclew/` holds ignored local paths
and disposable compiler cache; it is not needed for recovery. Published
`docs/generated/<bundle>/bindings.json` retains narratives and their dependencies.
`docs context` returns retained operations as authoring input, explicitly without
re-verifying their interpretation.

Service context without `--entrypoint` is the catalogue. Follow every
`nextCursor` with the same selection and `--cursor`, then request one exact
returned entrypoint ID at a time:

```sh
clew docs context --root /work/architecture --service orders --entrypoint RETURNED_ID --limit 100
clew docs context --root /work/architecture --scenario checkout --limit 100
```

Follow all pages needed for the selected operation. `omitted` means an item
exceeds the stdout budget, not that it is absent. Narrow the operation or report
an actionable gap. Context normally reuses the latest check; `--refresh` rebuilds
source evidence. Retained context is not a claim about current checkout bytes.

## Declare and explain

Interactions record engineer assertions. `origin` is `human`, `imported`, or
`agent-proposal`; proposals do not establish a traversable cross-service edge.
Use `interaction candidates --input ...` to inspect exact selectors before
saving an ambiguous declaration. Preserve origin, rationale, applicability,
stable IDs, and incomplete selectors rather than inventing missing identities.
`--expected-input-digest` on add/put prevents lost updates when editing existing
records; `interaction remove` requires it and refuses referenced declarations.
Scenario YAML remains manually editable and selects explicit interaction IDs.

A matching route is not runtime proof. Keep declaration origin, resolved caller
and receiver, call site, HTTP method/path, destination configuration key,
contract status, environment and runtime as separate certainty axes. The first
HTTP adapter recognizes resolved Spring RestTemplate literal method/path calls
and `@Value` destination keys. Unknown clients, dynamic routes, external calls,
unsupported control flow and runtime configuration remain explicit boundaries.

Write the explanation from returned source, then submit a closed JSON Narrative
using [the authoring example](authoring-example.md). Every discovered entrypoint
needs a detailed operation or an explicit actionable gap. For full-service
requests, do not silently stop after one representative endpoint. Include the
supported guards, alternate outcomes, loops, failures and state changes. Branch
markers must balance; every selected source condition and return needs a bound
corresponding event. A count check cannot establish semantic fidelity: read the
predicate and outcome, and keep their actual nesting/order in the narrative.
Do not convert unsupported branches into a linear happy path.

Source/dependency IDs must come from the current context. Every arrow/node keeps
its own source binding; cross-service arrows also name the declared interaction.
Local calls are compiler observations; cross-service transitions remain declared;
narratives are agent-inferred. Versioned OpenAPI 3.0 files supply contract facts,
including local references and constraints, separately from code behavior.

```sh
clew docs render --root /work/architecture --input /work/orders-narrative.json --input /work/inventory-narrative.json --input /work/checkout-narrative.json --require-complete
```

Render rechecks source before atomically publishing `docs/index.html` and an
immutable bundle with overview, service/scenario HTML, JSON, Markdown, diagrams
and bindings. Without `--require-complete`, explicit gaps are allowed and visible.
Manual content belongs outside `docs/generated/`; modifications to generated
outputs cause a conflict. Review readable summaries, branches, contracts and
clickable source inspectors. HTML is self-contained and requires no model API,
CDN or external diagram renderer. Source links need repository access; exact
retained snippets remain viewable offline.

## Refresh only affected explanations

`docs check` rebuilds current source evidence and compares semantic dependencies
with the portable baseline. Exit 0 means CURRENT; 4 means PARTIALLY_STALE/STALE;
3 means UNRESOLVED, including a missing baseline or source binding. Other invalid
inputs and conflicts use the normal CLI error codes. Large reports have
`items` and `nextCursor`; retain the top-level freshness status on every page.

Read `affected`, `unaffected`, `linkChanges` and coverage/catalogue changes.
Line movement alone refreshes source links without requiring prose changes.
Changed methods, routes, contracts, callees or declarations identify dependent
fragments. Re-author affected operations against the new context digest, keeping
unaffected text and engineer declarations. Rendering refuses stale retained
narratives until reviewed replacements are supplied. A digest detects changes;
it does not prove an agent's explanation. Missing history/source stays
UNRESOLVED. No hosted LLM or embedded API key is part of this workflow.
