# Source part read measurements

This synthetic CLI matrix covers ASCII (8 KiB), mixed Unicode and JSON-escaping text (~12 KiB), one 64 KiB line without a newline, empty text, 32-byte text, and a 1 MiB ASCII source. The first five cases run at Work budgets 2,048, 40,960, and 49,152 bytes; the 1 MiB case runs at 49,152 bytes. Every part was read through the public `docs work read-part` CLI and each response was measured from raw stdout including its final LF. `results.json` records canonical full-SOURCE bytes/digests, text bytes, stdout sums/maxima, ledger growth, retained-load and acquisition spans, trap attempts, and exact canonical record reconstruction. `modelInvocationCount` is inferred from unchanged persisted author job and attempt counts; the read-part command accepts no provider configuration. `proposal-flow.json` and `source-author-guard.json` preserve proposal-completion and zero-dispatch evidence; `non-source-failure-publication.json` records the unchanged non-SOURCE failure path.

Reproduce and refresh this opt-in artifact set from the repository root with:

```sh
CODECLEW_SOURCE_PART_ARTIFACT_DIR="$PWD/docs/product/validation/l1-source-parts-20260926" cargo test --locked -p clew --test docs_work_source_parts -- --test-threads=1
```

The CLI continues to hydrate each immutable Work Check in full. This matrix makes no claim about source-memory use, end-to-end author context, or performance. Tests do not write these checked-in samples unless `CODECLEW_SOURCE_PART_ARTIFACT_DIR` is set.
