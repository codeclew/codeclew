#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$ROOT"
umask 077
python3 -I -S "$ROOT/scripts/stabilization_control.py" guard --gate foundation-entry >/dev/null

CONTROL_HOME=${CODECLEW_CONTROL_HOME:-"$HOME/.cache/codeclew-control"}
EVIDENCE_PARENT="$CONTROL_HOME/qualification/foundation-entry"
EVIDENCE_ROOT="$EVIDENCE_PARENT/$(git rev-parse --verify HEAD)"
mkdir -p "$EVIDENCE_PARENT"
mkdir "$EVIDENCE_ROOT"
chmod 700 "$EVIDENCE_PARENT" "$EVIDENCE_ROOT"

python3 -I -S - "$ROOT" "$EVIDENCE_ROOT" <<'PY'
import hashlib
import json
import os
import pathlib
import stat
import subprocess
import sys

repository = pathlib.Path(sys.argv[1])
evidence = pathlib.Path(sys.argv[2])
expected = [f"S{index}" for index in range(12)]
status = json.loads(
    subprocess.check_output(
        [sys.executable, "-I", "-S", "scripts/stabilization_control.py", "status"],
        cwd=repository,
        text=True,
    )
)
if status.get("completed") != expected or status.get("nextStep") != "G0":
    raise SystemExit("foundation steps are not exactly complete")
if subprocess.check_output(
    ["git", "status", "--porcelain=v1", "--untracked-files=all"], cwd=repository
):
    raise SystemExit("foundation entry requires a clean repository")
worktrees = subprocess.check_output(
    ["git", "worktree", "list", "--porcelain"], cwd=repository, text=True
)
if sum(line.startswith("worktree ") for line in worktrees.splitlines()) != 1:
    raise SystemExit("foundation entry requires exactly one linked worktree")

gates = [
    "trusted-seed.sh",
    "runtime-contracts.sh",
    "component-cas.sh",
    "capsule-integration.sh",
    "workspace-set-authority.sh",
    "generation-reuse.sh",
    "bounded-context.sh",
    "transaction-recovery.sh",
    "security-cleanup.sh",
]
gate_digests = {}
for name in gates:
    path = repository / "scripts" / "qualification" / name
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode) or not metadata.st_mode & stat.S_IXUSR:
        raise SystemExit(f"foundation gate is not a regular executable: {name}")
    gate_digests[name] = "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()

commands = subprocess.check_output(["ps", "-axo", "command="], text=True)
for marker in (
    "__task-run-execute",
    "capsule-build-",
    "qualification/trusted-seed.sh",
    "qualification/runtime-contracts.sh",
    "qualification/component-cas.sh",
    "qualification/capsule-integration.sh",
    "qualification/workspace-set-authority.sh",
    "qualification/generation-reuse.sh",
    "qualification/bounded-context.sh",
    "qualification/transaction-recovery.sh",
    "qualification/security-cleanup.sh",
):
    if marker in commands:
        raise SystemExit(f"foundation process is still active: {marker}")

qualification = {
    "completedSteps": expected,
    "foundationGateDigests": gate_digests,
    "noActiveDerivedProcess": True,
    "schema": "codeclew-foundation-entry-qualification/1.0",
    "singleWorktree": True,
    "sourceRevision": subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=repository, text=True
    ).strip(),
    "status": "PASS",
}
temporary = evidence / ".qualification.json.tmp"
temporary.write_text(
    json.dumps(qualification, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
os.chmod(temporary, 0o400)
os.replace(temporary, evidence / "qualification.json")
print(json.dumps(qualification, sort_keys=True, separators=(",", ":")))
PY
