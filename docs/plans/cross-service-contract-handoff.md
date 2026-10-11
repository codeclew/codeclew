# Cross-service contract and receiving-fragment handoff

## Base and scope

Base commit: `1778bd13e7c26d3c7775ab4812c05fbf87e3ad5d`.
Working branch: `feat/cross-service-contract-fragments`.

This slice extends the existing saved `process.thread`, exact retained-source
admission, native HTML/Markdown projection and reader. It does not create a
parallel process generator, invoke an authoring provider, change entity pages,
qualify runtime behavior, or publish a site. Provider authoring remains at its
existing exhausted limit; no author/reviewer calls ran.

## Implementation

- Optional `serviceFragments` identify a process-scoped fragment by ID, selected
  entry method and actual root step. The normal process selector resolution and
  source admission are reused. Receiving subtrees must stay within their service;
  their projected method/source set excludes caller methods.
- Optional `boundaryContracts` bind an explicit call step to its explicitly
  targeted receiver fragment, or leave a receiver frontier. Request, response,
  failure and guarantee claims require evidence references or a minimal missing
  capture. Evidence binds service/revision/source/digest/inclusive lines and
  checks retained bytes; admission does not approve business meaning or wire
  compatibility. HTTP and Kafka are currently the admitted transport labels.
- Process pages offer in-place receiver expansion and service-fragment links.
  Service pages reuse the admitted subtree and link back to its process step.
  Source reload hashes include the fragment owner instead of guessing among
  multiple fragments. Selected sources remain in one closed source block.
- Service fragment bindings include process dependencies and exact sources, so
  freshness is independent of unrelated service section review. Contracts and
  service links also survive Markdown projection. Renderer identity already
  includes the changed thread implementation and reader assets.
- The process schema now admits its optional thread and references the added
  thread schema. Old saved threads omit the new collections on serialization.
  Older closed-schema consumers require coordinated upgrade.

## Verification and limits

Observed here: 62 reader tests passed, two optional retained-case tests skipped;
JavaScript syntax check, English content check, repository privacy worktree
check and `git diff --check` passed. Added Rust assertions exercise evidence
revision/digest/range drift, missing claims, fragment projection/return links
and old-input behavior, but they have not run in this environment.

Rust/Cargo/rustfmt are absent. An attempt to prepare package tooling failed
because the environment could not change package-manager user/group identity;
no escalation or toolchain substitution was used. This is reviewable source,
not a merge-qualified build. No native private capture/recompose/render replay,
real browser geometry/accessibility test, runtime request, compiler-worker or
full CI test ran. A separate private reference fixture is not native proof.

The declarations express evidence-backed interpretations; they do not implement
a native DTO/framework contract extractor. Full wire equivalence, environment
routing, transaction activation/commit and broker acknowledgements remain
separate captures. Claim completeness and semantic correctness are unproven.

## Integration checks

```sh
cargo fmt --all
cargo fmt --all --check
cargo test --locked -p clew --lib documentation::process -- --test-threads=1
cargo test --locked -p clew --lib documentation::render::tests -- --test-threads=1
cargo test --locked -p clew --lib documentation::render::process_catalog_tests -- --test-threads=1
cargo test --locked -p clew --lib documentation::source_span -- --test-threads=1
node --test scripts/test_documentation_visual_reader.cjs
python3 -I -S scripts/check_english_content.py
python3 -I -S scripts/check_repository_privacy.py --worktree
```

Resolve Rust compilation/formatting/test failures before integrating. Use the
normal full CI gate when preparing a merge. Obtain fresh retained evidence for
any private DTO/configuration/advice spans before filling contract bindings;
never fabricate IDs from the reference fixture or edit generated CAS output.
Then use ordinary `docs recompose` and `docs render --snapshot HANDLE`.

Qualify process → receiving service → process, expanded ancestors, exact source
focus, reload hashes, Back, catalogue/coverage return, RU/EN and narrow/wide
layouts in a real browser. DOM tests cannot qualify geometry or accessibility.
Keep private fixtures and source bytes out of the public Git tree/site.
