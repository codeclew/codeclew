#!/usr/bin/env python3
"""Build and verify the private compiler-derived S4K shape oracle.

The builder is deliberately separate from the measured pilot.  ``build``
reads the frozen private G1K inputs, replays only the local semantic
context/callables/impact contour, and writes canonical owner-only files.
``verify`` is the runner-compatible, non-measured verifier.  It never accepts
projection rows as input: exact rows can only be emitted by ``build`` after a
member-scoped full-symbol impact confirms the compiler projection.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from unittest import mock


sys.path.insert(0, os.fspath(Path(__file__).resolve().parent))

import run_thread_kotlin_descriptor_gate as descriptor_gate
import verify_thread_kotlin_descriptor_gate as g1k_verifier


SHAPE_SCHEMA = "codeclew-private-kotlin-descriptor-shape-oracle/1.0"
ATTESTATION_SCHEMA = "codeclew-private-kotlin-descriptor-shape-attestation/1.0"
REVIEW_SCHEMA = "codeclew-kotlin-descriptor-shape-builder-review/1.0"
RESOURCE_LEDGER_SCHEMA = "codeclew-kotlin-descriptor-shape-resources/1.0"
PUBLICATION_LEDGER_SCHEMA = "codeclew-kotlin-descriptor-shape-publication/1.0"
RESULT_SCHEMA = "codeclew-kotlin-descriptor-shape-builder-result/1.0"
PROJECTION_SCHEMA = "codeclew-kotlin-callable-fact/1.0"
REPOSITORY_BLOB_SCHEMA = "codeclew-repository-input-blob/2.0"
CAS_DOMAIN = b"codeclew-cas/v2\0"
COMPILATION = ":/main"
MAX_PRIVATE_BYTES = 16 * 1024 * 1024
MAX_CHECKED_BYTES = 512 * 1024
MAX_CLEW_BYTES = 256 * 1024 * 1024
MAX_GIT_BLOB_BYTES = 16 * 1024 * 1024
SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")
GIT_OID = descriptor_gate.GIT_BLOB_OID
SAFE_IDENTIFIER = re.compile(r"^[A-Za-z_][A-Za-z0-9_]{0,255}$")
SAFE_CATEGORY = re.compile(r"^[A-Z][A-Z0-9_]{0,127}$")
SAFE_REQUIRED_CHECK = re.compile(r"^VERIFY_[A-Z0-9_]{1,120}$")
THREAD_ID = re.compile(r"^thread:(?:sha256:)?[0-9A-Za-z_-]{1,128}$")
THREAD_CONTEXT_ID = re.compile(r"^thread-context:sha256:[0-9a-f]{64}$")
FACT_SET_ID = re.compile(r"^thread-callables:sha256:[0-9a-f]{64}$")
IMPACT_ID = re.compile(r"^thread-impact:sha256:[0-9a-f]{64}$")
SESSION_ID = descriptor_gate.SESSION_ID
EXACT_FIELDS = {
    "descriptorClass",
    "declarationKind",
    "name",
    "ownerIdentity",
    "normalizedSignature",
    "shapeDigest",
    "relativeFile",
    "blobOid",
    "sourceRange",
}
TYPE_ORACLE_KINDS = {"CLASS", "DATA_CLASS", "ENUM_CLASS", "INTERFACE"}
CLEW_ENV_ALLOW = {
    "HOME",
    "CODECLEW_HOME",
    "XDG_CACHE_HOME",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "JAVA_HOME",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "GRADLE_USER_HOME",
    "MAVEN_USER_HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "PATH",
}
CLEW_ENV_PATHS = {
    "HOME",
    "CODECLEW_HOME",
    "XDG_CACHE_HOME",
    "TMPDIR",
    "JAVA_HOME",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "GRADLE_USER_HOME",
    "MAVEN_USER_HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
}


class BuilderError(RuntimeError):
    """A path-free, identifier-free shape-oracle failure."""

    def __init__(self, code: str):
        if re.fullmatch(r"[A-Z][A-Z0-9_]{0,63}", code) is None:
            code = "SHAPE_BUILDER_INTERNAL_FAILURE"
        super().__init__(code)
        self.code = code


class ResidualProcessError(BuilderError):
    """The child process group could not be proven absent after termination."""

    def __init__(self) -> None:
        super().__init__("RESIDUAL_PROCESS_UNPROVEN")


@dataclass(frozen=True)
class Authorities:
    corpus: descriptor_gate.Corpus | None
    corpus_value: dict[str, Any] | None
    benchmark: descriptor_gate.Benchmark | None
    benchmark_value: dict[str, Any] | None
    g1k_value: dict[str, Any]
    g1k_digest: str
    clew: Path
    runtime_digest: str
    git: Path
    git_digest: str
    experiment_root_digest: str


@dataclass(frozen=True)
class SealedFixture:
    tree_oid: str
    content_digest: str
    files: tuple[tuple[str, int, str, bytes], ...]


@dataclass(frozen=True)
class ReviewAuthority:
    value: dict[str, Any]
    digest: str
    pilot_runner: Path
    pilot_runner_digest: str
    test_digest: str


class RuntimeWitness:
    def __init__(self) -> None:
        self.runtime_key: str | None = None

    def observe(self, runtime_key: Any, runtime_mode: Any) -> None:
        if (
            not isinstance(runtime_key, str)
            or SHA256.fullmatch(runtime_key) is None
            or runtime_mode != "RELEASE"
            or (
                self.runtime_key is not None
                and self.runtime_key != runtime_key
            )
        ):
            raise BuilderError("COMPILER_RUNTIME_AUTHORITY_CHANGED")
        self.runtime_key = runtime_key

    def require(self) -> str:
        if self.runtime_key is None:
            raise BuilderError("COMPILER_RUNTIME_AUTHORITY_CHANGED")
        return self.runtime_key


@dataclass(frozen=True)
class CompilerDeclaration:
    member_alias: str
    side: str
    symbol_identity: str
    compiler_name: str
    declaration_kind: str
    descriptor_class: str
    projected_shape: dict[str, Any]
    shape_digest: str
    source: dict[str, Any]

    def unpinned_row(self) -> dict[str, Any]:
        return {
            "descriptorClass": self.descriptor_class,
            "declarationKind": self.declaration_kind,
            "name": self.compiler_name,
            "ownerIdentity": self.projected_shape["ownerIdentity"],
            "normalizedSignature": self.symbol_identity.split(":", 1)[1],
            "shapeDigest": self.shape_digest,
            "relativeFile": self.source["path"],
            "sourceRange": {
                "startByte": self.source["start"],
                "endByte": self.source["end"],
            },
        }


def canonical_bytes(value: Any) -> bytes:
    try:
        return json.dumps(
            value, ensure_ascii=False, separators=(",", ":"), sort_keys=True
        ).encode("utf-8")
    except (TypeError, ValueError) as error:
        raise BuilderError("NON_CANONICAL_VALUE") from error


def authority_digest(value: Any) -> str:
    return f"sha256:{hashlib.sha256(canonical_bytes(value)).hexdigest()}"


def file_digest(path: Path, maximum: int = MAX_CLEW_BYTES) -> str:
    try:
        metadata = path.stat()
    except OSError as error:
        raise BuilderError("AUTHORITY_FILE_UNAVAILABLE") from error
    if not stat.S_ISREG(metadata.st_mode) or not 0 < metadata.st_size <= maximum:
        raise BuilderError("AUTHORITY_FILE_UNAVAILABLE")
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                digest.update(chunk)
    except OSError as error:
        raise BuilderError("AUTHORITY_FILE_UNAVAILABLE") from error
    return f"sha256:{digest.hexdigest()}"


def _duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise BuilderError("DUPLICATE_JSON_KEY")
        value[key] = item
    return value


def _json_value(raw: bytes, code: str, *, checked: bool = False) -> dict[str, Any]:
    try:
        value = json.loads(raw, object_pairs_hook=_duplicates)
    except (json.JSONDecodeError, UnicodeDecodeError, BuilderError) as error:
        raise BuilderError(code) from error
    canonical = canonical_bytes(value) if isinstance(value, dict) else b""
    accepted = {canonical, canonical + b"\n"} if checked else {canonical + b"\n"}
    if not isinstance(value, dict) or raw not in accepted:
        raise BuilderError(code)
    return value


def private_json(
    path: Path, code: str, maximum: int = MAX_PRIVATE_BYTES
) -> tuple[Path, dict[str, Any], bytes]:
    try:
        absolute = path if path.is_absolute() else Path.cwd() / path
        lstat = os.lstat(absolute)
        resolved = absolute.resolve(strict=True)
    except OSError as error:
        raise BuilderError(code) from error
    if (
        stat.S_ISLNK(lstat.st_mode)
        or not stat.S_ISREG(lstat.st_mode)
        or stat.S_IMODE(lstat.st_mode) != 0o600
        or lstat.st_uid != os.geteuid()
        or resolved != absolute.absolute()
        or not 0 < lstat.st_size <= maximum
    ):
        raise BuilderError(code)
    try:
        raw = resolved.read_bytes()
    except OSError as error:
        raise BuilderError(code) from error
    return resolved, _json_value(raw, code), raw


def checked_json(
    path: Path, code: str, maximum: int = MAX_CHECKED_BYTES
) -> tuple[Path, dict[str, Any], bytes]:
    try:
        absolute = path if path.is_absolute() else Path.cwd() / path
        lstat = os.lstat(absolute)
        resolved = absolute.resolve(strict=True)
        metadata = resolved.stat()
        raw = resolved.read_bytes()
    except OSError as error:
        raise BuilderError(code) from error
    if (
        stat.S_ISLNK(lstat.st_mode)
        or not stat.S_ISREG(metadata.st_mode)
        or not 0 < len(raw) <= maximum
    ):
        raise BuilderError(code)
    return resolved, _json_value(raw, code, checked=True), raw


def closed(value: Any, fields: set[str], code: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != fields:
        raise BuilderError(code)
    return value


def digest(value: Any, code: str) -> str:
    if not isinstance(value, str) or SHA256.fullmatch(value) is None:
        raise BuilderError(code)
    return value


def positive_integer(value: Any, code: str, *, zero: bool = False) -> int:
    minimum = 0 if zero else 1
    if type(value) is not int or value < minimum:
        raise BuilderError(code)
    return value


def cas_reference(value: Any, object_schema: str | None, code: str) -> dict[str, Any]:
    row = closed(value, {"schema", "objectSchema", "digest", "size"}, code)
    if row["schema"] != descriptor_gate.CAS_OBJECT_SCHEMA:
        raise BuilderError(code)
    if object_schema is not None and row["objectSchema"] != object_schema:
        raise BuilderError(code)
    if not isinstance(row["objectSchema"], str) or not row["objectSchema"]:
        raise BuilderError(code)
    digest(row["digest"], code)
    positive_integer(row["size"], code)
    return row


def executable(path: Path) -> tuple[Path, str]:
    try:
        resolved = path.resolve(strict=True)
        metadata = resolved.stat()
    except OSError as error:
        raise BuilderError("INVALID_CLEW_EXECUTABLE") from error
    if (
        not stat.S_ISREG(metadata.st_mode)
        or not os.access(resolved, os.X_OK)
        or not 0 < metadata.st_size <= MAX_CLEW_BYTES
    ):
        raise BuilderError("INVALID_CLEW_EXECUTABLE")
    return resolved, file_digest(resolved)


def regular_file(path: Path, code: str) -> tuple[Path, str]:
    try:
        resolved = path.resolve(strict=True)
        metadata = resolved.stat()
    except OSError as error:
        raise BuilderError(code) from error
    if not stat.S_ISREG(metadata.st_mode) or not 0 < metadata.st_size <= MAX_CLEW_BYTES:
        raise BuilderError(code)
    return resolved, file_digest(resolved)


def git_executable(candidate: Path) -> Path:
    path, _ = executable(candidate)
    return path


def _isolated_git_environment() -> dict[str, str]:
    return {
        "PATH": os.defpath,
        "HOME": os.fspath(Path(tempfile.gettempdir()).resolve()),
        "LANG": "C",
        "LC_ALL": "C",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_SYSTEM": os.devnull,
        "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_NO_REPLACE_OBJECTS": "1",
        "GIT_OPTIONAL_LOCKS": "0",
        "GIT_TERMINAL_PROMPT": "0",
    }


def _process_group_exists(process_group: int) -> bool:
    try:
        os.killpg(process_group, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def _terminate_process_group(process: subprocess.Popen[Any]) -> bool:
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    except OSError:
        return False
    deadline = time.monotonic() + 2.0
    while time.monotonic() < deadline and _process_group_exists(process.pid):
        # Reap the leader promptly.  On macOS a process group containing only
        # our unreaped zombie may answer killpg(0) with EPERM.
        process.poll()
        time.sleep(0.02)
    process.poll()
    if _process_group_exists(process.pid):
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except OSError:
            return False
    if process.poll() is None:
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            try:
                process.kill()
                process.wait(timeout=2)
            except (OSError, subprocess.TimeoutExpired):
                return False
    deadline = time.monotonic() + 2.0
    while time.monotonic() < deadline and _process_group_exists(process.pid):
        time.sleep(0.02)
    return not _process_group_exists(process.pid)


def _run_bounded_process(
    command: list[str],
    *,
    environment: dict[str, str],
    timeout: int,
    stdout_limit: int,
    stderr_limit: int,
    code: str,
) -> tuple[int, bytes, bytes]:
    if timeout <= 0 or stdout_limit < 0 or stderr_limit < 0:
        raise BuilderError(code)
    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            close_fds=True,
            start_new_session=True,
            env=environment,
        )
    except OSError as error:
        raise BuilderError(code) from error
    assert process.stdout is not None and process.stderr is not None
    streams = {
        process.stdout.fileno(): (process.stdout, stdout_limit, bytearray()),
        process.stderr.fileno(): (process.stderr, stderr_limit, bytearray()),
    }
    selector = selectors.DefaultSelector()
    for descriptor in streams:
        os.set_blocking(descriptor, False)
        selector.register(descriptor, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout

    def fail() -> None:
        if not _terminate_process_group(process):
            raise ResidualProcessError()
        raise BuilderError(code)

    try:
        while selector.get_map():
            remaining_time = deadline - time.monotonic()
            if remaining_time <= 0:
                fail()
            events = selector.select(min(0.1, remaining_time))
            for key, _ in events:
                stream, limit, buffer = streams[key.fd]
                try:
                    chunk = os.read(key.fd, min(65_536, limit + 1 - len(buffer)))
                except BlockingIOError:
                    continue
                if not chunk:
                    selector.unregister(key.fd)
                    continue
                buffer.extend(chunk)
                if len(buffer) > limit:
                    fail()
            if process.poll() is not None and _process_group_exists(process.pid):
                # A descendant inherited a pipe or otherwise survived the
                # leader.  It is never accepted as successful completion.
                fail()
        try:
            return_code = process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            fail()
            raise AssertionError("unreachable")
        if _process_group_exists(process.pid):
            fail()
        return (
            return_code,
            bytes(streams[process.stdout.fileno()][2]),
            bytes(streams[process.stderr.fileno()][2]),
        )
    finally:
        selector.close()
        process.stdout.close()
        process.stderr.close()


def _isolated_git(
    git: Path,
    repository: Path,
    arguments: list[str],
    *,
    maximum: int = MAX_CHECKED_BYTES,
) -> bytes:
    command = [
        os.fspath(git),
        "--no-replace-objects",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "credential.helper=",
        "-C",
        os.fspath(repository),
        *arguments,
    ]
    return_code, stdout, stderr = _run_bounded_process(
        command,
        environment=_isolated_git_environment(),
        timeout=60,
        stdout_limit=maximum,
        stderr_limit=MAX_CHECKED_BYTES,
        code="GIT_AUTHORITY_UNAVAILABLE",
    )
    if return_code != 0 or stderr:
        raise BuilderError("GIT_AUTHORITY_UNAVAILABLE")
    return stdout


def _isolated_git_text(
    git: Path, repository: Path, arguments: list[str], code: str
) -> str:
    try:
        raw = _isolated_git(git, repository, arguments)
        value = raw.decode("utf-8").strip()
    except (UnicodeDecodeError, BuilderError) as error:
        raise BuilderError(code) from error
    if not value or "\n" in value or "\r" in value:
        raise BuilderError(code)
    return value


def _output_target(path: Path, *, require_absent: bool = False) -> Path:
    absolute = path if path.is_absolute() else Path.cwd() / path
    try:
        parent = absolute.parent.resolve(strict=True)
    except OSError as error:
        raise BuilderError("PRIVATE_OUTPUT_INVALID") from error
    target = parent / absolute.name
    try:
        metadata = os.lstat(target)
    except FileNotFoundError:
        return target
    except OSError as error:
        raise BuilderError("PRIVATE_OUTPUT_INVALID") from error
    if require_absent or (
        stat.S_ISLNK(metadata.st_mode)
        or not stat.S_ISREG(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o600
        or metadata.st_uid != os.geteuid()
    ):
        raise BuilderError("PRIVATE_OUTPUT_INVALID")
    return target


def _experiment_root(path: Path) -> tuple[Path, str]:
    if not path.is_absolute():
        raise BuilderError("EXPERIMENT_ROOT_INVALID")
    try:
        lstat = os.lstat(path)
        resolved = path.resolve(strict=True)
    except OSError as error:
        raise BuilderError("EXPERIMENT_ROOT_INVALID") from error
    if (
        resolved != path
        or stat.S_ISLNK(lstat.st_mode)
        or not stat.S_ISDIR(lstat.st_mode)
        or stat.S_IMODE(lstat.st_mode) != 0o700
        or lstat.st_uid != os.geteuid()
    ):
        raise BuilderError("EXPERIMENT_ROOT_INVALID")
    expected = {
        "CODECLEW_HOME": resolved / "codeclew-state",
        "TMPDIR": resolved / "tmp",
    }
    identities: dict[str, dict[str, int]] = {}
    for key, child in expected.items():
        raw = os.environ.get(key)
        try:
            metadata = os.lstat(child)
            child_resolved = child.resolve(strict=True)
        except OSError as error:
            raise BuilderError("EXPERIMENT_ROOT_INVALID") from error
        if (
            raw != os.fspath(child)
            or child_resolved != child
            or stat.S_ISLNK(metadata.st_mode)
            or not stat.S_ISDIR(metadata.st_mode)
            or stat.S_IMODE(metadata.st_mode) != 0o700
            or metadata.st_uid != os.geteuid()
        ):
            raise BuilderError("EXPERIMENT_ROOT_INVALID")
        identities[key] = {"device": metadata.st_dev, "inode": metadata.st_ino}
    authority = {
        "device": lstat.st_dev,
        "inode": lstat.st_ino,
        "children": identities,
    }
    return resolved, authority_digest(authority)


def _require_private_children(root: Path, paths: list[Path]) -> None:
    for path in paths:
        absolute = path if path.is_absolute() else Path.cwd() / path
        try:
            parent = absolute.parent.resolve(strict=True)
        except OSError as error:
            raise BuilderError("EXPERIMENT_ROOT_INVALID") from error
        if parent != root or absolute.name in {"", ".", ".."}:
            raise BuilderError("EXPERIMENT_ROOT_INVALID")


def atomic_private_replace(path: Path, value: Any) -> None:
    target = _output_target(path)
    raw = canonical_bytes(value) + b"\n"
    temporary: str | None = None
    try:
        descriptor, temporary = tempfile.mkstemp(
            prefix=f".{target.name}.", dir=target.parent
        )
        with os.fdopen(descriptor, "wb", closefd=True) as stream:
            os.fchmod(stream.fileno(), 0o600)
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, target)
        temporary = None
        directory = os.open(target.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        metadata = os.stat(target)
        if stat.S_IMODE(metadata.st_mode) != 0o600 or metadata.st_uid != os.geteuid():
            raise BuilderError("PRIVATE_OUTPUT_WRITE_FAILED")
    except (OSError, BuilderError) as error:
        if temporary is not None:
            try:
                os.unlink(temporary)
            except OSError:
                pass
        if isinstance(error, BuilderError):
            raise
        raise BuilderError("PRIVATE_OUTPUT_WRITE_FAILED") from error


def _stage_private(path: Path, value: Any) -> tuple[Path, Path]:
    target = _output_target(path, require_absent=True)
    raw = canonical_bytes(value) + b"\n"
    temporary: str | None = None
    try:
        descriptor, temporary = tempfile.mkstemp(
            prefix=f".{target.name}.", dir=target.parent
        )
        with os.fdopen(descriptor, "wb", closefd=True) as stream:
            os.fchmod(stream.fileno(), 0o600)
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        return target, Path(temporary)
    except OSError as error:
        if temporary is not None:
            try:
                os.unlink(temporary)
            except OSError:
                pass
        raise BuilderError("PRIVATE_OUTPUT_WRITE_FAILED") from error


def _discard_staged(staged: list[tuple[Path, Path]]) -> None:
    for _, temporary in staged:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass
        except OSError:
            pass


def _publication_ledger_path(first_target: Path) -> Path:
    return first_target.with_name(f".{first_target.name}.pair-publication-pending.json")


def _create_private_once(
    path: Path,
    value: dict[str, Any],
    code: str = "PRIVATE_PUBLICATION_LEDGER_FAILED",
) -> None:
    raw = canonical_bytes(value) + b"\n"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags, 0o600)
        with os.fdopen(descriptor, "wb", closefd=True) as stream:
            os.fchmod(stream.fileno(), 0o600)
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except OSError as error:
        raise BuilderError(code) from error


def _publication_value(staged: list[tuple[Path, Path]]) -> dict[str, Any]:
    items: list[dict[str, Any]] = []
    for target, temporary in staged:
        metadata = os.lstat(temporary)
        items.append(
            {
                "target": os.fspath(target),
                "staged": os.fspath(temporary),
                "device": metadata.st_dev,
                "inode": metadata.st_ino,
                "size": metadata.st_size,
                "digest": file_digest(temporary),
            }
        )
    unsigned = {
        "schema": PUBLICATION_LEDGER_SCHEMA,
        "ownerPid": os.getpid(),
        "builderDigest": file_digest(Path(__file__).resolve(strict=True)),
        "items": items,
    }
    return {**unsigned, "authorityDigest": authority_digest(unsigned)}


def _validate_publication_value(
    value: Any, first_target: Path, second_target: Path
) -> list[dict[str, Any]]:
    if not isinstance(value, dict) or set(value) != {
        "schema", "authorityDigest", "ownerPid", "builderDigest", "items"
    }:
        raise BuilderError("PRIVATE_PUBLICATION_LEDGER_INVALID")
    unsigned = dict(value)
    declared = unsigned.pop("authorityDigest")
    items = value.get("items")
    if (
        value.get("schema") != PUBLICATION_LEDGER_SCHEMA
        or declared != authority_digest(unsigned)
        or type(value.get("ownerPid")) is not int
        or value["ownerPid"] <= 0
        or value.get("builderDigest") != file_digest(Path(__file__).resolve(strict=True))
        or not isinstance(items, list)
        or len(items) != 2
    ):
        raise BuilderError("PRIVATE_PUBLICATION_LEDGER_INVALID")
    expected_targets = [first_target, second_target]
    for item, target in zip(items, expected_targets, strict=True):
        if not isinstance(item, dict) or set(item) != {
            "target", "staged", "device", "inode", "size", "digest"
        }:
            raise BuilderError("PRIVATE_PUBLICATION_LEDGER_INVALID")
        staged = Path(item["staged"]) if isinstance(item.get("staged"), str) else Path()
        if (
            item.get("target") != os.fspath(target)
            or not staged.is_absolute()
            or staged.parent != target.parent
            or not staged.name.startswith(f".{target.name}.")
            or type(item.get("device")) is not int
            or type(item.get("inode")) is not int
            or type(item.get("size")) is not int
            or not 0 < item["size"] <= MAX_PRIVATE_BYTES
            or not isinstance(item.get("digest"), str)
            or SHA256.fullmatch(item["digest"]) is None
        ):
            raise BuilderError("PRIVATE_PUBLICATION_LEDGER_INVALID")
    return items


def _publication_file_present(path: Path, item: dict[str, Any]) -> bool:
    try:
        metadata = os.lstat(path)
    except FileNotFoundError:
        return False
    except OSError as error:
        raise BuilderError("PRIVATE_PUBLICATION_RECOVERY_FAILED") from error
    if (
        stat.S_ISLNK(metadata.st_mode)
        or not stat.S_ISREG(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o600
        or metadata.st_uid != os.geteuid()
        or metadata.st_dev != item["device"]
        or metadata.st_ino != item["inode"]
        or metadata.st_size != item["size"]
        or file_digest(path) != item["digest"]
    ):
        raise BuilderError("PRIVATE_PUBLICATION_RECOVERY_FAILED")
    return True


def recover_private_pair(first_path: Path, second_path: Path) -> None:
    first_target = _output_target(first_path)
    second_target = _output_target(second_path)
    ledger = _publication_ledger_path(first_target)
    try:
        os.lstat(ledger)
    except FileNotFoundError:
        return
    except OSError as error:
        raise BuilderError("PRIVATE_PUBLICATION_LEDGER_INVALID") from error
    _, value, _ = private_json(
        ledger, "PRIVATE_PUBLICATION_LEDGER_INVALID", 256 * 1024
    )
    items = _validate_publication_value(value, first_target, second_target)
    try:
        os.kill(value["ownerPid"], 0)
    except ProcessLookupError:
        pass
    except PermissionError as error:
        raise BuilderError("PRIVATE_PUBLICATION_ACTIVE") from error
    else:
        raise BuilderError("PRIVATE_PUBLICATION_ACTIVE")
    target_presence = [
        _publication_file_present(target, item)
        for target, item in zip([first_target, second_target], items, strict=True)
    ]
    if target_presence not in ([False, False], [True, True]):
        for present, target in zip(target_presence, [first_target, second_target], strict=True):
            if present:
                target.unlink()
    for item in items:
        staged = Path(item["staged"])
        if _publication_file_present(staged, item):
            staged.unlink()
    for parent in {first_target.parent, second_target.parent}:
        directory = os.open(parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    _remove_private_ledger(ledger)


def publish_private_pair(
    first_path: Path,
    first_value: Any,
    second_path: Path,
    second_value: Any,
) -> None:
    first_target = _output_target(first_path, require_absent=True)
    second_target = _output_target(second_path, require_absent=True)
    if first_target == second_target:
        raise BuilderError("PRIVATE_OUTPUT_INVALID")
    staged: list[tuple[Path, Path]] = []
    try:
        staged.append(_stage_private(first_target, first_value))
        staged.append(_stage_private(second_target, second_value))
    except BuilderError:
        _discard_staged(staged)
        raise
    ledger = _publication_ledger_path(first_target)
    publication = _publication_value(staged)
    _create_private_once(ledger, publication)
    try:
        for target, temporary in staged:
            os.link(temporary, target, follow_symlinks=False)
            temporary.unlink()
        for parent in {target.parent for target, _ in staged}:
            directory = os.open(parent, os.O_RDONLY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
    except OSError as error:
        # Preserve the exact 0600 ledger and every still-linked inode.  A
        # following invocation can then distinguish a complete pair from a
        # partial pair and recover it.  Best-effort unlink here could strand a
        # half-pair after deleting its only recovery authority.
        raise BuilderError("PRIVATE_OUTPUT_WRITE_FAILED") from error
    _discard_staged(staged)
    _remove_private_ledger(ledger)


def load_authorities(
    *,
    g1k_path: Path,
    clew_path: Path,
    git_path: Path,
    experiment_root_path: Path,
    corpus_path: Path | None = None,
    benchmark_path: Path | None = None,
) -> Authorities:
    if (corpus_path is None) != (benchmark_path is None):
        raise BuilderError("PRIVATE_AUTHORITIES_INCOMPLETE")
    _, g1k_value, _ = checked_json(g1k_path, "INVALID_G1K_EVIDENCE")
    try:
        g1k_verifier.verify_value(g1k_value)
    except g1k_verifier.EvidenceError as error:
        raise BuilderError("INVALID_G1K_EVIDENCE") from error
    g1k_digest = authority_digest(g1k_value)
    clew, runtime_digest = executable(clew_path)
    git = git_executable(git_path)
    git_digest = file_digest(git)
    _, experiment_root_digest = _experiment_root(experiment_root_path)
    if g1k_value["executionAuthority"]["clewAuthority"] != runtime_digest:
        raise BuilderError("G1K_RUNTIME_AUTHORITY_CHANGED")

    corpus = None
    corpus_value = None
    benchmark = None
    benchmark_value = None
    if corpus_path is not None and benchmark_path is not None:
        _, corpus_value, _ = private_json(
            corpus_path, "INVALID_PRIVATE_CORPUS", descriptor_gate.MAX_PRIVATE_CORPUS_BYTES
        )
        _, benchmark_value, benchmark_raw = private_json(
            benchmark_path,
            "INVALID_PRIVATE_BENCHMARK",
            descriptor_gate.MAX_PRIVATE_BENCHMARK_BYTES,
        )
        try:
            corpus = descriptor_gate.parse_corpus(corpus_value)
            corpus_digest = authority_digest(corpus_value)
            if corpus_digest != descriptor_gate.EXPECTED_CORPUS_DIGEST:
                raise BuilderError("INVALID_PRIVATE_CORPUS")
            benchmark = descriptor_gate.parse_benchmark(
                benchmark_value, benchmark_raw, corpus, corpus_digest
            )
            descriptor_gate.validate_oracle_files(git, corpus, benchmark)
        except descriptor_gate.GateError as error:
            raise BuilderError(error.code) from error
        selection = g1k_value["selectionAuthority"]
        if (
            selection["privateCorpusDigest"] != corpus_digest
            or selection["benchmarkDigest"] != benchmark.authority_digest
        ):
            raise BuilderError("G1K_SELECTION_AUTHORITY_CHANGED")
    return Authorities(
        corpus,
        corpus_value,
        benchmark,
        benchmark_value,
        g1k_value,
        g1k_digest,
        clew,
        runtime_digest,
        git,
        git_digest,
        experiment_root_digest,
    )


def _source_repository(git: Path) -> Path:
    candidate = Path(__file__).resolve().parent.parent
    root = _isolated_git_text(
        git, candidate, ["rev-parse", "--show-toplevel"], "PUBLIC_FIXTURE_INVALID"
    )
    try:
        resolved = Path(root).resolve(strict=True)
    except OSError as error:
        raise BuilderError("PUBLIC_FIXTURE_INVALID") from error
    if resolved != candidate:
        raise BuilderError("PUBLIC_FIXTURE_INVALID")
    return resolved


def load_sealed_public_fixture(git: Path) -> SealedFixture:
    repository = _source_repository(git)
    tree_oid = _isolated_git_text(
        git,
        repository,
        ["rev-parse", "--verify", "HEAD:fixtures/kotlin-basic"],
        "PUBLIC_FIXTURE_INVALID",
    )
    if GIT_OID.fullmatch(tree_oid) is None:
        raise BuilderError("PUBLIC_FIXTURE_INVALID")
    raw_tree = _isolated_git(
        git,
        repository,
        ["ls-tree", "-r", "-z", "--full-tree", "HEAD", "--", "fixtures/kotlin-basic"],
        maximum=4 * 1024 * 1024,
    )
    files: list[tuple[str, int, str, bytes]] = []
    authority_rows: list[dict[str, Any]] = []
    prefix = "fixtures/kotlin-basic/"
    for raw_row in raw_tree.split(b"\0"):
        if not raw_row:
            continue
        try:
            header, raw_path = raw_row.split(b"\t", 1)
            mode_raw, kind, oid_raw = header.split(b" ", 2)
            path = raw_path.decode("utf-8")
            mode_text = mode_raw.decode("ascii")
            oid = oid_raw.decode("ascii")
        except (ValueError, UnicodeDecodeError) as error:
            raise BuilderError("PUBLIC_FIXTURE_INVALID") from error
        if (
            kind != b"blob"
            or mode_text not in {"100644", "100755"}
            or GIT_OID.fullmatch(oid) is None
            or not path.startswith(prefix)
        ):
            raise BuilderError("PUBLIC_FIXTURE_INVALID")
        relative = path.removeprefix(prefix)
        if not relative or relative.startswith("/") or any(
            part in {"", ".", ".."} for part in relative.split("/")
        ):
            raise BuilderError("PUBLIC_FIXTURE_INVALID")
        content = _isolated_git(
            git,
            repository,
            ["cat-file", "blob", oid],
            maximum=MAX_GIT_BLOB_BYTES,
        )
        observed_oid = _isolated_git_text(
            git,
            repository,
            ["rev-parse", "--verify", f"HEAD:{path}"],
            "PUBLIC_FIXTURE_INVALID",
        )
        if observed_oid != oid:
            raise BuilderError("PUBLIC_FIXTURE_INVALID")
        mode = 0o755 if mode_text == "100755" else 0o644
        files.append((relative, mode, oid, content))
        authority_rows.append(
            {
                "relativeFile": relative,
                "mode": mode_text,
                "blobOid": oid,
                "size": len(content),
                "contentDigest": f"sha256:{hashlib.sha256(content).hexdigest()}",
            }
        )
    if (
        not files
        or len({row[0] for row in files}) != len(files)
        or "src/main/kotlin/com/acme/RelationFacts.kt" not in {row[0] for row in files}
    ):
        raise BuilderError("PUBLIC_FIXTURE_INVALID")
    files.sort(key=lambda row: row[0])
    authority_rows.sort(key=lambda row: row["relativeFile"])
    return SealedFixture(tree_oid, authority_digest(authority_rows), tuple(files))


def _fast_test_digest() -> str:
    return authority_digest(self_test())


def local_module_manifest() -> dict[str, Any]:
    directory = Path(__file__).resolve(strict=True).parent
    modules = [
        {
            "module": module,
            "digest": file_digest(directory / f"{module}.py"),
        }
        for module in sorted(
            {
                "run_thread_kotlin_descriptor_gate",
                "verify_thread_kotlin_descriptor_gate",
                "verify_thread_kotlin_pilot",
            }
        )
    ]
    unsigned = {
        "schema": "codeclew-kotlin-pilot-local-modules/1.0",
        "modules": modules,
        "testFileDigest": file_digest(
            directory / "test_run_thread_kotlin_pilot.py"
        ),
    }
    return {**unsigned, "authorityDigest": authority_digest(unsigned)}


def validate_review_manifest_value(
    value: Any,
    *,
    builder_digest: str,
    pilot_runner_digest: str,
    g1k_digest: str,
    fixture: SealedFixture,
    test_digest: str,
    module_manifest: dict[str, Any],
    git_digest: str,
    git_environment_digest: str,
) -> tuple[dict[str, Any], str]:
    root = closed(
        value,
        {
            "schema",
            "authorityDigest",
            "builderDigest",
            "pilotRunnerDigest",
            "g1kEvidenceDigest",
            "publicFixtureTreeOid",
            "publicFixtureContentDigest",
            "testDigest",
            "localModuleManifest",
            "gitDigest",
            "gitEnvironmentDigest",
            "verdict",
            "findings",
        },
        "INVALID_INDEPENDENT_REVIEW",
    )
    unsigned = dict(root)
    declared = unsigned.pop("authorityDigest")
    if (
        root["schema"] != REVIEW_SCHEMA
        or declared != authority_digest(unsigned)
        or root["builderDigest"] != builder_digest
        or root["pilotRunnerDigest"] != pilot_runner_digest
        or root["g1kEvidenceDigest"] != g1k_digest
        or root["publicFixtureTreeOid"] != fixture.tree_oid
        or root["publicFixtureContentDigest"] != fixture.content_digest
        or root["testDigest"] != test_digest
        or root["localModuleManifest"] != module_manifest
        or root["gitDigest"] != git_digest
        or root["gitEnvironmentDigest"] != git_environment_digest
        or root["verdict"] != "PASS"
        or root["findings"] != []
    ):
        raise BuilderError("INVALID_INDEPENDENT_REVIEW")
    digest(declared, "INVALID_INDEPENDENT_REVIEW")
    return root, declared


def load_review_authority(
    path: Path,
    authorities: Authorities,
    pilot_runner_path: Path,
    fixture: SealedFixture,
) -> ReviewAuthority:
    _, value, _ = checked_json(path, "INVALID_INDEPENDENT_REVIEW", 256 * 1024)
    pilot_runner, pilot_runner_digest = regular_file(
        pilot_runner_path, "INVALID_PILOT_RUNNER"
    )
    test_digest = _fast_test_digest()
    module_manifest = local_module_manifest()
    root, declared = validate_review_manifest_value(
        value,
        builder_digest=file_digest(Path(__file__).resolve()),
        pilot_runner_digest=pilot_runner_digest,
        g1k_digest=authorities.g1k_digest,
        fixture=fixture,
        test_digest=test_digest,
        module_manifest=module_manifest,
        git_digest=authorities.git_digest,
        git_environment_digest=authority_digest(_isolated_git_environment()),
    )
    return ReviewAuthority(
        root, declared, pilot_runner, pilot_runner_digest, test_digest
    )


def _bounded_string(value: Any, code: str, maximum: int = 4096) -> str:
    if (
        not isinstance(value, str)
        or not value
        or len(value.encode("utf-8")) > maximum
        or "\0" in value
    ):
        raise BuilderError(code)
    return value


def _typed_value(
    value: Any, type_field: str, nullable_field: str, fields: set[str], code: str
) -> None:
    row = closed(value, fields, code)
    rendered = _bounded_string(row.get(type_field), code)
    nullable = row.get(nullable_field)
    if (
        type(nullable) is not bool
        or ".." in rendered
        or "!" in rendered
        or "<ERROR" in rendered
        or nullable != rendered.rstrip().endswith("?")
    ):
        raise BuilderError(code)


def validate_projected_shape(value: Any) -> dict[str, Any]:
    """Validate the complete compiler projection and its kind-specific closure."""

    code = "INVALID_COMPILER_PROJECTION"
    if not isinstance(value, dict):
        raise BuilderError(code)
    kind = value.get("declarationKind")
    base = {
        "symbolIdentity",
        "declarationKind",
        "ownerIdentity",
        "containment",
        "visibility",
        "effectiveVisibility",
        "exportBoundary",
        "modality",
        "typeParameters",
    }
    additions = {
        "FUNCTION": {
            "compilerCallableId",
            "isOverride",
            "returnType",
            "returnNullable",
            "parameterTypes",
        },
        "CONSTRUCTOR": {
            "compilerCallableId",
            "compilerClassId",
            "isPrimary",
            "jvmDescriptor",
            "parameterTypes",
        },
        "PROPERTY": {
            "compilerCallableId",
            "isOverride",
            "declaredType",
            "declaredNullable",
        },
        "MUTABLE_PROPERTY": {
            "compilerCallableId",
            "isOverride",
            "declaredType",
            "declaredNullable",
        },
        "CLASS": {"compilerClassId"},
    }
    if kind not in additions:
        raise BuilderError(code)
    allowed = base | additions[kind]
    if kind == "FUNCTION" and "receiverType" in value:
        allowed.add("receiverType")
    if set(value) != allowed:
        raise BuilderError(code)

    identity = _bounded_string(value["symbolIdentity"], code)
    owner = _bounded_string(value["ownerIdentity"], code)
    containment = value["containment"]
    if (
        not isinstance(containment, list)
        or len(containment) > 256
        or any(not isinstance(item, str) or not item for item in containment)
        or (
            containment
            and containment[-1] != owner
        )
        or (not containment and not owner.startswith("package:"))
    ):
        raise BuilderError(code)
    visibility = value["visibility"]
    effective = value["effectiveVisibility"]
    export = value["exportBoundary"]
    if (
        visibility not in {"public", "internal", "private", "protected"}
        or effective
        not in {
            "public",
            "internal",
            "private-in-class",
            "private-in-file",
            "protected",
        }
        or export not in {"PUBLIC_API", "MODULE_API", "PRIVATE_API"}
        or value["modality"] not in {"FINAL", "OPEN", "ABSTRACT", "SEALED"}
    ):
        raise BuilderError(code)
    expected_export = (
        "PUBLIC_API"
        if effective in {"public", "protected"}
        else "MODULE_API"
        if effective == "internal"
        else "PRIVATE_API"
    )
    if export != expected_export:
        raise BuilderError(code)

    parameters = value["typeParameters"]
    if not isinstance(parameters, list) or len(parameters) > 256:
        raise BuilderError(code)
    for index, parameter in enumerate(parameters):
        row = closed(parameter, {"index", "compilerName", "bounds"}, code)
        bounds = row["bounds"]
        if (
            row["index"] != index
            or not isinstance(row["compilerName"], str)
            or not row["compilerName"]
            or not isinstance(bounds, list)
            or len(bounds) > 64
            or any(not isinstance(bound, str) or not bound for bound in bounds)
            or bounds != sorted(set(bounds))
        ):
            raise BuilderError(code)

    if kind == "FUNCTION":
        callable_id = _bounded_string(value["compilerCallableId"], code)
        prefix = f"callable:{callable_id}#jvm:"
        if (
            not identity.startswith(prefix)
            or len(identity) == len(prefix)
            or type(value["isOverride"]) is not bool
        ):
            raise BuilderError(code)
        _typed_value(value, "returnType", "returnNullable", set(value), code)
        call_parameters = value["parameterTypes"]
        if not isinstance(call_parameters, list) or len(call_parameters) > 1024:
            raise BuilderError(code)
        for index, parameter in enumerate(call_parameters):
            _typed_value(parameter, "type", "nullable", {"index", "type", "nullable"}, code)
            if parameter["index"] != index:
                raise BuilderError(code)
        if "receiverType" in value:
            _typed_value(value["receiverType"], "type", "nullable", {"type", "nullable"}, code)
    elif kind == "CONSTRUCTOR":
        callable_id = _bounded_string(value["compilerCallableId"], code)
        class_id = _bounded_string(value["compilerClassId"], code)
        jvm = _bounded_string(value["jvmDescriptor"], code)
        if (
            identity != f"constructor:{callable_id}#jvm:{jvm}"
            or owner != f"class:{class_id}"
            or type(value["isPrimary"]) is not bool
            or not jvm.startswith("(")
            or ")" not in jvm
        ):
            raise BuilderError(code)
        call_parameters = value["parameterTypes"]
        if not isinstance(call_parameters, list) or len(call_parameters) > 1024:
            raise BuilderError(code)
        for index, parameter in enumerate(call_parameters):
            _typed_value(parameter, "type", "nullable", {"index", "type", "nullable"}, code)
            if parameter["index"] != index:
                raise BuilderError(code)
    elif kind in {"PROPERTY", "MUTABLE_PROPERTY"}:
        callable_id = _bounded_string(value["compilerCallableId"], code)
        if identity != f"property:{callable_id}" or type(value["isOverride"]) is not bool:
            raise BuilderError(code)
        _typed_value(value, "declaredType", "declaredNullable", set(value), code)
    else:
        class_id = _bounded_string(value["compilerClassId"], code)
        if identity != f"class:{class_id}":
            raise BuilderError(code)
    return value


def _compiler_simple_name(projected: dict[str, Any]) -> str:
    kind = projected["declarationKind"]
    if kind == "CLASS":
        raw = projected["compilerClassId"]
    else:
        raw = projected.get("compilerCallableId")
    if not isinstance(raw, str):
        raise BuilderError("INVALID_COMPILER_PROJECTION")
    name = re.split(r"[./$]", raw)[-1]
    if SAFE_IDENTIFIER.fullmatch(name) is None:
        raise BuilderError("INVALID_COMPILER_PROJECTION")
    return name


def _source_anchor(value: Any) -> dict[str, Any]:
    row = closed(value, {"path", "start", "end", "contentRef"}, "INVALID_COMPILER_PROJECTION")
    try:
        descriptor_gate.safe_relative_kotlin_file(row["path"])
    except descriptor_gate.GateError as error:
        raise BuilderError("INVALID_COMPILER_PROJECTION") from error
    if (
        type(row["start"]) is not int
        or type(row["end"]) is not int
        or not 0 <= row["start"] < row["end"]
    ):
        raise BuilderError("INVALID_COMPILER_PROJECTION")
    cas_reference(
        row["contentRef"],
        "codeclew-repository-input-blob/2.0",
        "INVALID_COMPILER_PROJECTION",
    )
    return row


def declaration_from_finding(
    value: Any, *, member_alias: str, exact: bool
) -> CompilerDeclaration | None:
    if not isinstance(value, dict):
        raise BuilderError("INVALID_COMPILER_PROJECTION")
    detail = value.get("detail")
    if not isinstance(detail, dict) or detail.get("kind") != "DECLARATION":
        return None
    finding = closed(
        value,
        {
            "findingId",
            "side",
            "memberAlias",
            "factId",
            "authority",
            "shapeDigest",
            "source",
            "detail",
        },
        "INVALID_COMPILER_PROJECTION",
    )
    if finding["memberAlias"] != member_alias:
        return None
    if finding["side"] != member_alias.upper():
        raise BuilderError("INVALID_COMPILER_PROJECTION")
    if exact and finding["authority"] != "EXACT_PROJECTED_DECLARATION":
        return None
    digest(finding["findingId"], "INVALID_COMPILER_PROJECTION")
    digest(finding["factId"], "INVALID_COMPILER_PROJECTION")
    shape_digest = digest(finding["shapeDigest"], "INVALID_COMPILER_PROJECTION")
    source = _source_anchor(finding["source"])
    detail = closed(detail, {"kind", "detail"}, "INVALID_COMPILER_PROJECTION")
    declaration = detail["detail"]
    if not isinstance(declaration, dict):
        raise BuilderError("INVALID_COMPILER_PROJECTION")
    kind = declaration.get("declarationKind")
    detail_fields = {"declarationKind", "symbolIdentity", "projectedShape"}
    if kind == "CLASS":
        detail_fields.add("compilerClassId")
    elif kind in {"FUNCTION", "PROPERTY", "MUTABLE_PROPERTY"}:
        detail_fields.add("compilerCallableId")
    elif kind == "CONSTRUCTOR":
        detail_fields.update({"compilerCallableId", "compilerClassId"})
    else:
        raise BuilderError("INVALID_COMPILER_PROJECTION")
    closed(declaration, detail_fields, "INVALID_COMPILER_PROJECTION")
    projected = validate_projected_shape(declaration["projectedShape"])
    if (
        projected["declarationKind"] != kind
        or projected["symbolIdentity"] != declaration["symbolIdentity"]
        or declaration.get("compilerCallableId") != projected.get("compilerCallableId")
        or declaration.get("compilerClassId") != projected.get("compilerClassId")
        or authority_digest(projected) != shape_digest
    ):
        raise BuilderError("INVALID_COMPILER_PROJECTION")
    symbol_identity = projected["symbolIdentity"]
    if not isinstance(symbol_identity, str) or ":" not in symbol_identity:
        raise BuilderError("INVALID_COMPILER_PROJECTION")
    return CompilerDeclaration(
        member_alias=member_alias,
        side=finding["side"],
        symbol_identity=symbol_identity,
        compiler_name=_compiler_simple_name(projected),
        declaration_kind=kind,
        descriptor_class="TYPE" if kind == "CLASS" else "CALLABLE",
        projected_shape=projected,
        shape_digest=shape_digest,
        source=source,
    )


def validate_exact_row(value: Any) -> dict[str, Any]:
    row = closed(value, EXACT_FIELDS, "INVALID_SHAPE_ORACLE")
    descriptor_class = row["descriptorClass"]
    declaration_kind = row["declarationKind"]
    if (
        descriptor_class not in {"CALLABLE", "TYPE"}
        or declaration_kind
        not in {"FUNCTION", "CONSTRUCTOR", "CLASS", "PROPERTY", "MUTABLE_PROPERTY"}
        or (descriptor_class == "TYPE") != (declaration_kind == "CLASS")
        or not isinstance(row["name"], str)
        or SAFE_IDENTIFIER.fullmatch(row["name"]) is None
    ):
        raise BuilderError("INVALID_SHAPE_ORACLE")
    _bounded_string(row["ownerIdentity"], "INVALID_SHAPE_ORACLE")
    signature = _bounded_string(row["normalizedSignature"], "INVALID_SHAPE_ORACLE")
    digest(row["shapeDigest"], "INVALID_SHAPE_ORACLE")
    try:
        descriptor_gate.safe_relative_kotlin_file(row["relativeFile"])
    except descriptor_gate.GateError as error:
        raise BuilderError("INVALID_SHAPE_ORACLE") from error
    if not isinstance(row["blobOid"], str) or GIT_OID.fullmatch(row["blobOid"]) is None:
        raise BuilderError("INVALID_SHAPE_ORACLE")
    source = closed(
        row["sourceRange"], {"startByte", "endByte"}, "INVALID_SHAPE_ORACLE"
    )
    if (
        type(source["startByte"]) is not int
        or type(source["endByte"]) is not int
        or not 0 <= source["startByte"] < source["endByte"]
    ):
        raise BuilderError("INVALID_SHAPE_ORACLE")
    if declaration_kind == "FUNCTION":
        if "#jvm:" not in signature or _simple_name_from_signature(signature) != row["name"]:
            raise BuilderError("INVALID_SHAPE_ORACLE")
    elif declaration_kind == "CONSTRUCTOR":
        if "#jvm:" not in signature:
            raise BuilderError("INVALID_SHAPE_ORACLE")
    elif declaration_kind in {"PROPERTY", "MUTABLE_PROPERTY"}:
        if "#jvm:" in signature or _simple_name_from_signature(signature) != row["name"]:
            raise BuilderError("INVALID_SHAPE_ORACLE")
    elif signature != signature.strip() or _simple_name_from_signature(signature) != row["name"]:
        raise BuilderError("INVALID_SHAPE_ORACLE")
    return row


def _simple_name_from_signature(signature: str) -> str:
    raw = signature.split("#jvm:", 1)[0]
    return re.split(r"[./$]", raw)[-1]


def _manual_categories(
    benchmark_value: dict[str, Any], task_row: dict[str, Any]
) -> list[dict[str, str]]:
    try:
        values = benchmark_value["manualVerificationProfiles"][
            task_row["manualCategoryProfile"]
        ]
    except (KeyError, TypeError) as error:
        raise BuilderError("INVALID_PRIVATE_BENCHMARK") from error
    if not isinstance(values, list):
        raise BuilderError("INVALID_PRIVATE_BENCHMARK")
    rows: list[dict[str, str]] = []
    for value in values:
        if not isinstance(value, str) or SAFE_CATEGORY.fullmatch(value) is None:
            raise BuilderError("INVALID_PRIVATE_BENCHMARK")
        required = value if value.startswith("VERIFY_") else f"VERIFY_{value}"
        category = value.removeprefix("VERIFY_")
        if SAFE_CATEGORY.fullmatch(category) is None or SAFE_REQUIRED_CHECK.fullmatch(required) is None:
            raise BuilderError("INVALID_PRIVATE_BENCHMARK")
        rows.append({"category": category, "requiredCheck": required})
    rows.sort(key=lambda row: (row["category"], row["requiredCheck"]))
    if len({(row["category"], row["requiredCheck"]) for row in rows}) != len(rows):
        raise BuilderError("INVALID_PRIVATE_BENCHMARK")
    return rows


def _validate_manual(value: Any) -> list[dict[str, str]]:
    if not isinstance(value, list) or not value:
        raise BuilderError("INVALID_SHAPE_ORACLE")
    rows: list[dict[str, str]] = []
    for item in value:
        row = closed(item, {"category", "requiredCheck"}, "INVALID_SHAPE_ORACLE")
        if (
            not isinstance(row["category"], str)
            or SAFE_CATEGORY.fullmatch(row["category"]) is None
            or not isinstance(row["requiredCheck"], str)
            or SAFE_REQUIRED_CHECK.fullmatch(row["requiredCheck"]) is None
            or row["requiredCheck"] != f"VERIFY_{row['category']}"
        ):
            raise BuilderError("INVALID_SHAPE_ORACLE")
        rows.append(row)
    if rows != sorted(rows, key=lambda row: (row["category"], row["requiredCheck"])):
        raise BuilderError("INVALID_SHAPE_ORACLE")
    if len({(row["category"], row["requiredCheck"]) for row in rows}) != len(rows):
        raise BuilderError("INVALID_SHAPE_ORACLE")
    return rows


def validate_shape_oracle(
    value: Any,
    authorities: Authorities,
    *,
    projection_rows: dict[str, dict[str, Any]] | None = None,
) -> dict[str, Any]:
    root = closed(
        value,
        {"schema", "authorityDigest", "sourceAuthority", "fixture", "tasks"},
        "INVALID_SHAPE_ORACLE",
    )
    unsigned = dict(root)
    declared = unsigned.pop("authorityDigest")
    if root["schema"] != SHAPE_SCHEMA or declared != authority_digest(unsigned):
        raise BuilderError("INVALID_SHAPE_ORACLE")
    source = closed(
        root["sourceAuthority"],
        {
            "privateCorpusDigest",
            "benchmarkDigest",
            "g1kEvidenceDigest",
            "runtimeDigest",
            "runtimeKey",
            "gitDigest",
            "gitEnvironmentDigest",
            "localModuleManifestDigest",
            "compilerEnvironmentDigest",
            "compilerProjectionSchema",
            "publicFixtureTreeOid",
            "publicFixtureContentDigest",
        },
        "INVALID_SHAPE_ORACLE",
    )
    fixture_authority = load_sealed_public_fixture(authorities.git)
    if source != {
        "privateCorpusDigest": descriptor_gate.EXPECTED_CORPUS_DIGEST,
        "benchmarkDigest": descriptor_gate.EXPECTED_BENCHMARK_DIGEST,
        "g1kEvidenceDigest": authorities.g1k_digest,
        "runtimeDigest": authorities.runtime_digest,
        "runtimeKey": source["runtimeKey"],
        "gitDigest": authorities.git_digest,
        "gitEnvironmentDigest": authority_digest(_isolated_git_environment()),
        "localModuleManifestDigest": local_module_manifest()["authorityDigest"],
        "compilerEnvironmentDigest": authority_digest(_clew_environment()),
        "compilerProjectionSchema": PROJECTION_SCHEMA,
        "publicFixtureTreeOid": fixture_authority.tree_oid,
        "publicFixtureContentDigest": fixture_authority.content_digest,
    }:
        raise BuilderError("INVALID_SHAPE_ORACLE")
    digest(source["runtimeKey"], "INVALID_SHAPE_ORACLE")
    digest(source["compilerEnvironmentDigest"], "INVALID_SHAPE_ORACLE")

    fixture = root["fixture"]
    if not isinstance(fixture, list) or len(fixture) != 5:
        raise BuilderError("INVALID_SHAPE_ORACLE")
    for row in fixture:
        validate_exact_row(row)
        if projection_rows is not None:
            projected = projection_rows.get(authority_digest(row))
            if projected is None:
                raise BuilderError("UNATTESTED_COMPILER_PROJECTION")
            validate_projected_shape(projected)
            if authority_digest(projected) != row["shapeDigest"]:
                raise BuilderError("UNATTESTED_COMPILER_PROJECTION")
    if [row["name"] for row in fixture] != [
        "publicDescriptor",
        "overloadedDescriptor",
        "overloadedDescriptor",
        "genericDescriptor",
        "Envelope",
    ] or [row["declarationKind"] for row in fixture] != [
        "FUNCTION",
        "FUNCTION",
        "FUNCTION",
        "FUNCTION",
        "CLASS",
    ]:
        raise BuilderError("INVALID_SHAPE_ORACLE")
    if len({authority_digest(row) for row in fixture}) != 5:
        raise BuilderError("INVALID_SHAPE_ORACLE")

    tasks = root["tasks"]
    if not isinstance(tasks, list) or len(tasks) != 10:
        raise BuilderError("INVALID_SHAPE_ORACLE")
    g1k_tasks = authorities.g1k_value["tasks"]
    corpus_tasks = (
        {task.task_id: task for task in authorities.corpus.tasks}
        if authorities.corpus is not None
        else {}
    )
    benchmark_tasks = (
        {row["taskId"]: row for row in authorities.benchmark_value["tasks"]}
        if authorities.benchmark_value is not None
        else {}
    )
    side_oracles = (
        {(side.task_id, side.role): side for side in authorities.benchmark.sides}
        if authorities.benchmark is not None
        else {}
    )
    manual_count = 0
    for index, (row, g1k_task) in enumerate(zip(tasks, g1k_tasks, strict=True), 1):
        task = closed(
            row,
            {"taskId", "pairId", "manualVerification", "sides"},
            "INVALID_SHAPE_ORACLE",
        )
        expected_task_id = f"task-{index:02}"
        if task["taskId"] != expected_task_id or task["pairId"] != g1k_task["pairId"]:
            raise BuilderError("INVALID_SHAPE_ORACLE")
        manual = _validate_manual(task["manualVerification"])
        manual_count += len(manual)
        if authorities.benchmark_value is not None:
            expected_manual = _manual_categories(
                authorities.benchmark_value, benchmark_tasks[expected_task_id]
            )
            if manual != expected_manual:
                raise BuilderError("INVALID_SHAPE_ORACLE")
        sides = task["sides"]
        if not isinstance(sides, list) or len(sides) != 2:
            raise BuilderError("INVALID_SHAPE_ORACLE")
        slot_classes: set[str] = set()
        for role, side in zip(("provider", "consumer"), sides, strict=True):
            side = closed(
                side,
                {
                    "role",
                    "serviceAlias",
                    "revision",
                    "approvedFiles",
                    "exactDeclarations",
                },
                "INVALID_SHAPE_ORACLE",
            )
            expected_alias = g1k_task[role]
            if (
                side["role"] != role
                or side["serviceAlias"] != expected_alias
                or not isinstance(side["revision"], str)
                or GIT_OID.fullmatch(side["revision"]) is None
            ):
                raise BuilderError("INVALID_SHAPE_ORACLE")
            approved = side["approvedFiles"]
            if not isinstance(approved, list) or not approved:
                raise BuilderError("INVALID_SHAPE_ORACLE")
            approved_set: set[tuple[str, str]] = set()
            for approved_row in approved:
                approved_row = closed(
                    approved_row, {"relativeFile", "blobOid"}, "INVALID_SHAPE_ORACLE"
                )
                try:
                    descriptor_gate.safe_relative_kotlin_file(approved_row["relativeFile"])
                except descriptor_gate.GateError as error:
                    raise BuilderError("INVALID_SHAPE_ORACLE") from error
                if (
                    not isinstance(approved_row["blobOid"], str)
                    or GIT_OID.fullmatch(approved_row["blobOid"]) is None
                    or (approved_row["relativeFile"], approved_row["blobOid"]) in approved_set
                ):
                    raise BuilderError("INVALID_SHAPE_ORACLE")
                approved_set.add((approved_row["relativeFile"], approved_row["blobOid"]))
            declarations = side["exactDeclarations"]
            if not isinstance(declarations, list) or not declarations:
                raise BuilderError("INVALID_SHAPE_ORACLE")
            declaration_ids: set[str] = set()
            for declaration in declarations:
                validate_exact_row(declaration)
                if (
                    declaration["relativeFile"], declaration["blobOid"]
                ) not in approved_set:
                    raise BuilderError("INVALID_SHAPE_ORACLE")
                identity = authority_digest(declaration)
                if identity in declaration_ids:
                    raise BuilderError("INVALID_SHAPE_ORACLE")
                declaration_ids.add(identity)
                slot_classes.add(declaration["descriptorClass"])
                if projection_rows is not None:
                    projected = projection_rows.get(identity)
                    if projected is None:
                        raise BuilderError("UNATTESTED_COMPILER_PROJECTION")
                    validate_projected_shape(projected)
                    if authority_digest(projected) != declaration["shapeDigest"]:
                        raise BuilderError("UNATTESTED_COMPILER_PROJECTION")
            if authorities.benchmark is not None and authorities.corpus is not None:
                oracle = side_oracles[(expected_task_id, role)]
                expected_approved = [
                    {"relativeFile": item.relative_file, "blobOid": item.blob_oid}
                    for item in oracle.navigations
                ]
                allowed = {
                    (item.kind, item.name)
                    for navigation in oracle.navigations
                    for item in navigation.declarations
                }
                if (
                    side["revision"] != oracle.revision
                    or side["serviceAlias"]
                    != getattr(corpus_tasks[expected_task_id], role)
                    or approved != expected_approved
                ):
                    raise BuilderError("INVALID_SHAPE_ORACLE")
                for declaration in declarations:
                    oracle_kind = (
                        "FUN"
                        if declaration["declarationKind"] == "FUNCTION"
                        else declaration["declarationKind"]
                    )
                    if (oracle_kind, declaration["name"]) not in allowed and not (
                        declaration["descriptorClass"] == "TYPE"
                        and any(name == declaration["name"] for _, name in allowed)
                    ):
                        raise BuilderError("INVALID_SHAPE_ORACLE")
        if slot_classes != {"CALLABLE", "TYPE"}:
            raise BuilderError("INVALID_SHAPE_ORACLE")
    if manual_count != 74:
        raise BuilderError("INVALID_SHAPE_ORACLE")
    if projection_rows is not None:
        all_rows = fixture + [
            declaration
            for task in tasks
            for side in task["sides"]
            for declaration in side["exactDeclarations"]
        ]
        if set(projection_rows) != {authority_digest(row) for row in all_rows}:
            raise BuilderError("UNATTESTED_COMPILER_PROJECTION")
    return root


def _clew_environment(source: dict[str, str] | None = None) -> dict[str, str]:
    ambient = os.environ if source is None else source
    environment: dict[str, str] = {}
    for key in sorted(CLEW_ENV_ALLOW - {"PATH"}):
        value = ambient.get(key)
        if value is not None:
            if not isinstance(value, str) or not value or "\0" in value:
                raise BuilderError("COMPILER_ENVIRONMENT_INVALID")
            if key in CLEW_ENV_PATHS:
                path = Path(value)
                if not path.is_absolute():
                    raise BuilderError("COMPILER_ENVIRONMENT_INVALID")
                value = os.fspath(path.resolve(strict=False))
            environment[key] = value
    if "HOME" not in environment:
        raise BuilderError("COMPILER_ENVIRONMENT_INVALID")
    python = Path(sys.executable).resolve(strict=True)
    environment["PATH"] = f"{python.parent}:/usr/bin:/bin"
    environment["LANG"] = "C"
    environment["LC_ALL"] = "C"
    return environment


def _run_json(clew: Path, arguments: list[str], timeout_seconds: int) -> dict[str, Any]:
    return_code, raw, stderr = _run_bounded_process(
        [os.fspath(clew), "--json", *arguments],
        environment=_clew_environment(),
        timeout=timeout_seconds,
        stdout_limit=descriptor_gate.MAX_CLEW_STDOUT_BYTES,
        stderr_limit=descriptor_gate.MAX_CLEW_STDOUT_BYTES,
        code="COMPILER_EVIDENCE_FAILED",
    )
    if return_code != 0 or stderr or not raw:
        raise BuilderError("COMPILER_EVIDENCE_FAILED")
    try:
        return _json_value(raw, "COMPILER_EVIDENCE_FAILED")
    except BuilderError as error:
        raise BuilderError("COMPILER_EVIDENCE_FAILED") from error


def _validate_thread_closed(value: Any, thread_id: str) -> None:
    root = closed(
        value, {"schema", "threadId", "lifecycle"}, "SEMANTIC_CLEANUP_FAILED"
    )
    lifecycle = root["lifecycle"]
    if not isinstance(lifecycle, dict):
        raise BuilderError("SEMANTIC_CLEANUP_FAILED")
    closed(
        lifecycle,
        {
            "schema",
            "threadId",
            "threadAuthorityDigest",
            "sequence",
            "previousEventHash",
            "status",
            "eventHash",
            "updatedUnixMs",
        },
        "SEMANTIC_CLEANUP_FAILED",
    )
    if (
        root["schema"] != "codeclew-thread-lifecycle-result/1.0"
        or root["threadId"] != thread_id
        or lifecycle.get("schema") != "codeclew-thread-lifecycle-entry/1.0"
        or lifecycle.get("threadId") != thread_id
        or lifecycle.get("status") != "CLOSED"
        or SHA256.fullmatch(str(lifecycle.get("threadAuthorityDigest"))) is None
        or type(lifecycle.get("sequence")) is not int
        or lifecycle["sequence"] < 1
        or SHA256.fullmatch(str(lifecycle.get("eventHash"))) is None
        or type(lifecycle.get("updatedUnixMs")) is not int
        or lifecycle["updatedUnixMs"] < 0
    ):
        raise BuilderError("SEMANTIC_CLEANUP_FAILED")


def _validate_session_aborted(value: Any, session_id: str) -> None:
    root = closed(
        value, {"schema", "lifecycle"}, "SEMANTIC_CLEANUP_FAILED"
    )
    lifecycle = root["lifecycle"]
    if not isinstance(lifecycle, dict):
        raise BuilderError("SEMANTIC_CLEANUP_FAILED")
    closed(
        lifecycle,
        {
            "schema",
            "sessionId",
            "sessionAuthorityDigest",
            "sequence",
            "previousEventHash",
            "status",
            "eventHash",
            "updatedUnixMs",
        },
        "SEMANTIC_CLEANUP_FAILED",
    )
    if (
        root["schema"] != "codeclew-session-lifecycle-result/1.0"
        or lifecycle.get("schema") != "codeclew-session-lifecycle-entry/1.0"
        or lifecycle.get("sessionId") != session_id
        or lifecycle.get("status") != "ABORTED"
        or SHA256.fullmatch(str(lifecycle.get("sessionAuthorityDigest"))) is None
        or type(lifecycle.get("sequence")) is not int
        or lifecycle["sequence"] < 1
        or SHA256.fullmatch(str(lifecycle.get("eventHash"))) is None
        or type(lifecycle.get("updatedUnixMs")) is not int
        or lifecycle["updatedUnixMs"] < 0
    ):
        raise BuilderError("SEMANTIC_CLEANUP_FAILED")


def _strict_thread_close(clew: Path, thread_id: str) -> None:
    try:
        try:
            value = _run_json(
                clew, ["thread", "close", "--thread", thread_id], 120
            )
            _validate_thread_closed(value, thread_id)
        except BuilderError:
            # Recovery can observe a resource already collected after a crash
            # between the product mutation and the private-ledger checkpoint.
            pass
        collected = _run_json(clew, ["thread", "gc", "--thread", thread_id], 120)
        _validate_garbage_collected(collected, "thread", thread_id)
    except ResidualProcessError:
        raise
    except BuilderError as error:
        raise BuilderError("SEMANTIC_CLEANUP_FAILED") from error


def _strict_session_abort(clew: Path, session_id: str) -> None:
    try:
        try:
            value = _run_json(
                clew, ["session", "abort", "--session", session_id], 120
            )
            _validate_session_aborted(value, session_id)
        except BuilderError:
            pass
        collected = _run_json(
            clew, ["session", "gc", "--session", session_id], 120
        )
        _validate_garbage_collected(collected, "session", session_id)
    except ResidualProcessError:
        raise
    except BuilderError as error:
        raise BuilderError("SEMANTIC_CLEANUP_FAILED") from error


def _validate_garbage_collected(
    value: Any, resource_kind: str, resource_id: str
) -> None:
    if resource_kind == "thread":
        root = closed(
            value,
            {"schema", "threadId", "lifecycle"},
            "SEMANTIC_CLEANUP_FAILED",
        )
        if root["threadId"] != resource_id:
            raise BuilderError("SEMANTIC_CLEANUP_FAILED")
    elif resource_kind == "session":
        root = closed(
            value, {"schema", "lifecycle"}, "SEMANTIC_CLEANUP_FAILED"
        )
    else:
        raise BuilderError("SEMANTIC_CLEANUP_FAILED")
    identity = f"{resource_kind}Id"
    authority = f"{resource_kind}AuthorityDigest"
    lifecycle = closed(
        root["lifecycle"],
        {
            "schema",
            identity,
            authority,
            "sequence",
            "previousEventHash",
            "status",
            "eventHash",
            "updatedUnixMs",
        },
        "SEMANTIC_CLEANUP_FAILED",
    )
    previous = lifecycle["previousEventHash"]
    if (
        root["schema"] != f"codeclew-{resource_kind}-gc-result/1.0"
        or lifecycle["schema"]
        != f"codeclew-{resource_kind}-lifecycle-entry/1.0"
        or lifecycle[identity] != resource_id
        or not isinstance(lifecycle[authority], str)
        or SHA256.fullmatch(lifecycle[authority]) is None
        or type(lifecycle["sequence"]) is not int
        or lifecycle["sequence"] < 0
        or (
            previous is not None
            and (
                not isinstance(previous, str)
                or SHA256.fullmatch(previous) is None
            )
        )
        or lifecycle["status"] != "GARBAGE_COLLECTED"
        or not isinstance(lifecycle["eventHash"], str)
        or SHA256.fullmatch(lifecycle["eventHash"]) is None
        or type(lifecycle["updatedUnixMs"]) is not int
        or lifecycle["updatedUnixMs"] < 0
    ):
        raise BuilderError("SEMANTIC_CLEANUP_FAILED")


def _resource_ledger_value(
    runtime_digest: str,
    runtime_key: str | None,
    sessions: list[str],
    threads: list[str],
    open_in_flight: dict[str, str] | None,
    temporary_root: dict[str, Any] | None,
    unsafe_teardown: bool,
) -> dict[str, Any]:
    unsigned = {
        "schema": RESOURCE_LEDGER_SCHEMA,
        "ownerPid": os.getpid(),
        "builderDigest": file_digest(Path(__file__).resolve()),
        "runtimeDigest": runtime_digest,
        "runtimeKey": runtime_key,
        "sessions": sessions,
        "threads": threads,
        "openInFlight": open_in_flight,
        "temporaryRoot": temporary_root,
        "unsafeTeardown": unsafe_teardown,
    }
    return {**unsigned, "authorityDigest": authority_digest(unsigned)}


def _semantic_temp_identity(path: Path) -> dict[str, Any]:
    resolved = path.resolve(strict=True)
    metadata = os.lstat(resolved)
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or metadata.st_uid != os.geteuid()
        or resolved.parent != Path(tempfile.gettempdir()).resolve(strict=True)
        or not resolved.name.startswith("codeclew-shape-fixture-")
    ):
        raise BuilderError("SEMANTIC_TEMPORARY_ROOT_INVALID")
    return {
        "path": os.fspath(resolved),
        "device": metadata.st_dev,
        "inode": metadata.st_ino,
    }


def _semantic_temp_exists(value: dict[str, Any]) -> bool:
    path = Path(value["path"])
    try:
        metadata = os.lstat(path)
    except FileNotFoundError:
        return False
    except OSError as error:
        raise BuilderError("SEMANTIC_TEMPORARY_ROOT_INVALID") from error
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or metadata.st_uid != os.geteuid()
        or (metadata.st_dev, metadata.st_ino) != (value["device"], value["inode"])
    ):
        raise BuilderError("SEMANTIC_TEMPORARY_ROOT_INVALID")
    return True


def _remove_semantic_temp(value: dict[str, Any]) -> None:
    if not _semantic_temp_exists(value):
        return
    path = Path(value["path"])
    try:
        shutil.rmtree(path)
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except OSError as error:
        raise BuilderError("SEMANTIC_TEMPORARY_ROOT_CLEANUP_FAILED") from error


def _remove_private_ledger(path: Path) -> None:
    try:
        metadata = os.lstat(path)
        if (
            stat.S_ISLNK(metadata.st_mode)
            or not stat.S_ISREG(metadata.st_mode)
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_uid != os.geteuid()
        ):
            raise BuilderError("SEMANTIC_RECOVERY_FAILED")
        path.unlink()
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except FileNotFoundError:
        return
    except OSError as error:
        raise BuilderError("SEMANTIC_RECOVERY_FAILED") from error


def recover_resource_ledger(clew: Path, path: Path, runtime_digest: str) -> None:
    try:
        os.lstat(path)
    except FileNotFoundError:
        return
    except OSError as error:
        raise BuilderError("SEMANTIC_RECOVERY_FAILED") from error
    _, value, _ = private_json(path, "SEMANTIC_RECOVERY_FAILED", 256 * 1024)
    row = closed(
        value,
        {
            "schema",
            "authorityDigest",
            "ownerPid",
            "builderDigest",
            "runtimeDigest",
            "runtimeKey",
            "sessions",
            "threads",
            "openInFlight",
            "temporaryRoot",
            "unsafeTeardown",
        },
        "SEMANTIC_RECOVERY_FAILED",
    )
    sessions = row["sessions"]
    threads = row["threads"]
    unsigned = dict(row)
    declared = unsigned.pop("authorityDigest")
    open_in_flight = row["openInFlight"]
    temporary_root = row["temporaryRoot"]
    unsafe_teardown = row["unsafeTeardown"]
    if open_in_flight is not None and (
        not isinstance(open_in_flight, dict)
        or set(open_in_flight) != {"kind", "requestDigest"}
        or open_in_flight.get("kind") not in {"SESSION", "THREAD"}
        or not isinstance(open_in_flight.get("requestDigest"), str)
        or SHA256.fullmatch(open_in_flight["requestDigest"]) is None
    ):
        raise BuilderError("SEMANTIC_RECOVERY_FAILED")
    if temporary_root is not None and (
        not isinstance(temporary_root, dict)
        or set(temporary_root) != {"path", "device", "inode"}
        or not isinstance(temporary_root.get("path"), str)
        or not Path(temporary_root["path"]).is_absolute()
        or Path(temporary_root["path"]).parent
        != Path(tempfile.gettempdir()).resolve(strict=True)
        or not Path(temporary_root["path"]).name.startswith("codeclew-shape-fixture-")
        or type(temporary_root.get("device")) is not int
        or type(temporary_root.get("inode")) is not int
    ):
        raise BuilderError("SEMANTIC_RECOVERY_FAILED")
    if (
        row["schema"] != RESOURCE_LEDGER_SCHEMA
        or declared != authority_digest(unsigned)
        or type(row["ownerPid"]) is not int
        or row["ownerPid"] <= 0
        or row["builderDigest"] != file_digest(Path(__file__).resolve())
        or row["runtimeDigest"] != runtime_digest
        or (
            row["runtimeKey"] is not None
            and (
                not isinstance(row["runtimeKey"], str)
                or SHA256.fullmatch(row["runtimeKey"]) is None
            )
        )
        or not isinstance(sessions, list)
        or not isinstance(threads, list)
        or type(unsafe_teardown) is not bool
        or len(set(sessions)) != len(sessions)
        or len(set(threads)) != len(threads)
        or any(not isinstance(item, str) or SESSION_ID.fullmatch(item) is None for item in sessions)
        or any(not isinstance(item, str) or THREAD_ID.fullmatch(item) is None for item in threads)
    ):
        raise BuilderError("SEMANTIC_RECOVERY_FAILED")
    try:
        os.kill(row["ownerPid"], 0)
    except ProcessLookupError:
        pass
    except PermissionError as error:
        raise BuilderError("SEMANTIC_RECOVERY_ACTIVE") from error
    else:
        raise BuilderError("SEMANTIC_RECOVERY_ACTIVE")
    if unsafe_teardown or open_in_flight is not None or (
        temporary_root is not None and _semantic_temp_exists(temporary_root)
    ):
        raise BuilderError("OPERATOR_CLEANUP_REQUIRED")
    remaining_sessions = list(sessions)
    remaining_threads = list(threads)
    try:
        while remaining_threads:
            _strict_thread_close(clew, remaining_threads[-1])
            remaining_threads.pop()
            atomic_private_replace(
                path,
                _resource_ledger_value(
                    runtime_digest,
                    row["runtimeKey"],
                    remaining_sessions,
                    remaining_threads,
                    None,
                    temporary_root,
                    False,
                ),
            )
        while remaining_sessions:
            _strict_session_abort(clew, remaining_sessions[-1])
            remaining_sessions.pop()
            atomic_private_replace(
                path,
                _resource_ledger_value(
                    runtime_digest,
                    row["runtimeKey"],
                    remaining_sessions,
                    remaining_threads,
                    None,
                    temporary_root,
                    False,
                ),
            )
    except ResidualProcessError as error:
        atomic_private_replace(
            path,
            _resource_ledger_value(
                runtime_digest,
                row["runtimeKey"],
                remaining_sessions,
                remaining_threads,
                None,
                temporary_root,
                True,
            ),
        )
        raise BuilderError("OPERATOR_CLEANUP_REQUIRED") from error
    except BuilderError as error:
        raise BuilderError("SEMANTIC_RECOVERY_FAILED") from error
    _remove_private_ledger(path)


class SemanticResources:
    def __init__(
        self,
        clew: Path,
        ledger: Path,
        runtime_digest: str,
        runtime_witness: RuntimeWitness,
        temporary_root: dict[str, Any] | None = None,
    ):
        self.clew = clew
        self.ledger = ledger
        self.runtime_digest = runtime_digest
        self.runtime_witness = runtime_witness
        self.sessions: list[str] = []
        self.threads: list[str] = []
        self.open_in_flight: dict[str, str] | None = None
        self.temporary_root = temporary_root
        self.unsafe_teardown = False
        self.acquired = False

    def _persist(self) -> None:
        if not self.acquired:
            raise BuilderError("SEMANTIC_RECOVERY_REQUIRED")
        atomic_private_replace(
            self.ledger,
            _resource_ledger_value(
                self.runtime_digest,
                self.runtime_witness.runtime_key,
                self.sessions,
                self.threads,
                self.open_in_flight,
                self.temporary_root,
                self.unsafe_teardown,
            ),
        )

    def acquire(self) -> None:
        if self.acquired:
            raise BuilderError("SEMANTIC_RECOVERY_REQUIRED")
        _create_private_once(
            self.ledger,
            _resource_ledger_value(
                self.runtime_digest,
                self.runtime_witness.runtime_key,
                self.sessions,
                self.threads,
                self.open_in_flight,
                self.temporary_root,
                self.unsafe_teardown,
            ),
            "SEMANTIC_RECOVERY_REQUIRED",
        )
        self.acquired = True

    def mark_unsafe_teardown(self) -> None:
        self.unsafe_teardown = True
        self._persist()

    def begin_open(self, kind: str, request_digest: str) -> None:
        if (
            self.open_in_flight is not None
            or kind not in {"SESSION", "THREAD"}
            or SHA256.fullmatch(request_digest) is None
        ):
            raise BuilderError("SEMANTIC_RECOVERY_REQUIRED")
        self.open_in_flight = {"kind": kind, "requestDigest": request_digest}
        self._persist()

    def track_session(
        self, session_id: str, runtime_key: str, runtime_mode: str
    ) -> None:
        if SESSION_ID.fullmatch(session_id) is None or session_id in self.sessions:
            raise BuilderError("INVALID_COMPILER_SESSION")
        self.runtime_witness.observe(runtime_key, runtime_mode)
        self.sessions.append(session_id)
        self.open_in_flight = None
        self._persist()

    def track_thread(self, thread_id: str) -> None:
        if THREAD_ID.fullmatch(thread_id) is None or thread_id in self.threads:
            raise BuilderError("INVALID_COMPILER_THREAD")
        self.threads.append(thread_id)
        self.open_in_flight = None
        self._persist()

    def close(self) -> None:
        while self.threads:
            _strict_thread_close(self.clew, self.threads[-1])
            self.threads.pop()
            self._persist()
        while self.sessions:
            _strict_session_abort(self.clew, self.sessions[-1])
            self.sessions.pop()
            self._persist()
        if self.open_in_flight is not None:
            raise BuilderError("OPERATOR_CLEANUP_REQUIRED")
        if self.temporary_root is not None:
            _remove_semantic_temp(self.temporary_root)
        _remove_private_ledger(self.ledger)

    def __enter__(self) -> SemanticResources:
        if not self.acquired:
            self.acquire()
        return self

    def __exit__(self, _kind: object, error: object, _traceback: object) -> None:
        if isinstance(error, ResidualProcessError):
            # A still-live child may still mutate the product state.  Preserve
            # the private locator and require quarantine of this experiment.
            self.mark_unsafe_teardown()
            raise BuilderError("OPERATOR_CLEANUP_REQUIRED") from error
        try:
            self.close()
        except ResidualProcessError as residual:
            self.mark_unsafe_teardown()
            raise BuilderError("OPERATOR_CLEANUP_REQUIRED") from residual


def open_session(
    authorities: Authorities,
    resources: SemanticResources,
    service: descriptor_gate.Service,
    target_ref: str,
    timeout_seconds: int,
) -> str:
    resources.begin_open(
        "SESSION",
        authority_digest(
            {
                "serviceAlias": service.alias,
                "revision": service.revision,
                "targetRef": target_ref,
                "language": "kotlin",
                "compilation": COMPILATION,
                "generationJobs": 1,
            }
        ),
    )
    value = _run_json(
        authorities.clew,
        [
            "session",
            "open",
            "--repo",
            os.fspath(service.repository),
            "--target-ref",
            target_ref,
            "--language",
            "kotlin",
            "--compilation",
            COMPILATION,
            "--generation-jobs",
            "1",
        ],
        timeout_seconds,
    )
    try:
        session = descriptor_gate.parse_session_open(value, service, target_ref)
    except descriptor_gate.GateError as error:
        raise BuilderError("INVALID_COMPILER_SESSION") from error
    resources.track_session(
        session["sessionId"], session["runtimeKey"], session["runtimeMode"]
    )
    return session["sessionId"]


def open_thread(
    clew: Path,
    resources: SemanticResources,
    provider_session: str,
    consumer_session: str,
    provider_service: str,
    consumer_service: str,
    timeout_seconds: int,
) -> str:
    resources.begin_open(
        "THREAD",
        authority_digest(
            {
                "members": {
                    "provider": provider_session,
                    "consumer": consumer_session,
                },
                "serviceAliases": {
                    "provider": provider_service,
                    "consumer": consumer_service,
                },
            }
        ),
    )
    value = _run_json(
        clew,
        [
            "thread",
            "open",
            "--member",
            f"provider={provider_session}",
            "--member",
            f"consumer={consumer_session}",
            "--service-alias",
            f"provider={provider_service}",
            "--service-alias",
            f"consumer={consumer_service}",
        ],
        min(timeout_seconds, 300),
    )
    root = closed(value, {"schema", "status", "thread"}, "INVALID_COMPILER_THREAD")
    thread = root["thread"]
    if (
        root["schema"] != "codeclew-thread-open/1.0"
        or root["status"] != "OPEN"
        or not isinstance(thread, dict)
        or not isinstance(thread.get("threadId"), str)
        or THREAD_ID.fullmatch(thread["threadId"]) is None
        or not isinstance(thread.get("authorityDigest"), str)
        or SHA256.fullmatch(thread["authorityDigest"]) is None
    ):
        raise BuilderError("INVALID_COMPILER_THREAD")
    resources.track_thread(thread["threadId"])
    return thread["threadId"]


def create_thread_context(
    clew: Path,
    thread_id: str,
    terms: list[str],
    timeout_seconds: int,
) -> str:
    value = _run_json(
        clew,
        [
            "thread",
            "context",
            "--thread",
            thread_id,
            "--intent",
            "derive frozen Kotlin compiler descriptor shape oracle",
            *[part for term in terms for part in ("--term", term)],
            "--max-roots",
            "2",
        ],
        min(timeout_seconds, 300),
    )
    root = closed(
        value,
        {
            "schema",
            "threadId",
            "threadAuthorityDigest",
            "contextId",
            "contextAuthorityDigest",
            "evidenceDigest",
            "evidenceRef",
            "context",
        },
        "INVALID_COMPILER_CONTEXT",
    )
    context = root["context"]
    if (
        root["schema"] != "codeclew-thread-context-result/1.0"
        or root["threadId"] != thread_id
        or not isinstance(root["contextId"], str)
        or THREAD_CONTEXT_ID.fullmatch(root["contextId"]) is None
        or not isinstance(context, dict)
        or context.get("schema") != "codeclew-thread-context-projection/1.0"
        or context.get("threadId") != thread_id
        or context.get("contextId") != root["contextId"]
        or context.get("contextAuthorityDigest") != root["contextAuthorityDigest"]
        or not isinstance(context.get("members"), list)
        or len(context["members"]) != 2
    ):
        raise BuilderError("INVALID_COMPILER_CONTEXT")
    for key in ("threadAuthorityDigest", "contextAuthorityDigest", "evidenceDigest"):
        digest(root[key], "INVALID_COMPILER_CONTEXT")
    cas_reference(
        root["evidenceRef"],
        "codeclew-thread-context-evidence/1.0",
        "INVALID_COMPILER_CONTEXT",
    )
    return root["contextId"]


def create_callables(
    clew: Path,
    thread_id: str,
    context_id: str,
    task_id: str,
    pair_id: str,
    terms: list[str],
    timeout_seconds: int,
) -> str:
    value = _run_json(
        clew,
        [
            "thread",
            "callables",
            "--thread",
            thread_id,
            "--context",
            context_id,
            "--task-id",
            task_id,
            "--pair-id",
            pair_id,
            "--provider",
            "provider",
            "--consumer",
            "consumer",
            *[part for term in terms for part in ("--term", term)],
        ],
        min(timeout_seconds, 300),
    )
    root = closed(
        value,
        {
            "schema",
            "threadId",
            "threadAuthorityDigest",
            "contextId",
            "contextAuthorityDigest",
            "factSetId",
            "authorityDigest",
            "evidenceRef",
            "queryIndexRef",
            "callables",
        },
        "INVALID_COMPILER_CALLABLES",
    )
    projection = closed(
        root["callables"],
        {
            "schema",
            "factSetId",
            "authorityDigest",
            "bindingDigest",
            "threadId",
            "threadContextId",
            "tasks",
            "pairs",
            "members",
            "counts",
            "completeness",
            "queryIndexRef",
            "evidenceRef",
        },
        "INVALID_COMPILER_CALLABLES",
    )
    if (
        root["schema"] != "codeclew-thread-callables-result/1.0"
        or root["threadId"] != thread_id
        or root["contextId"] != context_id
        or not isinstance(root["factSetId"], str)
        or FACT_SET_ID.fullmatch(root["factSetId"]) is None
        or projection["schema"] != "codeclew-kotlin-callable-fact-set-projection/1.0"
        or projection["factSetId"] != root["factSetId"]
        or projection["authorityDigest"] != root["authorityDigest"]
        or projection["threadId"] != thread_id
        or projection["threadContextId"] != context_id
        or projection["evidenceRef"] != root["evidenceRef"]
        or projection["queryIndexRef"] != root["queryIndexRef"]
    ):
        raise BuilderError("INVALID_COMPILER_CALLABLES")
    for key in ("threadAuthorityDigest", "contextAuthorityDigest", "authorityDigest"):
        digest(root[key], "INVALID_COMPILER_CALLABLES")
    digest(projection["bindingDigest"], "INVALID_COMPILER_CALLABLES")
    cas_reference(root["evidenceRef"], None, "INVALID_COMPILER_CALLABLES")
    cas_reference(root["queryIndexRef"], None, "INVALID_COMPILER_CALLABLES")
    tasks = projection["tasks"]
    pairs = projection["pairs"]
    members = projection["members"]
    if (
        not isinstance(tasks, list)
        or len(tasks) != 1
        or tasks[0].get("taskId") != task_id
        or tasks[0].get("pairId") != pair_id
        or type(tasks[0].get("termCount")) is not int
        or not len(terms) <= tasks[0]["termCount"] <= 256
        or not isinstance(tasks[0].get("termsDigest"), str)
        or SHA256.fullmatch(tasks[0]["termsDigest"]) is None
        or not isinstance(pairs, list)
        or len(pairs) != 1
        or pairs[0].get("pairId") != pair_id
        or pairs[0].get("providerMember") != "provider"
        or pairs[0].get("consumerMember") != "consumer"
        or pairs[0].get("relationshipAuthority") != "DECLARED_TOPOLOGY"
        or not isinstance(members, list)
        or len(members) != 2
        or {member.get("memberAlias") for member in members}
        != {"provider", "consumer"}
    ):
        raise BuilderError("INVALID_COMPILER_CALLABLES")
    return root["factSetId"]


def run_impact(
    clew: Path,
    thread_id: str,
    fact_set_id: str,
    pair_id: str,
    subject_kind: str,
    subject: str,
    timeout_seconds: int,
    *,
    member: str | None = None,
) -> dict[str, Any]:
    command = [
        "thread",
        "impact",
        "--thread",
        thread_id,
        "--fact-set",
        fact_set_id,
        "--pair-id",
        pair_id,
        "--subject-kind",
        subject_kind,
        "--subject",
        subject,
    ]
    if member is not None:
        command.extend(["--member", member])
    value = _run_json(clew, command, min(timeout_seconds, 180))
    root = closed(
        value,
        {
            "schema",
            "threadId",
            "threadAuthorityDigest",
            "factSetId",
            "factSetAuthorityDigest",
            "impactId",
            "authorityDigest",
            "evidenceRef",
            "impact",
        },
        "INVALID_COMPILER_IMPACT",
    )
    impact = closed(
        root["impact"],
        {
            "schema",
            "impactId",
            "authorityDigest",
            "bindingDigest",
            "factSetAuthorityDigest",
            "pairId",
            "subjectKind",
            "relationshipAuthority",
            "shapeStatus",
            "certainty",
            "members",
            "findingCount",
            "sourceWindowCount",
            "obligationCount",
            "findingsTruncated",
            "sourceWindowsTruncated",
            "findings",
            "publicFindingsTruncated",
            "obligations",
            "sourceWindows",
            "evidenceRef",
        },
        "INVALID_COMPILER_IMPACT",
    )
    expected_kind = "TOKEN" if subject_kind == "token" else "FULL_SYMBOL"
    if (
        root["schema"] != "codeclew-thread-impact-result/1.0"
        or root["threadId"] != thread_id
        or root["factSetId"] != fact_set_id
        or root["impactId"] != impact["impactId"]
        or root["authorityDigest"] != impact["authorityDigest"]
        or root["factSetAuthorityDigest"] != impact["factSetAuthorityDigest"]
        or root["evidenceRef"] != impact["evidenceRef"]
        or impact["schema"] != "codeclew-kotlin-thread-impact-projection/1.0"
        or impact["pairId"] != pair_id
        or impact["subjectKind"] != expected_kind
        or impact["relationshipAuthority"] != "DECLARED_TOPOLOGY"
        or impact["certainty"] != "UNSURE"
        or impact["shapeStatus"]
        not in {
            "EXACT_PROJECTED_SHAPE_EQUAL",
            "EXACT_PROJECTED_SHAPE_DELTA",
            "UNSURE",
            "NOT_COMPARABLE",
        }
        or impact["findingsTruncated"] is not False
        or impact["publicFindingsTruncated"] is not False
        or impact["sourceWindowsTruncated"] is not False
        or not isinstance(impact["findings"], list)
        or not isinstance(impact["obligations"], list)
        or not isinstance(impact["sourceWindows"], list)
        or impact["findingCount"] != len(impact["findings"])
        or impact["obligationCount"] != len(impact["obligations"])
        or impact["sourceWindowCount"] != len(impact["sourceWindows"])
    ):
        raise BuilderError("INVALID_COMPILER_IMPACT")
    for key in (
        "threadAuthorityDigest",
        "factSetAuthorityDigest",
        "authorityDigest",
    ):
        digest(root[key], "INVALID_COMPILER_IMPACT")
    digest(impact["bindingDigest"], "INVALID_COMPILER_IMPACT")
    if (
        not isinstance(root["impactId"], str)
        or IMPACT_ID.fullmatch(root["impactId"]) is None
        or root["impactId"] != f"thread-impact:{root['authorityDigest']}"
    ):
        raise BuilderError("INVALID_COMPILER_IMPACT")
    cas_reference(root["evidenceRef"], None, "INVALID_COMPILER_IMPACT")
    members = impact["members"]
    if (
        not isinstance(members, list)
        or len(members) != 2
        or {(row.get("side"), row.get("memberAlias")) for row in members}
        != {("PROVIDER", "provider"), ("CONSUMER", "consumer")}
    ):
        raise BuilderError("INVALID_COMPILER_IMPACT")
    return impact


def _candidate_binding(candidate: CompilerDeclaration) -> bytes:
    return canonical_bytes(
        {
            "memberAlias": candidate.member_alias,
            "side": candidate.side,
            "symbolIdentity": candidate.symbol_identity,
            "projectedShape": candidate.projected_shape,
            "shapeDigest": candidate.shape_digest,
            "source": candidate.source,
        }
    )


def _confirmed_declarations(
    clew: Path,
    thread_id: str,
    fact_set_id: str,
    pair_id: str,
    terms: list[str],
    timeout_seconds: int,
) -> dict[str, list[CompilerDeclaration]]:
    discovered: dict[tuple[str, str], CompilerDeclaration] = {}
    for term in terms:
        impact = run_impact(
            clew,
            thread_id,
            fact_set_id,
            pair_id,
            "token",
            term,
            timeout_seconds,
        )
        for member_alias in ("provider", "consumer"):
            for finding in impact["findings"]:
                candidate = declaration_from_finding(
                    finding, member_alias=member_alias, exact=False
                )
                if candidate is None:
                    continue
                if (
                    candidate.compiler_name not in terms
                    or candidate.declaration_kind not in {"FUNCTION", "CLASS"}
                ):
                    continue
                key = (member_alias, candidate.symbol_identity)
                existing = discovered.get(key)
                if existing is not None and _candidate_binding(existing) != _candidate_binding(candidate):
                    raise BuilderError("COMPILER_PROJECTION_SUBSTITUTED")
                discovered[key] = candidate
    confirmed: dict[str, list[CompilerDeclaration]] = {
        "provider": [],
        "consumer": [],
    }
    for (member_alias, symbol_identity), candidate in sorted(discovered.items()):
        impact = run_impact(
            clew,
            thread_id,
            fact_set_id,
            pair_id,
            "full-symbol",
            symbol_identity,
            timeout_seconds,
            member=member_alias,
        )
        exact_rows: list[CompilerDeclaration] = []
        for finding in impact["findings"]:
            exact = declaration_from_finding(
                finding, member_alias=member_alias, exact=True
            )
            if exact is not None and exact.symbol_identity == symbol_identity:
                exact_rows.append(exact)
        if len(exact_rows) != 1 or _candidate_binding(exact_rows[0]) != _candidate_binding(candidate):
            raise BuilderError("COMPILER_PROJECTION_NOT_EXACT")
        confirmed[member_alias].append(exact_rows[0])
    return confirmed


def _matches_allowed(
    candidate: CompilerDeclaration,
    declarations: tuple[descriptor_gate.OracleDeclaration, ...],
) -> bool:
    for declaration in declarations:
        if candidate.compiler_name != declaration.name:
            continue
        if declaration.kind == "FUN" and candidate.declaration_kind == "FUNCTION":
            return True
        if declaration.kind in TYPE_ORACLE_KINDS and candidate.declaration_kind == "CLASS":
            return True
    return False


def _cas_blob_reference(content: bytes) -> dict[str, Any]:
    digest_value = hashlib.sha256(
        CAS_DOMAIN + REPOSITORY_BLOB_SCHEMA.encode("utf-8") + b"\0" + content
    ).hexdigest()
    return {
        "schema": descriptor_gate.CAS_OBJECT_SCHEMA,
        "objectSchema": REPOSITORY_BLOB_SCHEMA,
        "digest": f"sha256:{digest_value}",
        "size": len(content),
    }


def _pinned_blob(
    git: Path,
    service: descriptor_gate.Service,
    revision: str,
    relative_file: str,
    expected_oid: str,
    cache: dict[tuple[str, str, str, str], bytes],
) -> bytes:
    key = (os.fspath(service.repository), revision, relative_file, expected_oid)
    cached = cache.get(key)
    if cached is not None:
        return cached
    if service.revision != revision:
        raise BuilderError("SOURCE_REVISION_NOT_PINNED")
    observed_revision = _isolated_git_text(
        git,
        service.repository,
        ["rev-parse", "--verify", f"{revision}^{{commit}}"],
        "SOURCE_REVISION_NOT_PINNED",
    )
    observed_oid = _isolated_git_text(
        git,
        service.repository,
        ["rev-parse", "--verify", f"{revision}:{relative_file}"],
        "SOURCE_BLOB_NOT_PINNED",
    )
    if observed_revision != revision or observed_oid != expected_oid:
        raise BuilderError("SOURCE_BLOB_NOT_PINNED")
    content = _isolated_git(
        git,
        service.repository,
        ["cat-file", "blob", expected_oid],
        maximum=MAX_GIT_BLOB_BYTES,
    )
    cache[key] = content
    return content


def _bind_candidate_source(
    candidate: CompilerDeclaration,
    content: bytes,
    expected_oid: str,
) -> dict[str, Any]:
    expected_ref = _cas_blob_reference(content)
    if candidate.source["contentRef"] != expected_ref:
        raise BuilderError("SOURCE_CONTENT_REFERENCE_MISMATCH")
    start = candidate.source["start"]
    end = candidate.source["end"]
    if not 0 <= start < end <= len(content):
        raise BuilderError("SOURCE_RANGE_NOT_PINNED")
    try:
        content.decode("utf-8")
        content[:start].decode("utf-8")
        selected = content[start:end].decode("utf-8")
    except UnicodeDecodeError as error:
        raise BuilderError("SOURCE_RANGE_NOT_PINNED") from error
    if not selected.strip() or candidate.compiler_name not in selected:
        raise BuilderError("SOURCE_RANGE_NOT_PINNED")
    row = candidate.unpinned_row()
    row["blobOid"] = expected_oid
    validate_exact_row(row)
    return row


def _pin_candidates(
    candidates: list[CompilerDeclaration],
    oracle: descriptor_gate.OracleSide,
    service: descriptor_gate.Service,
    git: Path,
    blob_cache: dict[tuple[str, str, str, str], bytes],
) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]]]:
    navigation_by_path = {
        navigation.relative_file: navigation for navigation in oracle.navigations
    }
    rows: list[tuple[dict[str, Any], dict[str, Any]]] = []
    for candidate in candidates:
        navigation = navigation_by_path.get(candidate.source["path"])
        if navigation is None or not _matches_allowed(candidate, navigation.declarations):
            continue
        content = _pinned_blob(
            git,
            service,
            oracle.revision,
            navigation.relative_file,
            navigation.blob_oid,
            blob_cache,
        )
        row = _bind_candidate_source(candidate, content, navigation.blob_oid)
        rows.append((row, candidate.projected_shape))
    rows.sort(
        key=lambda item: (
            item[0]["descriptorClass"],
            item[0]["name"],
            item[0]["normalizedSignature"],
            item[0]["relativeFile"],
            item[0]["sourceRange"]["startByte"],
        )
    )
    if not rows:
        raise BuilderError("NO_APPROVED_EXACT_DECLARATION")
    if len({authority_digest(row) for row, _ in rows}) != len(rows):
        raise BuilderError("DUPLICATE_EXACT_DECLARATION")
    projections = {authority_digest(row): projected for row, projected in rows}
    return [row for row, _ in rows], projections


def _task_terms(
    provider: descriptor_gate.OracleSide, consumer: descriptor_gate.OracleSide
) -> list[str]:
    terms = sorted(set(descriptor_gate.side_query_terms(provider)) | set(descriptor_gate.side_query_terms(consumer)))
    if not terms or len(terms) > 16:
        raise BuilderError("TASK_QUERY_BUDGET_INVALID")
    return terms


def build_task_oracles(
    authorities: Authorities,
    resource_ledger: Path,
    runtime_witness: RuntimeWitness,
    timeout_seconds: int,
) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]]]:
    if (
        authorities.corpus is None
        or authorities.benchmark is None
        or authorities.benchmark_value is None
    ):
        raise BuilderError("PRIVATE_AUTHORITIES_REQUIRED")
    services = {service.alias: service for service in authorities.corpus.services}
    side_oracles = {
        (side.task_id, side.role): side for side in authorities.benchmark.sides
    }
    benchmark_tasks = {
        row["taskId"]: row for row in authorities.benchmark_value["tasks"]
    }
    output: list[dict[str, Any]] = []
    projections: dict[str, dict[str, Any]] = {}
    blob_cache: dict[tuple[str, str, str, str], bytes] = {}
    with SemanticResources(
        authorities.clew,
        resource_ledger,
        authorities.runtime_digest,
        runtime_witness,
    ) as resources:
        sessions: dict[str, str] = {}
        for service in authorities.corpus.services:
            try:
                target_ref = descriptor_gate.pinned_target_ref(authorities.git, service)
            except descriptor_gate.GateError as error:
                raise BuilderError(error.code) from error
            sessions[service.alias] = open_session(
                authorities, resources, service, target_ref, timeout_seconds
            )
        for task in authorities.corpus.tasks:
            provider_oracle = side_oracles[(task.task_id, "provider")]
            consumer_oracle = side_oracles[(task.task_id, "consumer")]
            terms = _task_terms(provider_oracle, consumer_oracle)
            thread_id = open_thread(
                authorities.clew,
                resources,
                sessions[task.provider],
                sessions[task.consumer],
                task.provider,
                task.consumer,
                timeout_seconds,
            )
            context_id = create_thread_context(
                authorities.clew, thread_id, terms, timeout_seconds
            )
            fact_set_id = create_callables(
                authorities.clew,
                thread_id,
                context_id,
                task.task_id,
                task.pair_id,
                terms,
                timeout_seconds,
            )
            confirmed = _confirmed_declarations(
                authorities.clew,
                thread_id,
                fact_set_id,
                task.pair_id,
                terms,
                timeout_seconds,
            )
            sides: list[dict[str, Any]] = []
            slot_classes: set[str] = set()
            for role, oracle in (
                ("provider", provider_oracle),
                ("consumer", consumer_oracle),
            ):
                exact_rows, side_projections = _pin_candidates(
                    confirmed[role],
                    oracle,
                    services[oracle.service_alias],
                    authorities.git,
                    blob_cache,
                )
                overlap = set(projections) & set(side_projections)
                if overlap:
                    # Repeated tasks may legitimately bind an identical row;
                    # it must carry the byte-identical projection.
                    if any(
                        canonical_bytes(projections[key])
                        != canonical_bytes(side_projections[key])
                        for key in overlap
                    ):
                        raise BuilderError("COMPILER_PROJECTION_SUBSTITUTED")
                projections.update(side_projections)
                slot_classes.update(row["descriptorClass"] for row in exact_rows)
                sides.append(
                    {
                        "role": role,
                        "serviceAlias": oracle.service_alias,
                        "revision": oracle.revision,
                        "approvedFiles": [
                            {
                                "relativeFile": navigation.relative_file,
                                "blobOid": navigation.blob_oid,
                            }
                            for navigation in oracle.navigations
                        ],
                        "exactDeclarations": exact_rows,
                    }
                )
            if slot_classes != {"CALLABLE", "TYPE"}:
                raise BuilderError("TASK_DESCRIPTOR_SLOTS_INCOMPLETE")
            output.append(
                {
                    "taskId": task.task_id,
                    "pairId": task.pair_id,
                    "manualVerification": _manual_categories(
                        authorities.benchmark_value, benchmark_tasks[task.task_id]
                    ),
                    "sides": sides,
                }
            )
    return output, projections


def _git_mutation(arguments: list[str], environment: dict[str, str] | None = None) -> None:
    return_code, stdout, stderr = _run_bounded_process(
        arguments,
        environment=(
            environment if environment is not None else _isolated_git_environment()
        ),
        timeout=60,
        stdout_limit=MAX_CHECKED_BYTES,
        stderr_limit=MAX_CHECKED_BYTES,
        code="PUBLIC_FIXTURE_GIT_FAILED",
    )
    if return_code != 0 or stdout or stderr:
        raise BuilderError("PUBLIC_FIXTURE_GIT_FAILED")


def _copy_public_fixture(
    fixture: SealedFixture, destination: Path, git: Path
) -> tuple[str, str]:
    try:
        destination.mkdir(mode=0o700)
        for relative, mode, _, content in fixture.files:
            target = destination.joinpath(*relative.split("/"))
            target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
            with os.fdopen(descriptor, "wb", closefd=True) as stream:
                stream.write(content)
                stream.flush()
                os.fsync(stream.fileno())
            os.chmod(target, mode)
    except (OSError, BuilderError) as error:
        if isinstance(error, BuilderError):
            raise
        raise BuilderError("PUBLIC_FIXTURE_INVALID") from error
    _git_mutation([os.fspath(git), "-C", os.fspath(destination), "init", "-q", "-b", "shape-oracle"])
    _git_mutation([os.fspath(git), "-C", os.fspath(destination), "add", "."])
    environment = {
        **_isolated_git_environment(),
        "GIT_AUTHOR_NAME": "Codeclew Shape Oracle",
        "GIT_AUTHOR_EMAIL": "shape-oracle" + "@" + "example.invalid",
        "GIT_COMMITTER_NAME": "Codeclew Shape Oracle",
        "GIT_COMMITTER_EMAIL": "shape-oracle" + "@" + "example.invalid",
        "GIT_AUTHOR_DATE": "2000-01-01T00:00:00Z",
        "GIT_COMMITTER_DATE": "2000-01-01T00:00:00Z",
    }
    _git_mutation(
        [
            os.fspath(git),
            "-C",
            os.fspath(destination),
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "fixture",
        ],
        environment,
    )
    try:
        revision = descriptor_gate.git_output(
            git, destination, ["rev-parse", "HEAD"]
        )
        blob = descriptor_gate.git_output(
            git,
            destination,
            ["rev-parse", "HEAD:src/main/kotlin/com/acme/RelationFacts.kt"],
        )
    except descriptor_gate.GateError as error:
        raise BuilderError("PUBLIC_FIXTURE_GIT_FAILED") from error
    if GIT_OID.fullmatch(revision) is None or GIT_OID.fullmatch(blob) is None:
        raise BuilderError("PUBLIC_FIXTURE_GIT_FAILED")
    return revision, blob


def build_public_fixture(
    authorities: Authorities,
    fixture_authority: SealedFixture,
    resource_ledger: Path,
    runtime_witness: RuntimeWitness,
    timeout_seconds: int,
) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]]]:
    git = authorities.git
    terms = [
        "publicDescriptor",
        "overloadedDescriptor",
        "genericDescriptor",
        "Envelope",
    ]
    relative_file = "src/main/kotlin/com/acme/RelationFacts.kt"
    allowed = (
        descriptor_gate.OracleDeclaration("FUN", "publicDescriptor"),
        descriptor_gate.OracleDeclaration("FUN", "overloadedDescriptor"),
        descriptor_gate.OracleDeclaration("FUN", "genericDescriptor"),
        descriptor_gate.OracleDeclaration("DATA_CLASS", "Envelope"),
    )
    with tempfile.TemporaryDirectory(prefix="codeclew-shape-fixture-") as directory:
        temporary = Path(directory)
        temporary.chmod(0o700)
        temporary_root = _semantic_temp_identity(temporary)
        resources = SemanticResources(
            authorities.clew,
            resource_ledger,
            authorities.runtime_digest,
            runtime_witness,
            temporary_root,
        )
        resources.acquire()
        repositories = [temporary / "provider", temporary / "consumer"]
        revisions: list[str] = []
        blobs: list[str] = []
        for repository in repositories:
            try:
                revision, blob = _copy_public_fixture(
                    fixture_authority, repository, git
                )
            except ResidualProcessError as error:
                resources.mark_unsafe_teardown()
                raise BuilderError("OPERATOR_CLEANUP_REQUIRED") from error
            revisions.append(revision)
            blobs.append(blob)
        if revisions[0] != revisions[1] or blobs[0] != blobs[1]:
            raise BuilderError("PUBLIC_FIXTURE_AUTHORITY_CHANGED")
        services = [
            descriptor_gate.Service(
                "public-provider",
                "public-provider",
                repositories[0],
                revisions[0],
            ),
            descriptor_gate.Service(
                "public-consumer",
                "public-consumer",
                repositories[1],
                revisions[1],
            ),
        ]
        with resources:
            sessions = [
                open_session(
                    authorities,
                    resources,
                    service,
                    "refs/heads/shape-oracle",
                    timeout_seconds,
                )
                for service in services
            ]
            thread_id = open_thread(
                authorities.clew,
                resources,
                sessions[0],
                sessions[1],
                services[0].alias,
                services[1].alias,
                timeout_seconds,
            )
            context_id = create_thread_context(
                authorities.clew, thread_id, terms, timeout_seconds
            )
            fact_set_id = create_callables(
                authorities.clew,
                thread_id,
                context_id,
                "public-fixture",
                "public-pair",
                terms,
                timeout_seconds,
            )
            confirmed = _confirmed_declarations(
                authorities.clew,
                thread_id,
                fact_set_id,
                "public-pair",
                terms,
                timeout_seconds,
            )
            semantic = lambda candidate: canonical_bytes(  # noqa: E731
                {
                    "symbolIdentity": candidate.symbol_identity,
                    "projectedShape": candidate.projected_shape,
                    "shapeDigest": candidate.shape_digest,
                    "path": candidate.source["path"],
                    "start": candidate.source["start"],
                    "end": candidate.source["end"],
                    "contentRef": candidate.source["contentRef"],
                }
            )
            provider_semantics = {
                candidate.symbol_identity: semantic(candidate)
                for candidate in confirmed["provider"]
            }
            consumer_semantics = {
                candidate.symbol_identity: semantic(candidate)
                for candidate in confirmed["consumer"]
            }
            if provider_semantics != consumer_semantics:
                raise BuilderError("PUBLIC_FIXTURE_COMPILER_DIVERGED")
            oracle = descriptor_gate.OracleSide(
                "public-fixture",
                "provider",
                services[0].alias,
                revisions[0],
                1,
                1,
                1,
                (
                    descriptor_gate.OracleNavigation(
                        relative_file, blobs[0], allowed
                    ),
                ),
            )
            rows, projections = _pin_candidates(
                confirmed["provider"],
                oracle,
                services[0],
                git,
                {},
            )

    grouped: dict[str, list[dict[str, Any]]] = {}
    for row in rows:
        grouped.setdefault(row["name"], []).append(row)
    if (
        len(grouped.get("publicDescriptor", [])) != 1
        or len(grouped.get("overloadedDescriptor", [])) != 2
        or len(grouped.get("genericDescriptor", [])) != 1
        or len(grouped.get("Envelope", [])) != 1
        or set(grouped)
        != {"publicDescriptor", "overloadedDescriptor", "genericDescriptor", "Envelope"}
    ):
        raise BuilderError("PUBLIC_FIXTURE_SHAPE_INCOMPLETE")
    public_row = grouped["publicDescriptor"][0]
    generic_row = grouped["genericDescriptor"][0]
    envelope_row = grouped["Envelope"][0]
    public_shape = projections[authority_digest(public_row)]
    generic_shape = projections[authority_digest(generic_row)]
    if (
        public_shape.get("returnNullable") is not True
        or not str(public_shape.get("returnType", "")).endswith("?")
        or not isinstance(generic_shape.get("typeParameters"), list)
        or len(generic_shape["typeParameters"]) != 1
        or envelope_row["declarationKind"] != "CLASS"
    ):
        raise BuilderError("PUBLIC_FIXTURE_SHAPE_INCOMPLETE")
    overloads = sorted(
        grouped["overloadedDescriptor"], key=lambda row: row["normalizedSignature"]
    )
    if len({row["normalizedSignature"] for row in overloads}) != 2:
        raise BuilderError("PUBLIC_FIXTURE_SHAPE_INCOMPLETE")
    ordered = [public_row, *overloads, generic_row, envelope_row]
    return ordered, projections


def validate_attestation(
    value: Any,
    shape: dict[str, Any],
    authorities: Authorities,
    review: ReviewAuthority,
) -> str:
    root = closed(
        value,
        {
            "schema",
            "authorityDigest",
            "shapeOracleDigest",
            "g1kEvidenceDigest",
            "runtimeDigest",
            "runtimeKey",
            "gitDigest",
            "gitEnvironmentDigest",
            "localModuleManifestDigest",
            "compilerEnvironmentDigest",
            "builderDigest",
            "compilerVerification",
            "reviewManifestDigest",
        },
        "INVALID_SHAPE_ATTESTATION",
    )
    unsigned = dict(root)
    declared = unsigned.pop("authorityDigest")
    if (
        root["schema"] != ATTESTATION_SCHEMA
        or declared != authority_digest(unsigned)
        or root["shapeOracleDigest"] != shape["authorityDigest"]
        or root["g1kEvidenceDigest"] != authorities.g1k_digest
        or root["runtimeDigest"] != authorities.runtime_digest
        or root["runtimeKey"] != shape["sourceAuthority"]["runtimeKey"]
        or root["gitDigest"] != authorities.git_digest
        or root["gitEnvironmentDigest"]
        != authority_digest(_isolated_git_environment())
        or root["localModuleManifestDigest"]
        != local_module_manifest()["authorityDigest"]
        or root["compilerEnvironmentDigest"]
        != shape["sourceAuthority"]["compilerEnvironmentDigest"]
        or SHA256.fullmatch(str(root["runtimeKey"])) is None
        or root["builderDigest"] != file_digest(Path(__file__).resolve())
        or root["compilerVerification"] != "PASS"
        or root["reviewManifestDigest"] != review.digest
    ):
        raise BuilderError("INVALID_SHAPE_ATTESTATION")
    digest(declared, "INVALID_SHAPE_ATTESTATION")
    return declared


def _merge_projections(
    target: dict[str, dict[str, Any]], incoming: dict[str, dict[str, Any]]
) -> None:
    for key, projected in incoming.items():
        existing = target.get(key)
        if existing is not None and canonical_bytes(existing) != canonical_bytes(projected):
            raise BuilderError("COMPILER_PROJECTION_SUBSTITUTED")
        target[key] = projected


def build(args: argparse.Namespace) -> dict[str, Any]:
    experiment_root, _ = _experiment_root(args.experiment_root)
    _require_private_children(
        experiment_root,
        [
            args.private_corpus,
            args.private_benchmark,
            args.shape_oracle,
            args.attestation,
            args.review_manifest,
        ],
    )
    authorities = load_authorities(
        g1k_path=args.g1k_evidence,
        clew_path=args.clew,
        git_path=args.git,
        experiment_root_path=args.experiment_root,
        corpus_path=args.private_corpus,
        benchmark_path=args.private_benchmark,
    )
    shape_candidate = _output_target(args.shape_oracle)
    attestation_candidate = _output_target(args.attestation)
    resource_ledger = shape_candidate.with_name(
        f".{shape_candidate.name}.resources-pending.json"
    )
    recover_resource_ledger(
        authorities.clew, resource_ledger, authorities.runtime_digest
    )
    recover_private_pair(shape_candidate, attestation_candidate)
    shape_target = _output_target(args.shape_oracle, require_absent=True)
    attestation_target = _output_target(args.attestation, require_absent=True)
    if shape_target == attestation_target:
        raise BuilderError("PRIVATE_OUTPUT_INVALID")
    fixture_authority = load_sealed_public_fixture(authorities.git)
    review = load_review_authority(
        args.review_manifest, authorities, args.pilot_runner, fixture_authority
    )
    runtime_witness = RuntimeWitness()

    fixture, fixture_projections = build_public_fixture(
        authorities,
        fixture_authority,
        resource_ledger,
        runtime_witness,
        args.timeout_seconds,
    )
    tasks, task_projections = build_task_oracles(
        authorities, resource_ledger, runtime_witness, args.timeout_seconds
    )
    runtime_key = runtime_witness.require()
    projections: dict[str, dict[str, Any]] = {}
    _merge_projections(projections, fixture_projections)
    _merge_projections(projections, task_projections)
    unsigned_shape = {
        "schema": SHAPE_SCHEMA,
        "sourceAuthority": {
            "privateCorpusDigest": descriptor_gate.EXPECTED_CORPUS_DIGEST,
            "benchmarkDigest": descriptor_gate.EXPECTED_BENCHMARK_DIGEST,
            "g1kEvidenceDigest": authorities.g1k_digest,
            "runtimeDigest": authorities.runtime_digest,
            "runtimeKey": runtime_key,
            "gitDigest": authorities.git_digest,
            "gitEnvironmentDigest": authority_digest(_isolated_git_environment()),
            "localModuleManifestDigest": local_module_manifest()["authorityDigest"],
            "compilerEnvironmentDigest": authority_digest(_clew_environment()),
            "compilerProjectionSchema": PROJECTION_SCHEMA,
            "publicFixtureTreeOid": fixture_authority.tree_oid,
            "publicFixtureContentDigest": fixture_authority.content_digest,
        },
        "fixture": fixture,
        "tasks": tasks,
    }
    shape = {
        **unsigned_shape,
        "authorityDigest": authority_digest(unsigned_shape),
    }
    validate_shape_oracle(shape, authorities, projection_rows=projections)
    unsigned_attestation = {
        "schema": ATTESTATION_SCHEMA,
        "shapeOracleDigest": shape["authorityDigest"],
        "g1kEvidenceDigest": authorities.g1k_digest,
        "runtimeDigest": authorities.runtime_digest,
        "runtimeKey": runtime_key,
        "gitDigest": authorities.git_digest,
        "gitEnvironmentDigest": authority_digest(_isolated_git_environment()),
        "localModuleManifestDigest": local_module_manifest()["authorityDigest"],
        "compilerEnvironmentDigest": authority_digest(_clew_environment()),
        "builderDigest": file_digest(Path(__file__).resolve()),
        "compilerVerification": "PASS",
        "reviewManifestDigest": review.digest,
    }
    attestation = {
        **unsigned_attestation,
        "authorityDigest": authority_digest(unsigned_attestation),
    }
    attestation_digest = validate_attestation(attestation, shape, authorities, review)
    publish_private_pair(shape_target, shape, attestation_target, attestation)

    # Re-open exactly what will be handed to the runner.  This intentionally
    # exercises the runner-compatible verifier without retaining projections
    # or repository locators in either private artifact.
    _, stored_shape, _ = private_json(shape_target, "INVALID_SHAPE_ORACLE")
    _, stored_attestation, _ = private_json(
        attestation_target, "INVALID_SHAPE_ATTESTATION", 256 * 1024
    )
    validate_shape_oracle(stored_shape, authorities)
    validate_attestation(stored_attestation, stored_shape, authorities, review)
    return {
        "schema": RESULT_SCHEMA,
        "status": "PASS",
        "shapeOracleDigest": shape["authorityDigest"],
        "attestationDigest": attestation_digest,
    }


def verify(args: argparse.Namespace) -> dict[str, Any]:
    experiment_root, _ = _experiment_root(args.experiment_root)
    private_inputs = [args.shape_oracle, args.attestation, args.review_manifest]
    private_inputs.extend(
        path
        for path in (args.private_corpus, args.private_benchmark)
        if path is not None
    )
    _require_private_children(experiment_root, private_inputs)
    authorities = load_authorities(
        g1k_path=args.g1k_evidence,
        clew_path=args.clew,
        git_path=args.git,
        experiment_root_path=args.experiment_root,
        corpus_path=args.private_corpus,
        benchmark_path=args.private_benchmark,
    )
    fixture_authority = load_sealed_public_fixture(authorities.git)
    review = load_review_authority(
        args.review_manifest, authorities, args.pilot_runner, fixture_authority
    )
    _, shape, _ = private_json(args.shape_oracle, "INVALID_SHAPE_ORACLE")
    _, attestation, _ = private_json(
        args.attestation, "INVALID_SHAPE_ATTESTATION", 256 * 1024
    )
    validate_shape_oracle(shape, authorities)
    attestation_digest = validate_attestation(attestation, shape, authorities, review)
    return {
        "schema": RESULT_SCHEMA,
        "status": "PASS",
        "shapeOracleDigest": shape["authorityDigest"],
        "attestationDigest": attestation_digest,
    }


def review_inputs(args: argparse.Namespace) -> dict[str, Any]:
    """Emit review ingredients, never a verdict or a review manifest."""

    authorities = load_authorities(
        g1k_path=args.g1k_evidence,
        clew_path=args.clew,
        git_path=args.git,
        experiment_root_path=args.experiment_root,
    )
    _, pilot_runner_digest = regular_file(
        args.pilot_runner, "INVALID_PILOT_RUNNER"
    )
    fixture = load_sealed_public_fixture(authorities.git)
    return {
        "schema": "codeclew-kotlin-descriptor-shape-builder-review-inputs/1.0",
        "builderDigest": file_digest(Path(__file__).resolve()),
        "pilotRunnerDigest": pilot_runner_digest,
        "g1kEvidenceDigest": authorities.g1k_digest,
        "publicFixtureTreeOid": fixture.tree_oid,
        "publicFixtureContentDigest": fixture.content_digest,
        "testDigest": _fast_test_digest(),
        "localModuleManifest": local_module_manifest(),
        "gitDigest": authorities.git_digest,
        "gitEnvironmentDigest": authority_digest(_isolated_git_environment()),
    }


def _synthetic_projection(
    name: str, *, kind: str = "FUNCTION", nullable: bool = False
) -> dict[str, Any]:
    if kind == "CLASS":
        class_id = f"sample/{name}"
        return {
            "symbolIdentity": f"class:{class_id}",
            "declarationKind": "CLASS",
            "ownerIdentity": "package:sample",
            "containment": [],
            "visibility": "public",
            "effectiveVisibility": "public",
            "exportBoundary": "PUBLIC_API",
            "modality": "FINAL",
            "typeParameters": [],
            "compilerClassId": class_id,
        }
    callable_id = f"sample/{name}"
    rendered = "kotlin/String?" if nullable else "kotlin/String"
    return {
        "symbolIdentity": (
            f"callable:{callable_id}#jvm:{name}(Ljava/lang/String;)Ljava/lang/String;"
        ),
        "declarationKind": "FUNCTION",
        "ownerIdentity": "package:sample",
        "containment": [],
        "visibility": "public",
        "effectiveVisibility": "public",
        "exportBoundary": "PUBLIC_API",
        "modality": "FINAL",
        "typeParameters": [],
        "compilerCallableId": callable_id,
        "isOverride": False,
        "returnType": rendered,
        "returnNullable": nullable,
        "parameterTypes": [
            {"index": 0, "type": "kotlin/String", "nullable": False}
        ],
    }


def _synthetic_row(name: str, *, kind: str = "FUNCTION", marker: str = "0") -> dict[str, Any]:
    projected = _synthetic_projection(name, kind=kind, nullable=name == "publicDescriptor")
    return {
        "descriptorClass": "TYPE" if kind == "CLASS" else "CALLABLE",
        "declarationKind": kind,
        "name": name,
        "ownerIdentity": projected["ownerIdentity"],
        "normalizedSignature": projected["symbolIdentity"].split(":", 1)[1],
        "shapeDigest": authority_digest(projected),
        "relativeFile": "src/main/kotlin/sample/Fixture.kt",
        "blobOid": marker * 40,
        "sourceRange": {"startByte": 1, "endByte": 2},
    }


def self_test() -> dict[str, Any]:
    checks = 0
    for descriptor in (1, 2):
        try:
            _run_bounded_process(
                [
                    os.fspath(Path(sys.executable).resolve(strict=True)),
                    "-I",
                    "-S",
                    "-c",
                    (
                        "import os\n"
                        f"while True: os.write({descriptor},b'x'*1048576)"
                    ),
                ],
                environment=_clew_environment(
                    {"HOME": os.fspath(Path(tempfile.gettempdir()).resolve())}
                ),
                timeout=5,
                stdout_limit=1024,
                stderr_limit=1024,
                code="SELF_TEST_LIMIT",
            )
        except BuilderError as error:
            if error.code != "SELF_TEST_LIMIT":
                raise
        else:
            raise BuilderError("SELF_TEST_FAILED")
        checks += 1
    with tempfile.TemporaryDirectory(
        prefix="codeclew-shape-process-self-test-"
    ) as process_directory:
        child_pid_path = Path(process_directory) / "child.pid"
        script = (
            "import pathlib,subprocess,sys\n"
            "child=subprocess.Popen([sys.executable,'-I','-S','-c',"
            "'import time;time.sleep(30)'])\n"
            "pathlib.Path(sys.argv[1]).write_text(str(child.pid),encoding='ascii')"
        )
        try:
            _run_bounded_process(
                [
                    os.fspath(Path(sys.executable).resolve(strict=True)),
                    "-I",
                    "-S",
                    "-c",
                    script,
                    os.fspath(child_pid_path),
                ],
                environment=_clew_environment(
                    {"HOME": os.fspath(Path(tempfile.gettempdir()).resolve())}
                ),
                timeout=5,
                stdout_limit=1024,
                stderr_limit=1024,
                code="SELF_TEST_RESIDUAL",
            )
        except BuilderError as error:
            if error.code != "SELF_TEST_RESIDUAL":
                raise
        else:
            raise BuilderError("SELF_TEST_FAILED")
        child_pid = int(child_pid_path.read_text(encoding="ascii"))
        for _ in range(100):
            try:
                os.kill(child_pid, 0)
            except ProcessLookupError:
                break
            time.sleep(0.02)
        else:
            raise BuilderError("SELF_TEST_FAILED")
        checks += 1
    hostile_environment = {
        "HOME": "/tmp/codeclew-shape-home",
        "CODECLEW_HOME": "/tmp/codeclew-shape-state",
        "PATH": "/usr/bin:/bin",
        "LANG": "hostile-locale",
        "LC_ALL": "hostile-locale",
        "LC_CTYPE": "hostile-locale",
        "JAVA_HOME": "/tmp/codeclew-shape-jdk",
        "CODECLEW_RUNTIME_SEED": "/tmp/hostile-seed",
        "CODECLEW_RUNTIME_CAPSULE": "/tmp/hostile-capsule",
        "JAVA_TOOL_OPTIONS": "-javaagent:/tmp/hostile.jar",
        "JDK_JAVA_OPTIONS": "-Dhostile=true",
        "PYTHONPATH": "/tmp/hostile-python",
        "GRADLE_USER_HOME": "/tmp/hostile-gradle",
        "MAVEN_USER_HOME": "/tmp/hostile-maven",
        "CARGO_HOME": "/tmp/hostile-cargo",
        "RUSTUP_HOME": "/tmp/hostile-rustup",
        "MAVEN_OPTS": "-Dhostile=true",
        "DYLD_INSERT_LIBRARIES": "/tmp/hostile.dylib",
    }
    filtered_environment = _clew_environment(hostile_environment)
    if filtered_environment != {
        "HOME": os.fspath(Path(hostile_environment["HOME"]).resolve(strict=False)),
        "CODECLEW_HOME": os.fspath(
            Path(hostile_environment["CODECLEW_HOME"]).resolve(strict=False)
        ),
        "PATH": f"{Path(sys.executable).resolve(strict=True).parent}:/usr/bin:/bin",
        "JAVA_HOME": os.fspath(Path(hostile_environment["JAVA_HOME"]).resolve(strict=False)),
        "GRADLE_USER_HOME": os.fspath(
            Path(hostile_environment["GRADLE_USER_HOME"]).resolve(strict=False)
        ),
        "MAVEN_USER_HOME": os.fspath(
            Path(hostile_environment["MAVEN_USER_HOME"]).resolve(strict=False)
        ),
        "CARGO_HOME": os.fspath(Path(hostile_environment["CARGO_HOME"]).resolve(strict=False)),
        "RUSTUP_HOME": os.fspath(Path(hostile_environment["RUSTUP_HOME"]).resolve(strict=False)),
        "LANG": "C",
        "LC_ALL": "C",
    }:
        raise BuilderError("SELF_TEST_FAILED")
    for locale_environment in (
        {"HOME": "/tmp/codeclew-shape-home"},
        {
            "HOME": "/tmp/codeclew-shape-home",
            "LANG": "ambient",
            "LC_CTYPE": "ambient",
        },
    ):
        normalized = _clew_environment(locale_environment)
        if (
            normalized.get("LANG") != "C"
            or normalized.get("LC_ALL") != "C"
            or "LC_CTYPE" in normalized
        ):
            raise BuilderError("SELF_TEST_FAILED")
    checks += 1
    runtime_witness = RuntimeWitness()
    runtime_witness.observe(authority_digest("release-runtime"), "RELEASE")
    runtime_witness.observe(authority_digest("release-runtime"), "RELEASE")
    if runtime_witness.require() != authority_digest("release-runtime"):
        raise BuilderError("SELF_TEST_FAILED")
    try:
        runtime_witness.observe(authority_digest("other-runtime"), "RELEASE")
    except BuilderError:
        checks += 1
    else:
        raise BuilderError("SELF_TEST_FAILED")
    try:
        RuntimeWitness().observe(authority_digest("development-runtime"), "DEVELOPMENT")
    except BuilderError:
        checks += 1
    else:
        raise BuilderError("SELF_TEST_FAILED")
    function = _synthetic_projection("publicDescriptor", nullable=True)
    validate_projected_shape(function)
    checks += 1
    finding = {
        "findingId": authority_digest("finding"),
        "side": "PROVIDER",
        "memberAlias": "provider",
        "factId": authority_digest("fact"),
        "authority": "EXACT_PROJECTED_DECLARATION",
        "shapeDigest": authority_digest(function),
        "source": {
            "path": "src/main/kotlin/sample/Fixture.kt",
            "start": 1,
            "end": 2,
            "contentRef": {
                "schema": descriptor_gate.CAS_OBJECT_SCHEMA,
                "objectSchema": "codeclew-repository-input-blob/2.0",
                "digest": authority_digest("source"),
                "size": 1,
            },
        },
        "detail": {
            "kind": "DECLARATION",
            "detail": {
                "declarationKind": "FUNCTION",
                "symbolIdentity": function["symbolIdentity"],
                "compilerCallableId": function["compilerCallableId"],
                "projectedShape": function,
            },
        },
    }
    declaration = declaration_from_finding(finding, member_alias="provider", exact=True)
    if declaration is None or declaration.compiler_name != "publicDescriptor":
        raise BuilderError("SELF_TEST_FAILED")
    checks += 1
    tampered = json.loads(canonical_bytes(finding))
    tampered["detail"]["detail"]["projectedShape"]["returnNullable"] = False
    try:
        declaration_from_finding(tampered, member_alias="provider", exact=True)
    except BuilderError:
        checks += 1
    else:
        raise BuilderError("SELF_TEST_FAILED")

    source_bytes = b"public fun publicDescriptor(value: String): String? = value\n"
    source_start = source_bytes.index(b"publicDescriptor")
    source_end = source_start + len(b"publicDescriptor")
    bound_candidate = CompilerDeclaration(
        member_alias=declaration.member_alias,
        side=declaration.side,
        symbol_identity=declaration.symbol_identity,
        compiler_name=declaration.compiler_name,
        declaration_kind=declaration.declaration_kind,
        descriptor_class=declaration.descriptor_class,
        projected_shape=declaration.projected_shape,
        shape_digest=declaration.shape_digest,
        source={
            "path": declaration.source["path"],
            "start": source_start,
            "end": source_end,
            "contentRef": _cas_blob_reference(source_bytes),
        },
    )
    bound = _bind_candidate_source(bound_candidate, source_bytes, "1" * 40)
    if bound["blobOid"] != "1" * 40 or bound["sourceRange"] != {
        "startByte": source_start,
        "endByte": source_end,
    }:
        raise BuilderError("SELF_TEST_FAILED")
    checks += 1
    invalid_source = CompilerDeclaration(
        **{
            **bound_candidate.__dict__,
            "source": {
                **bound_candidate.source,
                "contentRef": _cas_blob_reference(source_bytes + b"x"),
            },
        }
    )
    try:
        _bind_candidate_source(invalid_source, source_bytes, "1" * 40)
    except BuilderError as error:
        if error.code != "SOURCE_CONTENT_REFERENCE_MISMATCH":
            raise BuilderError("SELF_TEST_FAILED") from error
        checks += 1
    else:
        raise BuilderError("SELF_TEST_FAILED")

    g1k_value = g1k_verifier._valid_fixture()  # type: ignore[attr-defined]
    g1k_verifier.verify_value(g1k_value)
    runtime_digest = g1k_value["executionAuthority"]["clewAuthority"]
    self_test_git_raw = shutil.which("git")
    if self_test_git_raw is None:
        raise BuilderError("SELF_TEST_FAILED")
    self_test_git = git_executable(Path(self_test_git_raw))
    authorities = Authorities(
        None,
        None,
        None,
        None,
        g1k_value,
        authority_digest(g1k_value),
        Path("clew"),
        runtime_digest,
        self_test_git,
        file_digest(self_test_git),
        authority_digest("synthetic-experiment-root"),
    )
    fixture_authority = load_sealed_public_fixture(authorities.git)
    fixture = [
        _synthetic_row("publicDescriptor", marker="1"),
        _synthetic_row("overloadedDescriptor", marker="2"),
        {
            **_synthetic_row("overloadedDescriptor", marker="3"),
            "normalizedSignature": (
                "sample/overloadedDescriptor#jvm:"
                "overloadedDescriptor(I)Ljava/lang/String;"
            ),
        },
        _synthetic_row("genericDescriptor", marker="4"),
        _synthetic_row("Envelope", kind="CLASS", marker="5"),
    ]
    tasks: list[dict[str, Any]] = []
    for index, g1k_task in enumerate(g1k_value["tasks"]):
        manual_count = 8 if index < 4 else 7
        manual = [
            {
                "category": f"CHECK{index + 1:02}{item + 1:02}",
                "requiredCheck": f"VERIFY_CHECK{index + 1:02}{item + 1:02}",
            }
            for item in range(manual_count)
        ]
        sides = []
        for role, marker in (("provider", "a"), ("consumer", "b")):
            declaration_row = _synthetic_row(
                "sampleCallable" if role == "provider" else "SampleType",
                kind="FUNCTION" if role == "provider" else "CLASS",
                marker=marker,
            )
            sides.append(
                {
                    "role": role,
                    "serviceAlias": g1k_task[role],
                    "revision": marker * 40,
                    "approvedFiles": [
                        {
                            "relativeFile": declaration_row["relativeFile"],
                            "blobOid": declaration_row["blobOid"],
                        }
                    ],
                    "exactDeclarations": [declaration_row],
                }
            )
        tasks.append(
            {
                "taskId": g1k_task["taskId"],
                "pairId": g1k_task["pairId"],
                "manualVerification": manual,
                "sides": sides,
            }
        )
    unsigned_shape = {
        "schema": SHAPE_SCHEMA,
        "sourceAuthority": {
            "privateCorpusDigest": descriptor_gate.EXPECTED_CORPUS_DIGEST,
            "benchmarkDigest": descriptor_gate.EXPECTED_BENCHMARK_DIGEST,
            "g1kEvidenceDigest": authorities.g1k_digest,
            "runtimeDigest": runtime_digest,
            "runtimeKey": authority_digest("synthetic-runtime-key"),
            "gitDigest": authorities.git_digest,
            "gitEnvironmentDigest": authority_digest(_isolated_git_environment()),
            "localModuleManifestDigest": local_module_manifest()["authorityDigest"],
            "compilerEnvironmentDigest": authority_digest(_clew_environment()),
            "compilerProjectionSchema": PROJECTION_SCHEMA,
            "publicFixtureTreeOid": fixture_authority.tree_oid,
            "publicFixtureContentDigest": fixture_authority.content_digest,
        },
        "fixture": fixture,
        "tasks": tasks,
    }
    shape = {**unsigned_shape, "authorityDigest": authority_digest(unsigned_shape)}
    validate_shape_oracle(shape, authorities)
    checks += 1
    invalid_shape = json.loads(canonical_bytes(shape))
    invalid_shape["tasks"][0]["sides"][0]["revision"] = "not-an-oid"
    unsigned_invalid = dict(invalid_shape)
    unsigned_invalid.pop("authorityDigest")
    invalid_shape["authorityDigest"] = authority_digest(unsigned_invalid)
    try:
        validate_shape_oracle(invalid_shape, authorities)
    except BuilderError:
        checks += 1
    else:
        raise BuilderError("SELF_TEST_FAILED")
    unsigned_review = {
        "schema": REVIEW_SCHEMA,
        "builderDigest": file_digest(Path(__file__).resolve()),
        "pilotRunnerDigest": authority_digest("synthetic-pilot-runner"),
        "g1kEvidenceDigest": authorities.g1k_digest,
        "publicFixtureTreeOid": fixture_authority.tree_oid,
        "publicFixtureContentDigest": fixture_authority.content_digest,
        "testDigest": authority_digest("synthetic-fast-test"),
        "localModuleManifest": local_module_manifest(),
        "gitDigest": authorities.git_digest,
        "gitEnvironmentDigest": authority_digest(_isolated_git_environment()),
        "verdict": "PASS",
        "findings": [],
    }
    review_value = {
        **unsigned_review,
        "authorityDigest": authority_digest(unsigned_review),
    }
    review = ReviewAuthority(
        review_value,
        review_value["authorityDigest"],
        Path("pilot-runner"),
        review_value["pilotRunnerDigest"],
        review_value["testDigest"],
    )
    validated_review, validated_review_digest = validate_review_manifest_value(
        review_value,
        builder_digest=review_value["builderDigest"],
        pilot_runner_digest=review.value["pilotRunnerDigest"],
        g1k_digest=authorities.g1k_digest,
        fixture=fixture_authority,
        test_digest=review.value["testDigest"],
        module_manifest=review.value["localModuleManifest"],
        git_digest=authorities.git_digest,
        git_environment_digest=authority_digest(_isolated_git_environment()),
    )
    if validated_review != review.value or validated_review_digest != review.digest:
        raise BuilderError("SELF_TEST_FAILED")
    checks += 1
    invalid_review = dict(review_value)
    invalid_review["verdict"] = "FAIL"
    unsigned_invalid_review = dict(invalid_review)
    unsigned_invalid_review.pop("authorityDigest")
    invalid_review["authorityDigest"] = authority_digest(unsigned_invalid_review)
    try:
        validate_review_manifest_value(
            invalid_review,
            builder_digest=review_value["builderDigest"],
            pilot_runner_digest=review.value["pilotRunnerDigest"],
            g1k_digest=authorities.g1k_digest,
            fixture=fixture_authority,
            test_digest=review.value["testDigest"],
            module_manifest=review.value["localModuleManifest"],
            git_digest=authorities.git_digest,
            git_environment_digest=authority_digest(_isolated_git_environment()),
        )
    except BuilderError:
        checks += 1
    else:
        raise BuilderError("SELF_TEST_FAILED")
    unsigned_attestation = {
        "schema": ATTESTATION_SCHEMA,
        "shapeOracleDigest": shape["authorityDigest"],
        "g1kEvidenceDigest": authorities.g1k_digest,
        "runtimeDigest": runtime_digest,
        "runtimeKey": shape["sourceAuthority"]["runtimeKey"],
        "gitDigest": authorities.git_digest,
        "gitEnvironmentDigest": authority_digest(_isolated_git_environment()),
        "localModuleManifestDigest": local_module_manifest()["authorityDigest"],
        "compilerEnvironmentDigest": shape["sourceAuthority"]["compilerEnvironmentDigest"],
        "builderDigest": file_digest(Path(__file__).resolve()),
        "compilerVerification": "PASS",
        "reviewManifestDigest": review.digest,
    }
    attestation = {
        **unsigned_attestation,
        "authorityDigest": authority_digest(unsigned_attestation),
    }
    validate_attestation(attestation, shape, authorities, review)
    checks += 1
    invalid_attestation = dict(attestation)
    invalid_attestation["reviewManifestDigest"] = authority_digest("substituted-review")
    unsigned_invalid_attestation = dict(invalid_attestation)
    unsigned_invalid_attestation.pop("authorityDigest")
    invalid_attestation["authorityDigest"] = authority_digest(
        unsigned_invalid_attestation
    )
    try:
        validate_attestation(invalid_attestation, shape, authorities, review)
    except BuilderError:
        checks += 1
    else:
        raise BuilderError("SELF_TEST_FAILED")
    _validate_thread_closed(
        {
            "schema": "codeclew-thread-lifecycle-result/1.0",
            "threadId": "thread:sample",
            "lifecycle": {
                "schema": "codeclew-thread-lifecycle-entry/1.0",
                "threadId": "thread:sample",
                "threadAuthorityDigest": authority_digest("thread-authority"),
                "sequence": 1,
                "previousEventHash": authority_digest("thread-previous"),
                "status": "CLOSED",
                "eventHash": authority_digest("thread-event"),
                "updatedUnixMs": 1,
            },
        },
        "thread:sample",
    )
    _validate_session_aborted(
        {
            "schema": "codeclew-session-lifecycle-result/1.0",
            "lifecycle": {
                "schema": "codeclew-session-lifecycle-entry/1.0",
                "sessionId": "session:sample",
                "sessionAuthorityDigest": authority_digest("session-authority"),
                "sequence": 1,
                "previousEventHash": authority_digest("session-previous"),
                "status": "ABORTED",
                "eventHash": authority_digest("session-event"),
                "updatedUnixMs": 1,
            },
        },
        "session:sample",
    )
    for resource_kind in ("thread", "session"):
        resource_id = f"{resource_kind}:sample"
        lifecycle = {
            "schema": f"codeclew-{resource_kind}-lifecycle-entry/1.0",
            f"{resource_kind}Id": resource_id,
            f"{resource_kind}AuthorityDigest": authority_digest(
                f"{resource_kind}-authority"
            ),
            "sequence": 2,
            "previousEventHash": authority_digest(f"{resource_kind}-previous"),
            "status": "GARBAGE_COLLECTED",
            "eventHash": authority_digest(f"{resource_kind}-gc-event"),
            "updatedUnixMs": 2,
        }
        result = {
            "schema": f"codeclew-{resource_kind}-gc-result/1.0",
            "lifecycle": lifecycle,
        }
        if resource_kind == "thread":
            result["threadId"] = resource_id
        _validate_garbage_collected(result, resource_kind, resource_id)
    checks += 1
    with tempfile.TemporaryDirectory(prefix="codeclew-shape-self-test-") as directory:
        resource_root = Path(
            tempfile.mkdtemp(prefix="codeclew-shape-fixture-")
        ).resolve(strict=True)
        resource_root.chmod(0o700)
        temporary_identity = _semantic_temp_identity(resource_root)
        runtime_digest = authority_digest("self-test-runtime")
        resource_ledger = (
            Path(directory) / "resources-pending.json"
        ).resolve(strict=False)
        resource_witness = RuntimeWitness()
        resources = SemanticResources(
            Path(sys.executable).resolve(strict=True),
            resource_ledger,
            runtime_digest,
            resource_witness,
            temporary_identity,
        )
        resources.__enter__()
        challenger = SemanticResources(
            Path(sys.executable).resolve(strict=True),
            resource_ledger,
            runtime_digest,
            RuntimeWitness(),
            temporary_identity,
        )
        before_challenge = resource_ledger.read_bytes()
        try:
            challenger.acquire()
        except BuilderError as error:
            if error.code != "SEMANTIC_RECOVERY_REQUIRED":
                raise
        else:
            raise BuilderError("SELF_TEST_FAILED")
        if resource_ledger.read_bytes() != before_challenge:
            raise BuilderError("SELF_TEST_FAILED")
        open_digest = authority_digest("self-test-session-open")
        resources.begin_open("SESSION", open_digest)
        _, persisted_resource, _ = private_json(
            resource_ledger, "SELF_TEST_FAILED", 256 * 1024
        )
        if (
            persisted_resource.get("openInFlight")
            != {"kind": "SESSION", "requestDigest": open_digest}
            or persisted_resource.get("temporaryRoot") != temporary_identity
        ):
            raise BuilderError("SELF_TEST_FAILED")
        dead_resource = dict(persisted_resource)
        dead_resource.pop("authorityDigest")
        dead_resource["ownerPid"] = 99_999_999
        atomic_private_replace(
            resource_ledger,
            {
                **dead_resource,
                "authorityDigest": authority_digest(dead_resource),
            },
        )
        try:
            recover_resource_ledger(
                Path(sys.executable).resolve(strict=True),
                resource_ledger,
                runtime_digest,
            )
        except BuilderError as error:
            if error.code != "OPERATOR_CLEANUP_REQUIRED":
                raise
        else:
            raise BuilderError("SELF_TEST_FAILED")
        if not resource_ledger.exists() or not resource_root.exists():
            raise BuilderError("SELF_TEST_FAILED")
        _remove_semantic_temp(temporary_identity)
        _remove_private_ledger(resource_ledger)
        checks += 1

        partial_first = Path(directory) / "partial-shape.json"
        partial_second = Path(directory) / "partial-attestation.json"
        staged = [
            _stage_private(partial_first, {"value": 1}),
            _stage_private(partial_second, {"value": 2}),
        ]
        publication = _publication_value(staged)
        unsigned_publication = dict(publication)
        unsigned_publication.pop("authorityDigest")
        unsigned_publication["ownerPid"] = 99_999_999
        publication = {
            **unsigned_publication,
            "authorityDigest": authority_digest(unsigned_publication),
        }
        publication_ledger = _publication_ledger_path(staged[0][0])
        _create_private_once(publication_ledger, publication)
        os.link(staged[0][1], staged[0][0], follow_symlinks=False)
        staged[0][1].unlink()
        recover_private_pair(partial_first, partial_second)
        if (
            partial_first.exists()
            or partial_second.exists()
            or publication_ledger.exists()
            or any(temporary.exists() for _, temporary in staged)
        ):
            raise BuilderError("SELF_TEST_FAILED")
        checks += 1
        complete_first = Path(directory) / "complete-shape.json"
        complete_second = Path(directory) / "complete-attestation.json"
        complete_staged = [
            _stage_private(complete_first, {"value": 1}),
            _stage_private(complete_second, {"value": 2}),
        ]
        complete_publication = _publication_value(complete_staged)
        unsigned_complete = dict(complete_publication)
        unsigned_complete.pop("authorityDigest")
        unsigned_complete["ownerPid"] = 99_999_999
        complete_publication = {
            **unsigned_complete,
            "authorityDigest": authority_digest(unsigned_complete),
        }
        complete_ledger = _publication_ledger_path(complete_staged[0][0])
        _create_private_once(complete_ledger, complete_publication)
        for target, temporary in complete_staged:
            os.link(temporary, target, follow_symlinks=False)
            temporary.unlink()
        recover_private_pair(complete_first, complete_second)
        if (
            not complete_first.exists()
            or not complete_second.exists()
            or complete_ledger.exists()
        ):
            raise BuilderError("SELF_TEST_FAILED")
        checks += 1
        failed_first = Path(directory) / "failed-shape.json"
        failed_second = Path(directory) / "failed-attestation.json"
        with mock.patch.object(Path, "unlink", side_effect=OSError("injected unlink failure")):
            try:
                publish_private_pair(
                    failed_first, {"value": 1}, failed_second, {"value": 2}
                )
            except BuilderError:
                pass
            else:
                raise BuilderError("SELF_TEST_FAILED")
        failed_ledger = _publication_ledger_path(
            _output_target(failed_first)
        )
        if not failed_ledger.exists():
            raise BuilderError("SELF_TEST_FAILED")
        _, failed_publication, _ = private_json(
            failed_ledger, "SELF_TEST_FAILED", 256 * 1024
        )
        unsigned_failed = dict(failed_publication)
        unsigned_failed.pop("authorityDigest")
        unsigned_failed["ownerPid"] = 99_999_999
        atomic_private_replace(
            failed_ledger,
            {
                **unsigned_failed,
                "authorityDigest": authority_digest(unsigned_failed),
            },
        )
        recover_private_pair(failed_first, failed_second)
        if failed_first.exists() or failed_second.exists() or failed_ledger.exists():
            raise BuilderError("SELF_TEST_FAILED")
        checks += 1
        first = Path(directory) / "shape.json"
        second = Path(directory) / "attestation.json"
        publish_private_pair(first, {"value": 1}, second, {"value": 2})
        if (
            stat.S_IMODE(first.stat().st_mode) != 0o600
            or stat.S_IMODE(second.stat().st_mode) != 0o600
            or first.read_bytes() != canonical_bytes({"value": 1}) + b"\n"
        ):
            raise BuilderError("SELF_TEST_FAILED")
        try:
            publish_private_pair(first, {"value": 3}, second, {"value": 4})
        except BuilderError:
            checks += 1
        else:
            raise BuilderError("SELF_TEST_FAILED")
    return {
        "schema": RESULT_SCHEMA,
        "status": "PASS",
        "selfTestChecks": checks,
    }


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    build_parser = commands.add_parser("build")
    build_parser.add_argument("--private-corpus", type=Path, required=True)
    build_parser.add_argument("--private-benchmark", type=Path, required=True)
    build_parser.add_argument("--g1k-evidence", type=Path, required=True)
    build_parser.add_argument("--clew", type=Path, required=True)
    build_parser.add_argument("--git", type=Path, required=True)
    build_parser.add_argument("--experiment-root", type=Path, required=True)
    build_parser.add_argument("--shape-oracle", type=Path, required=True)
    build_parser.add_argument("--attestation", type=Path, required=True)
    build_parser.add_argument("--review-manifest", type=Path, required=True)
    build_parser.add_argument("--pilot-runner", type=Path, required=True)
    build_parser.add_argument("--timeout-seconds", type=int, default=900)

    verify_parser = commands.add_parser("verify")
    verify_parser.add_argument("--shape-oracle", type=Path, required=True)
    verify_parser.add_argument("--attestation", type=Path, required=True)
    verify_parser.add_argument("--g1k-evidence", type=Path, required=True)
    verify_parser.add_argument("--clew", type=Path, required=True)
    verify_parser.add_argument("--git", type=Path, required=True)
    verify_parser.add_argument("--experiment-root", type=Path, required=True)
    verify_parser.add_argument("--private-corpus", type=Path)
    verify_parser.add_argument("--private-benchmark", type=Path)
    verify_parser.add_argument("--review-manifest", type=Path, required=True)
    verify_parser.add_argument("--pilot-runner", type=Path, required=True)
    review_parser = commands.add_parser("review-inputs")
    review_parser.add_argument("--g1k-evidence", type=Path, required=True)
    review_parser.add_argument("--clew", type=Path, required=True)
    review_parser.add_argument("--git", type=Path, required=True)
    review_parser.add_argument("--experiment-root", type=Path, required=True)
    review_parser.add_argument("--pilot-runner", type=Path, required=True)
    return root


def main(argv: list[str] | None = None) -> int:
    raw = sys.argv[1:] if argv is None else argv
    try:
        if raw == ["--self-test"]:
            result = self_test()
        else:
            args = parser().parse_args(raw)
            if args.command == "build":
                if not 30 <= args.timeout_seconds <= 3600:
                    raise BuilderError("INVALID_TIMEOUT")
                result = build(args)
            elif args.command == "verify":
                result = verify(args)
            else:
                result = review_inputs(args)
    except BuilderError as error:
        result = {
            "schema": RESULT_SCHEMA,
            "status": "FAIL",
            "reason": error.code,
        }
        print(canonical_bytes(result).decode("utf-8"))
        return 1
    except Exception:
        result = {
            "schema": RESULT_SCHEMA,
            "status": "FAIL",
            "reason": "SHAPE_BUILDER_INTERNAL_FAILURE",
        }
        print(canonical_bytes(result).decode("utf-8"))
        return 1
    print(canonical_bytes(result).decode("utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
