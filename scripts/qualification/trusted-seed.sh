#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"
python3 -I -S "$ROOT/scripts/stabilization_control.py" guard --gate trusted-seed >/dev/null
umask 077

if [ -n "$(git status --porcelain=v1 --untracked-files=all)" ]; then
  printf '%s\n' 'trusted seed requires a clean source checkpoint' >&2
  exit 1
fi

SOURCE_REVISION=$(git rev-parse --verify HEAD)
SOURCE_TREE=$(git rev-parse --verify HEAD^{tree})
SEED_BASE=${CODECLEW_SEED_HOME:-"$HOME/.cache/codeclew-seeds"}

python3 -I -S - "$SEED_BASE" <<'PY'
import os
import pathlib
import stat
import sys

path = pathlib.Path(sys.argv[1])
if not path.is_absolute() or ".." in path.parts:
    raise SystemExit("seed home must be normalized and absolute")
flags = os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0)
descriptor = os.open("/", flags)
try:
    parts = [part for part in path.parts if part != path.anchor]
    for index, component in enumerate(parts):
        try:
            child = os.open(component, flags, dir_fd=descriptor)
        except FileNotFoundError:
            os.mkdir(component, mode=0o700, dir_fd=descriptor)
            child = os.open(component, flags, dir_fd=descriptor)
        metadata = os.fstat(child)
        leaf = index == len(parts) - 1
        if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid not in {0, os.geteuid()}:
            os.close(child)
            raise SystemExit("seed home authority is invalid")
        if not leaf and stat.S_IMODE(metadata.st_mode) & 0o022:
            os.close(child)
            raise SystemExit("seed home has an unsafe ancestor")
        if leaf:
            if metadata.st_uid != os.geteuid():
                os.close(child)
                raise SystemExit("seed home has a different owner")
            os.fchmod(child, 0o700)
        os.close(descriptor)
        descriptor = child
finally:
    os.close(descriptor)
PY

EPOCH="release-N-$SOURCE_REVISION"
FINAL="$SEED_BASE/$EPOCH"

publish_locator() {
  if [ "$#" -eq 0 ]; then
    python3 -I -S "$ROOT/scripts/trusted_seed_gc.py" \
      --root "$SEED_BASE" --publish-epoch "$EPOCH" \
      --expected-source-tree "$SOURCE_TREE"
  else
    python3 -I -S "$ROOT/scripts/trusted_seed_gc.py" \
      --root "$SEED_BASE" --publish-epoch "$EPOCH" --candidate "$1" \
      --expected-source-tree "$SOURCE_TREE"
  fi
}

seed_gc() {
  python3 -I -S "$ROOT/scripts/trusted_seed_gc.py" --root "$SEED_BASE" "$@" >&2
}

python3 -I -S "$ROOT/scripts/test_trusted_seed_gc.py"
seed_gc --protect-epoch "$EPOCH"

set +e
EXISTING_QUALIFICATION=$(python3 -I -S "$ROOT/scripts/trusted_seed_gc.py" \
  --root "$SEED_BASE" --validate-epoch "$EPOCH" \
  --expected-source-tree "$SOURCE_TREE")
EXISTING_STATUS=$?
set -e
if [ "$EXISTING_STATUS" -eq 0 ]; then
  AUDIT=$(mktemp -d "$SEED_BASE/.audit.XXXXXX")
  cleanup_existing() {
    result=$?
    trap - EXIT INT TERM
    cleanup_status=0
    python3 -I -S "$ROOT/scripts/bounded_gate_cleanup.py" \
      --timeout-seconds 30 tree --path "$AUDIT" >/dev/null 2>&1 || cleanup_status=$?
    if [ "$result" -eq 0 ] && [ "$cleanup_status" -ne 0 ]; then
      result=$cleanup_status
    fi
    exit "$result"
  }
  trap cleanup_existing EXIT INT TERM
  CODECLEW_HOME="$AUDIT/state" CODECLEW_RUNTIME_SEED="$FINAL/seed.json" \
    "$ROOT/clew" --bootstrap-warm-audit >"$AUDIT/warm.json"
  python3 -I -S - "$AUDIT/warm.json" <<'PY'
import json
import pathlib
import sys

audit = json.loads(pathlib.Path(sys.argv[1]).read_bytes())
if (
    audit.get("status") != "PASSED"
    or audit.get("counters", {}).get("processRuns") != 0
    or audit.get("counters", {}).get("digestFileCalls") != 0
):
    raise SystemExit("existing seed capsule failed its warm audit")
PY
  PUBLISHED_QUALIFICATION=$(publish_locator)
  printf '%s\n' "$PUBLISHED_QUALIFICATION"
  exit 0
fi
if [ "$EXISTING_STATUS" -eq 2 ]; then
  set +e
  RECOVERED_QUALIFICATION=$(publish_locator)
  RECOVERED_STATUS=$?
  set -e
  if [ "$RECOVERED_STATUS" -eq 0 ]; then
    EXISTING_QUALIFICATION=$RECOVERED_QUALIFICATION
    EXISTING_STATUS=0
  fi
fi
if [ "$EXISTING_STATUS" -eq 0 ]; then
  AUDIT=$(mktemp -d "$SEED_BASE/.audit.XXXXXX")
  cleanup_existing() {
    result=$?
    trap - EXIT INT TERM
    cleanup_status=0
    python3 -I -S "$ROOT/scripts/bounded_gate_cleanup.py" \
      --timeout-seconds 30 tree --path "$AUDIT" >/dev/null 2>&1 || cleanup_status=$?
    if [ "$result" -eq 0 ] && [ "$cleanup_status" -ne 0 ]; then
      result=$cleanup_status
    fi
    exit "$result"
  }
  trap cleanup_existing EXIT INT TERM
  CODECLEW_HOME="$AUDIT/state" CODECLEW_RUNTIME_SEED="$FINAL/seed.json" \
    "$ROOT/clew" --bootstrap-warm-audit >"$AUDIT/warm.json"
  python3 -I -S - "$AUDIT/warm.json" <<'PY'
import json
import pathlib
import sys

audit = json.loads(pathlib.Path(sys.argv[1]).read_bytes())
if (
    audit.get("status") != "PASSED"
    or audit.get("counters", {}).get("processRuns") != 0
    or audit.get("counters", {}).get("digestFileCalls") != 0
):
    raise SystemExit("recovered seed capsule failed its warm audit")
PY
  printf '%s\n' "$EXISTING_QUALIFICATION"
  exit 0
fi
if [ "$EXISTING_STATUS" -ne 3 ]; then
  printf '%s\n' "$EXISTING_QUALIFICATION" >&2
  exit "$EXISTING_STATUS"
fi

WORK=$(mktemp -d "$SEED_BASE/.candidate.XXXXXX")
cleanup() {
  result=$?
  trap - EXIT INT TERM
  if [ "$result" -ne 0 ] && [ -d "$WORK" ]; then
    FAILED="$SEED_BASE/failed-$SOURCE_REVISION-$$"
    mv "$WORK" "$FAILED"
  fi
  exit "$result"
}
trap cleanup EXIT INT TERM

"$ROOT/clew" --bootstrap-component-preflight \
  >"$WORK/component-preflight.json" \
  2>"$WORK/component-preflight.stderr"
python3 -I -S - "$WORK/component-preflight.json" <<'PY'
import json
import pathlib
import sys

value = json.loads(pathlib.Path(sys.argv[1]).read_bytes())
plan = value.get("parallelBuildPlan")
if (
    set(value) != {"componentIds", "mode", "parallelBuildPlan", "schema", "status"}
    or value.get("schema") != "codeclew-runtime-component-preflight/2.0"
    or value.get("status") != "PASS"
    or value.get("mode") != "RELEASE"
    or not isinstance(value.get("componentIds"), list)
    or not value["componentIds"]
    or value["componentIds"] != sorted(set(value["componentIds"]))
    or not isinstance(plan, dict)
    or set(plan) != {
        "cargoWorkers", "gradleHeapBytes", "gradleWorkers", "inputWorkers",
        "memoryBudgetBytes", "packageWorkers", "parallel", "profile",
    }
    or plan["profile"] != "PARALLEL"
    or plan["parallel"] is not True
    or any(
        type(plan[field]) is not int or plan[field] <= 0
        for field in (
            "cargoWorkers", "gradleHeapBytes", "gradleWorkers", "inputWorkers",
            "memoryBudgetBytes", "packageWorkers",
        )
    )
    or plan["gradleHeapBytes"] < 2 * 1024**3
    or plan["gradleHeapBytes"] > plan["memoryBudgetBytes"]
):
    raise SystemExit("trusted seed component preflight is invalid")
PY

CODECLEW_HOME="$WORK/parallel-state" \
  "$ROOT/clew" --bootstrap-cold-build-evidence=parallel \
  >"$WORK/parallel.json" 2>"$WORK/parallel.stderr"

if [ "$(git rev-parse --verify HEAD)" != "$SOURCE_REVISION" ] || \
   [ "$(git rev-parse --verify HEAD^{tree})" != "$SOURCE_TREE" ] || \
   [ -n "$(git status --porcelain=v1 --untracked-files=all)" ]; then
  printf '%s\n' 'source authority changed during trusted seed construction' >&2
  exit 1
fi

python3 -I -S - "$WORK" "$SOURCE_REVISION" "$SOURCE_TREE" <<'PY'
import hashlib
import json
import os
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
parallel_bytes = (root / "parallel.json").read_bytes()
parallel = json.loads(parallel_bytes)
if parallel.get("schema") != "codeclew-real-cold-build-evidence/1.0" or parallel.get("status") != "MEASURED" or parallel.get("mode") != "RELEASE":
    raise SystemExit("trusted seed build returned invalid evidence")
seed = {
    "artifactHashes": parallel["artifactHashes"],
    "buildEvidenceDigests": [
        "sha256:" + hashlib.sha256(parallel_bytes).hexdigest()
    ],
    "manifestDigest": parallel["manifestDigest"],
    "mode": "RELEASE",
    "runtimeKey": parallel["runtimeKey"],
    "schema": "codeclew-trusted-release-seed/1.0",
    "sourceRevision": sys.argv[2],
    "sourceTree": sys.argv[3],
    "stateEpoch": "sha256:" + hashlib.sha256((parallel["runtimeKey"] + "\0" + sys.argv[2]).encode()).hexdigest(),
    "workerTreeHashes": parallel["workerTreeHashes"],
}
seed["seedDigest"] = "sha256:" + hashlib.sha256(json.dumps(seed, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
temporary = root / ".seed.json.tmp"
temporary.write_text(json.dumps(seed, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
os.chmod(temporary, 0o400)
os.replace(temporary, root / "seed.json")
qualification = {
    "runtimeKey": seed["runtimeKey"],
    "schema": "codeclew-trusted-seed-qualification/1.0",
    "seedDigest": seed["seedDigest"],
    "status": "PASS",
}
qualification_path = root / "qualification.json"
qualification_path.write_text(
    json.dumps(qualification, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
os.chmod(qualification_path, 0o400)
PY

PUBLISHED_QUALIFICATION=$(publish_locator "${WORK##*/}")

WORK=
trap - EXIT INT TERM
printf '%s\n' "$PUBLISHED_QUALIFICATION"
