#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
umask 077

REPORT="$ROOT/benchmarks/reports/multi-compilation-latest.json"
REPORT_TEMP="$REPORT.tmp.$$"
GATE_BASE=${CODECLEW_GATE_HOME:-"$HOME/.cache/codeclew-gates"}
WORK=
SOURCE=
FINISHED=0

mkdir -p "$ROOT/benchmarks/reports"

write_incomplete_report() {
    python3 -I -S - "$REPORT_TEMP" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
path.write_text(json.dumps({
    "accepted": False,
    "releaseGatePassed": False,
    "schema": "codeclew-real-multi-compilation-gate/1.0",
    "status": "FAILED_INCOMPLETE",
}, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY
    mv -f -- "$REPORT_TEMP" "$REPORT"
    chmod 600 "$REPORT"
}

cleanup() {
    result=$?
    trap - EXIT INT TERM
    rm -f -- "$REPORT_TEMP"
    if [ -n "$WORK" ]; then
        python3 -I -S - "$WORK" <<'PY'
import os
import pathlib
import shutil
import stat
import sys

root = pathlib.Path(sys.argv[1])
if root.exists():
    metadata = root.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
        raise SystemExit("multi-compilation cleanup root is not a physical directory")
    for current, directories, _files in os.walk(root, topdown=False, followlinks=False):
        for name in directories:
            path = pathlib.Path(current, name)
            child = path.lstat()
            if stat.S_ISDIR(child.st_mode) and not stat.S_ISLNK(child.st_mode):
                path.chmod(0o700)
        pathlib.Path(current).chmod(0o700)
    shutil.rmtree(root)
PY
    fi
    rmdir -- "$GATE_BASE" 2>/dev/null || :
    if [ "$FINISHED" -eq 0 ]; then
        write_incomplete_report
    fi
    exit "$result"
}
trap cleanup EXIT INT TERM

python3 -I -S - "$GATE_BASE" <<'PY'
import os
import pathlib
import stat
import sys

base = pathlib.Path(sys.argv[1])
if not base.is_absolute() or ".." in base.parts:
    raise SystemExit("multi-compilation gate base must be normalized and absolute")
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
            raise SystemExit("multi-compilation gate base has an unsafe ancestor")
        if leaf:
            if metadata.st_uid != os.geteuid():
                os.close(child)
                raise SystemExit("multi-compilation gate base has a different owner")
            os.fchmod(child, 0o700)
        os.close(descriptor)
        descriptor = child
finally:
    os.close(descriptor)
PY

WORK=$(mktemp -d "$GATE_BASE/multi-compilation.XXXXXX")

set -- $(python3 -I -S - <<'PY'
import os
import pathlib
import subprocess


def sysctl_int(name):
    try:
        return int(subprocess.run(
            ["sysctl", "-n", name],
            check=True,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        ).stdout.strip())
    except (FileNotFoundError, subprocess.SubprocessError, ValueError):
        return 0


def linux_physical_cores():
    try:
        pairs = set()
        physical = core = None
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
        return len(pairs)
    except OSError:
        return 0


def memory_bytes():
    value = sysctl_int("hw.memsize")
    if value:
        return value
    cgroup = None
    try:
        raw = pathlib.Path("/sys/fs/cgroup/memory.max").read_text(encoding="ascii").strip()
        if raw != "max":
            cgroup = int(raw)
    except (OSError, ValueError):
        pass
    try:
        with open("/proc/meminfo", encoding="ascii") as stream:
            for line in stream:
                if line.startswith("MemTotal:"):
                    host = int(line.split()[1]) * 1024
                    return min(host, cgroup) if cgroup is not None else host
    except (OSError, ValueError, IndexError):
        pass
    return cgroup or 0


try:
    logical = len(os.sched_getaffinity(0))
except AttributeError:
    logical = os.cpu_count() or 1
physical = sysctl_int("hw.physicalcpu") or linux_physical_cores()
total_memory = memory_bytes()
reserved = max(1024**3, total_memory * 15 // 100)
budget = max(0, total_memory - reserved) * 70 // 100
admitted = max(1, min(logical, budget // (2 * 1024**3), 12, 16))
print(physical, logical, total_memory, admitted)
PY
)
PHYSICAL_CORES=$1
LOGICAL_CPUS=$2
TOTAL_MEMORY_BYTES=$3
ADMITTED_JOBS=$4

if [ "$PHYSICAL_CORES" -lt 4 ] || [ "$ADMITTED_JOBS" -lt 4 ]; then
    python3 -I -S - \
        "$PHYSICAL_CORES" "$LOGICAL_CPUS" "$TOTAL_MEMORY_BYTES" "$ADMITTED_JOBS" \
        >"$REPORT_TEMP" <<'PY'
import json
import sys

print(json.dumps({
    "accepted": True,
    "qualification": {
        "admittedGenerationJobs": int(sys.argv[4]),
        "logicalCpus": int(sys.argv[2]),
        "minimumAdmittedGenerationJobs": 4,
        "minimumPhysicalCores": 4,
        "physicalCores": int(sys.argv[1]),
        "totalMemoryBytes": int(sys.argv[3]),
    },
    "releaseGatePassed": False,
    "schema": "codeclew-real-multi-compilation-gate/1.0",
    "status": "SKIPPED_UNQUALIFIED_HOST",
    "thresholds": {"medianParallelRatioMax": 0.60},
}, sort_keys=True, separators=(",", ":")))
PY
    mv -f -- "$REPORT_TEMP" "$REPORT"
    chmod 600 "$REPORT"
    FINISHED=1
    cat "$REPORT"
    exit 0
fi

if [ -n "$(git status --porcelain=v1 --untracked-files=all)" ]; then
    echo "multi-compilation evidence requires a clean source HEAD" >&2
    exit 1
fi
SOURCE_REVISION=$(git rev-parse --verify HEAD)
SOURCE="$WORK/source"
git clone --quiet --no-local --no-checkout "$ROOT" "$SOURCE"
git -C "$SOURCE" checkout --quiet --detach "$SOURCE_REVISION"
if [ "$(git -C "$SOURCE" rev-parse --verify HEAD)" != "$SOURCE_REVISION" ] ||
    [ -n "$(git -C "$SOURCE" status --porcelain=v1 --untracked-files=all)" ]; then
    echo "multi-compilation frozen clone authority is invalid" >&2
    exit 1
fi

materialize_repository() {
    repository=$1
    mkdir -p "$repository"
    git -C "$SOURCE" archive HEAD fixtures/kotlin-multi-12 |
        tar -x -C "$repository" --strip-components=2
    mkdir -p "$repository/gradle/wrapper"
    cp "$SOURCE/fixtures/kotlin-basic/gradlew" "$repository/gradlew"
    cp "$SOURCE/fixtures/kotlin-basic/gradle/wrapper/gradle-wrapper.jar" \
        "$repository/gradle/wrapper/gradle-wrapper.jar"
    cp "$SOURCE/fixtures/kotlin-basic/gradle/wrapper/gradle-wrapper.properties" \
        "$repository/gradle/wrapper/gradle-wrapper.properties"
    chmod 755 "$repository/gradlew"
    git init -q -b main "$repository"
    git -C "$repository" add .
    GIT_AUTHOR_NAME='Codeclew Gate' \
    GIT_AUTHOR_EMAIL='gate@codeclew.invalid' \
    GIT_AUTHOR_DATE='2000-01-01T00:00:00Z' \
    GIT_COMMITTER_NAME='Codeclew Gate' \
    GIT_COMMITTER_EMAIL='gate@codeclew.invalid' \
    GIT_COMMITTER_DATE='2000-01-01T00:00:00Z' \
        git -C "$repository" -c commit.gpgsign=false commit -q -m baseline
    if [ -n "$(git -C "$repository" status --porcelain=v1 --untracked-files=all)" ]; then
        echo "materialized multi-compilation repository is not clean" >&2
        exit 1
    fi
}

run_trial() {
    pair=$1
    profile=$2
    lowercase=$(printf '%s' "$profile" | tr '[:upper:]' '[:lower:]')
    repository="$WORK/pair-$pair-$lowercase-repository"
    state_home="$WORK/pair-$pair-$lowercase-state"
    output="$WORK/pair-$pair-$lowercase.json"
    error_log="$WORK/pair-$pair-$lowercase.stderr"

    materialize_repository "$repository"
    if ! CODECLEW_HOME="$state_home" "$SOURCE/clew" --help \
        >"$WORK/pair-$pair-$lowercase-prime.stdout" 2>"$error_log"; then
        echo "multi-compilation runtime prime failed" >&2
        exit 1
    fi
    runtime_count=$(python3 -I -S - "$state_home/runtimes" <<'PY'
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
print(sum(
    path.is_dir() and re.fullmatch(r"[0-9a-f]{64}", path.name) is not None
    for path in root.iterdir()
))
PY
)
    if [ "$runtime_count" -ne 1 ]; then
        echo "multi-compilation trial did not prime exactly one runtime" >&2
        exit 1
    fi

    set -- session open --repo "$repository" --target-ref main
    module=0
    while [ "$module" -lt 12 ]; do
        compilation=$(printf ':module%02d/main' "$module")
        set -- "$@" --compilation "$compilation"
        module=$((module + 1))
    done
    if [ "$profile" = SERIAL ]; then
        set -- "$@" --generation-jobs 1
    fi
    if ! session_json=$(CODECLEW_HOME="$state_home" "$SOURCE/clew" "$@" 2>"$error_log"); then
        echo "multi-compilation session open failed" >&2
        exit 1
    fi
    session_id=$(printf '%s' "$session_json" | python3 -I -S -c '
import json, sys
value = json.load(sys.stdin)
session = value.get("session", {})
identifier = session.get("sessionId")
if not isinstance(identifier, str) or not identifier.startswith("session:"):
    raise SystemExit("session open returned invalid authority")
print(identifier)
')

    CODECLEW_HOME="$state_home" python3 -I -S - \
        "$SOURCE/clew" "$session_id" "$profile" "$pair" >"$output" <<'PY'
import json
import os
import subprocess
import sys
import time

clew, session_id, profile, pair = sys.argv[1:]
started = time.perf_counter_ns()
completed = subprocess.run(
    [
        clew,
        "context", "create",
        "--session", session_id,
        "--intent", "inspect the twelve independent value declarations",
        "--term", "value00",
        "--max-roots", "256",
    ],
    env=os.environ,
    stdin=subprocess.DEVNULL,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    check=False,
)
elapsed_millis = (time.perf_counter_ns() - started) / 1_000_000
if completed.returncode != 0:
    raise SystemExit("multi-compilation context create failed")
try:
    context = json.loads(completed.stdout)
except (UnicodeDecodeError, json.JSONDecodeError):
    raise SystemExit("multi-compilation context create returned invalid JSON")
print(json.dumps({
    "context": context,
    "elapsedMillis": round(elapsed_millis, 3),
    "pair": int(pair),
    "profile": profile,
}, sort_keys=True, separators=(",", ":")))
PY
}

# Alternating pair order reduces systematic thermal and cache-order bias. Every
# trial owns a fresh clean repository, state home, session, and primed capsule.
run_trial 1 SERIAL
run_trial 1 PARALLEL
run_trial 2 PARALLEL
run_trial 2 SERIAL
run_trial 3 SERIAL
run_trial 3 PARALLEL

python3 -I -S - "$WORK" "$SOURCE_REVISION" \
    "$PHYSICAL_CORES" "$LOGICAL_CPUS" "$TOTAL_MEMORY_BYTES" "$ADMITTED_JOBS" \
    >"$REPORT_TEMP" <<'PY'
import hashlib
import json
import pathlib
import re
import statistics
import sys

work = pathlib.Path(sys.argv[1])
source_revision = sys.argv[2]
physical_cores = int(sys.argv[3])
logical_cpus = int(sys.argv[4])
total_memory_bytes = int(sys.argv[5])
admitted_jobs = int(sys.argv[6])
orders = {
    1: ["SERIAL", "PARALLEL"],
    2: ["PARALLEL", "SERIAL"],
    3: ["SERIAL", "PARALLEL"],
}


def digest(value):
    encoded = json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def is_digest(value):
    return isinstance(value, str) and re.fullmatch(r"sha256:[0-9a-f]{64}", value) is not None


def cas_identity(value, label):
    if not isinstance(value, dict):
        raise SystemExit(f"{label} is not a CAS identity")
    identity = {
        "digest": value.get("digest"),
        "objectSchema": value.get("objectSchema"),
        "schema": value.get("schema"),
        "size": value.get("size"),
    }
    if (
        not is_digest(identity["digest"])
        or not isinstance(identity["objectSchema"], str)
        or identity["schema"] != "codeclew-cas-object/2.0"
        or not isinstance(identity["size"], int)
        or identity["size"] <= 0
    ):
        raise SystemExit(f"{label} has invalid CAS authority")
    return identity


def normalized_snapshot(result):
    context_result = result.get("context")
    if not isinstance(context_result, dict):
        raise SystemExit("context result is missing")
    if context_result.get("schema") != "codeclew-context-result/2.0":
        raise SystemExit("context result schema is invalid")
    projection = context_result.get("context")
    snapshot = projection.get("snapshot") if isinstance(projection, dict) else None
    if not isinstance(snapshot, dict):
        raise SystemExit("context projection has no semantic snapshot")
    compilations = snapshot.get("compilations")
    if not isinstance(compilations, list) or len(compilations) != 12:
        raise SystemExit("context projection does not contain twelve compilation outputs")
    expected = [f":module{index:02d}/main" for index in range(12)]
    outputs = []
    for value in compilations:
        if not isinstance(value, dict):
            raise SystemExit("compilation output is invalid")
        outputs.append({
            "compilation": value.get("compilation"),
            "compilerVersion": value.get("compilerVersion"),
            "generation": cas_identity(value.get("generation"), "generation"),
            "queryIndex": cas_identity(value.get("queryIndex"), "query index"),
        })
    outputs.sort(key=lambda value: value["compilation"] or "")
    if [value["compilation"] for value in outputs] != expected:
        raise SystemExit("context projection compilation set is not exact")
    if any(not isinstance(value["compilerVersion"], str) for value in outputs):
        raise SystemExit("context projection compiler version is invalid")
    normalized = {
        "baseRevision": snapshot.get("baseRevision"),
        "compilations": outputs,
        "repositorySnapshot": cas_identity(
            snapshot.get("repositorySnapshot"), "repository snapshot"
        ),
        "snapshotId": snapshot.get("snapshotId"),
    }
    if not re.fullmatch(r"[0-9a-f]{40,64}", normalized["baseRevision"] or ""):
        raise SystemExit("semantic snapshot base revision is invalid")
    if not is_digest(normalized["snapshotId"]):
        raise SystemExit("semantic snapshot id is invalid")
    encoded = json.dumps(normalized, sort_keys=True, separators=(",", ":"))
    if re.search(r'(?<![A-Za-z0-9])[A-Za-z]:[\\/]|(?:^|["])/', encoded):
        raise SystemExit("semantic comparison accidentally contains an absolute path")
    return normalized


baseline = None
ratios = []
trials = []
for pair, order in orders.items():
    measured = {}
    for profile in ("SERIAL", "PARALLEL"):
        result = json.loads(
            (work / f"pair-{pair}-{profile.lower()}.json").read_text(encoding="utf-8")
        )
        if result.get("profile") != profile or result.get("pair") != pair:
            raise SystemExit("trial identity is invalid")
        elapsed = result.get("elapsedMillis")
        if not isinstance(elapsed, (int, float)) or elapsed <= 0:
            raise SystemExit("trial elapsed time is invalid")
        semantic = normalized_snapshot(result)
        if baseline is None:
            baseline = semantic
        elif semantic != baseline:
            raise SystemExit("serial and parallel semantic compilation outputs differ")
        measured[profile] = {"elapsedMillis": elapsed, "semanticDigest": digest(semantic)}
    ratio = measured["PARALLEL"]["elapsedMillis"] / measured["SERIAL"]["elapsedMillis"]
    ratios.append(ratio)
    trials.append({
        "order": order,
        "pair": pair,
        "parallel": measured["PARALLEL"],
        "ratio": round(ratio, 6),
        "serial": measured["SERIAL"],
    })

median_ratio = statistics.median(ratios)
threshold = 0.60
passed = median_ratio <= threshold
report = {
    "accepted": passed,
    "measurements": {
        "medianParallelRatio": round(median_ratio, 6),
        "pairs": trials,
    },
    "qualification": {
        "admittedGenerationJobs": admitted_jobs,
        "logicalCpus": logical_cpus,
        "minimumAdmittedGenerationJobs": 4,
        "minimumPhysicalCores": 4,
        "physicalCores": physical_cores,
        "totalMemoryBytes": total_memory_bytes,
    },
    "releaseGatePassed": passed,
    "schema": "codeclew-real-multi-compilation-gate/1.0",
    "scope": {
        "compilationCount": 12,
        "fixture": "kotlin-multi-12",
        "publicWorkflow": ["session open", "context create"],
        "runtimePrimedBeforeTiming": True,
    },
    "semanticIdentity": {
        "baseRevision": baseline["baseRevision"],
        "compilationOutputs": baseline["compilations"],
        "digest": digest(baseline),
        "repositorySnapshot": baseline["repositorySnapshot"],
        "snapshotId": baseline["snapshotId"],
    },
    "sourceRevision": source_revision,
    "status": "PASSED" if passed else "FAILED_PARALLEL_RATIO",
    "thresholds": {"medianParallelRatioMax": threshold},
}
print(json.dumps(report, sort_keys=True, separators=(",", ":")))
PY

FINAL_STATUS=$(python3 -I -S - "$REPORT_TEMP" <<'PY'
import json
import pathlib
import sys

value = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
if value.get("schema") != "codeclew-real-multi-compilation-gate/1.0":
    raise SystemExit("multi-compilation gate returned an unexpected schema")
status = value.get("status")
if status not in {"PASSED", "FAILED_PARALLEL_RATIO"}:
    raise SystemExit("multi-compilation gate did not complete")
if (status == "PASSED") != (value.get("releaseGatePassed") is True):
    raise SystemExit("multi-compilation release gate status is inconsistent")
print(status)
PY
)
mv -f -- "$REPORT_TEMP" "$REPORT"
chmod 600 "$REPORT"
FINISHED=1
cat "$REPORT"
if [ "$FINAL_STATUS" != PASSED ]; then
    echo "real multi-compilation median parallel ratio exceeds 0.60" >&2
    exit 1
fi
