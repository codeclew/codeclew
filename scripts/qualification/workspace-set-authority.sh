#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$ROOT"
umask 077

CONTROL_HOME=${CODECLEW_CONTROL_HOME:-"$HOME/.cache/codeclew-control"}
EVIDENCE_PARENT="$CONTROL_HOME/qualification/workspace-set-authority"
EVIDENCE_ROOT="$EVIDENCE_PARENT/$(git rev-parse --verify HEAD)"
RUST_TARGET="$CONTROL_HOME/build-targets/rust"
mkdir -p "$EVIDENCE_PARENT" "$RUST_TARGET"
mkdir "$EVIDENCE_ROOT"
chmod 700 "$EVIDENCE_PARENT" "$EVIDENCE_ROOT" "$RUST_TARGET"
export CARGO_TARGET_DIR="$RUST_TARGET"

run_stage() {
  stage=$1
  shift
  if "$@" >"$EVIDENCE_ROOT/$stage.stdout" 2>"$EVIDENCE_ROOT/$stage.stderr"; then
    return 0
  fi
  printf '%s\n' "workspace-set-authority failed: $stage" >&2
  return 1
}

# Component-only checks: no real Gradle/Maven extraction and no edit/publish
# E2E are allowed before FOUNDATION_ENTRY.
run_stage rust-workspace-authority \
  cargo test --locked -p clew --lib \
    'kotlin_adapter_v2::tests::generation_set_workspace_materializes_and_mounts_one_shared_repository' \
    -- --exact --test-threads=1
run_stage rust-private-evidence \
  cargo test --locked -p clew --lib \
    'generation_service::tests::workspace_profile_is_private_canonical_and_rejects_impossible_counts' \
    -- --exact --test-threads=1
run_stage private-bridge-boundary python3 -I -S - <<'PY'
from pathlib import Path

adapter = Path("crates/clew/src/kotlin_adapter_v2.rs").read_text(encoding="utf-8")
generation = Path("crates/clew/src/generation_service.rs").read_text(encoding="utf-8")
if adapter.count(".open_project_verified(") != 1:
    raise SystemExit("legacy OpenProject must have exactly one private bridge call site")
if "fn open_compilation_from_set(" not in adapter:
    raise SystemExit("typed workspace-set bridge boundary is missing")
if ".open_project_verified(" in generation or ".open_compilation(" in generation:
    raise SystemExit("generation service bypasses the set bridge")
PY

python3 -I -S - "$EVIDENCE_ROOT" <<'PY'
import hashlib
import json
import os
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
qualification = {
    "contracts": [
        "EXACT_ORDERED_SET_AUTHORITY",
        "ONE_WORKSPACE_SET_AUTHORIZATION_EVIDENCE",
        "ONE_MATERIALIZATION_AND_MOUNT_SET",
        "OUTSIDE_DUPLICATE_UNSORTED_REFUSAL",
        "PER_COMPILATION_LIVE_WORKER_OWNERSHIP",
        "PRIVATE_LEGACY_OPEN_PROJECT_BRIDGE",
        "EXPLICIT_LEGACY_CALL_ACCOUNTING",
    ],
    "legacyBridge": "REQUIRED_UNTIL_POST_G1_MANAGED_WORKER_CUTOVER",
    "logDigests": {
        path.name: "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(root.iterdir(), key=lambda value: value.name)
        if path.is_file() and path.suffix in {".stdout", ".stderr"}
    },
    "performanceClaim": "NOT_RUN_BEFORE_Q2",
    "schema": "codeclew-workspace-set-authority-qualification/1.0",
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
