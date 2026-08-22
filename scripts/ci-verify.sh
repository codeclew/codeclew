#!/bin/sh
set -eu
umask 077

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --locked -p clew --lib 'context_v2::tests::' -- --test-threads=1
cargo test --locked -p clew --lib 'task_run_v2::tests::' -- --test-threads=1
cargo test --locked -p clew --lib 'session::tests::' -- --test-threads=1
cargo test --locked -p clew --bin clew 'tests::' -- --test-threads=1
python3 -I -S bootstrap/test_clew_bootstrap.py
python3 -I -S scripts/build-trusted-worker-distributions.py --verify-only
python3 -I -S scripts/check_repository_privacy.py --pre-commit
python3 -I -S scripts/usability-smoke.py

printf '%s\n' '{"schema":"codeclew-verification/1.0","status":"PASSED"}'
