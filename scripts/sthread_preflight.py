#!/usr/bin/env python3
"""Prepare and prove the local SThread runtime before an agent-context run.

The command is intentionally quiet: dependency caches are copied by the OS and
subprocess output is retained only as hashes/bounded failure summaries.  A
successful receipt means that the trusted worker can open the requested Gradle
model and that the Kotlin 2.1 and 2.3 compiler-semantic smoke fixtures pass.
"""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import stat
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
TRUSTED_WORKER_DISTRIBUTIONS = (
    ("workers/manifests/kotlin21.json", "workers/kotlin21/build/install/kotlin21"),
    ("workers/manifests/kotlin23.json", "workers/kotlin23/build/install/kotlin23"),
    ("workers/manifests/kotlin24.json", "workers/kotlin/build/install/kotlin"),
)
COMPILER_INDEX_SUCCESS_STATUSES = (
    "COLD_FULL",
    "INCREMENTAL",
    "RECOVERED_FULL",
    "UNCHANGED_HIT",
)
PROJECT_MODEL_CACHE_STATUSES = (
    "MEMORY_HIT",
    "PERSISTENT_HIT",
    "EXTRACTED_PUBLISHED",
    "EXTRACTED_NOT_PUBLISHED",
)
PROJECT_MODEL_PUBLISH_OUTCOMES = (
    "NOT_ATTEMPTED",
    "PUBLISHED",
    "INVALID_MODEL",
    "ROOT_UNAVAILABLE",
    "WRITE_FAILED",
)
PROJECT_MODEL_INVALID_REASONS = (
    "NOT_APPLICABLE",
    "MISSING_SEMANTIC_INPUT_MANIFEST_HASH",
    "INVALID_SEMANTIC_INPUT_MANIFEST_HASH",
    "SEMANTIC_INPUT_MANIFEST_HASH_MISMATCH",
    "MISSING_SEMANTIC_INPUT_MANIFEST",
    "MODEL_INPUTS_MANIFEST_MISMATCH",
    "JDK_FINGERPRINT_MANIFEST_MISMATCH",
    "MODEL_INPUTS_INVALID",
    "RESOURCE_IDENTITIES_INVALID",
    "JDK_HOME_INVALID",
    "JDK_HOME_MISMATCH",
    "JDK_FINGERPRINT_MISSING",
    "JDK_FINGERPRINT_INVALID",
)
TRUSTED_WORKER_ENVIRONMENT_REMOVALS = (
    "GRADLE_USER_HOME",
    "JAVA_HOME",
    "JAVA_TOOL_OPTIONS",
    "JDK_JAVA_OPTIONS",
    "_JAVA_OPTIONS",
    "CODECLEW_K1_BUILD_STATE_ROOT",
    "CODECLEW_K2_INDEX_ROOT",
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


def atomic_trusted_clew_launcher(path: Path, clew: Path) -> None:
    path = path.resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = (
        "#!/usr/bin/env python3\n"
        "import os\n"
        "import sys\n"
        f"for name in {TRUSTED_WORKER_ENVIRONMENT_REMOVALS!r}:\n"
        "    os.environ.pop(name, None)\n"
        f"clew = {str(clew)!r}\n"
        "os.execv(clew, [clew, *sys.argv[1:]])\n"
    ).encode()
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        os.fchmod(descriptor, 0o700)
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
    timeout = None if math.isinf(timeout_seconds) else timeout_seconds
    started = time.monotonic()
    try:
        completed = subprocess.run(
            list(argv),
            cwd=cwd,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
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


def prepare_compiler_index_root(path: Path | None, workspace: Path) -> Path:
    if path is None:
        temporary_root = Path(tempfile.gettempdir()).resolve()
        workspace_identity = hashlib.sha256(str(workspace).encode()).hexdigest()
        path = temporary_root / "codeclew-sthread-compiler-index" / workspace_identity
    if not path.is_absolute():
        raise PreflightFailure("COMPILER_INDEX_ROOT", "compiler index root must be absolute")
    lexical = Path(os.path.abspath(path))
    existed = lexical.exists()
    if lexical.is_symlink():
        raise PreflightFailure("COMPILER_INDEX_ROOT", "compiler index root must not be a symlink")
    lexical.mkdir(mode=0o700, parents=True, exist_ok=True)
    if not existed:
        lexical.chmod(0o700)
    canonical = lexical.resolve(strict=True)
    if canonical != lexical:
        raise PreflightFailure(
            "COMPILER_INDEX_ROOT",
            "compiler index root must be canonical and have no symlinked ancestor",
        )
    metadata = canonical.stat()
    if not stat.S_ISDIR(metadata.st_mode) or stat.S_IMODE(metadata.st_mode) & 0o077:
        raise PreflightFailure("COMPILER_INDEX_ROOT", "compiler index root must be a private directory")
    if canonical == workspace or canonical.is_relative_to(workspace) or workspace.is_relative_to(canonical):
        raise PreflightFailure("COMPILER_INDEX_ROOT", "compiler index root must be external to workspace")
    return canonical


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


def git_stdout(workspace: Path, *arguments: str, stage: str) -> str:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=workspace,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise PreflightFailure(stage, f"git {' '.join(arguments)} failed", detail=failure_summary(completed))
    return completed.stdout.decode("utf-8", errors="strict").strip()


def require_trusted_worker_seed_revision(seed: Path, expected_revision: str) -> Path:
    seed = require_real_directory(seed, "TRUSTED_WORKER_HYDRATION")
    root = git_stdout(
        seed, "rev-parse", "--show-toplevel", stage="TRUSTED_WORKER_HYDRATION"
    )
    if Path(root).resolve() != seed:
        raise PreflightFailure(
            "TRUSTED_WORKER_HYDRATION",
            "trusted worker seed must be the exact Git root that produced the distribution",
        )
    revision = git_stdout(
        seed, "rev-parse", "HEAD", stage="TRUSTED_WORKER_HYDRATION"
    )
    if revision != expected_revision:
        raise PreflightFailure(
            "TRUSTED_WORKER_HYDRATION",
            "trusted worker seed revision differs from the requested snapshot revision",
            detail={
                "expectedRevision": expected_revision,
                "actualRevision": revision,
            },
        )
    require_clean_tracked_worktree(seed, False)
    return seed


def prepare_sthread_snapshot(args: argparse.Namespace) -> dict[str, Any]:
    started = time.monotonic()
    requested_budget = 60.0 if args.budget_seconds is None else args.budget_seconds
    if requested_budget <= 0:
        raise PreflightFailure("ARGUMENTS", "budget-seconds must be positive")
    preparation_budget = min(60.0, requested_budget)
    deadline = started + preparation_budget
    workspace = require_real_directory(args.workspace, "WORKSPACE")
    require_clean_tracked_worktree(workspace, False)
    clew = (getattr(args, "clew", None) or workspace / "target" / "release" / "clew").resolve()
    if clew.is_symlink() or not clew.is_file() or not os.access(clew, os.X_OK):
        raise PreflightFailure(
            "STHREAD_SNAPSHOT_PREPARATION",
            "trusted clew executable is missing, non-regular, symlinked, or non-executable",
        )
    source_workspace_head = git_stdout(workspace, "rev-parse", "HEAD", stage="GIT_STATE")
    requested_revision = getattr(args, "snapshot_revision", None)
    expected_head = (
        git_stdout(
            workspace,
            "rev-parse",
            "--verify",
            f"{requested_revision}^{{commit}}",
            stage="GIT_STATE",
        )
        if requested_revision
        else source_workspace_head
    )
    trusted_worker_seed = require_trusted_worker_seed_revision(
        args.trusted_worker_seed, expected_head
    )
    parent = require_real_directory(args.snapshot_parent, "STHREAD_SNAPSHOT_PREPARATION")
    if parent == workspace or parent.is_relative_to(workspace) or workspace.is_relative_to(parent):
        raise PreflightFailure(
            "STHREAD_SNAPSHOT_PREPARATION",
            "snapshot parent must be external to the source workspace",
        )
    run_directory = Path(
        tempfile.mkdtemp(prefix="codeclew-sthread-snapshot-", dir=parent)
    ).resolve(strict=True)
    repository = run_directory / "source"
    branch = "codex/sthread-preflight-" + hashlib.sha256(str(run_directory).encode()).hexdigest()[:16]
    try:
        clone, _clone_millis = run_capture(
            ["git", "clone", "--quiet", "--no-hardlinks", str(workspace), str(repository)],
            cwd=run_directory,
            timeout_seconds=deadline - time.monotonic(),
            stage="STHREAD_SNAPSHOT_PREPARATION",
        )
        if clone.returncode != 0:
            raise PreflightFailure(
                "STHREAD_SNAPSHOT_PREPARATION",
                "isolated Git clone failed",
                detail=failure_summary(clone),
            )
        switched, _switch_millis = run_capture(
            ["git", "switch", "--quiet", "-c", branch, expected_head],
            cwd=repository,
            timeout_seconds=deadline - time.monotonic(),
            stage="STHREAD_SNAPSHOT_PREPARATION",
        )
        if switched.returncode != 0:
            raise PreflightFailure(
                "STHREAD_SNAPSHOT_PREPARATION",
                "isolated target branch creation failed",
                detail=failure_summary(switched),
            )
        gradle_seed = require_real_directory(args.gradle_cache_seed, "CACHE_HYDRATION")
        gradle_millis = clone_tree_contents(
            gradle_seed,
            repository / ".gradle",
            deadline - time.monotonic(),
        )
        worker_millis, worker_hit = hydrate_trusted_workers(
            trusted_worker_seed,
            repository,
            deadline,
        )
        gradle_wrapper = repository / "gradlew"
        if gradle_wrapper.is_symlink() or not gradle_wrapper.is_file() or not os.access(gradle_wrapper, os.X_OK):
            raise PreflightFailure(
                "STHREAD_SNAPSHOT_GRADLE_ROUTE",
                "prepared snapshot has no trusted executable Gradle wrapper",
            )
        route_environment = dict(os.environ)
        for name in TRUSTED_WORKER_ENVIRONMENT_REMOVALS:
            route_environment.pop(name, None)
        route_environment["GRADLE_USER_HOME"] = str(repository / ".gradle")
        configuration_arguments = gradle_configuration_arguments(args.compilation)
        route_probe, route_millis = run_capture(
            [
                str(gradle_wrapper),
                "--offline",
                "--no-daemon",
                "--quiet",
                *configuration_arguments,
            ],
            cwd=repository,
            timeout_seconds=deadline - time.monotonic(),
            stage="STHREAD_SNAPSHOT_GRADLE_ROUTE",
            environment=route_environment,
        )
        if route_probe.returncode != 0:
            raise PreflightFailure(
                "STHREAD_SNAPSHOT_GRADLE_ROUTE",
                "prepared snapshot cannot resolve the requested Gradle compilation",
                detail={
                    **failure_summary(route_probe),
                    "compilation": args.compilation,
                    "configurationArguments": configuration_arguments,
                },
            )
        trusted_launcher = run_directory / "trusted-clew"
        atomic_trusted_clew_launcher(trusted_launcher, clew)
        actual_head = git_stdout(repository, "rev-parse", "HEAD", stage="GIT_STATE")
        active_branch = git_stdout(
            repository, "symbolic-ref", "--short", "HEAD", stage="GIT_STATE"
        )
        status = git_stdout(
            repository, "status", "--porcelain=v1", stage="GIT_STATE"
        )
        if actual_head != expected_head or active_branch != branch or status:
            raise PreflightFailure(
                "STHREAD_SNAPSHOT_PREPARATION",
                "prepared snapshot is not the exact clean active target branch",
                detail={
                    "expectedHead": expected_head,
                    "actualHead": actual_head,
                    "expectedBranch": branch,
                    "actualBranch": active_branch,
                    "statusSha256": sha256_bytes(status.encode()),
                },
            )
        elapsed = monotonic_millis(started)
        if elapsed > round(preparation_budget * 1000):
            raise PreflightFailure(
                "STHREAD_SNAPSHOT_PREPARATION",
                "snapshot preparation exceeded its one-minute budget",
                detail={"elapsedMillis": elapsed, "budgetMillis": round(preparation_budget * 1000)},
            )
        return {
            "schema": SCHEMA,
            "status": "READY",
            "stage": "STHREAD_SNAPSHOT_READY",
            "workspaceRevision": expected_head,
            "sourceWorkspaceRevision": source_workspace_head,
            "trackedClean": True,
            "runDirectory": str(run_directory),
            "repository": str(repository),
            "targetRef": branch,
            "gradleUserHome": str(repository / ".gradle"),
            "trustedClewLauncher": str(trusted_launcher),
            "agentContextEnvironmentRemovals": list(TRUSTED_WORKER_ENVIRONMENT_REMOVALS),
            "elapsedMillis": elapsed,
            "budgetMillis": round(preparation_budget * 1000),
            "cache": {
                "gradleHydrationMillis": gradle_millis,
                "gradleRouteProbeMillis": route_millis,
                "trustedWorkerHydrationMillis": worker_millis,
                "trustedWorkerHit": worker_hit,
            },
        }
    except PreflightFailure as error:
        error.detail.setdefault("runDirectory", str(run_directory))
        error.detail.setdefault("repository", str(repository))
        raise


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


def gradle_configuration_arguments(compilation: str) -> list[str]:
    task = gradle_configuration_task(compilation)
    if compilation == ":/main":
        return [task]
    project_path = compilation[: -len("/main")].removeprefix(":").replace(":", "/")
    if not project_path or project_path.startswith("/") or any(
        component in ("", ".", "..") for component in project_path.split("/")
    ):
        raise PreflightFailure("ARGUMENTS", "Codeclew Gradle project directory is malformed")
    return ["-p", project_path, "properties"]


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


def hydrate_cargo_target(source: Path, target: Path, deadline: float, fingerprint: str) -> tuple[int, bool]:
    source = require_real_directory(source, "CACHE_HYDRATION")
    if target.exists() and require_real_directory(target, "CACHE_HYDRATION") == source:
        return 0, True
    required = ("release",)
    if not (source / "release").is_dir() or (source / "release").is_symlink():
        raise PreflightFailure("CACHE_HYDRATION", "Cargo target seed must contain a regular release/ directory")
    if cache_marker_matches(target, fingerprint, required):
        return 0, True
    elapsed = clone_tree_contents(source / "release", target / "release", deadline - time.monotonic())
    write_cache_marker(target, fingerprint, required)
    return elapsed, False


def hydrate_trusted_workers(seed: Path, workspace: Path, deadline: float) -> tuple[int, bool]:
    seed = require_real_directory(seed, "TRUSTED_WORKER_HYDRATION")
    if seed == workspace:
        for _manifest_name, distribution_name in TRUSTED_WORKER_DISTRIBUTIONS:
            require_real_directory(workspace / distribution_name, "TRUSTED_WORKER_HYDRATION")
        return 0, True
    for manifest_name, _distribution_name in TRUSTED_WORKER_DISTRIBUTIONS:
        source_manifest = seed / manifest_name
        target_manifest = workspace / manifest_name
        if (
            source_manifest.is_symlink()
            or target_manifest.is_symlink()
            or not source_manifest.is_file()
            or not target_manifest.is_file()
            or source_manifest.read_bytes() != target_manifest.read_bytes()
        ):
            raise PreflightFailure(
                "TRUSTED_WORKER_HYDRATION",
                f"trusted worker manifest differs from seed: {manifest_name}",
            )
    targets = [workspace / distribution_name for _manifest_name, distribution_name in TRUSTED_WORKER_DISTRIBUTIONS]
    if all(target.is_dir() and not target.is_symlink() for target in targets):
        return 0, True
    elapsed = 0
    for _manifest_name, distribution_name in TRUSTED_WORKER_DISTRIBUTIONS:
        source_distribution = require_real_directory(seed / distribution_name, "TRUSTED_WORKER_HYDRATION")
        target_distribution = workspace / distribution_name
        elapsed += clone_tree_contents(source_distribution, target_distribution, deadline - time.monotonic())
    return elapsed, False


def missing_stdout_markers(stdout: bytes, required: Sequence[bytes]) -> list[str]:
    return [marker.decode("utf-8", errors="replace") for marker in required if marker not in stdout]


def compiler_index_profile(stdout: bytes) -> dict[str, Any]:
    result = next(
        (
            value
            for value in reversed(json_values(stdout))
            if isinstance(value, dict) and value.get("schema") == "semantic-index-result/0.1"
        ),
        None,
    )
    profile = result.get("compilerIndex") if isinstance(result, dict) else None
    if not isinstance(profile, dict):
        shape = "missing-result"
        if isinstance(result, dict):
            shape = "missing-key" if "compilerIndex" not in result else type(profile).__name__
        raise PreflightFailure(
            "KOTLIN_2_1_COMPILER_INDEX",
            "Kotlin 2.1 index result lacks typed compiler-index telemetry",
            detail={"semanticIndexResult": isinstance(result, dict), "compilerIndexShape": shape},
        )
    required = {
        "backend": str,
        "status": str,
        "valid": bool,
        "totalMicros": int,
        "compilerMicros": int,
        "firExtractionMicros": int,
        "totalFiles": int,
        "compiledFiles": int,
        "reusedFiles": int,
        "recovered": bool,
        "fallbackUsed": bool,
    }
    if any(not isinstance(profile.get(key), expected) for key, expected in required.items()):
        raise PreflightFailure(
            "KOTLIN_2_1_COMPILER_INDEX",
            "Kotlin 2.1 compiler-index telemetry is incomplete or malformed",
        )
    if profile["backend"] != "BTA_PERSISTENT":
        raise PreflightFailure("KOTLIN_2_1_COMPILER_INDEX", "unexpected compiler-index backend")
    if profile["status"] not in COMPILER_INDEX_SUCCESS_STATUSES:
        raise PreflightFailure("KOTLIN_2_1_COMPILER_INDEX", "compiler-index did not complete persistently")
    if not profile["valid"] or profile["fallbackUsed"]:
        raise PreflightFailure("KOTLIN_2_1_COMPILER_INDEX", "compiler-index fell back or returned invalid facts")
    digest = profile.get("graphDigest")
    if not isinstance(digest, str) or len(digest) != 64 or any(character not in "0123456789abcdef" for character in digest):
        raise PreflightFailure("KOTLIN_2_1_COMPILER_INDEX", "compiler-index graph digest is malformed")
    if min(profile[key] for key in ("totalMicros", "compilerMicros", "firExtractionMicros", "totalFiles", "compiledFiles", "reusedFiles")) < 0:
        raise PreflightFailure("KOTLIN_2_1_COMPILER_INDEX", "compiler-index telemetry contains a negative value")
    if profile["status"] == "UNCHANGED_HIT" and (
        profile["compiledFiles"] != 0 or profile["reusedFiles"] != profile["totalFiles"]
    ):
        raise PreflightFailure("KOTLIN_2_1_COMPILER_INDEX", "unchanged compiler-index hit has inconsistent counts")
    return profile


def project_model_cache_profile(stdout: bytes) -> dict[str, Any]:
    result = next(
        (
            value
            for value in reversed(json_values(stdout))
            if isinstance(value, dict) and value.get("schema") == "semantic-index-result/0.1"
        ),
        None,
    )
    profile = result.get("projectModelCache") if isinstance(result, dict) else None
    if not isinstance(profile, dict):
        shape = "missing-result"
        if isinstance(result, dict):
            shape = "missing-key" if "projectModelCache" not in result else type(profile).__name__
        raise PreflightFailure(
            "KOTLIN_2_1_PROJECT_MODEL_CACHE",
            "Kotlin 2.1 index result lacks typed project-model cache telemetry",
            detail={"semanticIndexResult": isinstance(result, dict), "projectModelCacheShape": shape},
        )
    required = {
        "status": str,
        "publishOutcome": str,
        "publishInvalidReason": str,
        "totalMicros": int,
        "keyMicros": int,
        "loadMicros": int,
        "extractionMicros": int,
        "publishMicros": int,
        "persistentConfigured": bool,
        "published": bool,
    }
    if any(type(profile.get(key)) is not expected for key, expected in required.items()):
        raise PreflightFailure(
            "KOTLIN_2_1_PROJECT_MODEL_CACHE",
            "project-model cache telemetry is incomplete or malformed",
        )
    if profile["status"] not in PROJECT_MODEL_CACHE_STATUSES:
        raise PreflightFailure(
            "KOTLIN_2_1_PROJECT_MODEL_CACHE", "project-model cache status is unknown"
        )
    if profile["publishOutcome"] not in PROJECT_MODEL_PUBLISH_OUTCOMES:
        raise PreflightFailure(
            "KOTLIN_2_1_PROJECT_MODEL_CACHE", "project-model publish outcome is unknown"
        )
    if profile["publishInvalidReason"] not in PROJECT_MODEL_INVALID_REASONS:
        raise PreflightFailure(
            "KOTLIN_2_1_PROJECT_MODEL_CACHE", "project-model invalid reason is unknown"
        )
    timings = tuple(
        profile[key]
        for key in ("totalMicros", "keyMicros", "loadMicros", "extractionMicros", "publishMicros")
    )
    if min(timings) < 0 or sum(timings[1:]) > timings[0]:
        raise PreflightFailure(
            "KOTLIN_2_1_PROJECT_MODEL_CACHE", "project-model cache timings are inconsistent"
        )
    status = profile["status"]
    outcome = profile["publishOutcome"]
    invalid_reason = profile["publishInvalidReason"]
    consistent = {
        "MEMORY_HIT": outcome == "NOT_ATTEMPTED"
        and invalid_reason == "NOT_APPLICABLE"
        and profile["loadMicros"] == 0
        and profile["extractionMicros"] == 0
        and profile["publishMicros"] == 0
        and not profile["published"],
        "PERSISTENT_HIT": outcome == "NOT_ATTEMPTED"
        and invalid_reason == "NOT_APPLICABLE"
        and profile["persistentConfigured"]
        and profile["extractionMicros"] == 0
        and profile["publishMicros"] == 0
        and not profile["published"],
        "EXTRACTED_PUBLISHED": outcome == "PUBLISHED"
        and invalid_reason == "NOT_APPLICABLE"
        and profile["persistentConfigured"]
        and profile["published"],
        "EXTRACTED_NOT_PUBLISHED": outcome
        in {"INVALID_MODEL", "ROOT_UNAVAILABLE", "WRITE_FAILED"}
        and ((outcome == "INVALID_MODEL") == (invalid_reason != "NOT_APPLICABLE"))
        and not profile["published"],
    }[status]
    if not consistent:
        raise PreflightFailure(
            "KOTLIN_2_1_PROJECT_MODEL_CACHE",
            "project-model cache status disagrees with its timing/publication fields",
        )
    return profile


def semantic_index_signature(result: dict[str, Any]) -> dict[str, Any]:
    required = ("declarationRelationHash", "declarationDescriptorHash")
    if any(not isinstance(result.get(key), str) or not result[key] for key in required):
        raise PreflightFailure(
            "KOTLIN_2_1_COMPILER_INDEX",
            "semantic index result lacks normalized graph identities",
        )
    if not isinstance(result.get("files"), int) or result["files"] < 0:
        raise PreflightFailure(
            "KOTLIN_2_1_COMPILER_INDEX",
            "semantic index result has an invalid file count",
        )
    return {key: result[key] for key in (*required, "files")}


def receipt_argv(argv: Sequence[str]) -> list[str]:
    rendered = list(argv)
    for index, token in enumerate(rendered[:-1]):
        if token == "--compiler-index-root":
            rendered[index + 1] = "<private-compiler-index-root>"
    return rendered


def probe(
    *,
    kind: str,
    argv: Sequence[str],
    cwd: Path,
    deadline: float,
    environment: dict[str, str] | None = None,
    required_stdout_markers: Sequence[bytes] = (),
    expected_compiler_index_status: str | None = None,
    require_compiler_index: bool = False,
    expected_project_model_cache_status: str | None = None,
    require_project_model_cache: bool = False,
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
        "argv": receipt_argv(argv),
        "durationMillis": elapsed,
        "exitCode": completed.returncode,
        "stdoutSha256": sha256_bytes(completed.stdout),
        "stderrSha256": sha256_bytes(completed.stderr),
    }
    if completed.returncode != 0:
        raise PreflightFailure(kind, f"{kind} probe failed", detail={**row, **failure_summary(completed)})
    missing_markers = missing_stdout_markers(completed.stdout, required_stdout_markers)
    if missing_markers:
        raise PreflightFailure(
            kind,
            f"{kind} probe lacks required capability markers",
            detail={**row, "missingMarkers": missing_markers},
        )
    if require_compiler_index or expected_compiler_index_status is not None:
        profile = compiler_index_profile(completed.stdout)
        if expected_compiler_index_status is not None and profile["status"] != expected_compiler_index_status:
            raise PreflightFailure(
                "KOTLIN_2_1_COMPILER_INDEX",
                f"expected {expected_compiler_index_status}, got {profile['status']}",
                detail={**row, "compilerIndex": profile},
            )
        row["compilerIndex"] = profile
        result = next(
            value
            for value in reversed(json_values(completed.stdout))
            if isinstance(value, dict) and value.get("schema") == "semantic-index-result/0.1"
        )
        timing = result.get("timing")
        worker_profile = result.get("workerProfile")
        row["semanticIndex"] = semantic_index_signature(result)
        timing_keys = (
            "openProjectMicros",
            "indexFilesMicros",
            "inspectReceiptMicros",
            "repositoryPublicationMicros",
            "totalMicros",
        )
        if not isinstance(timing, dict) or any(
            not isinstance(timing.get(key), int) or timing[key] < 0 for key in timing_keys
        ):
            raise PreflightFailure("KOTLIN_2_1_COMPILER_INDEX", "index phase telemetry is malformed")
        if not isinstance(worker_profile, dict):
            raise PreflightFailure("KOTLIN_2_1_COMPILER_INDEX", "worker phase telemetry is missing")
        row["indexTiming"] = {key: timing[key] for key in timing_keys}
        if timing.get("openProjectIncludedInIndexFiles") is not True or timing["openProjectMicros"] != 0:
            raise PreflightFailure(
                "KOTLIN_2_1_COMPILER_INDEX",
                "verified IndexFiles must own the sole authoritative OpenProject phase",
            )
        row["indexTiming"]["openProjectIncludedInIndexFiles"] = True
        row["workerProfile"] = worker_profile
    if require_project_model_cache or expected_project_model_cache_status is not None:
        project_model_profile = project_model_cache_profile(completed.stdout)
        if (
            expected_project_model_cache_status is not None
            and project_model_profile["status"] != expected_project_model_cache_status
        ):
            raise PreflightFailure(
                "KOTLIN_2_1_PROJECT_MODEL_CACHE",
                f"expected {expected_project_model_cache_status}, got {project_model_profile['status']}",
                detail={**row, "projectModelCache": project_model_profile},
            )
        row["projectModelCache"] = project_model_profile
    return row


def probe_group(specifications: Sequence[dict[str, Any]], deadline: float) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    failures: list[PreflightFailure] = []
    with ThreadPoolExecutor(max_workers=len(specifications), thread_name_prefix="sthread-preflight") as executor:
        futures = {
            executor.submit(
                probe,
                kind=str(specification["kind"]),
                argv=list(specification["argv"]),
                cwd=Path(specification["cwd"]),
                deadline=deadline,
                environment=specification.get("environment"),
                required_stdout_markers=tuple(specification.get("required_stdout_markers", ())),
                expected_compiler_index_status=specification.get("expected_compiler_index_status"),
                require_compiler_index=bool(specification.get("require_compiler_index", False)),
                expected_project_model_cache_status=specification.get(
                    "expected_project_model_cache_status"
                ),
                require_project_model_cache=bool(
                    specification.get("require_project_model_cache", False)
                ),
            ): str(specification["kind"])
            for specification in specifications
        }
        for future in as_completed(futures):
            try:
                results.append(future.result())
            except PreflightFailure as error:
                failures.append(error)
    if failures:
        failures.sort(key=lambda error: error.stage)
        raise failures[0]
    return sorted(results, key=lambda row: str(row["kind"]))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, default=ROOT)
    parser.add_argument("--compilation", default=":workers:kotlin21/main")
    parser.add_argument("--gradle-cache-seed", type=Path, default=Path.home() / ".gradle")
    parser.add_argument("--maven-repository-seed", type=Path, default=Path.home() / ".m2" / "repository")
    parser.add_argument("--cargo-target-seed", type=Path, default=ROOT / "target")
    parser.add_argument("--trusted-worker-seed", type=Path, default=ROOT)
    parser.add_argument("--compiler-index-root", type=Path)
    parser.add_argument("--clew", type=Path)
    parser.add_argument("--receipt", type=Path)
    parser.add_argument(
        "--budget-seconds",
        type=float,
        help="optional operational timeout; cold preparation is unlimited by default",
    )
    parser.add_argument("--allow-dirty", action="store_true", help="development-only; READY will record trackedClean=false")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--skip-smoke", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--prepare-sthread-snapshot",
        action="store_true",
        help="create a clean active-branch snapshot with a CoW Gradle cache in at most one minute",
    )
    parser.add_argument(
        "--snapshot-parent",
        type=Path,
        default=Path(tempfile.gettempdir()),
        help="external parent directory for --prepare-sthread-snapshot",
    )
    parser.add_argument(
        "--snapshot-revision",
        help="prepare a clean recovery snapshot at this exact reachable commit instead of HEAD",
    )
    parser.add_argument(
        "--print-compiler-index-root",
        action="store_true",
        help="prepare and print the exact canonical private compiler-index root, then exit",
    )
    return parser.parse_args()


def receipt_exit_code(receipt: dict[str, Any]) -> int:
    return 0 if receipt.get("status") == "READY" else 1


def require_same_compiler_index_graph(first: dict[str, Any], warm: dict[str, Any]) -> None:
    if warm["compilerIndex"]["status"] != "UNCHANGED_HIT":
        raise PreflightFailure(
            "KOTLIN_2_1_COMPILER_INDEX",
            "independent warm compiler-index probe did not reuse the existing generation",
        )
    if warm["compilerIndex"]["graphDigest"] != first["compilerIndex"]["graphDigest"]:
        raise PreflightFailure(
            "KOTLIN_2_1_COMPILER_INDEX",
            "first and unchanged compiler-index generations have different graph digests",
        )


def require_incremental_equivalent_to_full(
    incremental: dict[str, Any], fresh_full: dict[str, Any]
) -> None:
    profile = incremental["compilerIndex"]
    if profile["status"] != "INCREMENTAL":
        raise PreflightFailure(
            "KOTLIN_2_1_INCREMENTAL_EQUIVALENCE",
            "edited same-root probe did not use incremental compilation",
        )
    if not (0 < profile["compiledFiles"] < profile["totalFiles"]):
        raise PreflightFailure(
            "KOTLIN_2_1_INCREMENTAL_EQUIVALENCE",
            "incremental compilation did not reuse a strict subset of indexed files",
        )
    if profile["compiledFiles"] + profile["reusedFiles"] != profile["totalFiles"]:
        raise PreflightFailure(
            "KOTLIN_2_1_INCREMENTAL_EQUIVALENCE",
            "incremental compiler-index file counts are inconsistent",
        )
    if fresh_full["compilerIndex"]["status"] != "COLD_FULL":
        raise PreflightFailure(
            "KOTLIN_2_1_INCREMENTAL_EQUIVALENCE",
            "fresh-root comparison did not perform a full compilation",
        )
    if incremental["semanticIndex"] != fresh_full["semanticIndex"]:
        raise PreflightFailure(
            "KOTLIN_2_1_INCREMENTAL_EQUIVALENCE",
            "incremental and fresh-full normalized semantic graphs differ",
            detail={
                "incremental": incremental["semanticIndex"],
                "freshFull": fresh_full["semanticIndex"],
            },
        )


def initialize_incremental_fixture(
    source: Path, wrapper_source: Path, fixture_root: Path, deadline: float
) -> tuple[Path, Path]:
    target = fixture_root / source.name
    wrapper_target = fixture_root / wrapper_source.name
    clone_tree_contents(source, target, deadline - time.monotonic())
    clone_tree_contents(wrapper_source, wrapper_target, deadline - time.monotonic())
    for repository in (target, wrapper_target):
        for relative in (".semantic-thread", ".git", "build", ".kotlin"):
            candidate = repository / relative
            if candidate.exists():
                if candidate.is_symlink():
                    raise PreflightFailure(
                        "KOTLIN_2_1_INCREMENTAL_EQUIVALENCE",
                        f"temporary fixture contains a symlinked runtime path: {repository.name}/{relative}",
                    )
                shutil.rmtree(candidate)
    delegated_wrapper = wrapper_target / "gradlew"
    if (
        delegated_wrapper.is_symlink()
        or not delegated_wrapper.is_file()
        or not os.access(delegated_wrapper, os.X_OK)
    ):
        raise PreflightFailure(
            "KOTLIN_2_1_INCREMENTAL_EQUIVALENCE",
            "temporary fixture lacks its executable sibling Gradle wrapper",
        )
    commands = (
        ["git", "init", "--quiet"],
        ["git", "add", "-A"],
        [
            "git",
            "-c",
            "user.name=Codeclew Preflight",
            "-c",
            "user.email=preflight@invalid",
            "commit",
            "--quiet",
            "-m",
            "preflight fixture",
        ],
    )
    for argv in commands:
        completed, _elapsed = run_capture(
            argv,
            cwd=target,
            timeout_seconds=deadline - time.monotonic(),
            stage="KOTLIN_2_1_INCREMENTAL_EQUIVALENCE",
        )
        if completed.returncode != 0:
            raise PreflightFailure(
                "KOTLIN_2_1_INCREMENTAL_EQUIVALENCE",
                "temporary incremental fixture initialization failed",
                detail=failure_summary(completed),
            )
    candidates = sorted((target / "src/main/kotlin").rglob("*.kt"))
    candidates = [path for path in candidates if path.is_file() and not path.is_symlink()]
    if len(candidates) < 2:
        raise PreflightFailure(
            "KOTLIN_2_1_INCREMENTAL_EQUIVALENCE",
            "incremental fixture needs at least two regular Kotlin sources",
        )
    return target, candidates[0]


def self_test() -> None:
    event = b'{"event":"request_completed","success":false}\n{\n  "error":{"code":"X","message":"boom"}\n}\n'
    completed = subprocess.CompletedProcess(["clew"], 7, event, b"")
    summary = failure_summary(completed)
    assert summary["code"] == "X" and summary["message"] == "boom"
    assert json_values(event)[0]["event"] == "request_completed"
    assert json_values(event)[1]["error"]["code"] == "X"
    assert gradle_configuration_task(":/main") == "properties"
    assert gradle_configuration_task(":workers:kotlin21/main") == ":workers:kotlin21:properties"
    assert gradle_configuration_arguments(":/main") == ["properties"]
    assert gradle_configuration_arguments(":workers:kotlin21/main") == [
        "-p",
        "workers/kotlin21",
        "properties",
    ]
    assert receipt_exit_code({"status": "READY"}) == 0
    assert receipt_exit_code({"status": "FAILED"}) == 1
    assert receipt_exit_code({}) == 1
    graph = "a" * 64
    require_same_compiler_index_graph(
        {"compilerIndex": {"status": "COLD_FULL", "graphDigest": graph}},
        {"compilerIndex": {"status": "UNCHANGED_HIT", "graphDigest": graph}},
    )
    for warm in (
        {"compilerIndex": {"status": "COLD_FULL", "graphDigest": graph}},
        {"compilerIndex": {"status": "UNCHANGED_HIT", "graphDigest": "b" * 64}},
    ):
        try:
            require_same_compiler_index_graph(
                {"compilerIndex": {"status": "COLD_FULL", "graphDigest": graph}}, warm
            )
            raise AssertionError("invalid independent warm compiler-index proof accepted")
        except PreflightFailure:
            pass
    signature = {
        "declarationRelationHash": "sha256:" + "c" * 64,
        "declarationDescriptorHash": "sha256:" + "d" * 64,
        "files": 4,
    }
    require_incremental_equivalent_to_full(
        {
            "compilerIndex": {
                "status": "INCREMENTAL",
                "totalFiles": 4,
                "compiledFiles": 1,
                "reusedFiles": 3,
            },
            "semanticIndex": signature,
        },
        {"compilerIndex": {"status": "COLD_FULL"}, "semanticIndex": signature},
    )
    try:
        require_incremental_equivalent_to_full(
            {
                "compilerIndex": {
                    "status": "INCREMENTAL",
                    "totalFiles": 4,
                    "compiledFiles": 1,
                    "reusedFiles": 3,
                },
                "semanticIndex": signature,
            },
            {
                "compilerIndex": {"status": "COLD_FULL"},
                "semanticIndex": {**signature, "files": 3},
            },
        )
        raise AssertionError("different incremental and full semantic graphs accepted")
    except PreflightFailure:
        pass
    project_model_row = {
        "status": "PERSISTENT_HIT",
        "publishOutcome": "NOT_ATTEMPTED",
        "publishInvalidReason": "NOT_APPLICABLE",
        "totalMicros": 120,
        "keyMicros": 20,
        "loadMicros": 90,
        "extractionMicros": 0,
        "publishMicros": 0,
        "persistentConfigured": True,
        "published": False,
    }
    project_model_stdout = canonical(
        {
            "schema": "semantic-index-result/0.1",
            "projectModelCache": project_model_row,
        }
    )
    assert project_model_cache_profile(project_model_stdout) == project_model_row
    project_native_row = {
        **project_model_row,
        "status": "EXTRACTED_NOT_PUBLISHED",
        "publishOutcome": "ROOT_UNAVAILABLE",
        "loadMicros": 0,
        "extractionMicros": 30,
        "publishMicros": 1,
        "persistentConfigured": False,
    }
    assert project_model_cache_profile(
        canonical(
            {
                "schema": "semantic-index-result/0.1",
                "projectModelCache": project_native_row,
            }
        )
    ) == project_native_row
    invalid_model_row = {
        **project_model_row,
        "status": "EXTRACTED_NOT_PUBLISHED",
        "publishOutcome": "INVALID_MODEL",
        "publishInvalidReason": "SEMANTIC_INPUT_MANIFEST_HASH_MISMATCH",
        "loadMicros": 10,
        "extractionMicros": 30,
        "publishMicros": 20,
    }
    assert project_model_cache_profile(
        canonical(
            {
                "schema": "semantic-index-result/0.1",
                "projectModelCache": invalid_model_row,
            }
        )
    ) == invalid_model_row
    for invalid in (
        {**project_model_row, "status": "NEW_STATUS"},
        {**project_model_row, "publishOutcome": "NEW_OUTCOME"},
        {**project_model_row, "publishInvalidReason": "NEW_REASON"},
        {**project_model_row, "extractionMicros": 1},
        {**project_model_row, "keyMicros": 121},
        {**project_model_row, "published": True},
        {**project_model_row, "publishOutcome": "WRITE_FAILED"},
        {
            **project_model_row,
            "status": "EXTRACTED_NOT_PUBLISHED",
            "publishOutcome": "INVALID_MODEL",
            "publishInvalidReason": "NOT_APPLICABLE",
            "extractionMicros": 30,
            "publishMicros": 20,
        },
    ):
        try:
            project_model_cache_profile(
                canonical(
                    {
                        "schema": "semantic-index-result/0.1",
                        "projectModelCache": invalid,
                    }
                )
            )
            raise AssertionError("invalid project-model cache profile accepted")
        except PreflightFailure:
            pass
    capability_output = b"Usage: clew --compiler-index-root <DIR> agent-context --model-input <MODEL_INPUT>"
    assert missing_stdout_markers(capability_output, (b"--model-input", b"--compiler-index-root")) == []
    assert missing_stdout_markers(capability_output, (b"--proof-context",)) == ["--proof-context"]
    assert receipt_argv(["clew", "--compiler-index-root", "/secret", "index"]) == [
        "clew",
        "--compiler-index-root",
        "<private-compiler-index-root>",
        "index",
    ]
    try:
        gradle_configuration_task(":workers:kotlin21/test")
        raise AssertionError("non-main compilation accepted")
    except PreflightFailure:
        pass
    with tempfile.TemporaryDirectory(prefix="sthread-preflight-self-test-") as raw:
        root = Path(raw).resolve()
        receipt = root / "receipt.json"
        atomic_json(receipt, {"schema": SCHEMA, "status": "READY"})
        assert receipt.read_bytes() == b'{"schema":"codeclew.sthread-preflight/0.1","status":"READY"}\n'
        cache = root / "cache"
        (cache / "caches").mkdir(parents=True)
        (cache / "wrapper").mkdir()
        write_cache_marker(cache, "sha256:" + "a" * 64, ("caches", "wrapper"))
        assert cache_marker_matches(cache, "sha256:" + "a" * 64, ("caches", "wrapper"))
        assert not cache_marker_matches(cache, "sha256:" + "b" * 64, ("caches", "wrapper"))
        cargo_seed = root / "cargo-seed"
        (cargo_seed / "release").mkdir(parents=True)
        (cargo_seed / "release" / "seed-artifact").write_bytes(b"artifact")
        (cargo_seed / "debug").mkdir()
        (cargo_seed / "debug" / "must-not-copy").write_bytes(b"debug")
        cargo_target = root / "cargo-target"
        elapsed, hit = hydrate_cargo_target(cargo_seed, cargo_target, time.monotonic() + 5, "sha256:" + "c" * 64)
        assert elapsed >= 0 and not hit
        assert (cargo_target / "release" / "seed-artifact").read_bytes() == b"artifact"
        assert not (cargo_target / "debug").exists()
        elapsed, hit = hydrate_cargo_target(cargo_seed, cargo_target, time.monotonic() + 5, "sha256:" + "c" * 64)
        assert elapsed == 0 and hit
        worker_seed = root / "worker-seed"
        worker_target = root / "worker-target"
        for manifest_name, distribution_name in TRUSTED_WORKER_DISTRIBUTIONS:
            (worker_seed / manifest_name).parent.mkdir(parents=True, exist_ok=True)
            (worker_target / manifest_name).parent.mkdir(parents=True, exist_ok=True)
            (worker_seed / manifest_name).write_bytes(manifest_name.encode())
            (worker_target / manifest_name).write_bytes(manifest_name.encode())
            (worker_seed / distribution_name).mkdir(parents=True)
            (worker_seed / distribution_name / "worker").write_bytes(distribution_name.encode())
        elapsed, hit = hydrate_trusted_workers(worker_seed, worker_target, time.monotonic() + 5)
        assert elapsed >= 0 and not hit
        assert all((worker_target / distribution_name / "worker").is_file() for _, distribution_name in TRUSTED_WORKER_DISTRIBUTIONS)
        elapsed, hit = hydrate_trusted_workers(worker_seed, worker_target, time.monotonic() + 5)
        assert elapsed == 0 and hit
        workspace = root / "workspace"
        workspace.mkdir()
        compiler_index = prepare_compiler_index_root(root / "compiler-index", workspace.resolve())
        assert compiler_index.is_dir() and stat.S_IMODE(compiler_index.stat().st_mode) == 0o700
        fixture_seed = root / "fixture-seed"
        kotlin21_seed = fixture_seed / "kotlin-2-1"
        kotlin24_seed = fixture_seed / "kotlin-basic"
        (kotlin21_seed / "src/main/kotlin").mkdir(parents=True)
        (kotlin21_seed / "src/main/kotlin/A.kt").write_bytes(b"class A\n")
        (kotlin21_seed / "src/main/kotlin/B.kt").write_bytes(b"class B\n")
        (kotlin21_seed / "gradlew").write_bytes(b"#!/bin/sh\nexit 0\n")
        (kotlin21_seed / "gradlew").chmod(0o755)
        kotlin24_seed.mkdir(parents=True)
        (kotlin24_seed / "gradlew").write_bytes(b"#!/bin/sh\nexit 0\n")
        (kotlin24_seed / "gradlew").chmod(0o755)
        copied_fixture, changed_source = initialize_incremental_fixture(
            kotlin21_seed,
            kotlin24_seed,
            root / "fixture-copy",
            time.monotonic() + 5,
        )
        assert copied_fixture.name == "kotlin-2-1" and changed_source.name == "A.kt"
        assert os.access(copied_fixture.parent / "kotlin-basic/gradlew", os.X_OK)
        snapshot_seed = root / "snapshot-seed"
        snapshot_seed.mkdir()
        (snapshot_seed / ".gitignore").write_text(".gradle/\nworkers/**/build/\n", encoding="utf-8")
        (snapshot_seed / "gradlew").write_text(
            "#!/usr/bin/env python3\n"
            "import os, sys\n"
            "expected = ['--offline', '--no-daemon', '--quiet', 'properties']\n"
            "if sys.argv[1:] != expected or os.environ.get('GRADLE_USER_HOME') != os.path.join(os.getcwd(), '.gradle'):\n"
            "    raise SystemExit(9)\n",
            encoding="utf-8",
        )
        (snapshot_seed / "gradlew").chmod(0o755)
        for manifest_name, distribution_name in TRUSTED_WORKER_DISTRIBUTIONS:
            manifest = snapshot_seed / manifest_name
            manifest.parent.mkdir(parents=True, exist_ok=True)
            manifest.write_text(manifest_name, encoding="utf-8")
            distribution = snapshot_seed / distribution_name
            distribution.mkdir(parents=True)
            (distribution / "worker").write_text(distribution_name, encoding="utf-8")
        subprocess.run(["git", "init", "--quiet"], cwd=snapshot_seed, check=True)
        subprocess.run(["git", "add", "."], cwd=snapshot_seed, check=True)
        subprocess.run(
            [
                "git",
                "-c",
                "user.name=Codeclew Preflight",
                "-c",
                "user.email=preflight@invalid",
                "commit",
                "--quiet",
                "-m",
                "snapshot seed",
            ],
            cwd=snapshot_seed,
            check=True,
        )
        historical_revision = git_stdout(
            snapshot_seed, "rev-parse", "HEAD", stage="SELF_TEST"
        )
        (snapshot_seed / "later.txt").write_text("newer source head\n", encoding="utf-8")
        subprocess.run(["git", "add", "later.txt"], cwd=snapshot_seed, check=True)
        subprocess.run(
            [
                "git",
                "-c",
                "user.name=Codeclew Preflight",
                "-c",
                "user.email=preflight@invalid",
                "commit",
                "--quiet",
                "-m",
                "newer source head",
            ],
            cwd=snapshot_seed,
            check=True,
        )
        source_revision = git_stdout(
            snapshot_seed, "rev-parse", "HEAD", stage="SELF_TEST"
        )
        assert source_revision != historical_revision
        try:
            require_trusted_worker_seed_revision(
                snapshot_seed, historical_revision
            )
            raise AssertionError("mismatched trusted-worker seed revision accepted")
        except PreflightFailure as error:
            assert error.stage == "TRUSTED_WORKER_HYDRATION"
        historical_worker_seed = root / "historical-worker-seed"
        subprocess.run(
            [
                "git",
                "clone",
                "--quiet",
                "--no-hardlinks",
                str(snapshot_seed),
                str(historical_worker_seed),
            ],
            check=True,
        )
        subprocess.run(
            ["git", "switch", "--quiet", "--detach", historical_revision],
            cwd=historical_worker_seed,
            check=True,
        )
        for _manifest_name, distribution_name in TRUSTED_WORKER_DISTRIBUTIONS:
            shutil.copytree(
                snapshot_seed / distribution_name,
                historical_worker_seed / distribution_name,
            )
        assert (
            require_trusted_worker_seed_revision(
                historical_worker_seed, historical_revision
            )
            == historical_worker_seed.resolve()
        )
        gradle_seed = root / "gradle-seed"
        gradle_seed.mkdir()
        (gradle_seed / "marker").write_text("cache", encoding="utf-8")
        fake_clew = root / "fake-clew"
        fake_clew.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os\n"
            "names = " + repr(TRUSTED_WORKER_ENVIRONMENT_REMOVALS) + "\n"
            "print(json.dumps({name: os.environ.get(name) for name in names}, sort_keys=True))\n",
            encoding="utf-8",
        )
        fake_clew.chmod(0o700)
        snapshot_parent = root / "snapshot-parent"
        snapshot_parent.mkdir()
        snapshot = prepare_sthread_snapshot(
            argparse.Namespace(
                workspace=snapshot_seed,
                gradle_cache_seed=gradle_seed,
                trusted_worker_seed=historical_worker_seed,
                snapshot_parent=snapshot_parent,
                budget_seconds=5.0,
                clew=fake_clew,
                compilation=":/main",
                snapshot_revision=historical_revision,
            )
        )
        assert snapshot["status"] == "READY"
        assert snapshot["stage"] == "STHREAD_SNAPSHOT_READY"
        prepared_repository = Path(snapshot["repository"])
        assert snapshot["workspaceRevision"] == historical_revision
        assert snapshot["sourceWorkspaceRevision"] == source_revision
        assert git_stdout(prepared_repository, "rev-parse", "HEAD", stage="SELF_TEST") == snapshot["workspaceRevision"]
        assert git_stdout(prepared_repository, "symbolic-ref", "--short", "HEAD", stage="SELF_TEST") == snapshot["targetRef"]
        assert git_stdout(prepared_repository, "status", "--porcelain=v1", stage="SELF_TEST") == ""
        assert (Path(snapshot["gradleUserHome"]) / "marker").read_text(encoding="utf-8") == "cache"
        trusted_launcher = Path(snapshot["trustedClewLauncher"])
        assert trusted_launcher.is_file() and os.access(trusted_launcher, os.X_OK)
        poisoned = os.environ.copy()
        for name in TRUSTED_WORKER_ENVIRONMENT_REMOVALS:
            poisoned[name] = "forged"
        sanitized = subprocess.run(
            [str(trusted_launcher)],
            cwd=prepared_repository,
            env=poisoned,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=True,
        )
        assert json.loads(sanitized.stdout) == {
            name: None for name in TRUSTED_WORKER_ENVIRONMENT_REMOVALS
        }
        assert snapshot["agentContextEnvironmentRemovals"] == list(
            TRUSTED_WORKER_ENVIRONMENT_REMOVALS
        )
        assert snapshot["cache"]["gradleRouteProbeMillis"] >= 0
    print(json.dumps({"schema": SCHEMA, "status": "SELF_TEST_PASSED"}, separators=(",", ":")))


def execute(args: argparse.Namespace) -> dict[str, Any]:
    started = time.monotonic()
    if args.budget_seconds is not None and args.budget_seconds <= 0:
        raise PreflightFailure("ARGUMENTS", "budget-seconds must be positive")
    deadline = math.inf if args.budget_seconds is None else started + args.budget_seconds
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
    compiler_index_root = prepare_compiler_index_root(args.compiler_index_root, workspace)
    fingerprint = toolchain_fingerprint(workspace)
    head = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=workspace, stdout=subprocess.PIPE, check=True
    ).stdout.decode().strip()
    format_probe = probe(
        kind="RUST_FORMAT",
        argv=["cargo", "fmt", "--all", "--", "--check"],
        cwd=workspace,
        deadline=deadline,
    )

    gradle_seed = require_real_directory(args.gradle_cache_seed, "CACHE_HYDRATION")
    fixture21 = workspace / "fixtures" / "kotlin-2-1"
    fixture24 = workspace / "fixtures" / "kotlin-basic"
    fixture_maven = workspace / "fixtures" / "kotlin-maven"
    if not args.skip_smoke:
        require_real_directory(fixture21, "SMOKE_FIXTURE")
        require_real_directory(fixture24, "SMOKE_FIXTURE")
        require_real_directory(fixture_maven, "SMOKE_FIXTURE")
    hydration_workers = 6 if not args.skip_smoke else 3
    with ThreadPoolExecutor(max_workers=hydration_workers, thread_name_prefix="sthread-hydration") as executor:
        cargo_future = executor.submit(
            hydrate_cargo_target,
            args.cargo_target_seed,
            workspace / "target",
            deadline,
            fingerprint,
        )
        worker_future = executor.submit(hydrate_trusted_workers, args.trusted_worker_seed, workspace, deadline)
        workspace_gradle_future = executor.submit(
            hydrate_gradle_cache,
            gradle_seed,
            workspace / ".gradle",
            deadline,
            fingerprint,
        )
        if not args.skip_smoke:
            fixture21_future = executor.submit(
                hydrate_gradle_cache,
                gradle_seed,
                fixture21 / ".gradle",
                deadline,
                fingerprint,
            )
            fixture24_future = executor.submit(
                hydrate_gradle_cache,
                gradle_seed,
                fixture24 / ".gradle",
                deadline,
                fingerprint,
            )
            maven_future = executor.submit(
                hydrate_maven_repository,
                args.maven_repository_seed,
                fixture_maven / ".semantic-thread" / "maven-repository",
                deadline,
                fingerprint,
            )
        cargo_millis, cargo_cache_hit = cargo_future.result()
        worker_millis, worker_cache_hit = worker_future.result()
        copied, gradle_millis, workspace_cache_hit = workspace_gradle_future.result()
        if not args.skip_smoke:
            _, fixture21_gradle_millis, fixture21_cache_hit = fixture21_future.result()
            _, fixture24_gradle_millis, fixture24_cache_hit = fixture24_future.result()
            gradle_millis += fixture21_gradle_millis + fixture24_gradle_millis
            maven_millis, maven_cache_hit = maven_future.result()
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
    configuration_arguments = gradle_configuration_arguments(args.compilation)
    probes = [
        format_probe,
        probe(
            kind="STHREAD_PROTOCOL_CAPABILITY",
            argv=[str(clew), "agent-context", "--help"],
            cwd=workspace,
            deadline=deadline,
            required_stdout_markers=(b"--model-input", b"--compiler-index-root"),
        )
    ]
    runtime_specifications: list[dict[str, Any]] = [
        {
            "kind": "GRADLE_CONFIGURATION",
            "argv": [
                str(workspace / "gradlew"),
                "--offline",
                "--no-daemon",
                "--quiet",
                *configuration_arguments,
            ],
            "cwd": workspace,
            "environment": gradle_environment,
        },
        {
            "kind": "TRUSTED_WORKER_BOOTSTRAP",
            "argv": [str(clew), "doctor"],
            "cwd": workspace,
            "required_stdout_markers": (
                b'"semantic-doctor/0.1"',
                b'"compilerVersion"',
            ),
        },
    ]
    if not args.skip_smoke:
        runtime_specifications.append(
            {
                "kind": "OPEN_PROJECT_KOTLIN_2_4",
                "argv": [
                    str(clew),
                    "project",
                    "inspect",
                    "--repo",
                    str(fixture24),
                    "--compilation",
                    ":/main",
                ],
                "cwd": workspace,
            }
        )
        for kind, relative_repo, compilation, _build_system in DEFAULT_SMOKES:
            compiler_index_arguments = (
                ["--compiler-index-root", str(compiler_index_root)]
                if kind == "KOTLIN_2_1_COMPILER_SEMANTIC"
                else []
            )
            runtime_specifications.append(
                {
                    "kind": kind,
                    "argv": [
                        str(clew),
                        *compiler_index_arguments,
                        "index",
                        "--repo",
                        str(workspace / relative_repo),
                        "--compilation",
                        compilation,
                    ],
                    "cwd": workspace,
                    "require_compiler_index": kind == "KOTLIN_2_1_COMPILER_SEMANTIC",
                    "require_project_model_cache": kind == "KOTLIN_2_1_COMPILER_SEMANTIC",
                }
            )
    runtime_probes = probe_group(runtime_specifications, deadline)
    probes.extend(runtime_probes)
    if not args.skip_smoke:
        kotlin21 = next(row for row in runtime_probes if row["kind"] == "KOTLIN_2_1_COMPILER_SEMANTIC")
        warm_probe = probe(
            kind="KOTLIN_2_1_COMPILER_INDEX_WARM",
            argv=[
                str(clew),
                "--compiler-index-root",
                str(compiler_index_root),
                "index",
                "--repo",
                str(fixture21),
                "--compilation",
                ":/main",
            ],
            cwd=workspace,
            deadline=deadline,
            expected_compiler_index_status="UNCHANGED_HIT",
            expected_project_model_cache_status="EXTRACTED_NOT_PUBLISHED",
        )
        require_same_compiler_index_graph(kotlin21, warm_probe)
        probes.append(warm_probe)
        with tempfile.TemporaryDirectory(prefix="codeclew-sthread-incremental-") as raw:
            incremental_root = Path(raw).resolve()
            incremental_fixture, changed_source = initialize_incremental_fixture(
                fixture21, fixture24, incremental_root / "fixtures", deadline
            )
            shared_index_root = prepare_compiler_index_root(
                incremental_root / "same-index", workspace
            )
            fresh_index_root = prepare_compiler_index_root(
                incremental_root / "fresh-index", workspace
            )
            base_probe = probe(
                kind="KOTLIN_2_1_INCREMENTAL_BASE",
                argv=[
                    str(clew),
                    "--compiler-index-root",
                    str(shared_index_root),
                    "index",
                    "--repo",
                    str(incremental_fixture),
                    "--compilation",
                    ":/main",
                ],
                cwd=workspace,
                deadline=deadline,
                expected_compiler_index_status="COLD_FULL",
                expected_project_model_cache_status="EXTRACTED_NOT_PUBLISHED",
            )
            with changed_source.open("ab") as stream:
                stream.write(b"\n// codeclew incremental preflight\n")
                stream.flush()
                os.fsync(stream.fileno())
            incremental_probe = probe(
                kind="KOTLIN_2_1_INCREMENTAL_EDITED",
                argv=[
                    str(clew),
                    "--compiler-index-root",
                    str(shared_index_root),
                    "index",
                    "--repo",
                    str(incremental_fixture),
                    "--compilation",
                    ":/main",
                ],
                cwd=workspace,
                deadline=deadline,
                expected_compiler_index_status="INCREMENTAL",
                expected_project_model_cache_status="EXTRACTED_NOT_PUBLISHED",
            )
            fresh_full_probe = probe(
                kind="KOTLIN_2_1_INCREMENTAL_FRESH_FULL",
                argv=[
                    str(clew),
                    "--compiler-index-root",
                    str(fresh_index_root),
                    "index",
                    "--repo",
                    str(incremental_fixture),
                    "--compilation",
                    ":/main",
                ],
                cwd=workspace,
                deadline=deadline,
                expected_compiler_index_status="COLD_FULL",
                expected_project_model_cache_status="EXTRACTED_NOT_PUBLISHED",
            )
            require_incremental_equivalent_to_full(incremental_probe, fresh_full_probe)
            probes.extend((base_probe, incremental_probe, fresh_full_probe))

    elapsed = monotonic_millis(started)
    return {
        "schema": SCHEMA,
        "status": "READY",
        "stage": "COMPLETE",
        "workspaceRevision": head,
        "trackedClean": tracked_clean,
        "toolchainFingerprint": fingerprint,
        "compilerIndexRootIdentity": sha256_bytes(str(compiler_index_root).encode()),
        "budgetMillis": None if args.budget_seconds is None else round(args.budget_seconds * 1000),
        "elapsedMillis": elapsed,
        "cache": {
            "cargoHydrationMillis": cargo_millis,
            "cargoHit": cargo_cache_hit,
            "trustedWorkerHydrationMillis": worker_millis,
            "trustedWorkerHit": worker_cache_hit,
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
    selected_modes = sum(
        bool(value)
        for value in (args.self_test, args.print_compiler_index_root, args.prepare_sthread_snapshot)
    )
    if selected_modes > 1:
        print("preflight modes are mutually exclusive", file=sys.stderr)
        return 2
    if args.self_test:
        self_test()
        return 0
    if args.print_compiler_index_root:
        workspace = require_real_directory(args.workspace, "WORKSPACE")
        root = prepare_compiler_index_root(args.compiler_index_root, workspace)
        print(
            json.dumps(
                {
                    "schema": SCHEMA,
                    "status": "COMPILER_INDEX_ROOT_READY",
                    "path": str(root),
                    "identity": sha256_bytes(str(root).encode()),
                },
                separators=(",", ":"),
            )
        )
        return 0
    started = time.monotonic()
    try:
        receipt = prepare_sthread_snapshot(args) if args.prepare_sthread_snapshot else execute(args)
    except PreflightFailure as error:
        receipt = {
            "schema": SCHEMA,
            "status": "FAILED",
            "stage": error.stage,
            "elapsedMillis": monotonic_millis(started),
            "failure": {"message": str(error), **error.detail},
        }
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
    if args.receipt is not None:
        atomic_json(args.receipt, receipt)
    print(canonical(receipt).decode(), end="")
    return receipt_exit_code(receipt)


if __name__ == "__main__":
    raise SystemExit(main())
