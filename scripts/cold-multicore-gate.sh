#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
mkdir -p benchmarks/reports
umask 077

PHYSICAL_CORES=$(python3 -I -S - <<'PY'
import subprocess


def macos_physical_cores():
    try:
        value = subprocess.run(
            ["sysctl", "-n", "hw.physicalcpu"],
            check=True,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        ).stdout.strip()
        return int(value)
    except (FileNotFoundError, subprocess.SubprocessError, ValueError):
        return None


def linux_physical_cores():
    try:
        pairs = set()
        physical = None
        core = None
        with open("/proc/cpuinfo", encoding="utf-8") as stream:
            for line in stream:
                if not line.strip():
                    if physical is not None and core is not None:
                        pairs.add((physical, core))
                    physical = core = None
                elif line.startswith("physical id"):
                    physical = line.split(":", 1)[1].strip()
                elif line.startswith("core id"):
                    core = line.split(":", 1)[1].strip()
        if physical is not None and core is not None:
            pairs.add((physical, core))
        return len(pairs) or None
    except OSError:
        return None


print(macos_physical_cores() or linux_physical_cores() or 0)
PY
)

REPORT=benchmarks/reports/cold-multicore-latest.json
TEMPORARY="$REPORT.tmp.$$"
GATE_BASE="$HOME/.cache/codeclew-gates"
python3 -I -S - "$GATE_BASE" <<'PY'
import os
import pathlib
import stat
import sys

base = pathlib.Path(sys.argv[1])
if not base.is_absolute() or ".." in base.parts:
    raise SystemExit("cold gate base must be normalized and absolute")
flags = os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0)
descriptor = os.open("/", flags)
try:
    components = [part for part in base.parts if part != base.anchor]
    for index, component in enumerate(components):
        try:
            child = os.open(component, flags, dir_fd=descriptor)
        except FileNotFoundError:
            os.mkdir(component, mode=0o700, dir_fd=descriptor)
            child = os.open(component, flags, dir_fd=descriptor)
        metadata = os.fstat(child)
        leaf = index == len(components) - 1
        if (
            not stat.S_ISDIR(metadata.st_mode)
            or metadata.st_uid not in {0, os.geteuid()}
            or (not leaf and stat.S_IMODE(metadata.st_mode) & 0o022)
        ):
            os.close(child)
            raise SystemExit("cold gate base has an unsafe ancestor")
        if leaf:
            if metadata.st_uid != os.geteuid():
                os.close(child)
                raise SystemExit("cold gate base has a different owner")
            os.fchmod(child, 0o700)
        os.close(descriptor)
        descriptor = child
finally:
    os.close(descriptor)
PY
WORK=$(mktemp -d "$GATE_BASE/run.XXXXXX")
SOURCE=
FINISHED=0

write_incomplete_report() {
    python3 -I -S - "$TEMPORARY" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
path.write_text(json.dumps({
    "accepted": False,
    "releaseGatePassed": False,
    "schema": "codeclew-real-cold-runtime-gate/1.0",
    "status": "FAILED_INCOMPLETE",
}, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY
    mv -f -- "$TEMPORARY" "$REPORT"
    chmod 600 "$REPORT"
}

cleanup() {
    result=$?
    trap - EXIT INT TERM
    rm -f -- "$TEMPORARY"
    python3 -I -S - "$WORK" <<'PY'
import os
import pathlib
import shutil
import stat
import sys

root = pathlib.Path(sys.argv[1])
if not root.exists():
    raise SystemExit(0)
metadata = root.lstat()
if not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
    raise SystemExit("cold gate cleanup root is not a physical directory")
for current, directories, _files in os.walk(root, topdown=False, followlinks=False):
    for name in directories:
        path = pathlib.Path(current, name)
        child = path.lstat()
        if stat.S_ISDIR(child.st_mode) and not stat.S_ISLNK(child.st_mode):
            path.chmod(0o700)
    pathlib.Path(current).chmod(0o700)
shutil.rmtree(root)
PY
    rmdir -- "$GATE_BASE" 2>/dev/null || :
    if [ "$FINISHED" -eq 0 ]; then
        write_incomplete_report
    fi
    exit "$result"
}
trap cleanup EXIT INT TERM

if [ "$PHYSICAL_CORES" -lt 4 ]; then
    python3 -I -S - "$PHYSICAL_CORES" >"$TEMPORARY" <<'PY'
import json
import sys

print(json.dumps({
    "accepted": True,
    "qualification": {
        "minimumPhysicalCores": 4,
        "physicalCores": int(sys.argv[1]),
    },
    "releaseGatePassed": False,
    "schema": "codeclew-real-cold-runtime-gate/1.0",
    "status": "SKIPPED_UNQUALIFIED_HOST",
    "thresholds": {"runtimeMedianRatioMax": 0.65},
}, sort_keys=True, separators=(",", ":")))
PY
else
    if [ -n "$(git status --porcelain=v1 --untracked-files=all)" ]; then
        echo "cold runtime evidence requires a clean source HEAD" >&2
        exit 1
    fi
    SOURCE_REVISION=$(git rev-parse --verify HEAD)
    SOURCE="$WORK/source"
    git clone --quiet --no-local --no-checkout "$ROOT" "$SOURCE"
    git -C "$SOURCE" checkout --quiet --detach "$SOURCE_REVISION"
    if [ "$(git -C "$SOURCE" rev-parse --verify HEAD)" != "$SOURCE_REVISION" ] ||
        [ -n "$(git -C "$SOURCE" status --porcelain=v1 --untracked-files=all)" ]; then
        echo "cold runtime frozen clone authority is invalid" >&2
        exit 1
    fi

    run_evidence() {
        pair=$1
        profile=$2
        lowercase=$(printf '%s' "$profile" | tr '[:upper:]' '[:lower:]')
        home="$WORK/pair-$pair-$lowercase-home"
        output="$WORK/pair-$pair-$lowercase.json"
        CODECLEW_HOME="$home" "$SOURCE/clew" "--bootstrap-cold-build-evidence=$lowercase" >"$output"
    }

    run_evidence 1 SERIAL
    run_evidence 1 PARALLEL
    run_evidence 2 PARALLEL
    run_evidence 2 SERIAL
    run_evidence 3 SERIAL
    run_evidence 3 PARALLEL

    python3 -I -S - "$WORK" "$PHYSICAL_CORES" "$SOURCE_REVISION" >"$TEMPORARY" <<'PY'
import json
import pathlib
import statistics
import sys

work = pathlib.Path(sys.argv[1])
physical_cores = int(sys.argv[2])
source_revision = sys.argv[3]
orders = {
    1: ["SERIAL", "PARALLEL"],
    2: ["PARALLEL", "SERIAL"],
    3: ["SERIAL", "PARALLEL"],
}


def load(pair, profile):
    path = work / f"pair-{pair}-{profile.lower()}.json"
    value = json.loads(path.read_text(encoding="utf-8"))
    if value.get("schema") != "codeclew-real-cold-build-evidence/1.0":
        raise SystemExit("cold bootstrap returned an unexpected evidence schema")
    if value.get("status") != "MEASURED":
        raise SystemExit("cold bootstrap did not return measured evidence")
    if value.get("mode") != "RELEASE":
        raise SystemExit("cold runtime release gate produced a non-release capsule")
    for field in ("runtimeKey", "manifestDigest"):
        digest = value.get(field)
        if not isinstance(digest, str) or not digest.startswith("sha256:"):
            raise SystemExit(f"cold bootstrap returned an invalid {field}")
    for field in ("artifactHashes", "workerTreeHashes"):
        digests = value.get(field)
        if not isinstance(digests, dict) or not digests:
            raise SystemExit(f"cold bootstrap returned an invalid {field}")
        if not all(
            isinstance(digest, str) and digest.startswith("sha256:")
            for digest in digests.values()
        ):
            raise SystemExit(f"cold bootstrap returned an invalid digest in {field}")
    wall_millis = value.get("wallMillis")
    if not isinstance(wall_millis, int) or wall_millis <= 0:
        raise SystemExit("cold bootstrap returned an invalid wall time")
    plan = value.get("buildPlan")
    if not isinstance(plan, dict) or plan.get("profile") != profile:
        raise SystemExit("cold bootstrap used the wrong build profile")
    if plan.get("parallel") is not (profile == "PARALLEL"):
        raise SystemExit("cold bootstrap build plan disagrees with its profile")
    return value


identity_fields = (
    "mode",
    "runtimeKey",
    "manifestDigest",
    "artifactHashes",
    "workerTreeHashes",
)
baseline_identity = None
identity_mismatches = []
trials = []
ratios = []
for pair, order in orders.items():
    measured = {profile: load(pair, profile) for profile in ("SERIAL", "PARALLEL")}
    for value in measured.values():
        identity = {field: value.get(field) for field in identity_fields}
        if baseline_identity is None:
            baseline_identity = identity
        elif identity != baseline_identity:
            identity_mismatches.append({
                "differingFields": sorted(
                    field for field in identity_fields
                    if identity.get(field) != baseline_identity.get(field)
                ),
                "observed": identity,
                "pair": pair,
                "profile": value["buildPlan"]["profile"],
            })
    serial_millis = measured["SERIAL"]["wallMillis"]
    parallel_millis = measured["PARALLEL"]["wallMillis"]
    ratio = parallel_millis / serial_millis
    ratios.append(ratio)
    trials.append({
        "order": order,
        "pair": pair,
        "parallel": {
            "buildPlan": measured["PARALLEL"]["buildPlan"],
            "wallMillis": parallel_millis,
        },
        "ratio": round(ratio, 6),
        "serial": {
            "buildPlan": measured["SERIAL"]["buildPlan"],
            "wallMillis": serial_millis,
        },
    })

median_ratio = statistics.median(ratios)
threshold = 0.65
passed = median_ratio <= threshold and not identity_mismatches
status = (
    "FAILED_NONDETERMINISTIC_CAPSULE"
    if identity_mismatches
    else "PASSED" if passed else "FAILED_RUNTIME_RATIO"
)
report = {
    "accepted": passed,
    "identity": baseline_identity,
    "identityMismatches": identity_mismatches,
    "measurements": {
        "medianRatio": round(median_ratio, 6),
        "pairs": trials,
    },
    "qualification": {
        "minimumPhysicalCores": 4,
        "physicalCores": physical_cores,
    },
    "releaseGatePassed": passed,
    "schema": "codeclew-real-cold-runtime-gate/1.0",
    "scope": {
        "multiCompilationGenerationPassed": False,
        "runtimeCapsuleColdBuildPassed": passed,
    },
    "sourceRevision": source_revision,
    "status": status,
    "thresholds": {"runtimeMedianRatioMax": threshold},
}
print(json.dumps(report, sort_keys=True, separators=(",", ":")))
PY
fi

mv -f -- "$TEMPORARY" "$REPORT"
chmod 600 "$REPORT"
FINISHED=1

python3 -I -S - "$REPORT" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
value = json.loads(path.read_text(encoding="utf-8"))
if value.get("schema") != "codeclew-real-cold-runtime-gate/1.0":
    raise SystemExit("cold runtime gate returned an unexpected schema")
status = value.get("status")
if value.get("accepted") is not True or status not in {
    "PASSED",
    "SKIPPED_UNQUALIFIED_HOST",
}:
    raise SystemExit("cold runtime gate failed")
if status == "SKIPPED_UNQUALIFIED_HOST" and value.get("releaseGatePassed") is not False:
    raise SystemExit("unqualified multicore host falsely passed the release gate")
if status == "PASSED":
    scope = value.get("scope", {})
    if scope.get("runtimeCapsuleColdBuildPassed") is not True:
        raise SystemExit("runtime capsule cold-build scope was not proved")
    if scope.get("multiCompilationGenerationPassed") is not False:
        raise SystemExit("runtime gate falsely claimed multi-compilation evidence")
PY
cat "$REPORT"
