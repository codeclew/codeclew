# Isolated deterministic documentation drivers

`driver.py` is a test transport, not a model integration. The public CLI tests
copy it outside source/documentation roots, register the exact file and Python
runtime, and launch it through `macos-seatbelt-stdio/1.0`. It emits the versioned
transport envelope and deterministic proposal/review/expansion results. Options
exercise rejection, replay, false authority, missing/excessive usage, stalled or
oversized output, and real attempts at prohibited filesystem/tool operations.
No provider credentials or paid requests are used.

See `docsys_t04_*` in `crates/clew/tests/documentation_system.rs` for executable
configuration examples and assertions. Access-denial tests are macOS-specific;
missing configuration is portable. The driver is trusted fixture code: usage
metadata is explicitly marked `TRANSPORT_METADATA` only when the test intends
reconciliation. `MAXIMUM_ONLY` cases prove model-visible usage cannot release
reserved maxima. Unknown usage and failed/cancelled calls remain capped charges.

Production drivers must obtain transport metadata independently of model output,
enforce provider-side token and monetary caps, and keep credentials out of
commands and artifacts. Separate reviewer calls may share model mistakes.
