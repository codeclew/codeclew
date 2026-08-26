#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
python3 -I -S "$ROOT/scripts/stabilization_control.py" guard --gate multi-compilation >/dev/null
umask 077
# Native PROJECT_NATIVE qualification must not inherit or reuse an ambient
# Gradle daemon. The script bytes are part of the check authority and the
# parent GRADLE_OPTS value is separately digested by native authority.
GRADLE_OPTS="${GRADLE_OPTS:+$GRADLE_OPTS }-Dorg.gradle.daemon=false"
export GRADLE_OPTS

REPORT="$ROOT/benchmarks/reports/multi-compilation-latest.json"
REPORT_TEMP="$REPORT.tmp.$$"
GATE_BASE=${CODECLEW_GATE_HOME:-"$HOME/.cache/codeclew-gates"}
SEED_BASE=${CODECLEW_SEED_HOME:-"$HOME/.cache/codeclew-seeds"}
WORK=
SOURCE=
SEED_STATE=
SEED_FILE=
ACTIVE_SESSION=
ACTIVE_STATE=
FINISHED=0
FAILURE_STAGE=SETUP
CLEANUP_HELPER="$ROOT/scripts/bounded_gate_cleanup.py"

mkdir -p "$ROOT/benchmarks/reports"

write_incomplete_report() {
    cause_file=
    current_file=
    if [ -n "$WORK" ]; then
        cause_file="$WORK/failure-cause"
        current_file="$WORK/current-stage"
    fi
    python3 -I -S - "$REPORT_TEMP" "$FAILURE_STAGE" "$cause_file" "$current_file" <<'PY'
import json
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
stage = sys.argv[2]
for candidate_path in sys.argv[3:]:
    if not candidate_path:
        continue
    try:
        candidate = pathlib.Path(candidate_path).read_text(encoding="ascii").strip()
    except OSError:
        continue
    if re.fullmatch(r"[A-Z0-9_]{1,128}", candidate):
        stage = candidate
        break
path.write_text(json.dumps({
    "accepted": False,
    "failureStage": stage,
    "releaseGatePassed": False,
    "schema": "codeclew-real-multi-compilation-gate/3.0",
    "status": "FAILED_INCOMPLETE",
}, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY
    mv -f -- "$REPORT_TEMP" "$REPORT"
    chmod 600 "$REPORT"
}

cleanup() {
    result=$?
    cleanup_failed=0
    trap - EXIT INT TERM
    rm -f -- "$REPORT_TEMP"
    if [ "$FINISHED" -eq 0 ]; then
        write_incomplete_report
    fi
    if [ -n "$ACTIVE_SESSION" ] && [ -n "$SOURCE" ] && [ -n "$ACTIVE_STATE" ] && [ -n "$SEED_FILE" ]; then
        CODECLEW_HOME="$ACTIVE_STATE" CODECLEW_RUNTIME_SEED="$SEED_FILE" \
            python3 -I -S "$CLEANUP_HELPER" \
                session --clew "$SOURCE/clew" --session "$ACTIVE_SESSION" || cleanup_failed=1
    fi
    if [ -n "$WORK" ]; then
        python3 -I -S "$CLEANUP_HELPER" \
            tree --path "$WORK" || cleanup_failed=1
    fi
    if [ "$cleanup_failed" -ne 0 ]; then
        if [ "$result" -eq 0 ]; then
            FAILURE_STAGE=CLEANUP_FAILED
        fi
        FINISHED=0
        write_incomplete_report || :
        result=1
    fi
    rmdir -- "$GATE_BASE" 2>/dev/null || :
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
        allowed = os.sched_getaffinity(0)
        processor = physical = core = None
        with open("/proc/cpuinfo", encoding="utf-8") as stream:
            for line in stream:
                if not line.strip():
                    if processor in allowed and physical is not None and core is not None:
                        pairs.add((physical, core))
                    processor = physical = core = None
                elif line.startswith("processor"):
                    processor = int(line.split(":", 1)[1].strip())
                elif line.startswith("physical id"):
                    physical = line.split(":", 1)[1].strip()
                elif line.startswith("core id"):
                    core = line.split(":", 1)[1].strip()
        if processor in allowed and physical is not None and core is not None:
            pairs.add((physical, core))
        return len(pairs)
    except (AttributeError, OSError, ValueError):
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
physical = linux_physical_cores() or min(sysctl_int("hw.physicalcpu"), logical)
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
    "accepted": False,
    "qualification": {
        "admittedGenerationJobs": int(sys.argv[4]),
        "logicalCpus": int(sys.argv[2]),
        "minimumAdmittedGenerationJobs": 4,
        "minimumPhysicalCores": 4,
        "physicalCores": int(sys.argv[1]),
        "totalMemoryBytes": int(sys.argv[3]),
    },
    "releaseGatePassed": False,
    "schema": "codeclew-real-multi-compilation-gate/3.0",
    "status": "SKIPPED_UNQUALIFIED_HOST",
    "thresholds": {"medianParallelRatioMax": 0.60},
}, sort_keys=True, separators=(",", ":")))
PY
    mv -f -- "$REPORT_TEMP" "$REPORT"
    chmod 600 "$REPORT"
    FINISHED=1
    cat "$REPORT"
    exit 1
fi

if [ -n "$(git status --porcelain=v1 --untracked-files=all)" ]; then
    FAILURE_STAGE=SOURCE_NOT_CLEAN
    echo "multi-compilation evidence requires a clean source HEAD" >&2
    exit 1
fi
SOURCE_REVISION=$(git rev-parse --verify HEAD)
SOURCE="$WORK/source"
git clone --quiet --no-local --no-checkout "$ROOT" "$SOURCE"
git -C "$SOURCE" checkout --quiet --detach "$SOURCE_REVISION"
CLEANUP_HELPER="$SOURCE/scripts/bounded_gate_cleanup.py"
if [ "$(git -C "$SOURCE" rev-parse --verify HEAD)" != "$SOURCE_REVISION" ] ||
    [ -n "$(git -C "$SOURCE" status --porcelain=v1 --untracked-files=all)" ]; then
    FAILURE_STAGE=FROZEN_CLONE_INVALID
    echo "multi-compilation frozen clone authority is invalid" >&2
    exit 1
fi

FAILURE_STAGE=TRUSTED_SEED_INVALID
python3 -I -S - "$SEED_BASE" "$WORK/seed-authority.json" "$SOURCE_REVISION" <<'PY'
import hashlib
import json
import os
import pathlib
import re
import stat
import sys

root = pathlib.Path(sys.argv[1])
output = pathlib.Path(sys.argv[2])
source_revision = sys.argv[3]
if (
    not root.is_absolute()
    or ".." in root.parts
    or root.resolve(strict=True) != root
):
    raise SystemExit("trusted seed home is unsafe")
root_metadata = root.lstat()
if (
    not stat.S_ISDIR(root_metadata.st_mode)
    or stat.S_ISLNK(root_metadata.st_mode)
    or root_metadata.st_uid != os.geteuid()
    or stat.S_IMODE(root_metadata.st_mode) != 0o700
):
    raise SystemExit("trusted seed home permissions are invalid")


def read_private(path, expected_mode, limit, label):
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) != expected_mode
            or metadata.st_size > limit
        ):
            raise SystemExit(f"{label} permissions are invalid")
        with os.fdopen(descriptor, "rb", closefd=False) as stream:
            value = stream.read(limit + 1)
        if len(value) > limit:
            raise SystemExit(f"{label} is oversized")
        return value
    finally:
        os.close(descriptor)


locator_path = root / "current.json"
locator = json.loads(read_private(locator_path, 0o600, 4096, "trusted seed locator"))
epoch = locator.get("epoch")
if (
    locator.get("schema") != "codeclew-trusted-seed-locator/2.0"
    or not isinstance(epoch, str)
    or re.fullmatch(r"release-N-[0-9a-f]{40}", epoch) is None
    or pathlib.PurePath(epoch).name != epoch
):
    raise SystemExit("trusted seed locator authority is invalid")
epoch_root = root / epoch
seed_path = epoch_root / "seed.json"
parallel_root = epoch_root / "parallel-state"
state = parallel_root / "v2"
for path in (epoch_root, parallel_root, state):
    metadata = path.lstat()
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or stat.S_ISLNK(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or path.resolve(strict=True) != path
    ):
        raise SystemExit("trusted seed epoch permissions are invalid")
seed = json.loads(read_private(seed_path, 0o400, 1024 * 1024, "trusted seed file"))
unsigned = dict(seed)
expected_digest = unsigned.pop("seedDigest", None)
actual_digest = "sha256:" + hashlib.sha256(
    json.dumps(unsigned, sort_keys=True, separators=(",", ":")).encode()
).hexdigest()
runtime_key = seed.get("runtimeKey")
if (
    seed.get("schema") != "codeclew-trusted-release-seed/1.0"
    or seed.get("mode") != "RELEASE"
    or expected_digest != actual_digest
    or locator.get("seedDigest") != expected_digest
    or locator.get("runtimeKey") != runtime_key
    or seed.get("sourceRevision") != source_revision
    or re.fullmatch(r"sha256:[0-9a-f]{64}", str(runtime_key)) is None
):
    raise SystemExit("trusted seed digest authority is invalid")
value = {
    "runtimeKey": runtime_key,
    "schema": "codeclew-multi-compilation-seed-locator/1.0",
    "seedDigest": expected_digest,
    "seedPath": str(seed_path),
    "stateRoot": str(parallel_root),
}
output.write_text(
    json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
os.chmod(output, 0o600)
PY
SEED_STATE=$(python3 -I -S -c \
    'import json,sys; print(json.load(open(sys.argv[1]))["stateRoot"])' \
    "$WORK/seed-authority.json")
SEED_RUNTIME_KEY=$(python3 -I -S -c \
    'import json,sys; print(json.load(open(sys.argv[1]))["runtimeKey"])' \
    "$WORK/seed-authority.json")
SEED_FILE=$(python3 -I -S -c \
    'import json,sys; print(json.load(open(sys.argv[1]))["seedPath"])' \
    "$WORK/seed-authority.json")

FAILURE_STAGE=RUNTIME_READINESS
READINESS_STATE="$WORK/runtime-readiness-state"
CODECLEW_HOME="$READINESS_STATE" CODECLEW_RUNTIME_SEED="$SEED_FILE" \
    "$SOURCE/clew" --bootstrap-warm-audit \
    >"$WORK/readiness.json" 2>"$WORK/readiness.stderr"
CODECLEW_HOME="$READINESS_STATE" CODECLEW_RUNTIME_SEED="$SEED_FILE" \
    "$SOURCE/clew" --bootstrap-warm-audit \
    >"$WORK/warm.json" 2>"$WORK/warm.stderr"
python3 -I -S - "$WORK/readiness.json" "$WORK/warm.json" "$READINESS_STATE" <<'PY'
import json
import pathlib
import re
import sys

readiness = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
warm = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
state = pathlib.Path(sys.argv[3]) / "v2" / "runtimes"
for value in (readiness, warm):
    if (
        value.get("schema") != "codeclew-bootstrap-warm-audit/2.0"
        or value.get("status") != "PASSED"
        or value.get("capsuleBuildInvoked") is not False
        or value.get("coldToolchainInvoked") is not False
    ):
        raise SystemExit("trusted runtime readiness is invalid")
if any(
    path.is_dir() and re.fullmatch(r"[0-9a-f]{64}", path.name)
    for path in state.iterdir()
):
    raise SystemExit("sealed runtime was copied into disposable state")
PY

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

    FAILURE_STAGE="TRIAL_${pair}_${profile}_MATERIALIZATION"
    materialize_repository "$repository"

    set -- session open --repo "$repository" --target-ref main --language kotlin
    module=0
    while [ "$module" -lt 12 ]; do
        compilation=$(printf ':module%02d/main' "$module")
        set -- "$@" --compilation "$compilation"
        module=$((module + 1))
    done
    if [ "$profile" = SERIAL ]; then
        set -- "$@" --generation-jobs 1
    fi
    FAILURE_STAGE="TRIAL_${pair}_${profile}_SESSION_OPEN"
    if ! session_json=$(CODECLEW_HOME="$state_home" CODECLEW_RUNTIME_SEED="$SEED_FILE" \
        "$SOURCE/clew" "$@" 2>"$error_log"); then
        echo "multi-compilation session open failed" >&2
        exit 1
    fi
    FAILURE_STAGE="TRIAL_${pair}_${profile}_SESSION_OPEN_PARSE"
    validated_session=$(printf '%s' "$session_json" | python3 -I -S -c '
import json, re, sys
value = json.load(sys.stdin)
session = value.get("session", {})
identifier = session.get("sessionId")
authority = session.get("authorityDigest")
if (
    value.get("schema") != "codeclew-session-open/4.0"
    or value.get("status") != "OPEN"
    or not isinstance(identifier, str)
    or re.fullmatch(
        r"session:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
        identifier,
    ) is None
    or session.get("runtimeMode") != "RELEASE"
    or session.get("runtimeKey") != sys.argv[1]
    or session.get("modelCachePolicy") != "NON_CACHEABLE"
    or re.fullmatch(r"sha256:[0-9a-f]{64}", str(authority)) is None
):
    raise SystemExit("session open returned invalid authority")
print(json.dumps({"authorityDigest": authority, "sessionId": identifier}, sort_keys=True, separators=(",", ":")))
' "$SEED_RUNTIME_KEY")
    session_id=$(printf '%s' "$validated_session" | python3 -I -S -c \
        'import json,sys; print(json.load(sys.stdin)["sessionId"])')
    session_authority_digest=$(printf '%s' "$validated_session" | python3 -I -S -c \
        'import json,sys; print(json.load(sys.stdin)["authorityDigest"])')
    ACTIVE_SESSION=$session_id
    ACTIVE_STATE=$state_home

    FAILURE_STAGE="TRIAL_${pair}_${profile}_CONTEXT"
    CODECLEW_HOME="$state_home" CODECLEW_RUNTIME_SEED="$SEED_FILE" \
        python3 -I -S - "$SOURCE/clew" "$session_id" "$session_authority_digest" "$profile" "$pair" \
        "$state_home" "$repository" "$WORK/current-stage" >"$output" <<'PY'
import json
import os
import pathlib
import selectors
import signal
import stat
import subprocess
import sys
import time

(
    clew,
    session_id,
    session_authority_digest,
    profile,
    pair,
    state,
    repository,
    stage_path_value,
) = sys.argv[1:]
sys.path.insert(0, str(pathlib.Path(clew).parent / "scripts"))
from multi_compilation_authority import (  # noqa: E402
    WorkspaceAuthorityError,
    refuse_copied_runtime,
    require_session_authority,
)
stage_path = pathlib.Path(stage_path_value)
cause_path = stage_path.with_name("failure-cause")


def record_stage(value):
    temporary = stage_path.with_name(f".{stage_path.name}.{os.getpid()}.tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="ascii") as stream:
            stream.write(value + "\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, stage_path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def record_cause(value):
    if cause_path.exists():
        return
    temporary = cause_path.with_name(f".{cause_path.name}.{os.getpid()}.tmp")
    try:
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w", encoding="ascii") as stream:
            stream.write(value + "\n")
            stream.flush()
            os.fsync(stream.fileno())
        try:
            os.link(temporary, cause_path)
        except FileExistsError:
            pass
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def invoke(arguments, timeout):
    process = subprocess.Popen(
        [clew, *arguments],
        env=os.environ,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    def terminate():
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                return
            process.wait()

    limits = {"stdout": 64 * 1024, "stderr": 1024 * 1024}
    buffers = {"stdout": bytearray(), "stderr": bytearray()}
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ, "stdout")
    selector.register(process.stderr, selectors.EVENT_READ, "stderr")
    deadline = time.monotonic() + timeout
    try:
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                terminate()
                raise SystemExit("multi-compilation command timed out")
            for key, _events in selector.select(min(remaining, 0.5)):
                name = key.data
                chunk = os.read(key.fileobj.fileno(), min(65536, limits[name] + 1 - len(buffers[name])))
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                buffers[name].extend(chunk)
                if len(buffers[name]) > limits[name]:
                    terminate()
                    raise SystemExit(f"multi-compilation command {name} exceeded its limit")
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            terminate()
            raise SystemExit("multi-compilation command timed out")
        process.wait(timeout=remaining)
    except subprocess.TimeoutExpired:
        terminate()
        raise SystemExit("multi-compilation command timed out")
    finally:
        selector.close()
    stdout = bytes(buffers["stdout"])
    stderr = bytes(buffers["stderr"])
    if b"\0" in stdout:
        raise SystemExit("multi-compilation command stdout is invalid")
    if b"\0" in stderr:
        raise SystemExit("multi-compilation command diagnostics are invalid")
    return process.returncode, stdout


failure = None
failure_stage = None
context = None
workspace_profile = None
started = time.perf_counter_ns()
try:
    record_stage(f"TRIAL_{pair}_{profile}_CONTEXT")
    returncode, stdout = invoke([
        "context", "create",
        "--session", session_id,
        "--intent", "inspect the twelve independent value declarations",
        "--term", "value00",
        "--max-roots", "256",
    ], 1200)
    elapsed_millis = (time.perf_counter_ns() - started) / 1_000_000
    if returncode != 0:
        failure = "multi-compilation context create failed"
        failure_stage = f"TRIAL_{pair}_{profile}_CONTEXT_COMMAND_FAILED"
    else:
        try:
            context = json.loads(stdout)
        except (UnicodeDecodeError, json.JSONDecodeError):
            failure = "multi-compilation context create returned invalid JSON"
            failure_stage = f"TRIAL_{pair}_{profile}_CONTEXT_JSON_INVALID"
        profile_path = pathlib.Path(state).joinpath(
            "v2",
            "sessions",
            session_id.removeprefix("session:"),
            "compilations",
            "workspace-profile.json",
        )
        if failure is None:
            try:
                metadata = profile_path.lstat()
                if (
                    stat.S_ISLNK(metadata.st_mode)
                    or not stat.S_ISREG(metadata.st_mode)
                    or metadata.st_uid != os.geteuid()
                    or stat.S_IMODE(metadata.st_mode) != 0o600
                    or metadata.st_size > 1024 * 1024
                ):
                    raise OSError("unsafe workspace profile")
                workspace_profile = json.loads(profile_path.read_text(encoding="utf-8"))
                require_session_authority(workspace_profile, session_authority_digest)
            except (OSError, json.JSONDecodeError, WorkspaceAuthorityError):
                failure = "multi-compilation run has no exact workspace profile"
                failure_stage = f"TRIAL_{pair}_{profile}_WORKSPACE_PROFILE_INVALID"
finally:
    if failure_stage is not None:
        record_cause(failure_stage)
    record_stage(f"TRIAL_{pair}_{profile}_SESSION_CLOSE")
    try:
        close_code, close_stdout = invoke(
            ["session", "close", "--session", session_id], 60
        )
    except BaseException:
        close_code, close_stdout = -1, b""
        failure = failure or "multi-compilation session close timed out"
        failure_stage = failure_stage or f"TRIAL_{pair}_{profile}_SESSION_CLOSE_TIMEOUT"
        record_cause(failure_stage)
    record_stage(f"TRIAL_{pair}_{profile}_SESSION_GC")
    try:
        gc_code, gc_stdout = invoke(
            ["session", "gc", "--session", session_id], 60
        )
    except BaseException:
        gc_code, gc_stdout = -1, b""
        failure = failure or "multi-compilation session GC timed out"
        failure_stage = failure_stage or f"TRIAL_{pair}_{profile}_SESSION_GC_TIMEOUT"
        record_cause(failure_stage)
    try:
        closed = json.loads(close_stdout)
        collected = json.loads(gc_stdout)
    except (UnicodeDecodeError, json.JSONDecodeError):
        failure = failure or "multi-compilation session cleanup returned invalid JSON"
        failure_stage = failure_stage or f"TRIAL_{pair}_{profile}_SESSION_CLEANUP_JSON_INVALID"
    else:
        if (
            close_code != 0
            or gc_code != 0
            or closed.get("lifecycle", {}).get("status") != "CLOSED"
            or collected.get("lifecycle", {}).get("status") != "GARBAGE_COLLECTED"
        ):
            failure = failure or "multi-compilation session cleanup failed"
            failure_stage = failure_stage or f"TRIAL_{pair}_{profile}_SESSION_CLEANUP_FAILED"
if failure is not None:
    record_cause(failure_stage or f"TRIAL_{pair}_{profile}_FAILED")
    record_stage(failure_stage or f"TRIAL_{pair}_{profile}_FAILED")
    raise SystemExit(failure)
record_stage(f"TRIAL_{pair}_{profile}_POST_CLEANUP_AUDIT")
try:
    refuse_copied_runtime(pathlib.Path(state))
except WorkspaceAuthorityError as error:
    raise SystemExit(str(error)) from error
worktrees = subprocess.run(
    ["git", "worktree", "list", "--porcelain"],
    cwd=repository,
    stdin=subprocess.DEVNULL,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    check=False,
)
if (
    worktrees.returncode != 0
    or sum(line.startswith(b"worktree ") for line in worktrees.stdout.splitlines()) != 1
):
    raise SystemExit("multi-compilation trial leaked a managed worktree")
stage_path.unlink()
try:
    cause_path.unlink()
except FileNotFoundError:
    pass
print(json.dumps({
    "context": context,
    "elapsedMillis": round(elapsed_millis, 3),
    "pair": int(pair),
    "profile": profile,
    "workspaceProfile": workspace_profile,
}, sort_keys=True, separators=(",", ":")))
PY
    ACTIVE_SESSION=
    ACTIVE_STATE=
}

# Alternating pair order reduces systematic thermal and cache-order bias. Every
# trial owns a fresh clean repository and session while all trials lease the
# same verified immutable RELEASE capsule. Repository identity prevents
# generation-head reuse; session close/GC removes each derived worktree.
run_trial 1 SERIAL
run_trial 1 PARALLEL
run_trial 2 PARALLEL
run_trial 2 SERIAL
run_trial 3 SERIAL
run_trial 3 PARALLEL

FAILURE_STAGE=REPORT_VALIDATION
python3 -I -S - "$WORK" "$SOURCE_REVISION" \
    "$PHYSICAL_CORES" "$LOGICAL_CPUS" "$TOTAL_MEMORY_BYTES" "$ADMITTED_JOBS" \
    "$SEED_RUNTIME_KEY" \
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
runtime_key = sys.argv[7]
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


if not is_digest(runtime_key):
    raise SystemExit("trusted runtime key is invalid")


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
baseline_workspace_contour = None
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
        workspace_profile = result.get("workspaceProfile")
        if (
            not isinstance(workspace_profile, dict)
            or workspace_profile.get("schema")
            != "codeclew-project-native-workspace-profile/3.0"
            or workspace_profile.get("baseRevision") != semantic["baseRevision"]
            or workspace_profile.get("compilationCount") != 12
            or workspace_profile.get("materializations") != 1
            or workspace_profile.get("derivedMountSets") != 1
            or workspace_profile.get("runtimeKey") != runtime_key
            or workspace_profile.get("repositorySnapshot")
            != semantic["repositorySnapshot"]
            or workspace_profile.get("workspaceSetAuthorizations") != 1
            or workspace_profile.get("authorizedCompilationCount") != 12
            or not is_digest(workspace_profile.get("sessionAuthorityDigest"))
            or not is_digest(workspace_profile.get("workspaceSetAuthorityDigest"))
            or not isinstance(workspace_profile.get("legacyOpenProjectCalls"), int)
            or workspace_profile["legacyOpenProjectCalls"] < 0
            or workspace_profile["legacyOpenProjectCalls"] > 12
        ):
            raise SystemExit("multi-compilation workspace profile is invalid")
        if baseline is None:
            baseline = semantic
        elif semantic != baseline:
            raise SystemExit("serial and parallel semantic compilation outputs differ")
        workspace_contour = {
            "authorizedCompilationCount": workspace_profile["authorizedCompilationCount"],
            "compilationCount": workspace_profile["compilationCount"],
            "derivedMountSets": workspace_profile["derivedMountSets"],
            "legacyOpenProjectCalls": workspace_profile["legacyOpenProjectCalls"],
            "materializations": workspace_profile["materializations"],
            "schema": workspace_profile["schema"],
            "workspaceSetAuthorityDigest": workspace_profile[
                "workspaceSetAuthorityDigest"
            ],
            "workspaceSetAuthorizations": workspace_profile[
                "workspaceSetAuthorizations"
            ],
        }
        expected_workspace_authority = digest({
            "compilations": [f":module{index:02d}/main" for index in range(12)],
            "language": "kotlin",
            "providerMode": "PROJECT_NATIVE_LEGACY_BRIDGE",
            "repositorySnapshot": semantic["snapshotId"],
            "schema": "codeclew-kotlin-workspace-set-authorization/1.0",
        })
        if workspace_contour["workspaceSetAuthorityDigest"] != expected_workspace_authority:
            raise SystemExit("workspace-set authority digest is not independently reproducible")
        if baseline_workspace_contour is None:
            baseline_workspace_contour = workspace_contour
        elif workspace_contour != baseline_workspace_contour:
            raise SystemExit("serial and parallel workspace authority contours differ")
        measured[profile] = {
            "elapsedMillis": elapsed,
            "semanticDigest": digest(semantic),
            "sessionAuthorityDigest": workspace_profile["sessionAuthorityDigest"],
            "workspaceAuthority": workspace_contour,
        }
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
set_authorization_limit = 1
legacy_call_limit = 12
workspace_contour_passed = (
    baseline_workspace_contour["workspaceSetAuthorizations"]
    == set_authorization_limit
    and baseline_workspace_contour["legacyOpenProjectCalls"] <= legacy_call_limit
)
passed = median_ratio <= threshold and workspace_contour_passed
status = (
    "FAILED_WORKSPACE_AUTHORITY_CONTOUR"
    if not workspace_contour_passed
    else "PASSED" if passed else "FAILED_PARALLEL_RATIO"
)
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
    "schema": "codeclew-real-multi-compilation-gate/3.0",
    "scope": {
        "compilationCount": 12,
        "fixture": "kotlin-multi-12",
        "publicWorkflow": ["session open", "context create"],
        "runtimeAuthority": "SHARED_TRUSTED_RELEASE_LEASE",
        "runtimePrimedBeforeTiming": True,
        "temporaryLegacyBridge": baseline_workspace_contour["legacyOpenProjectCalls"] > 0,
        "workspaceAuthority": "RUST_WORKSPACE_SET_AUTHORIZATION",
        "workspaceProfile": baseline_workspace_contour,
    },
    "semanticIdentity": {
        "baseRevision": baseline["baseRevision"],
        "compilationOutputs": baseline["compilations"],
        "digest": digest(baseline),
        "repositorySnapshot": baseline["repositorySnapshot"],
        "snapshotId": baseline["snapshotId"],
    },
    "runtimeKey": runtime_key,
    "sourceRevision": source_revision,
    "status": status,
    "thresholds": {
        "maxLegacyOpenProjectCalls": legacy_call_limit,
        "maxWorkspaceSetAuthorizations": set_authorization_limit,
        "medianParallelRatioMax": threshold,
    },
}
print(json.dumps(report, sort_keys=True, separators=(",", ":")))
PY

FINAL_STATUS=$(python3 -I -S - "$REPORT_TEMP" <<'PY'
import json
import pathlib
import sys

value = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
if value.get("schema") != "codeclew-real-multi-compilation-gate/3.0":
    raise SystemExit("multi-compilation gate returned an unexpected schema")
status = value.get("status")
if status not in {"PASSED", "FAILED_PARALLEL_RATIO", "FAILED_WORKSPACE_AUTHORITY_CONTOUR"}:
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
    echo "real multi-compilation gate failed ratio or workspace authority contour" >&2
    exit 1
fi
