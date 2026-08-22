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
if [ -f "$FINAL/seed.json" ]; then
  CODECLEW_HOME="$FINAL/parallel-state" "$ROOT/clew" --bootstrap-warm-audit >"$FINAL/parallel-warm.json"
  CODECLEW_HOME="$FINAL/serial-state" "$ROOT/clew" --bootstrap-warm-audit >"$FINAL/serial-warm.json"
  python3 -I -S - "$FINAL" "$SOURCE_REVISION" "$SOURCE_TREE" <<'PY'
import json
import pathlib
import stat
import sys

root = pathlib.Path(sys.argv[1])
seed = json.loads((root / "seed.json").read_bytes())
if (
    seed.get("schema") != "codeclew-trusted-release-seed/1.0"
    or seed.get("sourceRevision") != sys.argv[2]
    or seed.get("sourceTree") != sys.argv[3]
):
    raise SystemExit("existing seed authority differs from the source checkpoint")
for name in ("parallel-warm.json", "serial-warm.json"):
    audit = json.loads((root / name).read_bytes())
    if (
        audit.get("status") != "PASSED"
        or audit.get("counters", {}).get("processRuns") != 0
        or audit.get("counters", {}).get("digestFileCalls") != 0
    ):
        raise SystemExit("existing seed capsule failed its warm audit")
qualification = {
    "runtimeKey": seed["runtimeKey"],
    "schema": "codeclew-trusted-seed-qualification/1.0",
    "seedDigest": seed["seedDigest"],
    "status": "PASS",
}
(root / "qualification.json").write_text(json.dumps(qualification, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
os.chmod(root / "qualification.json", 0o400)
PY
  exit 0
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

CODECLEW_HOME="$WORK/parallel-state" \
  "$ROOT/clew" --bootstrap-cold-build-evidence=parallel >"$WORK/parallel.json"
CODECLEW_HOME="$WORK/serial-state" \
  "$ROOT/clew" --bootstrap-cold-build-evidence=serial >"$WORK/serial.json"

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
serial_bytes = (root / "serial.json").read_bytes()
parallel = json.loads(parallel_bytes)
serial = json.loads(serial_bytes)
for value in (parallel, serial):
    if value.get("schema") != "codeclew-real-cold-build-evidence/1.0" or value.get("status") != "MEASURED" or value.get("mode") != "RELEASE":
        raise SystemExit("trusted seed build returned invalid evidence")
for field in ("runtimeKey", "manifestDigest", "artifactHashes", "workerTreeHashes"):
    if parallel.get(field) != serial.get(field):
        raise SystemExit(f"serial/parallel trusted seed mismatch: {field}")
seed = {
    "artifactHashes": parallel["artifactHashes"],
    "buildEvidenceDigests": sorted([
        "sha256:" + hashlib.sha256(parallel_bytes).hexdigest(),
        "sha256:" + hashlib.sha256(serial_bytes).hexdigest(),
    ]),
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
print(json.dumps({
    "runtimeKey": seed["runtimeKey"],
    "schema": "codeclew-trusted-seed-qualification/1.0",
    "seedDigest": seed["seedDigest"],
    "status": "PASS",
}, sort_keys=True, separators=(",", ":")))
PY

mv "$WORK" "$FINAL"
chmod 700 "$FINAL"
python3 -I -S - "$SEED_BASE" "$EPOCH" <<'PY'
import json
import os
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
seed = json.loads((root / sys.argv[2] / "seed.json").read_bytes())
value = {
    "epoch": sys.argv[2],
    "runtimeKey": seed["runtimeKey"],
    "schema": "codeclew-trusted-seed-locator/1.0",
    "seedDigest": seed["seedDigest"],
}
temporary = root / ".current.json.tmp"
temporary.write_text(json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
os.chmod(temporary, 0o600)
os.replace(temporary, root / "current.json")
PY

WORK=
trap - EXIT INT TERM
cat "$FINAL/qualification.json"
