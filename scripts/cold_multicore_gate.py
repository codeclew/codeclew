#!/usr/bin/env python3
"""Matched, cache-authoritative cold capsule benchmark for multicore hosts."""

from __future__ import annotations

import contextlib
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import secrets
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass
from typing import BinaryIO

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "bootstrap"))
from bounded_gate_cleanup import cleanup_tree as bounded_gate_cleanup  # noqa: E402
from cold_cache_authority import (  # noqa: E402
    CacheAuthorityError,
    clone_seed,
    create_seed_candidate,
    probe_cow,
    publish_seed,
    recover_seed,
    seed_creation_lock,
)
from private_diagnostic_store import (  # noqa: E402
    DiagnosticStoreError,
    store_diagnostic_bytes,
)
from host_resources import HostResourceError, effective_host_resources  # noqa: E402


REPORT_SCHEMA = "codeclew-real-cold-runtime-gate/2.0"
ARM_SCHEMA = "codeclew-cold-runtime-arm/2.0"
COHORT_SCHEMA = "codeclew-cold-runtime-cohort/1.0"
COHORT_COMPLETE_SCHEMA = "codeclew-cold-runtime-cohort-complete/1.0"
LOAD_SCHEMA = "codeclew-cold-runtime-load-authority/1.0"
BOOT_SCHEMA = "codeclew-host-boot-authority/1.0"
HOST_SCHEMA = "codeclew-cold-runtime-host-authority/1.0"
CRITICAL_RATIO_MAX = 0.65
BLOCK_RATIO_MAX = 0.85
TOTAL_RATIO_MAX = 0.65
ORDER_INTERACTION_MAX = 1.5
UNQUALIFIED_EXIT_CODE = 2
MAX_PROCESS_OUTPUT = 1024 * 1024
MAX_RECEIPT_BYTES = 4 * 1024 * 1024
COMMAND_TERMINATION_GRACE_SECONDS = 2.0
COMMAND_KILL_WAIT_SECONDS = 2.0
STALE_CLEANUP_TIMEOUT = 30
GRADLE_MIN_HEAP_BYTES = 2 * 1024**3
GRADLE_MAX_HEAP_BYTES = 8 * 1024**3
GRADLE_NON_HEAP_BYTES = 2 * 1024**3
TOOLCHAIN_WORKER_MEMORY_BYTES = 1536 * 1024**2
TOOLCHAIN_STAGES = ("GRADLE_WORKERS", "CARGO_BINARIES")
PRIME_STAGES = ("CARGO_DEPENDENCIES", "GRADLE_DEPENDENCIES")
CAPSULE_STAGES = (
    "INPUT_STAGING",
    "GRADLE_WORKERS",
    "CARGO_BINARIES",
    "COMPONENT_PUBLICATION",
    "CAPSULE_ASSEMBLY",
    "CAPSULE_SEAL_AND_VERIFY",
)
ARMS = (
    ("block-1-serial", 1, ["SERIAL", "PARALLEL"], "SERIAL"),
    ("block-1-parallel", 1, ["SERIAL", "PARALLEL"], "PARALLEL"),
    ("block-2-parallel", 2, ["PARALLEL", "SERIAL"], "PARALLEL"),
    ("block-2-serial", 2, ["PARALLEL", "SERIAL"], "SERIAL"),
)
DIGEST_PATTERN = re.compile(r"sha256:[0-9a-f]{64}")
REVISION_PATTERN = re.compile(r"[0-9a-f]{40}")
COHORT_DIRECTORY_PATTERN = re.compile(r"cohort-([0-9a-f]{64})")
COHORT_CANDIDATE_PATTERN = re.compile(r"\.candidate-[0-9a-f]{64}")
RUN_DIRECTORY_PATTERN = re.compile(r"run\.[A-Za-z0-9_-]{6,64}")
SAFE_IDENTIFIER_PATTERN = re.compile(r"[A-Za-z0-9._-]{1,128}")
ARM_DIGEST_DOMAIN = b"codeclew-cold-runtime-arm/v2\0"
COHORT_DIGEST_DOMAIN = b"codeclew-cold-runtime-cohort/v1\0"
AUTHORITY_DIGEST_DOMAIN = b"codeclew-cold-runtime-authority/v1\0"

BUILD_AUTHORITY_FIELDS = frozenset(
    {
        "artifactIds", "componentIds", "inputDigest", "mode", "runtimeKey", "schema",
        "status", "toolchainDigest", "workerIds",
    }
)
PLAN_FIELDS = frozenset(
    {
        "cargoWorkers", "gradleHeapBytes", "gradleWorkers", "inputWorkers", "memoryBudgetBytes",
        "packageWorkers", "parallel", "profile", "stageWallMillis",
        "toolchainCriticalPathMillis", "toolchainStages", "toolchainWallMillis",
    }
)
COLD_EVIDENCE_FIELDS = frozenset(
    {
        "artifactHashes", "buildPlan", "componentHits", "componentMisses",
        "manifestDigest", "mode", "runtimeKey", "schema", "stageWallMillis",
        "status", "wallMillis", "workerTreeHashes",
    }
)
WARM_AUDIT_FIELDS = frozenset(
    {
        "capsuleBuildInvoked", "coldToolchainInvoked", "counters",
        "forbiddenWarmProcesses", "schema", "status",
    }
)
WARM_COUNTER_FIELDS = frozenset(
    {
        "checkpointHits", "checkpointMisses", "digestFileCalls", "metadataChecks",
        "processRuns",
    }
)
LOAD_FIELDS = frozenset(
    {"after", "before", "bootAuthorityDigest", "physicalCores", "schema"}
)
LOAD_SNAPSHOT_FIELDS = frozenset({"capturedMonotonicNanos", "loadAverage"})
ARM_FIELDS = frozenset(
    {
        "armDigest", "armId", "artifactHashes", "artifactIds", "block", "bootAuthorityDigest",
        "buildAuthorityDigest", "buildPlan", "cacheSeedDigest", "capsuleWallMillis",
        "cohortId", "componentHits", "componentIds", "componentMisses",
        "criticalPathMillis", "hostAuthorityDigest", "loadAuthority", "logicalCores",
        "manifestDigest", "order", "outerWallMillis", "physicalCores",
        "predecessorArmDigest", "qualificationCores",
        "profile", "runtimeKey", "schema", "sequence", "sourceRevision",
        "stageWallMillis", "status", "warmAudit", "workerIds", "workerTreeHashes",
        "totalMemoryBytes",
    }
)
COHORT_FIELDS = frozenset(
    {
        "armOrder", "bootAuthority", "bootAuthorityDigest", "buildAuthority",
        "buildAuthorityDigest", "cacheSeedDigest", "cohortId", "cohortNonce",
        "createdUnixNanos", "hostAuthority", "hostAuthorityDigest", "schema",
        "sourceRevision", "status",
    }
)
COHORT_COMPLETE_FIELDS = frozenset(
    {"armCount", "cohortId", "finalArmDigest", "reportDigest", "schema", "status"}
)


class GateError(RuntimeError):
    def __init__(self, stage: str, message: str, diagnostic: bytes | None = None):
        super().__init__(message)
        self.stage = stage
        self.diagnostic = diagnostic


class GateInterrupted(BaseException):
    def __init__(self, signum: int):
        self.signum = signum
        super().__init__(f"cold gate interrupted by signal {signum}")


@dataclass(frozen=True)
class RunResult:
    returncode: int
    stderr: bytes
    stderr_truncated: bool
    stdout: bytes
    stdout_truncated: bool


@dataclass
class LockHandle:
    descriptor: int
    label: Path
    identity: tuple[int, ...]

    def verify(self) -> None:
        try:
            held = os.fstat(self.descriptor)
            probe = _open_private_directory_fd(self.label.parent, create=False)
        except (OSError, GateError) as error:
            raise GateError("COHORT_ADMISSION", "directory lock authority disappeared") from error
        try:
            observed = os.fstat(probe)
            label_exists = False
            for name in (self.label.name, f"{self.label.name}.authority"):
                try:
                    os.stat(name, dir_fd=self.descriptor, follow_symlinks=False)
                    label_exists = True
                except FileNotFoundError:
                    pass
            if (
                _directory_lock_identity(held) != self.identity
                or _directory_lock_identity(observed) != self.identity
                or label_exists
            ):
                raise GateError("COHORT_ADMISSION", "directory lock authority changed")
        finally:
            os.close(probe)

    def close(self) -> None:
        os.close(self.descriptor)


@dataclass
class CohortHandle:
    path: Path
    arms: Path
    authority: dict[str, object]
    lock: LockHandle

    def close(self) -> None:
        self.lock.close()


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def digest(value: object) -> str:
    return "sha256:" + hashlib.sha256(canonical(value)).hexdigest()


def domain_digest(domain: bytes, value: object) -> str:
    return "sha256:" + hashlib.sha256(domain + canonical(value)).hexdigest()


def valid_digest(value: object) -> bool:
    return isinstance(value, str) and DIGEST_PATTERN.fullmatch(value) is not None


def exact_nonnegative_int(value: object) -> bool:
    return type(value) is int and value >= 0


def exact_positive_int(value: object) -> bool:
    return type(value) is int and value > 0


def physical_cores() -> int:
    if platform.system() == "Darwin":
        completed = run(
            ["/usr/sbin/sysctl", "-n", "hw.physicalcpu"], Path("/"), os.environ.copy()
        )
        if (
            completed.returncode == 0
            and not completed.stdout_truncated
            and not completed.stderr_truncated
        ):
            try:
                return int(completed.stdout.strip())
            except ValueError:
                return 0
    if platform.system() == "Linux":
        pairs: set[tuple[str, str]] = set()
        physical = core = None
        try:
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
        except OSError:
            return 0
        return len(pairs)
    return 0


def host_memory_bytes() -> int:
    try:
        return int(effective_host_resources()["totalMemoryBytes"])
    except (HostResourceError, KeyError, TypeError, ValueError) as error:
        raise GateError(
            "PREFLIGHT_HOST_AUTHORITY", "host memory authority is unavailable"
        ) from error


def memory_budget_bytes(total_memory: int) -> int:
    reserved = max(1024**3, total_memory * 15 // 100)
    return max(0, total_memory - reserved) * 70 // 100


def expected_build_resources(
    logical_cores: int, total_memory: int, profile: str
) -> dict[str, int]:
    budget = memory_budget_bytes(total_memory)
    if profile == "PARALLEL":
        gradle_budget = budget * 55 // 100
        cargo_budget = budget - gradle_budget
        gradle_heap = min(
            GRADLE_MAX_HEAP_BYTES,
            max(0, gradle_budget - GRADLE_NON_HEAP_BYTES),
        ) // 1024**2 * 1024**2
        desired_cargo = max(1, logical_cores // 2)
        desired_gradle = max(1, logical_cores - desired_cargo)
        return {
            "cargoWorkers": min(
                desired_cargo,
                max(1, cargo_budget // TOOLCHAIN_WORKER_MEMORY_BYTES),
            ),
            "gradleHeapBytes": gradle_heap,
            "gradleWorkers": min(
                desired_gradle,
                max(1, gradle_heap // TOOLCHAIN_WORKER_MEMORY_BYTES),
            ),
            "inputWorkers": min(8, logical_cores),
            "memoryBudgetBytes": budget,
            "packageWorkers": min(8, logical_cores),
        }
    gradle_heap = min(
        GRADLE_MAX_HEAP_BYTES,
        max(0, budget - GRADLE_NON_HEAP_BYTES),
    ) // 1024**2 * 1024**2
    return {
        "cargoWorkers": 1,
        "gradleHeapBytes": gradle_heap,
        "gradleWorkers": 1,
        "inputWorkers": 1,
        "memoryBudgetBytes": budget,
        "packageWorkers": 1,
    }


def _directory_flags() -> int:
    return os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0)


def _open_private_directory_fd(path: Path, *, create: bool = True) -> int:
    """Return a retained fd for an absolute private tree without path publication."""
    normalized = Path(os.path.normpath(os.fspath(path)))
    if not path.is_absolute() or path != normalized or path == Path("/") or ".." in path.parts:
        raise GateError("PREFLIGHT_PRIVATE_STATE", "private state path is not normalized")
    descriptor = os.open("/", _directory_flags())
    try:
        parts = [part for part in path.parts if part != path.anchor]
        for index, component in enumerate(parts):
            created = False
            try:
                child = os.open(component, _directory_flags(), dir_fd=descriptor)
            except FileNotFoundError:
                if not create:
                    raise
                try:
                    os.mkdir(component, mode=0o700, dir_fd=descriptor)
                    created = True
                except FileExistsError:
                    pass
                try:
                    child = os.open(component, _directory_flags(), dir_fd=descriptor)
                except (NotADirectoryError, OSError) as error:
                    raise GateError(
                        "PREFLIGHT_PRIVATE_STATE",
                        "private state contains a symlink or non-directory ancestor",
                    ) from error
            except (NotADirectoryError, OSError) as error:
                raise GateError(
                    "PREFLIGHT_PRIVATE_STATE",
                    "private state contains a symlink or non-directory ancestor",
                ) from error
            metadata = os.fstat(child)
            try:
                by_name = os.stat(component, dir_fd=descriptor, follow_symlinks=False)
            except OSError:
                os.close(child)
                raise GateError(
                    "PREFLIGHT_PRIVATE_STATE", "private state binding disappeared"
                )
            leaf = index == len(parts) - 1
            mode = stat.S_IMODE(metadata.st_mode)
            sticky_root = metadata.st_uid == 0 and bool(mode & stat.S_ISVTX)
            if (
                not stat.S_ISDIR(metadata.st_mode)
                or (metadata.st_dev, metadata.st_ino)
                != (by_name.st_dev, by_name.st_ino)
                or metadata.st_uid not in {0, os.geteuid()}
                or (not leaf and mode & 0o022 and not sticky_root)
            ):
                os.close(child)
                raise GateError("PREFLIGHT_PRIVATE_STATE", "private state ancestor is unsafe")
            if leaf:
                if metadata.st_uid != os.geteuid():
                    os.close(child)
                    raise GateError("PREFLIGHT_PRIVATE_STATE", "private state owner is unsafe")
                if created:
                    os.fchmod(child, 0o700)
                if stat.S_IMODE(os.fstat(child).st_mode) != 0o700:
                    os.close(child)
                    raise GateError("PREFLIGHT_PRIVATE_STATE", "private state mode is unsafe")
            os.close(descriptor)
            descriptor = child
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def private_directory(path: Path) -> Path:
    descriptor = _open_private_directory_fd(path)
    os.close(descriptor)
    return path


def _fsync_directory(path: Path) -> None:
    descriptor = os.open(path, _directory_flags())
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def atomic_json(path: Path, value: object, mode: int = 0o600) -> None:
    parent = _open_private_directory_fd(path.parent)
    temporary = f".{path.name}.{os.getpid()}.{secrets.token_hex(12)}.tmp"
    try:
        descriptor = os.open(
            temporary,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
            mode,
            dir_fd=parent,
        )
        try:
            payload = canonical(value) + b"\n"
            offset = 0
            while offset < len(payload):
                offset += os.write(descriptor, payload[offset:])
            os.fchmod(descriptor, mode)
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        os.replace(temporary, path.name, src_dir_fd=parent, dst_dir_fd=parent)
        os.fsync(parent)
    finally:
        try:
            os.unlink(temporary, dir_fd=parent)
        except FileNotFoundError:
            pass
        os.close(parent)


def immutable_json(path: Path, value: object) -> None:
    """Publish a canonical 0400 receipt atomically without replacement."""
    parent = _open_private_directory_fd(path.parent)
    temporary = f".{path.name}.{os.getpid()}.{secrets.token_hex(12)}.tmp"
    try:
        descriptor = os.open(
            temporary,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
            0o600,
            dir_fd=parent,
        )
        try:
            payload = canonical(value) + b"\n"
            offset = 0
            while offset < len(payload):
                offset += os.write(descriptor, payload[offset:])
            os.fchmod(descriptor, 0o400)
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        try:
            os.link(
                temporary,
                path.name,
                src_dir_fd=parent,
                dst_dir_fd=parent,
                follow_symlinks=False,
            )
        except FileExistsError as error:
            raise GateError("ARM_RECEIPT", "immutable receipt already exists") from error
        os.unlink(temporary, dir_fd=parent)
        os.fsync(parent)
    finally:
        try:
            os.unlink(temporary, dir_fd=parent)
        except FileNotFoundError:
            pass
        os.close(parent)


def _read_exact_json(
    path: Path, expected_fields: frozenset[str] | None, mode: int = 0o400
) -> dict[str, object] | None:
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    parent = None
    try:
        parent = _open_private_directory_fd(path.parent, create=False)
        before = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
        descriptor = os.open(path.name, flags, dir_fd=parent)
        metadata = os.fstat(descriptor)
        identity = lambda item: (  # noqa: E731
            item.st_dev, item.st_ino, item.st_mode, item.st_uid, item.st_nlink,
            item.st_size, item.st_mtime_ns, item.st_ctime_ns,
        )
        if (
            not stat.S_ISREG(metadata.st_mode)
            or stat.S_ISLNK(before.st_mode)
            or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) != mode
            or metadata.st_nlink != 1
            or metadata.st_size > MAX_RECEIPT_BYTES
            or identity(metadata) != identity(before)
        ):
            os.close(descriptor)
            return None
        buffer = bytearray()
        try:
            while len(buffer) <= MAX_RECEIPT_BYTES:
                block = os.read(
                    descriptor,
                    min(64 * 1024, MAX_RECEIPT_BYTES + 1 - len(buffer)),
                )
                if not block:
                    break
                buffer.extend(block)
            after_fd = os.fstat(descriptor)
        finally:
            os.close(descriptor)
        after_path = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
        if identity(after_fd) != identity(metadata) or identity(after_path) != identity(metadata):
            return None
        payload = bytes(buffer)
        value = json.loads(payload)
    except (FileNotFoundError, OSError, GateError, ValueError, TypeError):
        return None
    finally:
        if parent is not None:
            os.close(parent)
    if not isinstance(value, dict) or (
        expected_fields is not None and set(value) != expected_fields
    ):
        return None
    if payload != canonical(value) + b"\n":
        return None
    return value


def _process_group_exists(process_group: int) -> bool:
    try:
        os.killpg(process_group, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def _terminate_process_group(process: subprocess.Popen[bytes]) -> None:
    process_group = process.pid
    try:
        os.killpg(process_group, signal.SIGTERM)
    except ProcessLookupError:
        process.poll()
        return
    deadline = time.monotonic() + COMMAND_TERMINATION_GRACE_SECONDS
    while _process_group_exists(process_group) and time.monotonic() < deadline:
        process.poll()
        time.sleep(0.02)
    if _process_group_exists(process_group):
        try:
            os.killpg(process_group, signal.SIGKILL)
        except ProcessLookupError:
            pass
    deadline = time.monotonic() + COMMAND_KILL_WAIT_SECONDS
    while _process_group_exists(process_group) and time.monotonic() < deadline:
        process.poll()
        time.sleep(0.02)
    try:
        process.wait(timeout=0)
    except (subprocess.TimeoutExpired, ChildProcessError):
        pass


@contextlib.contextmanager
def _command_signal_scope(
    process: subprocess.Popen[bytes],
    previous_mask: set[signal.Signals] | None,
):
    if threading.current_thread() is not threading.main_thread():
        yield {"armed": False, "seen": False, "signum": None}
        return
    previous_handlers: dict[signal.Signals, object] = {}
    state: dict[str, object] = {"armed": False, "seen": False, "signum": None}

    def interrupted(signum: int, _frame: object) -> None:
        state["seen"] = True
        state["signum"] = signum
        if state["armed"]:
            raise GateInterrupted(signum)

    try:
        for signum in (signal.SIGINT, signal.SIGTERM):
            previous_handlers[signum] = signal.getsignal(signum)
            signal.signal(signum, interrupted)
        if previous_mask is not None:
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
        yield state
    finally:
        managed = {signal.SIGINT, signal.SIGTERM}
        if previous_mask is not None:
            managed.difference_update(previous_mask)
        try:
            if previous_mask is not None and managed:
                # Linearize completion before restoring dispositions. Signals
                # arriving from this point remain pending and are delivered to
                # the caller's original handler after the atomic handoff.
                signal.pthread_sigmask(signal.SIG_BLOCK, managed)
        finally:
            try:
                for signum, handler in previous_handlers.items():
                    signal.signal(signum, handler)
            finally:
                if previous_mask is not None:
                    signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)


def _spawn_command(
    command: list[str], cwd: Path, environment: dict[str, str]
) -> tuple[subprocess.Popen[bytes], set[signal.Signals] | None]:
    previous_mask = None
    child_setup = None
    if (
        threading.current_thread() is threading.main_thread()
        and hasattr(signal, "pthread_sigmask")
    ):
        previous_mask = signal.pthread_sigmask(
            signal.SIG_BLOCK, {signal.SIGINT, signal.SIGTERM}
        )

        def child_setup() -> None:
            assert previous_mask is not None
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)

    try:
        process = subprocess.Popen(
            command,
            cwd=cwd,
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            preexec_fn=child_setup,
        )
        return process, previous_mask
    except BaseException:
        if previous_mask is not None:
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
        raise


def run(command: list[str], cwd: Path, environment: dict[str, str]) -> RunResult:
    """Run one owned process group while retaining bounded output."""
    process, previous_mask = _spawn_command(command, cwd, environment)
    outputs: dict[str, bytearray] = {"stdout": bytearray(), "stderr": bytearray()}
    truncated = {"stdout": False, "stderr": False}

    def drain(name: str, stream: BinaryIO) -> None:
        try:
            while block := stream.read(64 * 1024):
                remaining = MAX_PROCESS_OUTPUT - len(outputs[name])
                if remaining > 0:
                    outputs[name].extend(block[:remaining])
                if len(block) > max(0, remaining):
                    truncated[name] = True
        except (OSError, ValueError):
            return

    threads: list[threading.Thread] = []
    interrupted = False
    termination_attempted = False
    try:
        assert process.stdout is not None and process.stderr is not None
        threads = [
            threading.Thread(target=drain, args=("stdout", process.stdout), daemon=True),
            threading.Thread(target=drain, args=("stderr", process.stderr), daemon=True),
        ]
        for thread in threads:
            thread.start()
        with _command_signal_scope(process, previous_mask) as signal_state:
            signal_state["armed"] = True
            try:
                if signal_state["seen"]:
                    raise GateInterrupted(int(signal_state["signum"]))
                returncode = process.wait()
                if _process_group_exists(process.pid):
                    raise GateError(
                        "PROCESS_RESIDUAL", "child process left a residual process group"
                    )
            except GateInterrupted as error:
                interrupted = True
                signal_state["armed"] = False
                _terminate_process_group(process)
                termination_attempted = True
                raise GateError(
                    "PROCESS_CANCELLED", "cold gate command was cancelled"
                ) from error
            except BaseException:
                interrupted = True
                signal_state["armed"] = False
                _terminate_process_group(process)
                termination_attempted = True
                raise
    except BaseException:
        interrupted = True
        if not termination_attempted:
            _terminate_process_group(process)
        raise
    finally:
        if (
            previous_mask is not None
            and threading.current_thread() is threading.main_thread()
        ):
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
        for thread in threads:
            thread.join(timeout=5)
        if any(thread.is_alive() for thread in threads):
            process.stdout.close()
            process.stderr.close()
            for thread in threads:
                thread.join(timeout=1)
            if not interrupted:
                raise GateError("PROCESS_OUTPUT", "child process retained an output pipe")
        process.stdout.close()
        process.stderr.close()
    return RunResult(
        returncode=returncode,
        stderr=bytes(outputs["stderr"]),
        stderr_truncated=truncated["stderr"],
        stdout=bytes(outputs["stdout"]),
        stdout_truncated=truncated["stdout"],
    )


def _diagnostic(result: RunResult) -> bytes:
    value = bytearray(result.stderr)
    if result.stderr_truncated:
        value.extend(b"\n[stderr truncated by cold gate]\n")
    return bytes(value)


def _json_output(result: RunResult, stage: str) -> dict[str, object]:
    if result.stdout_truncated or result.stderr_truncated:
        raise GateError(stage, "bounded command output was truncated", _diagnostic(result))
    try:
        value = json.loads(result.stdout)
    except (ValueError, TypeError) as error:
        raise GateError(stage, "command JSON output is invalid", result.stdout) from error
    if not isinstance(value, dict) or result.stdout != canonical(value) + b"\n":
        raise GateError(stage, "command JSON output is not canonical", result.stdout)
    return value


def clean_process_preflight() -> None:
    completed = run(["/bin/ps", "-axo", "command="], Path("/"), os.environ.copy())
    if (
        completed.returncode != 0
        or completed.stdout_truncated
        or completed.stderr_truncated
    ):
        raise GateError("PREFLIGHT_PROCESS_AUDIT", "process audit is unavailable")
    forbidden = ("org.gradle.launcher.daemon.bootstrap.GradleDaemon", "KotlinCompileDaemon")
    if any(marker in line for marker in forbidden for line in completed.stdout.decode().splitlines()):
        raise GateError(
            "PREFLIGHT_PROCESS_AUDIT",
            "ambient Gradle or Kotlin daemon would contaminate cold evidence",
        )


def frozen_source(repository: Path, work: Path, revision: str) -> Path:
    source = work / "source"
    completed = run(
        ["git", "clone", "--quiet", "--no-local", "--no-checkout", str(repository), str(source)],
        repository,
        os.environ.copy(),
    )
    if completed.returncode != 0 or completed.stdout_truncated or completed.stderr_truncated:
        raise GateError("PREFLIGHT_SOURCE_CLONE", "frozen source clone failed", _diagnostic(completed))
    completed = run(["git", "checkout", "--quiet", "--detach", revision], source, os.environ.copy())
    if completed.returncode != 0 or completed.stdout_truncated or completed.stderr_truncated:
        raise GateError("PREFLIGHT_SOURCE_CLONE", "frozen source checkout failed", _diagnostic(completed))
    observed = run(["git", "rev-parse", "--verify", "HEAD"], source, os.environ.copy())
    dirty = run(["git", "status", "--porcelain=v1", "--untracked-files=all"], source, os.environ.copy())
    alternates = source / ".git" / "objects" / "info" / "alternates"
    if (
        observed.returncode != 0
        or observed.stdout_truncated
        or observed.stderr_truncated
        or observed.stdout.decode().strip() != revision
        or dirty.returncode != 0
        or dirty.stdout_truncated
        or dirty.stderr_truncated
        or dirty.stdout
        or alternates.exists()
        or alternates.is_symlink()
    ):
        raise GateError("PREFLIGHT_SOURCE_CLONE", "frozen source authority is invalid")
    return source


def isolated_environment(cache_home: Path, state_home: Path, original_home: Path) -> dict[str, str]:
    environment = os.environ.copy()
    environment.update(
        {
            "CARGO_HOME": str(cache_home / ".cargo"),
            "CODECLEW_HOME": str(state_home),
            "GRADLE_USER_HOME": str(cache_home / ".gradle"),
            "HOME": str(cache_home),
            "XDG_CACHE_HOME": str(cache_home / ".cache"),
            "XDG_CONFIG_HOME": str(cache_home / ".config"),
            "XDG_DATA_HOME": str(cache_home / ".local" / "share"),
        }
    )
    rustup = Path(environment.get("RUSTUP_HOME", str(original_home / ".rustup")))
    if rustup.is_dir():
        environment["RUSTUP_HOME"] = str(rustup)
    return environment


def validate_build_authority(value: object) -> dict[str, object]:
    if not isinstance(value, dict) or set(value) != BUILD_AUTHORITY_FIELDS:
        raise GateError("PREFLIGHT_CACHE_AUTHORITY", "dependency cache authority schema is invalid")
    components = value.get("componentIds")
    artifacts = value.get("artifactIds")
    workers = value.get("workerIds")
    if (
        value.get("schema") != "codeclew-dependency-cache-authority/1.0"
        or value.get("status") != "PASS"
        or value.get("mode") != "RELEASE"
        or not valid_digest(value.get("runtimeKey"))
        or not valid_digest(value.get("inputDigest"))
        or not valid_digest(value.get("toolchainDigest"))
        or not _safe_identifiers(artifacts)
        or not _safe_identifiers(components)
        or not _safe_identifiers(workers)
    ):
        raise GateError("PREFLIGHT_CACHE_AUTHORITY", "dependency cache authority is invalid")
    return value


def dependency_authority(source: Path) -> dict[str, object]:
    completed = run(
        [str(source / "clew"), "--bootstrap-dependency-cache-authority"],
        source,
        os.environ.copy(),
    )
    if completed.returncode != 0:
        raise GateError(
            "PREFLIGHT_CACHE_AUTHORITY", "dependency cache authority failed", _diagnostic(completed)
        )
    return validate_build_authority(_json_output(completed, "PREFLIGHT_CACHE_AUTHORITY"))


def validate_prime_evidence(
    evidence: object, authority: dict[str, object]
) -> dict[str, object]:
    expected_fields = BUILD_AUTHORITY_FIELDS | {"stageWallMillis", "wallMillis"}
    if not isinstance(evidence, dict) or set(evidence) != expected_fields:
        raise GateError("DEPENDENCY_CACHE_PRIME", "dependency prime schema is not exact")
    for name in BUILD_AUTHORITY_FIELDS - {"status"}:
        if evidence.get(name) != authority.get(name):
            raise GateError(
                "DEPENDENCY_CACHE_PRIME",
                "dependency prime differs from its build authority",
            )
    timings = evidence.get("stageWallMillis")
    if (
        evidence.get("status") != "PRIMED"
        or not _timing_map(timings, PRIME_STAGES, positive=True)
        or not exact_positive_int(evidence.get("wallMillis"))
        or int(evidence["wallMillis"]) < sum(int(timings[name]) for name in PRIME_STAGES)
    ):
        raise GateError(
            "DEPENDENCY_CACHE_PRIME",
            "dependency prime timing authority is invalid",
        )
    return evidence


def prepare_cache_seed(
    source: Path,
    work: Path,
    control_home: Path,
    authority: dict[str, object],
    original_home: Path,
) -> tuple[Path, dict[str, object]]:
    key = str(authority["runtimeKey"])
    store = control_home / "qualification" / "cold-runtime" / "cache-seeds"
    with seed_creation_lock(store, key):
        try:
            recovered = recover_seed(store, key)
        except FileNotFoundError:
            recovered = None
        if recovered is not None:
            return recovered
        candidate = create_seed_candidate(store, key)
        candidate_metadata = candidate.lstat()
        try:
            for relative in (".cargo", ".gradle", ".cache", ".config", ".local/share"):
                (candidate / relative).mkdir(mode=0o700, parents=True, exist_ok=True)
            prime_state = work / "prime-state"
            environment = isolated_environment(candidate, prime_state, original_home)
            completed = run(
                [str(source / "clew"), "--bootstrap-prime-dependency-cache"],
                source,
                environment,
            )
            if completed.returncode != 0:
                raise GateError(
                    "DEPENDENCY_CACHE_PRIME",
                    "dependency cache prime failed",
                    _diagnostic(completed),
                )
            validate_prime_evidence(
                _json_output(completed, "DEPENDENCY_CACHE_PRIME"), authority
            )
            return publish_seed(candidate, store, key)
        except Exception:
            if candidate.exists() and not candidate.is_symlink():
                cleanup_owned_tree(
                    candidate,
                    store,
                    (candidate_metadata.st_dev, candidate_metadata.st_ino),
                )
            raise


def boot_authority() -> dict[str, object]:
    system = platform.system()
    if system == "Linux":
        try:
            identity = Path("/proc/sys/kernel/random/boot_id").read_text(encoding="ascii").strip()
        except OSError as error:
            raise GateError("PREFLIGHT_BOOT_AUTHORITY", "boot identity is unavailable") from error
        if re.fullmatch(r"[0-9a-fA-F-]{36}", identity) is None:
            raise GateError("PREFLIGHT_BOOT_AUTHORITY", "boot identity is invalid")
    elif system == "Darwin":
        completed = run(["/usr/sbin/sysctl", "-n", "kern.boottime"], Path("/"), os.environ.copy())
        if completed.returncode != 0 or completed.stdout_truncated:
            raise GateError("PREFLIGHT_BOOT_AUTHORITY", "boot identity is unavailable")
        identity = completed.stdout.decode().strip()
        if not identity:
            raise GateError("PREFLIGHT_BOOT_AUTHORITY", "boot identity is invalid")
    else:
        raise GateError("PREFLIGHT_BOOT_AUTHORITY", "unsupported host boot authority")
    return {"identity": identity, "platform": system, "schema": BOOT_SCHEMA}


def host_authority(
    physical_core_count: int, resources: dict[str, object] | None = None
) -> dict[str, object]:
    try:
        resources = resources or effective_host_resources()
        logical = int(resources["logicalCores"])
        total_memory = int(resources["totalMemoryBytes"])
    except (HostResourceError, KeyError, TypeError, ValueError) as error:
        raise GateError("PREFLIGHT_HOST_AUTHORITY", "host compute authority is invalid") from error
    if physical_core_count <= 0 or logical <= 0 or total_memory <= 0:
        raise GateError("PREFLIGHT_HOST_AUTHORITY", "host compute authority is invalid")
    return {
        "logicalCores": logical,
        "machine": platform.machine(),
        "physicalCores": physical_core_count,
        "platform": platform.system(),
        "qualificationCores": min(physical_core_count, logical),
        "release": platform.release(),
        "resourceAuthority": resources,
        "schema": HOST_SCHEMA,
        "totalMemoryBytes": total_memory,
    }


def _load_snapshot() -> dict[str, object]:
    try:
        averages = os.getloadavg()
    except OSError as error:
        raise GateError("ARM_LOAD_AUTHORITY", "host load authority is unavailable") from error
    if len(averages) != 3 or any(not math.isfinite(value) or value < 0 for value in averages):
        raise GateError("ARM_LOAD_AUTHORITY", "host load authority is invalid")
    return {
        "capturedMonotonicNanos": time.monotonic_ns(),
        "loadAverage": [round(value, 6) for value in averages],
    }


def load_authority(
    before: dict[str, object], after: dict[str, object], boot_digest: str, cores: int
) -> dict[str, object]:
    return {
        "after": after,
        "before": before,
        "bootAuthorityDigest": boot_digest,
        "physicalCores": cores,
        "schema": LOAD_SCHEMA,
    }


def validate_load_authority(
    value: object, boot_digest: str, physical_core_count: int
) -> bool:
    if not isinstance(value, dict) or set(value) != LOAD_FIELDS:
        return False
    if (
        value.get("schema") != LOAD_SCHEMA
        or value.get("bootAuthorityDigest") != boot_digest
        or value.get("physicalCores") != physical_core_count
    ):
        return False
    snapshots = [value.get("before"), value.get("after")]
    for snapshot in snapshots:
        if not isinstance(snapshot, dict) or set(snapshot) != LOAD_SNAPSHOT_FIELDS:
            return False
        average = snapshot.get("loadAverage")
        if (
            not exact_positive_int(snapshot.get("capturedMonotonicNanos"))
            or not isinstance(average, list)
            or len(average) != 3
            or any(
                not isinstance(item, (int, float))
                or isinstance(item, bool)
                or not math.isfinite(item)
                or item < 0
                for item in average
            )
        ):
            return False
    return int(snapshots[1]["capturedMonotonicNanos"]) >= int(
        snapshots[0]["capturedMonotonicNanos"]
    )


def _timing_map(value: object, expected: tuple[str, ...], *, positive: bool) -> bool:
    if not isinstance(value, dict) or set(value) != set(expected):
        return False
    predicate = exact_positive_int if positive else exact_nonnegative_int
    return all(predicate(value[name]) for name in expected)


def _digest_map(value: object) -> bool:
    return (
        isinstance(value, dict)
        and bool(value)
        and all(
            isinstance(name, str)
            and SAFE_IDENTIFIER_PATTERN.fullmatch(name) is not None
            and valid_digest(item)
            for name, item in value.items()
        )
    )


def _safe_identifiers(value: object) -> bool:
    return (
        isinstance(value, list)
        and bool(value)
        and all(
            isinstance(item, str) and SAFE_IDENTIFIER_PATTERN.fullmatch(item) is not None
            for item in value
        )
        and value == sorted(set(value))
    )


def validate_cold_evidence(
    evidence: object, profile: str, common: dict[str, object]
) -> tuple[dict[str, object], int]:
    if not isinstance(evidence, dict) or set(evidence) != COLD_EVIDENCE_FIELDS:
        raise GateError("ARM_EVIDENCE", "cold build evidence schema is not exact")
    plan = evidence.get("buildPlan")
    if not isinstance(plan, dict) or set(plan) != PLAN_FIELDS:
        raise GateError("ARM_EVIDENCE", "cold build plan schema is not exact")
    plan_stages = plan.get("stageWallMillis")
    stage_wall = evidence.get("stageWallMillis")
    logical_cores = common.get("logicalCores")
    total_memory = common.get("totalMemoryBytes")
    if (
        evidence.get("schema") != "codeclew-real-cold-build-evidence/1.0"
        or evidence.get("status") != "MEASURED"
        or evidence.get("mode") != "RELEASE"
        or evidence.get("runtimeKey") != common.get("runtimeKey")
        or not valid_digest(evidence.get("runtimeKey"))
        or not valid_digest(evidence.get("manifestDigest"))
        or not _digest_map(evidence.get("artifactHashes"))
        or not _digest_map(evidence.get("workerTreeHashes"))
        or sorted(evidence["artifactHashes"]) != common.get("artifactIds")
        or sorted(evidence["workerTreeHashes"]) != common.get("workerIds")
        or evidence.get("componentHits") != []
        or evidence.get("componentMisses") != common.get("componentIds")
        or not _safe_identifiers(evidence.get("componentMisses"))
        or not exact_positive_int(logical_cores)
        or not exact_positive_int(total_memory)
        or not exact_positive_int(evidence.get("wallMillis"))
        or plan.get("profile") != profile
        or type(plan.get("parallel")) is not bool
        or plan.get("parallel") != (profile == "PARALLEL")
        or plan.get("toolchainStages") != list(TOOLCHAIN_STAGES)
        or not _timing_map(plan_stages, TOOLCHAIN_STAGES, positive=True)
        or not _timing_map(stage_wall, CAPSULE_STAGES, positive=False)
        or any(stage_wall[name] != plan_stages[name] for name in TOOLCHAIN_STAGES)
        or not exact_positive_int(plan.get("cargoWorkers"))
        or not exact_positive_int(plan.get("gradleHeapBytes"))
        or not exact_positive_int(plan.get("gradleWorkers"))
        or not exact_positive_int(plan.get("inputWorkers"))
        or not exact_positive_int(plan.get("packageWorkers"))
        or not exact_positive_int(plan.get("memoryBudgetBytes"))
        or not exact_positive_int(plan.get("toolchainWallMillis"))
        or not exact_positive_int(plan.get("toolchainCriticalPathMillis"))
    ):
        raise GateError("ARM_EVIDENCE", "cold build evidence is incomplete")
    expected_resources = expected_build_resources(
        int(logical_cores), int(total_memory), profile
    )
    if any(plan[name] != value for name, value in expected_resources.items()):
        raise GateError(
            "ARM_EVIDENCE", "build resource authority differs from the host plan"
        )
    recomputed = (
        max(int(plan_stages[name]) for name in TOOLCHAIN_STAGES)
        if profile == "PARALLEL"
        else sum(int(plan_stages[name]) for name in TOOLCHAIN_STAGES)
    )
    sequential = sum(int(stage_wall[name]) for name in CAPSULE_STAGES if name not in TOOLCHAIN_STAGES)
    if (
        plan.get("toolchainCriticalPathMillis") != recomputed
        or int(plan["toolchainWallMillis"]) < recomputed
        or int(plan["toolchainWallMillis"]) > int(evidence["wallMillis"])
        or int(evidence["wallMillis"]) < recomputed + sequential
    ):
        raise GateError("ARM_EVIDENCE", "cold build critical path is not independently reproducible")
    return evidence, recomputed


def validate_warm_audit(value: object) -> dict[str, object]:
    if not isinstance(value, dict) or set(value) != WARM_AUDIT_FIELDS:
        raise GateError("ARM_WARM_AUDIT", "warm audit schema is not exact")
    counters = value.get("counters")
    if (
        value.get("schema") != "codeclew-bootstrap-warm-audit/2.0"
        or value.get("status") != "PASSED"
        or value.get("coldToolchainInvoked") is not False
        or value.get("capsuleBuildInvoked") is not False
        or value.get("forbiddenWarmProcesses") != ["cargo", "rustc", "gradle", "maven"]
        or not isinstance(counters, dict)
        or set(counters) != WARM_COUNTER_FIELDS
        or any(not exact_nonnegative_int(item) for item in counters.values())
        or counters.get("processRuns") != 0
        or counters.get("digestFileCalls") != 0
        or counters.get("checkpointHits") != 1
        or counters.get("checkpointMisses") != 0
        or not exact_positive_int(counters.get("metadataChecks"))
    ):
        raise GateError("ARM_WARM_AUDIT", "warm audit did not prove an exact warm hit")
    return value


def _arm_digest(value: dict[str, object]) -> str:
    unsigned = dict(value)
    unsigned.pop("armDigest", None)
    return domain_digest(ARM_DIGEST_DOMAIN, unsigned)


def validate_arm(value: object, expected: dict[str, object] | None = None) -> dict[str, object]:
    if not isinstance(value, dict) or set(value) != ARM_FIELDS:
        raise GateError("ARM_RECEIPT", "arm receipt schema is not exact")
    if expected is not None and any(value.get(name) != field for name, field in expected.items()):
        raise GateError("ARM_RECEIPT", "arm receipt authority mismatch")
    if (
        value.get("schema") != ARM_SCHEMA
        or value.get("status") != "PASS"
        or value.get("armDigest") != _arm_digest(value)
        or not valid_digest(value.get("cohortId"))
        or not valid_digest(value.get("bootAuthorityDigest"))
        or not valid_digest(value.get("buildAuthorityDigest"))
        or not valid_digest(value.get("cacheSeedDigest"))
        or not valid_digest(value.get("hostAuthorityDigest"))
        or not valid_digest(value.get("runtimeKey"))
        or not valid_digest(value.get("manifestDigest"))
        or not _digest_map(value.get("artifactHashes"))
        or not _digest_map(value.get("workerTreeHashes"))
        or not _safe_identifiers(value.get("artifactIds"))
        or not _safe_identifiers(value.get("workerIds"))
        or not exact_positive_int(value.get("sequence"))
        or not exact_positive_int(value.get("block"))
        or not exact_positive_int(value.get("criticalPathMillis"))
        or not exact_positive_int(value.get("capsuleWallMillis"))
        or not exact_positive_int(value.get("outerWallMillis"))
        or not _safe_identifiers(value.get("componentIds"))
        or not exact_positive_int(value.get("logicalCores"))
        or not exact_positive_int(value.get("physicalCores"))
        or not exact_positive_int(value.get("qualificationCores"))
        or not exact_positive_int(value.get("totalMemoryBytes"))
        or not validate_load_authority(
            value.get("loadAuthority"),
            str(value["bootAuthorityDigest"]),
            int(value["physicalCores"]),
        )
        or REVISION_PATTERN.fullmatch(str(value.get("sourceRevision"))) is None
    ):
        raise GateError("ARM_RECEIPT", "arm receipt is invalid")
    predecessor = value.get("predecessorArmDigest")
    if predecessor is not None and not valid_digest(predecessor):
        raise GateError("ARM_RECEIPT", "arm predecessor authority is invalid")
    evidence = {
        "artifactHashes": value["artifactHashes"],
        "buildPlan": value["buildPlan"],
        "componentHits": value["componentHits"],
        "componentMisses": value["componentMisses"],
        "manifestDigest": value["manifestDigest"],
        "mode": "RELEASE",
        "runtimeKey": value["runtimeKey"],
        "schema": "codeclew-real-cold-build-evidence/1.0",
        "stageWallMillis": value["stageWallMillis"],
        "status": "MEASURED",
        "wallMillis": value["capsuleWallMillis"],
        "workerTreeHashes": value["workerTreeHashes"],
    }
    _evidence, recomputed = validate_cold_evidence(
        evidence,
        str(value["profile"]),
        {
            "componentIds": value["componentIds"],
            "artifactIds": value["artifactIds"],
            "logicalCores": value["logicalCores"],
            "runtimeKey": value["runtimeKey"],
            "totalMemoryBytes": value["totalMemoryBytes"],
            "workerIds": value["workerIds"],
        },
    )
    if recomputed != value["criticalPathMillis"]:
        raise GateError("ARM_RECEIPT", "arm critical path authority is invalid")
    validate_warm_audit(value["warmAudit"])
    return value


def read_arm(path: Path, expected: dict[str, object]) -> dict[str, object] | None:
    value = _read_exact_json(path, ARM_FIELDS)
    if value is None:
        return None
    try:
        return validate_arm(value, expected)
    except GateError:
        return None


def run_arm(
    source: Path, cache_home: Path, state_home: Path, receipt_path: Path,
    original_home: Path, common: dict[str, object], arm_id: str, block: int,
    order: list[str], profile: str, sequence: int, predecessor: str | None, cores: int,
) -> dict[str, object]:
    expected = {
        **common, "armId": arm_id, "block": block, "order": order,
        "predecessorArmDigest": predecessor, "profile": profile, "sequence": sequence,
    }
    if receipt_path.exists() or receipt_path.is_symlink():
        raise GateError("ARM_RECEIPT", "fresh cohort arm receipt already exists")
    observed_boot = boot_authority()
    if domain_digest(AUTHORITY_DIGEST_DOMAIN, observed_boot) != common["bootAuthorityDigest"]:
        raise GateError("ARM_BOOT_AUTHORITY", "host rebooted during the cold cohort")
    before = _load_snapshot()
    environment = isolated_environment(cache_home, state_home, original_home)
    started = time.monotonic()
    completed = run(
        [str(source / "clew"), f"--bootstrap-cold-build-evidence={profile.lower()}"],
        source, environment,
    )
    outer_wall = int((time.monotonic() - started) * 1000)
    after = _load_snapshot()
    if completed.returncode != 0:
        raise GateError(
            f"ARM_{arm_id.upper().replace('-', '_')}", "cold build arm failed", _diagnostic(completed)
        )
    evidence, critical_path = validate_cold_evidence(
        _json_output(completed, "ARM_EVIDENCE"), profile, common
    )
    if not exact_positive_int(outer_wall) or outer_wall < int(evidence["wallMillis"]):
        raise GateError("ARM_EVIDENCE", "outer wall clock is inconsistent with capsule evidence")
    warm = run([str(source / "clew"), "--bootstrap-warm-audit"], source, environment)
    if warm.returncode != 0:
        raise GateError("ARM_WARM_AUDIT", "cold build arm warm audit failed", _diagnostic(warm))
    warm_value = validate_warm_audit(_json_output(warm, "ARM_WARM_AUDIT"))
    if domain_digest(AUTHORITY_DIGEST_DOMAIN, boot_authority()) != common["bootAuthorityDigest"]:
        raise GateError("ARM_BOOT_AUTHORITY", "host rebooted during the cold cohort")
    value: dict[str, object] = {
        **expected,
        "artifactHashes": evidence["artifactHashes"], "buildPlan": evidence["buildPlan"],
        "capsuleWallMillis": evidence["wallMillis"], "componentHits": evidence["componentHits"],
        "componentMisses": evidence["componentMisses"], "criticalPathMillis": critical_path,
        "loadAuthority": load_authority(before, after, str(common["bootAuthorityDigest"]), cores),
        "manifestDigest": evidence["manifestDigest"], "outerWallMillis": outer_wall,
        "runtimeKey": evidence["runtimeKey"], "schema": ARM_SCHEMA,
        "stageWallMillis": evidence["stageWallMillis"], "status": "PASS",
        "warmAudit": warm_value, "workerTreeHashes": evidence["workerTreeHashes"],
    }
    value["armDigest"] = _arm_digest(value)
    validate_arm(value, expected)
    immutable_json(receipt_path, value)
    persisted = read_arm(receipt_path, expected)
    if persisted != value:
        raise GateError("ARM_RECEIPT", "immutable arm receipt verification failed")
    return value


def aggregate(
    arms: list[dict[str, object]],
    source_revision: str,
    cores: int,
    cohort_authority: dict[str, object] | None = None,
) -> dict[str, object]:
    if len(arms) != len(ARMS):
        raise GateError("EVIDENCE_AGGREGATION", "cold cohort arm count is invalid")
    validated = [validate_arm(arm) for arm in arms]
    predecessor = None
    cohort_fields = (
        "artifactIds", "bootAuthorityDigest", "buildAuthorityDigest", "cacheSeedDigest",
        "cohortId", "componentIds", "hostAuthorityDigest", "logicalCores",
        "physicalCores", "qualificationCores", "sourceRevision", "totalMemoryBytes",
        "workerIds",
    )
    cohort_baseline = {field: validated[0][field] for field in cohort_fields}
    if cohort_authority is not None:
        cohort_id = str(cohort_authority.get("cohortId"))
        _validate_cohort(
            cohort_authority, f"cohort-{cohort_id.removeprefix('sha256:')}"
        )
        expected_cohort = {
            "bootAuthorityDigest": cohort_authority["bootAuthorityDigest"],
            "artifactIds": cohort_authority["buildAuthority"]["artifactIds"],
            "buildAuthorityDigest": cohort_authority["buildAuthorityDigest"],
            "cacheSeedDigest": cohort_authority["cacheSeedDigest"],
            "cohortId": cohort_authority["cohortId"],
            "componentIds": cohort_authority["buildAuthority"]["componentIds"],
            "hostAuthorityDigest": cohort_authority["hostAuthorityDigest"],
            "logicalCores": cohort_authority["hostAuthority"]["logicalCores"],
            "physicalCores": cohort_authority["hostAuthority"]["physicalCores"],
            "qualificationCores": cohort_authority["hostAuthority"]["qualificationCores"],
            "sourceRevision": cohort_authority["sourceRevision"],
            "totalMemoryBytes": cohort_authority["hostAuthority"]["totalMemoryBytes"],
            "workerIds": cohort_authority["buildAuthority"]["workerIds"],
        }
        if cohort_baseline != expected_cohort:
            raise GateError(
                "EVIDENCE_AGGREGATION", "arm chain differs from its cohort authority"
            )
    for sequence, (arm, specification) in enumerate(zip(validated, ARMS), 1):
        arm_id, block, order, profile = specification
        if (
            arm["sequence"] != sequence or arm["armId"] != arm_id or arm["block"] != block
            or arm["order"] != order or arm["profile"] != profile
            or arm["predecessorArmDigest"] != predecessor
            or {field: arm[field] for field in cohort_fields} != cohort_baseline
        ):
            raise GateError("EVIDENCE_AGGREGATION", "cold cohort predecessor authority is invalid")
        predecessor = str(arm["armDigest"])
    if cohort_baseline["sourceRevision"] != source_revision:
        raise GateError("EVIDENCE_AGGREGATION", "cold cohort source authority is invalid")
    if cohort_baseline["qualificationCores"] != cores:
        raise GateError("EVIDENCE_AGGREGATION", "cold cohort host qualification is invalid")

    by_block: dict[int, dict[str, dict[str, object]]] = {}
    identity_fields = ("runtimeKey", "manifestDigest", "artifactHashes", "workerTreeHashes")
    baseline = {field: validated[0][field] for field in identity_fields}
    identity_mismatches = []
    for arm in validated:
        identity = {field: arm[field] for field in identity_fields}
        if identity != baseline:
            identity_mismatches.append(
                {
                    "armId": arm["armId"],
                    "differingFields": sorted(
                        field for field in identity_fields if identity[field] != baseline[field]
                    ),
                }
            )
        by_block.setdefault(int(arm["block"]), {})[str(arm["profile"])] = arm
    blocks = []
    critical_ratios = []
    total_ratios = []
    for block_id in sorted(by_block):
        measured = by_block[block_id]
        if set(measured) != {"SERIAL", "PARALLEL"}:
            raise GateError("EVIDENCE_AGGREGATION", "matched cold-build block is incomplete")
        serial = measured["SERIAL"]
        parallel = measured["PARALLEL"]
        serial_critical = int(serial["criticalPathMillis"])
        parallel_critical = int(parallel["criticalPathMillis"])
        critical_ratio = parallel_critical / serial_critical
        total_ratio = int(parallel["outerWallMillis"]) / int(serial["outerWallMillis"])
        critical_ratios.append(critical_ratio)
        total_ratios.append(total_ratio)
        blocks.append(
            {
                "block": block_id, "criticalPathRatio": round(critical_ratio, 6),
                "order": serial["order"],
                "parallel": {
                    "capsuleWallMillis": parallel["capsuleWallMillis"],
                    "loadAuthority": parallel["loadAuthority"],
                    "outerWallMillis": parallel["outerWallMillis"],
                    "stageWallMillis": parallel["stageWallMillis"],
                    "toolchainCriticalPathMillis": parallel_critical,
                },
                "serial": {
                    "capsuleWallMillis": serial["capsuleWallMillis"],
                    "loadAuthority": serial["loadAuthority"],
                    "outerWallMillis": serial["outerWallMillis"],
                    "stageWallMillis": serial["stageWallMillis"],
                    "toolchainCriticalPathMillis": serial_critical,
                },
                "totalWallRatio": round(total_ratio, 6),
            }
        )
    critical_geomean = math.sqrt(math.prod(critical_ratios))
    total_geomean = math.sqrt(math.prod(total_ratios))
    interaction = max(critical_ratios) / min(critical_ratios)
    noisy = interaction > ORDER_INTERACTION_MAX
    passed = (
        not identity_mismatches and not noisy and critical_geomean <= CRITICAL_RATIO_MAX
        and max(critical_ratios) <= BLOCK_RATIO_MAX and total_geomean <= TOTAL_RATIO_MAX
    )
    if identity_mismatches:
        status = "FAILED_NONDETERMINISTIC_CAPSULE"
    elif noisy:
        status = "FAILED_NOISY"
    elif passed:
        status = "PASSED"
    else:
        status = "FAILED_RUNTIME_RATIO"
    return {
        "accepted": passed, "authorities": cohort_baseline, "identity": baseline,
        "identityMismatches": identity_mismatches,
        "measurements": {
            "blocks": blocks,
            "criticalPathGeometricMeanRatio": round(critical_geomean, 6),
            "orderInteraction": round(interaction, 6),
            "totalWallGeometricMeanRatio": round(total_geomean, 6),
        },
        "qualification": {"effectiveCores": cores, "minimumEffectiveCores": 4},
        "releaseGatePassed": passed, "schema": REPORT_SCHEMA,
        "scope": {"multiCompilationGenerationPassed": False, "runtimeCapsuleColdBuildPassed": passed},
        "sourceRevision": source_revision, "status": status,
        "thresholds": {
            "blockCriticalPathRatioMax": BLOCK_RATIO_MAX,
            "criticalPathGeometricMeanRatioMax": CRITICAL_RATIO_MAX,
            "orderInteractionMax": ORDER_INTERACTION_MAX,
            "totalWallGeometricMeanRatioMax": TOTAL_RATIO_MAX,
        },
    }


def _stat_identity(metadata: os.stat_result) -> tuple[int, ...]:
    return (
        metadata.st_dev, metadata.st_ino, metadata.st_mode, metadata.st_uid,
        metadata.st_nlink, metadata.st_size, metadata.st_mtime_ns, metadata.st_ctime_ns,
    )


def _directory_lock_identity(metadata: os.stat_result) -> tuple[int, ...]:
    return (metadata.st_dev, metadata.st_ino, metadata.st_mode, metadata.st_uid)


def _open_private_lock(
    path: Path, *, create: bool, nonblocking: bool = False
) -> LockHandle | None:
    if not path.name or path.name in {".", ".."} or "/" in path.name:
        raise GateError("COHORT_ADMISSION", "lock name is invalid")
    descriptor = _open_private_directory_fd(path.parent, create=create)
    try:
        # No path entry is a lock authority. The retained directory inode is
        # the lock, eliminating file create/crash and unlink split-brain gaps.
        for name in (path.name, f"{path.name}.authority"):
            try:
                os.stat(name, dir_fd=descriptor, follow_symlinks=False)
            except FileNotFoundError:
                continue
            raise GateError("COHORT_ADMISSION", "obsolete lock entry is unsafe")
        identity = _directory_lock_identity(os.fstat(descriptor))
        operation = fcntl.LOCK_EX | (fcntl.LOCK_NB if nonblocking else 0)
        try:
            fcntl.flock(descriptor, operation)
        except BlockingIOError:
            os.close(descriptor)
            return None
        handle = LockHandle(descriptor, path, identity)
        handle.verify()
        return handle
    except BaseException:
        os.close(descriptor)
        raise


def cleanup_owned_tree(path: Path, parent: Path, identity: tuple[int, int]) -> bool:
    """Delete only the exact physical direct child admitted by the controller."""
    if (
        not path.is_absolute() or not parent.is_absolute() or ".." in path.parts
        or path.parent != parent
    ):
        return False
    try:
        metadata = path.lstat()
    except OSError:
        return False
    if (
        not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or (metadata.st_dev, metadata.st_ino) != identity
    ):
        return False
    # Never call shutil.rmtree(work): bounded_gate_cleanup performs no-follow deletion.
    try:
        return bounded_gate_cleanup(str(path), STALE_CLEANUP_TIMEOUT, identity)
    except Exception:
        return False


def recover_stale_workdirs(gate_base: Path) -> None:
    """Remove only exact abandoned gate workdirs while admission is held."""
    for candidate in sorted(gate_base.iterdir(), key=lambda item: item.name):
        if candidate.name in {"admission.lock", "admission.lock.authority"}:
            continue
        metadata = candidate.lstat()
        if RUN_DIRECTORY_PATTERN.fullmatch(candidate.name) is None:
            raise GateError("STALE_WORK_AUTHORITY", "unknown cold gate work entry is unsafe")
        if (
            not stat.S_ISDIR(metadata.st_mode)
            or stat.S_ISLNK(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) != 0o700
        ):
            raise GateError("STALE_WORK_AUTHORITY", "stale cold gate workdir is unsafe")
        if not cleanup_owned_tree(
            candidate, gate_base, (metadata.st_dev, metadata.st_ino)
        ):
            raise GateError("STALE_WORK_CLEANUP", "bounded stale workdir cleanup failed")


def acquire_gate_admission(gate_base: Path) -> LockHandle:
    lock = _open_private_lock(gate_base / "admission.lock", create=True)
    assert lock is not None
    try:
        lock.verify()
        recover_stale_workdirs(gate_base)
        lock.verify()
        return lock
    except BaseException:
        lock.close()
        raise


def _validate_cohort(value: object, directory_name: str) -> dict[str, object]:
    if not isinstance(value, dict) or set(value) != COHORT_FIELDS:
        raise GateError("STALE_RUN_AUTHORITY", "stale cohort schema is invalid")
    unsigned = dict(value)
    observed_id = unsigned.pop("cohortId", None)
    boot = value.get("bootAuthority")
    host = value.get("hostAuthority")
    build = value.get("buildAuthority")
    if (
        value.get("schema") != COHORT_SCHEMA or value.get("status") != "ACTIVE"
        or re.fullmatch(r"[0-9a-f]{64}", str(value.get("cohortNonce"))) is None
        or not exact_positive_int(value.get("createdUnixNanos"))
        or REVISION_PATTERN.fullmatch(str(value.get("sourceRevision"))) is None
        or not valid_digest(value.get("cacheSeedDigest"))
        or value.get("armOrder") != [arm_id for arm_id, _block, _order, _profile in ARMS]
        or not isinstance(boot, dict)
        or set(boot) != {"identity", "platform", "schema"}
        or boot.get("schema") != BOOT_SCHEMA
        or not isinstance(boot.get("identity"), str)
        or not boot.get("identity")
        or not isinstance(boot.get("platform"), str)
        or value.get("bootAuthorityDigest") != domain_digest(AUTHORITY_DIGEST_DOMAIN, boot)
        or not isinstance(host, dict)
        or set(host) != {
            "logicalCores", "machine", "physicalCores", "platform", "release",
            "qualificationCores", "resourceAuthority", "schema", "totalMemoryBytes",
        }
        or host.get("schema") != HOST_SCHEMA
        or not exact_positive_int(host.get("physicalCores"))
        or not exact_positive_int(host.get("logicalCores"))
        or not exact_positive_int(host.get("qualificationCores"))
        or host.get("qualificationCores")
        != min(int(host["logicalCores"]), int(host["physicalCores"]))
        or not exact_positive_int(host.get("totalMemoryBytes"))
        or not isinstance(host.get("resourceAuthority"), dict)
        or host["resourceAuthority"].get("schema")
        != "codeclew-effective-host-resources/1.0"
        or host["resourceAuthority"].get("logicalCores") != host.get("logicalCores")
        or host["resourceAuthority"].get("totalMemoryBytes")
        != host.get("totalMemoryBytes")
        or any(not isinstance(host.get(name), str) or not host.get(name) for name in ("machine", "platform", "release"))
        or value.get("hostAuthorityDigest") != domain_digest(AUTHORITY_DIGEST_DOMAIN, host)
        or not isinstance(build, dict)
        or value.get("buildAuthorityDigest") != domain_digest(AUTHORITY_DIGEST_DOMAIN, build)
        or observed_id != domain_digest(COHORT_DIGEST_DOMAIN, unsigned)
        or directory_name != f"cohort-{str(observed_id).removeprefix('sha256:')}"
    ):
        raise GateError("STALE_RUN_AUTHORITY", "stale cohort authority is invalid")
    validate_build_authority(build)
    return value


def _read_persisted_arm_chain(
    cohort_path: Path, authority: dict[str, object]
) -> list[dict[str, object]]:
    arms_path = cohort_path / "arms"
    expected_names = {
        f"{sequence:02d}-{arm_id}.json"
        for sequence, (arm_id, _block, _order, _profile) in enumerate(ARMS, 1)
    }
    try:
        observed_names = {path.name for path in arms_path.iterdir()}
    except OSError as error:
        raise GateError("COHORT_COMPLETION", "arm receipt directory is unavailable") from error
    if observed_names != expected_names:
        raise GateError("COHORT_COMPLETION", "persisted arm receipt closure is incomplete")
    common = {
        "artifactIds": authority["buildAuthority"]["artifactIds"],
        "bootAuthorityDigest": authority["bootAuthorityDigest"],
        "buildAuthorityDigest": authority["buildAuthorityDigest"],
        "cacheSeedDigest": authority["cacheSeedDigest"],
        "cohortId": authority["cohortId"],
        "componentIds": authority["buildAuthority"]["componentIds"],
        "hostAuthorityDigest": authority["hostAuthorityDigest"],
        "logicalCores": authority["hostAuthority"]["logicalCores"],
        "physicalCores": authority["hostAuthority"]["physicalCores"],
        "qualificationCores": authority["hostAuthority"]["qualificationCores"],
        "runtimeKey": authority["buildAuthority"]["runtimeKey"],
        "sourceRevision": authority["sourceRevision"],
        "totalMemoryBytes": authority["hostAuthority"]["totalMemoryBytes"],
        "workerIds": authority["buildAuthority"]["workerIds"],
    }
    predecessor = None
    persisted_arms = []
    for sequence, specification in enumerate(ARMS, 1):
        arm_id, block, order, profile = specification
        expected = {
            **common,
            "armId": arm_id,
            "block": block,
            "order": order,
            "predecessorArmDigest": predecessor,
            "profile": profile,
            "sequence": sequence,
        }
        persisted = read_arm(arms_path / f"{sequence:02d}-{arm_id}.json", expected)
        if persisted is None:
            raise GateError("COHORT_COMPLETION", "persisted arm chain is invalid")
        persisted_arms.append(persisted)
        predecessor = str(persisted["armDigest"])
    return persisted_arms


def _valid_complete(path: Path, cohort: dict[str, object]) -> bool:
    value = _read_exact_json(path, COHORT_COMPLETE_FIELDS)
    if not (
        value and value.get("schema") == COHORT_COMPLETE_SCHEMA
        and value.get("status") == "COMPLETE"
        and value.get("cohortId") == cohort.get("cohortId")
        and value.get("armCount") == len(ARMS)
        and valid_digest(value.get("finalArmDigest"))
        and valid_digest(value.get("reportDigest"))
    ):
        return False
    try:
        arms = _read_persisted_arm_chain(path.parent, cohort)
    except GateError:
        return False
    report = _read_exact_json(path.parent / "REPORT.json", None)
    try:
        expected_report = {
            **aggregate(
                arms,
                str(cohort["sourceRevision"]),
                int(cohort["hostAuthority"]["qualificationCores"]),
                cohort,
            ),
            "cleanupStatus": "PASSED",
        }
    except (GateError, KeyError, TypeError, ValueError):
        return False
    try:
        closure = {entry.name for entry in path.parent.iterdir()}
    except OSError:
        return False
    return bool(
        closure == {"COMPLETE.json", "REPORT.json", "arms", "cohort.json"}
        and
        report == expected_report
        and digest(report) == value.get("reportDigest")
        and arms[-1].get("armDigest") == value.get("finalArmDigest")
    )


def _recover_completed_receipt_publication(path: Path) -> None:
    """Finish only the exact link-before-unlink COMPLETE publication state."""
    parent = _open_private_directory_fd(path.parent, create=False)
    final_descriptor = -1
    candidate_descriptor = -1
    try:
        final = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
        candidate_pattern = re.compile(
            rf"\.{re.escape(path.name)}\.[0-9]+\.[0-9a-f]{{24}}\.tmp"
        )
        with os.scandir(parent) as entries:
            candidates = sorted(
                entry.name for entry in entries if candidate_pattern.fullmatch(entry.name)
            )
        if final.st_nlink == 1 and not candidates:
            return
        if len(candidates) != 1 or final.st_nlink != 2:
            raise GateError(
                "STALE_RUN_AUTHORITY", "completed receipt journal is unsafe"
            )
        candidate = candidates[0]
        flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
        final_descriptor = os.open(path.name, flags, dir_fd=parent)
        candidate_descriptor = os.open(candidate, flags, dir_fd=parent)
        final_opened = os.fstat(final_descriptor)
        candidate_opened = os.fstat(candidate_descriptor)
        if (
            not stat.S_ISREG(final_opened.st_mode)
            or final_opened.st_uid != os.geteuid()
            or stat.S_IMODE(final_opened.st_mode) != 0o400
            or final_opened.st_size > MAX_RECEIPT_BYTES
            or final_opened.st_nlink != 2
            or _stat_identity(final_opened) != _stat_identity(candidate_opened)
            or _stat_identity(final_opened) != _stat_identity(final)
        ):
            raise GateError(
                "STALE_RUN_AUTHORITY", "completed receipt journal binding is unsafe"
            )
        os.unlink(candidate, dir_fd=parent)
        os.fsync(parent)
        repaired = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
        if (
            (repaired.st_dev, repaired.st_ino)
            != (final_opened.st_dev, final_opened.st_ino)
            or repaired.st_nlink != 1
        ):
            raise GateError(
                "STALE_RUN_AUTHORITY", "completed receipt journal recovery failed"
            )
    finally:
        if candidate_descriptor >= 0:
            os.close(candidate_descriptor)
        if final_descriptor >= 0:
            os.close(final_descriptor)
        os.close(parent)


def recover_stale_runs(runs_root: Path) -> None:
    for candidate in sorted(runs_root.iterdir(), key=lambda item: item.name):
        if candidate.name in {"admission.lock", "admission.lock.authority"}:
            continue
        metadata = candidate.lstat()
        if COHORT_CANDIDATE_PATTERN.fullmatch(candidate.name) is not None:
            if not cleanup_owned_tree(candidate, runs_root, (metadata.st_dev, metadata.st_ino)):
                raise GateError("STALE_RUN_CLEANUP", "bounded stale candidate cleanup failed")
            continue
        if COHORT_DIRECTORY_PATTERN.fullmatch(candidate.name) is None:
            raise GateError("STALE_RUN_AUTHORITY", "unknown cohort state entry is unsafe")
        if (
            not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode)
            or metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o700
        ):
            raise GateError("STALE_RUN_AUTHORITY", "stale cohort directory is unsafe")
        cohort = _read_exact_json(candidate / "cohort.json", COHORT_FIELDS)
        if cohort is None:
            raise GateError("STALE_RUN_AUTHORITY", "stale cohort receipt is unavailable")
        cohort = _validate_cohort(cohort, candidate.name)
        lock = _open_private_lock(candidate / "cohort.lock", create=False, nonblocking=True)
        if lock is None:
            continue
        try:
            lock.verify()
            complete_path = candidate / "COMPLETE.json"
            if complete_path.exists() or complete_path.is_symlink():
                _recover_completed_receipt_publication(complete_path)
                lock.verify()
                if not _valid_complete(complete_path, cohort):
                    raise GateError("STALE_RUN_AUTHORITY", "completed cohort receipt is invalid")
                lock.verify()
                continue
            if not cleanup_owned_tree(candidate, runs_root, (metadata.st_dev, metadata.st_ino)):
                raise GateError("STALE_RUN_CLEANUP", "bounded stale cohort cleanup failed")
        finally:
            lock.close()


def create_cohort(
    runs_root: Path, source_revision: str, cache_digest: str,
    host: dict[str, object], boot: dict[str, object], build: dict[str, object],
) -> CohortHandle:
    private_directory(runs_root)
    admission = _open_private_lock(runs_root / "admission.lock", create=True)
    assert admission is not None
    try:
        recover_stale_runs(runs_root)
        admission.verify()
        unsigned: dict[str, object] = {
            "armOrder": [arm_id for arm_id, _block, _order, _profile in ARMS],
            "bootAuthority": boot,
            "bootAuthorityDigest": domain_digest(AUTHORITY_DIGEST_DOMAIN, boot),
            "buildAuthority": build,
            "buildAuthorityDigest": domain_digest(AUTHORITY_DIGEST_DOMAIN, build),
            "cacheSeedDigest": cache_digest, "cohortNonce": secrets.token_hex(32),
            "createdUnixNanos": time.time_ns(), "hostAuthority": host,
            "hostAuthorityDigest": domain_digest(AUTHORITY_DIGEST_DOMAIN, host),
            "schema": COHORT_SCHEMA, "sourceRevision": source_revision, "status": "ACTIVE",
        }
        cohort_id = domain_digest(COHORT_DIGEST_DOMAIN, unsigned)
        authority = {**unsigned, "cohortId": cohort_id}
        final = runs_root / f"cohort-{cohort_id.removeprefix('sha256:')}"
        candidate = runs_root / f".candidate-{secrets.token_hex(32)}"
        candidate.mkdir(mode=0o700)
        lock = _open_private_lock(candidate / "cohort.lock", create=True)
        assert lock is not None
        try:
            (candidate / "arms").mkdir(mode=0o700)
            immutable_json(candidate / "cohort.json", authority)
            os.rename(candidate, final)
            _fsync_directory(runs_root)
            lock.label = final / "cohort.lock"
            lock.verify()
        except BaseException:
            lock.close()
            if candidate.exists():
                metadata = candidate.lstat()
                cleanup_owned_tree(candidate, runs_root, (metadata.st_dev, metadata.st_ino))
            raise
        return CohortHandle(final, final / "arms", authority, lock)
    finally:
        admission.close()


def complete_cohort(
    cohort: CohortHandle, arms: list[dict[str, object]], report: dict[str, object]
) -> None:
    cohort.lock.verify()
    if len(arms) != len(ARMS):
        raise GateError("COHORT_COMPLETION", "incomplete cohort cannot be completed")
    cohort_id = str(cohort.authority.get("cohortId"))
    _validate_cohort(
        cohort.authority, f"cohort-{cohort_id.removeprefix('sha256:')}"
    )
    persisted_arms = _read_persisted_arm_chain(cohort.path, cohort.authority)
    if persisted_arms != arms:
        raise GateError(
            "COHORT_COMPLETION", "persisted arm chain differs from completion input"
        )
    expected_report = {
        **aggregate(
            persisted_arms,
            str(cohort.authority["sourceRevision"]),
            int(cohort.authority["hostAuthority"]["qualificationCores"]),
            cohort.authority,
        ),
        "cleanupStatus": "PASSED",
    }
    if report != expected_report:
        raise GateError("COHORT_COMPLETION", "final report authority is incomplete")
    report_path = cohort.path / "REPORT.json"
    immutable_json(report_path, report)
    if _read_exact_json(report_path, None) != report:
        raise GateError("COHORT_COMPLETION", "immutable cohort report verification failed")
    value = {
        "armCount": len(arms), "cohortId": cohort.authority["cohortId"],
        "finalArmDigest": arms[-1]["armDigest"], "reportDigest": digest(report),
        "schema": COHORT_COMPLETE_SCHEMA, "status": "COMPLETE",
    }
    complete_path = cohort.path / "COMPLETE.json"
    immutable_json(complete_path, value)
    persisted = _read_exact_json(complete_path, COHORT_COMPLETE_FIELDS)
    if persisted != value or not _valid_complete(complete_path, cohort.authority):
        raise GateError("COHORT_COMPLETION", "cohort completion receipt verification failed")
    cohort.lock.verify()


def publish_completion_if_eligible(
    cohort: CohortHandle,
    arms: list[dict[str, object]],
    report: dict[str, object],
    *,
    measurements_complete: bool,
    cleanup_ok: bool,
) -> bool:
    if not measurements_complete or not cleanup_ok:
        return False
    complete_cohort(cohort, arms, report)
    return True


def apply_cleanup_outcome(
    value: dict[str, object], exit_code: int, cleanup_ok: bool
) -> tuple[dict[str, object], int]:
    if cleanup_ok:
        return {**value, "cleanupStatus": "PASSED"}, exit_code
    if exit_code == 0:
        return (
            {
                "accepted": False, "cleanupStatus": "FAILED", "diagnosticDigest": None,
                "diagnosticStatus": "NOT_AVAILABLE", "failureStage": "CLEANUP_WORKTREE",
                "releaseGatePassed": False, "schema": REPORT_SCHEMA,
                "status": "FAILED_INCOMPLETE",
            },
            1,
        )
    return {**value, "cleanupStatus": "FAILED"}, exit_code


def _failure_value(error: BaseException, control_home: Path) -> dict[str, object]:
    if isinstance(error, GateError):
        failure_stage = error.stage
        diagnostic = error.diagnostic
    else:
        failure_stage = "CACHE_AUTHORITY"
        diagnostic = str(error).encode()
    diagnostic_digest = None
    diagnostic_status = "NOT_AVAILABLE"
    if diagnostic:
        try:
            redacted = canonical({
                "byteCount": len(diagnostic),
                "contentDigest": "sha256:" + hashlib.sha256(diagnostic).hexdigest(),
                "schema": "codeclew-redacted-gate-diagnostic/1.0",
                "status": "REDACTED",
            }) + b"\n"
            diagnostic_digest = store_diagnostic_bytes(redacted, control_home)
            diagnostic_status = "STORED_PRIVATE"
        except (DiagnosticStoreError, OSError):
            diagnostic_status = "PERSISTENCE_FAILED"
    return {
        "accepted": False, "diagnosticDigest": diagnostic_digest,
        "diagnosticStatus": diagnostic_status, "failureStage": failure_stage,
        "releaseGatePassed": False, "schema": REPORT_SCHEMA, "status": "FAILED_INCOMPLETE",
    }


def _git_line(repository: Path, arguments: list[str], stage: str) -> str:
    completed = run(["git", *arguments], repository, os.environ.copy())
    if completed.returncode != 0 or completed.stdout_truncated or completed.stderr_truncated:
        raise GateError(stage, "git authority command failed", _diagnostic(completed))
    return completed.stdout.decode().strip()


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    report = repository / "benchmarks" / "reports" / "cold-multicore-latest.json"
    private_directory(report.parent)
    control_home = private_directory(
        Path(os.environ.get(
            "CODECLEW_CONTROL_HOME", str(Path.home() / ".cache" / "codeclew-control")
        ))
    )
    gate_base = private_directory(Path.home() / ".cache" / "codeclew-gates")
    gate_admission = acquire_gate_admission(gate_base)
    work = Path(tempfile.mkdtemp(prefix="run.", dir=gate_base))
    work_metadata = work.lstat()
    if (
        RUN_DIRECTORY_PATTERN.fullmatch(work.name) is None
        or not stat.S_ISDIR(work_metadata.st_mode)
        or work_metadata.st_uid != os.geteuid()
        or stat.S_IMODE(work_metadata.st_mode) != 0o700
    ):
        gate_admission.close()
        raise GateError("PREFLIGHT_PRIVATE_STATE", "cold gate work authority is unsafe")
    cohort: CohortHandle | None = None
    arms: list[dict[str, object]] = []
    completion_ready = False
    value: dict[str, object]
    exit_code = 1
    try:
        physical_core_count = physical_cores()
        host = host_authority(physical_core_count)
        cores = int(host["qualificationCores"])
        if cores < 4:
            value = {
                "accepted": False,
                "qualification": {
                    "effectiveCores": cores,
                    "minimumEffectiveCores": 4,
                    "physicalCores": physical_core_count,
                },
                "releaseGatePassed": False, "schema": REPORT_SCHEMA,
                "status": "SKIPPED_UNQUALIFIED_HOST",
            }
            # A typed skip is useful smoke evidence but can never satisfy the
            # formal release controller, whose verifier binds success to zero.
            exit_code = UNQUALIFIED_EXIT_CODE
        else:
            clean_process_preflight()
            probe_cow(work / "cow-probe")
            source_revision = _git_line(
                repository, ["rev-parse", "--verify", "HEAD"], "PREFLIGHT_SOURCE_REVISION"
            )
            if REVISION_PATTERN.fullmatch(source_revision) is None:
                raise GateError("PREFLIGHT_SOURCE_REVISION", "source revision is invalid")
            if _git_line(
                repository, ["status", "--porcelain=v1", "--untracked-files=all"],
                "PREFLIGHT_SOURCE_CLEAN",
            ):
                raise GateError(
                    "PREFLIGHT_SOURCE_CLEAN", "cold runtime evidence requires a clean source HEAD"
                )
            source = frozen_source(repository, work, source_revision)
            authority = dependency_authority(source)
            seed, seed_manifest = prepare_cache_seed(
                source, work, control_home, authority, Path.home()
            )
            cache_digest = str(seed_manifest["contentDigest"])
            boot = boot_authority()
            runs_root = private_directory(
                control_home / "qualification" / "cold-runtime" / "runs-v2"
            )
            cohort = create_cohort(
                runs_root, source_revision, cache_digest, host, boot, authority
            )
            common = {
                "artifactIds": authority["artifactIds"],
                "bootAuthorityDigest": cohort.authority["bootAuthorityDigest"],
                "buildAuthorityDigest": cohort.authority["buildAuthorityDigest"],
                "cacheSeedDigest": cache_digest, "cohortId": cohort.authority["cohortId"],
                "componentIds": authority["componentIds"],
                "hostAuthorityDigest": cohort.authority["hostAuthorityDigest"],
                "logicalCores": host["logicalCores"],
                "physicalCores": host["physicalCores"],
                "qualificationCores": host["qualificationCores"],
                "runtimeKey": authority["runtimeKey"], "sourceRevision": source_revision,
                "totalMemoryBytes": host["totalMemoryBytes"],
                "workerIds": authority["workerIds"],
            }
            predecessor = None
            for sequence, (arm_id, block, order, profile) in enumerate(ARMS, 1):
                cohort.lock.verify()
                gate_admission.verify()
                destination = work / f"cache-{arm_id}"
                observed = clone_seed(seed, destination, str(authority["runtimeKey"]))
                if observed["contentDigest"] != cache_digest:
                    raise GateError("PREFLIGHT_CACHE_CLONES", "cache clone digest mismatch")
                arm = run_arm(
                    source, destination, work / f"state-{arm_id}",
                    cohort.arms / f"{sequence:02d}-{arm_id}.json", Path.home(), common,
                    arm_id, block, order, profile, sequence, predecessor,
                    int(host["physicalCores"]),
                )
                arms.append(arm)
                predecessor = str(arm["armDigest"])
                cohort.lock.verify()
            value = aggregate(arms, source_revision, cores, cohort.authority)
            completion_ready = True
            exit_code = 0 if value["accepted"] else 1
    except (GateError, CacheAuthorityError, OSError, ValueError, TypeError) as error:
        value = _failure_value(error, control_home)
        exit_code = 1

    try:
        gate_admission.verify()
        cleanup_ok = cleanup_owned_tree(
            work, gate_base, (work_metadata.st_dev, work_metadata.st_ino)
        )
    except Exception:
        cleanup_ok = False
    value, exit_code = apply_cleanup_outcome(value, exit_code, cleanup_ok)
    if cohort is not None:
        try:
            gate_admission.verify()
            publish_completion_if_eligible(
                cohort,
                arms,
                value,
                measurements_complete=completion_ready,
                cleanup_ok=cleanup_ok,
            )
        except (GateError, OSError, ValueError, TypeError) as error:
            value = _failure_value(
                GateError("COHORT_COMPLETION", str(error)), control_home
            )
            value["cleanupStatus"] = "PASSED"
            exit_code = 1
    try:
        gate_admission.verify()
        atomic_json(report, value)
        stream = sys.stdout if exit_code == 0 else sys.stderr
        print(canonical(value).decode(), file=stream)
        return exit_code
    finally:
        if cohort is not None:
            cohort.close()
        gate_admission.close()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (GateError, CacheAuthorityError, OSError, ValueError, TypeError) as error:
        stage = error.stage if isinstance(error, GateError) else "PREFLIGHT_GATE_AUTHORITY"
        failure = {
            "accepted": False,
            "diagnosticDigest": None,
            "diagnosticStatus": "NOT_AVAILABLE",
            "failureStage": stage,
            "releaseGatePassed": False,
            "schema": REPORT_SCHEMA,
            "status": "FAILED_INCOMPLETE",
        }
        print(canonical(failure).decode(), file=sys.stderr)
        raise SystemExit(1)
