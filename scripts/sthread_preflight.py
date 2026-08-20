#!/usr/bin/env python3
"""Prepare and prove the local SThread runtime before an agent-context run.

The command is intentionally quiet: dependency caches are copied by the OS and
subprocess output is retained only as hashes/bounded failure summaries.  A
successful receipt means that the trusted worker can open the requested Gradle
model and that the Kotlin 2.1 and 2.3 compiler-semantic smoke fixtures pass.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from typing import Any, Sequence


SCHEMA = "codeclew.sthread-preflight/0.1"
ROOT = Path(__file__).resolve().parents[1]
GRADLE_CACHE_MEMBERS = ("caches", "wrapper", "jdks")
CACHE_MARKER_SCHEMA = "codeclew.sthread-cache-hydration/0.1"
CACHE_MARKER_NAME = ".codeclew-sthread-preflight-cache.json"
DEFAULT_SMOKES = (
    ("KOTLIN_2_1_COMPILER_SEMANTIC", "fixtures/kotlin-2-1", ":/main", "GRADLE"),
    ("KOTLIN_2_3_COMPILER_SEMANTIC", "fixtures/kotlin-maven", ":/main", "MAVEN"),
)


class PreflightFailure(RuntimeError):
    def __init__(self, stage: str, message: str, *, detail: dict[str, Any] | None = None):
        super().__init__(message)
        self.stage = stage
        self.detail = detail or {}


def monotonic_millis(start: float) -> int:
    return round((time.monotonic() - start) * 1000)


def sha256_bytes(raw: bytes) -> str:
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return "sha256:" + digest.hexdigest()


def canonical(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n").encode()


def atomic_json(path: Path, value: dict[str, Any]) -> None:
    path = path.resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = canonical(value)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def run_capture(
    argv: Sequence[str],
    *,
    cwd: Path,
    timeout_seconds: float,
    stage: str,
    environment: dict[str, str] | None = None,
) -> tuple[subprocess.CompletedProcess[bytes], int]:
    if timeout_seconds <= 0:
        raise PreflightFailure(stage, "preflight wall-time budget was exhausted")
    started = time.monotonic()
    try:
        completed = subprocess.run(
            list(argv),
            cwd=cwd,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout_seconds,
            env=environment,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise PreflightFailure(
            stage,
            "preflight command exceeded its remaining wall-time budget",
            detail={"argv": list(argv), "timeoutMillis": round(timeout_seconds * 1000)},
        ) from error
    return completed, monotonic_millis(started)


def json_values(raw: bytes) -> list[Any]:
    text = raw.decode("utf-8", errors="replace")
    decoder = json.JSONDecoder()
    values: list[Any] = []
    cursor = 0
    while cursor < len(text):
        brace = text.find("{", cursor)
        if brace < 0:
            break
        try:
            value, length = decoder.raw_decode(text[brace:])
        except json.JSONDecodeError:
            cursor = brace + 1
            continue
        values.append(value)
        cursor = brace + length
    return values


def failure_summary(completed: subprocess.CompletedProcess[bytes]) -> dict[str, Any]:
    values = json_values(completed.stdout)
    error = next(
        (
            value.get("error")
            for value in reversed(values)
            if isinstance(value, dict) and isinstance(value.get("error"), dict)
        ),
        None,
    )
    summary: dict[str, Any] = {
        "exitCode": completed.returncode,
        "stdoutSha256": sha256_bytes(completed.stdout),
        "stderrSha256": sha256_bytes(completed.stderr),
    }
    if error is not None:
        summary["code"] = error.get("code")
        message = error.get("message")
        if isinstance(message, str):
            summary["message"] = message[:2048]
            summary["messageSha256"] = sha256_bytes(message.encode())
    elif completed.stderr:
        bounded = completed.stderr.decode("utf-8", errors="replace")[-2048:]
        summary["message"] = bounded
        summary["messageSha256"] = sha256_bytes(completed.stderr)
    return summary


def require_real_directory(path: Path, stage: str) -> Path:
    if path.is_symlink() or not path.is_dir():
        raise PreflightFailure(stage, f"required directory is absent or a symlink: {path}")
    return path.resolve()


def tracked_files(workspace: Path) -> list[Path]:
    completed = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=workspace,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise PreflightFailure("GIT_STATE", "git ls-files failed", detail=failure_summary(completed))
    selected: list[Path] = []
    for token in completed.stdout.split(b"\0"):
        if not token:
            continue
        relative = Path(os.fsdecode(token))
        name = relative.as_posix()
        if (
            name.startswith("workers/")
            or name.startswith("crates/clew/")
            or name.startswith("gradle/")
            or name in {"Cargo.toml", "Cargo.lock", "settings.gradle.kts", "build.gradle.kts", "gradle.properties", "rust-toolchain.toml"}
            or name == "scripts/sthread_preflight.py"
        ):
            selected.append(relative)
    return sorted(selected, key=lambda path: path.as_posix())


def toolchain_fingerprint(workspace: Path) -> str:
    digest = hashlib.sha256()
    for relative in tracked_files(workspace):
        path = workspace / relative
        if path.is_symlink() or not path.is_file():
            raise PreflightFailure("TOOLCHAIN_FINGERPRINT", f"tracked toolchain input is not a regular file: {relative}")
        digest.update(relative.as_posix().encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return "sha256:" + digest.hexdigest()


def require_clean_tracked_worktree(workspace: Path, allow_dirty: bool) -> bool:
    completed = subprocess.run(
        ["git", "status", "--porcelain=v1", "--untracked-files=no"],
        cwd=workspace,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise PreflightFailure("GIT_STATE", "git status failed", detail=failure_summary(completed))
    clean = not completed.stdout.strip()
    if not clean and not allow_dirty:
        raise PreflightFailure("GIT_STATE", "tracked worktree is dirty; SThread requires a clean base")
    return clean


def clone_tree_contents(source: Path, target: Path, remaining_seconds: float) -> int:
    target.mkdir(parents=True, exist_ok=True)
    if sys.platform == "darwin":
        argv = ["cp", "-cR", f"{source}/.", str(target)]
    else:
        argv = ["cp", "-a", "--reflink=auto", f"{source}/.", str(target)]
    completed, elapsed = run_capture(argv, cwd=target.parent, timeout_seconds=remaining_seconds, stage="CACHE_HYDRATION")
    if completed.returncode != 0:
        raise PreflightFailure("CACHE_HYDRATION", "copy-on-write cache hydration failed", detail=failure_summary(completed))
    return elapsed


def cache_marker_matches(target_root: Path, fingerprint: str, required: Sequence[str]) -> bool:
    marker = target_root / CACHE_MARKER_NAME
    if marker.is_symlink() or not marker.is_file():
        return False
    try:
        value = json.loads(marker.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return False
    return (
        value == {
            "schema": CACHE_MARKER_SCHEMA,
            "dependencyFingerprint": fingerprint,
            "members": list(required),
        }
        and all((target_root / member).is_dir() and not (target_root / member).is_symlink() for member in required)
    )


def write_cache_marker(target_root: Path, fingerprint: str, members: Sequence[str]) -> None:
    atomic_json(
        target_root / CACHE_MARKER_NAME,
        {
            "schema": CACHE_MARKER_SCHEMA,
            "dependencyFingerprint": fingerprint,
            "members": list(members),
        },
    )


def gradle_configuration_task(compilation: str) -> str:
    if not compilation.startswith(":") or not compilation.endswith("/main"):
        raise PreflightFailure(
            "ARGUMENTS",
            "Codeclew Gradle compilation must be canonical and end in /main",
        )
    project_path = compilation[: -len("/main")]
    if project_path == ":":
        return "properties"
    if not project_path or "//" in project_path or project_path.endswith(":"):
        raise PreflightFailure("ARGUMENTS", "Codeclew Gradle compilation is malformed")
    return f"{project_path}:properties"


def hydrate_gradle_cache(
    source_root: Path,
    target_root: Path,
    deadline: float,
    fingerprint: str,
) -> tuple[list[str], int, bool]:
    required = [member for member in GRADLE_CACHE_MEMBERS if (source_root / member).is_dir()]
    if "caches" not in required or "wrapper" not in required:
        raise PreflightFailure("CACHE_HYDRATION", "Gradle seed must contain caches/ and wrapper/")
    if cache_marker_matches(target_root, fingerprint, required):
        return required, 0, True
    copied: list[str] = []
    total_millis = 0
    target_root.mkdir(parents=True, exist_ok=True)
    for member in GRADLE_CACHE_MEMBERS:
        source = source_root / member
        if not source.exists():
            continue
        require_real_directory(source, "CACHE_HYDRATION")
        total_millis += clone_tree_contents(source, target_root / member, deadline - time.monotonic())
        copied.append(member)
    write_cache_marker(target_root, fingerprint, copied)
    return copied, total_millis, False


def hydrate_maven_repository(source: Path, target: Path, deadline: float, fingerprint: str) -> tuple[int, bool]:
    require_real_directory(source, "CACHE_HYDRATION")
    marker_root = target.parent
    if cache_marker_matches(marker_root, fingerprint, ("maven-repository",)):
        return 0, True
    elapsed = clone_tree_contents(source, target, deadline - time.monotonic())
    write_cache_marker(marker_root, fingerprint, ("maven-repository",))
    return elapsed, False


def probe(
    *,
    kind: str,
    argv: Sequence[str],
    cwd: Path,
    deadline: float,
    environment: dict[str, str] | None = None,
) -> dict[str, Any]:
    completed, elapsed = run_capture(
        argv,
        cwd=cwd,
        timeout_seconds=deadline - time.monotonic(),
        stage=kind,
        environment=environment,
    )
    row = {
        "kind": kind,
        "argv": list(argv),
        "durationMillis": elapsed,
        "exitCode": completed.returncode,
        "stdoutSha256": sha256_bytes(completed.stdout),
        "stderrSha256": sha256_bytes(completed.stderr),
    }
    if completed.returncode != 0:
        raise PreflightFailure(kind, f"{kind} probe failed", detail={**row, **failure_summary(completed)})
    return row


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, default=ROOT)
    parser.add_argument("--compilation", default=":workers:kotlin21/main")
    parser.add_argument("--gradle-cache-seed", type=Path, default=Path.home() / ".gradle")
    parser.add_argument("--maven-repository-seed", type=Path, default=Path.home() / ".m2" / "repository")
    parser.add_argument("--clew", type=Path)
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--budget-seconds", type=float, default=60.0)
    parser.add_argument("--allow-dirty", action="store_true", help="development-only; READY will record trackedClean=false")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--skip-smoke", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    return parser.parse_args()


def self_test() -> None:
    event = b'{"event":"request_completed","success":false}\n{\n  "error":{"code":"X","message":"boom"}\n}\n'
    completed = subprocess.CompletedProcess(["clew"], 7, event, b"")
    summary = failure_summary(completed)
    assert summary["code"] == "X" and summary["message"] == "boom"
    assert json_values(event)[0]["event"] == "request_completed"
    assert json_values(event)[1]["error"]["code"] == "X"
    assert gradle_configuration_task(":/main") == "properties"
    assert gradle_configuration_task(":workers:kotlin21/main") == ":workers:kotlin21:properties"
    try:
        gradle_configuration_task(":workers:kotlin21/test")
        raise AssertionError("non-main compilation accepted")
    except PreflightFailure:
        pass
    with tempfile.TemporaryDirectory(prefix="sthread-preflight-self-test-") as raw:
        root = Path(raw)
        receipt = root / "receipt.json"
        atomic_json(receipt, {"schema": SCHEMA, "status": "READY"})
        assert receipt.read_bytes() == b'{"schema":"codeclew.sthread-preflight/0.1","status":"READY"}\n'
        cache = root / "cache"
        (cache / "caches").mkdir(parents=True)
        (cache / "wrapper").mkdir()
        write_cache_marker(cache, "sha256:" + "a" * 64, ("caches", "wrapper"))
        assert cache_marker_matches(cache, "sha256:" + "a" * 64, ("caches", "wrapper"))
        assert not cache_marker_matches(cache, "sha256:" + "b" * 64, ("caches", "wrapper"))
    print(json.dumps({"schema": SCHEMA, "status": "SELF_TEST_PASSED"}, separators=(",", ":")))


def execute(args: argparse.Namespace) -> dict[str, Any]:
    started = time.monotonic()
    if args.budget_seconds <= 0:
        raise PreflightFailure("ARGUMENTS", "budget-seconds must be positive")
    deadline = started + args.budget_seconds
    workspace = require_real_directory(args.workspace, "WORKSPACE")
    git_root = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=workspace,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if git_root.returncode != 0 or Path(git_root.stdout.decode().strip()).resolve() != workspace:
        raise PreflightFailure("WORKSPACE", "workspace must be the exact Git root")
    tracked_clean = require_clean_tracked_worktree(workspace, args.allow_dirty)
    fingerprint = toolchain_fingerprint(workspace)
    head = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=workspace, stdout=subprocess.PIPE, check=True
    ).stdout.decode().strip()

    gradle_seed = require_real_directory(args.gradle_cache_seed, "CACHE_HYDRATION")
    copied, gradle_millis, workspace_cache_hit = hydrate_gradle_cache(
        gradle_seed, workspace / ".gradle", deadline, fingerprint
    )
    fixture21 = workspace / "fixtures" / "kotlin-2-1"
    fixture24 = workspace / "fixtures" / "kotlin-basic"
    fixture_maven = workspace / "fixtures" / "kotlin-maven"
    if not args.skip_smoke:
        require_real_directory(fixture21, "SMOKE_FIXTURE")
        require_real_directory(fixture24, "SMOKE_FIXTURE")
        require_real_directory(fixture_maven, "SMOKE_FIXTURE")
        _, fixture21_gradle_millis, fixture21_cache_hit = hydrate_gradle_cache(
            gradle_seed, fixture21 / ".gradle", deadline, fingerprint
        )
        _, fixture24_gradle_millis, fixture24_cache_hit = hydrate_gradle_cache(
            gradle_seed, fixture24 / ".gradle", deadline, fingerprint
        )
        gradle_millis += fixture21_gradle_millis + fixture24_gradle_millis
        maven_millis, maven_cache_hit = hydrate_maven_repository(
            args.maven_repository_seed,
            fixture_maven / ".semantic-thread" / "maven-repository",
            deadline,
            fingerprint,
        )
    else:
        maven_millis = 0
        fixture21_cache_hit = False
        fixture24_cache_hit = False
        maven_cache_hit = False

    clew = (args.clew or workspace / "target" / "release" / "clew").resolve()
    build_millis = 0
    if not args.skip_build:
        build, build_millis = run_capture(
            ["cargo", "build", "--offline", "--release", "-p", "clew", "--bin", "clew"],
            cwd=workspace,
            timeout_seconds=deadline - time.monotonic(),
            stage="CLEW_BUILD",
        )
        if build.returncode != 0:
            raise PreflightFailure("CLEW_BUILD", "release clew build failed", detail=failure_summary(build))
    if clew.is_symlink() or not clew.is_file() or not os.access(clew, os.X_OK):
        raise PreflightFailure("CLEW_BINARY", f"clew binary is absent or not executable: {clew}")

    gradle_environment = dict(os.environ)
    gradle_environment["GRADLE_USER_HOME"] = str(workspace / ".gradle")
    configuration_task = gradle_configuration_task(args.compilation)
    probes = [
        probe(
            kind="GRADLE_CONFIGURATION",
            argv=[
                str(workspace / "gradlew"),
                "--offline",
                "--no-daemon",
                "--quiet",
                configuration_task,
            ],
            cwd=workspace,
            deadline=deadline,
            environment=gradle_environment,
        )
    ]
    if not args.skip_smoke:
        probes.append(
            probe(
                kind="OPEN_PROJECT_KOTLIN_2_4",
                argv=[str(clew), "project", "inspect", "--repo", str(fixture24), "--compilation", ":/main"],
                cwd=workspace,
                deadline=deadline,
            )
        )
        for kind, relative_repo, compilation, _build_system in DEFAULT_SMOKES:
            probes.append(
                probe(
                    kind=kind,
                    argv=[str(clew), "index", "--repo", str(workspace / relative_repo), "--compilation", compilation],
                    cwd=workspace,
                    deadline=deadline,
                )
            )

    elapsed = monotonic_millis(started)
    return {
        "schema": SCHEMA,
        "status": "READY",
        "stage": "COMPLETE",
        "workspaceRevision": head,
        "trackedClean": tracked_clean,
        "toolchainFingerprint": fingerprint,
        "budgetMillis": round(args.budget_seconds * 1000),
        "elapsedMillis": elapsed,
        "cache": {
            "gradleMembers": copied,
            "gradleHydrationMillis": gradle_millis,
            "mavenHydrationMillis": maven_millis,
            "workspaceHit": workspace_cache_hit,
            "fixture21Hit": fixture21_cache_hit,
            "fixture24Hit": fixture24_cache_hit,
            "mavenHit": maven_cache_hit,
        },
        "binary": {"path": str(clew), "sha256": sha256_file(clew), "buildMillis": build_millis},
        "probes": probes,
    }


def main() -> int:
    args = parse_args()
    if args.self_test:
        self_test()
        return 0
    started = time.monotonic()
    try:
        receipt = execute(args)
    except PreflightFailure as error:
        receipt = {
            "schema": SCHEMA,
            "status": "FAILED",
            "stage": error.stage,
            "elapsedMillis": monotonic_millis(started),
            "failure": {"message": str(error), **error.detail},
        }
        exit_code = 1
    except Exception as error:  # Preserve an unexpected preflight defect as a typed stop.
        receipt = {
            "schema": SCHEMA,
            "status": "FAILED",
            "stage": "PREFLIGHT_INTERNAL",
            "elapsedMillis": monotonic_millis(started),
            "failure": {
                "message": f"{type(error).__name__}: {error}",
                "messageSha256": sha256_bytes(f"{type(error).__name__}: {error}".encode()),
            },
        }
        exit_code = 1
    if args.receipt is not None:
        atomic_json(args.receipt, receipt)
    print(canonical(receipt).decode(), end="")
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
