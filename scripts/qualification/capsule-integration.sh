#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$ROOT"
umask 077

CONTROL_HOME=${CODECLEW_CONTROL_HOME:-"$HOME/.cache/codeclew-control"}
SEED_HOME=${CODECLEW_SEED_HOME:-"$HOME/.cache/codeclew-seeds"}
REVISION=$(git rev-parse --verify HEAD)
SOURCE_TREE=$(git rev-parse --verify HEAD^{tree})
EVIDENCE_PARENT="$CONTROL_HOME/qualification/capsule-integration"
EVIDENCE_ROOT="$EVIDENCE_PARENT/$REVISION"
WORK="$CONTROL_HOME/tmp/capsule-integration-$REVISION"
mkdir -p "$EVIDENCE_PARENT" "$CONTROL_HOME/tmp"
chmod 700 "$EVIDENCE_PARENT" "$CONTROL_HOME/tmp"
mkdir "$EVIDENCE_ROOT" "$WORK"
chmod 700 "$EVIDENCE_ROOT" "$WORK"

if ! git clone --quiet --no-hardlinks "$ROOT" "$WORK/repo" \
  >"$EVIDENCE_ROOT/clone.stdout" 2>"$EVIDENCE_ROOT/clone.stderr"
then
  printf '%s\n' 'capsule-integration failed: detached clone' >&2
  exit 1
fi

python3 -I -S - "$WORK/repo/bootstrap/clew_bootstrap.py" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
with path.open("ab") as stream:
    stream.write(b"\n# component reuse integration authority probe\n")
PY
git -C "$WORK/repo" add bootstrap/clew_bootstrap.py
git -C "$WORK/repo" \
  -c user.name='Codeclew Tests' \
  -c user.email='tests@codeclew.invalid' \
  commit -qm 'Component reuse authority probe'

if ! python3 -I -S "$ROOT/scripts/trusted_seed_gc.py" \
  --root "$SEED_HOME" --run-current-state parallel-state \
  --expected-source-revision "$REVISION" \
  --expected-source-tree "$SOURCE_TREE" -- \
  "$WORK/repo/clew" --bootstrap-cold-build-evidence=parallel \
  >"$EVIDENCE_ROOT/reuse.json" 2>"$EVIDENCE_ROOT/reuse.stderr"
then
  printf '%s\n' 'capsule-integration failed: component reuse run' >&2
  exit 1
fi

python3 -I -S - "$SEED_HOME" "$EVIDENCE_ROOT" <<'PY'
import hashlib
import json
import os
import pathlib
import sys

seed_home = pathlib.Path(sys.argv[1])
evidence_root = pathlib.Path(sys.argv[2])
current = json.loads((seed_home / "current.json").read_bytes())
seed = json.loads((seed_home / current["epoch"] / "seed.json").read_bytes())
reuse_bytes = (evidence_root / "reuse.json").read_bytes()
reuse = json.loads(reuse_bytes)
expected_components = ["clew", "kotlin24"]
if (
    reuse.get("schema") != "codeclew-real-cold-build-evidence/1.0"
    or reuse.get("status") != "MEASURED"
    or reuse.get("mode") != "RELEASE"
    or reuse.get("runtimeKey") == seed.get("runtimeKey")
    or reuse.get("componentHits") != expected_components
    or reuse.get("componentMisses") != []
    or reuse.get("buildPlan", {}).get("toolchainStages") != []
    or reuse.get("artifactHashes") != seed.get("artifactHashes")
    or reuse.get("workerTreeHashes") != seed.get("workerTreeHashes")
):
    raise SystemExit("capsule component reuse evidence is invalid")
qualification = {
    "componentHits": expected_components,
    "componentMisses": [],
    "reuseEvidenceDigest": "sha256:" + hashlib.sha256(reuse_bytes).hexdigest(),
    "schema": "codeclew-capsule-component-integration-qualification/1.0",
    "status": "PASS",
    "toolchainStages": [],
}
temporary = evidence_root / ".qualification.json.tmp"
temporary.write_text(
    json.dumps(qualification, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
os.chmod(temporary, 0o400)
os.replace(temporary, evidence_root / "qualification.json")
print(json.dumps(qualification, sort_keys=True, separators=(",", ":")))
PY

python3 -I -S - "$WORK" "$CONTROL_HOME/tmp" <<'PY'
import pathlib
import shutil
import sys

target = pathlib.Path(sys.argv[1]).resolve(strict=True)
parent = pathlib.Path(sys.argv[2]).resolve(strict=True)
if target.parent != parent or not target.name.startswith("capsule-integration-"):
    raise SystemExit("refusing to discard an unowned integration work directory")
shutil.rmtree(target)
PY
