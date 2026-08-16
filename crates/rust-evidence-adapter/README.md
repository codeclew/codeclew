# Codeclew Rust evidence adapter

This crate is deliberately a nested, standalone Cargo workspace. It must not be
added to Codeclew's root workspace until the Kotlin decision core has been
frozen and its digest recorded.

The adapter emits `codeclew.adapter-output/0.1`. It obtains navigation evidence
from the SCIP output of an explicitly pinned `rust-analyzer`, and records an
independent `cargo check` receipt. SCIP evidence is never treated as a proof of
program behaviour or as complete change-impact coverage.

Every executable is supplied explicitly with its expected SHA-256 digest. The
adapter refuses to run if a digest differs. Because Cargo and rust-analyzer can
execute build scripts and procedural macros, collection also requires the
explicit `--allow-trusted-workspace-code-execution` acknowledgement.

```text
codeclew-rust-evidence-adapter \
  --repo /absolute/repository \
  --rust-analyzer /absolute/rust-analyzer \
  --rust-analyzer-sha256 <sha256> \
  --cargo /absolute/cargo --cargo-sha256 <sha256> \
  --rustc /absolute/rustc --rustc-sha256 <sha256> \
  --git /absolute/git --git-sha256 <sha256> \
  --allow-trusted-workspace-code-execution \
  --output /absolute/evidence.json

codeclew-rust-evidence-adapter \
  --repo /absolute/repository \
  <the same pinned executables and acknowledgement> \
  --seed-entity '<opaque entity id or exact native SCIP identity>' \
  --max-depth 2 --max-entities 200 \
  --output /absolute/impact.json
```

Without `--seed-entity`, the embedded impact result is deterministically
`UNKNOWN/NO_SEED_ENTITY`. With a seed, it is a deterministic, bounded heuristic
over compiler-resolved SCIP facts. Its receipt always carries the adapter's
macro, cfg, dynamic, unsafe and FFI boundaries. Truncation is explicit.

The fixture conformance test requires an independently provisioned provider:

```text
CODECLEW_TEST_RUST_ANALYZER=/absolute/rust-analyzer \
CODECLEW_TEST_RUST_ANALYZER_SHA256=<sha256> \
cargo test --manifest-path crates/rust-evidence-adapter/Cargo.toml
```

If these variables are absent, the real-provider test reports a skip; the
fail-closed digest test still runs. No executable is downloaded implicitly.
