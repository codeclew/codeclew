#!/bin/sh
set -eu
umask 077

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

CI_TMP_BASE=$(python3 -I -S -c 'import os, pwd; print(pwd.getpwuid(os.geteuid()).pw_dir)')
TEST_TMP_ROOT=$(mktemp -d "$CI_TMP_BASE/.codeclew-ci.XXXXXX")
chmod 700 "$TEST_TMP_ROOT"
trap 'rm -rf -- "$TEST_TMP_ROOT"' EXIT HUP INT TERM
TMPDIR=$TEST_TMP_ROOT
export TMPDIR

python3 -I -S scripts/test_pilot_case_record.py
python3 -I -S scripts/test_pilot_release_gate.py
python3 -I -S scripts/test_check_repository_privacy.py
python3 -I -S scripts/check_english_content.py
python3 -I -S scripts/test_macos_distribution.py
python3 -I -S scripts/test_build_macos_release.py
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --locked -p clew --lib 'operations::tests::' -- --test-threads=1
cargo test --locked -p clew --lib 'context_v2::tests::' -- --test-threads=1
cargo test --locked -p clew --lib 'task_run_v2::tests::' -- --test-threads=1
cargo test --locked -p clew --lib 'session::tests::' -- --test-threads=1
cargo test --locked -p clew --bin clew 'tests::' -- --test-threads=1
cargo test --locked -p clew --test managed_cli \
  managed_operational_commands_are_path_free_and_support_recovery -- --test-threads=1
cargo test --locked -p clew --test managed_cli \
  managed_support_summary_requires_private_input_and_drops_private_material -- --test-threads=1
python3 -I -S bootstrap/test_clew_bootstrap.py
GIT_CONFIG_COUNT=2 \
GIT_CONFIG_KEY_0=user.name \
GIT_CONFIG_VALUE_0='Codeclew Maintainers' \
GIT_CONFIG_KEY_1=user.email \
GIT_CONFIG_VALUE_1='maintainers@codeclew.invalid' \
python3 -I -S scripts/check_repository_privacy.py --pre-commit
python3 -I -S scripts/usability-smoke.py

printf '%s\n' '{"schema":"codeclew-verification/1.0","status":"PASSED"}'
