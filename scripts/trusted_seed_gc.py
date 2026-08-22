#!/usr/bin/env python3
"""Crash-recoverable reclamation for unreachable trusted-seed epochs."""

from __future__ import annotations

import argparse
import contextlib
from dataclasses import dataclass
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import select
import signal
import stat
import subprocess
import sys
import threading
import time
from typing import BinaryIO


RELEASE_EPOCH = re.compile(r"release-N-[0-9a-f]{40}\Z")
FAILED_EPOCH = re.compile(r"failed-[0-9a-f]{40}-[1-9][0-9]{0,9}\Z")
TOMBSTONE = re.compile(
    r"\.gc-(release-N-[0-9a-f]{40}|failed-[0-9a-f]{40}-[1-9][0-9]{0,9})\Z"
)
DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")
ALLOWED_DIRECTORY_MODES = {0o500, 0o700}
ALLOWED_FILE_MODES = {0o400, 0o500, 0o600}
MAX_ENTRIES = 250_000
MAX_APPARENT_BYTES = 64 * 1024**3
MAX_DEPTH = 64
MAX_LOCKS = 8192
MAX_GITDIR_FILES = 1024
MAX_GITDIR_BYTES = 64 * 1024
MAX_ROOT_ENTRIES = 4096


class UnsafeEpoch(Exception):
    pass


class BusyEpoch(Exception):
    pass


class ExternalGitdir(Exception):
    pass


class AuthorityRefusal(Exception):
    def __init__(self, reason: str):
        super().__init__(reason)
        self.reason = reason


class SupervisorInterrupted(Exception):
    def __init__(self, signum: int):
        self.signum = signum
        super().__init__(f"supervisor interrupted by signal {signum}")


class ProcessGroupReapFailure(AuthorityRefusal):
    def __init__(self, leader_exit_code: int):
        self.leader_exit_code = leader_exit_code
        super().__init__("PROCESS_GROUP_AUTHORITY")


DESCENDANT_LEAK_EXIT = 125
PROCESS_GROUP_GRACE_SECONDS = 0.25
PROCESS_GROUP_TERM_SECONDS = 2.0
PROCESS_GROUP_KILL_SECONDS = 2.0
PROCESS_GROUP_GUARDIAN_READY_SECONDS = 2.0
TRUSTED_PS = "/bin/ps"
_GUARDIAN_PROCESSES: set[subprocess.Popen[bytes]] = set()
_GUARDIAN_PROCESSES_LOCK = threading.Lock()
PROCESS_GROUP_GUARDIAN = r"""
import fcntl
import json
import os
import stat
import subprocess
import sys
import time

expected_value = json.loads(sys.argv[1])
process_group = int(sys.argv[2])
ps_path = sys.argv[3]
lifecycle_fd = int(sys.argv[4])
ready_fd = int(sys.argv[5])


def pin_forever():
    while True:
        time.sleep(3600)


def observe():
    try:
        completed = subprocess.run(
            [ps_path, "-axo", "pid=,pgid=,lstart="],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except Exception:
        return None
    if completed.returncode != 0:
        return None
    all_processes = {}
    group = {}
    for line in completed.stdout.splitlines():
        if not line.strip():
            continue
        fields = line.strip().split(None, 2)
        if len(fields) != 3:
            return None
        try:
            pid = int(fields[0])
            pgid = int(fields[1])
        except ValueError:
            return None
        start = fields[2]
        if pid <= 0 or pgid <= 0 or not start or pid in all_processes:
            return None
        all_processes[pid] = (pgid, start)
        if pgid == process_group:
            group[pid] = start
    if not all_processes:
        return None
    return group


try:
    if (
        not isinstance(expected_value, list)
        or not expected_value
        or not os.path.isabs(ps_path)
    ):
        raise ValueError("invalid guardian authority")
    tracked = {}
    for item in expected_value:
        if (
            not isinstance(item, list)
            or len(item) != 2
            or not isinstance(item[0], int)
            or item[0] <= 0
            or not isinstance(item[1], str)
            or not item[1]
            or item[0] in tracked
        ):
            raise ValueError("invalid process identity")
        tracked[item[0]] = item[1]
    lifecycle = os.fstat(lifecycle_fd)
    executable = os.stat(ps_path, follow_symlinks=False)
    if (
        not stat.S_ISREG(executable.st_mode)
        or executable.st_uid != 0
        or executable.st_mode & 0o022
        or lifecycle.st_uid != os.getuid()
    ):
        raise ValueError("unsafe guardian authority")
    fcntl.flock(lifecycle_fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
    current = observe()
    if current is None:
        pin_forever()
    os.write(ready_fd, b"READY\n")
    os.close(ready_fd)
except Exception:
    try:
        os.close(ready_fd)
    except OSError:
        pass
    pin_forever()

while True:
    if not current:
        break
    if any(pid in current and current[pid] != start for pid, start in tracked.items()):
        pin_forever()
    live_tracked = {pid for pid, start in tracked.items() if current.get(pid) == start}
    unknown = set(current) - set(tracked)
    if unknown and not live_tracked:
        pin_forever()
    for pid in unknown:
        tracked[pid] = current[pid]
    time.sleep(0.1)
    current = observe()
    if current is None:
        pin_forever()
"""


def _remember_guardian(process: subprocess.Popen[bytes]) -> None:
    with _GUARDIAN_PROCESSES_LOCK:
        _GUARDIAN_PROCESSES.add(process)

    def reap() -> None:
        try:
            process.wait()
        finally:
            with _GUARDIAN_PROCESSES_LOCK:
                _GUARDIAN_PROCESSES.discard(process)

    threading.Thread(target=reap, daemon=True).start()


def _process_group_exists(process_group: int) -> bool:
    try:
        os.killpg(process_group, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def _wait_process_group_gone(process_group: int, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while _process_group_exists(process_group):
        if time.monotonic() >= deadline:
            return False
        time.sleep(0.01)
    return True


def _terminate_process_group(process_group: int) -> None:
    if _wait_process_group_gone(process_group, PROCESS_GROUP_GRACE_SECONDS):
        return
    try:
        os.killpg(process_group, signal.SIGTERM)
    except ProcessLookupError:
        return
    if _wait_process_group_gone(process_group, PROCESS_GROUP_TERM_SECONDS):
        return
    try:
        os.killpg(process_group, signal.SIGKILL)
    except ProcessLookupError:
        return
    if not _wait_process_group_gone(process_group, PROCESS_GROUP_KILL_SECONDS):
        raise AuthorityRefusal("PROCESS_GROUP_AUTHORITY")


def _pin_process_group_until_gone(lifecycle: BinaryIO, process_group: int) -> None:
    identities = _process_group_identities(process_group)
    if identities is None:
        _hold_process_group_pin_locally(process_group, None)
        return
    if not identities:
        return
    encoded = json.dumps(identities, separators=(",", ":"))
    ready_read, ready_write = os.pipe()
    guardian: subprocess.Popen[bytes] | None = None
    try:
        guardian = subprocess.Popen(
            [
                sys.executable,
                "-I",
                "-S",
                "-c",
                PROCESS_GROUP_GUARDIAN,
                encoded,
                str(process_group),
                TRUSTED_PS,
                str(lifecycle.fileno()),
                str(ready_write),
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
            pass_fds=(lifecycle.fileno(), ready_write),
        )
        os.close(ready_write)
        ready_write = -1
        readable, _, _ = select.select(
            [ready_read], [], [], PROCESS_GROUP_GUARDIAN_READY_SECONDS
        )
        ready = os.read(ready_read, 16) if readable else b""
        if ready == b"READY\n" and guardian.poll() is None:
            _remember_guardian(guardian)
            guardian = None
            return
    except OSError:
        pass
    finally:
        if ready_write >= 0:
            os.close(ready_write)
        os.close(ready_read)
        if guardian is not None and guardian.poll() is None:
            _remember_guardian(guardian)
    # Losing the pin would permit destructive GC while the governed group is
    # still live. Any startup/handshake ambiguity retains this process and its
    # lifecycle lock until a complete trusted observation proves disappearance.
    _hold_process_group_pin_locally(process_group, identities)


def _ps_group_snapshot(process_group: int) -> dict[int, str] | None:
    try:
        completed = subprocess.run(
            [TRUSTED_PS, "-axo", "pid=,pgid=,lstart="],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except OSError:
        return None
    if completed.returncode != 0:
        return None
    all_processes: dict[int, tuple[int, str]] = {}
    group: dict[int, str] = {}
    for line in completed.stdout.splitlines():
        if not line.strip():
            continue
        fields = line.strip().split(None, 2)
        if len(fields) != 3:
            return None
        try:
            pid = int(fields[0])
            pgid = int(fields[1])
        except ValueError:
            return None
        start = fields[2]
        if pid <= 0 or pgid <= 0 or not start or pid in all_processes:
            return None
        all_processes[pid] = (pgid, start)
        if pgid == process_group:
            group[pid] = start
    if not all_processes:
        return None
    return group


def _hold_process_group_pin_locally(
    process_group: int, identities: list[tuple[int, str]] | None
) -> None:
    if identities is None:
        while True:
            time.sleep(3600)
    tracked = dict(identities)
    while True:
        current = _ps_group_snapshot(process_group)
        if current is None:
            while True:
                time.sleep(3600)
        if not current:
            return
        if any(
            pid in current and current[pid] != start
            for pid, start in tracked.items()
        ):
            while True:
                time.sleep(3600)
        live_tracked = {
            pid for pid, start in tracked.items() if current.get(pid) == start
        }
        unknown = set(current) - set(tracked)
        if unknown and not live_tracked:
            while True:
                time.sleep(3600)
        for pid in unknown:
            tracked[pid] = current[pid]
        time.sleep(0.1)


def _process_group_identities(
    process_group: int,
) -> list[tuple[int, str]] | None:
    snapshot = _ps_group_snapshot(process_group)
    if snapshot is None:
        return None
    identities = sorted(snapshot.items())
    if not identities and _process_group_exists(process_group):
        return None
    return identities


def _identities_still_live(identities: list[tuple[int, str]]) -> bool:
    if not identities:
        return False
    try:
        output = subprocess.check_output(
            [TRUSTED_PS, "-axo", "pid=,lstart="], text=True
        )
    except (OSError, subprocess.SubprocessError):
        return True
    current = {}
    for line in output.splitlines():
        fields = line.strip().split(None, 1)
        if len(fields) == 2:
            current[fields[0]] = fields[1]
    return any(current.get(str(pid)) == start for pid, start in identities)


@contextlib.contextmanager
def _termination_guard():
    if threading.current_thread() is not threading.main_thread():
        yield
        return
    watched = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)
    previous = {signum: signal.getsignal(signum) for signum in watched}
    interrupted = False

    def interrupt(signum, _frame):
        nonlocal interrupted
        if interrupted:
            return
        interrupted = True
        raise SupervisorInterrupted(signum)

    try:
        for signum in watched:
            signal.signal(signum, interrupt)
        yield
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)


@dataclass(frozen=True)
class Snapshot:
    identity: tuple[int, int]
    locks: dict[tuple[str, ...], tuple[int, int]]
    apparent_bytes: int
    entries: int


@dataclass
class LockedFile:
    stream: BinaryIO
    relative: tuple[str, ...]
    identity: tuple[int, int]


_BOOTSTRAP = None


def _bootstrap_verifier():
    global _BOOTSTRAP
    if _BOOTSTRAP is None:
        path = Path(__file__).resolve().parent.parent / "bootstrap" / "clew_bootstrap.py"
        specification = importlib.util.spec_from_file_location(
            "_codeclew_trusted_seed_bootstrap", path
        )
        if specification is None or specification.loader is None:
            raise AuthorityRefusal("CAPSULE_AUTHORITY")
        module = importlib.util.module_from_spec(specification)
        sys.modules[specification.name] = module
        try:
            specification.loader.exec_module(module)
        except BaseException as error:
            raise AuthorityRefusal("CAPSULE_AUTHORITY") from error
        _BOOTSTRAP = module
    return _BOOTSTRAP


def _is_epoch(name: str) -> bool:
    return RELEASE_EPOCH.fullmatch(name) is not None or FAILED_EPOCH.fullmatch(name) is not None


def _open_private_root(value: str) -> tuple[str, int]:
    if not os.path.isabs(value) or ".." in value.split(os.sep) or value != os.path.normpath(value):
        raise AuthorityRefusal("ROOT_AUTHORITY")
    flags = os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(os.sep, flags)
    try:
        components = [part for part in value.split(os.sep) if part]
        if not components:
            raise AuthorityRefusal("ROOT_AUTHORITY")
        for index, component in enumerate(components):
            child = os.open(component, flags, dir_fd=descriptor)
            metadata = os.fstat(child)
            leaf = index == len(components) - 1
            valid = stat.S_ISDIR(metadata.st_mode) and metadata.st_uid in {0, os.geteuid()}
            if not leaf:
                valid = valid and stat.S_IMODE(metadata.st_mode) & 0o022 == 0
            else:
                valid = (
                    valid
                    and metadata.st_uid == os.geteuid()
                    and stat.S_IMODE(metadata.st_mode) == 0o700
                )
            if not valid:
                os.close(child)
                raise AuthorityRefusal("ROOT_AUTHORITY")
            os.close(descriptor)
            descriptor = child
        return value, descriptor
    except BaseException:
        os.close(descriptor)
        raise


def _lifecycle_lock(root_fd: int, *, shared: bool = False) -> BinaryIO:
    try:
        os.mkdir("locks", 0o700, dir_fd=root_fd)
        _fsync_directory(root_fd)
    except FileExistsError:
        pass
    try:
        locks_fd, locks_metadata = _open_directory_at(root_fd, "locks")
    except (OSError, UnsafeEpoch) as error:
        raise AuthorityRefusal("LIFECYCLE_AUTHORITY") from error
    if (
        locks_metadata.st_uid != os.geteuid()
        or stat.S_IMODE(locks_metadata.st_mode) != 0o700
    ):
        os.close(locks_fd)
        raise AuthorityRefusal("LIFECYCLE_AUTHORITY")
    flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open("lifecycle.lock", flags, 0o600, dir_fd=locks_fd)
    except OSError as error:
        os.close(locks_fd)
        raise AuthorityRefusal("LIFECYCLE_AUTHORITY") from error
    os.close(locks_fd)
    metadata = os.fstat(descriptor)
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) != 0o600
    ):
        os.close(descriptor)
        raise AuthorityRefusal("LIFECYCLE_AUTHORITY")
    stream = os.fdopen(descriptor, "r+b")
    try:
        fcntl.flock(stream.fileno(), fcntl.LOCK_SH if shared else fcntl.LOCK_EX)
    except BaseException:
        stream.close()
        raise
    return stream


def _read_current(root_fd: int) -> dict[str, object] | None:
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open("current.json", flags, dir_fd=root_fd)
    except FileNotFoundError:
        return None
    except OSError as error:
        raise AuthorityRefusal("LOCATOR_AUTHORITY") from error
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_size > MAX_GITDIR_BYTES
        ):
            raise AuthorityRefusal("LOCATOR_AUTHORITY")
        payload = b""
        while len(payload) <= MAX_GITDIR_BYTES:
            block = os.read(descriptor, min(8192, MAX_GITDIR_BYTES + 1 - len(payload)))
            if not block:
                break
            payload += block
        if len(payload) > MAX_GITDIR_BYTES:
            raise AuthorityRefusal("LOCATOR_AUTHORITY")
        value = json.loads(payload)
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise AuthorityRefusal("LOCATOR_AUTHORITY") from error
    finally:
        os.close(descriptor)
    generation = value.get("generation") if isinstance(value, dict) else None
    rollback = value.get("rollback") if isinstance(value, dict) else None
    rollback_valid = rollback is None
    if isinstance(rollback, dict):
        rollback_generation = rollback.get("generation")
        rollback_valid = (
            set(rollback)
            == {
                "epoch",
                "generation",
                "publicationDigest",
                "runtimeKey",
                "seedDigest",
            }
            and isinstance(rollback.get("epoch"), str)
            and RELEASE_EPOCH.fullmatch(rollback["epoch"]) is not None
            and isinstance(rollback_generation, int)
            and not isinstance(rollback_generation, bool)
            and rollback_generation >= 1
            and isinstance(rollback.get("publicationDigest"), str)
            and DIGEST.fullmatch(rollback["publicationDigest"]) is not None
            and isinstance(rollback.get("runtimeKey"), str)
            and DIGEST.fullmatch(rollback["runtimeKey"]) is not None
            and isinstance(rollback.get("seedDigest"), str)
            and DIGEST.fullmatch(rollback["seedDigest"]) is not None
        )
    if (
        not isinstance(value, dict)
        or set(value)
        != {
            "epoch",
            "generation",
            "publicationDigest",
            "rollback",
            "runtimeKey",
            "schema",
            "seedDigest",
        }
        or value.get("schema") != "codeclew-trusted-seed-locator/2.0"
        or not isinstance(value.get("epoch"), str)
        or RELEASE_EPOCH.fullmatch(value["epoch"]) is None
        or not isinstance(generation, int)
        or isinstance(generation, bool)
        or generation < 1
        or not isinstance(value.get("publicationDigest"), str)
        or DIGEST.fullmatch(value["publicationDigest"]) is None
        or not isinstance(value.get("runtimeKey"), str)
        or DIGEST.fullmatch(value["runtimeKey"]) is None
        or not isinstance(value.get("seedDigest"), str)
        or DIGEST.fullmatch(value["seedDigest"]) is None
        or not rollback_valid
        or (generation == 1) != (rollback is None)
        or (
            isinstance(rollback, dict)
            and (
                rollback["generation"] != generation - 1
                or rollback["epoch"] == value["epoch"]
            )
        )
        or payload
        != (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
    ):
        raise AuthorityRefusal("LOCATOR_AUTHORITY")
    return value


def _atomic_json_at(root_fd: int, name: str, value: dict[str, object]) -> None:
    temporary = f".{name}.{secrets.token_hex(16)}.tmp"
    descriptor = os.open(
        temporary,
        os.O_CREAT | os.O_EXCL | os.O_WRONLY | getattr(os, "O_NOFOLLOW", 0),
        0o600,
        dir_fd=root_fd,
    )
    try:
        payload = (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
        with os.fdopen(descriptor, "wb", closefd=False) as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
    finally:
        os.close(descriptor)
    try:
        os.rename(temporary, name, src_dir_fd=root_fd, dst_dir_fd=root_fd)
    finally:
        if _exists_at(root_fd, temporary):
            os.unlink(temporary, dir_fd=root_fd)
    _fsync_directory(root_fd)


def _publication_record_for_locator(
    locator: dict[str, object],
) -> dict[str, object]:
    record = dict(locator)
    record["schema"] = "codeclew-trusted-seed-publication/1.0"
    return record


def _read_publication(
    root_fd: int, epoch: str, *, container_name: str | None = None
) -> dict[str, object]:
    epoch_fd, _metadata = _open_directory_at(
        root_fd, container_name if container_name is not None else epoch
    )
    try:
        descriptor = os.open(
            "publication.json",
            os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
            dir_fd=epoch_fd,
        )
        try:
            metadata = os.fstat(descriptor)
            if (
                not stat.S_ISREG(metadata.st_mode)
                or metadata.st_uid != os.geteuid()
                or stat.S_IMODE(metadata.st_mode) != 0o400
                or metadata.st_size > MAX_GITDIR_BYTES
            ):
                raise AuthorityRefusal("PUBLICATION_AUTHORITY")
            payload = b""
            while len(payload) <= MAX_GITDIR_BYTES:
                block = os.read(
                    descriptor,
                    min(8192, MAX_GITDIR_BYTES + 1 - len(payload)),
                )
                if not block:
                    break
                payload += block
        finally:
            os.close(descriptor)
    except (OSError, UnsafeEpoch) as error:
        raise AuthorityRefusal("PUBLICATION_AUTHORITY") from error
    finally:
        os.close(epoch_fd)
    try:
        value = json.loads(payload)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise AuthorityRefusal("PUBLICATION_AUTHORITY") from error
    unsigned = dict(value) if isinstance(value, dict) else {}
    publication_digest = unsigned.pop("publicationDigest", None)
    if (
        not isinstance(value, dict)
        or value.get("schema") != "codeclew-trusted-seed-publication/1.0"
        or not isinstance(publication_digest, str)
        or DIGEST.fullmatch(publication_digest) is None
        or publication_digest
        != "sha256:"
        + hashlib.sha256(
            json.dumps(unsigned, sort_keys=True, separators=(",", ":")).encode()
        ).hexdigest()
        or payload
        != (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
    ):
        raise AuthorityRefusal("PUBLICATION_AUTHORITY")
    return value


def _validate_publication(
    root_fd: int,
    locator: dict[str, object],
    *,
    exact: bool,
    container_name: str | None = None,
) -> None:
    epoch = locator["epoch"]
    assert isinstance(epoch, str)
    record = _read_publication(root_fd, epoch, container_name=container_name)
    if exact:
        if record != _publication_record_for_locator(locator):
            raise AuthorityRefusal("PUBLICATION_AUTHORITY")
        return
    for field in (
        "epoch",
        "generation",
        "publicationDigest",
        "runtimeKey",
        "seedDigest",
    ):
        if record.get(field) != locator.get(field):
            raise AuthorityRefusal("PUBLICATION_AUTHORITY")


def _create_or_validate_publication(
    root_fd: int,
    locator: dict[str, object],
    *,
    container_name: str | None = None,
) -> None:
    epoch = locator["epoch"]
    assert isinstance(epoch, str)
    epoch_fd, _metadata = _open_directory_at(
        root_fd, container_name if container_name is not None else epoch
    )
    descriptor: int | None = None
    temporary = f".publication.{secrets.token_hex(16)}.tmp"
    try:
        descriptor = os.open(
            temporary,
            os.O_CREAT
            | os.O_EXCL
            | os.O_WRONLY
            | getattr(os, "O_NOFOLLOW", 0),
            0o600,
            dir_fd=epoch_fd,
        )
        payload = (
            json.dumps(
                _publication_record_for_locator(locator),
                sort_keys=True,
                separators=(",", ":"),
            )
            + "\n"
        ).encode()
        with os.fdopen(descriptor, "wb", closefd=False) as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.fchmod(descriptor, 0o400)
        os.fsync(descriptor)
        try:
            os.link(
                temporary,
                "publication.json",
                src_dir_fd=epoch_fd,
                dst_dir_fd=epoch_fd,
                follow_symlinks=False,
            )
        except FileExistsError:
            pass
        _fsync_directory(epoch_fd)
    finally:
        if descriptor is not None:
            os.close(descriptor)
        try:
            try:
                os.unlink(temporary, dir_fd=epoch_fd)
                _fsync_directory(epoch_fd)
            except FileNotFoundError:
                pass
        finally:
            os.close(epoch_fd)
    _validate_publication(
        root_fd,
        locator,
        exact=True,
        container_name=container_name,
    )


def _open_directory_at(parent_fd: int, name: str) -> tuple[int, os.stat_result]:
    flags = os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0)
    before = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    descriptor = os.open(name, flags, dir_fd=parent_fd)
    metadata = os.fstat(descriptor)
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or stat.S_ISLNK(before.st_mode)
        or (before.st_dev, before.st_ino) != (metadata.st_dev, metadata.st_ino)
    ):
        os.close(descriptor)
        raise UnsafeEpoch
    return descriptor, metadata


def _lexically_inside(root: str, target: str) -> bool:
    try:
        return os.path.commonpath((root, target)) == root
    except ValueError:
        return False


def _check_gitdir(
    directory_fd: int,
    name: str,
    metadata: os.stat_result,
    internal_roots: tuple[str, ...],
) -> None:
    if metadata.st_size > MAX_GITDIR_BYTES:
        raise UnsafeEpoch
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(name, flags, dir_fd=directory_fd)
    try:
        opened = os.fstat(descriptor)
        if (opened.st_dev, opened.st_ino) != (metadata.st_dev, metadata.st_ino):
            raise UnsafeEpoch
        payload = b""
        while len(payload) <= MAX_GITDIR_BYTES:
            block = os.read(descriptor, min(8192, MAX_GITDIR_BYTES + 1 - len(payload)))
            if not block:
                break
            payload += block
        if len(payload) > MAX_GITDIR_BYTES:
            raise UnsafeEpoch
        line = payload.decode("utf-8")
    except (OSError, UnicodeError) as error:
        raise UnsafeEpoch from error
    finally:
        os.close(descriptor)
    match = re.fullmatch(r"gitdir: ([^\r\n]+)\n?", line)
    if match is None or not os.path.isabs(match.group(1)):
        raise UnsafeEpoch
    target = os.path.normpath(match.group(1))
    if any(_lexically_inside(root, target) for root in internal_roots):
        return
    try:
        os.lstat(target)
    except FileNotFoundError:
        return
    except OSError as error:
        raise UnsafeEpoch from error
    raise ExternalGitdir


def _bounded_names(descriptor: int, limit: int) -> list[str]:
    if limit < 0:
        raise UnsafeEpoch
    names: list[str] = []
    try:
        with os.scandir(descriptor) as entries:
            for entry in entries:
                if len(names) >= limit:
                    raise UnsafeEpoch
                names.append(entry.name)
    except UnsafeEpoch:
        raise
    except OSError as error:
        raise UnsafeEpoch from error
    names.sort()
    return names


def _scan_directory(
    descriptor: int,
    root_device: int,
    relative: tuple[str, ...],
    internal_roots: tuple[str, ...],
    state: dict[str, object],
) -> None:
    if len(relative) > MAX_DEPTH:
        raise UnsafeEpoch
    names = _bounded_names(descriptor, MAX_ENTRIES - int(state["entries"]))
    for name in names:
        try:
            metadata = os.stat(name, dir_fd=descriptor, follow_symlinks=False)
        except OSError as error:
            raise UnsafeEpoch from error
        if metadata.st_uid != os.geteuid() or metadata.st_dev != root_device:
            raise UnsafeEpoch
        state["entries"] = int(state["entries"]) + 1
        if int(state["entries"]) > MAX_ENTRIES:
            raise UnsafeEpoch
        child_relative = relative + (name,)
        mode = stat.S_IMODE(metadata.st_mode)
        if stat.S_ISDIR(metadata.st_mode) and not stat.S_ISLNK(metadata.st_mode):
            if mode not in ALLOWED_DIRECTORY_MODES:
                raise UnsafeEpoch
            child, opened = _open_directory_at(descriptor, name)
            try:
                if opened.st_uid != os.geteuid() or opened.st_dev != root_device:
                    raise UnsafeEpoch
                _scan_directory(child, root_device, child_relative, internal_roots, state)
            finally:
                os.close(child)
        elif stat.S_ISREG(metadata.st_mode):
            if mode not in ALLOWED_FILE_MODES:
                raise UnsafeEpoch
            state["bytes"] = int(state["bytes"]) + metadata.st_size
            if int(state["bytes"]) > MAX_APPARENT_BYTES:
                raise UnsafeEpoch
            if name.endswith(".lock") or name.endswith(".lease"):
                locks = state["locks"]
                assert isinstance(locks, dict)
                locks[child_relative] = (metadata.st_dev, metadata.st_ino)
                if len(locks) > MAX_LOCKS:
                    raise UnsafeEpoch
            if name == ".git":
                state["gitdirs"] = int(state["gitdirs"]) + 1
                if int(state["gitdirs"]) > MAX_GITDIR_FILES:
                    raise UnsafeEpoch
                _check_gitdir(descriptor, name, metadata, internal_roots)
        else:
            raise UnsafeEpoch


def _scan_candidate(root_fd: int, name: str, root_value: str) -> Snapshot:
    descriptor, metadata = _open_directory_at(root_fd, name)
    try:
        if (
            metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) not in ALLOWED_DIRECTORY_MODES
        ):
            raise UnsafeEpoch
        source = TOMBSTONE.fullmatch(name)
        epoch = source.group(1) if source is not None else name
        internal_roots = (
            os.path.join(root_value, epoch),
            os.path.join(root_value, ".gc-" + epoch),
        )
        state: dict[str, object] = {"bytes": 0, "entries": 0, "gitdirs": 0, "locks": {}}
        _scan_directory(descriptor, metadata.st_dev, (), internal_roots, state)
        locks = state["locks"]
        assert isinstance(locks, dict)
        return Snapshot(
            identity=(metadata.st_dev, metadata.st_ino),
            locks=locks,
            apparent_bytes=int(state["bytes"]),
            entries=int(state["entries"]),
        )
    finally:
        os.close(descriptor)


def _open_relative_file(root_fd: int, root_name: str, relative: tuple[str, ...]) -> int:
    descriptor, _metadata = _open_directory_at(root_fd, root_name)
    try:
        for component in relative[:-1]:
            child, _child_metadata = _open_directory_at(descriptor, component)
            os.close(descriptor)
            descriptor = child
        flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
        return os.open(relative[-1], flags, dir_fd=descriptor)
    finally:
        os.close(descriptor)


def _close_locks(held: dict[tuple[str, ...], LockedFile]) -> None:
    for relative in sorted(held, reverse=True):
        held[relative].stream.close()
    held.clear()


def _acquire_locks(
    root_fd: int,
    root_name: str,
    snapshot: Snapshot,
    held: dict[tuple[str, ...], LockedFile] | None = None,
) -> dict[tuple[str, ...], LockedFile]:
    result = {} if held is None else held
    try:
        for relative, identity in sorted(snapshot.locks.items()):
            existing = result.get(relative)
            if existing is not None:
                if existing.identity != identity:
                    raise UnsafeEpoch
                continue
            descriptor = _open_relative_file(root_fd, root_name, relative)
            metadata = os.fstat(descriptor)
            if (
                not stat.S_ISREG(metadata.st_mode)
                or metadata.st_uid != os.geteuid()
                or stat.S_IMODE(metadata.st_mode) not in ALLOWED_FILE_MODES
                or (metadata.st_dev, metadata.st_ino) != identity
            ):
                os.close(descriptor)
                raise UnsafeEpoch
            stream = os.fdopen(descriptor, "rb")
            try:
                fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                stream.close()
                raise BusyEpoch from error
            result[relative] = LockedFile(stream, relative, identity)
        return result
    except BaseException:
        if held is None:
            _close_locks(result)
        raise


def _locks_stable(held: dict[tuple[str, ...], LockedFile], snapshot: Snapshot) -> bool:
    if set(held) != set(snapshot.locks):
        return False
    for relative, locked in held.items():
        metadata = os.fstat(locked.stream.fileno())
        if (metadata.st_dev, metadata.st_ino) != snapshot.locks[relative]:
            return False
    return True


def _fsync_directory(descriptor: int) -> None:
    os.fsync(descriptor)


def _delete_directory_contents(
    descriptor: int, root_device: int, state: list[int] | None = None
) -> None:
    if state is None:
        state = [0]
    metadata = os.fstat(descriptor)
    if metadata.st_uid != os.geteuid() or metadata.st_dev != root_device:
        raise UnsafeEpoch
    os.fchmod(descriptor, 0o700)
    for name in _bounded_names(descriptor, MAX_ENTRIES - state[0]):
        state[0] += 1
        child_metadata = os.stat(name, dir_fd=descriptor, follow_symlinks=False)
        if child_metadata.st_uid != os.geteuid() or child_metadata.st_dev != root_device:
            raise UnsafeEpoch
        if stat.S_ISDIR(child_metadata.st_mode) and not stat.S_ISLNK(child_metadata.st_mode):
            child, opened = _open_directory_at(descriptor, name)
            try:
                if opened.st_dev != root_device or opened.st_uid != os.geteuid():
                    raise UnsafeEpoch
                _delete_directory_contents(child, root_device, state)
            finally:
                os.close(child)
            os.rmdir(name, dir_fd=descriptor)
        elif stat.S_ISREG(child_metadata.st_mode):
            os.unlink(name, dir_fd=descriptor)
        else:
            raise UnsafeEpoch
    _fsync_directory(descriptor)


def _delete_tombstone(root_fd: int, name: str, identity: tuple[int, int]) -> None:
    descriptor, metadata = _open_directory_at(root_fd, name)
    try:
        if (metadata.st_dev, metadata.st_ino) != identity:
            raise UnsafeEpoch
        _delete_directory_contents(descriptor, metadata.st_dev)
    finally:
        os.close(descriptor)
    os.rmdir(name, dir_fd=root_fd)
    _fsync_directory(root_fd)


def _exists_at(root_fd: int, name: str) -> bool:
    try:
        os.stat(name, dir_fd=root_fd, follow_symlinks=False)
        return True
    except FileNotFoundError:
        return False


def _restore(root_fd: int, tombstone: str, epoch: str) -> bool:
    if _exists_at(root_fd, epoch) or not _exists_at(root_fd, tombstone):
        return False
    try:
        os.rename(tombstone, epoch, src_dir_fd=root_fd, dst_dir_fd=root_fd)
        _fsync_directory(root_fd)
        return True
    except OSError:
        return False


def _base_report() -> dict[str, int | str]:
    return {
        "deletedBytes": 0,
        "deletedEpochs": 0,
        "protectedEpochs": 0,
        "recoveredTombstones": 0,
        "retainedTombstones": 0,
        "schema": "codeclew-trusted-seed-gc/1.0",
        "skippedBusy": 0,
        "skippedGitdir": 0,
        "skippedUnsafe": 0,
        "status": "PASS",
    }


def _seed_locator(
    root_fd: int,
    root_value: str,
    epoch: str,
    expected_source_tree: str | None = None,
    logical_epoch: str | None = None,
) -> dict[str, str]:
    authority_epoch = logical_epoch if logical_epoch is not None else epoch
    if RELEASE_EPOCH.fullmatch(authority_epoch) is None:
        raise AuthorityRefusal("PUBLISH_AUTHORITY")
    root_identity = os.fstat(root_fd)
    root_path_identity = os.stat(root_value, follow_symlinks=False)
    if (root_identity.st_dev, root_identity.st_ino) != (
        root_path_identity.st_dev,
        root_path_identity.st_ino,
    ):
        raise AuthorityRefusal("ROOT_AUTHORITY")
    epoch_fd, epoch_metadata = _open_directory_at(root_fd, epoch)
    try:
        if (
            epoch_metadata.st_uid != os.geteuid()
            or stat.S_IMODE(epoch_metadata.st_mode) != 0o700
        ):
            raise AuthorityRefusal("PUBLISH_AUTHORITY")
        flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
        descriptor = os.open("seed.json", flags, dir_fd=epoch_fd)
        try:
            metadata = os.fstat(descriptor)
            if (
                not stat.S_ISREG(metadata.st_mode)
                or metadata.st_uid != os.geteuid()
                or stat.S_IMODE(metadata.st_mode) != 0o400
                or metadata.st_size > 1024 * 1024
            ):
                raise AuthorityRefusal("PUBLISH_AUTHORITY")
            payload = b""
            while len(payload) <= 1024 * 1024:
                block = os.read(descriptor, min(8192, 1024 * 1024 + 1 - len(payload)))
                if not block:
                    break
                payload += block
            seed = json.loads(payload)
        finally:
            os.close(descriptor)
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise AuthorityRefusal("PUBLISH_AUTHORITY") from error
    finally:
        os.close(epoch_fd)
    expected_fields = {
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
    unsigned = dict(seed) if isinstance(seed, dict) else {}
    expected_digest = unsigned.pop("seedDigest", None)
    actual_digest = "sha256:" + hashlib.sha256(
        json.dumps(unsigned, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()
    source_revision = authority_epoch.removeprefix("release-N-")
    artifact_hashes = seed.get("artifactHashes") if isinstance(seed, dict) else None
    worker_hashes = seed.get("workerTreeHashes") if isinstance(seed, dict) else None
    evidence_digests = seed.get("buildEvidenceDigests") if isinstance(seed, dict) else None
    if (
        not isinstance(seed, dict)
        or set(seed) != expected_fields
        or seed.get("schema") != "codeclew-trusted-release-seed/1.0"
        or seed.get("mode") != "RELEASE"
        or expected_digest != actual_digest
        or seed.get("sourceRevision") != source_revision
        or not isinstance(seed.get("sourceTree"), str)
        or re.fullmatch(r"[0-9a-f]{40}", seed["sourceTree"]) is None
        or (
            expected_source_tree is not None
            and seed["sourceTree"] != expected_source_tree
        )
        or not isinstance(seed.get("stateEpoch"), str)
        or DIGEST.fullmatch(seed["stateEpoch"]) is None
        or not isinstance(seed.get("manifestDigest"), str)
        or DIGEST.fullmatch(seed["manifestDigest"]) is None
        or not isinstance(seed.get("runtimeKey"), str)
        or DIGEST.fullmatch(seed["runtimeKey"]) is None
        or not isinstance(seed.get("seedDigest"), str)
        or DIGEST.fullmatch(seed["seedDigest"]) is None
        or payload
        != (json.dumps(seed, sort_keys=True, separators=(",", ":")) + "\n").encode()
        or not isinstance(artifact_hashes, dict)
        or not artifact_hashes
        or any(
            not isinstance(name, str)
            or not name
            or not isinstance(digest, str)
            or DIGEST.fullmatch(digest) is None
            for name, digest in artifact_hashes.items()
        )
        or not isinstance(worker_hashes, dict)
        or not worker_hashes
        or any(
            not isinstance(name, str)
            or not name
            or not isinstance(digest, str)
            or DIGEST.fullmatch(digest) is None
            for name, digest in worker_hashes.items()
        )
        or not isinstance(evidence_digests, list)
        or not evidence_digests
        or evidence_digests != sorted(set(evidence_digests))
        or any(
            not isinstance(digest, str) or DIGEST.fullmatch(digest) is None
            for digest in evidence_digests
        )
    ):
        raise AuthorityRefusal("PUBLISH_AUTHORITY")
    runtime_key = seed["runtimeKey"]
    verifier = _bootstrap_verifier()
    manifests = []
    try:
        for state_name in ("parallel-state",):
            capsule = (
                Path(root_value)
                / epoch
                / state_name
                / "v2"
                / "runtimes"
                / runtime_key.removeprefix("sha256:")
            )
            manifests.append(verifier.verify_capsule(capsule, runtime_key))
    except BaseException as error:
        raise AuthorityRefusal("CAPSULE_AUTHORITY") from error
    for manifest in manifests:
        manifest_artifacts = {
            name: row.get("sha256")
            for name, row in sorted(manifest.get("artifacts", {}).items())
            if isinstance(row, dict)
        }
        manifest_workers = {
            name: row.get("treeHash")
            for name, row in sorted(manifest.get("workers", {}).items())
            if isinstance(row, dict)
        }
        if (
            manifest.get("mode") != "RELEASE"
            or manifest.get("manifestDigest") != seed["manifestDigest"]
            or manifest_artifacts != artifact_hashes
            or manifest_workers != worker_hashes
        ):
            raise AuthorityRefusal("CAPSULE_AUTHORITY")
    root_path_identity = os.stat(root_value, follow_symlinks=False)
    if (root_identity.st_dev, root_identity.st_ino) != (
        root_path_identity.st_dev,
        root_path_identity.st_ino,
    ):
        raise AuthorityRefusal("ROOT_AUTHORITY")
    return {
        "epoch": authority_epoch,
        "runtimeKey": runtime_key,
        "seedDigest": seed["seedDigest"],
    }


def _locator_matches_seed(
    locator: dict[str, object], seed_identity: dict[str, str]
) -> bool:
    return all(locator.get(field) == seed_identity[field] for field in seed_identity)


def _validate_embedded_rollback(
    root_fd: int,
    root_value: str,
    locator: dict[str, object],
) -> None:
    rollback = locator["rollback"]
    if rollback is None:
        return
    assert isinstance(rollback, dict)
    epoch = rollback["epoch"]
    assert isinstance(epoch, str)
    try:
        _scan_candidate(root_fd, epoch, root_value)
        if not _locator_matches_seed(
            rollback, _seed_locator(root_fd, root_value, epoch)
        ):
            raise AuthorityRefusal("ROLLBACK_AUTHORITY")
        _validate_publication(root_fd, rollback, exact=False)
    except AuthorityRefusal:
        raise
    except (OSError, UnsafeEpoch, ExternalGitdir) as error:
        raise AuthorityRefusal("ROLLBACK_AUTHORITY") from error


def _current_locator(
    seed_identity: dict[str, str],
    prior: dict[str, object] | None,
) -> dict[str, object]:
    if prior is None:
        generation = 1
        rollback = None
    elif prior["epoch"] == seed_identity["epoch"]:
        generation = prior["generation"]
        rollback = prior["rollback"]
    else:
        generation = int(prior["generation"]) + 1
        rollback = {
            "epoch": prior["epoch"],
            "generation": prior["generation"],
            "publicationDigest": prior["publicationDigest"],
            "runtimeKey": prior["runtimeKey"],
            "seedDigest": prior["seedDigest"],
        }
    locator = {
        **seed_identity,
        "generation": generation,
        "rollback": rollback,
        "schema": "codeclew-trusted-seed-locator/2.0",
    }
    publication_unsigned = dict(locator)
    publication_unsigned["schema"] = "codeclew-trusted-seed-publication/1.0"
    locator["publicationDigest"] = "sha256:" + hashlib.sha256(
        json.dumps(
            publication_unsigned, sort_keys=True, separators=(",", ":")
        ).encode()
    ).hexdigest()
    return locator


def publish(
    root_value: str,
    epoch: str,
    candidate: str | None = None,
    expected_source_tree: str | None = None,
) -> dict[str, int | str]:
    if RELEASE_EPOCH.fullmatch(epoch) is None:
        raise AuthorityRefusal("PUBLISH_AUTHORITY")
    if candidate is not None and re.fullmatch(r"\.candidate\.[A-Za-z0-9]{6,64}", candidate) is None:
        raise AuthorityRefusal("PUBLISH_AUTHORITY")
    normalized_root, root_fd = _open_private_root(root_value)
    lifecycle: BinaryIO | None = None
    try:
        lifecycle = _lifecycle_lock(root_fd)
        prior_current = _read_current(root_fd)
        if prior_current is None:
            names = _bounded_names(root_fd, MAX_ROOT_ENTRIES)
            releases = [name for name in names if _is_epoch(name)]
            tombstones = [name for name in names if TOMBSTONE.fullmatch(name)]
            if releases or tombstones:
                if candidate is not None or releases != [epoch] or tombstones:
                    raise AuthorityRefusal("HISTORY_AUTHORITY")
        if prior_current is not None:
            prior_epoch = prior_current["epoch"]
            assert isinstance(prior_epoch, str)
            _scan_candidate(root_fd, prior_epoch, normalized_root)
            if not _locator_matches_seed(
                prior_current,
                _seed_locator(root_fd, normalized_root, prior_epoch),
            ):
                raise AuthorityRefusal("CURRENT_AUTHORITY")
            _validate_publication(root_fd, prior_current, exact=True)
            _validate_embedded_rollback(root_fd, normalized_root, prior_current)
        if candidate is not None:
            if _exists_at(root_fd, epoch):
                raise AuthorityRefusal("PUBLISH_CONFLICT")
            snapshot = _scan_candidate(root_fd, candidate, normalized_root)
            locator = _current_locator(
                _seed_locator(
                    root_fd,
                    normalized_root,
                    candidate,
                    expected_source_tree,
                    logical_epoch=epoch,
                ),
                prior_current,
            )
            _create_or_validate_publication(
                root_fd, locator, container_name=candidate
            )
            os.rename(candidate, epoch, src_dir_fd=root_fd, dst_dir_fd=root_fd)
            _fsync_directory(root_fd)
            confirmed = _scan_candidate(root_fd, epoch, normalized_root)
            if confirmed.identity != snapshot.identity:
                raise AuthorityRefusal("PUBLISH_AUTHORITY")
            _validate_publication(root_fd, locator, exact=True)
        else:
            _scan_candidate(root_fd, epoch, normalized_root)
            locator = _current_locator(
                _seed_locator(
                    root_fd, normalized_root, epoch, expected_source_tree
                ),
                prior_current,
            )
            _validate_publication(root_fd, locator, exact=True)
        _atomic_json_at(root_fd, "current.json", locator)
        gc_report = _collect_locked(normalized_root, root_fd, locator, [])
        if gc_report["status"] != "PASS":
            raise AuthorityRefusal("POST_PUBLISH_GC_AUTHORITY")
        return {
            "runtimeKey": locator["runtimeKey"],
            "schema": "codeclew-trusted-seed-qualification/1.0",
            "seedDigest": locator["seedDigest"],
            "status": "PASS",
        }
    except AuthorityRefusal:
        raise
    except (OSError, UnsafeEpoch, ExternalGitdir) as error:
        # A completed candidate may remain visible when publication fails, but
        # current.json is never changed to an unvalidated/missing epoch.
        raise AuthorityRefusal("PUBLISH_AUTHORITY") from error
    finally:
        if lifecycle is not None:
            lifecycle.close()
        os.close(root_fd)


def validate_epoch(
    root_value: str, epoch: str, expected_source_tree: str | None = None
) -> dict[str, str]:
    if RELEASE_EPOCH.fullmatch(epoch) is None:
        raise AuthorityRefusal("EPOCH_AUTHORITY")
    normalized_root, root_fd = _open_private_root(root_value)
    lifecycle: BinaryIO | None = None
    try:
        lifecycle = _lifecycle_lock(root_fd, shared=True)
        if not _exists_at(root_fd, epoch):
            raise AuthorityRefusal("EPOCH_MISSING")
        current = _read_current(root_fd)
        if current is None or current["epoch"] != epoch:
            raise AuthorityRefusal("CURRENT_AUTHORITY")
        _scan_candidate(root_fd, epoch, normalized_root)
        locator = _seed_locator(
            root_fd, normalized_root, epoch, expected_source_tree
        )
        if not _locator_matches_seed(current, locator):
            raise AuthorityRefusal("CURRENT_AUTHORITY")
        _validate_publication(root_fd, current, exact=True)
        _validate_embedded_rollback(root_fd, normalized_root, current)
        return {
            "runtimeKey": locator["runtimeKey"],
            "schema": "codeclew-trusted-seed-qualification/1.0",
            "seedDigest": locator["seedDigest"],
            "status": "PASS",
        }
    finally:
        if lifecycle is not None:
            lifecycle.close()
        os.close(root_fd)


def authority_digest(
    root_value: str, expected_revision: str, expected_tree: str
) -> str:
    if (
        re.fullmatch(r"[0-9a-f]{40}", expected_revision) is None
        or re.fullmatch(r"[0-9a-f]{40}", expected_tree) is None
    ):
        raise AuthorityRefusal("SOURCE_AUTHORITY")
    normalized_root, root_fd = _open_private_root(root_value)
    lifecycle: BinaryIO | None = None
    try:
        lifecycle = _lifecycle_lock(root_fd, shared=True)
        root_identity = os.fstat(root_fd)
        current = _read_current(root_fd)
        if current is None:
            raise AuthorityRefusal("CURRENT_AUTHORITY")
        epoch = current["epoch"]
        if epoch != "release-N-" + expected_revision:
            raise AuthorityRefusal("SOURCE_AUTHORITY")
        _scan_candidate(root_fd, epoch, normalized_root)
        if not _locator_matches_seed(
            current,
            _seed_locator(root_fd, normalized_root, epoch, expected_tree),
        ):
            raise AuthorityRefusal("CURRENT_AUTHORITY")
        _validate_publication(root_fd, current, exact=True)
        _validate_embedded_rollback(root_fd, normalized_root, current)
        path_metadata = os.stat(normalized_root, follow_symlinks=False)
        if (path_metadata.st_dev, path_metadata.st_ino) != (
            root_identity.st_dev,
            root_identity.st_ino,
        ):
            raise AuthorityRefusal("ROOT_AUTHORITY")
        return "sha256:" + hashlib.sha256(
            json.dumps(
                {
                    "locator": current,
                    "sourceRevision": expected_revision,
                    "sourceTree": expected_tree,
                },
                sort_keys=True,
                separators=(",", ":"),
            ).encode()
        ).hexdigest()
    except AuthorityRefusal:
        raise
    except (OSError, UnsafeEpoch, ExternalGitdir, json.JSONDecodeError) as error:
        raise AuthorityRefusal("CURRENT_AUTHORITY") from error
    finally:
        if lifecycle is not None:
            lifecycle.close()
        os.close(root_fd)


def run_current_state(
    root_value: str,
    state_name: str,
    expected_revision: str,
    expected_tree: str,
    command: list[str],
) -> int:
    if (
        state_name != "parallel-state"
        or re.fullmatch(r"[0-9a-f]{40}", expected_revision) is None
        or re.fullmatch(r"[0-9a-f]{40}", expected_tree) is None
        or not command
        or any(not value or "\x00" in value for value in command)
    ):
        raise AuthorityRefusal("COMMAND_AUTHORITY")
    normalized_root, root_fd = _open_private_root(root_value)
    lifecycle: BinaryIO | None = None
    process: subprocess.Popen[bytes] | None = None
    try:
        # Direct-state qualification is the one contour that intentionally
        # exercises the seed component CAS in place. Hold lifecycle SH before
        # discovery and until every descendant has exited, so publisher/GC
        # cannot withdraw the epoch during pre-lock bootstrap work.
        lifecycle = _lifecycle_lock(root_fd, shared=True)
        current = _read_current(root_fd)
        epoch = "release-N-" + expected_revision
        if current is None or current["epoch"] != epoch:
            raise AuthorityRefusal("CURRENT_AUTHORITY")
        _scan_candidate(root_fd, epoch, normalized_root)
        if not _locator_matches_seed(
            current,
            _seed_locator(root_fd, normalized_root, epoch, expected_tree),
        ):
            raise AuthorityRefusal("CURRENT_AUTHORITY")
        _validate_publication(root_fd, current, exact=True)
        _validate_embedded_rollback(root_fd, normalized_root, current)
        epoch_fd, epoch_metadata = _open_directory_at(root_fd, epoch)
        try:
            state_fd, state_metadata = _open_directory_at(epoch_fd, state_name)
            try:
                if (
                    state_metadata.st_uid != os.geteuid()
                    or state_metadata.st_dev != epoch_metadata.st_dev
                    or stat.S_IMODE(state_metadata.st_mode) != 0o700
                ):
                    raise AuthorityRefusal("CURRENT_AUTHORITY")
            finally:
                os.close(state_fd)
        finally:
            os.close(epoch_fd)
        environment = dict(os.environ)
        environment.pop("CODECLEW_RUNTIME_SEED", None)
        environment["CODECLEW_HOME"] = os.path.join(
            normalized_root, epoch, state_name
        )
        with _termination_guard():
            process_secured = False
            try:
                watched = {signal.SIGINT, signal.SIGTERM, signal.SIGHUP}
                previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, watched)
                try:
                    process = subprocess.Popen(
                        command,
                        env=environment,
                        stdin=subprocess.DEVNULL,
                        start_new_session=True,
                        pass_fds=(lifecycle.fileno(),),
                    )
                finally:
                    signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
                result = process.wait()
                descendants_remained = _process_group_exists(process.pid)
                cleanup_error: BaseException | None = None
                try:
                    _terminate_process_group(process.pid)
                    process_secured = True
                except BaseException as error:
                    _pin_process_group_until_gone(lifecycle, process.pid)
                    process_secured = True
                    cleanup_error = error
                if cleanup_error is not None:
                    if result != 0:
                        raise ProcessGroupReapFailure(result) from cleanup_error
                    raise cleanup_error
                if result != 0:
                    return result
                return DESCENDANT_LEAK_EXIT if descendants_remained else 0
            except BaseException as primary:
                if process is not None and not process_secured:
                    try:
                        _terminate_process_group(process.pid)
                    except BaseException as cleanup_error:
                        _pin_process_group_until_gone(lifecycle, process.pid)
                        process_secured = True
                        raise primary from cleanup_error
                raise
    except AuthorityRefusal:
        raise
    except (OSError, UnsafeEpoch, ExternalGitdir) as error:
        raise AuthorityRefusal("COMMAND_AUTHORITY") from error
    finally:
        if lifecycle is not None:
            lifecycle.close()
        os.close(root_fd)


def _delete_with_locks(root_fd: int, root_value: str, name: str) -> tuple[int, int]:
    held: dict[tuple[str, ...], LockedFile] = {}
    try:
        first = _scan_candidate(root_fd, name, root_value)
        held = _acquire_locks(root_fd, name, first)
        second = _scan_candidate(root_fd, name, root_value)
        held = _acquire_locks(root_fd, name, second, held)
        if first.identity != second.identity or not _locks_stable(held, second):
            raise UnsafeEpoch
        _delete_tombstone(root_fd, name, second.identity)
        return second.apparent_bytes, second.entries
    finally:
        _close_locks(held)


def _collect_locked(
    normalized_root: str,
    root_fd: int,
    current_locator: dict[str, object],
    protected_values: list[str],
) -> dict[str, int | str]:
    report = _base_report()
    current = current_locator["epoch"]
    assert isinstance(current, str)
    protected = set(protected_values)
    protected.add(current)
    rollback = current_locator["rollback"]
    if rollback is not None:
        assert isinstance(rollback, dict)
        rollback_epoch = rollback["epoch"]
        assert isinstance(rollback_epoch, str)
        protected.add(rollback_epoch)
    try:
        names = _bounded_names(root_fd, MAX_ROOT_ENTRIES)
    except UnsafeEpoch as error:
        raise AuthorityRefusal("ROOT_BOUNDS") from error

    # Recover all protected authority before destructive work.
    for tombstone in names:
        match = TOMBSTONE.fullmatch(tombstone)
        if match is None or match.group(1) not in protected:
            continue
        epoch = match.group(1)
        try:
            _scan_candidate(root_fd, tombstone, normalized_root)
            if _restore(root_fd, tombstone, epoch):
                report["recoveredTombstones"] += 1
            else:
                report["retainedTombstones"] += 1
        except (OSError, UnsafeEpoch, ExternalGitdir):
            report["retainedTombstones"] += 1
    try:
        _scan_candidate(root_fd, current, normalized_root)
        if not _locator_matches_seed(
            current_locator, _seed_locator(root_fd, normalized_root, current)
        ):
            raise AuthorityRefusal("CURRENT_AUTHORITY")
        _validate_publication(root_fd, current_locator, exact=True)
    except (OSError, UnsafeEpoch, ExternalGitdir, AuthorityRefusal):
        report["status"] = "REFUSED_CURRENT"
        return report
    try:
        _validate_embedded_rollback(root_fd, normalized_root, current_locator)
    except (OSError, UnsafeEpoch, ExternalGitdir, AuthorityRefusal):
        report["status"] = "REFUSED_ROLLBACK"
        return report

    for tombstone in names:
        match = TOMBSTONE.fullmatch(tombstone)
        if match is None or match.group(1) in protected:
            continue
        try:
            apparent_bytes, _entries = _delete_with_locks(root_fd, normalized_root, tombstone)
            report["deletedEpochs"] += 1
            report["deletedBytes"] += apparent_bytes
        except BusyEpoch:
            report["skippedBusy"] += 1
            report["retainedTombstones"] += 1
        except ExternalGitdir:
            report["skippedGitdir"] += 1
            report["retainedTombstones"] += 1
        except (OSError, UnsafeEpoch):
            report["skippedUnsafe"] += 1
            report["retainedTombstones"] += 1

    for epoch in names:
        if not _is_epoch(epoch):
            continue
        if epoch in protected:
            report["protectedEpochs"] += 1
            continue
        tombstone = ".gc-" + epoch
        if _exists_at(root_fd, tombstone):
            report["skippedUnsafe"] += 1
            continue
        held: dict[tuple[str, ...], LockedFile] = {}
        withdrawn = False
        deletion_started = False
        apparent_bytes = 0
        try:
            first = _scan_candidate(root_fd, epoch, normalized_root)
            apparent_bytes = first.apparent_bytes
            held = _acquire_locks(root_fd, epoch, first)
            if not _locks_stable(held, first):
                raise UnsafeEpoch
            os.rename(epoch, tombstone, src_dir_fd=root_fd, dst_dir_fd=root_fd)
            withdrawn = True
            _fsync_directory(root_fd)
            second = _scan_candidate(root_fd, tombstone, normalized_root)
            held = _acquire_locks(root_fd, tombstone, second, held)
            if first.identity != second.identity or not _locks_stable(held, second):
                raise UnsafeEpoch
            deletion_started = True
            _delete_tombstone(root_fd, tombstone, second.identity)
            withdrawn = False
            report["deletedEpochs"] += 1
            report["deletedBytes"] += apparent_bytes
        except BusyEpoch:
            report["skippedBusy"] += 1
        except ExternalGitdir:
            report["skippedGitdir"] += 1
        except (OSError, UnsafeEpoch):
            report["skippedUnsafe"] += 1
        finally:
            if withdrawn:
                if not deletion_started and _restore(root_fd, tombstone, epoch):
                    withdrawn = False
                else:
                    report["retainedTombstones"] += 1
            _close_locks(held)
    return report


def collect(root_value: str, protected_values: list[str]) -> dict[str, int | str]:
    for name in protected_values:
        if not _is_epoch(name):
            raise AuthorityRefusal("PROTECT_AUTHORITY")
    normalized_root, root_fd = _open_private_root(root_value)
    lifecycle: BinaryIO | None = None
    try:
        lifecycle = _lifecycle_lock(root_fd)
        current_locator = _read_current(root_fd)
        if current_locator is None:
            report = _base_report()
            report["status"] = "NO_CURRENT"
            return report
        return _collect_locked(
            normalized_root, root_fd, current_locator, protected_values
        )
    finally:
        if lifecycle is not None:
            lifecycle.close()
        os.close(root_fd)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True)
    parser.add_argument("--protect-epoch", action="append", default=[])
    parser.add_argument("--publish-epoch")
    parser.add_argument("--validate-epoch")
    parser.add_argument("--candidate")
    parser.add_argument("--expected-source-tree")
    parser.add_argument("--expected-source-revision")
    parser.add_argument(
        "--run-current-state", choices=("parallel-state",)
    )
    parser.add_argument("command", nargs=argparse.REMAINDER)
    arguments = parser.parse_args()
    try:
        operations = sum(
            value is not None
            for value in (
                arguments.publish_epoch,
                arguments.validate_epoch,
                arguments.run_current_state,
            )
        )
        if operations > 1:
            raise AuthorityRefusal("COMMAND_AUTHORITY")
        if arguments.run_current_state is not None:
            command = list(arguments.command)
            if command[:1] == ["--"]:
                command = command[1:]
            if (
                arguments.protect_epoch
                or arguments.candidate is not None
                or arguments.publish_epoch is not None
                or arguments.validate_epoch is not None
                or arguments.expected_source_revision is None
                or arguments.expected_source_tree is None
            ):
                raise AuthorityRefusal("COMMAND_AUTHORITY")
            return run_current_state(
                arguments.root,
                arguments.run_current_state,
                arguments.expected_source_revision,
                arguments.expected_source_tree,
                command,
            )
        if arguments.command or arguments.expected_source_revision is not None:
            raise AuthorityRefusal("COMMAND_AUTHORITY")
        if arguments.validate_epoch is not None:
            if arguments.protect_epoch or arguments.candidate is not None:
                raise AuthorityRefusal("COMMAND_AUTHORITY")
            report = validate_epoch(
                arguments.root,
                arguments.validate_epoch,
                arguments.expected_source_tree,
            )
        elif arguments.publish_epoch is not None:
            if arguments.protect_epoch:
                raise AuthorityRefusal("PUBLISH_AUTHORITY")
            report = publish(
                arguments.root,
                arguments.publish_epoch,
                arguments.candidate,
                arguments.expected_source_tree,
            )
        else:
            if arguments.candidate is not None or arguments.expected_source_tree is not None:
                raise AuthorityRefusal("PUBLISH_AUTHORITY")
            report = collect(arguments.root, arguments.protect_epoch)
    except SupervisorInterrupted as error:
        return 128 + error.signum
    except AuthorityRefusal as error:
        refusal: dict[str, int | str] = {
            "reason": error.reason,
            "schema": "codeclew-trusted-seed-gc/1.0",
            "status": "REFUSED",
        }
        if isinstance(error, ProcessGroupReapFailure):
            refusal["leaderExitCode"] = error.leader_exit_code
        print(
            json.dumps(
                refusal,
                sort_keys=True,
                separators=(",", ":"),
            )
        )
        return 3 if error.reason == "EPOCH_MISSING" else 2
    except OSError:
        print(
            json.dumps(
                {
                    "reason": "IO_AUTHORITY",
                    "schema": "codeclew-trusted-seed-gc/1.0",
                    "status": "REFUSED",
                },
                sort_keys=True,
                separators=(",", ":"),
            )
        )
        return 2
    print(json.dumps(report, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
