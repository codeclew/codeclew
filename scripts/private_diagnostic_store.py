#!/usr/bin/env python3
"""Private, durable, content-addressed diagnostics for stabilization gates."""

from __future__ import annotations

import hashlib
import os
from pathlib import Path
import secrets
import stat


class DiagnosticStoreError(RuntimeError):
    pass


def _normalized_absolute(path: Path, label: str) -> Path:
    if not path.is_absolute() or ".." in path.parts or Path(os.path.normpath(path)) != path:
        raise DiagnosticStoreError(f"{label} must be normalized and absolute")
    return path


def _read_descriptor(descriptor: int, limit: int) -> bytes:
    value = bytearray()
    while len(value) <= limit:
        block = os.read(descriptor, min(64 * 1024, limit + 1 - len(value)))
        if not block:
            break
        value.extend(block)
    if len(value) > limit:
        raise DiagnosticStoreError("diagnostic is oversized")
    return bytes(value)


def read_bounded_owned(path: Path, limit: int = 1024 * 1024) -> bytes:
    path = _normalized_absolute(path, "diagnostic source")
    try:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    except OSError as error:
        raise DiagnosticStoreError("diagnostic source is unavailable") from error
    try:
        before = os.fstat(descriptor)
        if (
            not stat.S_ISREG(before.st_mode)
            or before.st_uid != os.geteuid()
            or stat.S_IMODE(before.st_mode) != 0o600
            or before.st_size > limit
        ):
            raise DiagnosticStoreError("diagnostic source authority is unsafe")
        value = _read_descriptor(descriptor, limit)
        after = os.fstat(descriptor)
        identity_before = (
            before.st_dev,
            before.st_ino,
            before.st_mode,
            before.st_uid,
            before.st_size,
            before.st_mtime_ns,
            before.st_ctime_ns,
        )
        identity_after = (
            after.st_dev,
            after.st_ino,
            after.st_mode,
            after.st_uid,
            after.st_size,
            after.st_mtime_ns,
            after.st_ctime_ns,
        )
        if len(value) != before.st_size or identity_before != identity_after:
            raise DiagnosticStoreError("diagnostic source changed while it was read")
        return value
    finally:
        os.close(descriptor)


def _open_private_root(control_home: Path) -> int:
    control_home = _normalized_absolute(control_home, "control home")
    flags = os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(control_home.anchor, flags)
    try:
        components = [part for part in control_home.parts if part != control_home.anchor]
        for index, component in enumerate(components):
            child = os.open(component, flags, dir_fd=descriptor)
            metadata = os.fstat(child)
            leaf = index == len(components) - 1
            valid = (
                stat.S_ISDIR(metadata.st_mode)
                and metadata.st_uid in {0, os.geteuid()}
                and (leaf or not stat.S_IMODE(metadata.st_mode) & 0o022)
                and (
                    not leaf
                    or (
                        metadata.st_uid == os.geteuid()
                        and stat.S_IMODE(metadata.st_mode) == 0o700
                    )
                )
            )
            if not valid:
                os.close(child)
                raise DiagnosticStoreError("control home authority is unsafe")
            os.close(descriptor)
            descriptor = child
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def _open_private_child(parent: int, name: str) -> int:
    if not name or name in {".", ".."} or "/" in name:
        raise DiagnosticStoreError("diagnostic directory component is invalid")
    flags = os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0)
    try:
        child = os.open(name, flags, dir_fd=parent)
    except FileNotFoundError:
        created = False
        try:
            os.mkdir(name, mode=0o700, dir_fd=parent)
            created = True
        except FileExistsError:
            pass
        if created:
            os.fsync(parent)
        child = os.open(name, flags, dir_fd=parent)
    metadata = os.fstat(child)
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) != 0o700
    ):
        os.close(child)
        raise DiagnosticStoreError("diagnostic directory authority is unsafe")
    return child


def _verify_object(directory: int, name: str, expected: bytes) -> None:
    descriptor = os.open(
        name,
        os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
        dir_fd=directory,
    )
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) != 0o400
            or metadata.st_size != len(expected)
            or _read_descriptor(descriptor, len(expected)) != expected
        ):
            raise DiagnosticStoreError("diagnostic CAS collision is unsafe")
    finally:
        os.close(descriptor)


def store_diagnostic_bytes(value: bytes, control_home: Path) -> str:
    if not isinstance(value, bytes) or not value or len(value) > 1024 * 1024:
        raise DiagnosticStoreError("diagnostic bytes are invalid")
    digest = "sha256:" + hashlib.sha256(value).hexdigest()
    root = _open_private_root(control_home)
    try:
        diagnostics = _open_private_child(root, "diagnostics")
        try:
            directory = _open_private_child(diagnostics, "cold-runtime")
        finally:
            os.close(diagnostics)
        try:
            name = f"{digest.removeprefix('sha256:')}.stderr"
            temporary = f".{name}.{os.getpid()}.{secrets.token_hex(16)}.tmp"
            descriptor = os.open(
                temporary,
                os.O_WRONLY
                | os.O_CREAT
                | os.O_EXCL
                | getattr(os, "O_NOFOLLOW", 0),
                0o600,
                dir_fd=directory,
            )
            try:
                offset = 0
                while offset < len(value):
                    offset += os.write(descriptor, value[offset:])
                os.fsync(descriptor)
                os.fchmod(descriptor, 0o400)
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
            try:
                os.link(
                    temporary,
                    name,
                    src_dir_fd=directory,
                    dst_dir_fd=directory,
                    follow_symlinks=False,
                )
                os.fsync(directory)
            except FileExistsError:
                pass
            finally:
                os.unlink(temporary, dir_fd=directory)
                os.fsync(directory)
            _verify_object(directory, name, value)
        finally:
            os.close(directory)
    finally:
        os.close(root)
    return digest


def store_diagnostic(source: Path, control_home: Path) -> str:
    return store_diagnostic_bytes(read_bounded_owned(source), control_home)
