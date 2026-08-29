#!/usr/bin/env python3
"""Build or reuse one immutable Codeclew runtime capsule, then execute it."""

from __future__ import annotations

import argparse
import contextlib
from concurrent.futures import ThreadPoolExecutor
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import uuid

try:
    from host_resources import HostResourceError, effective_host_resources
except ModuleNotFoundError as error:
    if error.name != "host_resources":
        raise
    resource_path = Path(__file__).resolve().with_name("host_resources.py")
    resource_spec = importlib.util.spec_from_file_location(
        "_codeclew_host_resources", resource_path
    )
    if resource_spec is None or resource_spec.loader is None:
        raise RuntimeError("host resource authority loader is unavailable") from error
    resource_module = importlib.util.module_from_spec(resource_spec)
    resource_spec.loader.exec_module(resource_module)
    HostResourceError = resource_module.HostResourceError
    effective_host_resources = resource_module.effective_host_resources


SCHEMA = "codeclew-runtime-capsule/4.0"
DOMAIN = b"codeclew-runtime/v2\0"
COMPONENT_SCHEMA = "codeclew-runtime-component/1.0"
COMPONENT_AUTHORITY_SCHEMA = "codeclew-runtime-component-authority/1.0"
COMPONENT_DOMAIN = b"codeclew-runtime-component/v1\0"
COMPONENT_REGISTRY_SCHEMA = "codeclew-runtime-component-registry/1.0"
RELEASE_SOURCE_SCHEMA = "codeclew-release-source/1.0"
MAX_MANIFEST_BYTES = 1024 * 1024
MAX_CHECKPOINT_BYTES = 16 * 1024 * 1024
MAX_CHECKPOINT_NODES = 100_000
MIN_COLD_BUILD_FREE_BYTES = 6 * 1024 * 1024 * 1024
GC_SESSION_SCHEMAS = {
    "codeclew-session/3.0",
    "codeclew-session/4.0",
    "codeclew-session/5.0",
}
GRADLE_MIN_HEAP_BYTES = 2 * 1024**3
GRADLE_MAX_HEAP_BYTES = 8 * 1024**3
GRADLE_NON_HEAP_BYTES = 2 * 1024**3
TOOLCHAIN_WORKER_MEMORY_BYTES = 1536 * 1024**2
BUILD_TERMINATION_GRACE_SECONDS = 2.0
BUILD_KILL_WAIT_SECONDS = 2.0
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
            stderr=subprocess.DEVNULL,
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
        if _process_group_exists(process.pid):
            progress(
                "STAGE_FAILED",
                stage,
                durationMillis=duration,
                exitCode=return_code,
                residualProcessGroup=True,
            )
            raise BootstrapError(
                f"capsule build stage left a residual process group: {stage}"
            )
        progress("STAGE_COMPLETED", stage, durationMillis=duration)
    except BaseException:
        supervisor.cancel()
        raise
    finally:
        if process is not None:
            supervisor.unregister(process)


def selected_source(relative: str, registry: dict[str, object]) -> bool:
    if ".semantic-thread" in Path(relative).parts:
        return False
    if relative in ROOT_FILES:
        return True
    if relative.startswith("bootstrap/"):
        return relative in {
            "bootstrap/clew_bootstrap.py",
            "bootstrap/host_resources.py",
            "bootstrap/runtime_components.json",
        }
    for component in registry["components"]:
        if relative in component["inputFiles"]:
            return True
        for root in [*component["inputRoots"], *component["optionalInputRoots"]]:
            if relative.startswith(root + "/"):
                return True
    return False


def source_manifest(source: Path) -> tuple[list[dict[str, object]], bool]:
    registry = load_component_registry(source)
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
    paths = sorted({
        row.decode()
        for row in [*tracked, *untracked]
        if row and selected_source(row.decode(), registry)
    })
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
            # Git's executable bit is boolean.  The process umask may materialize
            # the same tracked executable as 0700 or 0755, but that must not
            # create a different RELEASE authority.
            "mode": 0o111 if metadata.st_mode & 0o111 else 0,
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
    development = any(selected_source(path, registry) for path in dirty_paths)
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
            or (bool(metadata.st_mode & 0o111) != bool(row["mode"]))
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
        "runtimes/components",
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


def prepare_cold_build_capacity(root: Path, runtime_key_value: str) -> None:
    garbage_collect_runtime_capsules(root, runtime_key_value)
    require_cold_build_capacity(root)


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


def cleanup_session_id(command: list[str]) -> str | None:
    """Return the authority id only for runtime-agnostic session cleanup."""
    if len(command) < 4 or command[:2] not in (["session", "close"], ["session", "gc"]):
        return None
    values: list[str] = []
    for index, argument in enumerate(command[2:]):
        if argument == "--session" and index + 3 < len(command):
            values.append(command[index + 3])
        elif argument.startswith("--session="):
            values.append(argument.split("=", 1)[1])
    if len(values) != 1:
        return None
    session_id = values[0]
    component = session_id.removeprefix("session:")
    if (
        not session_id.startswith("session:")
        or not component
        or len(component) > 128
        or any(
            not (
                character.isascii()
                and (character.isalnum() or character in "-_")
            )
            for character in component
        )
    ):
        return None
    return session_id


def sealed_session_cleanup_runtime(
    root: Path, session_id: str
) -> tuple[str, Path, object]:
    """Lease the existing capsule bound to a close/gc session authority.

    Cleanup is deliberately independent of the current source digest: active
    sessions retain their capsule as a runtime-GC root, and the selected binary
    validates the complete session authority before changing lifecycle state.
    """
    component = session_id.removeprefix("session:")
    sessions_fd = os.open(root / "sessions", _directory_flags())
    session_fd = -1
    authority_fd = -1
    try:
        session_fd = os.open(component, _directory_flags(), dir_fd=sessions_fd)
        authority_fd = os.open(
            "authority.json",
            os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
            dir_fd=session_fd,
        )
        metadata = os.fstat(authority_fd)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_size > MAX_MANIFEST_BYTES
        ):
            raise BootstrapError("session cleanup authority is unsafe")
        with os.fdopen(authority_fd, "rb") as stream:
            authority_fd = -1
            payload = stream.read(MAX_MANIFEST_BYTES + 1)
        value = json.loads(payload)
        if (
            not isinstance(value, dict)
            or value.get("schema") not in GC_SESSION_SCHEMAS
            or value.get("sessionId") != session_id
            or not valid_runtime_key(value.get("authorityDigest"))
            or not valid_runtime_key(value.get("runtimeKey"))
            or value.get("runtimeMode") not in {"RELEASE", "DEVELOPMENT"}
        ):
            raise BootstrapError("session cleanup authority is invalid")
        key = str(value["runtimeKey"])
        capsule = _runtime_capsule_directory(root, key)
        if capsule is None:
            raise BootstrapError("session cleanup runtime capsule is unavailable")
        lease_path = root / "locks" / f"runtime-{key.removeprefix('sha256:')}.lease"
        lease = lease_path.open("a+b")
        try:
            os.chmod(lease_path, 0o600)
            fcntl.flock(lease, fcntl.LOCK_SH)
            manifest = verify_capsule(capsule, key)
            if manifest.get("mode") != value["runtimeMode"]:
                raise BootstrapError("session cleanup runtime authority mismatch")
        except Exception:
            lease.close()
            raise
        return key, capsule, lease
    except FileNotFoundError as error:
        raise BootstrapError("session cleanup authority is unavailable") from error
    finally:
        if authority_fd >= 0:
            os.close(authority_fd)
        if session_fd >= 0:
            os.close(session_fd)
        os.close(sessions_fd)


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


def _trusted_seed_lifecycle(root: Path) -> object:
    if (
        not root.is_absolute()
        or ".." in root.parts
        or root.resolve(strict=True) != root
    ):
        raise BootstrapError("trusted seed lifecycle root is unsafe")
    root_descriptor = os.open(root, _directory_flags())
    try:
        root_metadata = os.fstat(root_descriptor)
        if (
            not stat.S_ISDIR(root_metadata.st_mode)
            or root_metadata.st_uid != os.geteuid()
            or stat.S_IMODE(root_metadata.st_mode) != 0o700
        ):
            raise BootstrapError("trusted seed lifecycle root is unsafe")
        locks_descriptor = os.open("locks", _directory_flags(), dir_fd=root_descriptor)
    finally:
        os.close(root_descriptor)
    try:
        locks_metadata = os.fstat(locks_descriptor)
        if (
            not stat.S_ISDIR(locks_metadata.st_mode)
            or locks_metadata.st_uid != os.geteuid()
            or stat.S_IMODE(locks_metadata.st_mode) != 0o700
        ):
            raise BootstrapError("trusted seed lifecycle locks are unsafe")
        descriptor = os.open(
            "lifecycle.lock",
            os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
            dir_fd=locks_descriptor,
        )
    finally:
        os.close(locks_descriptor)
    metadata = os.fstat(descriptor)
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) != 0o600
    ):
        os.close(descriptor)
        raise BootstrapError("trusted seed lifecycle lock is unsafe")
    lifecycle = os.fdopen(descriptor, "rb")
    fcntl.flock(lifecycle, fcntl.LOCK_SH)
    return lifecycle


def sealed_runtime_seed(source: Path) -> tuple[str, Path, object]:
    configured = os.environ.get("CODECLEW_RUNTIME_SEED")
    if configured is None:
        raise BootstrapError("sealed runtime seed authority is unavailable")
    seed_path = Path(configured)
    if (
        not seed_path.is_absolute()
        or ".." in seed_path.parts
        or seed_path.name != "seed.json"
        or re.fullmatch(r"release-N-[0-9a-f]{40}", seed_path.parent.name) is None
    ):
        raise BootstrapError("sealed runtime seed path is unsafe")
    lifecycle = _trusted_seed_lifecycle(seed_path.parent.parent)
    try:
        return _sealed_runtime_seed_locked(source, seed_path)
    finally:
        lifecycle.close()


def _verify_release_source(source: Path, seed: dict[str, object]) -> None:
    manifest_path = source / "release-source.json"
    descriptor = os.open(
        manifest_path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    )
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) != 0o400
            or metadata.st_size > MAX_MANIFEST_BYTES
        ):
            raise BootstrapError("release source manifest is unsafe")
        with os.fdopen(descriptor, "rb", closefd=False) as stream:
            encoded = stream.read(MAX_MANIFEST_BYTES + 1)
    finally:
        os.close(descriptor)
    try:
        manifest = json.loads(encoded)
    except (ValueError, TypeError) as error:
        raise BootstrapError("release source manifest is invalid") from error
    required = {
        "files",
        "manifestDigest",
        "schema",
        "sourceRevision",
        "sourceTree",
    }
    unsigned = dict(manifest) if isinstance(manifest, dict) else {}
    expected_digest = unsigned.get("manifestDigest")
    unsigned["manifestDigest"] = ""
    if (
        not isinstance(manifest, dict)
        or set(manifest) != required
        or manifest.get("schema") != RELEASE_SOURCE_SCHEMA
        or encoded != canonical(manifest) + b"\n"
        or not valid_runtime_key(expected_digest)
        or digest_bytes(canonical(unsigned)) != expected_digest
        or manifest.get("sourceRevision") != seed.get("sourceRevision")
        or manifest.get("sourceTree") != seed.get("sourceTree")
        or expected_digest != seed.get("sourcePayloadDigest")
        or not isinstance(manifest.get("files"), list)
        or not manifest["files"]
    ):
        raise BootstrapError("release source manifest authority mismatch")
    observed: set[str] = set()
    previous = ""
    for row in manifest["files"]:
        if not isinstance(row, dict) or set(row) != {"mode", "path", "sha256", "size"}:
            raise BootstrapError("release source file authority is invalid")
        relative = row.get("path")
        if (
            not isinstance(relative, str)
            or not relative
            or "\\" in relative
            or Path(relative).is_absolute()
            or any(part in {"", ".", ".."} for part in relative.split("/"))
            or relative <= previous
            or row.get("mode") not in {0, 0o111}
            or not isinstance(row.get("size"), int)
            or row["size"] < 0
            or not valid_runtime_key(row.get("sha256"))
        ):
            raise BootstrapError("release source file authority is invalid")
        previous = relative
        target = source / relative
        target_metadata = target.lstat()
        expected_mode = 0o500 if row["mode"] else 0o400
        if (
            stat.S_ISLNK(target_metadata.st_mode)
            or not stat.S_ISREG(target_metadata.st_mode)
            or target_metadata.st_uid != os.geteuid()
            or stat.S_IMODE(target_metadata.st_mode) != expected_mode
            or target_metadata.st_size != row["size"]
            or digest_file(target) != row["sha256"]
        ):
            raise BootstrapError("release source file authority mismatch")
        observed.add(relative)
    actual = set()
    for path in source.rglob("*"):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise BootstrapError("release source contains a symlink")
        if stat.S_ISREG(metadata.st_mode):
            actual.add(path.relative_to(source).as_posix())
        elif not stat.S_ISDIR(metadata.st_mode):
            raise BootstrapError("release source contains an unsupported entry")
    if actual != observed | {"release-source.json"}:
        raise BootstrapError("release source closure mismatch")


def _sealed_runtime_seed_locked(source: Path, seed_path: Path) -> tuple[str, Path, object]:
    if seed_path.resolve(strict=True) != seed_path:
        raise BootstrapError("sealed runtime seed path is unsafe")
    descriptor = os.open(
        seed_path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    )
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) != 0o400
            or metadata.st_size > MAX_MANIFEST_BYTES
        ):
            raise BootstrapError("sealed runtime seed file is unsafe")
        with os.fdopen(descriptor, "rb", closefd=False) as stream:
            seed_bytes = stream.read(MAX_MANIFEST_BYTES + 1)
        if len(seed_bytes) > MAX_MANIFEST_BYTES:
            raise BootstrapError("sealed runtime seed file is oversized")
    finally:
        os.close(descriptor)
    epoch = seed_path.parent
    epoch_metadata = epoch.lstat()
    if (
        not re.fullmatch(r"release-N-[0-9a-f]{40}", epoch.name)
        or stat.S_ISLNK(epoch_metadata.st_mode)
        or not stat.S_ISDIR(epoch_metadata.st_mode)
        or epoch_metadata.st_uid != os.geteuid()
        or stat.S_IMODE(epoch_metadata.st_mode) != 0o700
    ):
        raise BootstrapError("sealed runtime seed epoch is unsafe")
    try:
        seed = json.loads(seed_bytes)
    except (OSError, ValueError, TypeError) as error:
        raise BootstrapError("sealed runtime seed is invalid") from error
    v1_fields = {
        "artifactHashes",
        "buildEvidenceDigests",
        "manifestDigest",
        "mode",
        "runtimeKey",
        "schema",
        "seedDigest",
        "sourceRevision",
        "sourceTree",
        "stateEpoch",
        "workerTreeHashes",
    }
    v2_fields = v1_fields | {"sourcePayloadDigest"}
    unsigned = dict(seed) if isinstance(seed, dict) else {}
    expected_digest = unsigned.pop("seedDigest", None)
    if (
        not isinstance(seed, dict)
        or (
            seed.get("schema") == "codeclew-trusted-release-seed/1.0"
            and set(seed) != v1_fields
        )
        or (
            seed.get("schema") == "codeclew-trusted-release-seed/2.0"
            and set(seed) != v2_fields
        )
        or seed.get("schema") not in {
            "codeclew-trusted-release-seed/1.0",
            "codeclew-trusted-release-seed/2.0",
        }
        or seed.get("mode") != "RELEASE"
        or expected_digest != digest_bytes(canonical(unsigned))
        or not valid_runtime_key(seed.get("runtimeKey"))
    ):
        raise BootstrapError("sealed runtime seed authority mismatch")
    if seed.get("schema") == "codeclew-trusted-release-seed/1.0":
        revision = run(["git", "rev-parse", "HEAD"], source).decode().strip()
        tree = run(["git", "rev-parse", "HEAD^{tree}"], source).decode().strip()
        if seed.get("sourceRevision") != revision or seed.get("sourceTree") != tree:
            raise BootstrapError("sealed runtime seed source authority mismatch")
    else:
        _verify_release_source(source, seed)
    parallel = epoch / "parallel-state"
    state = parallel / "v2"
    for path in (parallel, state):
        state_metadata = path.lstat()
        if (
            stat.S_ISLNK(state_metadata.st_mode)
            or not stat.S_ISDIR(state_metadata.st_mode)
            or state_metadata.st_uid != os.geteuid()
            or stat.S_IMODE(state_metadata.st_mode) != 0o700
            or path.resolve(strict=True) != path
        ):
            raise BootstrapError("sealed runtime seed state is unsafe")
    key = str(seed["runtimeKey"])
    locks = state / "locks"
    locks_descriptor = os.open(locks, _directory_flags())
    locks_metadata = os.fstat(locks_descriptor)
    if (
        not stat.S_ISDIR(locks_metadata.st_mode)
        or locks_metadata.st_uid != os.geteuid()
        or stat.S_IMODE(locks_metadata.st_mode) != 0o700
    ):
        os.close(locks_descriptor)
        raise BootstrapError("sealed runtime seed locks are unsafe")
    lease_path = locks / f"runtime-{key.removeprefix('sha256:')}.lease"
    try:
        lease_descriptor = os.open(
            lease_path.name,
            os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
            dir_fd=locks_descriptor,
        )
    except OSError as error:
        os.close(locks_descriptor)
        raise BootstrapError("sealed runtime seed lease is unsafe") from error
    os.close(locks_descriptor)
    lease_metadata = os.fstat(lease_descriptor)
    if (
        not stat.S_ISREG(lease_metadata.st_mode)
        or lease_metadata.st_uid != os.geteuid()
        or stat.S_IMODE(lease_metadata.st_mode) != 0o600
    ):
        os.close(lease_descriptor)
        raise BootstrapError("sealed runtime seed lease is unsafe")
    lease = os.fdopen(lease_descriptor, "rb")
    fcntl.flock(lease, fcntl.LOCK_SH)
    try:
        # The shared lease must cover both discovery and verification. Otherwise
        # a concurrent GC can remove or replace the capsule after verification
        # but before the caller starts using it.
        capsule = _runtime_capsule_directory(state, key)
        if capsule is None:
            raise BootstrapError("sealed runtime seed capsule is unavailable")
        manifest = verify_capsule(capsule, key)
        artifact_hashes = {
            name: value["sha256"]
            for name, value in sorted(manifest["artifacts"].items())
        }
        worker_hashes = {
            name: value["treeHash"]
            for name, value in sorted(manifest["workers"].items())
        }
        if (
            manifest.get("mode") != "RELEASE"
            or manifest.get("manifestDigest") != seed.get("manifestDigest")
            or artifact_hashes != seed.get("artifactHashes")
            or worker_hashes != seed.get("workerTreeHashes")
        ):
            raise BootstrapError("sealed runtime seed capsule authority mismatch")
        return key, capsule, lease
    except BaseException:
        lease.close()
        raise


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


def revalidate_checkpoint_capsule(
    source: Path, root: Path, key: str
) -> dict[str, object] | None:
    capsule = _runtime_capsule_directory(root, key)
    if capsule is None:
        return None
    try:
        manifest = verify_capsule(capsule, key)
    except Exception as error:
        quarantine(root, capsule, type(error).__name__)
        return None
    inputs, development = source_manifest(source)
    mode = "DEVELOPMENT" if development else "RELEASE"
    toolchain = manifest.get("toolchainAuthority")
    platform_authority = manifest.get("platformAuthority")
    if (
        not isinstance(toolchain, dict)
        or set(toolchain) != {"python", "rust", "jdk"}
        or any(not isinstance(toolchain[name], dict) for name in toolchain)
        or not isinstance(platform_authority, dict)
    ):
        raise BootstrapError("runtime manifest toolchain authority is invalid")
    tools = {**toolchain, "platform": platform_authority}
    if (
        manifest.get("mode") != mode
        or manifest.get("inputDigest") != digest_bytes(canonical(inputs))
        or runtime_key(mode, inputs, tools) != key
    ):
        return None
    return {
        "capsule": str(capsule),
        "inputs": inputs,
        "mode": mode,
        "revalidated": True,
        "runtimeKey": key,
    }


def runtime_key(mode: str, inputs: list[dict[str, object]], tools: dict[str, object]) -> str:
    digest = hashlib.sha256()
    digest.update(DOMAIN)
    digest.update(mode.encode())
    digest.update(b"\0")
    digest.update(canonical({"inputs": inputs, "toolchains": tools}))
    return "sha256:" + digest.hexdigest()


def _component_identifier(value: object, label: str) -> str:
    if (
        not isinstance(value, str)
        or not value
        or len(value.encode("utf-8")) > 128
        or any(
            not (
                character.isascii()
                and (character.isalnum() or character in {"-", "_", ".", ":"})
            )
            for character in value
        )
    ):
        raise BootstrapError(f"runtime component {label} is invalid")
    return value


def _component_input_rows(
    inputs: list[dict[str, object]],
) -> list[dict[str, object]]:
    normalized = []
    observed = set()
    for row in inputs:
        if not isinstance(row, dict) or set(row) != {"mode", "path", "sha256", "size"}:
            raise BootstrapError("runtime component input row is invalid")
        relative = row["path"]
        path = Path(relative) if isinstance(relative, str) else Path("")
        if (
            not isinstance(relative, str)
            or not relative
            or path.is_absolute()
            or any(
                not isinstance(component, str)
                or component in {"", ".", ".."}
                for component in relative.split("/")
            )
            or "\\" in relative
            or relative in observed
            or not isinstance(row["size"], int)
            or isinstance(row["size"], bool)
            or row["size"] < 0
            or row["mode"] not in {0, 0o111}
            or not valid_runtime_key(row["sha256"])
        ):
            raise BootstrapError("runtime component input row is invalid")
        observed.add(relative)
        normalized.append(dict(row))
    if not normalized:
        raise BootstrapError("runtime component input closure is empty")
    normalized.sort(key=lambda row: str(row["path"]))
    return normalized


def component_authority(
    mode: str,
    component_kind: str,
    component_id: str,
    inputs: list[dict[str, object]],
    toolchain_authority: dict[str, object],
    build_contract: dict[str, object],
) -> dict[str, object]:
    if mode not in {"RELEASE", "DEVELOPMENT"}:
        raise BootstrapError("runtime component mode is invalid")
    kind = _component_identifier(component_kind, "kind")
    identifier = _component_identifier(component_id, "id")
    rows = _component_input_rows(inputs)
    if not isinstance(toolchain_authority, dict) or not toolchain_authority:
        raise BootstrapError("runtime component toolchain authority is invalid")
    if not isinstance(build_contract, dict) or not build_contract:
        raise BootstrapError("runtime component build contract is invalid")
    _validate_component_value(toolchain_authority)
    _validate_component_value(build_contract)
    authority = {
        "buildContractDigest": digest_bytes(canonical(build_contract)),
        "componentId": identifier,
        "componentKind": kind,
        "inputDigest": digest_bytes(canonical({"inputs": rows})),
        "mode": mode,
        "schema": COMPONENT_AUTHORITY_SCHEMA,
        "toolchainDigest": digest_bytes(canonical(toolchain_authority)),
    }
    authority["componentKey"] = digest_bytes(
        COMPONENT_DOMAIN + canonical(authority)
    )
    return authority


def _validate_component_value(value: object, depth: int = 0) -> None:
    if depth > 32:
        raise BootstrapError("runtime component authority is too deeply nested")
    if value is None or isinstance(value, (bool, int, str)):
        return
    if isinstance(value, (list, tuple)):
        for item in value:
            _validate_component_value(item, depth + 1)
        return
    if isinstance(value, dict) and all(isinstance(key, str) for key in value):
        for item in value.values():
            _validate_component_value(item, depth + 1)
        return
    raise BootstrapError("runtime component authority contains an unsupported value")


def _registry_relative(value: object, label: str) -> str:
    if not isinstance(value, str) or not value or "\\" in value:
        raise BootstrapError(f"runtime component registry {label} is invalid")
    path = Path(value)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in value.split("/")):
        raise BootstrapError(f"runtime component registry {label} is invalid")
    return value


def load_component_registry(source: Path) -> dict[str, object]:
    path = source / "bootstrap" / "runtime_components.json"
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_size > MAX_MANIFEST_BYTES
            or metadata.st_uid != os.geteuid()
        ):
            raise BootstrapError("runtime component registry is unsafe")
        with os.fdopen(descriptor, "rb", closefd=False) as stream:
            encoded = stream.read(MAX_MANIFEST_BYTES + 1)
    finally:
        os.close(descriptor)
    try:
        registry = json.loads(encoded)
    except (ValueError, TypeError) as error:
        raise BootstrapError("runtime component registry is invalid") from error
    if (
        not isinstance(registry, dict)
        or set(registry) != {"components", "schema"}
        or registry.get("schema") != COMPONENT_REGISTRY_SCHEMA
        or encoded != canonical(registry) + b"\n"
        or not isinstance(registry.get("components"), list)
        or not registry["components"]
    ):
        raise BootstrapError("runtime component registry is not canonical and closed")
    expected_fields = {
        "buildContract",
        "componentId",
        "componentKind",
        "inputFiles",
        "inputRoots",
        "optionalInputRoots",
        "toolchainKeys",
    }
    component_ids = set()
    runtime_names = set()
    distributions = set()
    core_count = 0
    for component in registry["components"]:
        if not isinstance(component, dict) or set(component) != expected_fields:
            raise BootstrapError("runtime component registry row is invalid")
        identifier = _component_identifier(component["componentId"], "id")
        kind = _component_identifier(component["componentKind"], "kind")
        if identifier in component_ids or kind not in {"core-binary", "language-adapter"}:
            raise BootstrapError("runtime component registry identity is duplicated or unsupported")
        component_ids.add(identifier)
        for field in (
            "inputFiles",
            "inputRoots",
            "optionalInputRoots",
            "toolchainKeys",
        ):
            values = component[field]
            if (
                not isinstance(values, list)
                or (field != "optionalInputRoots" and not values)
                or not all(isinstance(value, str) and value for value in values)
                or values != sorted(set(values))
            ):
                raise BootstrapError(f"runtime component registry {field} is invalid")
        for value in [
            *component["inputFiles"],
            *component["inputRoots"],
            *component["optionalInputRoots"],
        ]:
            _registry_relative(value, "input path")
        for value in component["toolchainKeys"]:
            _component_identifier(value, "toolchain key")
        contract = component["buildContract"]
        if not isinstance(contract, dict):
            raise BootstrapError("runtime component registry build contract is invalid")
        _validate_component_value(contract)
        executor = contract.get("executor")
        if kind == "core-binary":
            core_count += 1
            if set(contract) != {"artifactName", "binary", "executor", "package"} or executor != "CARGO":
                raise BootstrapError("runtime core build contract is invalid")
            for field in ("artifactName", "binary", "package"):
                _component_identifier(contract.get(field), field)
        else:
            if set(contract) != {
                "compilerVersion",
                "distribution",
                "executor",
                "manifest",
                "protocol",
                "runtimeName",
                "task",
            } or executor != "GRADLE":
                raise BootstrapError("runtime adapter build contract is invalid")
            runtime_name = _component_identifier(contract.get("runtimeName"), "runtime name")
            distribution = _registry_relative(contract.get("distribution"), "distribution")
            _registry_relative(contract.get("manifest"), "manifest")
            if (
                runtime_name in runtime_names
                or distribution in distributions
                or contract.get("protocol") != "semantic-thread.worker.v1"
                or not isinstance(contract.get("compilerVersion"), str)
                or not contract["compilerVersion"]
                or not isinstance(contract.get("task"), str)
                or not contract["task"].startswith(":")
            ):
                raise BootstrapError("runtime adapter registry authority is invalid")
            runtime_names.add(runtime_name)
            distributions.add(distribution)
    if core_count != 1:
        raise BootstrapError("runtime component registry must contain exactly one core")
    return registry


def runtime_component_specs(
    mode: str,
    inputs: list[dict[str, object]],
    tools: dict[str, object],
    registry: dict[str, object],
) -> list[dict[str, object]]:
    by_path = {str(row["path"]): row for row in inputs}
    requested_roots = {
        root
        for component in registry["components"]
        for root in [
            *component["inputRoots"],
            *component["optionalInputRoots"],
        ]
    }
    rows_by_root: dict[str, dict[str, dict[str, object]]] = {
        root: {} for root in requested_roots
    }
    for relative, row in by_path.items():
        parts = relative.split("/")
        for index in range(1, len(parts)):
            parent = "/".join(parts[:index])
            if parent in rows_by_root:
                rows_by_root[parent][relative] = row
    specs = []
    for component in registry["components"]:
        selected: dict[str, dict[str, object]] = {}
        for relative in component["inputFiles"]:
            if relative not in by_path:
                raise BootstrapError(
                    f"runtime component input file is absent: {relative}"
                )
            selected[relative] = by_path[relative]
        for root in component["inputRoots"]:
            matches = rows_by_root[root]
            if not matches:
                raise BootstrapError(
                    f"runtime component input root is empty: {root}"
                )
            selected.update(matches)
        for root in component["optionalInputRoots"]:
            selected.update(rows_by_root[root])
        toolchain = {}
        for key in component["toolchainKeys"]:
            if key not in tools:
                raise BootstrapError(
                    f"runtime component toolchain authority is absent: {key}"
                )
            toolchain[key] = tools[key]
        authority = component_authority(
            mode,
            component["componentKind"],
            component["componentId"],
            list(selected.values()),
            toolchain,
            component["buildContract"],
        )
        specs.append({**component, "authority": authority})
    return specs


def _component_file_rows(root: Path) -> list[dict[str, object]]:
    rows = []
    for path in sorted(root.rglob("*"), key=str):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise BootstrapError("runtime component contains a symlink")
        if stat.S_ISDIR(metadata.st_mode):
            continue
        if not stat.S_ISREG(metadata.st_mode):
            raise BootstrapError("runtime component contains an unsupported entry")
        rows.append({
            "mode": 0o111 if metadata.st_mode & 0o111 else 0,
            "path": path.relative_to(root).as_posix(),
            "sha256": digest_file(path),
            "size": metadata.st_size,
        })
    return rows


def _component_tree_hash(rows: list[dict[str, object]]) -> str:
    return digest_bytes(COMPONENT_DOMAIN + b"tree\0" + canonical({"files": rows}))


def _component_path(root: Path, key: str) -> Path:
    if not valid_runtime_key(key):
        raise BootstrapError("runtime component key is invalid")
    return root / "runtimes" / "components" / key.removeprefix("sha256:")


def _read_component_manifest(path: Path) -> tuple[dict[str, object], bytes]:
    descriptor = os.open(
        path,
        os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
    )
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or metadata.st_size > MAX_MANIFEST_BYTES
            or stat.S_IMODE(metadata.st_mode) != 0o400
        ):
            raise BootstrapError("runtime component manifest is unsafe")
        with os.fdopen(descriptor, "rb", closefd=False) as stream:
            encoded = stream.read(MAX_MANIFEST_BYTES + 1)
    finally:
        os.close(descriptor)
    try:
        value = json.loads(encoded)
    except (ValueError, TypeError) as error:
        raise BootstrapError("runtime component manifest is invalid") from error
    if not isinstance(value, dict) or encoded != canonical(value) + b"\n":
        raise BootstrapError("runtime component manifest is not canonical")
    return value, encoded


def verify_component(
    root: Path,
    key: str,
    expected_authority: dict[str, object] | None = None,
) -> tuple[Path, dict[str, object]]:
    component = _component_path(root, key)
    metadata = component.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise BootstrapError("runtime component root is unsafe")
    if {path.name for path in component.iterdir()} != {"READY", "component.json", "files"}:
        raise BootstrapError("runtime component root closure mismatch")
    files_root = component / "files"
    files_metadata = files_root.lstat()
    if stat.S_ISLNK(files_metadata.st_mode) or not stat.S_ISDIR(files_metadata.st_mode):
        raise BootstrapError("runtime component files root is unsafe")
    manifest, _encoded = _read_component_manifest(component / "component.json")
    expected_digest = manifest.get("manifestDigest")
    unsigned = dict(manifest)
    unsigned["manifestDigest"] = ""
    required = {
        "buildContractDigest",
        "componentId",
        "componentKey",
        "componentKind",
        "files",
        "inputDigest",
        "manifestDigest",
        "mode",
        "schema",
        "toolchainDigest",
        "treeHash",
    }
    if (
        set(manifest) != required
        or manifest.get("schema") != COMPONENT_SCHEMA
        or manifest.get("componentKey") != key
        or manifest.get("mode") not in {"RELEASE", "DEVELOPMENT"}
        or not valid_runtime_key(expected_digest)
        or digest_bytes(canonical(unsigned)) != expected_digest
    ):
        raise BootstrapError("runtime component manifest authority mismatch")
    if expected_authority is not None:
        for field in (
            "buildContractDigest",
            "componentId",
            "componentKey",
            "componentKind",
            "inputDigest",
            "mode",
            "toolchainDigest",
        ):
            if manifest.get(field) != expected_authority.get(field):
                raise BootstrapError("runtime component expected authority mismatch")
    rows = _component_file_rows(files_root)
    if manifest.get("files") != rows or manifest.get("treeHash") != _component_tree_hash(rows):
        raise BootstrapError("runtime component output closure mismatch")
    for row in rows:
        metadata = (files_root / str(row["path"])).lstat()
        expected_mode = 0o500 if row["mode"] else 0o400
        if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != expected_mode:
            raise BootstrapError("runtime component output mode is unsafe")
    ready = component / "READY"
    ready_metadata = ready.lstat()
    if (
        stat.S_ISLNK(ready_metadata.st_mode)
        or not stat.S_ISREG(ready_metadata.st_mode)
        or stat.S_IMODE(ready_metadata.st_mode) != 0o400
        or ready.read_bytes() != (key + "\n").encode()
    ):
        raise BootstrapError("runtime component is not ready")
    verify_sealed_capsule(component)
    return component, manifest


def _quarantine_component(root: Path, component: Path, reason: str) -> None:
    expected_parent = root / "runtimes" / "components"
    if component.parent != expected_parent or not valid_runtime_key(
        "sha256:" + component.name
    ):
        raise BootstrapError("unsafe runtime component quarantine target")
    destination = root / "quarantine" / (
        f"component-{component.name}-{uuid.uuid4().hex}"
    )
    metadata = component.lstat()
    if not stat.S_ISLNK(metadata.st_mode):
        os.chmod(component, 0o700)
    os.replace(component, destination)
    record = destination.with_suffix(".json")
    descriptor = os.open(record, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    try:
        with os.fdopen(descriptor, "wb", closefd=False) as stream:
            stream.write(canonical({
                "componentKey": "sha256:" + component.name,
                "reason": reason,
                "schema": "codeclew-runtime-component-quarantine/1.0",
            }) + b"\n")
            stream.flush()
            os.fsync(stream.fileno())
    finally:
        os.close(descriptor)


def publish_component(
    root: Path,
    authority: dict[str, object],
    output: Path,
) -> tuple[Path, bool]:
    key = authority.get("componentKey")
    authority_fields = {
        "buildContractDigest",
        "componentId",
        "componentKey",
        "componentKind",
        "inputDigest",
        "mode",
        "schema",
        "toolchainDigest",
    }
    if set(authority) != authority_fields or authority.get("schema") != COMPONENT_AUTHORITY_SCHEMA:
        raise BootstrapError("runtime component publication authority is invalid")
    _component_identifier(authority.get("componentKind"), "kind")
    _component_identifier(authority.get("componentId"), "id")
    if authority.get("mode") not in {"RELEASE", "DEVELOPMENT"} or any(
        not valid_runtime_key(authority.get(field))
        for field in ("buildContractDigest", "componentKey", "inputDigest", "toolchainDigest")
    ):
        raise BootstrapError("runtime component publication authority is invalid")
    # The authority itself is already content-addressed. Recompute its key from
    # the closed digest-only projection so callers cannot substitute an ID.
    unsigned_authority = {
        field: authority[field]
        for field in (
            "buildContractDigest",
            "componentId",
            "componentKind",
            "inputDigest",
            "mode",
            "schema",
            "toolchainDigest",
        )
    }
    if key != digest_bytes(COMPONENT_DOMAIN + canonical(unsigned_authority)):
        raise BootstrapError("runtime component key differs from its authority")
    destination = _component_path(root, key)
    lock_path = root / "locks" / f"component-{key.removeprefix('sha256:')}.lock"
    with lock_path.open("a+b") as lock:
        os.chmod(lock_path, 0o600)
        fcntl.flock(lock, fcntl.LOCK_EX)
        if destination.exists() or destination.is_symlink():
            try:
                component, _manifest = verify_component(root, key, authority)
                return component, False
            except (BootstrapError, OSError, ValueError, TypeError) as error:
                _quarantine_component(root, destination, type(error).__name__)
        temporary = Path(tempfile.mkdtemp(prefix="component-build-", dir=root / "tmp"))
        staged = temporary / "component"
        files_root = staged / "files"
        try:
            source_rows = _component_file_rows(output)
            if not source_rows:
                raise BootstrapError("runtime component output closure is empty")
            files_root.mkdir(mode=0o700, parents=True)
            for row in source_rows:
                relative = Path(str(row["path"]))
                target = files_root / relative
                target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                shutil.copyfile(output / relative, target, follow_symlinks=False)
                os.chmod(target, 0o700 if row["mode"] else 0o600)
            rows = _component_file_rows(files_root)
            if rows != source_rows:
                raise BootstrapError("runtime component changed while it was copied")
            manifest = {
                **unsigned_authority,
                "componentKey": key,
                "files": rows,
                "manifestDigest": "",
                "schema": COMPONENT_SCHEMA,
                "treeHash": _component_tree_hash(rows),
            }
            manifest["manifestDigest"] = digest_bytes(canonical(manifest))
            (staged / "component.json").write_bytes(canonical(manifest) + b"\n")
            (staged / "READY").write_text(key + "\n")
            seal_capsule(staged, seal_root=False)
            fsync_tree(staged)
            os.rename(staged, destination)
            os.chmod(destination, 0o500)
            fsync_tree(destination)
            component, _manifest = verify_component(root, key, authority)
            return component, True
        finally:
            discard_private_tree(temporary)


def materialize_component(
    root: Path,
    key: str,
    destination: Path,
    expected_authority: dict[str, object] | None = None,
) -> list[dict[str, object]]:
    component, manifest = verify_component(root, key, expected_authority)
    return _materialize_verified_component(
        component, manifest, destination, verify_content=True
    )


def _materialize_verified_component(
    component: Path,
    manifest: dict[str, object],
    destination: Path,
    *,
    verify_content: bool,
) -> list[dict[str, object]]:
    if destination.exists() or destination.is_symlink():
        raise BootstrapError("runtime component materialization target already exists")
    destination.mkdir(mode=0o700, parents=True)
    rows = manifest["files"]
    for row in rows:
        relative = Path(str(row["path"]))
        target = destination / relative
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        shutil.copyfile(component / "files" / relative, target, follow_symlinks=False)
        os.chmod(target, 0o700 if row["mode"] else 0o600)
    if verify_content:
        if _component_file_rows(destination) != rows:
            raise BootstrapError("runtime component materialization mismatch")
    else:
        for row in rows:
            target = destination / str(row["path"])
            metadata = target.lstat()
            if (
                stat.S_ISLNK(metadata.st_mode)
                or not stat.S_ISREG(metadata.st_mode)
                or metadata.st_size != row["size"]
                or bool(metadata.st_mode & 0o111) != bool(row["mode"])
            ):
                raise BootstrapError("runtime component materialization metadata mismatch")
    return rows


def stage_inputs(
    source: Path,
    destination: Path,
    rows: list[dict[str, object]],
    *,
    workers: int | None = None,
) -> None:
    destination.mkdir(mode=0o700)

    def copy(row: dict[str, object]) -> None:
        relative = Path(str(row["path"]))
        target = destination / relative
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        shutil.copyfile(source / relative, target, follow_symlinks=False)
        os.chmod(target, 0o700 if row["mode"] else 0o600)

    selected_workers = workers or min(8, max(1, os.cpu_count() or 1))
    with ThreadPoolExecutor(max_workers=selected_workers) as executor:
        list(executor.map(copy, rows))
    verify_source_manifest(destination, rows, full_closure=False)


def build_environment(
    stage: Path, root: Path, *, gradle_required: bool = True
) -> dict[str, str]:
    environment = sanitized_environment()
    physical_home = Path.home()
    # Build scripts sometimes embed HOME/USER even when rustc source paths are
    # remapped.  Give child processes a stable, non-personal logical identity;
    # keep Cargo/Rustup stores explicit so the installed toolchain remains
    # discoverable without consulting the logical home.
    environment["CARGO_HOME"] = environment.get(
        "CARGO_HOME", str(physical_home / ".cargo")
    )
    environment["RUSTUP_HOME"] = environment.get(
        "RUSTUP_HOME", str(physical_home / ".rustup")
    )
    environment["HOME"] = "/codeclew/home"
    environment["USER"] = "codeclew"
    environment["LOGNAME"] = "codeclew"
    environment["XDG_CONFIG_HOME"] = "/codeclew/config"
    cargo_target = stage.parent / "cargo-target"
    cargo_target.mkdir(mode=0o700)
    environment["CARGO_TARGET_DIR"] = str(cargo_target)
    environment["CARGO_INCREMENTAL"] = "0"
    # Rust embeds source/OUT_DIR paths in panic locations and uses them when
    # deriving the Mach-O UUID.  Every capsule build has a private random
    # directory, so release artifacts are reproducible only when those paths
    # are mapped into a stable, non-personal namespace.  Use the encoded form
    # so whitespace in a local path cannot change argument tokenization.
    # rustc applies the last matching prefix, therefore order mappings from
    # broadest to most specific.
    remaps = [
        f"--remap-path-prefix={physical_home}=/codeclew/home",
        f"--remap-path-prefix={environment['CARGO_HOME']}=/codeclew/cargo-home",
        f"--remap-path-prefix={environment['RUSTUP_HOME']}=/codeclew/rustup-home",
        f"--remap-path-prefix={stage.parent}=/codeclew/build",
        f"--remap-path-prefix={stage}=/codeclew/source",
    ]
    environment["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(remaps)
    environment["GIT_TERMINAL_PROMPT"] = "0"
    if gradle_required:
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
            "mode": 0o111 if metadata.st_mode & 0o111 else 0,
            "path": path.relative_to(root).as_posix(),
            "size": metadata.st_size,
            "sha256": digest_file(path),
        })
    return rows


def verify_capsule_has_no_private_paths(capsule: Path, paths: list[Path]) -> None:
    forbidden = sorted(
        {
            str(path).encode()
            for path in paths
            if path.is_absolute() and str(path) not in {"", "/"}
        },
        key=len,
        reverse=True,
    )
    if not forbidden:
        return
    overlap = max(map(len, forbidden)) - 1
    for artifact in sorted(path for path in capsule.rglob("*") if path.is_file()):
        tail = b""
        with artifact.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                window = tail + chunk
                matches = [value for value in forbidden if value in window]
                if matches:
                    fingerprints = ",".join(
                        sorted(hashlib.sha256(value).hexdigest()[:16] for value in matches)
                    )
                    origins = set()
                    for value in matches:
                        for suffix, label in [
                            (b"/.cargo", "cargo-home"),
                            (b"/.rustup", "rustup-home"),
                            (b"/work", "workspace"),
                            (b"/.cache", "cache-home"),
                            (b"/.local", "local-home"),
                        ]:
                            if value + suffix in window:
                                origins.add(label)
                    origin_summary = ",".join(sorted(origins or {"other"}))
                    raise BootstrapError(
                        "capsule artifact contains a private build path: "
                        f"{artifact.relative_to(capsule).as_posix()} "
                        f"(path fingerprints: {fingerprints}; origins: {origin_summary})"
                    )
                tail = window[-overlap:] if overlap else b""


def tree_hash(rows: list[dict[str, object]]) -> str:
    digest = hashlib.sha256()
    for row in rows:
        digest.update(str(row["path"]).encode())
        digest.update(b"\0")
        digest.update(str(row["mode"]).encode())
        digest.update(b"\0")
        digest.update(str(row["size"]).encode())
        digest.update(b"\0")
        digest.update(str(row["sha256"]).encode())
        digest.update(b"\0")
    return "sha256:" + digest.hexdigest()


def bootstrap_self_test() -> None:
    rows = [
        {"mode": 0o111, "path": "bin/a", "size": 3, "sha256": "sha256:" + "0" * 64},
        {"mode": 0, "path": "lib/b", "size": 5, "sha256": "sha256:" + "1" * 64},
    ]
    assert tree_hash(rows) == "sha256:6fd9755d0c290c62d1e09d5f6f13387c889754193b9ff8d30ff15b1e21b6ccdd"
    first = locator_key("RELEASE", rows, {"tool": "a"})
    assert first == locator_key("RELEASE", rows, {"tool": "a"})
    assert first != locator_key("DEVELOPMENT", rows, {"tool": "a"})
    assert first != locator_key("RELEASE", rows, {"tool": "b"})
    assert runtime_build_plan(8, 32 * 1024**3)["parallel"] is True
    assert runtime_build_plan(8, 8 * 1024**3)["parallel"] is False
    try:
        runtime_build_plan(8, 4 * 1024**3)
    except BootstrapError:
        pass
    else:
        raise AssertionError("cold heap admission accepted an undersized host")
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
    if (
        manifest.get("schema") != "trusted-worker-distribution/0.2"
        or expected != rows
        or manifest.get("treeHash") != tree_hash(rows)
    ):
        raise BootstrapError(
            "RELEASE worker differs from its committed manifest; regenerate and verify all "
            f"affected worker variants: {manifest_relative}"
        )


def runtime_build_plan(
    cpu_count: int,
    total_memory_bytes: int,
    profile: str = "AUTO",
) -> dict[str, object]:
    if profile not in {"AUTO", "SERIAL", "PARALLEL"}:
        raise BootstrapError("cold build profile is invalid")
    cpu_count = max(1, cpu_count)
    reserved = max(1024**3, total_memory_bytes * 15 // 100)
    memory_budget = max(0, total_memory_bytes - reserved) * 70 // 100
    parallel_gradle_budget = memory_budget * 55 // 100
    parallel_cargo_budget = memory_budget - parallel_gradle_budget
    parallel_gradle_heap = max(0, parallel_gradle_budget - GRADLE_NON_HEAP_BYTES)
    parallel_admissible = (
        cpu_count >= 2
        and parallel_gradle_heap >= GRADLE_MIN_HEAP_BYTES
        and parallel_cargo_budget >= TOOLCHAIN_WORKER_MEMORY_BYTES
    )
    if profile == "PARALLEL" and not parallel_admissible:
        raise BootstrapError("host authority cannot admit the parallel cold build profile")
    parallel = parallel_admissible if profile == "AUTO" else profile == "PARALLEL"
    if parallel:
        gradle_heap = (
            min(GRADLE_MAX_HEAP_BYTES, parallel_gradle_heap) // 1024**2 * 1024**2
        )
        desired_cargo_workers = max(1, cpu_count // 2)
        desired_gradle_workers = max(1, cpu_count - desired_cargo_workers)
        cargo_workers = min(
            desired_cargo_workers,
            max(1, parallel_cargo_budget // TOOLCHAIN_WORKER_MEMORY_BYTES),
        )
        gradle_workers = min(
            desired_gradle_workers,
            max(1, gradle_heap // TOOLCHAIN_WORKER_MEMORY_BYTES),
        )
    else:
        gradle_heap = min(
            GRADLE_MAX_HEAP_BYTES,
            max(0, memory_budget - GRADLE_NON_HEAP_BYTES),
        ) // 1024**2 * 1024**2
        if gradle_heap < GRADLE_MIN_HEAP_BYTES:
            raise BootstrapError("host authority cannot admit the cold build heap")
        cargo_workers = (
            1
            if profile == "SERIAL"
            else min(
                cpu_count,
                max(1, memory_budget // TOOLCHAIN_WORKER_MEMORY_BYTES),
            )
        )
        gradle_workers = (
            1
            if profile == "SERIAL"
            else min(
                cpu_count,
                max(1, gradle_heap // TOOLCHAIN_WORKER_MEMORY_BYTES),
            )
        )
    return {
        "profile": profile,
        "parallel": parallel,
        "cargoWorkers": cargo_workers,
        "gradleHeapBytes": gradle_heap,
        "gradleWorkers": gradle_workers,
        "inputWorkers": min(8, cpu_count) if parallel else 1,
        "packageWorkers": min(8, cpu_count) if parallel else 1,
        "memoryBudgetBytes": memory_budget,
    }


def host_memory_bytes() -> int:
    try:
        return int(effective_host_resources()["totalMemoryBytes"])
    except (HostResourceError, KeyError, TypeError, ValueError) as error:
        raise BootstrapError(
            "host memory authority is unavailable for capsule admission"
        ) from error


def host_cpu_count() -> int:
    try:
        return int(effective_host_resources()["logicalCores"])
    except (HostResourceError, KeyError, TypeError, ValueError) as error:
        raise BootstrapError(
            "host CPU authority is unavailable for capsule admission"
        ) from error


def gradle_jvm_options(plan: dict[str, object]) -> str:
    heap = plan.get("gradleHeapBytes")
    if (
        type(heap) is not int
        or heap < GRADLE_MIN_HEAP_BYTES
        or heap > GRADLE_MAX_HEAP_BYTES
        or heap % 1024**2 != 0
    ):
        raise BootstrapError("Gradle heap authority is invalid")
    return (
        "-Xms256m "
        f"-Xmx{heap // 1024**2}m "
        "-XX:MaxMetaspaceSize=1024m "
        "-XX:MaxDirectMemorySize=512m "
        "-XX:+ExitOnOutOfMemoryError"
    )


def gradle_daemon_registry_base(stage: Path) -> Path:
    """Return a private absolute registry path for Gradle's single-use daemon."""
    if not stage.is_absolute():
        raise BootstrapError("Gradle build stage authority must be absolute")
    return stage.parent / ".codeclew-gradle-daemon"


def build_toolchains(
    stage: Path,
    environment: dict[str, str],
    plan: dict[str, object] | None = None,
    *,
    cargo_required: bool = True,
    gradle_tasks: list[str] | None = None,
) -> dict[str, object]:
    started = time.monotonic()
    plan = plan or runtime_build_plan(host_cpu_count(), host_memory_bytes())
    gradle_tasks = sorted(set(gradle_tasks or []))
    evidence_profile = plan["profile"] in {"SERIAL", "PARALLEL"}
    stages: list[tuple[str, list[str]]] = []
    if gradle_tasks:
        stages.append(("GRADLE_WORKERS", [
            str(stage / "gradlew"),
            *gradle_tasks,
            "--no-daemon",
            "--no-watch-fs",
            *(["--parallel"] if plan["parallel"] else []),
            "--no-build-cache" if evidence_profile else "--build-cache",
            *(["--offline", "-Pkotlin.compiler.execution.strategy=in-process"] if evidence_profile else []),
            "-Dorg.gradle.daemon.idletimeout=1000",
            "-Dorg.gradle.daemon.registry.base="
            f"{gradle_daemon_registry_base(stage)}",
            f"-Dorg.gradle.jvmargs={gradle_jvm_options(plan)}",
            f"--max-workers={plan['gradleWorkers']}",
            "--quiet",
        ]))
    if cargo_required:
        stages.append(("CARGO_BINARIES", [
            "cargo", "build", "--frozen" if evidence_profile else "--locked",
            "--release", "-p", "clew",
            "--bin", "clew", "--jobs", str(plan["cargoWorkers"]),
        ]))
    supervisor = BuildProcessSupervisor()
    max_workers = min(len(stages), 2) if plan["parallel"] else 1
    if not stages:
        return {
            **plan,
            "stageWallMillis": {},
            "toolchainCriticalPathMillis": 0,
            "toolchainStages": [],
            "toolchainWallMillis": 0,
        }
    stage_wall_millis: dict[str, int] = {}

    def execute(name: str, arguments: list[str]) -> tuple[str, int]:
        stage_started = time.monotonic()
        run_build_stage(arguments, stage, environment, name, supervisor)
        return name, int((time.monotonic() - stage_started) * 1000)

    with build_signal_scope(supervisor):
        try:
            with ThreadPoolExecutor(max_workers=max_workers) as executor:
                try:
                    if plan["parallel"] and len(stages) > 1:
                        futures = [
                            executor.submit(execute, name, arguments)
                            for name, arguments in stages
                        ]
                        for future in futures:
                            name, duration = future.result()
                            stage_wall_millis[name] = duration
                    else:
                        for name, arguments in stages:
                            measured_name, duration = executor.submit(
                                execute, name, arguments
                            ).result()
                            stage_wall_millis[measured_name] = duration
                except BaseException:
                    # Cancel before ThreadPoolExecutor.__exit__ waits for the
                    # sibling stage, otherwise a failed Gradle task could leave
                    # Cargo running until its natural completion (or vice versa).
                    supervisor.cancel()
                    raise
        except BaseException:
            supervisor.cancel()
            raise
    toolchain_wall_millis = int((time.monotonic() - started) * 1000)
    critical_path_millis = (
        max(stage_wall_millis.values())
        if plan["parallel"]
        else sum(stage_wall_millis.values())
    )
    return {
        **plan,
        "stageWallMillis": dict(sorted(stage_wall_millis.items())),
        "toolchainCriticalPathMillis": critical_path_millis,
        "toolchainStages": [name for name, _arguments in stages],
        "toolchainWallMillis": toolchain_wall_millis,
    }


def dependency_cache_authority(source: Path) -> dict[str, object]:
    inputs, development = source_manifest(source)
    mode = "DEVELOPMENT" if development else "RELEASE"
    tools = toolchain_authority(source)
    specs = runtime_component_specs(
        mode, inputs, tools, load_component_registry(source)
    )
    key = runtime_key(mode, inputs, tools)
    return {
        "artifactIds": sorted(
            spec["buildContract"]["artifactName"]
            for spec in specs
            if spec["componentKind"] == "core-binary"
        ),
        "componentIds": sorted(spec["componentId"] for spec in specs),
        "inputDigest": digest_bytes(canonical(inputs)),
        "mode": mode,
        "runtimeKey": key,
        "schema": "codeclew-dependency-cache-authority/1.0",
        "status": "PASS",
        "toolchainDigest": digest_bytes(canonical(tools)),
        "workerIds": sorted(
            spec["buildContract"]["runtimeName"]
            for spec in specs
            if spec["componentKind"] == "language-adapter"
        ),
    }


def prime_dependency_cache(source: Path, root: Path) -> dict[str, object]:
    authority = dependency_cache_authority(source)
    inputs, _development = source_manifest(source)
    tools = toolchain_authority(source)
    specs = runtime_component_specs(
        authority["mode"], inputs, tools, load_component_registry(source)
    )
    temporary = Path(tempfile.mkdtemp(prefix="dependency-prime-", dir=root / "tmp"))
    stage = temporary / "source"
    started = time.monotonic()
    try:
        plan = runtime_build_plan(host_cpu_count(), host_memory_bytes(), "PARALLEL")
        stage_inputs(source, stage, inputs, workers=int(plan["inputWorkers"]))
        environment = build_environment(stage, root, gradle_required=True)
        supervisor = BuildProcessSupervisor()
        gradle_tasks = sorted(
            spec["buildContract"]["task"]
            for spec in specs
            if spec["buildContract"]["executor"] == "GRADLE"
        )
        commands = [
            ("CARGO_DEPENDENCIES", ["cargo", "fetch", "--locked"]),
            (
                "GRADLE_DEPENDENCIES",
                [
                    str(stage / "gradlew"),
                    *gradle_tasks,
                    "--no-daemon",
                    "--no-watch-fs",
                    "--parallel",
                    "--no-build-cache",
                    "-Dorg.gradle.daemon.idletimeout=1000",
                    "-Dorg.gradle.daemon.registry.base="
                    f"{gradle_daemon_registry_base(stage)}",
                    f"-Dorg.gradle.jvmargs={gradle_jvm_options(plan)}",
                    f"--max-workers={plan['gradleWorkers']}",
                    "-Pkotlin.compiler.execution.strategy=in-process",
                    "--quiet",
                ],
            ),
        ]
        stage_timings = {}
        with build_signal_scope(supervisor):
            for name, arguments in commands:
                stage_started = time.monotonic()
                run_build_stage(arguments, stage, environment, name, supervisor)
                stage_timings[name] = int(
                    (time.monotonic() - stage_started) * 1000
                )
        verify_source_manifest(stage, inputs, full_closure=False)
        return {
            **authority,
            "stageWallMillis": dict(sorted(stage_timings.items())),
            "status": "PRIMED",
            "wallMillis": int((time.monotonic() - started) * 1000),
        }
    except BaseException:
        try:
            supervisor.cancel()
        except UnboundLocalError:
            pass
        raise
    finally:
        discard_private_tree(temporary)


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
    if not root.is_absolute() or ".." in root.parts or root == Path(root.anchor):
        raise BootstrapError("temporary cleanup authority is invalid")
    parent_fd = _open_private_tree(root.parent)
    try:
        try:
            metadata = os.stat(root.name, dir_fd=parent_fd, follow_symlinks=False)
        except FileNotFoundError:
            return
        if (
            not stat.S_ISDIR(metadata.st_mode)
            or stat.S_ISLNK(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
        ):
            raise BootstrapError("temporary cleanup root is unsafe")
        if not _remove_tree_at(parent_fd, root.name, metadata):
            raise BootstrapError("temporary cleanup authority changed")
    finally:
        os.close(parent_fd)


def verify_sealed_capsule(capsule: Path) -> None:
    for path in [capsule, *capsule.rglob("*")]:
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise BootstrapError("sealed capsule contains a symlink")
        if stat.S_ISDIR(metadata.st_mode):
            if stat.S_IMODE(metadata.st_mode) != 0o500:
                raise BootstrapError("capsule directory is not sealed read-only")
        elif stat.S_ISREG(metadata.st_mode):
            if stat.S_IMODE(metadata.st_mode) not in {0o400, 0o500}:
                raise BootstrapError("capsule file is not sealed read-only")
        else:
            raise BootstrapError("sealed capsule contains an unsupported entry")


def build_capsule(
    source: Path,
    root: Path,
    key: str,
    mode: str,
    inputs: list[dict[str, object]],
    tools: dict[str, object],
    *,
    build_profile: str = "AUTO",
    evidence: dict[str, object] | None = None,
) -> Path:
    started = time.monotonic()
    plan = runtime_build_plan(host_cpu_count(), host_memory_bytes(), build_profile)
    registry = load_component_registry(source)
    specs = runtime_component_specs(mode, inputs, tools, registry)
    temporary = Path(tempfile.mkdtemp(prefix="capsule-build-", dir=root / "tmp"))
    stage = temporary / "source"
    capsule = temporary / "capsule"
    stage_wall_millis: dict[str, int] = {}
    try:
        build_cache_lock = root / "locks/build-cache.lock"
        with build_cache_lock.open("a+b") as lock:
            os.chmod(build_cache_lock, 0o600)
            fcntl.flock(lock, fcntl.LOCK_EX)
            component_hits = []
            missing_specs = []
            verified_components: dict[
                str, tuple[Path, dict[str, object]]
            ] = {}
            for spec in specs:
                authority = spec["authority"]
                try:
                    verified_components[spec["componentId"]] = verify_component(
                        root, authority["componentKey"], authority
                    )
                    component_hits.append(spec["componentId"])
                except (FileNotFoundError, BootstrapError, OSError, ValueError, TypeError):
                    missing_specs.append(spec)
            build_result = {**plan, "toolchainStages": []}
            if missing_specs:
                phase_started = time.monotonic()
                stage_inputs(
                    source, stage, inputs, workers=int(plan["inputWorkers"])
                )
                stage_wall_millis["INPUT_STAGING"] = int(
                    (time.monotonic() - phase_started) * 1000
                )
                cargo_required = any(
                    spec["buildContract"]["executor"] == "CARGO"
                    for spec in missing_specs
                )
                gradle_tasks = [
                    spec["buildContract"]["task"]
                    for spec in missing_specs
                    if spec["buildContract"]["executor"] == "GRADLE"
                ]
                environment = build_environment(
                    stage, root, gradle_required=bool(gradle_tasks)
                )
                build_result = build_toolchains(
                    stage,
                    environment,
                    plan,
                    cargo_required=cargo_required,
                    gradle_tasks=gradle_tasks,
                )
                stage_wall_millis.update(build_result.get("stageWallMillis", {}))
                verify_source_manifest(stage, inputs, full_closure=False)
                phase_started = time.monotonic()
                outputs = temporary / "component-outputs"
                outputs.mkdir(mode=0o700)
                for spec in missing_specs:
                    contract = spec["buildContract"]
                    if contract["executor"] == "CARGO":
                        output = outputs / spec["componentId"]
                        output.mkdir(mode=0o700)
                        source_binary = (
                            Path(environment["CARGO_TARGET_DIR"])
                            / "release"
                            / contract["binary"]
                        )
                        target = output / contract["artifactName"]
                        shutil.copy2(source_binary, target)
                        os.chmod(target, 0o700)
                    else:
                        output = stage / contract["distribution"]
                        if mode == "RELEASE":
                            verify_release_worker(
                                stage, contract["manifest"], file_rows(output)
                            )
                    publish_component(root, spec["authority"], output)
                    verified_components[spec["componentId"]] = verify_component(
                        root,
                        spec["authority"]["componentKey"],
                        spec["authority"],
                    )
                stage_wall_millis["COMPONENT_PUBLICATION"] = int(
                    (time.monotonic() - phase_started) * 1000
                )

            phase_started = time.monotonic()
            progress("STAGE_STARTED", "ASSEMBLE_VERIFIED_COMPONENTS")
            core_specs = [
                spec for spec in specs if spec["componentKind"] == "core-binary"
            ]
            adapter_specs = [
                spec for spec in specs if spec["componentKind"] == "language-adapter"
            ]
            (capsule / "bin").mkdir(mode=0o700, parents=True)

            def assemble(spec: dict[str, object]) -> None:
                contract = spec["buildContract"]
                destination = (
                    capsule / "bin"
                    if spec["componentKind"] == "core-binary"
                    else capsule / contract["distribution"]
                )
                component, component_manifest = verified_components[
                    spec["componentId"]
                ]
                _materialize_verified_component(
                    component,
                    component_manifest,
                    destination,
                    verify_content=False,
                )

            # The core target directory already exists because it is part of
            # capsule layout; materialize it through a temporary sibling and
            # atomically move each verified artifact into bin.
            for spec in core_specs:
                temporary_core = capsule / f".component-{spec['componentId']}"
                component, component_manifest = verified_components[
                    spec["componentId"]
                ]
                _materialize_verified_component(
                    component,
                    component_manifest,
                    temporary_core,
                    verify_content=False,
                )
                for artifact in temporary_core.iterdir():
                    os.replace(artifact, capsule / "bin" / artifact.name)
                temporary_core.rmdir()
            with ThreadPoolExecutor(
                max_workers=max(1, min(int(plan["packageWorkers"]), len(adapter_specs)))
            ) as executor:
                list(executor.map(assemble, adapter_specs))
            progress("STAGE_COMPLETED", "ASSEMBLE_VERIFIED_COMPONENTS")
            stage_wall_millis["CAPSULE_ASSEMBLY"] = int(
                (time.monotonic() - phase_started) * 1000
            )

        phase_started = time.monotonic()
        workers = {}
        for spec in adapter_specs:
            contract = spec["buildContract"]
            rows = file_rows(capsule / contract["distribution"])
            workers[contract["runtimeName"]] = {
                "compilerVersion": contract["compilerVersion"],
                "protocol": contract["protocol"],
                "distribution": contract["distribution"],
                "treeHash": tree_hash(rows),
                "files": rows,
            }
        private_paths = [source, temporary, stage, root]
        physical_home = Path.home()
        generic_runner_homes = {
            "/" + "home" + "/runner",
            "/" + "Users" + "/runner",
        }
        generic_github_home = (
            os.environ.get("GITHUB_ACTIONS") == "true"
            and physical_home.as_posix() in generic_runner_homes
        )
        # GitHub-hosted runners use a documented generic account path for the
        # shared Cargo cache.  It is stable public infrastructure, not tenant
        # identity.  Source, state, and build roots remain forbidden above.
        if not generic_github_home:
            private_paths.append(physical_home)
        verify_capsule_has_no_private_paths(capsule, private_paths)
        artifacts = {}
        for spec in core_specs:
            name = spec["buildContract"]["artifactName"]
            path = capsule / "bin" / name
            artifacts[name] = {
                "mode": 0o111 if path.stat().st_mode & 0o111 else 0,
                "path": f"bin/{name}",
                "size": path.stat().st_size,
                "sha256": digest_file(path),
            }
        manifest = {
            "schema": SCHEMA,
            "runtimeKey": key,
            "mode": mode,
            "manifestDigest": "",
            "artifactIds": sorted(artifacts),
            "inputDigest": digest_bytes(canonical(inputs)),
            "platformAuthority": tools["platform"],
            "toolchainAuthority": {
                "python": tools["python"],
                "rust": tools["rust"],
                "jdk": tools["jdk"],
            },
            "artifacts": artifacts,
            "components": {
                spec["componentId"]: spec["authority"]["componentKey"]
                for spec in specs
            },
            "workers": workers,
            "workerIds": sorted(workers),
        }
        manifest["manifestDigest"] = digest_bytes(canonical(manifest))
        runtime_manifest = capsule / "runtime.json"
        runtime_manifest.write_bytes(canonical(manifest) + b"\n")
        verify_source_manifest(
            source,
            inputs,
            expected_development=mode == "DEVELOPMENT",
        )
        ready = capsule / "READY"
        ready.write_text(key + "\n")
        # macOS refuses to rename a directory whose own mode is 0500. Seal every
        # descendant first, retain a private 0700 staging root for the rename,
        # then seal that root before releasing the per-key lock.
        seal_capsule(capsule, seal_root=False)
        fsync_tree(capsule)
        destination = root / "runtimes" / key.removeprefix("sha256:")
        if destination.exists():
            verify_capsule(destination, key)
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
        if evidence is not None:
            evidence.update({
                "buildPlan": build_result,
                "componentHits": sorted(component_hits),
                "componentMisses": sorted(
                    spec["componentId"] for spec in missing_specs
                ),
                "stageWallMillis": {
                    **dict(sorted(stage_wall_millis.items())),
                    "CAPSULE_SEAL_AND_VERIFY": int(
                        (time.monotonic() - phase_started) * 1000
                    ),
                },
                "wallMillis": int((time.monotonic() - started) * 1000),
            })
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
    components = manifest.get("components")
    artifacts = manifest.get("artifacts")
    workers = manifest.get("workers")
    if (
        manifest.get("schema") != SCHEMA
        or expected != digest_bytes(canonical(manifest))
        or manifest.get("runtimeKey") != key
        or not isinstance(manifest.get("platformAuthority"), dict)
        or not isinstance(manifest.get("toolchainAuthority"), dict)
        or not isinstance(components, dict)
        or not components
        or not isinstance(artifacts, dict)
        or not artifacts
        or manifest.get("artifactIds") != sorted(artifacts)
        or not isinstance(workers, dict)
        or not workers
        or manifest.get("workerIds") != sorted(workers)
        or any(_component_identifier(name, "artifact id") != name for name in artifacts)
        or any(_component_identifier(name, "worker id") != name for name in workers)
        or any(
            _component_identifier(name, "manifest id") != name
            or not valid_runtime_key(component_key)
            for name, component_key in components.items()
        )
    ):
        raise BootstrapError("runtime manifest authority mismatch")
    manifest["manifestDigest"] = expected
    for artifact in artifacts.values():
        target = path / artifact["path"]
        expected_mode = 0o500 if artifact.get("mode") == 0o111 else 0o400
        if (
            not target.is_file()
            or target.is_symlink()
            or target.stat().st_size != artifact["size"]
            or stat.S_IMODE(target.stat().st_mode) != expected_mode
            or digest_file(target) != artifact["sha256"]
        ):
            raise BootstrapError("runtime executable authority mismatch")
    for worker in workers.values():
        if worker.get("protocol") != "semantic-thread.worker.v1":
            raise BootstrapError("runtime worker protocol authority mismatch")
        distribution = path / worker["distribution"]
        rows = file_rows(distribution)
        if rows != worker["files"] or tree_hash(rows) != worker["treeHash"]:
            raise BootstrapError("runtime worker authority mismatch")
    expected_files = {"runtime.json"}
    expected_files.update(
        str(value["path"]) for value in artifacts.values()
    )
    for worker in workers.values():
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
    try:
        current = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    except FileNotFoundError:
        return False
    if (current.st_dev, current.st_ino) != (expected.st_dev, expected.st_ino):
        return False
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
                        value.get("schema") in GC_SESSION_SCHEMAS
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
    component_preflight = command == ["--bootstrap-component-preflight"]
    dependency_authority = command == ["--bootstrap-dependency-cache-authority"]
    dependency_prime = command == ["--bootstrap-prime-dependency-cache"]
    cold_evidence_profile = None
    if len(command) == 1 and command[0].startswith("--bootstrap-cold-build-evidence="):
        requested = command[0].split("=", 1)[1].upper()
        if requested not in {"SERIAL", "PARALLEL"}:
            raise BootstrapError("cold build evidence profile must be serial or parallel")
        cold_evidence_profile = requested
    warm_audit = command == ["--bootstrap-warm-audit"]
    source = known.source_root.resolve(strict=True)
    if dependency_authority:
        print(canonical(dependency_cache_authority(source)).decode())
        return 0
    if component_preflight:
        inputs, development = source_manifest(source)
        mode = "DEVELOPMENT" if development else "RELEASE"
        tools = toolchain_authority(source)
        specs = runtime_component_specs(
            mode, inputs, tools, load_component_registry(source)
        )
        parallel_plan = runtime_build_plan(
            host_cpu_count(), host_memory_bytes(), "PARALLEL"
        )
        # Derive the same fail-closed path invariant used by cold/prime builds.
        # Gradle 9.6 rejects a relative registry path before compilation.
        gradle_daemon_registry_base(source)
        print(canonical({
            "componentIds": sorted(spec["componentId"] for spec in specs),
            "mode": mode,
            "parallelBuildPlan": parallel_plan,
            "schema": "codeclew-runtime-component-preflight/2.0",
            "status": "PASS",
        }).decode())
        return 0
    root, state_fd = state_root()
    if dependency_prime:
        try:
            print(canonical(prime_dependency_cache(source, root)).decode())
        finally:
            os.close(state_fd)
        return 0
    external_seed = os.environ.get("CODECLEW_RUNTIME_SEED") is not None
    if external_seed and cold_evidence_profile is not None:
        raise BootstrapError("cold build evidence cannot use a sealed runtime seed")
    path_to_checkpoint = checkpoint_path(root, source)
    checkpoint_key = (
        None
        if cold_evidence_profile is not None
        else read_checkpoint_candidate_key(path_to_checkpoint, root)
    )
    checkpoint = None
    lease = None
    cold_toolchain_invoked = False
    capsule_build_invoked = False
    cold_build_evidence: dict[str, object] = {}
    if external_seed:
        key, capsule, lease = sealed_runtime_seed(source)
        checkpoint = {"externalSeed": True, "runtimeKey": key}
    elif checkpoint_key is not None:
        checkpoint_lock = root / "locks" / (
            f"runtime-{checkpoint_key.removeprefix('sha256:')}.lock"
        )
        with checkpoint_lock.open("a+b") as lock:
            os.chmod(checkpoint_lock, 0o600)
            fcntl.flock(lock, fcntl.LOCK_EX)
            checkpoint = read_valid_checkpoint(path_to_checkpoint, source, root)
            if checkpoint is None:
                checkpoint = revalidate_checkpoint_capsule(
                    source, root, checkpoint_key
                )
            if checkpoint is not None:
                key = str(checkpoint["runtimeKey"])
                capsule = Path(str(checkpoint["capsule"]))
                lease_path = root / "locks" / (
                    f"runtime-{key.removeprefix('sha256:')}.lease"
                )
                lease = lease_path.open("a+b")
                os.chmod(lease_path, 0o600)
                fcntl.flock(lease, fcntl.LOCK_SH)
    if checkpoint is None and (cleanup_id := cleanup_session_id(command)) is not None:
        key, capsule, lease = sealed_session_cleanup_runtime(root, cleanup_id)
        checkpoint = {"sessionCleanup": True, "runtimeKey": key}
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
        if cold_evidence_profile is not None and capsule.exists():
            raise BootstrapError("cold build evidence requires a fresh CODECLEW_HOME")
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
                prepare_cold_build_capacity(root, key)
                if tools is None:
                    tools = toolchain_authority(source)
                    cold_toolchain_invoked = True
                    rebuilt_key = runtime_key(mode, inputs, tools)
                    if rebuilt_key != key:
                        raise BootstrapError(
                            "runtime locator disagrees with cold toolchain authority"
                        )
                capsule_build_invoked = True
                capsule = build_capsule(
                    source,
                    root,
                    key,
                    mode,
                    inputs,
                    tools,
                    build_profile=cold_evidence_profile or "AUTO",
                    evidence=cold_build_evidence if cold_evidence_profile is not None else None,
                )
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
    if cold_evidence_profile is not None:
        manifest = verify_capsule(capsule, key)
        lease.close()
        print(canonical({
            "schema": "codeclew-real-cold-build-evidence/1.0",
            "status": "MEASURED",
            "mode": manifest["mode"],
            "runtimeKey": key,
            "manifestDigest": manifest["manifestDigest"],
            "artifactHashes": {
                name: value["sha256"]
                for name, value in sorted(manifest["artifacts"].items())
            },
            "workerTreeHashes": {
                name: value["treeHash"]
                for name, value in sorted(manifest["workers"].items())
            },
            **cold_build_evidence,
        }).decode())
        return 0
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
