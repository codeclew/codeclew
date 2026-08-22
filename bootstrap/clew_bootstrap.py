#!/usr/bin/env python3
"""Build or reuse one immutable Codeclew runtime capsule, then execute it."""

from __future__ import annotations

import argparse
import contextlib
from concurrent.futures import ThreadPoolExecutor
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import uuid


SCHEMA = "codeclew-runtime-capsule/2.0"
DOMAIN = b"codeclew-runtime/v2\0"
MAX_MANIFEST_BYTES = 1024 * 1024
MAX_CHECKPOINT_BYTES = 16 * 1024 * 1024
MAX_CHECKPOINT_NODES = 100_000
MIN_COLD_BUILD_FREE_BYTES = 6 * 1024 * 1024 * 1024
BUILD_TERMINATION_GRACE_SECONDS = 2.0
BUILD_KILL_WAIT_SECONDS = 2.0
WORKERS = {
    "kotlin21": ("2.1.21", "workers/kotlin21/build/install/kotlin21", "workers/manifests/kotlin21.json"),
    "kotlin23": ("2.3.0", "workers/kotlin23/build/install/kotlin23", "workers/manifests/kotlin23.json"),
    "kotlin24": ("2.4.10", "workers/kotlin/build/install/kotlin", "workers/manifests/kotlin24.json"),
}
ROOT_FILES = {
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "build.gradle.kts",
    "settings.gradle.kts", "gradlew", "gradlew.bat", "clew",
}
INJECTION_ENV = {
    "RUSTC", "RUSTC_WRAPPER", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS",
    "RUSTUP_TOOLCHAIN", "CARGO_BUILD_TARGET", "CARGO_TARGET_DIR",
    "JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "_JAVA_OPTIONS", "GRADLE_OPTS",
    "PYTHONPATH", "PYTHONHOME", "PYTHONSTARTUP", "PYTHONINSPECT",
    "PYTHONWARNINGS", "PYTHONSAFEPATH",
}

STATE_ROOT_FD_ENV = "CODECLEW_STATE_ROOT_FD"
RUNTIME_ROOT_FD_ENV = "CODECLEW_RUNTIME_ROOT_FD"
RUNTIME_LEASE_FD_ENV = "CODECLEW_RUNTIME_LEASE_FD"

_AUDIT_COUNTERS = {
    "processRuns": 0,
    "digestFileCalls": 0,
    "metadataChecks": 0,
    "checkpointHits": 0,
    "checkpointMisses": 0,
}


class BootstrapError(RuntimeError):
    pass


class BootstrapInterrupted(BootstrapError):
    def __init__(self, signum: int):
        self.signum = signum
        super().__init__(f"capsule build interrupted by signal {signum}")


def _process_group_exists(process_group: int) -> bool:
    try:
        os.killpg(process_group, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def _signal_process_groups(processes: list[subprocess.Popen[bytes]], signum: int) -> None:
    for process in processes:
        try:
            os.killpg(process.pid, signum)
        except ProcessLookupError:
            pass


def _terminate_process_groups(
    processes: list[subprocess.Popen[bytes]],
    *,
    grace_seconds: float = BUILD_TERMINATION_GRACE_SECONDS,
    kill_wait_seconds: float = BUILD_KILL_WAIT_SECONDS,
) -> None:
    """Boundedly stop complete build process groups, including surviving children."""
    process_groups = sorted({process.pid for process in processes})
    if not process_groups:
        return
    _signal_process_groups(processes, signal.SIGTERM)
    grace_deadline = time.monotonic() + max(0.0, grace_seconds)
    survivors = process_groups
    while survivors and time.monotonic() < grace_deadline:
        for process in processes:
            process.poll()
        survivors = [group for group in survivors if _process_group_exists(group)]
        if survivors:
            time.sleep(min(0.05, max(0.0, grace_deadline - time.monotonic())))
    survivors = [group for group in survivors if _process_group_exists(group)]
    for process_group in survivors:
        try:
            os.killpg(process_group, signal.SIGKILL)
        except ProcessLookupError:
            pass
    kill_deadline = time.monotonic() + max(0.0, kill_wait_seconds)
    while survivors and time.monotonic() < kill_deadline:
        for process in processes:
            process.poll()
        survivors = [group for group in survivors if _process_group_exists(group)]
        if survivors:
            time.sleep(min(0.05, max(0.0, kill_deadline - time.monotonic())))
    # Reap group leaders without extending the bounded group-shutdown deadline.
    for process in processes:
        try:
            process.wait(timeout=0)
        except (subprocess.TimeoutExpired, ChildProcessError):
            pass


class BuildProcessSupervisor:
    """Own every cold-build process group until it exits or is cancelled."""

    def __init__(self) -> None:
        self._cancelled = threading.Event()
        self._lock = threading.Lock()
        self._processes: dict[int, subprocess.Popen[bytes]] = {}
        self._termination_started: set[int] = set()

    def request_cancel(self) -> None:
        self._cancelled.set()

    def cancelled(self) -> bool:
        return self._cancelled.is_set()

    def register(self, process: subprocess.Popen[bytes]) -> None:
        with self._lock:
            self._processes[process.pid] = process
            cancelled = self._cancelled.is_set()
        if cancelled:
            self.cancel()
            raise BootstrapError("capsule build was cancelled")

    def unregister(self, process: subprocess.Popen[bytes]) -> None:
        with self._lock:
            self._processes.pop(process.pid, None)

    def cancel(self) -> None:
        self.request_cancel()
        with self._lock:
            processes = [
                process for process_id, process in self._processes.items()
                if process_id not in self._termination_started
            ]
            self._termination_started.update(process.pid for process in processes)
        _terminate_process_groups(processes)


@contextlib.contextmanager
def build_signal_scope(supervisor: BuildProcessSupervisor):
    """Translate process cancellation signals while preserving prior handlers."""
    if threading.current_thread() is not threading.main_thread():
        yield
        return
    previous = {}

    def interrupt(signum: int, _frame: object) -> None:
        # Popen runs in worker threads, so a signal cannot land between fork and
        # registration on the Python signal-handling thread.
        supervisor.request_cancel()
        raise BootstrapInterrupted(signum)

    try:
        for signum in (signal.SIGINT, signal.SIGTERM):
            previous[signum] = signal.getsignal(signum)
            signal.signal(signum, interrupt)
        yield
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def digest_bytes(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def valid_runtime_key(value: object) -> bool:
    if not isinstance(value, str) or not value.startswith("sha256:"):
        return False
    digest = value.removeprefix("sha256:")
    return len(digest) == 64 and all(character in "0123456789abcdef" for character in digest)


def digest_file(path: Path) -> str:
    _AUDIT_COUNTERS["digestFileCalls"] += 1
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return "sha256:" + digest.hexdigest()


def run(arguments: list[str], cwd: Path, environment: dict[str, str] | None = None) -> bytes:
    _AUDIT_COUNTERS["processRuns"] += 1
    completed = subprocess.run(
        arguments,
        cwd=cwd,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=None,
        check=False,
    )
    if completed.returncode != 0:
        raise BootstrapError(f"bootstrap command failed ({completed.returncode}): {arguments[0]}")
    return completed.stdout


def progress(event: str, stage: str, **values: object) -> None:
    print(canonical({
        "schema": "codeclew-capsule-progress/2.0",
        "event": event,
        "stage": stage,
        **values,
    }).decode(), file=sys.stderr, flush=True)


def reset_audit_counters() -> None:
    for name in _AUDIT_COUNTERS:
        _AUDIT_COUNTERS[name] = 0


def _metadata_identity(path: Path) -> dict[str, object]:
    _AUDIT_COUNTERS["metadataChecks"] += 1
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return {"path": str(path), "exists": False}
    return {
        "path": str(path),
        "exists": True,
        "device": metadata.st_dev,
        "inode": metadata.st_ino,
        "size": metadata.st_size,
        "mode": metadata.st_mode,
        "modifiedNs": metadata.st_mtime_ns,
        "changedNs": metadata.st_ctime_ns,
    }


def _metadata_matches(row: dict[str, object]) -> bool:
    path_value = row.get("path")
    if not isinstance(path_value, str) or not Path(path_value).is_absolute():
        return False
    return _metadata_identity(Path(path_value)) == row


def _environment_checkpoint() -> dict[str, object]:
    return {
        "PATH": os.environ.get("PATH"),
        "JAVA_HOME": os.environ.get("JAVA_HOME"),
        "HOME": os.environ.get("HOME"),
        "XDG_CONFIG_HOME": os.environ.get("XDG_CONFIG_HOME"),
        "GIT_CONFIG_GLOBAL": os.environ.get("GIT_CONFIG_GLOBAL"),
        "GIT_CONFIG_SYSTEM": os.environ.get("GIT_CONFIG_SYSTEM"),
        "MACOSX_DEPLOYMENT_TARGET": os.environ.get("MACOSX_DEPLOYMENT_TARGET"),
        "pythonExecutable": str(Path(sys.executable).resolve(strict=True)),
        "pythonVersion": platform.python_version(),
        "platform": [
            platform.system(),
            platform.release(),
            platform.machine(),
            list(platform.libc_ver()),
        ],
    }


def run_build_stage(
    arguments: list[str],
    cwd: Path,
    environment: dict[str, str],
    stage: str,
    supervisor: BuildProcessSupervisor | None = None,
) -> None:
    supervisor = supervisor or BuildProcessSupervisor()
    if supervisor.cancelled():
        raise BootstrapError("capsule build was cancelled")
    progress("STAGE_STARTED", stage)
    started = time.monotonic()
    process: subprocess.Popen[bytes] | None = None
    try:
        process = subprocess.Popen(
            arguments,
            cwd=cwd,
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=None,
            start_new_session=True,
        )
        supervisor.register(process)
        next_heartbeat = started + 5
        while True:
            if supervisor.cancelled():
                raise BootstrapError("capsule build was cancelled")
            return_code = process.poll()
            if return_code is not None:
                break
            now = time.monotonic()
            if now >= next_heartbeat:
                progress(
                    "HEARTBEAT",
                    stage,
                    durationMillis=int((now - started) * 1000),
                )
                next_heartbeat = now + 5
            time.sleep(0.25)
        duration = int((time.monotonic() - started) * 1000)
        if return_code != 0:
            progress("STAGE_FAILED", stage, durationMillis=duration, exitCode=return_code)
            raise BootstrapError(f"capsule build stage failed ({return_code}): {stage}")
        progress("STAGE_COMPLETED", stage, durationMillis=duration)
    except BaseException:
        supervisor.cancel()
        raise
    finally:
        if process is not None:
            supervisor.unregister(process)


def selected_source(relative: str) -> bool:
    if ".semantic-thread" in Path(relative).parts:
        return False
    if relative in ROOT_FILES:
        return True
    if relative.startswith("bootstrap/"):
        return relative == "bootstrap/clew_bootstrap.py"
    if relative.startswith("schemas/"):
        return True
    if relative.startswith("gradle/wrapper/"):
        return True
    if relative.startswith(".cargo/"):
        return True
    if relative.startswith("crates/"):
        parts = relative.split("/")
        if "tests" in parts or "examples" in parts or "target" in parts:
            return False
        return parts[-1] in {"Cargo.toml", "build.rs"} or "/src/" in relative
    if relative.startswith("workers/manifests/"):
        return relative.endswith(".json")
    if relative.startswith("workers/"):
        parts = relative.split("/")
        if len(parts) == 3 and parts[-1] == "build.gradle.kts":
            return True
        return "/src/main/" in relative
    return False


def source_manifest(source: Path) -> tuple[list[dict[str, object]], bool]:
    exclusions = [
        ":(top,exclude).semantic-thread",
        ":(top,glob,exclude).semantic-thread/**",
        ":(top,glob,exclude)**/.semantic-thread",
        ":(top,glob,exclude)**/.semantic-thread/**",
    ]
    tracked = run(["git", "ls-files", "-z", "--", ".", *exclusions], source).split(b"\0")
    untracked = run(
        ["git", "ls-files", "--others", "--exclude-standard", "-z", "--", ".", *exclusions],
        source,
    ).split(b"\0")
    paths = sorted({row.decode() for row in [*tracked, *untracked] if row and selected_source(row.decode())})
    if not paths:
        raise BootstrapError("runtime input closure is empty")
    rows: list[dict[str, object]] = []
    for relative in paths:
        path = source / relative
        try:
            metadata = path.lstat()
        except FileNotFoundError:
            # A tracked deletion is part of DEVELOPMENT mode, but deleted bytes are
            # intentionally absent from the actual runtime input closure.
            continue
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
            raise BootstrapError(f"runtime input is not a regular file: {relative}")
        rows.append({
            "path": relative,
            "size": metadata.st_size,
            "mode": metadata.st_mode & 0o111,
            "sha256": digest_file(path),
        })
    dirty_output = run(
        ["git", "status", "--porcelain=v1", "--untracked-files=all", "--", ".", *exclusions],
        source,
    ).decode()
    dirty_paths = set()
    for line in dirty_output.splitlines():
        value = line[3:]
        if " -> " in value:
            value = value.split(" -> ", 1)[1]
        dirty_paths.add(value.strip('"'))
    development = any(selected_source(path) for path in dirty_paths)
    return rows, development


def verify_source_manifest(
    source: Path,
    rows: list[dict[str, object]],
    *,
    full_closure: bool = True,
    expected_development: bool | None = None,
) -> None:
    for row in rows:
        path = source / str(row["path"])
        metadata = path.lstat()
        if (
            stat.S_ISLNK(metadata.st_mode)
            or not stat.S_ISREG(metadata.st_mode)
            or metadata.st_size != row["size"]
            or metadata.st_mode & 0o111 != row["mode"]
            or digest_file(path) != row["sha256"]
        ):
            raise BootstrapError(f"runtime input changed during bootstrap: {row['path']}")
    if full_closure:
        observed, development = source_manifest(source)
        if observed != rows or (
            expected_development is not None and development != expected_development
        ):
            raise BootstrapError("runtime input closure changed during bootstrap")


def _directory_flags() -> int:
    return os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0)


def _open_private_tree(path: Path) -> int:
    """Create/open an absolute private tree without following a path component."""
    if not path.is_absolute() or ".." in path.parts:
        raise BootstrapError("Codeclew state root must be normalized and absolute")
    descriptor = os.open("/", _directory_flags())
    try:
        parts = [part for part in path.parts if part != path.anchor]
        for index, component in enumerate(parts):
            try:
                child = os.open(component, _directory_flags(), dir_fd=descriptor)
            except FileNotFoundError:
                os.mkdir(component, mode=0o700, dir_fd=descriptor)
                child = os.open(component, _directory_flags(), dir_fd=descriptor)
            except NotADirectoryError as error:
                raise BootstrapError(
                    "Codeclew state root contains a symlink or non-directory ancestor; "
                    "use a physical normalized CODECLEW_HOME path"
                ) from error
            metadata = os.fstat(child)
            leaf = index == len(parts) - 1
            if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid not in {0, os.geteuid()}:
                os.close(child)
                raise BootstrapError("Codeclew state ancestor is unsafe")
            if not leaf and stat.S_IMODE(metadata.st_mode) & 0o022:
                os.close(child)
                raise BootstrapError("Codeclew state ancestor is group/world writable")
            if leaf:
                if metadata.st_uid != os.geteuid():
                    os.close(child)
                    raise BootstrapError("Codeclew state root has a different owner")
                os.fchmod(child, 0o700)
            os.close(descriptor)
            descriptor = child
        return descriptor
    except Exception:
        os.close(descriptor)
        raise


def _ensure_private_descendant(root_fd: int, relative: str) -> None:
    descriptor = os.dup(root_fd)
    try:
        for component in Path(relative).parts:
            if component in {"", ".", ".."}:
                raise BootstrapError("Codeclew state child is invalid")
            try:
                child = os.open(component, _directory_flags(), dir_fd=descriptor)
            except FileNotFoundError:
                os.mkdir(component, mode=0o700, dir_fd=descriptor)
                child = os.open(component, _directory_flags(), dir_fd=descriptor)
            metadata = os.fstat(child)
            if (
                not stat.S_ISDIR(metadata.st_mode)
                or metadata.st_uid != os.geteuid()
                or stat.S_IMODE(metadata.st_mode) & 0o077
            ):
                os.close(child)
                raise BootstrapError(f"Codeclew state child is unsafe: {relative}")
            os.fchmod(child, 0o700)
            os.close(descriptor)
            descriptor = child
    finally:
        os.close(descriptor)


def state_root() -> tuple[Path, int]:
    explicit = os.environ.get("CODECLEW_HOME")
    if explicit:
        root = Path(explicit)
    elif os.environ.get("XDG_CACHE_HOME"):
        root = Path(os.environ["XDG_CACHE_HOME"]) / "codeclew"
    else:
        root = Path.home() / ".cache" / "codeclew"
    home_fd = _open_private_tree(root)
    try:
        _ensure_private_descendant(home_fd, "v2")
    finally:
        os.close(home_fd)
    root = root / "v2"
    root_fd = _open_private_tree(root)
    for child in [
        "runtimes", "repos", "sessions", "runs", "locks", "tmp", "quarantine",
        "objects", "objects/sha256", "objects/packs-v3", "generations", "attempts", "gc",
        "dependency-cache", "runtimes/locators", "runtimes/checkpoints",
    ]:
        _ensure_private_descendant(root_fd, child)
    return root, root_fd


def require_cold_build_capacity(root: Path) -> None:
    free = shutil.disk_usage(root).free
    if free < MIN_COLD_BUILD_FREE_BYTES:
        required_gib = MIN_COLD_BUILD_FREE_BYTES // (1024 * 1024 * 1024)
        available_mib = free // (1024 * 1024)
        raise BootstrapError(
            f"cold runtime build requires at least {required_gib} GiB free "
            f"on the CODECLEW_HOME volume; available={available_mib} MiB"
        )


def sanitized_environment() -> dict[str, str]:
    def allowed(name: str) -> bool:
        if name in INJECTION_ENV or name.startswith("CODECLEW_"):
            return False
        if name.startswith("CARGO_TARGET_") and name.endswith(("_RUNNER", "_LINKER", "_RUSTFLAGS")):
            return False
        if name.startswith("CARGO_BUILD_") and name.endswith(("RUSTC", "RUSTC_WRAPPER", "RUSTFLAGS")):
            return False
        return True

    return {name: value for name, value in os.environ.items() if allowed(name)}


def toolchain_authority(source: Path) -> dict[str, object]:
    environment = sanitized_environment()
    python_executable = Path(sys.executable).resolve(strict=True)
    rustc = run(["rustc", "-Vv"], source, environment).decode().strip()
    cargo = run(["cargo", "-V"], source, environment).decode().strip()
    java_home = Path(environment.get("JAVA_HOME", ""))
    if not java_home.is_absolute():
        java_binary = shutil.which("java")
        if not java_binary:
            raise BootstrapError("JDK 21 is unavailable")
        java_home = Path(java_binary).resolve(strict=True).parent.parent
    java_files = [java_home / "release", java_home / "bin/java", java_home / "lib/modules"]
    if not all(path.is_file() and not path.is_symlink() for path in java_files[:2]):
        raise BootstrapError("JDK authority files are unavailable")
    java_release = (java_home / "release").read_text(errors="strict")
    platform_authority: dict[str, object] = {
        "system": platform.system(),
        "release": platform.release(),
        "machine": platform.machine(),
        "libc": platform.libc_ver(),
        "rustTarget": next(
            (line.split(":", 1)[1].strip() for line in rustc.splitlines() if line.startswith("host:")),
            None,
        ),
    }
    if platform.system() == "Darwin":
        platform_authority["deploymentTarget"] = environment.get("MACOSX_DEPLOYMENT_TARGET")
        platform_authority["productVersion"] = platform.mac_ver()[0]
    elif platform.system() == "Linux":
        loaders = [
            Path("/lib64/ld-linux-x86-64.so.2"),
            Path("/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2"),
            Path("/lib/ld-musl-x86_64.so.1"),
            Path("/lib/ld-musl-aarch64.so.1"),
            Path("/lib/ld-linux-aarch64.so.1"),
        ]
        loader = next((candidate for candidate in loaders if candidate.is_file()), None)
        platform_authority["elfLoaderSha256"] = digest_file(loader) if loader else None
    return {
        "python": {
            "implementation": platform.python_implementation(),
            "version": platform.python_version(),
            "executableSha256": digest_file(python_executable),
        },
        "rust": {"rustcVv": digest_bytes(rustc.encode()), "cargoVersion": cargo},
        "jdk": {
            "releaseSha256": digest_bytes(java_release.encode()),
            "javaSha256": digest_file(java_home / "bin/java"),
            "modulesSha256": digest_file(java_home / "lib/modules") if (java_home / "lib/modules").is_file() else None,
        },
        "platform": platform_authority,
    }


def fast_toolchain_locator_authority() -> dict[str, object]:
    python_executable = Path(sys.executable).resolve(strict=True)
    resolved = {}
    for name in ["rustc", "cargo", "java"]:
        executable = shutil.which(name)
        if not executable:
            raise BootstrapError(f"{name} is unavailable")
        path = Path(executable).resolve(strict=True)
        metadata = path.stat()
        resolved[name] = {
            "path": str(path),
            "device": metadata.st_dev,
            "inode": metadata.st_ino,
            "size": metadata.st_size,
            "modifiedNs": metadata.st_mtime_ns,
        }
    java_home = Path(os.environ.get("JAVA_HOME", ""))
    if not java_home.is_absolute():
        java_home = Path(resolved["java"]["path"]).parent.parent
    release = java_home / "release"
    if not release.is_file() or release.is_symlink():
        raise BootstrapError("JDK release authority is unavailable")
    release_metadata = release.stat()
    python_metadata = python_executable.stat()
    return {
        "python": {
            "implementation": platform.python_implementation(),
            "version": platform.python_version(),
            "path": str(python_executable),
            "device": python_metadata.st_dev,
            "inode": python_metadata.st_ino,
            "size": python_metadata.st_size,
            "modifiedNs": python_metadata.st_mtime_ns,
        },
        "executables": resolved,
        "jdkRelease": {
            "path": str(release),
            "device": release_metadata.st_dev,
            "inode": release_metadata.st_ino,
            "size": release_metadata.st_size,
            "modifiedNs": release_metadata.st_mtime_ns,
        },
        "platform": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "libc": platform.libc_ver(),
        },
    }


def locator_key(mode: str, inputs: list[dict[str, object]], fast_tools: dict[str, object]) -> str:
    return digest_bytes(DOMAIN + b"locator\0" + mode.encode() + b"\0" + canonical({
        "inputs": inputs,
        "toolchains": fast_tools,
    }))


def locator_path(root: Path, locator: str) -> Path:
    directory = root / "runtimes" / "locators"
    directory.mkdir(mode=0o700, exist_ok=True)
    os.chmod(directory, 0o700)
    return directory / (locator.removeprefix("sha256:") + ".json")


def _runtime_capsule_directory(root: Path, key: object) -> Path | None:
    if not valid_runtime_key(key):
        return None
    capsule = root / "runtimes" / str(key).removeprefix("sha256:")
    try:
        metadata = capsule.lstat()
    except FileNotFoundError:
        return None
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        return None
    return capsule


def read_locator(path: Path, expected: str, root: Path | None = None) -> str | None:
    if not path.exists():
        return None
    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode) or metadata.st_size > 4096:
        raise BootstrapError("runtime locator is unsafe")
    value = json.loads(path.read_bytes())
    if (
        value.get("schema") != "codeclew-runtime-locator/2.0"
        or value.get("locatorKey") != expected
        or not isinstance(value.get("runtimeKey"), str)
        or not value["runtimeKey"].startswith("sha256:")
    ):
        raise BootstrapError("runtime locator authority mismatch")
    key = value["runtimeKey"]
    if not valid_runtime_key(key):
        raise BootstrapError("runtime locator authority mismatch")
    if root is not None and _runtime_capsule_directory(root, key) is None:
        return None
    return key


def write_locator(path: Path, locator: str, runtime: str) -> None:
    temporary = path.parent / f".locator-{uuid.uuid4().hex}"
    descriptor = os.open(temporary, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    try:
        with os.fdopen(descriptor, "wb", closefd=False) as stream:
            stream.write(canonical({
                "schema": "codeclew-runtime-locator/2.0",
                "locatorKey": locator,
                "runtimeKey": runtime,
            }) + b"\n")
            stream.flush()
            os.fsync(stream.fileno())
    finally:
        os.close(descriptor)
    os.replace(temporary, path)


def checkpoint_path(root: Path, source: Path) -> Path:
    metadata = source.lstat()
    identity = canonical({
        "path": str(source),
        "device": metadata.st_dev,
        "inode": metadata.st_ino,
    })
    name = hashlib.sha256(DOMAIN + b"checkpoint\0" + identity).hexdigest() + ".json"
    return root / "runtimes" / "checkpoints" / name


def _runtime_discovery_directories(source: Path) -> list[Path]:
    directories = {source}
    roots = [
        source / ".cargo",
        source / "bootstrap",
        source / "schemas",
        source / "gradle" / "wrapper",
        source / "crates",
        source / "workers",
    ]
    for root in roots:
        if not root.is_dir() or root.is_symlink():
            continue
        directories.add(root)
        for current, names, _files in os.walk(root, followlinks=False):
            current_path = Path(current)
            names[:] = [
                name for name in names
                if name != ".semantic-thread"
                and name != "target"
                and not (
                    current_path.is_relative_to(source / "crates")
                    and name in {"tests", "examples"}
                )
            ]
            directories.add(current_path)
    return sorted(directories, key=str)


def _git_control_paths(source: Path) -> list[Path]:
    git_directory = Path(
        run(["git", "rev-parse", "--absolute-git-dir"], source).decode().strip()
    )
    common_value = run(["git", "rev-parse", "--git-common-dir"], source).decode().strip()
    common_directory = Path(common_value)
    if not common_directory.is_absolute():
        common_directory = (source / common_directory).resolve(strict=False)
    paths = {source / ".git", git_directory, common_directory}
    for directory in {git_directory, common_directory}:
        for relative in [
            "HEAD", "index", "commondir", "packed-refs", "config", "config.worktree",
            "shallow", "info", "info/exclude", "refs",
        ]:
            paths.add(directory / relative)
        refs = directory / "refs"
        if refs.is_dir() and not refs.is_symlink():
            for current, names, files in os.walk(refs, followlinks=False):
                current_path = Path(current)
                paths.add(current_path)
                paths.update(current_path / name for name in names)
                paths.update(current_path / name for name in files)
    home = Path(os.environ.get("HOME", str(Path.home())))
    xdg = Path(os.environ.get("XDG_CONFIG_HOME", str(home / ".config")))
    candidates = [
        home / ".gitconfig",
        xdg / "git" / "config",
        Path("/etc/gitconfig"),
    ]
    for name in ["GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM"]:
        if os.environ.get(name):
            candidates.append(Path(os.environ[name]))
    paths.update(candidate for candidate in candidates if candidate.is_absolute())
    return sorted(paths, key=str)


def _toolchain_checkpoint_paths(fast_tools: dict[str, object]) -> list[Path]:
    paths = [
        Path(str(fast_tools["python"]["path"])),
        Path(str(fast_tools["jdkRelease"]["path"])),
    ]
    paths.extend(Path(str(value["path"])) for value in fast_tools["executables"].values())
    java = Path(str(fast_tools["executables"]["java"]["path"]))
    paths.append(java.parent.parent / "lib" / "modules")
    return sorted(set(paths), key=str)


def write_checkpoint(
    path: Path,
    source: Path,
    capsule: Path,
    key: str,
    mode: str,
    inputs: list[dict[str, object]],
    fast_tools: dict[str, object],
) -> None:
    discovery_directories = _runtime_discovery_directories(source)
    source_paths = set(discovery_directories)
    source_paths.update(directory / ".gitignore" for directory in discovery_directories)
    for row in inputs:
        value = source / str(row["path"])
        source_paths.add(value)
        parent = value.parent
        while parent != source and parent.is_relative_to(source):
            source_paths.add(parent)
            parent = parent.parent
    source_paths.update(_git_control_paths(source))
    source_paths.update(_toolchain_checkpoint_paths(fast_tools))
    rust_sysroot = Path(
        run(["rustc", "--print", "sysroot"], source, sanitized_environment())
        .decode()
        .strip()
    )
    if not rust_sysroot.is_absolute():
        raise BootstrapError("Rust sysroot authority is not absolute")
    source_paths.update([rust_sysroot / "bin" / "rustc", rust_sysroot / "bin" / "cargo"])
    capsule_paths = [capsule, *capsule.rglob("*")]
    payload = {
        "schema": "codeclew-runtime-checkpoint/3.0",
        "source": _metadata_identity(source),
        "environment": _environment_checkpoint(),
        "runtimeKey": key,
        "mode": mode,
        "inputs": inputs,
        "capsule": str(capsule),
        "sourceNodes": [_metadata_identity(value) for value in sorted(source_paths, key=str)],
        "capsuleNodes": [_metadata_identity(value) for value in sorted(capsule_paths, key=str)],
    }
    encoded = canonical(payload) + b"\n"
    if len(encoded) > MAX_CHECKPOINT_BYTES:
        raise BootstrapError("runtime metadata checkpoint exceeds its bounded size")
    temporary = path.parent / f".checkpoint-{uuid.uuid4().hex}"
    descriptor = os.open(temporary, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    try:
        with os.fdopen(descriptor, "wb", closefd=False) as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
    finally:
        os.close(descriptor)
    os.replace(temporary, path)
    os.chmod(path, 0o600)


def read_valid_checkpoint(path: Path, source: Path, root: Path) -> dict[str, object] | None:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        _AUDIT_COUNTERS["checkpointMisses"] += 1
        return None
    if (
        stat.S_ISLNK(metadata.st_mode)
        or not stat.S_ISREG(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o600
        or metadata.st_uid != os.geteuid()
        or metadata.st_size > MAX_CHECKPOINT_BYTES
    ):
        raise BootstrapError("runtime metadata checkpoint is unsafe")
    try:
        value = json.loads(path.read_bytes())
    except (OSError, ValueError, TypeError):
        _AUDIT_COUNTERS["checkpointMisses"] += 1
        return None
    source_nodes = value.get("sourceNodes")
    capsule_nodes = value.get("capsuleNodes")
    key = value.get("runtimeKey")
    if not valid_runtime_key(key):
        _AUDIT_COUNTERS["checkpointMisses"] += 1
        return None
    capsule = root / "runtimes" / str(key).removeprefix("sha256:")
    valid_shape = (
        value.get("schema") == "codeclew-runtime-checkpoint/3.0"
        and value.get("environment") == _environment_checkpoint()
        and value.get("capsule") == str(capsule)
        and isinstance(value.get("inputs"), list)
        and value.get("mode") in {"RELEASE", "DEVELOPMENT"}
        and isinstance(source_nodes, list)
        and isinstance(capsule_nodes, list)
        and 0 < len(source_nodes) <= MAX_CHECKPOINT_NODES
        and 0 < len(capsule_nodes) <= MAX_CHECKPOINT_NODES
        and all(isinstance(row, dict) for row in source_nodes)
        and all(isinstance(row, dict) for row in capsule_nodes)
        and value.get("source") == _metadata_identity(source)
    )
    if valid_shape and all(_metadata_matches(row) for row in [*source_nodes, *capsule_nodes]):
        _AUDIT_COUNTERS["checkpointHits"] += 1
        return value
    _AUDIT_COUNTERS["checkpointMisses"] += 1
    return None


def read_checkpoint_candidate_key(path: Path, root: Path) -> str | None:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return None
    if (
        stat.S_ISLNK(metadata.st_mode)
        or not stat.S_ISREG(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o600
        or metadata.st_uid != os.geteuid()
        or metadata.st_size > MAX_CHECKPOINT_BYTES
    ):
        raise BootstrapError("runtime metadata checkpoint is unsafe")
    try:
        value = json.loads(path.read_bytes())
    except (OSError, ValueError, TypeError):
        return None
    if value.get("schema") != "codeclew-runtime-checkpoint/3.0":
        return None
    key = value.get("runtimeKey")
    if not valid_runtime_key(key):
        return None
    capsule = root / "runtimes" / str(key).removeprefix("sha256:")
    if value.get("capsule") != str(capsule):
        return None
    if _runtime_capsule_directory(root, key) is None:
        return None
    return str(key)


def runtime_key(mode: str, inputs: list[dict[str, object]], tools: dict[str, object]) -> str:
    digest = hashlib.sha256()
    digest.update(DOMAIN)
    digest.update(mode.encode())
    digest.update(b"\0")
    digest.update(canonical({"inputs": inputs, "toolchains": tools}))
    return "sha256:" + digest.hexdigest()


def stage_inputs(source: Path, destination: Path, rows: list[dict[str, object]]) -> None:
    destination.mkdir(mode=0o700)

    def copy(row: dict[str, object]) -> None:
        relative = Path(str(row["path"]))
        target = destination / relative
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        shutil.copyfile(source / relative, target, follow_symlinks=False)
        os.chmod(target, 0o755 if row["mode"] else 0o600)

    with ThreadPoolExecutor(max_workers=min(8, max(1, os.cpu_count() or 1))) as executor:
        list(executor.map(copy, rows))
    verify_source_manifest(destination, rows, full_closure=False)


def build_environment(stage: Path, root: Path) -> dict[str, str]:
    environment = sanitized_environment()
    cargo_target = stage.parent / "cargo-target"
    cargo_target.mkdir(mode=0o700)
    environment["CARGO_TARGET_DIR"] = str(cargo_target)
    environment["CARGO_INCREMENTAL"] = "0"
    environment["GIT_TERMINAL_PROMPT"] = "0"
    gradle_home = Path(environment.get("GRADLE_USER_HOME", str(Path.home() / ".gradle")))
    for relative in ["init.gradle", "init.gradle.kts", "init.d"]:
        if (gradle_home / relative).exists():
            raise BootstrapError("Gradle init injection is unsupported for trusted capsule builds")
    return environment


def file_rows(root: Path) -> list[dict[str, object]]:
    rows = []
    for path in sorted(path for path in root.rglob("*") if path.is_file()):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
            raise BootstrapError("capsule output contains an unsafe file")
        rows.append({
            "path": path.relative_to(root).as_posix(),
            "size": metadata.st_size,
            "sha256": digest_file(path),
        })
    return rows


def tree_hash(rows: list[dict[str, object]]) -> str:
    digest = hashlib.sha256()
    for row in rows:
        digest.update(str(row["path"]).encode())
        digest.update(b"\0")
        digest.update(str(row["size"]).encode())
        digest.update(b"\0")
        digest.update(str(row["sha256"]).encode())
        digest.update(b"\0")
    return "sha256:" + digest.hexdigest()


def bootstrap_self_test() -> None:
    rows = [
        {"path": "bin/a", "size": 3, "sha256": "sha256:" + "0" * 64},
        {"path": "lib/b", "size": 5, "sha256": "sha256:" + "1" * 64},
    ]
    assert tree_hash(rows) == "sha256:17991e194c0c77b4a7ff59263df0339e2a26c7e8bc5556e11a3afeb2510c6177"
    first = locator_key("RELEASE", rows, {"tool": "a"})
    assert first == locator_key("RELEASE", rows, {"tool": "a"})
    assert first != locator_key("DEVELOPMENT", rows, {"tool": "a"})
    assert first != locator_key("RELEASE", rows, {"tool": "b"})
    assert runtime_build_plan(8, 32 * 1024**3)["parallel"] is True
    assert runtime_build_plan(8, 4 * 1024**3)["parallel"] is False
    assert warm_audit_payload(False, False)["status"] == "PASSED"
    assert warm_audit_payload(True, False)["status"] == "COLD_MISS"
    assert warm_audit_payload(False, True)["status"] == "COLD_MISS"
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        staged = root / "staged"
        destination = root / "published"
        (staged / "bin").mkdir(parents=True)
        executable = staged / "bin" / "clew"
        executable.write_bytes(b"binary")
        os.chmod(executable, 0o700)
        os.replace(staged, destination)
        seal_capsule(destination)
        verify_sealed_capsule(destination)
        (root / "quarantine").mkdir(mode=0o700)
        published_executable = destination / "bin" / "clew"
        os.chmod(published_executable, 0o700)
        with published_executable.open("ab") as stream:
            stream.write(b"corrupt")
        quarantine(root, destination, "SelfTestCorruption")
        assert not destination.exists()
        quarantined = [path for path in (root / "quarantine").iterdir() if path.is_dir()]
        assert len(quarantined) == 1
        metadata = quarantined[0].with_suffix(".json")
        assert metadata.is_file() and stat.S_IMODE(metadata.stat().st_mode) == 0o600


def warm_audit_payload(toolchain_invoked: bool, capsule_build_invoked: bool) -> dict[str, object]:
    return {
        "schema": "codeclew-bootstrap-warm-audit/2.0",
        "status": "PASSED" if not toolchain_invoked and not capsule_build_invoked else "COLD_MISS",
        "coldToolchainInvoked": toolchain_invoked,
        "capsuleBuildInvoked": capsule_build_invoked,
        "counters": dict(_AUDIT_COUNTERS),
        "forbiddenWarmProcesses": ["cargo", "rustc", "gradle", "maven"],
    }


def verify_release_worker(stage: Path, manifest_relative: str, rows: list[dict[str, object]]) -> None:
    manifest = json.loads((stage / manifest_relative).read_text())
    expected = manifest.get("files")
    if expected != rows or manifest.get("treeHash") != tree_hash(rows):
        raise BootstrapError(
            "RELEASE worker differs from its committed manifest; regenerate and verify all "
            f"affected worker variants: {manifest_relative}"
        )


def runtime_build_plan(cpu_count: int, total_memory_bytes: int) -> dict[str, object]:
    cpu_count = max(1, cpu_count)
    reserved = max(1024**3, total_memory_bytes * 15 // 100)
    memory_budget = max(0, total_memory_bytes - reserved) * 70 // 100
    cargo_memory = 2 * 1024**3
    gradle_memory = 3 * 1024**3
    parallel = cpu_count >= 2 and memory_budget >= cargo_memory + gradle_memory
    if parallel:
        cargo_workers = max(1, cpu_count // 2)
        gradle_workers = max(1, cpu_count - cargo_workers)
    else:
        cargo_workers = cpu_count
        gradle_workers = cpu_count
    return {
        "parallel": parallel,
        "cargoWorkers": cargo_workers,
        "gradleWorkers": gradle_workers,
        "memoryBudgetBytes": memory_budget,
    }


def host_memory_bytes() -> int:
    try:
        pages = os.sysconf("SC_PHYS_PAGES")
        page_size = os.sysconf("SC_PAGE_SIZE")
        if pages > 0 and page_size > 0:
            return int(pages * page_size)
    except (AttributeError, OSError, ValueError):
        pass
    raise BootstrapError("host memory authority is unavailable for capsule admission")


def build_toolchains(stage: Path, environment: dict[str, str]) -> dict[str, object]:
    plan = runtime_build_plan(os.cpu_count() or 1, host_memory_bytes())
    gradle = [
        str(stage / "gradlew"),
        ":workers:kotlin21:installDist",
        ":workers:kotlin23:installDist",
        ":workers:kotlin:installDist",
        "--no-daemon",
        "--parallel",
        "--build-cache",
        f"--max-workers={plan['gradleWorkers']}",
        "--quiet",
    ]
    cargo = [
        "cargo", "build", "--locked", "--release", "-p", "clew",
        "--bin", "clew", "--bin", "semanticd", "--jobs", str(plan["cargoWorkers"]),
    ]
    supervisor = BuildProcessSupervisor()
    max_workers = 2 if plan["parallel"] else 1
    with build_signal_scope(supervisor):
        try:
            with ThreadPoolExecutor(max_workers=max_workers) as executor:
                try:
                    if plan["parallel"]:
                        futures = [
                            executor.submit(
                                run_build_stage, gradle, stage, environment,
                                "GRADLE_WORKERS", supervisor,
                            ),
                            executor.submit(
                                run_build_stage, cargo, stage, environment,
                                "CARGO_BINARIES", supervisor,
                            ),
                        ]
                        for future in futures:
                            future.result()
                    else:
                        executor.submit(
                            run_build_stage, gradle, stage, environment,
                            "GRADLE_WORKERS", supervisor,
                        ).result()
                        executor.submit(
                            run_build_stage, cargo, stage, environment,
                            "CARGO_BINARIES", supervisor,
                        ).result()
                except BaseException:
                    # Cancel before ThreadPoolExecutor.__exit__ waits for the
                    # sibling stage, otherwise a failed Gradle task could leave
                    # Cargo running until its natural completion (or vice versa).
                    supervisor.cancel()
                    raise
        except BaseException:
            supervisor.cancel()
            raise
    return plan


def package_worker(
    stage: Path,
    capsule: Path,
    mode: str,
    item: tuple[str, tuple[str, str, str]],
) -> tuple[str, dict[str, object]]:
    name, (compiler, distribution, manifest) = item
    source_distribution = stage / distribution
    destination = capsule / distribution
    shutil.copytree(source_distribution, destination, symlinks=False)
    rows = file_rows(destination)
    if mode == "RELEASE":
        verify_release_worker(stage, manifest, rows)
    return name, {
        "compilerVersion": compiler,
        "protocol": "semantic-thread.worker.v1",
        "distribution": distribution,
        "treeHash": tree_hash(rows),
        "files": rows,
    }


def seal_capsule(capsule: Path, *, seal_root: bool = True) -> None:
    for path in sorted(capsule.rglob("*"), key=lambda value: len(value.parts), reverse=True):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise BootstrapError("capsule output contains a symlink")
        if stat.S_ISREG(metadata.st_mode):
            os.chmod(path, 0o500 if metadata.st_mode & 0o111 else 0o400)
        elif stat.S_ISDIR(metadata.st_mode):
            os.chmod(path, 0o500)
        else:
            raise BootstrapError("capsule output contains an unsupported filesystem entry")
    os.chmod(capsule, 0o500 if seal_root else 0o700)


def fsync_tree(root: Path) -> None:
    files = [path for path in root.rglob("*") if path.is_file()]
    directories = [root, *(path for path in root.rglob("*") if path.is_dir())]
    for path in files:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    for path in sorted(directories, key=lambda value: len(value.parts), reverse=True):
        descriptor = os.open(path, _directory_flags())
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)


def discard_private_tree(root: Path) -> None:
    if not root.exists():
        return
    for path in sorted(
        (entry for entry in root.rglob("*") if entry.is_dir()),
        key=lambda value: len(value.parts),
        reverse=True,
    ):
        os.chmod(path, 0o700)
    os.chmod(root, 0o700)
    shutil.rmtree(root)


def verify_sealed_capsule(capsule: Path) -> None:
    for path in [capsule, *capsule.rglob("*")]:
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise BootstrapError("sealed capsule contains a symlink")
        if stat.S_ISDIR(metadata.st_mode):
            if metadata.st_mode & 0o277:
                raise BootstrapError("capsule directory is not sealed read-only")
        elif stat.S_ISREG(metadata.st_mode):
            if metadata.st_mode & 0o277:
                raise BootstrapError("capsule file is not sealed read-only")
        else:
            raise BootstrapError("sealed capsule contains an unsupported entry")


def build_capsule(source: Path, root: Path, key: str, mode: str, inputs: list[dict[str, object]], tools: dict[str, object]) -> Path:
    temporary = Path(tempfile.mkdtemp(prefix="capsule-build-", dir=root / "tmp"))
    stage = temporary / "source"
    capsule = temporary / "capsule"
    try:
        stage_inputs(source, stage, inputs)
        environment = build_environment(stage, root)
        build_cache_lock = root / "locks/build-cache.lock"
        with build_cache_lock.open("a+b") as lock:
            os.chmod(build_cache_lock, 0o600)
            fcntl.flock(lock, fcntl.LOCK_EX)
            build_toolchains(stage, environment)
        verify_source_manifest(stage, inputs, full_closure=False)
        (capsule / "bin").mkdir(mode=0o700, parents=True)
        cargo_target = Path(environment["CARGO_TARGET_DIR"]) / "release"
        for name in ["clew", "semanticd"]:
            shutil.copy2(cargo_target / name, capsule / "bin" / name)
            os.chmod(capsule / "bin" / name, 0o500)
        progress("STAGE_STARTED", "PACKAGE_AND_HASH_WORKERS")
        with ThreadPoolExecutor(max_workers=len(WORKERS)) as executor:
            packaged = executor.map(
                lambda item: package_worker(stage, capsule, mode, item),
                WORKERS.items(),
            )
            workers = dict(packaged)
        progress("STAGE_COMPLETED", "PACKAGE_AND_HASH_WORKERS")
        artifacts = {}
        for name in ["clew", "semanticd"]:
            path = capsule / "bin" / name
            artifacts[name] = {"path": f"bin/{name}", "size": path.stat().st_size, "sha256": digest_file(path)}
        manifest = {
            "schema": SCHEMA,
            "runtimeKey": key,
            "mode": mode,
            "manifestDigest": "",
            "inputDigest": digest_bytes(canonical(inputs)),
            "platformAuthority": tools["platform"],
            "toolchainAuthority": {
                "python": tools["python"],
                "rust": tools["rust"],
                "jdk": tools["jdk"],
            },
            "artifacts": artifacts,
            "workers": workers,
        }
        manifest["manifestDigest"] = digest_bytes(canonical(manifest))
        runtime_manifest = capsule / "runtime.json"
        runtime_manifest.write_bytes(canonical(manifest) + b"\n")
        verify_source_manifest(
            source,
            inputs,
            expected_development=mode == "DEVELOPMENT",
        )
        verify_capsule(capsule, key, require_sealed=False, require_ready=False)
        ready = capsule / "READY"
        ready.write_text(key + "\n")
        # macOS refuses to rename a directory whose own mode is 0500. Seal every
        # descendant first, retain a private 0700 staging root for the rename,
        # then seal that root before releasing the per-key lock.
        seal_capsule(capsule, seal_root=False)
        fsync_tree(capsule)
        verify_capsule(capsule, key, require_sealed=False)
        destination = root / "runtimes" / key.removeprefix("sha256:")
        if destination.exists():
            return destination
        os.rename(capsule, destination)
        try:
            os.chmod(destination, 0o500)
            fsync_tree(destination)
            verify_capsule(destination, key)
            runtimes_fd = os.open(root / "runtimes", _directory_flags())
            try:
                os.fsync(runtimes_fd)
            finally:
                os.close(runtimes_fd)
        except Exception as error:
            quarantine(root, destination, type(error).__name__)
            raise BootstrapError("published capsule could not be sealed") from error
        return destination
    finally:
        discard_private_tree(temporary)


def verify_capsule(
    path: Path,
    key: str,
    *,
    require_sealed: bool = True,
    require_ready: bool = True,
) -> dict[str, object]:
    manifest_path = path / "runtime.json"
    if not manifest_path.is_file() or manifest_path.stat().st_size > MAX_MANIFEST_BYTES:
        raise BootstrapError("runtime manifest is unavailable")
    manifest = json.loads(manifest_path.read_bytes())
    expected = manifest.get("manifestDigest")
    manifest["manifestDigest"] = ""
    if (
        manifest.get("schema") != SCHEMA
        or expected != digest_bytes(canonical(manifest))
        or manifest.get("runtimeKey") != key
        or not isinstance(manifest.get("platformAuthority"), dict)
        or not isinstance(manifest.get("toolchainAuthority"), dict)
    ):
        raise BootstrapError("runtime manifest authority mismatch")
    manifest["manifestDigest"] = expected
    for artifact in manifest.get("artifacts", {}).values():
        target = path / artifact["path"]
        if (
            not target.is_file()
            or target.is_symlink()
            or target.stat().st_size != artifact["size"]
            or digest_file(target) != artifact["sha256"]
        ):
            raise BootstrapError("runtime executable authority mismatch")
    for worker in manifest.get("workers", {}).values():
        if worker.get("protocol") != "semantic-thread.worker.v1":
            raise BootstrapError("runtime worker protocol authority mismatch")
        distribution = path / worker["distribution"]
        rows = file_rows(distribution)
        if rows != worker["files"] or tree_hash(rows) != worker["treeHash"]:
            raise BootstrapError("runtime worker authority mismatch")
    expected_files = {"runtime.json"}
    expected_files.update(
        str(value["path"]) for value in manifest.get("artifacts", {}).values()
    )
    for worker in manifest.get("workers", {}).values():
        distribution = Path(str(worker["distribution"]))
        expected_files.update(
            (distribution / str(row["path"])).as_posix()
            for row in worker["files"]
        )
    if require_ready:
        expected_files.add("READY")
    observed_files = {
        value.relative_to(path).as_posix()
        for value in path.rglob("*")
        if value.is_file() and not value.is_symlink()
    }
    if observed_files != expected_files:
        raise BootstrapError("runtime capsule closure mismatch")
    expected_directories = {"."}
    for relative in expected_files:
        parent = Path(relative).parent
        while parent != Path("."):
            expected_directories.add(parent.as_posix())
            parent = parent.parent
    observed_directories = {"."}
    observed_directories.update(
        value.relative_to(path).as_posix()
        for value in path.rglob("*")
        if value.is_dir() and not value.is_symlink()
    )
    if observed_directories != expected_directories:
        raise BootstrapError("runtime capsule directory closure mismatch")
    if require_ready:
        ready = path / "READY"
        if not ready.is_file() or ready.is_symlink() or ready.read_text() != key + "\n":
            raise BootstrapError("runtime capsule is not ready")
    if require_sealed:
        verify_sealed_capsule(path)
    return manifest


def quarantine(root: Path, capsule: Path, reason: str) -> None:
    try:
        metadata = capsule.lstat()
    except FileNotFoundError:
        return
    reason_token = reason if reason.isascii() and reason.replace("_", "").isalnum() else "VerificationFailure"
    reason_token = reason_token[:64]
    directory = root / "quarantine"
    destination = directory / f"{capsule.name}-{uuid.uuid4().hex}"
    try:
        if stat.S_ISDIR(metadata.st_mode) and not stat.S_ISLNK(metadata.st_mode):
            # macOS refuses to rename a sealed directory whose own mode is 0500.
            # Only the capsule root is relaxed; quarantined contents stay sealed.
            os.chmod(capsule, 0o700)
        os.replace(capsule, destination)
        metadata_path = directory / f"{destination.name}.json"
        temporary = directory / f".quarantine-{uuid.uuid4().hex}"
        descriptor = os.open(temporary, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        try:
            with os.fdopen(descriptor, "wb", closefd=False) as stream:
                stream.write(canonical({
                    "schema": "codeclew-runtime-quarantine/2.0",
                    "reason": reason_token,
                }) + b"\n")
                stream.flush()
                os.fsync(stream.fileno())
        finally:
            os.close(descriptor)
        os.replace(temporary, metadata_path)
    except OSError as error:
        raise BootstrapError("unsafe runtime capsule could not be quarantined") from error


def _open_gc_lock(locks_fd: int, name: str):
    descriptor = os.open(
        name,
        os.O_CREAT | os.O_RDWR | getattr(os, "O_NOFOLLOW", 0),
        0o600,
        dir_fd=locks_fd,
    )
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid():
            raise BootstrapError("runtime GC lock authority is unsafe")
        os.fchmod(descriptor, 0o600)
        return os.fdopen(descriptor, "a+b")
    except Exception:
        os.close(descriptor)
        raise


def _remove_tree_at(parent_fd: int, name: str, expected: os.stat_result) -> bool:
    try:
        descriptor = os.open(name, _directory_flags(), dir_fd=parent_fd)
    except (FileNotFoundError, NotADirectoryError, OSError):
        return False
    try:
        observed = os.fstat(descriptor)
        if (observed.st_dev, observed.st_ino) != (expected.st_dev, expected.st_ino):
            return False
        os.fchmod(descriptor, 0o700)
        with os.scandir(descriptor) as entries:
            children = list(entries)
        for entry in children:
            metadata = entry.stat(follow_symlinks=False)
            if stat.S_ISDIR(metadata.st_mode) and not stat.S_ISLNK(metadata.st_mode):
                if not _remove_tree_at(descriptor, entry.name, metadata):
                    return False
            else:
                os.unlink(entry.name, dir_fd=descriptor)
    finally:
        os.close(descriptor)
    os.rmdir(name, dir_fd=parent_fd)
    return True


def _cleanup_runtime_records(root: Path, runtime_key_value: str) -> None:
    for relative, maximum in [
        ("locators", 4096),
        ("checkpoints", MAX_CHECKPOINT_BYTES),
    ]:
        directory = root / "runtimes" / relative
        try:
            descriptor = os.open(directory, _directory_flags())
        except FileNotFoundError:
            continue
        try:
            with os.scandir(descriptor) as entries:
                records = list(entries)
            for entry in records:
                try:
                    metadata = entry.stat(follow_symlinks=False)
                    if (
                        stat.S_ISLNK(metadata.st_mode)
                        or not stat.S_ISREG(metadata.st_mode)
                        or metadata.st_size > maximum
                    ):
                        continue
                    record_fd = os.open(
                        entry.name,
                        os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
                        dir_fd=descriptor,
                    )
                    try:
                        with os.fdopen(record_fd, "rb") as stream:
                            value = json.load(stream)
                    except (OSError, ValueError, TypeError):
                        continue
                    if isinstance(value, dict) and value.get("runtimeKey") == runtime_key_value:
                        os.unlink(entry.name, dir_fd=descriptor)
                except FileNotFoundError:
                    continue
        finally:
            os.close(descriptor)


def _session_lifecycle_is_collected(
    session_fd: int,
    session_id: str,
    authority_digest: object,
) -> bool:
    """Return true only for one exact, self-authenticating terminal projection.

    Any missing, malformed, stale, or replaced lifecycle file retains the
    runtime. GC safety is more important than reclaiming a questionable root.
    """
    if not valid_runtime_key(authority_digest):
        return False
    lifecycle_fd = -1
    try:
        lifecycle_fd = os.open(
            "lifecycle.json",
            os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
            dir_fd=session_fd,
        )
        metadata = os.fstat(lifecycle_fd)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or metadata.st_size > MAX_MANIFEST_BYTES
        ):
            return False
        with os.fdopen(lifecycle_fd, "rb") as stream:
            lifecycle_fd = -1
            payload = stream.read(MAX_MANIFEST_BYTES + 1)
        value = json.loads(payload)
        if not isinstance(value, dict) or canonical(value) != payload:
            return False
        if (
            value.get("schema") != "codeclew-session-lifecycle-entry/1.0"
            or value.get("sessionId") != session_id
            or value.get("sessionAuthorityDigest") != authority_digest
            or value.get("status") != "GARBAGE_COLLECTED"
            or not isinstance(value.get("sequence"), int)
            or value.get("sequence", -1) < 2
            or not valid_runtime_key(value.get("eventHash"))
        ):
            return False
        unsigned = dict(value)
        expected = unsigned["eventHash"]
        unsigned["eventHash"] = ""
        return digest_bytes(canonical(unsigned)) == expected
    except (FileNotFoundError, OSError, ValueError, TypeError):
        return False
    finally:
        if lifecycle_fd >= 0:
            os.close(lifecycle_fd)


def _session_runtime_roots(root: Path) -> set[str]:
    directory = root / "sessions"
    try:
        sessions_fd = os.open(directory, _directory_flags())
    except FileNotFoundError:
        return set()
    roots: set[str] = set()
    try:
        with os.scandir(sessions_fd) as entries:
            sessions = list(entries)
        for entry in sessions:
            try:
                metadata = entry.stat(follow_symlinks=False)
                if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
                    continue
                session_fd = os.open(entry.name, _directory_flags(), dir_fd=sessions_fd)
                try:
                    authority_fd = os.open(
                        "authority.json",
                        os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
                        dir_fd=session_fd,
                    )
                    try:
                        authority_metadata = os.fstat(authority_fd)
                        if (
                            not stat.S_ISREG(authority_metadata.st_mode)
                            or authority_metadata.st_uid != os.geteuid()
                            or authority_metadata.st_size > MAX_MANIFEST_BYTES
                        ):
                            continue
                        with os.fdopen(authority_fd, "rb") as stream:
                            authority_fd = -1
                            value = json.load(stream)
                    finally:
                        if authority_fd >= 0:
                            os.close(authority_fd)
                    if not isinstance(value, dict) or not valid_runtime_key(value.get("runtimeKey")):
                        continue
                    session_id = f"session:{entry.name}"
                    # A valid terminal lifecycle is the only authority that
                    # may release a session root. Every questionable record
                    # fails open by retaining the referenced runtime.
                    if not (
                        value.get("schema") == "codeclew-session/3.0"
                        and value.get("sessionId") == session_id
                        and _session_lifecycle_is_collected(
                            session_fd,
                            session_id,
                            value.get("authorityDigest"),
                        )
                    ):
                        roots.add(str(value["runtimeKey"]).removeprefix("sha256:"))
                finally:
                    os.close(session_fd)
            except (FileNotFoundError, OSError, ValueError, TypeError):
                continue
        return roots
    finally:
        os.close(sessions_fd)


def garbage_collect_runtime_capsules(
    root: Path,
    current_key: str,
    *,
    keep_newest: int = 2,
) -> list[str]:
    """Remove unreachable old capsules while every live runtime retains a lease."""
    if not valid_runtime_key(current_key):
        raise BootstrapError("runtime GC current key is invalid")
    current_name = current_key.removeprefix("sha256:")
    runtimes_fd = os.open(root / "runtimes", _directory_flags())
    locks_fd = os.open(root / "locks", _directory_flags())
    removed: list[str] = []
    try:
        candidates: list[tuple[int, str, os.stat_result]] = []
        with os.scandir(runtimes_fd) as entries:
            runtime_entries = list(entries)
        for entry in runtime_entries:
            key = "sha256:" + entry.name
            if not valid_runtime_key(key):
                continue
            try:
                metadata = entry.stat(follow_symlinks=False)
            except OSError:
                continue
            if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
                continue
            candidates.append((metadata.st_mtime_ns, entry.name, metadata))
        newest = sorted(
            (candidate for candidate in candidates if candidate[1] != current_name),
            key=lambda candidate: (candidate[0], candidate[1]),
            reverse=True,
        )[:max(0, keep_newest)]
        retained = {
            current_name,
            *(candidate[1] for candidate in newest),
            *_session_runtime_roots(root),
        }
        for _modified, name, metadata in candidates:
            if name in retained:
                continue
            key = "sha256:" + name
            try:
                with _open_gc_lock(locks_fd, f"runtime-{name}.lock") as build_lock:
                    try:
                        fcntl.flock(build_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    except BlockingIOError:
                        continue
                    with _open_gc_lock(locks_fd, f"runtime-{name}.lease") as lease:
                        try:
                            fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        except BlockingIOError:
                            continue
                        if _remove_tree_at(runtimes_fd, name, metadata):
                            removed.append(key)
                            try:
                                _cleanup_runtime_records(root, key)
                            except (OSError, BootstrapError, ValueError, TypeError):
                                pass
            except (OSError, BootstrapError):
                continue
        return removed
    finally:
        os.close(locks_fd)
        os.close(runtimes_fd)


def main() -> int:
    reset_audit_counters()
    if sys.version_info < (3, 11):
        raise BootstrapError("Codeclew requires Python 3.11 or newer")
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--source-root", type=Path, required=True)
    known, command = parser.parse_known_args()
    if command == ["--bootstrap-self-test"]:
        bootstrap_self_test()
        print(canonical({"schema": "codeclew-bootstrap-self-test/1.0", "status": "PASSED"}).decode())
        return 0
    warm_audit = command == ["--bootstrap-warm-audit"]
    source = known.source_root.resolve(strict=True)
    root, state_fd = state_root()
    path_to_checkpoint = checkpoint_path(root, source)
    checkpoint_key = read_checkpoint_candidate_key(path_to_checkpoint, root)
    checkpoint = None
    lease = None
    cold_toolchain_invoked = False
    capsule_build_invoked = False
    if checkpoint_key is not None:
        checkpoint_lock = root / "locks" / (
            f"runtime-{checkpoint_key.removeprefix('sha256:')}.lock"
        )
        with checkpoint_lock.open("a+b") as lock:
            os.chmod(checkpoint_lock, 0o600)
            fcntl.flock(lock, fcntl.LOCK_EX)
            checkpoint = read_valid_checkpoint(path_to_checkpoint, source, root)
            if checkpoint is not None:
                key = str(checkpoint["runtimeKey"])
                capsule = Path(str(checkpoint["capsule"]))
                lease_path = root / "locks" / (
                    f"runtime-{key.removeprefix('sha256:')}.lease"
                )
                lease = lease_path.open("a+b")
                os.chmod(lease_path, 0o600)
                fcntl.flock(lease, fcntl.LOCK_SH)
    if checkpoint is None:
        inputs, development = source_manifest(source)
        mode = "DEVELOPMENT" if development else "RELEASE"
        fast_tools = fast_toolchain_locator_authority()
        locator = locator_key(mode, inputs, fast_tools)
        path_to_locator = locator_path(root, locator)
        key = read_locator(path_to_locator, locator, root)
        tools = None
        if key is None:
            tools = toolchain_authority(source)
            key = runtime_key(mode, inputs, tools)
        capsule = root / "runtimes" / key.removeprefix("sha256:")
        cold_toolchain_invoked = tools is not None
        lock_path = root / "locks" / f"runtime-{key.removeprefix('sha256:')}.lock"
        with lock_path.open("a+b") as lock:
            os.chmod(lock_path, 0o600)
            fcntl.flock(lock, fcntl.LOCK_EX)
            if capsule.exists():
                try:
                    verify_capsule(capsule, key)
                except Exception as error:
                    quarantine(root, capsule, type(error).__name__)
            if not capsule.exists():
                require_cold_build_capacity(root)
                if tools is None:
                    tools = toolchain_authority(source)
                    cold_toolchain_invoked = True
                    rebuilt_key = runtime_key(mode, inputs, tools)
                    if rebuilt_key != key:
                        raise BootstrapError(
                            "runtime locator disagrees with cold toolchain authority"
                        )
                capsule_build_invoked = True
                capsule = build_capsule(source, root, key, mode, inputs, tools)
            verify_capsule(capsule, key)
            write_locator(path_to_locator, locator, key)
            write_checkpoint(
                path_to_checkpoint, source, capsule, key, mode, inputs, fast_tools
            )
            lease_path = root / "locks" / (
                f"runtime-{key.removeprefix('sha256:')}.lease"
            )
            lease = lease_path.open("a+b")
            os.chmod(lease_path, 0o600)
            fcntl.flock(lease, fcntl.LOCK_SH)
    if lease is None:
        raise BootstrapError("runtime lease authority is unavailable")
    if checkpoint is None:
        garbage_collect_runtime_capsules(root, key)
    if warm_audit:
        lease.close()
        print(canonical(warm_audit_payload(cold_toolchain_invoked, capsule_build_invoked)).decode())
        return 0
    runtime_fd = os.open(capsule, _directory_flags())
    os.set_inheritable(state_fd, True)
    os.set_inheritable(runtime_fd, True)
    os.set_inheritable(lease.fileno(), True)
    environment = {name: value for name, value in os.environ.items() if not name.startswith("CODECLEW_")}
    environment[STATE_ROOT_FD_ENV] = str(state_fd)
    environment[RUNTIME_ROOT_FD_ENV] = str(runtime_fd)
    environment[RUNTIME_LEASE_FD_ENV] = str(lease.fileno())
    os.execve(capsule / "bin/clew", [str(capsule / "bin/clew"), *command], environment)
    return 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except BootstrapInterrupted as error:
        print(canonical({
            "schema": "codeclew-bootstrap-error/1.0",
            "error": str(error),
        }).decode(), file=sys.stderr)
        raise SystemExit(128 + error.signum)
    except BootstrapError as error:
        print(canonical({"schema": "codeclew-bootstrap-error/1.0", "error": str(error)}).decode(), file=sys.stderr)
        raise SystemExit(7)
    except Exception as error:
        print(canonical({
            "schema": "codeclew-bootstrap-error/2.0",
            "error": "unexpected bootstrap failure",
            "errorType": type(error).__name__,
        }).decode(), file=sys.stderr)
        raise SystemExit(7)
