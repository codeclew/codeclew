# Documentation basic workflows: repair handoff

Base: `c458776eb970b381d1eba4ef6f007a9936df669e`.
Branch: `fix/documentation-basic-workflows`.

## Problem and behavior

A required captured dependency larger than the 49,152-byte Work ceiling could
be omitted forever. Source and retained-operation parts could complete their
omissions; DEPENDENCY could not. Repeated reads or a larger item count could not
resolve `REQUIRED_CONTEXT_NOT_READ`.

The change reuses the retained canonical-JSON chunk engine for captured
DEPENDENCY records, adds `docs work read-dependency-part`, and supplies its exact
recovery input in an oversized omission. Schema, kind, Work, snapshot, ID,
canonical record digest, UTF-8 ranges, fragment digest, cursor and receipt digest
stay bound. Complete contiguous recorded parts unlock only the exact dependency
reference and the manual initial-context gate. Automatic role packet delivery
remains independently required; nothing waives evidence completeness.

Service defaults no longer demand a standalone gap for every unclassified
source callable. Explicit helper authoring/gaps remain admitted, and public
boundaries still require an operation or gap. Raw callables remain in the source
inventory and are selectable from it. Normal navigation lists public boundaries
and authored operations rather than all helper placeholders. This does not
resolve inherited HTTP routes or infer runtime registration.

Internal process discovery is opt-in through `catalog/reader.json` with
`showInternalProcesses: true`, and requires actual candidates. Saved processes
remain navigable without speculative internal cards. Existing typed visuals,
source inventory, exclusion policies and declared process scopes are retained.
The check summary separates callable candidates, public-boundary classification,
actual coverage meaning and projected contract-operation count, with conditional
contract recovery guidance. The short agent recovery runbook explains stage
identity, exact references, cursor consumption and when repeating a command
cannot make progress.

## Verification

Observed: 66 reader DOM tests passed; two optional retained-case tests skipped.
JavaScript syntax and diff-whitespace checks passed. Focused Rust regressions
were added for a synthetic dependency over 200 KiB, actual default-page omission,
recorded bounded read/resume, exact reconstruction, gate completion, corruption,
missing parts, changed snapshot, cross-Work cursor and service isolation. Another
Rust regression covers optional helper gaps versus mandatory public boundaries.

Rust/Cargo 1.92 are unavailable in this cloud environment. Rust compilation,
formatting and these Rust tests are pending; DOM tests cannot establish native
Work acceptance. No provider calls, private source publication, native replay of
the supplied private service, real browser geometry or full CI test ran.

The reported installed version is 0.13.25; this public base declares 0.13.23.
Matching Work/reader code explains the bounded-record failure, but do not claim
that this branch reproduces the entire installed release. In this checkout
source acquisition already invokes contract capture: a successful retained
refresh is recovery evidence, not proof that `check` never projects contracts.

## Required integration checks

```sh
cargo fmt --all
cargo fmt --all --check
cargo test --locked -p clew --lib documentation::work_retained_parts -- --test-threads=1
cargo test --locked -p clew --lib documentation::work_parts -- --test-threads=1
cargo test --locked -p clew --lib documentation::work:: -- --test-threads=1
cargo test --locked -p clew --lib documentation::proposals -- --test-threads=1
cargo test --locked -p clew --lib documentation::render::process_catalog_tests -- --test-threads=1
node --test scripts/test_documentation_visual_reader.cjs
python3 -I -S scripts/check_english_content.py
python3 -I -S scripts/check_repository_privacy.py --worktree
```

Resolve compilation/formatting/test failures before integration. Qualify the
same change on the actual release checkout and replay the private service
through supported capture/Work/proposal commands. Confirm large dependency
parts permit submission without changing maxBytes, disabling gates, dropping
contract boundaries or inventing receipts. Confirm default documents avoid
helper gap dumps while public operations, authored helpers and raw inventory
remain accessible. Include generated interface source to qualify inherited
routes; do not relabel all controller methods as HTTP entries.

Run normal full CI when preparing a merge. Keep the private transcript, service
source and source-specific work/receipt identifiers out of public commits.
