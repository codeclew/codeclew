"""Deterministic, path-free authority for the Maven distribution on PATH."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import stat
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping


SCHEMA = "codeclew-maven-distribution-authority/1.0"
MAX_FILES = 4096
MAX_NODES = 8192
MAX_DEPTH = 64
MAX_FILE_BYTES = 128 * 1024 * 1024
MAX_TOTAL_BYTES = 512 * 1024 * 1024
AUTHORITY_DIRECTORIES = ("bin", "boot", "conf", "lib")


class MavenAuthorityError(RuntimeError):
    """The Maven entrypoint does not close over a valid distribution."""


@dataclass(frozen=True)
class MavenAuthority:
    executable: Path
    distribution_root: Path
    digest: str


def _canonical_bytes(value: object) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")


def _identity(metadata: os.stat_result) -> tuple[int, int, int, int, int, int]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_mode,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def _file_row_at(
    directory_descriptor: int,
    name: str,
    relative_file: str,
    counters: dict[str, int],
) -> dict[str, object]:
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    flags |= getattr(os, "O_NONBLOCK", 0)
    try:
        descriptor = os.open(name, flags, dir_fd=directory_descriptor)
    except OSError as error:
        raise MavenAuthorityError("MAVEN_AUTHORITY_UNAVAILABLE") from error
    try:
        before = os.fstat(descriptor)
        if (
            not stat.S_ISREG(before.st_mode)
            or before.st_size < 0
            or before.st_size > MAX_FILE_BYTES
        ):
            raise MavenAuthorityError("MAVEN_AUTHORITY_INVALID")
        next_file_count = counters["files"] + 1
        next_total_bytes = counters["bytes"] + before.st_size
        if next_file_count > MAX_FILES or next_total_bytes > MAX_TOTAL_BYTES:
            raise MavenAuthorityError("MAVEN_AUTHORITY_TOO_LARGE")
        digest = hashlib.sha256()
        observed_size = 0
        while True:
            try:
                chunk = os.read(descriptor, min(1024 * 1024, MAX_FILE_BYTES + 1 - observed_size))
            except OSError as error:
                raise MavenAuthorityError("MAVEN_AUTHORITY_UNAVAILABLE") from error
            if not chunk:
                break
            observed_size += len(chunk)
            if observed_size > MAX_FILE_BYTES:
                raise MavenAuthorityError("MAVEN_AUTHORITY_TOO_LARGE")
            digest.update(chunk)
        after = os.fstat(descriptor)
    except MavenAuthorityError:
        raise
    except OSError as error:
        raise MavenAuthorityError("MAVEN_AUTHORITY_UNAVAILABLE") from error
    finally:
        os.close(descriptor)
    try:
        linked = os.stat(
            name, dir_fd=directory_descriptor, follow_symlinks=False
        )
    except OSError as error:
        raise MavenAuthorityError("MAVEN_AUTHORITY_CHANGED") from error
    if (
        observed_size != before.st_size
        or _identity(before) != _identity(after)
        or _identity(before) != _identity(linked)
    ):
        raise MavenAuthorityError("MAVEN_AUTHORITY_CHANGED")
    counters["files"] = next_file_count
    counters["bytes"] = next_total_bytes
    return {
        "relativeFile": relative_file,
        "mode": stat.S_IMODE(before.st_mode),
        "size": before.st_size,
        "digest": f"sha256:{digest.hexdigest()}",
    }


def _distribution_root(entrypoint: Path) -> Path:
    if entrypoint.parent.name != "bin":
        raise MavenAuthorityError("MAVEN_DISTRIBUTION_UNRESOLVED")
    install_root = entrypoint.parent.parent
    for raw_candidate in (install_root / "libexec", install_root):
        try:
            raw_metadata = os.lstat(raw_candidate)
            if stat.S_ISLNK(raw_metadata.st_mode) or not stat.S_ISDIR(raw_metadata.st_mode):
                continue
            candidate = raw_candidate.resolve(strict=True)
            launcher = candidate / "bin" / "mvn"
            launcher_metadata = os.lstat(launcher)
        except OSError:
            continue
        if not stat.S_ISREG(launcher_metadata.st_mode) or stat.S_ISLNK(launcher_metadata.st_mode):
            continue
        valid = os.access(launcher, os.X_OK)
        for directory_name in AUTHORITY_DIRECTORIES:
            try:
                metadata = os.lstat(candidate / directory_name)
            except OSError:
                valid = False
                break
            if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
                valid = False
                break
        if valid:
            return candidate
    raise MavenAuthorityError("MAVEN_DISTRIBUTION_UNRESOLVED")


def _directory_rows_at(
    parent_descriptor: int,
    name: str,
    relative_directory: str,
    depth: int,
    counters: dict[str, int],
) -> list[dict[str, object]]:
    if depth > MAX_DEPTH or not name or "/" in name or name in {".", ".."}:
        raise MavenAuthorityError("MAVEN_AUTHORITY_TOO_LARGE")
    directory_flags = (
        os.O_RDONLY
        | getattr(os, "O_CLOEXEC", 0)
        | getattr(os, "O_NOFOLLOW", 0)
        | getattr(os, "O_DIRECTORY", 0)
    )
    try:
        linked_before = os.stat(
            name, dir_fd=parent_descriptor, follow_symlinks=False
        )
        descriptor = os.open(name, directory_flags, dir_fd=parent_descriptor)
    except OSError as error:
        raise MavenAuthorityError("MAVEN_AUTHORITY_INVALID") from error
    try:
        opened_before = os.fstat(descriptor)
        if (
            not stat.S_ISDIR(opened_before.st_mode)
            or _identity(linked_before) != _identity(opened_before)
        ):
            raise MavenAuthorityError("MAVEN_AUTHORITY_CHANGED")
        names: list[str] = []
        try:
            with os.scandir(descriptor) as iterator:
                for entry in iterator:
                    counters["nodes"] += 1
                    if counters["nodes"] > MAX_NODES:
                        raise MavenAuthorityError("MAVEN_AUTHORITY_TOO_LARGE")
                    if not entry.name or "/" in entry.name or entry.name in {".", ".."}:
                        raise MavenAuthorityError("MAVEN_AUTHORITY_INVALID")
                    names.append(entry.name)
        except MavenAuthorityError:
            raise
        except OSError as error:
            raise MavenAuthorityError("MAVEN_AUTHORITY_UNAVAILABLE") from error

        rows: list[dict[str, object]] = []
        for child_name in sorted(names):
            relative_file = f"{relative_directory}/{child_name}"
            try:
                child_metadata = os.stat(
                    child_name,
                    dir_fd=descriptor,
                    follow_symlinks=False,
                )
            except OSError as error:
                raise MavenAuthorityError("MAVEN_AUTHORITY_CHANGED") from error
            if stat.S_ISLNK(child_metadata.st_mode):
                raise MavenAuthorityError("MAVEN_AUTHORITY_INVALID")
            if stat.S_ISDIR(child_metadata.st_mode):
                rows.extend(
                    _directory_rows_at(
                        descriptor,
                        child_name,
                        relative_file,
                        depth + 1,
                        counters,
                    )
                )
            elif stat.S_ISREG(child_metadata.st_mode):
                rows.append(
                    _file_row_at(
                        descriptor, child_name, relative_file, counters
                    )
                )
            else:
                raise MavenAuthorityError("MAVEN_AUTHORITY_INVALID")

        opened_after = os.fstat(descriptor)
        linked_after = os.stat(
            name, dir_fd=parent_descriptor, follow_symlinks=False
        )
        if (
            _identity(opened_before) != _identity(opened_after)
            or _identity(opened_before) != _identity(linked_after)
        ):
            raise MavenAuthorityError("MAVEN_AUTHORITY_CHANGED")
        return rows
    except OSError as error:
        raise MavenAuthorityError("MAVEN_AUTHORITY_CHANGED") from error
    finally:
        os.close(descriptor)


def _authority_rows(root: Path) -> list[dict[str, object]]:
    directory_flags = (
        os.O_RDONLY
        | getattr(os, "O_CLOEXEC", 0)
        | getattr(os, "O_NOFOLLOW", 0)
        | getattr(os, "O_DIRECTORY", 0)
    )
    try:
        linked_before = os.lstat(root)
        descriptor = os.open(root, directory_flags)
    except OSError as error:
        raise MavenAuthorityError("MAVEN_AUTHORITY_INVALID") from error
    try:
        opened_before = os.fstat(descriptor)
        if (
            stat.S_ISLNK(linked_before.st_mode)
            or not stat.S_ISDIR(opened_before.st_mode)
            or _identity(linked_before) != _identity(opened_before)
        ):
            raise MavenAuthorityError("MAVEN_AUTHORITY_CHANGED")
        counters = {"nodes": 0, "files": 0, "bytes": 0}
        rows: list[dict[str, object]] = []
        for directory_name in AUTHORITY_DIRECTORIES:
            rows.extend(
                _directory_rows_at(
                    descriptor,
                    directory_name,
                    directory_name,
                    1,
                    counters,
                )
            )
        opened_after = os.fstat(descriptor)
        linked_after = os.lstat(root)
        if (
            _identity(opened_before) != _identity(opened_after)
            or _identity(opened_before) != _identity(linked_after)
        ):
            raise MavenAuthorityError("MAVEN_AUTHORITY_CHANGED")
        return rows
    except OSError as error:
        raise MavenAuthorityError("MAVEN_AUTHORITY_CHANGED") from error
    finally:
        os.close(descriptor)


def discover(
    environment: Mapping[str, str] | None = None,
) -> MavenAuthority:
    source = os.environ if environment is None else environment
    search_path = source.get("PATH")
    if not isinstance(search_path, str) or not search_path or "\0" in search_path:
        raise MavenAuthorityError("MAVEN_ENTRYPOINT_UNAVAILABLE")
    raw = shutil.which("mvn", path=search_path)
    if raw is None:
        raise MavenAuthorityError("MAVEN_ENTRYPOINT_UNAVAILABLE")
    try:
        entrypoint = Path(raw).resolve(strict=True)
        entrypoint_metadata = os.lstat(entrypoint)
    except OSError as error:
        raise MavenAuthorityError("MAVEN_ENTRYPOINT_UNAVAILABLE") from error
    if (
        stat.S_ISLNK(entrypoint_metadata.st_mode)
        or not stat.S_ISREG(entrypoint_metadata.st_mode)
        or not os.access(entrypoint, os.X_OK)
    ):
        raise MavenAuthorityError("MAVEN_ENTRYPOINT_INVALID")

    distribution_root = _distribution_root(entrypoint)
    rows: list[dict[str, object]] = []
    total_bytes = 0
    for row in _authority_rows(distribution_root):
        rows.append(row)
        total_bytes += int(row["size"])
        if total_bytes > MAX_TOTAL_BYTES:
            raise MavenAuthorityError("MAVEN_AUTHORITY_TOO_LARGE")
    rows.sort(key=lambda row: str(row["relativeFile"]))

    relative_files = {str(row["relativeFile"]) for row in rows}
    if (
        "bin/mvn" not in relative_files
        or not any(name.startswith("conf/") for name in relative_files)
        or not any(
            name.endswith(".jar") and name.startswith(("boot/", "lib/"))
            for name in relative_files
        )
    ):
        raise MavenAuthorityError("MAVEN_AUTHORITY_INCOMPLETE")
    unsigned = {
        "schema": SCHEMA,
        "distributionFiles": rows,
        "fileCount": len(rows),
        "totalBytes": total_bytes,
    }
    digest = f"sha256:{hashlib.sha256(_canonical_bytes(unsigned)).hexdigest()}"
    return MavenAuthority(distribution_root / "bin" / "mvn", distribution_root, digest)
