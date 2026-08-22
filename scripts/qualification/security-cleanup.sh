#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$ROOT"
umask 077

CONTROL_HOME=${CODECLEW_CONTROL_HOME:-"$HOME/.cache/codeclew-control"}
EVIDENCE_PARENT="$CONTROL_HOME/qualification/security-cleanup"
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
  printf '%s\n' "security-cleanup failed: $stage" >&2
  return 1
}

run_stage managed-state-security \
  cargo test --locked -p clew --lib 'state::tests::' -- --test-threads=1
run_stage snapshot-security \
  cargo test --locked -p clew --lib 'repository_snapshot::tests::' -- --test-threads=1
run_stage replaced-root-gc-refusal \
  cargo test --locked -p clew --lib \
    'session::tests::gc_refuses_a_replaced_state_root_before_following_any_locator' \
    -- --exact --test-threads=1
run_stage exhaustive-gc-cleanliness \
  cargo test --locked -p clew --lib \
    'session::tests::gc_cleanliness_reports_ignored_and_untracked_outputs_without_classifying_them' \
    -- --exact --test-threads=1
run_stage managed-only-gc \
  cargo test --locked -p clew --lib \
    'session::tests::gc_removes_only_managed_worktrees_and_leaves_legacy_state_inert' \
    -- --exact --test-threads=1
run_stage legacy-plan-and-cleanliness-inert \
  cargo test --locked -p clew --lib \
    'task_run_v2::tests::legacy_state_is_neither_a_plan_target_nor_a_cleanliness_input' \
    -- --exact --test-threads=1

python3 -I -S - "$EVIDENCE_ROOT" <<'PY'
import hashlib
import json
import os
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
qualification = {
    "contracts": [
        "PATH_FREE_REPOSITORY_AUTHORITY",
        "PRIVATE_0700_0600_MANAGED_STATE",
        "DESCRIPTOR_BOUND_PATH_REPLACEMENT_REFUSAL",
        "SYMLINK_ANCESTOR_AND_TARGET_REFUSAL",
        "SEALED_READ_ONLY_MATERIALIZATION",
        "ROOT_AND_NESTED_LEGACY_SUBTREE_INERT",
        "EXHAUSTIVE_WORKTREE_INDEX_UNTRACKED_GC_CHECK",
        "MANAGED_DERIVED_OUTPUTS_ONLY_CLEANUP",
        "UNKNOWN_CANDIDATE_STATE_REFUSAL",
    ],
    "legacyMutation": "NONE",
    "logDigests": {
        path.name: "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(root.iterdir(), key=lambda value: value.name)
        if path.is_file() and path.suffix in {".stdout", ".stderr"}
    },
    "schema": "codeclew-security-cleanup-qualification/1.0",
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
