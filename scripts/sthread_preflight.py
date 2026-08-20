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
        row["workerProfile"] = worker_profile
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


def self_test() -> None:
    event = b'{"event":"request_completed","success":false}\n{\n  "error":{"code":"X","message":"boom"}\n}\n'
    completed = subprocess.CompletedProcess(["clew"], 7, event, b"")
    summary = failure_summary(completed)
    assert summary["code"] == "X" and summary["message"] == "boom"
    assert json_values(event)[0]["event"] == "request_completed"
    assert json_values(event)[1]["error"]["code"] == "X"
    assert gradle_configuration_task(":/main") == "properties"
    assert gradle_configuration_task(":workers:kotlin21/main") == ":workers:kotlin21:properties"
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
    configuration_task = gradle_configuration_task(args.compilation)
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
                configuration_task,
            ],
            "cwd": workspace,
            "environment": gradle_environment,
        }
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
        )
        require_same_compiler_index_graph(kotlin21, warm_probe)
        probes.append(warm_probe)

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
