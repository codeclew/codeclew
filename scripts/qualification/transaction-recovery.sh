#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$ROOT"
umask 077

CONTROL_HOME=${CODECLEW_CONTROL_HOME:-"$HOME/.cache/codeclew-control"}
EVIDENCE_PARENT="$CONTROL_HOME/qualification/transaction-recovery"
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
  printf '%s\n' "transaction-recovery failed: $stage" >&2
  return 1
}

run_stage session-ledger-contracts \
  cargo test --locked -p clew --lib 'session::tests::' -- --test-threads=1
run_stage candidate-recovery-contracts \
  cargo test --locked -p clew --lib 'task_run_v2::tests::' -- --test-threads=1
run_stage verified-process-group-cancellation \
  cargo test --locked -p clew --bin clew \
    'tests::cancellation_targets_only_verified_process_group' \
    -- --exact --test-threads=1
run_stage recovery-cli-contract \
  cargo test --locked -p clew --bin clew \
    'tests::recovery_and_cancellation_are_explicit' \
    -- --exact --test-threads=1
run_stage no-rollback-boundary python3 -I -S - <<'PY'
from pathlib import Path

roots = [Path("crates/clew/src/main.rs"), Path("crates/clew/src/session.rs"), Path("crates/clew/src/task_run_v2.rs")]
source = "\n".join(path.read_text(encoding="utf-8") for path in roots)
required = (
    "RunStatus::Created",
    "RunStatus::WorktreeRecoveryRequired",
    "recoverable_candidate_commit",
    "process_start_token",
    "process_group(0)",
    "--ff-only",
)
for marker in required:
    if marker not in source:
        raise SystemExit(f"transaction contract marker is missing: {marker}")
if "reset --hard" in source or '"reset", "--hard"' in source:
    raise SystemExit("automatic hard reset is forbidden after candidate authority exists")
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
        "DETERMINISTIC_RUN_AND_TRANSACTION_ID",
        "EARLY_CREATED_APPEND_ONLY_LEDGER",
        "IDEMPOTENT_START_AND_DURABLE_STATUS",
        "DETACHED_PROCESS_GROUP_SUPERVISOR",
        "VERIFIED_PROCESS_TREE_CANCELLATION",
        "PREPARE_PUBLISH_RECOVER_SEPARATION",
        "COMMITTED_CANDIDATE_NEVER_AUTO_DISCARDED",
        "FAST_FORWARD_OR_CAS_PUBLICATION",
        "FORWARD_ONLY_WORKTREE_RECOVERY",
        "LEDGER_TAMPER_AND_STALE_WRITER_REFUSAL",
    ],
    "endToEndPublish": "NOT_RUN_BEFORE_Q3",
    "logDigests": {
        path.name: "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(root.iterdir(), key=lambda value: value.name)
        if path.is_file() and path.suffix in {".stdout", ".stderr"}
    },
    "schema": "codeclew-transaction-recovery-qualification/1.0",
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
