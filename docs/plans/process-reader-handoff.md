# Process reader handoff

## Goal and branch

Continue the source-bound end-to-end process reader work on
`feat/process-reader-clarity`. Base develop was
`fc2d6fa7c0f52602c126f216eeb970ab906d77c6`. The preceding local slice restored
native `process.thread`, exact method/source validation, Markdown source closure,
Kotlin root lookup, a declared interaction index and a reader with collapsed
source/diagnostic sections. This follow-up adds explicit branches/effects and a
bounded extraction API. The pushed branch is the source of truth; do not assume
local commit IDs equal connector-created remote commit IDs.

## What changed

- `documentation/process_thread.rs`: IF_ELSE, FIRST_MATCH and INDEPENDENT branch
  arms, cited conditions, explicit default shape and selected effect kinds.
  Global IDs/counts, recursive citations and old-input serialization are checked.
  Branch coverage and effect completeness remain NOT_PROVEN.
- `documentation/process_extract.rs`: pure bounded query packet over the retained
  common observation model. It records source references, guards/errors, local
  association edges, structural outcome paths, independent frontiers and exact
  cache/freshness inputs. It never executes a model or target code.
- `documentation/process_graph.rs`: optional bounded collector reusing the
  existing complete graph's association policy; full graph behavior is retained.
- `docs process extract`: snapshot + query JSON -> atomic packet file. Saved
  `process.extraction` attaches the same packet to native HTML and Markdown after
  exact-root checks. The authored thread remains the explanation.
- Reader: alternatives appear as separate arms, selected effects have citations,
  packet decisions and UNKNOWN/budget frontiers are visible. Labels stay escaped.
- Fixtures: cyclic calls, errors, unknown transport, ambiguity, damaged source,
  budget loss, freshness and no authority upgrade across provider labels.

See [the contract](../operations/docs-process-threads.md),
[extraction API](../operations/process-extraction.md) and
[fixture benchmark](../product/validation/process-envelope-fixtures.md).

## Reproduction

Use the repository's pinned toolchain and normal source-development environment:

```sh
cargo fmt --all --check
cargo test --locked -p clew --lib documentation::process -- --test-threads=1 --nocapture
cargo test --locked -p clew --lib documentation::render::tests -- --test-threads=1
cargo test --locked -p clew --lib documentation::render::process_catalog_tests -- --test-threads=1
cargo test --locked -p clew --lib documentation::source_span -- --test-threads=1
node --test scripts/test_documentation_visual_reader.cjs
python3 -I -S scripts/check_english_content.py
python3 -I -S scripts/check_repository_privacy.py --worktree
```

For a retained private case, set `CODECLEW_PROCESS_CASE_DIR` to the scenario JSON
directory for the reader test. The case itself is outside the repository and must
not be committed. Use ordinary `docs recompose` after changing declarations, then
`docs render --snapshot HANDLE`; do not edit private CAS objects or weaken
freshness. New extraction inputs use exact root observation IDs from that Check.

## Limits and honest plan status

The implementation slices are bounded prototypes, not a complete semantic slicer.
Backward relevance is call membership; alias/data/effect transfer, path feasibility,
transaction/runtime closure and cross-service ownership are not solved. Future
qualified semantic summaries can refine this profile without changing authority
of old packets. No thread-shaped output proves absence of a bug or a strict FSM.

The reader and native pipeline can be verified deterministically. Real browser
geometry/accessibility qualification is still outstanding: the cloud browser
rejected filesystem navigation and no installed headless browser was available.
Do not mark screenshots or mobile geometry verified based on DOM tests. Use an
approved browser surface/environment for the next visual run.

The small benchmark records fixture byte counts and lost coverage at a budget
cut. Manual reading time, model tokens, monetary savings and multi-agent wall
time have not been measured; no paid model benchmarks ran. Do not relabel these
unmeasured baselines as passed. Full CI/compiler-worker qualification is separate
from the focused Rust/reader tests; no worker code changed in this slice.

## Verification at handoff

Focused checks passed: 65 process tests, 23 renderer tests, one source-span test,
and 58 reader tests with the retained case (one optional test skipped). The full
CI command was attempted and stopped at preflight because .NET SDK 10+ is absent.
Compiler-worker and full CI qualification are not established by these results.

The archived Task case has callable observations without exact retained scope.
The extraction API correctly rejects those roots; it is not a successful empty
packet. Its declared branch/effect presentation remains independently usable.
A new qualified capture is required for that archive's automatic packet replay.
Earlier archived SQLite replay also reported malformed database state; its cause
is unresolved and no private CAS object or freshness check was weakened.

## Next actions

1. Run a real browser qualification for narrow/wide layouts, keyboard focus,
   catalogue/coverage return, RU/EN, lifecycle and legacy processes without thread.
2. Review authored conditions and effects against exact source, including generic
   updates, caller status overrides, mutually exclusive finalize arms and separate
   Task/Run/Chunk states. Machine acceptance does not replace this judgment.
3. For richer automatic slicing, admit qualified data/control/effect summaries
   through the common adapter capability contract. Preserve semantic vs budget
   frontier and configuration/runtime uncertainties; do not assume FIR-only facts.
4. Before claiming agent savings, run a budgeted human/model comparison on a frozen
   corpus with correctness, coverage, calibration, tokens and latency. Independent
   cuts and shared interface summaries are prerequisites for parallel fan-out.

These are engineering follow-ups, not an additional approval layer.

## Final delivery task

Commit the implementation and this handoff, then push `feat/process-reader-clarity`.
Verify the published tree matches the reviewed local tree. The next Codex session
should start from this branch and this document, reproduce focused checks, and
continue the open qualification tasks above. Keep private source archives out of
the public repository.
