#!/usr/bin/env python3
"""Crash-safe content-addressed dependency-cache seeds for cold evidence."""

from __future__ import annotations

import argparse
import contextlib
import ctypes
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import secrets
import stat
import subprocess
import sys
from typing import BinaryIO


SCHEMA = "codeclew-cold-cache-seed/2.0"
LOCATOR_SCHEMA = "codeclew-cold-cache-locator/1.0"
PENDING_SCHEMA = "codeclew-cold-cache-publication/1.0"
DIGEST = "sha256:"
DIGEST_PATTERN = re.compile(r"sha256:[0-9a-f]{64}")
MAX_ENTRIES = 750_000
MAX_BYTES = 48 * 1024**3
MAX_MANIFEST_BYTES = 1024 * 1024
SEED_MANIFEST = "SEED.json"
VOLATILE_NAMES = {".package-cache", "gc.properties"}
VOLATILE_SUFFIXES = {".lock", ".lck"}
VOLATILE_DIRECTORIES = {".gradle/.tmp", ".gradle/daemon", ".gradle/workers"}
STORE_DIRECTORIES = ("locks", "locators", "objects", "pending")


class CacheAuthorityError(RuntimeError):
    pass


def _absolute(path: str | Path, label: str) -> Path:
    value = Path(path)
    if (
        not value.is_absolute()
        or value == Path(value.anchor)
        or ".." in value.parts
        or Path(os.path.normpath(value)) != value
    ):
        raise CacheAuthorityError(f"{label} must be normalized and absolute")
    return value


def _directory_flags() -> int:
    return os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0)


def _safe_ancestor(metadata: os.stat_result) -> bool:
    mode = stat.S_IMODE(metadata.st_mode)
    return (
        stat.S_ISDIR(metadata.st_mode)
        and metadata.st_uid in {0, os.geteuid()}
        and (mode & 0o022 == 0 or (metadata.st_uid == 0 and mode & stat.S_ISVTX != 0))
    )


def _open_private_directory(
    path: Path, *, create: bool, leaf_modes: set[int] | None = None
) -> int:
    path = _absolute(path, "private directory")
    descriptor = os.open("/", _directory_flags())
    try:
        components = [part for part in path.parts if part != path.anchor]
        allowed_leaf_modes = leaf_modes or {0o700}
        for index, component in enumerate(components):
            leaf = index == len(components) - 1
            try:
                child = os.open(component, _directory_flags(), dir_fd=descriptor)
            except FileNotFoundError:
                if not create:
                    raise
                created = False
                try:
                    os.mkdir(component, 0o700, dir_fd=descriptor)
                    created = True
                except FileExistsError:
                    pass
                if created:
                    os.fsync(descriptor)
                child = os.open(component, _directory_flags(), dir_fd=descriptor)
            metadata = os.fstat(child)
            before = os.stat(component, dir_fd=descriptor, follow_symlinks=False)
            if (
                stat.S_ISLNK(before.st_mode)
                or (before.st_dev, before.st_ino) != (metadata.st_dev, metadata.st_ino)
                or (
                    leaf
                    and (
                        not stat.S_ISDIR(metadata.st_mode)
                        or metadata.st_uid != os.geteuid()
                        or stat.S_IMODE(metadata.st_mode) not in allowed_leaf_modes
                    )
                )
                or (not leaf and not _safe_ancestor(metadata))
            ):
                os.close(child)
                raise CacheAuthorityError("private directory authority is unsafe")
            os.close(descriptor)
            descriptor = child
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def _ensure_directory(parent_fd: int, name: str, *, create: bool) -> int:
    try:
        descriptor = os.open(name, _directory_flags(), dir_fd=parent_fd)
    except FileNotFoundError:
        if not create:
            raise
        created = False
        try:
            os.mkdir(name, 0o700, dir_fd=parent_fd)
            created = True
        except FileExistsError:
            # A concurrent publisher may have created the same store
            # component after our failed open. The no-follow open and
            # identity/mode checks below still establish its authority.
            pass
        if created:
            os.fsync(parent_fd)
        descriptor = os.open(name, _directory_flags(), dir_fd=parent_fd)
    metadata = os.fstat(descriptor)
    before = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    if (
        stat.S_ISLNK(before.st_mode)
        or not stat.S_ISDIR(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or (before.st_dev, before.st_ino) != (metadata.st_dev, metadata.st_ino)
    ):
        os.close(descriptor)
        raise CacheAuthorityError("cache store directory authority is unsafe")
    return descriptor


def _open_store(store: Path, *, create: bool = True) -> tuple[int, dict[str, int]]:
    store_fd = _open_private_directory(store, create=create)
    children: dict[str, int] = {}
    try:
        for name in STORE_DIRECTORIES:
            children[name] = _ensure_directory(store_fd, name, create=create)
        return store_fd, children
    except BaseException:
        for descriptor in children.values():
            os.close(descriptor)
        os.close(store_fd)
        raise


def _regular_identity(metadata: os.stat_result) -> tuple[int, ...]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_mode,
        metadata.st_nlink,
        metadata.st_uid,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def _directory_identity(metadata: os.stat_result) -> tuple[int, ...]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_mode,
        metadata.st_uid,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def _read_file_at(parent_fd: int, name: str, limit: int) -> tuple[bytes, os.stat_result]:
    before = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    if (
        stat.S_ISLNK(before.st_mode)
        or not stat.S_ISREG(before.st_mode)
        or before.st_nlink != 1
    ):
        raise CacheAuthorityError("cache file authority is unsafe")
    descriptor = os.open(
        name, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0), dir_fd=parent_fd
    )
    try:
        opened = os.fstat(descriptor)
        if (
            opened.st_uid != os.geteuid()
            or opened.st_nlink != 1
            or _regular_identity(opened) != _regular_identity(before)
            or opened.st_size > limit
        ):
            raise CacheAuthorityError("cache file authority changed while opening")
        payload = b""
        while len(payload) <= limit:
            block = os.read(descriptor, min(1024 * 1024, limit + 1 - len(payload)))
            if not block:
                break
            payload += block
        after_fd = os.fstat(descriptor)
        after_path = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
        if (
            len(payload) > limit
            or _regular_identity(opened) != _regular_identity(after_fd)
            or _regular_identity(opened) != _regular_identity(after_path)
        ):
            raise CacheAuthorityError("cache file changed while reading")
        return payload, opened
    finally:
        os.close(descriptor)


def _hash_file_at(
    parent_fd: int, name: str, remaining_bytes: int
) -> tuple[str, os.stat_result]:
    before = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    if (
        stat.S_ISLNK(before.st_mode)
        or not stat.S_ISREG(before.st_mode)
        or before.st_nlink != 1
    ):
        raise CacheAuthorityError("cache file authority is unsafe")
    descriptor = os.open(
        name, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0), dir_fd=parent_fd
    )
    try:
        opened = os.fstat(descriptor)
        if (
            opened.st_uid != os.geteuid()
            or opened.st_nlink != 1
            or _regular_identity(opened) != _regular_identity(before)
            or opened.st_size > remaining_bytes
        ):
            raise CacheAuthorityError("cache file exceeds or changed within its authority")
        hasher = hashlib.sha256()
        observed_bytes = 0
        while block := os.read(descriptor, 1024 * 1024):
            observed_bytes += len(block)
            if observed_bytes > remaining_bytes:
                raise CacheAuthorityError("cache seed exceeds its bounded closure")
            hasher.update(block)
        after_fd = os.fstat(descriptor)
        after_path = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
        if (
            observed_bytes != opened.st_size
            or _regular_identity(opened) != _regular_identity(after_fd)
            or _regular_identity(opened) != _regular_identity(after_path)
        ):
            raise CacheAuthorityError("cache file changed while hashing")
        return DIGEST + hasher.hexdigest(), opened
    finally:
        os.close(descriptor)


def _scan_tree_fd(
    root_fd: int,
    *,
    exclude_root: set[str] | None = None,
    root_modes: set[int] | None = None,
    directory_modes: set[int] | None = None,
    file_modes: set[int] | None = None,
) -> tuple[list[dict[str, object]], int]:
    rows: list[dict[str, object]] = []
    apparent_bytes = 0
    entries = 0
    excluded = exclude_root or set()
    root_before = os.fstat(root_fd)
    if (
        not stat.S_ISDIR(root_before.st_mode)
        or root_before.st_uid != os.geteuid()
        or (
            root_modes is not None
            and stat.S_IMODE(root_before.st_mode) not in root_modes
        )
    ):
        raise CacheAuthorityError("cache root authority is unsafe")

    def visit(directory_fd: int, prefix: str) -> None:
        nonlocal apparent_bytes, entries
        directory_before = os.fstat(directory_fd)
        with os.scandir(directory_fd) as iterator:
            names = sorted(entry.name for entry in iterator)
        for name in names:
            if not prefix and name in excluded:
                continue
            if name in {"", ".", ".."} or "/" in name or "\x00" in name:
                raise CacheAuthorityError("cache entry name is unsafe")
            before = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
            entries += 1
            if entries > MAX_ENTRIES:
                raise CacheAuthorityError("cache seed exceeds its bounded closure")
            relative = f"{prefix}/{name}" if prefix else name
            if stat.S_ISLNK(before.st_mode):
                raise CacheAuthorityError("cache seed contains a symlink")
            if before.st_uid != os.geteuid():
                raise CacheAuthorityError("cache entry has a different owner")
            if stat.S_ISDIR(before.st_mode):
                if (
                    directory_modes is not None
                    and stat.S_IMODE(before.st_mode) not in directory_modes
                ):
                    raise CacheAuthorityError("cache directory mode is unsafe")
                rows.append({"path": relative, "type": "directory"})
                child_fd = os.open(name, _directory_flags(), dir_fd=directory_fd)
                try:
                    opened = os.fstat(child_fd)
                    if _directory_identity(opened) != _directory_identity(before):
                        raise CacheAuthorityError("cache directory changed while opening")
                    visit(child_fd, relative)
                    after_fd = os.fstat(child_fd)
                    after_path = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
                    if (
                        _directory_identity(opened) != _directory_identity(after_fd)
                        or _directory_identity(opened) != _directory_identity(after_path)
                    ):
                        raise CacheAuthorityError("cache directory changed while scanning")
                finally:
                    os.close(child_fd)
            elif stat.S_ISREG(before.st_mode):
                if before.st_nlink != 1:
                    raise CacheAuthorityError("cache file has multiple links")
                if (
                    file_modes is not None
                    and stat.S_IMODE(before.st_mode) not in file_modes
                ):
                    raise CacheAuthorityError("cache file mode is unsafe")
                file_digest, opened = _hash_file_at(
                    directory_fd, name, MAX_BYTES - apparent_bytes
                )
                apparent_bytes += opened.st_size
                if apparent_bytes > MAX_BYTES:
                    raise CacheAuthorityError("cache seed exceeds its bounded closure")
                rows.append(
                    {
                        "executable": bool(opened.st_mode & 0o111),
                        "path": relative,
                        "sha256": file_digest,
                        "size": opened.st_size,
                        "type": "file",
                    }
                )
            else:
                raise CacheAuthorityError("cache seed contains an unsupported entry")
        if _directory_identity(directory_before) != _directory_identity(os.fstat(directory_fd)):
            raise CacheAuthorityError("cache directory changed while scanning")

    visit(root_fd, "")
    if _directory_identity(root_before) != _directory_identity(os.fstat(root_fd)):
        raise CacheAuthorityError("cache root changed while scanning")
    return rows, apparent_bytes


def cache_rows(root: Path) -> tuple[list[dict[str, object]], int]:
    descriptor = _open_private_directory(_absolute(root, "cache root"), create=False)
    try:
        return _scan_tree_fd(descriptor, exclude_root={SEED_MANIFEST})
    finally:
        os.close(descriptor)


def content_digest(rows: list[dict[str, object]]) -> str:
    encoded = json.dumps({"files": rows}, sort_keys=True, separators=(",", ":")).encode()
    return DIGEST + hashlib.sha256(b"codeclew-cold-cache/v2\0" + encoded).hexdigest()


def _manifest(rows: list[dict[str, object]], size: int) -> dict[str, object]:
    return {
        "apparentBytes": size,
        "contentDigest": content_digest(rows),
        "entries": len(rows),
        "schema": SCHEMA,
    }


def _canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode() + b"\n"


def _write_all(descriptor: int, payload: bytes) -> None:
    offset = 0
    while offset < len(payload):
        written = os.write(descriptor, payload[offset:])
        if written <= 0:
            raise OSError("short write")
        offset += written


def _write_immutable_json_at(parent_fd: int, name: str, value: object) -> bool:
    temporary = f".{name}.{secrets.token_hex(16)}.tmp"
    descriptor = os.open(
        temporary,
        os.O_CREAT | os.O_EXCL | os.O_WRONLY | getattr(os, "O_NOFOLLOW", 0),
        0o600,
        dir_fd=parent_fd,
    )
    try:
        _write_all(descriptor, _canonical(value))
        os.fsync(descriptor)
        os.fchmod(descriptor, 0o400)
        os.fsync(descriptor)
        try:
            os.link(
                temporary,
                name,
                src_dir_fd=parent_fd,
                dst_dir_fd=parent_fd,
                follow_symlinks=False,
            )
            created = True
            os.fsync(parent_fd)
        except FileExistsError:
            created = False
    finally:
        os.close(descriptor)
        try:
            os.unlink(temporary, dir_fd=parent_fd)
            os.fsync(parent_fd)
        except FileNotFoundError:
            pass
    return created


def _read_canonical_json_at(
    parent_fd: int, name: str, *, mode: int = 0o400
) -> dict[str, object]:
    payload, metadata = _read_file_at(parent_fd, name, MAX_MANIFEST_BYTES)
    if stat.S_IMODE(metadata.st_mode) != mode:
        raise CacheAuthorityError("cache authority file mode is unsafe")
    try:
        value = json.loads(payload)
    except (ValueError, TypeError) as error:
        raise CacheAuthorityError("cache authority file is invalid") from error
    if not isinstance(value, dict) or payload != _canonical(value):
        raise CacheAuthorityError("cache authority file is not canonical")
    return value


def _locator(authority_key: str, digest_value: str) -> dict[str, object]:
    return {
        "authorityKey": authority_key,
        "contentDigest": digest_value,
        "schema": LOCATOR_SCHEMA,
    }


def _pending(authority_key: str, digest_value: str) -> dict[str, object]:
    return {
        "authorityKey": authority_key,
        "contentDigest": digest_value,
        "schema": PENDING_SCHEMA,
    }


def _validate_binding(value: dict[str, object], expected: dict[str, object]) -> None:
    if value != expected:
        raise CacheAuthorityError("cache runtime locator authority conflicts")


class AuthorityLock:
    """A flock whose two names remain bound to its original inode authority."""

    def __init__(
        self,
        stream: BinaryIO,
        parent_fd: int,
        name: str,
        companion: str,
        identity: tuple[int, ...],
    ) -> None:
        self._stream = stream
        self._parent_fd = os.dup(parent_fd)
        self._name = name
        self._companion = companion
        self._identity = identity
        self._closed = False

    def fileno(self) -> int:
        return self._stream.fileno()

    def revalidate(self) -> None:
        if self._closed:
            raise CacheAuthorityError("cache authority lock is closed")
        locked = os.fstat(self._stream.fileno())
        descriptors: list[int] = []
        try:
            for name in (self._name, self._companion):
                descriptor = os.open(
                    name,
                    os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
                    dir_fd=self._parent_fd,
                )
                descriptors.append(descriptor)
                opened = os.fstat(descriptor)
                by_name = os.stat(
                    name, dir_fd=self._parent_fd, follow_symlinks=False
                )
                if (
                    stat.S_ISLNK(by_name.st_mode)
                    or _regular_identity(opened) != _regular_identity(by_name)
                    or _regular_identity(opened) != self._identity
                ):
                    raise CacheAuthorityError("cache authority lock pair was replaced")
            if (
                not stat.S_ISREG(locked.st_mode)
                or locked.st_uid != os.geteuid()
                or stat.S_IMODE(locked.st_mode) != 0o600
                or locked.st_size != 0
                or locked.st_nlink != 2
                or _regular_identity(locked) != self._identity
            ):
                raise CacheAuthorityError("cache authority lock pair is unsafe")
        finally:
            for descriptor in descriptors:
                os.close(descriptor)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            self._stream.close()
        finally:
            os.close(self._parent_fd)


def _lock_file(
    locks_fd: int, name: str, *, shared: bool, create: bool
) -> AuthorityLock:
    companion = name + ".companion"
    candidate = "." + name + ".candidate"
    flags = os.O_RDWR | getattr(os, "O_NOFOLLOW", 0)
    descriptor = -1
    fcntl.flock(locks_fd, fcntl.LOCK_EX)
    try:
        def observed(entry: str) -> os.stat_result | None:
            try:
                return os.stat(entry, dir_fd=locks_fd, follow_symlinks=False)
            except FileNotFoundError:
                return None

        primary = observed(name)
        peer = observed(companion)
        staged = observed(candidate)
        if staged is not None:
            if not create:
                raise CacheAuthorityError("cache authority lock publication is incomplete")
            present = [value for value in (staged, primary, peer) if value is not None]
            expected_links = len(present)
            if (
                not stat.S_ISREG(staged.st_mode)
                or stat.S_ISLNK(staged.st_mode)
                or staged.st_uid != os.geteuid()
                or stat.S_IMODE(staged.st_mode) != 0o600
                or staged.st_size != 0
                or staged.st_nlink != expected_links
                or any(
                    (value.st_dev, value.st_ino) != (staged.st_dev, staged.st_ino)
                    for value in present
                )
                or (peer is not None and primary is None)
            ):
                raise CacheAuthorityError("cache authority lock candidate is unsafe")
            if primary is None:
                os.link(
                    candidate, name, src_dir_fd=locks_fd, dst_dir_fd=locks_fd,
                    follow_symlinks=False,
                )
                os.fsync(locks_fd)
                primary = observed(name)
            if peer is None:
                os.link(
                    candidate, companion, src_dir_fd=locks_fd, dst_dir_fd=locks_fd,
                    follow_symlinks=False,
                )
                os.fsync(locks_fd)
                peer = observed(companion)
            os.unlink(candidate, dir_fd=locks_fd)
            os.fsync(locks_fd)
            staged = None
        elif (primary is None) != (peer is None):
            raise CacheAuthorityError("cache authority lock pair is incomplete")
        elif primary is None:
            if not create:
                raise FileNotFoundError(name)
            descriptor = os.open(
                candidate,
                flags | os.O_CREAT | os.O_EXCL,
                0o600,
                dir_fd=locks_fd,
            )
            os.fchmod(descriptor, 0o600)
            os.fsync(descriptor)
            os.fsync(locks_fd)
            os.link(
                candidate, name, src_dir_fd=locks_fd, dst_dir_fd=locks_fd,
                follow_symlinks=False,
            )
            os.fsync(locks_fd)
            os.link(
                candidate, companion, src_dir_fd=locks_fd, dst_dir_fd=locks_fd,
                follow_symlinks=False,
            )
            os.fsync(locks_fd)
            os.unlink(candidate, dir_fd=locks_fd)
            os.fsync(locks_fd)
            os.close(descriptor)
            descriptor = -1
        descriptor = os.open(name, flags, dir_fd=locks_fd)
        metadata = os.fstat(descriptor)
        by_name = os.stat(name, dir_fd=locks_fd, follow_symlinks=False)
        by_companion = os.stat(
            companion, dir_fd=locks_fd, follow_symlinks=False
        )
        if (
            not stat.S_ISREG(metadata.st_mode)
            or stat.S_ISLNK(by_name.st_mode)
            or stat.S_ISLNK(by_companion.st_mode)
            or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_size != 0
            or metadata.st_nlink != 2
            or _regular_identity(metadata) != _regular_identity(by_name)
            or _regular_identity(metadata) != _regular_identity(by_companion)
        ):
            raise CacheAuthorityError("cache authority lock pair is unsafe")
        stream = os.fdopen(descriptor, "r+b")
        descriptor = -1
    except BaseException:
        if descriptor >= 0:
            os.close(descriptor)
            descriptor = -1
        raise
    finally:
        fcntl.flock(locks_fd, fcntl.LOCK_UN)
    authority: AuthorityLock | None = None
    try:
        fcntl.flock(stream.fileno(), fcntl.LOCK_SH if shared else fcntl.LOCK_EX)
        authority = AuthorityLock(
            stream,
            locks_fd,
            name,
            companion,
            _regular_identity(metadata),
        )
        authority.revalidate()
        return authority
    except BaseException:
        if authority is not None:
            authority.close()
        else:
            stream.close()
        raise


def _lifecycle_lock(locks_fd: int, *, shared: bool, create: bool) -> AuthorityLock:
    return _lock_file(
        locks_fd, "lifecycle.lock", shared=shared, create=create
    )


def _safe_unlink_tree(parent_fd: int, name: str, budget: list[int]) -> None:
    metadata = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    budget[0] += 1
    budget[1] += metadata.st_size
    if budget[0] > MAX_ENTRIES or budget[1] > MAX_BYTES:
        raise CacheAuthorityError("cache cleanup exceeds its bounded authority")
    if stat.S_ISLNK(metadata.st_mode) or stat.S_ISREG(metadata.st_mode):
        os.unlink(name, dir_fd=parent_fd)
        return
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid():
        raise CacheAuthorityError("cache cleanup entry is unsafe")
    child_fd = os.open(name, _directory_flags(), dir_fd=parent_fd)
    try:
        opened = os.fstat(child_fd)
        if (opened.st_dev, opened.st_ino) != (metadata.st_dev, metadata.st_ino):
            raise CacheAuthorityError("cache cleanup target changed")
        os.fchmod(child_fd, 0o700)
        with os.scandir(child_fd) as iterator:
            names = sorted(entry.name for entry in iterator)
        for child in names:
            _safe_unlink_tree(child_fd, child, budget)
    finally:
        os.close(child_fd)
    after = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    if (after.st_dev, after.st_ino) != (metadata.st_dev, metadata.st_ino):
        raise CacheAuthorityError("cache cleanup target changed")
    os.rmdir(name, dir_fd=parent_fd)


def _discard(root: Path) -> None:
    root = _absolute(root, "cache cleanup root")
    parent_fd = _open_private_directory(root.parent, create=False)
    try:
        try:
            _safe_unlink_tree(parent_fd, root.name, [0, 0])
            os.fsync(parent_fd)
        except FileNotFoundError:
            return
    finally:
        os.close(parent_fd)


def _candidate_prefix(authority_key: str) -> str:
    if DIGEST_PATTERN.fullmatch(authority_key) is None:
        raise CacheAuthorityError("cache authority key is invalid")
    return ".candidate-" + authority_key.removeprefix(DIGEST) + "-"


@contextlib.contextmanager
def seed_creation_lock(store: Path, authority_key: str):
    """Singleflight one authority across recover, prime, and publication."""
    store = _absolute(store, "cache store")
    _candidate_prefix(authority_key)
    store_fd, children = _open_store(store, create=True)
    creation: AuthorityLock | None = None
    try:
        creation = _lock_file(
            children["locks"],
            "creation-" + authority_key.removeprefix(DIGEST) + ".lock",
            shared=False,
            create=True,
        )
        yield
        creation.revalidate()
    finally:
        if creation is not None:
            creation.close()
        for descriptor in children.values():
            os.close(descriptor)
        os.close(store_fd)


def create_seed_candidate(store: Path, authority_key: str) -> Path:
    """Create an authority-named candidate while its creation lock is held."""
    store = _absolute(store, "cache store")
    name = _candidate_prefix(authority_key) + secrets.token_hex(20)
    store_fd, children = _open_store(store, create=True)
    try:
        os.mkdir(name, 0o700, dir_fd=store_fd)
        os.fsync(store_fd)
        return store / name
    finally:
        for descriptor in children.values():
            os.close(descriptor)
        os.close(store_fd)


def _authority_candidates(store_fd: int, authority_key: str) -> list[str]:
    prefix = _candidate_prefix(authority_key)
    pattern = re.compile(re.escape(prefix) + r"[0-9a-f]{40}")
    with os.scandir(store_fd) as iterator:
        names = sorted(entry.name for entry in iterator if entry.name.startswith(prefix))
    if any(pattern.fullmatch(name) is None for name in names):
        raise CacheAuthorityError("cache candidate identity is malformed")
    return names


def _discard_candidates(store_fd: int, names: list[str]) -> None:
    for name in names:
        try:
            _safe_unlink_tree(store_fd, name, [0, 0])
        except FileNotFoundError:
            pass
    if names:
        os.fsync(store_fd)


def _normalize_fd(directory_fd: int, prefix: str = "") -> None:
    with os.scandir(directory_fd) as iterator:
        names = sorted(entry.name for entry in iterator)
    for name in names:
        relative = f"{prefix}/{name}" if prefix else name
        metadata = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
        if stat.S_ISLNK(metadata.st_mode):
            raise CacheAuthorityError("cache candidate contains a symlink")
        if stat.S_ISDIR(metadata.st_mode):
            if relative in VOLATILE_DIRECTORIES:
                _safe_unlink_tree(directory_fd, name, [0, 0])
                continue
            child_fd = os.open(name, _directory_flags(), dir_fd=directory_fd)
            try:
                _normalize_fd(child_fd, relative)
                os.fchmod(child_fd, 0o700)
            finally:
                os.close(child_fd)
        elif stat.S_ISREG(metadata.st_mode):
            if name == SEED_MANIFEST:
                raise CacheAuthorityError("cache candidate already contains a manifest")
            if name in VOLATILE_NAMES or Path(name).suffix in VOLATILE_SUFFIXES:
                os.unlink(name, dir_fd=directory_fd)
        else:
            raise CacheAuthorityError("cache candidate contains an unsupported entry")


def normalize_candidate(candidate: Path) -> None:
    descriptor = _open_private_directory(
        _absolute(candidate, "cache candidate"), create=False
    )
    try:
        _normalize_fd(descriptor)
    finally:
        os.close(descriptor)


def _seal_children(directory_fd: int) -> None:
    with os.scandir(directory_fd) as iterator:
        names = sorted(entry.name for entry in iterator)
    for name in names:
        metadata = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
        if stat.S_ISLNK(metadata.st_mode):
            raise CacheAuthorityError("cache seed contains a symlink")
        if stat.S_ISDIR(metadata.st_mode):
            child_fd = os.open(name, _directory_flags(), dir_fd=directory_fd)
            try:
                _seal_children(child_fd)
                os.fchmod(child_fd, 0o500)
                os.fsync(child_fd)
            finally:
                os.close(child_fd)
        elif stat.S_ISREG(metadata.st_mode):
            if metadata.st_nlink != 1:
                raise CacheAuthorityError("cache seed file has multiple links")
            descriptor = os.open(
                name,
                os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
                dir_fd=directory_fd,
            )
            try:
                opened = os.fstat(descriptor)
                if (
                    opened.st_nlink != 1
                    or _regular_identity(opened) != _regular_identity(metadata)
                ):
                    raise CacheAuthorityError("cache seed file authority changed")
                os.fchmod(descriptor, 0o500 if opened.st_mode & 0o111 else 0o400)
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
        else:
            raise CacheAuthorityError("cache seed contains an unsupported entry")
    os.fsync(directory_fd)


def _object_fd(objects_fd: int, digest_value: str) -> int:
    if DIGEST_PATTERN.fullmatch(digest_value) is None:
        raise CacheAuthorityError("cache content digest is invalid")
    name = digest_value.removeprefix(DIGEST)
    descriptor = os.open(name, _directory_flags(), dir_fd=objects_fd)
    metadata = os.fstat(descriptor)
    before = os.stat(name, dir_fd=objects_fd, follow_symlinks=False)
    if (
        stat.S_ISLNK(before.st_mode)
        or metadata.st_uid != os.geteuid()
        or (before.st_dev, before.st_ino) != (metadata.st_dev, metadata.st_ino)
    ):
        os.close(descriptor)
        raise CacheAuthorityError("cache object authority is unsafe")
    return descriptor


def _validate_modes(directory_fd: int, *, root_modes: set[int]) -> None:
    if stat.S_IMODE(os.fstat(directory_fd).st_mode) not in root_modes:
        raise CacheAuthorityError("cache seed directory mode is unsafe")
    with os.scandir(directory_fd) as entries:
        names = sorted(entry.name for entry in entries)
    for name in names:
        metadata = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
        if stat.S_ISDIR(metadata.st_mode):
            child_fd = os.open(name, _directory_flags(), dir_fd=directory_fd)
            try:
                _validate_modes(child_fd, root_modes={0o500})
            finally:
                os.close(child_fd)
        elif stat.S_ISREG(metadata.st_mode):
            if metadata.st_nlink != 1:
                raise CacheAuthorityError("cache seed file has multiple links")
            expected = 0o500 if metadata.st_mode & 0o111 else 0o400
            if stat.S_IMODE(metadata.st_mode) != expected:
                raise CacheAuthorityError("cache seed file mode is unsafe")
        else:
            raise CacheAuthorityError("cache seed contains an unsupported entry")


def _validate_object_fd(
    object_fd: int,
    digest_value: str,
    *,
    recover_root: bool,
    seal_recovered_root: bool = True,
) -> dict[str, object]:
    root_mode = stat.S_IMODE(os.fstat(object_fd).st_mode)
    _validate_modes(
        object_fd, root_modes={0o500, 0o700} if recover_root else {0o500}
    )
    value = _read_canonical_json_at(object_fd, SEED_MANIFEST)
    rows, size = _scan_tree_fd(
        object_fd,
        exclude_root={SEED_MANIFEST},
        root_modes={0o500, 0o700} if recover_root else {0o500},
        directory_modes={0o500},
        file_modes={0o400, 0o500},
    )
    expected = _manifest(rows, size)
    final_value = _read_canonical_json_at(object_fd, SEED_MANIFEST)
    if (
        value != expected
        or final_value != value
        or value["contentDigest"] != digest_value
    ):
        raise CacheAuthorityError("cache seed closure differs from its manifest")
    if recover_root and seal_recovered_root and root_mode == 0o700:
        os.fchmod(object_fd, 0o500)
        os.fsync(object_fd)
    return value


def _store_for_seed(seed: Path) -> tuple[Path, str]:
    seed = _absolute(seed, "cache seed")
    if seed.parent.name != "objects" or DIGEST_PATTERN.fullmatch(DIGEST + seed.name) is None:
        raise CacheAuthorityError("cache seed path is not content addressed")
    return seed.parent.parent, DIGEST + seed.name


def _read_locator(locators_fd: int, authority_key: str) -> dict[str, object]:
    value = _read_canonical_json_at(
        locators_fd, authority_key.removeprefix(DIGEST) + ".json"
    )
    if value.get("schema") != LOCATOR_SCHEMA:
        raise CacheAuthorityError("cache runtime locator schema is invalid")
    return value


def resolve_seed(
    store: Path, authority_key: str
) -> tuple[Path, dict[str, object]]:
    store = _absolute(store, "cache store")
    if DIGEST_PATTERN.fullmatch(authority_key) is None:
        raise CacheAuthorityError("cache authority key is invalid")
    store_fd, children = _open_store(store, create=False)
    lifecycle: AuthorityLock | None = None
    try:
        lifecycle = _lifecycle_lock(children["locks"], shared=True, create=False)
        lifecycle.revalidate()
        locator = _read_locator(children["locators"], authority_key)
        digest_value = locator.get("contentDigest")
        if not isinstance(digest_value, str) or DIGEST_PATTERN.fullmatch(digest_value) is None:
            raise CacheAuthorityError("cache runtime locator digest is invalid")
        _validate_binding(locator, _locator(authority_key, digest_value))
        object_fd = _object_fd(children["objects"], digest_value)
        try:
            manifest = _validate_object_fd(
                object_fd, digest_value, recover_root=False
            )
        finally:
            os.close(object_fd)
        lifecycle.revalidate()
        return store / "objects" / digest_value.removeprefix(DIGEST), manifest
    finally:
        if lifecycle is not None:
            lifecycle.close()
        for descriptor in children.values():
            os.close(descriptor)
        os.close(store_fd)


def recover_seed(
    store: Path, authority_key: str
) -> tuple[Path, dict[str, object]] | None:
    """Finish a crash-interrupted publication while its creation lock is held."""
    store = _absolute(store, "cache store")
    if DIGEST_PATTERN.fullmatch(authority_key) is None:
        raise CacheAuthorityError("cache authority key is invalid")
    store_fd, children = _open_store(store, create=False)
    lifecycle: AuthorityLock | None = None
    try:
        candidates = _authority_candidates(store_fd, authority_key)
        try:
            lifecycle = _lifecycle_lock(
                children["locks"], shared=False, create=False
            )
        except FileNotFoundError:
            for name in ("locators", "objects", "pending"):
                with os.scandir(children[name]) as iterator:
                    if next(iterator, None) is not None:
                        raise CacheAuthorityError(
                            "cache publication exists without lifecycle authority"
                        )
            _discard_candidates(store_fd, candidates)
            return None

        lifecycle.revalidate()

        locator_name = authority_key.removeprefix(DIGEST) + ".json"
        try:
            locator = _read_locator(children["locators"], authority_key)
        except FileNotFoundError:
            locator = None
        if locator is not None:
            digest_value = locator.get("contentDigest")
            if (
                not isinstance(digest_value, str)
                or DIGEST_PATTERN.fullmatch(digest_value) is None
            ):
                raise CacheAuthorityError("cache runtime locator digest is invalid")
            _validate_binding(locator, _locator(authority_key, digest_value))
            object_fd = _object_fd(children["objects"], digest_value)
            try:
                lifecycle.revalidate()
                manifest = _validate_object_fd(
                    object_fd, digest_value, recover_root=True
                )
            finally:
                os.close(object_fd)
            try:
                pending = _read_canonical_json_at(children["pending"], locator_name)
                _validate_binding(pending, _pending(authority_key, digest_value))
                lifecycle.revalidate()
                os.unlink(locator_name, dir_fd=children["pending"])
                os.fsync(children["pending"])
            except FileNotFoundError:
                pass
            lifecycle.revalidate()
            _discard_candidates(store_fd, candidates)
            lifecycle.revalidate()
            return store / "objects" / digest_value.removeprefix(DIGEST), manifest

        try:
            pending = _read_canonical_json_at(children["pending"], locator_name)
        except FileNotFoundError:
            lifecycle.revalidate()
            _discard_candidates(store_fd, candidates)
            lifecycle.revalidate()
            return None
        digest_value = pending.get("contentDigest")
        if (
            not isinstance(digest_value, str)
            or DIGEST_PATTERN.fullmatch(digest_value) is None
        ):
            raise CacheAuthorityError("cache pending publication digest is invalid")
        _validate_binding(pending, _pending(authority_key, digest_value))
        try:
            object_fd = _object_fd(children["objects"], digest_value)
        except FileNotFoundError:
            matching: list[str] = []
            for name in candidates:
                candidate_fd = os.open(name, _directory_flags(), dir_fd=store_fd)
                try:
                    try:
                        _validate_object_fd(
                            candidate_fd,
                            digest_value,
                            recover_root=True,
                            seal_recovered_root=False,
                        )
                    except (CacheAuthorityError, FileNotFoundError, ValueError, TypeError):
                        continue
                    matching.append(name)
                finally:
                    os.close(candidate_fd)
            if len(matching) > 1:
                raise CacheAuthorityError(
                    "cache pending publication has ambiguous candidates"
                )
            if not matching:
                lifecycle.revalidate()
                _discard_candidates(store_fd, candidates)
                os.unlink(locator_name, dir_fd=children["pending"])
                os.fsync(children["pending"])
                lifecycle.revalidate()
                return None
            lifecycle.revalidate()
            _rename_no_replace(
                store_fd,
                matching[0],
                children["objects"],
                digest_value.removeprefix(DIGEST),
            )
            os.fsync(store_fd)
            os.fsync(children["objects"])
            candidates.remove(matching[0])
            object_fd = _object_fd(children["objects"], digest_value)
        try:
            lifecycle.revalidate()
            manifest = _validate_object_fd(
                object_fd, digest_value, recover_root=True
            )
        finally:
            os.close(object_fd)
        os.fsync(children["objects"])
        binding = _locator(authority_key, digest_value)
        lifecycle.revalidate()
        if not _write_immutable_json_at(children["locators"], locator_name, binding):
            _validate_binding(
                _read_locator(children["locators"], authority_key), binding
            )
        lifecycle.revalidate()
        os.unlink(locator_name, dir_fd=children["pending"])
        os.fsync(children["pending"])
        lifecycle.revalidate()
        _discard_candidates(store_fd, candidates)
        lifecycle.revalidate()
        return store / "objects" / digest_value.removeprefix(DIGEST), manifest
    finally:
        if lifecycle is not None:
            lifecycle.close()
        for descriptor in children.values():
            os.close(descriptor)
        os.close(store_fd)


def validate_seed(seed: Path, authority_key: str) -> dict[str, object]:
    if DIGEST_PATTERN.fullmatch(authority_key) is None:
        raise CacheAuthorityError("cache authority key is invalid")
    store, digest_value = _store_for_seed(seed)
    store_fd, children = _open_store(store, create=False)
    lifecycle: AuthorityLock | None = None
    try:
        lifecycle = _lifecycle_lock(children["locks"], shared=True, create=False)
        lifecycle.revalidate()
        _validate_binding(
            _read_locator(children["locators"], authority_key),
            _locator(authority_key, digest_value),
        )
        object_fd = _object_fd(children["objects"], digest_value)
        try:
            observed = _validate_object_fd(
                object_fd, digest_value, recover_root=False
            )
        finally:
            os.close(object_fd)
        lifecycle.revalidate()
        return observed
    finally:
        if lifecycle is not None:
            lifecycle.close()
        for descriptor in children.values():
            os.close(descriptor)
        os.close(store_fd)


def _prepare_candidate(candidate_fd: int) -> dict[str, object]:
    _normalize_fd(candidate_fd)
    rows, size = _scan_tree_fd(candidate_fd)
    value = _manifest(rows, size)
    if not _write_immutable_json_at(candidate_fd, SEED_MANIFEST, value):
        raise CacheAuthorityError("cache candidate manifest publication conflicted")
    _seal_children(candidate_fd)
    return value


def _rename_no_replace(
    source_fd: int, source: str, destination_fd: int, destination: str
) -> None:
    """Atomically move one directory without replacing an existing object."""
    libc = ctypes.CDLL(None, use_errno=True)
    source_bytes = os.fsencode(source)
    destination_bytes = os.fsencode(destination)
    system = platform.system()
    if system == "Darwin":
        operation = libc.renameatx_np
        operation.argtypes = [
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_uint,
        ]
        operation.restype = ctypes.c_int
        result = operation(
            source_fd, source_bytes, destination_fd, destination_bytes, 0x00000004
        )
    elif system == "Linux":
        operation = libc.renameat2
        operation.argtypes = [
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_uint,
        ]
        operation.restype = ctypes.c_int
        result = operation(
            source_fd, source_bytes, destination_fd, destination_bytes, 0x00000001
        )
    else:
        raise CacheAuthorityError("atomic cache publication is unsupported on this host")
    if result == 0:
        return
    error_number = ctypes.get_errno()
    if error_number in {errno.EEXIST, errno.ENOTEMPTY}:
        raise FileExistsError(error_number, "cache content object already exists")
    raise OSError(error_number, "atomic cache content publication failed")


def publish_seed(
    candidate: Path, store: Path, authority_key: str
) -> tuple[Path, dict[str, object]]:
    candidate = _absolute(candidate, "cache candidate")
    store = _absolute(store, "cache store")
    if DIGEST_PATTERN.fullmatch(authority_key) is None:
        raise CacheAuthorityError("cache authority key is invalid")
    if candidate.parent != store or not candidate.name.startswith(".candidate-"):
        raise CacheAuthorityError("cache candidate must be a private store child")
    store_fd, children = _open_store(store)
    lifecycle: AuthorityLock | None = None
    candidate_fd: int | None = None
    try:
        try:
            lifecycle = _lifecycle_lock(
                children["locks"], shared=False, create=False
            )
        except FileNotFoundError:
            for name in ("locators", "objects", "pending"):
                with os.scandir(children[name]) as iterator:
                    if next(iterator, None) is not None:
                        raise CacheAuthorityError(
                            "cache publication exists without lifecycle authority"
                        )
            lifecycle = _lifecycle_lock(
                children["locks"], shared=False, create=True
            )
        lifecycle.revalidate()
        candidate_fd = os.open(candidate.name, _directory_flags(), dir_fd=store_fd)
        candidate_metadata = os.fstat(candidate_fd)
        if (
            candidate_metadata.st_uid != os.geteuid()
            or stat.S_IMODE(candidate_metadata.st_mode) != 0o700
        ):
            raise CacheAuthorityError("cache candidate authority is unsafe")
        lifecycle.revalidate()
        value = _prepare_candidate(candidate_fd)
        digest_value = str(value["contentDigest"])
        binding = _locator(authority_key, digest_value)
        locator_name = authority_key.removeprefix(DIGEST) + ".json"
        pending_name = locator_name
        try:
            existing = _read_locator(children["locators"], authority_key)
        except FileNotFoundError:
            existing = None
        if existing is not None:
            _validate_binding(existing, binding)
            object_fd = _object_fd(children["objects"], digest_value)
            try:
                observed = _validate_object_fd(
                    object_fd, digest_value, recover_root=False
                )
            finally:
                os.close(object_fd)
            os.close(candidate_fd)
            candidate_fd = None
            lifecycle.revalidate()
            _safe_unlink_tree(store_fd, candidate.name, [0, 0])
            os.fsync(store_fd)
            try:
                observed_pending = _read_canonical_json_at(
                    children["pending"], pending_name
                )
                _validate_binding(observed_pending, _pending(authority_key, digest_value))
                lifecycle.revalidate()
                os.unlink(pending_name, dir_fd=children["pending"])
                os.fsync(children["pending"])
            except FileNotFoundError:
                pass
            lifecycle.revalidate()
            return store / "objects" / digest_value.removeprefix(DIGEST), observed

        pending_value = _pending(authority_key, digest_value)
        try:
            observed_pending = _read_canonical_json_at(children["pending"], pending_name)
            _validate_binding(observed_pending, pending_value)
        except FileNotFoundError:
            lifecycle.revalidate()
            if not _write_immutable_json_at(children["pending"], pending_name, pending_value):
                observed_pending = _read_canonical_json_at(
                    children["pending"], pending_name
                )
                _validate_binding(observed_pending, pending_value)

        object_name = digest_value.removeprefix(DIGEST)
        try:
            object_fd = _object_fd(children["objects"], digest_value)
        except FileNotFoundError:
            try:
                lifecycle.revalidate()
                _rename_no_replace(
                    store_fd, candidate.name, children["objects"], object_name
                )
            except FileExistsError:
                pass
            else:
                os.fsync(children["objects"])
                os.fsync(store_fd)
                os.close(candidate_fd)
                candidate_fd = None
            object_fd = _object_fd(children["objects"], digest_value)
        try:
            lifecycle.revalidate()
            observed = _validate_object_fd(
                object_fd, digest_value, recover_root=True
            )
        finally:
            os.close(object_fd)
        os.fsync(children["objects"])
        lifecycle.revalidate()
        if not _write_immutable_json_at(children["locators"], locator_name, binding):
            _validate_binding(_read_locator(children["locators"], authority_key), binding)
        try:
            lifecycle.revalidate()
            os.unlink(pending_name, dir_fd=children["pending"])
            os.fsync(children["pending"])
        except FileNotFoundError:
            pass
        if candidate_fd is not None:
            os.close(candidate_fd)
            candidate_fd = None
            lifecycle.revalidate()
            _safe_unlink_tree(store_fd, candidate.name, [0, 0])
            os.fsync(store_fd)
        lifecycle.revalidate()
        return store / "objects" / object_name, observed
    finally:
        if candidate_fd is not None:
            os.close(candidate_fd)
        try:
            _safe_unlink_tree(store_fd, candidate.name, [0, 0])
            os.fsync(store_fd)
        except FileNotFoundError:
            pass
        if lifecycle is not None:
            lifecycle.close()
        for descriptor in children.values():
            os.close(descriptor)
        os.close(store_fd)


def _cow_command(source: Path, destination: Path) -> list[str]:
    if platform.system() == "Darwin":
        return ["/bin/cp", "-cR", str(source), str(destination)]
    if platform.system() == "Linux":
        return ["/bin/cp", "--reflink=always", "-a", str(source), str(destination)]
    raise CacheAuthorityError("copy-on-write cache clones are unsupported on this host")


def _make_writable_fd(directory_fd: int) -> None:
    with os.scandir(directory_fd) as iterator:
        names = sorted(entry.name for entry in iterator)
    for name in names:
        metadata = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
        if stat.S_ISLNK(metadata.st_mode):
            raise CacheAuthorityError("cache clone contains a symlink")
        if stat.S_ISDIR(metadata.st_mode):
            child_fd = os.open(name, _directory_flags(), dir_fd=directory_fd)
            try:
                _make_writable_fd(child_fd)
                os.fchmod(child_fd, 0o700)
            finally:
                os.close(child_fd)
        elif stat.S_ISREG(metadata.st_mode):
            if metadata.st_nlink != 1:
                raise CacheAuthorityError("cache clone file has multiple links")
            descriptor = os.open(
                name,
                os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
                dir_fd=directory_fd,
            )
            try:
                opened = os.fstat(descriptor)
                if (
                    opened.st_nlink != 1
                    or _regular_identity(opened) != _regular_identity(metadata)
                ):
                    raise CacheAuthorityError("cache clone file authority changed")
                os.fchmod(descriptor, 0o700 if metadata.st_mode & 0o111 else 0o600)
            finally:
                os.close(descriptor)
        else:
            raise CacheAuthorityError("cache clone contains an unsupported entry")
    os.fchmod(directory_fd, 0o700)


def clone_seed(seed: Path, destination: Path, authority_key: str) -> dict[str, object]:
    seed = _absolute(seed, "cache seed")
    destination = _absolute(destination, "cache clone")
    store, digest_value = _store_for_seed(seed)
    store_fd, children = _open_store(store, create=False)
    lifecycle: AuthorityLock | None = None
    try:
        lifecycle = _lifecycle_lock(children["locks"], shared=True, create=False)
        lifecycle.revalidate()
        _validate_binding(
            _read_locator(children["locators"], authority_key),
            _locator(authority_key, digest_value),
        )
        object_fd = _object_fd(children["objects"], digest_value)
        try:
            expected = _validate_object_fd(
                object_fd, digest_value, recover_root=False
            )
        finally:
            os.close(object_fd)
        if destination.exists() or destination.is_symlink():
            raise CacheAuthorityError("cache clone destination already exists")
        lifecycle.revalidate()
        completed = subprocess.run(
            _cow_command(seed, destination),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            check=False,
        )
        if completed.returncode != 0:
            raise CacheAuthorityError("copy-on-write cache clone is unavailable")
        clone_fd = _open_private_directory(
            destination, create=False, leaf_modes={0o500}
        )
        try:
            if os.fstat(clone_fd).st_ino == os.stat(seed).st_ino:
                raise CacheAuthorityError("cache clone aliases its seed root")
            _make_writable_fd(clone_fd)
            observed_manifest = _read_canonical_json_at(
                clone_fd, SEED_MANIFEST, mode=0o600
            )
            rows, size = _scan_tree_fd(
                clone_fd,
                exclude_root={SEED_MANIFEST},
                root_modes={0o700},
                directory_modes={0o700},
                file_modes={0o600, 0o700},
            )
            observed = _manifest(rows, size)
            if observed != expected or observed_manifest != expected:
                raise CacheAuthorityError("cache clone differs from its sealed seed")
            lifecycle.revalidate()
            return observed
        finally:
            os.close(clone_fd)
    except BaseException:
        try:
            _discard(destination)
        except (FileNotFoundError, CacheAuthorityError):
            pass
        raise
    finally:
        if lifecycle is not None:
            lifecycle.close()
        for descriptor in children.values():
            os.close(descriptor)
        os.close(store_fd)


def probe_cow(root: Path) -> None:
    root = _absolute(root, "probe root")
    root_fd = _open_private_directory(root, create=True)
    os.close(root_fd)
    source = root / f".cow-source-{secrets.token_hex(8)}"
    destination = root / f".cow-destination-{secrets.token_hex(8)}"
    source.mkdir(mode=0o700)
    payload = source / "payload"
    payload.write_bytes(b"codeclew-cow-probe")
    try:
        completed = subprocess.run(
            _cow_command(source, destination),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            check=False,
        )
        if (
            completed.returncode != 0
            or (destination / "payload").read_bytes() != payload.read_bytes()
        ):
            raise CacheAuthorityError("copy-on-write cache clone preflight failed")
    finally:
        _discard(source)
        _discard(destination)


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="operation", required=True)
    probe = subparsers.add_parser("probe")
    probe.add_argument("--root", required=True)
    publish = subparsers.add_parser("publish")
    publish.add_argument("--candidate", required=True)
    publish.add_argument("--store", required=True)
    publish.add_argument("--authority-key", required=True)
    validate = subparsers.add_parser("validate")
    validate.add_argument("--seed", required=True)
    validate.add_argument("--authority-key", required=True)
    clone = subparsers.add_parser("clone")
    clone.add_argument("--seed", required=True)
    clone.add_argument("--destination", required=True)
    clone.add_argument("--authority-key", required=True)
    arguments = parser.parse_args()
    if arguments.operation == "probe":
        probe_cow(_absolute(arguments.root, "probe root"))
        value = {"schema": SCHEMA, "status": "COW_AVAILABLE"}
    elif arguments.operation == "publish":
        _path, value = publish_seed(
            _absolute(arguments.candidate, "cache candidate"),
            _absolute(arguments.store, "cache store"),
            arguments.authority_key,
        )
        value = {**value, "status": "PUBLISHED"}
    elif arguments.operation == "validate":
        value = {
            **validate_seed(
                _absolute(arguments.seed, "cache seed"), arguments.authority_key
            ),
            "status": "VALID",
        }
    else:
        value = {
            **clone_seed(
                _absolute(arguments.seed, "cache seed"),
                _absolute(arguments.destination, "cache clone"),
                arguments.authority_key,
            ),
            "status": "CLONED",
        }
    print(json.dumps(value, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (CacheAuthorityError, OSError, ValueError, TypeError):
        print(
            json.dumps(
                {"schema": SCHEMA, "status": "REFUSED"},
                sort_keys=True,
                separators=(",", ":"),
            ),
            file=sys.stderr,
        )
        raise SystemExit(2)
