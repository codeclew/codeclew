#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$ROOT"
umask 077

CONTROL_HOME=${CODECLEW_CONTROL_HOME:-"$HOME/.cache/codeclew-control"}
EVIDENCE_PARENT="$CONTROL_HOME/qualification/runtime-contracts"
EVIDENCE_ROOT="$EVIDENCE_PARENT/$(git rev-parse --verify HEAD)"
RUST_TARGET="$CONTROL_HOME/build-targets/rust"
mkdir -p "$EVIDENCE_PARENT" "$RUST_TARGET"
mkdir "$EVIDENCE_ROOT"
chmod 700 "$EVIDENCE_ROOT"
chmod 700 "$EVIDENCE_PARENT" "$RUST_TARGET"
export CARGO_TARGET_DIR="$RUST_TARGET"

run_stage() {
  stage=$1
  shift
  if "$@" >"$EVIDENCE_ROOT/$stage.stdout" 2>"$EVIDENCE_ROOT/$stage.stderr"; then
    return 0
  fi
  printf '%s\n' "runtime-contracts failed: $stage" >&2
  return 1
}

# These tests are deliberately component-local. They exercise authority-domain
# separation, immutable publication/quarantine, warm checkpoint behavior,
# singleflight, managed-state safety, leases, and runtime-root retention without
# starting a real cold build or an edit/publish end-to-end flow.
run_stage bootstrap-contracts \
  python3 -I -S bootstrap/test_clew_bootstrap.py -q
run_stage rust-runtime-authority \
  cargo test --locked -p clew --lib 'runtime::tests::' -- --test-threads=1
run_stage rust-state-authority \
  cargo test --locked -p clew --lib 'state::tests::' -- --test-threads=1
run_stage rust-session-runtime-root \
  cargo test --locked -p clew --lib \
    'session::tests::descriptor_authority_ignores_a_replaced_state_root_path' \
    -- --exact --test-threads=1

python3 -I -S - "$EVIDENCE_ROOT" <<'PY'
import hashlib
import json
import os
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
logs = {}
for path in sorted(root.iterdir(), key=lambda value: value.name):
    if path.is_file() and path.suffix in {".stdout", ".stderr"}:
        logs[path.name] = "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
qualification = {
    "contracts": [
        "MODE_DOMAIN_SEPARATION",
        "IMMUTABLE_PUBLICATION_AND_QUARANTINE",
        "WARM_CHECKPOINT_NO_TOOLCHAIN",
        "SINGLEFLIGHT_PUBLICATION",
        "PRIVATE_MANAGED_STATE",
        "LEASE_AND_SESSION_RUNTIME_RETENTION",
    ],
    "logDigests": logs,
    "schema": "codeclew-runtime-contract-qualification/1.0",
    "status": "PASS",
}
temporary = root / ".qualification.json.tmp"
temporary.write_text(
    json.dumps(qualification, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
os.chmod(temporary, 0o400)
os.replace(temporary, root / "qualification.json")
print(json.dumps(qualification, sort_keys=True, separators=(",", ":")))
PY
