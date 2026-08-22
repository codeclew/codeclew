#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$ROOT"
umask 077
python3 -I -S "$ROOT/scripts/stabilization_control.py" guard --gate provider-models >/dev/null

CONTROL_HOME=${CODECLEW_CONTROL_HOME:-"$HOME/.cache/codeclew-control"}
SEED_HOME=${CODECLEW_SEED_HOME:-"$HOME/.cache/codeclew-seeds"}
REVISION=$(git rev-parse --verify HEAD)
EVIDENCE_PARENT="$CONTROL_HOME/qualification/provider-models"
EVIDENCE_ROOT="$EVIDENCE_PARENT/$REVISION"
WORK_PARENT="$CONTROL_HOME/tmp"
mkdir -p "$EVIDENCE_PARENT" "$WORK_PARENT"
chmod 700 "$EVIDENCE_PARENT" "$WORK_PARENT"
mkdir "$EVIDENCE_ROOT"
chmod 700 "$EVIDENCE_ROOT"
WORK=$(mktemp -d "$WORK_PARENT/provider-models.XXXXXX")

cleanup() {
  result=$?
  trap - EXIT INT TERM
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
    raise SystemExit("provider gate cleanup root is not a physical directory")
for current, directories, _files in os.walk(root, topdown=False, followlinks=False):
    for name in directories:
        path = pathlib.Path(current, name)
        child = path.lstat()
        if stat.S_ISDIR(child.st_mode) and not stat.S_ISLNK(child.st_mode):
            path.chmod(0o700)
    pathlib.Path(current).chmod(0o700)
shutil.rmtree(root)
PY
  exit "$result"
}
trap cleanup EXIT INT TERM

python3 -I -S - "$ROOT" "$SEED_HOME" "$WORK" "$EVIDENCE_ROOT" "$REVISION" <<'PY'
import hashlib
import json
import os
import pathlib
import re
import signal
import stat
import subprocess
import sys
import time


class GateFailure(RuntimeError):
    pass


repository = pathlib.Path(sys.argv[1])
seed_home = pathlib.Path(sys.argv[2])
work = pathlib.Path(sys.argv[3])
evidence_root = pathlib.Path(sys.argv[4])
source_revision = sys.argv[5]
clew = repository / "clew"


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def sha256(value):
    return "sha256:" + hashlib.sha256(value).hexdigest()


def require_private_regular(path, mode):
    metadata = path.lstat()
    if (
        not stat.S_ISREG(metadata.st_mode)
        or stat.S_ISLNK(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) != mode
    ):
        raise GateFailure("trusted seed locator permissions are invalid")


def require_private_directory(path):
    metadata = path.lstat()
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or stat.S_ISLNK(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) & 0o077
    ):
        raise GateFailure("trusted seed state permissions are invalid")


def resolve_seed():
    locator_path = seed_home / "current.json"
    require_private_regular(locator_path, 0o600)
    locator = json.loads(locator_path.read_bytes())
    epoch = locator.get("epoch")
    if (
        locator.get("schema") != "codeclew-trusted-seed-locator/1.0"
        or not isinstance(epoch, str)
        or re.fullmatch(r"release-N-[0-9a-f]{40}", epoch) is None
        or pathlib.PurePath(epoch).name != epoch
    ):
        raise GateFailure("trusted seed locator authority is invalid")
    epoch_root = seed_home / epoch
    require_private_directory(epoch_root)
    seed_path = epoch_root / "seed.json"
    require_private_regular(seed_path, 0o400)
    seed = json.loads(seed_path.read_bytes())
    unsigned = dict(seed)
    expected_digest = unsigned.pop("seedDigest", None)
    actual_digest = sha256(canonical(unsigned).encode())
    if (
        seed.get("schema") != "codeclew-trusted-release-seed/1.0"
        or seed.get("mode") != "RELEASE"
        or expected_digest != actual_digest
        or locator.get("seedDigest") != expected_digest
        or locator.get("runtimeKey") != seed.get("runtimeKey")
        or re.fullmatch(r"sha256:[0-9a-f]{64}", str(seed.get("runtimeKey"))) is None
    ):
        raise GateFailure("trusted seed digest authority is invalid")
    state = epoch_root / "parallel-state"
    require_private_directory(state)
    return locator, seed, state


def run(arguments, *, cwd, environment, timeout, stage):
    started = time.perf_counter_ns()
    process = subprocess.Popen(
        [str(value) for value in arguments],
        cwd=cwd,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            stdout, stderr = process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            stdout, stderr = process.communicate()
        raise GateFailure(f"{stage} exceeded its bounded timeout")
    elapsed_ms = round((time.perf_counter_ns() - started) / 1_000_000, 3)
    if process.returncode != 0:
        raise GateFailure(f"{stage} failed")
    return stdout, stderr, elapsed_ms


def run_json(arguments, *, environment, timeout, stage):
    stdout, stderr, elapsed_ms = run(
        arguments,
        cwd=repository,
        environment=environment,
        timeout=timeout,
        stage=stage,
    )
    if stderr.strip():
        raise GateFailure(f"{stage} wrote unexpected stderr")
    try:
        value = json.loads(stdout)
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise GateFailure(f"{stage} returned invalid JSON")
    return value, stdout, elapsed_ms


def materialize(name, fixture):
    target = work / name
    target.mkdir(mode=0o700)
    archive = work / f"{name}.tar"
    with archive.open("wb") as stream:
        completed = subprocess.run(
            ["git", "archive", "--format=tar", "HEAD", fixture],
            cwd=repository,
            stdin=subprocess.DEVNULL,
            stdout=stream,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    if completed.returncode != 0:
        raise GateFailure(f"{name} archive failed")
    extracted = subprocess.run(
        ["tar", "-xf", archive, "-C", target, "--strip-components=2"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    archive.unlink()
    if extracted.returncode != 0 or (target / ".semantic-thread").exists():
        raise GateFailure(f"{name} materialization failed")
    run(
        ["git", "init", "-q", "-b", "main"],
        cwd=target,
        environment=os.environ.copy(),
        timeout=30,
        stage=f"{name} git init",
    )
    run(
        ["git", "add", "."],
        cwd=target,
        environment=os.environ.copy(),
        timeout=30,
        stage=f"{name} git add",
    )
    commit_environment = os.environ.copy()
    commit_environment.update({
        "GIT_AUTHOR_NAME": "Codeclew Gate",
        "GIT_AUTHOR_EMAIL": "gate@codeclew.invalid",
        "GIT_AUTHOR_DATE": "2000-01-01T00:00:00Z",
        "GIT_COMMITTER_NAME": "Codeclew Gate",
        "GIT_COMMITTER_EMAIL": "gate@codeclew.invalid",
        "GIT_COMMITTER_DATE": "2000-01-01T00:00:00Z",
    })
    run(
        ["git", "-c", "commit.gpgsign=false", "commit", "-q", "-m", "baseline"],
        cwd=target,
        environment=commit_environment,
        timeout=30,
        stage=f"{name} baseline commit",
    )
    return target


def validate_audit(value, *, exact_warm):
    counters = value.get("counters", {})
    if (
        value.get("schema") != "codeclew-bootstrap-warm-audit/2.0"
        or value.get("status") != "PASSED"
        or value.get("capsuleBuildInvoked") is not False
        or value.get("coldToolchainInvoked") is not False
    ):
        raise GateFailure("trusted runtime readiness audit failed")
    if exact_warm and (
        counters.get("checkpointHits") != 1
        or counters.get("checkpointMisses") != 0
        or counters.get("digestFileCalls") != 0
        or counters.get("processRuns") != 0
    ):
        raise GateFailure("trusted runtime did not reach the exact warm path")


def validate_context(value, raw, compilation):
    context = value.get("context", {})
    snapshot = context.get("snapshot", {})
    snapshot_compilations = snapshot.get("compilations", [])
    completeness = value.get("completeness", {})
    versions = context.get("compilerVersions", {})
    if (
        value.get("schema") != "codeclew-context-result/2.0"
        or not isinstance(value.get("contextId"), str)
        or not value["contextId"].startswith("context:")
        or len(raw) > 64 * 1024
        or context.get("schema") != "codeclew-bounded-context-projection/4.0"
        or context.get("compilations") != [compilation]
        or set(versions) != {compilation}
        or not isinstance(versions.get(compilation), str)
        or not versions[compilation]
        or len(snapshot_compilations) != 1
        or snapshot_compilations[0].get("compilation") != compilation
        or snapshot_compilations[0].get("compilerVersion") != versions[compilation]
        or not str(snapshot_compilations[0].get("generation", {}).get("digest", "")).startswith("sha256:")
        or not str(snapshot_compilations[0].get("queryIndex", {}).get("digest", "")).startswith("sha256:")
        or completeness.get("status") not in {"COMPLETE_TASK", "CONDITIONAL_TASK"}
        or completeness.get("support") != "SUPPORTED"
        or context.get("generationAuthority", {}).get("coverage") not in {"COMPLETE", "PARTIAL"}
        or context.get("generationAuthority", {}).get("certainty") not in {"VERIFIED", "UNSURE"}
    ):
        raise GateFailure("provider context authority is invalid")
    return {
        "compilerVersion": versions[compilation],
        "completeness": completeness.get("status"),
        "contextDigest": sha256(raw),
        "generationDigest": snapshot_compilations[0]["generation"]["digest"],
        "queryIndexDigest": snapshot_compilations[0]["queryIndex"]["digest"],
        "stdoutBytes": len(raw),
    }


def qualify_provider(name, path, native_command, term, environment):
    session_id = None
    cleanup_error = None
    try:
        native_stdout, native_stderr, native_ms = run(
            native_command,
            cwd=path,
            environment=os.environ.copy(),
            timeout=600,
            stage=f"{name} native build",
        )
        session, session_raw, session_ms = run_json(
            [
                clew,
                "session", "open", "--json",
                "--repo", path,
                "--target-ref", "main",
                "--compilation", ":/main",
            ],
            environment=environment,
            timeout=300,
            stage=f"{name} session open",
        )
        authority = session.get("session", {})
        session_id = authority.get("sessionId")
        if (
            session.get("schema") != "codeclew-session-open/4.0"
            or session.get("status") != "OPEN"
            or not isinstance(session_id, str)
            or not session_id.startswith("session:")
            or authority.get("runtimeMode") != "RELEASE"
            or authority.get("compilations") != [":/main"]
            or authority.get("modelCachePolicy") != "NON_CACHEABLE"
            or not str(authority.get("runtimeKey", "")).startswith("sha256:")
        ):
            raise GateFailure(f"{name} session authority is invalid")
        context, context_raw, context_ms = run_json(
            [
                clew,
                "context", "create", "--json",
                "--session", session_id,
                "--intent", f"inspect {name} provider model and generation authority",
                "--term", term,
                "--max-roots", "8",
            ],
            environment=environment,
            timeout=600,
            stage=f"{name} context create",
        )
        context_summary = validate_context(context, context_raw, ":/main")
        return {
            **context_summary,
            "contextMillis": context_ms,
            "nativeBuildMillis": native_ms,
            "nativeOutputDigest": sha256(native_stdout + b"\0" + native_stderr),
            "provider": name,
            "runtimeKey": authority["runtimeKey"],
            "sessionMillis": session_ms,
            "sessionOutputDigest": sha256(session_raw),
        }
    finally:
        if session_id is not None:
            try:
                closed, _raw, _elapsed = run_json(
                    [clew, "session", "close", "--json", "--session", session_id],
                    environment=environment,
                    timeout=60,
                    stage=f"{name} session close",
                )
                if closed.get("lifecycle", {}).get("status") != "CLOSED":
                    raise GateFailure(f"{name} session did not close")
                collected, _raw, _elapsed = run_json(
                    [clew, "session", "gc", "--json", "--session", session_id],
                    environment=environment,
                    timeout=60,
                    stage=f"{name} session gc",
                )
                if collected.get("lifecycle", {}).get("status") != "GARBAGE_COLLECTED":
                    raise GateFailure(f"{name} session did not garbage collect")
            except GateFailure as error:
                cleanup_error = error
        if cleanup_error is not None:
            raise cleanup_error


try:
    locator, seed, state = resolve_seed()
    clew_environment = os.environ.copy()
    clew_environment["CODECLEW_HOME"] = str(state)

    readiness, _raw, readiness_ms = run_json(
        [clew, "--bootstrap-warm-audit"],
        environment=clew_environment,
        timeout=120,
        stage="runtime readiness",
    )
    validate_audit(readiness, exact_warm=False)
    warm, _raw, warm_ms = run_json(
        [clew, "--bootstrap-warm-audit"],
        environment=clew_environment,
        timeout=30,
        stage="runtime exact warm audit",
    )
    validate_audit(warm, exact_warm=True)

    gradle = materialize("gradle", "fixtures/kotlin-basic")
    maven = materialize("maven", "fixtures/kotlin-maven")
    providers = [
        qualify_provider(
            "gradle",
            gradle,
            [gradle / "gradlew", "--no-daemon", "--quiet", "compileKotlin"],
            "com.acme.total",
            clew_environment,
        ),
        qualify_provider(
            "maven",
            maven,
            [maven / "mvnw", "-q", "-DskipTests", "compile"],
            "MavenFlow",
            clew_environment,
        ),
    ]
    if len({value["runtimeKey"] for value in providers}) != 1:
        raise GateFailure("providers did not use one trusted runtime")
    if providers[0]["runtimeKey"] != locator["runtimeKey"]:
        raise GateFailure("provider runtime differs from the trusted seed")

    qualification = {
        "coldToolchainInvoked": False,
        "providers": providers,
        "readinessMillis": readiness_ms,
        "runtimeKey": locator["runtimeKey"],
        "schema": "codeclew-provider-model-qualification/1.0",
        "seedDigest": locator["seedDigest"],
        "seedSourceRevision": seed["sourceRevision"],
        "sourceRevision": source_revision,
        "status": "PASS",
        "warmAuditMillis": warm_ms,
        "warmCounters": warm["counters"],
    }
    temporary = evidence_root / ".qualification.json.tmp"
    temporary.write_text(canonical(qualification) + "\n", encoding="utf-8")
    os.chmod(temporary, 0o400)
    os.replace(temporary, evidence_root / "qualification.json")
    print(canonical(qualification))
except (GateFailure, json.JSONDecodeError, OSError, subprocess.SubprocessError) as error:
    print(f"provider-models failed: {error}", file=sys.stderr)
    raise SystemExit(1)
PY
