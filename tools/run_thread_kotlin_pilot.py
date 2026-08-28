#!/usr/bin/env python3
"""Prepare, execute, audit, and project the frozen S4K descriptor pilot.

This runner deliberately separates hidden preparation from measured arms.
Private authority, oracle, run, and warm files are caller-owned regular files
with mode 0600.  ``execute`` never receives the oracle through the prompt or
broker.  Paid Codex arms are never retried automatically.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import secrets
import selectors
import shlex
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Callable

sys.path.insert(0, os.fspath(Path(__file__).resolve().parent))

import run_thread_kotlin_descriptor_gate as descriptor_gate
import maven_distribution_authority
import thread_kotlin_pilot_broker as broker
import verify_thread_kotlin_descriptor_gate as g1k_verifier
import verify_thread_kotlin_pilot as public_verifier


PRIVATE_AUTHORITY_SCHEMA = "codeclew-private-kotlin-descriptor-pilot-protocol/2.0"
PRIVATE_ORACLE_SCHEMA = "codeclew-private-kotlin-descriptor-pilot-oracle/1.0"
PRIVATE_SHAPE_ORACLE_SCHEMA = "codeclew-private-kotlin-descriptor-shape-oracle/1.0"
PRIVATE_SHAPE_ATTESTATION_SCHEMA = "codeclew-private-kotlin-descriptor-shape-attestation/1.0"
PRIVATE_RUN_SCHEMA = "codeclew-private-kotlin-descriptor-pilot-run/1.0"
PRIVATE_WARM_ATTESTATION_SCHEMA = "codeclew-private-kotlin-descriptor-warm-attestation/1.0"
PRIVATE_WARM_SCHEMA = "codeclew-private-kotlin-descriptor-warm-run/1.0"
PRIVATE_DRAFT_SCHEMA = "codeclew-private-kotlin-descriptor-pilot-draft/1.0"
BROKER_CANARY_LEDGER_SCHEMA = "codeclew-kotlin-pilot-broker-canary-ledger/1.0"
IMPLEMENTATION_REVIEW_SCHEMA = "codeclew-kotlin-pilot-implementation-review/1.0"
VALUE_REVIEW_SCHEMA = "codeclew-kotlin-pilot-value-review/1.0"
PREPARE_PUBLICATION_SCHEMA = "codeclew-kotlin-pilot-prepare-publication/1.0"
EXECUTE_ADMISSION_SCHEMA = "codeclew-kotlin-pilot-execute-admission/1.0"
CODEX_AUTH_LEASE_SCHEMA = "codeclew-kotlin-pilot-codex-auth-lease/1.0"
CODEX_AUTH_INCIDENT_SCHEMA = "codeclew-kotlin-pilot-codex-auth-incident/1.0"
LOCAL_MODULE_MANIFEST_SCHEMA = "codeclew-kotlin-pilot-local-modules/1.0"
ANSWER_SCHEMA = "codeclew-kotlin-descriptor-pilot-answer/1.0"
FROZEN_AT = "2026-08-27"
MAX_PRIVATE_BYTES = 16 * 1024 * 1024
MAX_JSONL_BYTES = 64 * 1024 * 1024
SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")
SAFE_MODEL = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$")
SAFE_REASONING = {"low", "medium", "high", "xhigh"}
SEMANTIC_ENVIRONMENT_KEYS = {
    "HOME",
    "CODECLEW_HOME",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "XDG_CACHE_HOME",
    "JAVA_HOME",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "GRADLE_USER_HOME",
    "MAVEN_USER_HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "PATH",
}
LOCAL_MODULE_FILES = (
    ("maven_distribution_authority", "maven_distribution_authority.py"),
    ("run_thread_kotlin_descriptor_gate", "run_thread_kotlin_descriptor_gate.py"),
    ("verify_thread_kotlin_descriptor_gate", "verify_thread_kotlin_descriptor_gate.py"),
    ("verify_thread_kotlin_pilot", "verify_thread_kotlin_pilot.py"),
)
CODEX_ENVIRONMENT_KEYS = {
    "CODEX_HOME", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE",
    "USER", "LOGNAME", "TERM", "SSL_CERT_FILE", "SSL_CERT_DIR", "PATH", "SHELL",
}
SEMANTIC_PATH_KEYS = {
    "HOME", "CODECLEW_HOME", "TMPDIR", "XDG_CACHE_HOME", "JAVA_HOME",
    "SSL_CERT_FILE", "SSL_CERT_DIR", "GRADLE_USER_HOME", "MAVEN_USER_HOME",
    "CARGO_HOME", "RUSTUP_HOME",
}
EXACT_FIELDS = {
    "descriptorClass",
    "declarationKind",
    "name",
    "ownerIdentity",
    "normalizedSignature",
    "shapeStatus",
    "shapeDigest",
    "relativeFile",
    "blobOid",
    "sourceRange",
}
ANSWER_FIELDS = {
    "schema",
    "taskId",
    "pairId",
    "arm",
    "members",
    "manualVerification",
    "relationship",
    "httpEndpointEquivalence",
    "compatibility",
}
BUDGETS = {
    "wallMs": 600_000,
    "noncachedInputTokens": 40_000,
    "toolStarts": 40,
    "queryTerms": 16,
    "returnedFacts": 128,
    "selectedFiles": 12,
    "sourceWindows": 24,
    "agentVisibleEvidenceBytes": 8_388_608,
    "answerBytes": 65_536,
    "contextCreates": 1,
    "contextExpansions": 1,
    "singleSemanticCommandMs": 60_000,
}
PROTOCOL = {
    "schema": "codeclew-kotlin-descriptor-pilot-closed-protocol/1.0",
    "frozenAt": FROZEN_AT,
    "taskCount": 10,
    "arms": ["DEFAULT", "CODECLEW"],
    "armOrder": "ODD_DEFAULT_FIRST_EVEN_CODECLEW_FIRST",
    "promptOracleHints": False,
    "brokerTransport": "PRIVATE_ATOMIC_MAILBOX_V1",
    "brokerChildAudit": "MACOS_SEATBELT_V1",
    "brokerSemanticProjection": "MANAGED_IDENTIFIERS_REDACTED_V1",
    "experimentRootPolicy": "FRESH_0700_SINGLE_OWNER_FAIL_STOP_V1",
    "experimentResourceLedger": "PERSISTENT_0600_UNTIL_PROJECT_V1",
    "openRecoveryPolicy": "IN_FLIGHT_OPEN_TERMINATES_AND_QUARANTINES_EXPERIMENT_V1",
    "executeAdmission": "ROOT_SCOPED_RETAINED_OEXCL_V1",
    "terminalPhase": "PROJECT",
    "projectActions": ["DRAFT", "PUBLISH"],
    "implementationReviewGate": "REQUIRED_BEFORE_EXECUTE",
    "valueReviewGate": "REQUIRED_AFTER_DRAFT_BEFORE_PUBLISH",
    "relationshipAuthority": "DECLARED_TOPOLOGY",
    "partialCredit": False,
    "armFailureClasses": [
        "MODEL_OUTPUT", "MODEL_EXIT", "PRODUCT_REFUSAL", "RESOURCE_LIMIT"
    ],
    "runInvalidation": "AMBIGUOUS_AUDIT_AUTHORITY_OR_TEARDOWN_ONLY",
    "cancellationPolicy": "TERMINAL_INVALID_NO_RESUME_NO_RETRY",
    "aggregateResult": "PASS_IFF_ALL_TEN_CODECLEW_TASKS_AND_COMPARATIVE_GATE_PASS",
    "budgets": BUDGETS,
    "denominators": {
        "tasks": 10,
        "declaredMembers": 20,
        "top10RelevantFiles": 20,
        "descriptorSlots": 20,
        "manualCategories": 74,
    },
}

ARM_FAILURE_CLASSES = {
    "MODEL_OUTPUT", "MODEL_EXIT", "PRODUCT_REFUSAL", "RESOURCE_LIMIT"
}
BROKER_PROTOCOL_VIOLATIONS = {
    "BROKER_AUDIT_INCONSISTENT",
    "BROKER_CAPABILITY_INVALID",
    "BROKER_CAPABILITY_MISSING",
    "BROKER_FRAME_INVALID",
    "BROKER_FRAME_LIMIT",
    "BROKER_INTERNAL_FAILURE",
    "BROKER_PROCESS_GROUP_RESIDUAL",
    "BROKER_SESSION_ALREADY_BOUND",
    "BROKER_TRANSPORT_FAILED",
    "INVALID_BROKER_REQUEST",
    "UNKNOWN_BROKER_OPERATION",
}

_ACTIVE_PROCESS: subprocess.Popen[bytes] | None = None
_ACTIVE_BROKER_STOP: threading.Event | None = None
_ACTIVE_BROKER_THREAD: threading.Thread | None = None
_ACTIVE_BROKER_SESSION: broker.BrokerSession | None = None


class PilotError(RuntimeError):
    """A deliberately locator-free pilot failure."""

    def __init__(self, code: str):
        if re.fullmatch(r"[A-Z][A-Z0-9_]{0,63}", code) is None:
            code = "PILOT_INTERNAL_FAILURE"
        super().__init__(code)
        self.code = code


def canonical_bytes(value: Any) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")


def authority_digest(value: Any) -> str:
    return f"sha256:{hashlib.sha256(canonical_bytes(value)).hexdigest()}"


def file_digest(path: Path, maximum: int = 256 * 1024 * 1024) -> str:
    try:
        metadata = path.stat()
    except OSError as error:
        raise PilotError("AUTHORITY_FILE_UNAVAILABLE") from error
    if not stat.S_ISREG(metadata.st_mode) or not 0 < metadata.st_size <= maximum:
        raise PilotError("AUTHORITY_FILE_UNAVAILABLE")
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                digest.update(chunk)
    except OSError as error:
        raise PilotError("AUTHORITY_FILE_UNAVAILABLE") from error
    return f"sha256:{digest.hexdigest()}"


def local_module_manifest() -> dict[str, Any]:
    """Return the closed byte authority for every locally imported helper.

    The broker and schema have their own first-class authority fields.  This
    manifest closes the remaining local Python import graph plus the focused
    harness test whose bytes the implementation review evaluated.
    """

    tools = Path(__file__).resolve(strict=True).parent
    rows = [
        {
            "module": module,
            "digest": file_digest((tools / filename).resolve(strict=True)),
        }
        for module, filename in LOCAL_MODULE_FILES
    ]
    expected_modules = sorted(module for module, _ in LOCAL_MODULE_FILES)
    if [row["module"] for row in rows] != expected_modules:
        raise PilotError("LOCAL_MODULE_AUTHORITY_INVALID")
    unsigned = {
        "schema": LOCAL_MODULE_MANIFEST_SCHEMA,
        "modules": rows,
        "testFileDigest": file_digest(
            (tools / "test_run_thread_kotlin_pilot.py").resolve(strict=True)
        ),
    }
    return {**unsigned, "authorityDigest": authority_digest(unsigned)}


def _validate_local_module_manifest(value: Any) -> dict[str, Any]:
    row = closed(
        value,
        {"schema", "modules", "testFileDigest", "authorityDigest"},
        "LOCAL_MODULE_AUTHORITY_INVALID",
    )
    modules = row["modules"]
    if (
        row["schema"] != LOCAL_MODULE_MANIFEST_SCHEMA
        or not isinstance(modules, list)
        or len(modules) != len(LOCAL_MODULE_FILES)
        or row["testFileDigest"] is None
        or not isinstance(row["testFileDigest"], str)
        or SHA256.fullmatch(row["testFileDigest"]) is None
    ):
        raise PilotError("LOCAL_MODULE_AUTHORITY_INVALID")
    observed: list[str] = []
    for raw in modules:
        item = closed(
            raw,
            {"module", "digest"},
            "LOCAL_MODULE_AUTHORITY_INVALID",
        )
        if (
            not isinstance(item["module"], str)
            or item["module"] not in {module for module, _ in LOCAL_MODULE_FILES}
            or not isinstance(item["digest"], str)
            or SHA256.fullmatch(item["digest"]) is None
        ):
            raise PilotError("LOCAL_MODULE_AUTHORITY_INVALID")
        observed.append(item["module"])
    unsigned = dict(row)
    declared = unsigned.pop("authorityDigest")
    if (
        observed != sorted(module for module, _ in LOCAL_MODULE_FILES)
        or len(set(observed)) != len(observed)
        or declared != authority_digest(unsigned)
    ):
        raise PilotError("LOCAL_MODULE_AUTHORITY_INVALID")
    return row


def _verify_local_module_authority(value: Any) -> dict[str, Any]:
    sealed = _validate_local_module_manifest(value)
    if sealed != local_module_manifest():
        raise PilotError("LOCAL_MODULE_AUTHORITY_CHANGED")
    return sealed


def _duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise PilotError("DUPLICATE_JSON_KEY")
        result[key] = value
    return result


def private_json(path: Path, label: str, maximum: int = MAX_PRIVATE_BYTES) -> tuple[Path, dict[str, Any], bytes]:
    try:
        absolute = path if path.is_absolute() else Path.cwd() / path
        metadata = os.lstat(absolute)
        resolved = absolute.resolve(strict=True)
    except OSError as error:
        raise PilotError(f"INVALID_{label}") from error
    if (
        stat.S_ISLNK(metadata.st_mode)
        or not stat.S_ISREG(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o600
        or metadata.st_uid != os.geteuid()
        or resolved != absolute.absolute()
        or not 0 < metadata.st_size <= maximum
    ):
        raise PilotError(f"INVALID_{label}")
    try:
        raw = resolved.read_bytes()
        value = json.loads(raw, object_pairs_hook=_duplicates)
    except (OSError, json.JSONDecodeError, UnicodeDecodeError) as error:
        raise PilotError(f"INVALID_{label}") from error
    if not isinstance(value, dict) or raw != canonical_bytes(value) + b"\n":
        raise PilotError(f"INVALID_{label}")
    return resolved, value, raw


def checked_json(path: Path, label: str, maximum: int = 512 * 1024) -> tuple[Path, dict[str, Any], bytes]:
    try:
        absolute = path if path.is_absolute() else Path.cwd() / path
        resolved = absolute.resolve(strict=True)
        metadata = os.stat(resolved)
        raw = resolved.read_bytes()
        value = json.loads(raw, object_pairs_hook=_duplicates)
    except (OSError, json.JSONDecodeError, UnicodeDecodeError) as error:
        raise PilotError(f"INVALID_{label}") from error
    if (
        not stat.S_ISREG(metadata.st_mode)
        or not 0 < len(raw) <= maximum
        or not isinstance(value, dict)
        or raw not in {canonical_bytes(value), canonical_bytes(value) + b"\n"}
    ):
        raise PilotError(f"INVALID_{label}")
    return resolved, value, raw


def _bounded_private_answer(path: Path) -> bytes:
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise PilotError("ANSWER_MISSING") from error
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or not 0 < metadata.st_size <= BUDGETS["answerBytes"]
        ):
            raise PilotError("ANSWER_SIZE_INVALID")
        chunks = bytearray()
        while len(chunks) <= BUDGETS["answerBytes"]:
            chunk = os.read(
                descriptor,
                min(65_536, BUDGETS["answerBytes"] + 1 - len(chunks)),
            )
            if not chunk:
                break
            chunks.extend(chunk)
        if len(chunks) != metadata.st_size:
            raise PilotError("ANSWER_SIZE_INVALID")
        return bytes(chunks)
    finally:
        os.close(descriptor)


def _output_locator(path: Path) -> Path:
    absolute = path if path.is_absolute() else Path.cwd() / path
    try:
        parent = absolute.parent.resolve(strict=True)
    except OSError as error:
        raise PilotError("OUTPUT_WRITE_FAILED") from error
    return parent / absolute.name


def _experiment_root(path: Path) -> tuple[Path, dict[str, Any]]:
    try:
        absolute = path if path.is_absolute() else Path.cwd() / path
        metadata = os.lstat(absolute)
        root = absolute.resolve(strict=True)
    except OSError as error:
        raise PilotError("INVALID_EXPERIMENT_ROOT") from error
    if (
        root != absolute.absolute()
        or not stat.S_ISDIR(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or metadata.st_uid != os.geteuid()
    ):
        raise PilotError("INVALID_EXPERIMENT_ROOT")
    for name, environment_key in (
        ("codeclew-state", "CODECLEW_HOME"),
        ("tmp", "TMPDIR"),
    ):
        directory = root / name
        try:
            child_metadata = os.lstat(directory)
            resolved = directory.resolve(strict=True)
        except OSError as error:
            raise PilotError("INVALID_EXPERIMENT_ROOT") from error
        if (
            resolved != directory
            or not stat.S_ISDIR(child_metadata.st_mode)
            or stat.S_IMODE(child_metadata.st_mode) != 0o700
            or child_metadata.st_uid != os.geteuid()
            or os.environ.get(environment_key) != os.fspath(directory)
        ):
            raise PilotError("INVALID_EXPERIMENT_ROOT")
    return root, {
        "path": os.fspath(root),
        "device": metadata.st_dev,
        "inode": metadata.st_ino,
    }


def _require_experiment_paths(root: Path, paths: list[Path]) -> None:
    observed: set[Path] = set()
    for raw in paths:
        target = _output_locator(raw)
        if target.parent != root or target in observed:
            raise PilotError("EXPERIMENT_PATH_AUTHORITY_INVALID")
        observed.add(target)


def output_target(path: Path, private: bool) -> Path:
    target = _output_locator(path)
    try:
        metadata = os.lstat(target)
    except FileNotFoundError:
        return target
    except OSError as error:
        raise PilotError("OUTPUT_WRITE_FAILED") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise PilotError("OUTPUT_WRITE_FAILED")
    if private and (stat.S_IMODE(metadata.st_mode) != 0o600 or metadata.st_uid != os.geteuid()):
        raise PilotError("OUTPUT_WRITE_FAILED")
    return target


def fresh_output_target(path: Path) -> Path:
    """Resolve a caller-owned phase output and refuse every pre-existing node."""

    target = _output_locator(path)
    try:
        os.lstat(target)
    except FileNotFoundError:
        return target
    except OSError as error:
        raise PilotError("OUTPUT_WRITE_FAILED") from error
    raise PilotError("OUTPUT_ALREADY_EXISTS")


def require_distinct_paths(inputs: list[Path], outputs: list[Path]) -> None:
    if len(set(inputs)) != len(inputs) or len(set(outputs)) != len(outputs):
        raise PilotError("OUTPUT_PATH_COLLISION")
    if set(inputs) & set(outputs):
        raise PilotError("OUTPUT_PATH_COLLISION")


def atomic_write(path: Path, value: Any, mode: int) -> None:
    target = output_target(path, mode == 0o600)
    raw = canonical_bytes(value) + b"\n"
    temporary: str | None = None
    try:
        descriptor, temporary = tempfile.mkstemp(prefix=f".{target.name}.", dir=target.parent)
        with os.fdopen(descriptor, "wb", closefd=True) as stream:
            os.fchmod(stream.fileno(), mode)
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
        if stat.S_IMODE(os.stat(target).st_mode) != mode:
            raise PilotError("OUTPUT_WRITE_FAILED")
    except (OSError, PilotError) as error:
        if temporary is not None:
            try:
                os.unlink(temporary)
            except OSError:
                pass
        if isinstance(error, PilotError):
            raise
        raise PilotError("OUTPUT_WRITE_FAILED") from error


def _fsync_directory(path: Path, code: str) -> None:
    try:
        descriptor = os.open(path, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    except OSError as error:
        raise PilotError(code) from error


def _private_identity(metadata: os.stat_result) -> dict[str, int]:
    return {
        "device": metadata.st_dev,
        "inode": metadata.st_ino,
        "size": metadata.st_size,
        "mode": stat.S_IMODE(metadata.st_mode),
        "uid": metadata.st_uid,
    }


def _create_once_private_json(path: Path, value: Any, code: str) -> None:
    """Durably install canonical JSON without ever replacing an existing node."""

    target = fresh_output_target(path)
    creating = fresh_output_target(
        target.with_name(f".{target.name}.creating")
    )
    raw = canonical_bytes(value) + b"\n"
    descriptor: int | None = None
    identity: dict[str, int] | None = None
    try:
        descriptor = os.open(
            creating,
            os.O_WRONLY
            | os.O_CREAT
            | os.O_EXCL
            | getattr(os, "O_NOFOLLOW", 0),
            0o600,
        )
        offset = 0
        while offset < len(raw):
            offset += os.write(descriptor, raw[offset:])
        os.fsync(descriptor)
        identity = _private_identity(os.fstat(descriptor))
        os.close(descriptor)
        descriptor = None
        _fsync_directory(target.parent, code)
        os.link(creating, target, follow_symlinks=False)
        _fsync_directory(target.parent, code)
        os.unlink(creating)
        _fsync_directory(target.parent, code)
    except (OSError, PilotError) as error:
        if descriptor is not None:
            try:
                os.close(descriptor)
            except OSError:
                pass
        if identity is not None:
            try:
                if _private_identity(os.lstat(creating)) == identity:
                    os.unlink(creating)
                    _fsync_directory(target.parent, code)
            except (OSError, PilotError):
                pass
        if isinstance(error, PilotError):
            raise
        raise PilotError(code) from error


def _read_exact_private_file(
    path: Path,
    identity: dict[str, int],
    content_digest: str,
    value_digest: str,
) -> bytes | None:
    """Return bytes only for the exact canonical private inode in the ledger."""

    descriptor: int | None = None
    try:
        metadata = os.lstat(path)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or _private_identity(metadata) != identity
            or identity["mode"] != 0o600
            or identity["uid"] != os.geteuid()
            or not 0 < identity["size"] <= MAX_PRIVATE_BYTES
        ):
            return None
        descriptor = os.open(
            path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
        )
        if _private_identity(os.fstat(descriptor)) != identity:
            return None
        raw = bytearray()
        while len(raw) <= identity["size"]:
            chunk = os.read(descriptor, min(65_536, identity["size"] + 1 - len(raw)))
            if not chunk:
                break
            raw.extend(chunk)
        if len(raw) != identity["size"] or _private_identity(os.fstat(descriptor)) != identity:
            return None
        body = bytes(raw)
        if f"sha256:{hashlib.sha256(body).hexdigest()}" != content_digest:
            return None
        if not body.endswith(b"\n"):
            return None
        value = json.loads(body[:-1], object_pairs_hook=_duplicates)
        if (
            not isinstance(value, dict)
            or canonical_bytes(value) + b"\n" != body
            or authority_digest(value) != value_digest
        ):
            return None
        return body
    except (OSError, json.JSONDecodeError, UnicodeDecodeError, PilotError):
        return None
    finally:
        if descriptor is not None:
            os.close(descriptor)


def _unlink_exact_private_file(
    path: Path,
    identity: dict[str, int],
    content_digest: str,
    value_digest: str,
    code: str,
) -> bool:
    if _read_exact_private_file(path, identity, content_digest, value_digest) is None:
        return False
    try:
        if _private_identity(os.lstat(path)) != identity:
            return False
        os.unlink(path)
        _fsync_directory(path.parent, code)
        return True
    except FileNotFoundError:
        return False
    except OSError as error:
        raise PilotError(code) from error


def closed(value: Any, fields: set[str], code: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != fields:
        raise PilotError(code)
    return value


def executable(path: Path, code: str) -> tuple[Path, str]:
    try:
        resolved = path.resolve(strict=True)
        metadata = resolved.stat()
    except OSError as error:
        raise PilotError(code) from error
    if not stat.S_ISREG(metadata.st_mode) or not os.access(resolved, os.X_OK):
        raise PilotError(code)
    return resolved, file_digest(resolved)


def _python_framework_executable(python: Path) -> tuple[Path, str] | None:
    candidate = (
        python.parent.parent
        / "Resources"
        / "Python.app"
        / "Contents"
        / "MacOS"
        / "Python"
    )
    try:
        candidate.resolve(strict=True)
    except FileNotFoundError:
        return None
    except OSError as error:
        raise PilotError("INVALID_PYTHON_FRAMEWORK_EXECUTABLE") from error
    return executable(candidate, "INVALID_PYTHON_FRAMEWORK_EXECUTABLE")


def _require_python_runtime_authority(executables: dict[str, Any]) -> None:
    python, python_digest = executable(
        Path(executables["python"]), "INVALID_PYTHON_EXECUTABLE"
    )
    declared_framework = executables.get("pythonFramework")
    declared_framework_digest = executables.get("pythonFrameworkDigest")
    observed_framework = _python_framework_executable(python)
    if (
        os.fspath(python) != executables["python"]
        or python_digest != executables["pythonDigest"]
        or (
            observed_framework is None
            and (declared_framework is not None or declared_framework_digest is not None)
        )
        or (
            observed_framework is not None
            and (
                os.fspath(observed_framework[0]) != declared_framework
                or observed_framework[1] != declared_framework_digest
            )
        )
    ):
        raise PilotError("EXECUTABLE_AUTHORITY_CHANGED")


def git_executable() -> tuple[Path, str]:
    raw: str | None = None
    if sys.platform == "darwin" and Path("/usr/bin/xcrun").is_file():
        try:
            completed = subprocess.run(
                ["/usr/bin/xcrun", "--find", "git"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"},
                timeout=10,
                check=False,
                text=True,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise PilotError("INVALID_GIT_EXECUTABLE") from error
        if completed.returncode == 0 and "\n" not in completed.stdout.strip():
            raw = completed.stdout.strip()
    if raw is None:
        raw = shutil.which("git")
    if not raw:
        raise PilotError("INVALID_GIT_EXECUTABLE")
    return executable(Path(raw), "INVALID_GIT_EXECUTABLE")


def maven_executable() -> tuple[Path, str]:
    try:
        authority = maven_distribution_authority.discover()
    except maven_distribution_authority.MavenAuthorityError as error:
        raise PilotError("INVALID_MAVEN_EXECUTABLE") from error
    return authority.executable, authority.digest


def _require_maven_authority(expected_digest: str) -> Path:
    executable_path, observed_digest = maven_executable()
    if observed_digest != expected_digest:
        raise PilotError("MAVEN_AUTHORITY_CHANGED")
    return executable_path


def _seatbelt_path(path: Path) -> str:
    raw = os.fspath(path)
    if not raw.startswith("/") or any(character in raw for character in {'"', "\\", "\n", "\r", "\0"}):
        raise PilotError("BROKER_AUDIT_PATH_INVALID")
    return raw


def _state_root(environment: dict[str, str]) -> Path:
    if environment.get("CODECLEW_HOME"):
        root = Path(environment["CODECLEW_HOME"])
    elif environment.get("XDG_CACHE_HOME"):
        root = Path(environment["XDG_CACHE_HOME"]) / "codeclew"
    elif environment.get("HOME"):
        root = Path(environment["HOME"]) / ".cache" / "codeclew"
    else:
        raise PilotError("BROKER_AUDIT_STATE_UNAVAILABLE")
    try:
        return (root / "v2").resolve(strict=True)
    except OSError as error:
        raise PilotError("BROKER_AUDIT_STATE_UNAVAILABLE") from error


def _effective_cache_roots(
    environment: dict[str, str], state_root: Path, code: str
) -> list[tuple[str, Path]]:
    home = Path(environment["HOME"])
    candidates = [
        ("CODECLEW_DEPENDENCY", state_root / "dependency-cache"),
        ("GRADLE", Path(environment.get("GRADLE_USER_HOME", os.fspath(home / ".gradle")))),
        ("MAVEN", Path(environment.get("MAVEN_USER_HOME", os.fspath(home / ".m2")))),
        ("CARGO", Path(environment.get("CARGO_HOME", os.fspath(home / ".cargo")))),
        ("RUSTUP", Path(environment.get("RUSTUP_HOME", os.fspath(home / ".rustup")))),
    ]
    resolved_home = home.resolve(strict=False)
    result: list[tuple[str, Path]] = []
    for label, raw in candidates:
        if not raw.is_absolute():
            raise PilotError(code)
        path = raw.resolve(strict=False)
        if path in {Path("/"), resolved_home}:
            raise PilotError(code)
        result.append((label, path))
    if len({path for _, path in result}) != len(result):
        raise PilotError(code)
    return result


def _broker_canary_ledger_path(authority_path: Path) -> Path:
    absolute = authority_path if authority_path.is_absolute() else Path.cwd() / authority_path
    return absolute.with_name(f".{absolute.name}.broker-canaries-pending.json")


def _create_private_once(
    path: Path, value: dict[str, Any], code: str, mode: int = 0o600
) -> None:
    raw = canonical_bytes(value) + b"\n"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags, mode)
        with os.fdopen(descriptor, "wb", closefd=True) as stream:
            os.fchmod(stream.fileno(), mode)
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except OSError as error:
        raise PilotError(code) from error


def _write_owned_private_run(
    path: Path,
    value: dict[str, Any],
    owner_pid: int,
    owner_token: str,
) -> None:
    """Checkpoint only the exact create-once run won by this executor."""

    try:
        _, current, _ = private_json(path, "PRIVATE_RUN", MAX_JSONL_BYTES)
    except PilotError as error:
        raise PilotError("PRIVATE_RUN_OWNER_CHANGED") from error
    unsigned = dict(current)
    declared = unsigned.pop("runDigest", None)
    if (
        current.get("ownerPid") != owner_pid
        or current.get("ownerToken") != owner_token
        or declared != authority_digest(unsigned)
        or value.get("ownerPid") != owner_pid
        or value.get("ownerToken") != owner_token
    ):
        raise PilotError("PRIVATE_RUN_OWNER_CHANGED")
    atomic_write(path, value, 0o600)


def _execute_admission_path(experiment_root: Path) -> Path:
    return experiment_root / ".codeclew-s4k-execute-admission.json"


def _admit_execute(
    experiment_root: Path,
    authority: dict[str, Any],
    output: Path,
    owner: dict[str, Any],
) -> tuple[Path, dict[str, Any]]:
    """Create the never-removed, experiment-scoped paid-run admission."""

    marker = _execute_admission_path(experiment_root)
    try:
        root_metadata = os.lstat(experiment_root)
    except OSError as error:
        raise PilotError("EXECUTE_ADMISSION_INVALID") from error
    unsigned = {
        "schema": EXECUTE_ADMISSION_SCHEMA,
        "status": "STARTED",
        "pilotAuthorityDigest": authority.get("authorityDigest"),
        "protocolDigest": authority.get("protocolDigest"),
        "experimentRootAuthority": authority.get("experimentRoot"),
        "privateRunOutputName": output.name,
        "ownerPid": owner.get("ownerPid"),
        "ownerToken": owner.get("ownerToken"),
    }
    if (
        output.parent != experiment_root
        or unsigned["experimentRootAuthority"]
        != {
            "path": os.fspath(experiment_root),
            "device": root_metadata.st_dev,
            "inode": root_metadata.st_ino,
        }
        or not isinstance(unsigned["pilotAuthorityDigest"], str)
        or SHA256.fullmatch(unsigned["pilotAuthorityDigest"]) is None
        or not isinstance(unsigned["protocolDigest"], str)
        or SHA256.fullmatch(unsigned["protocolDigest"]) is None
        or type(unsigned["ownerPid"]) is not int
        or unsigned["ownerPid"] <= 0
        or not isinstance(unsigned["ownerToken"], str)
        or re.fullmatch(r"[0-9a-f]{64}", unsigned["ownerToken"]) is None
        or not output.name
        or output.name in {".", ".."}
    ):
        raise PilotError("EXECUTE_ADMISSION_INVALID")
    sealed = {**unsigned, "admissionDigest": authority_digest(unsigned)}
    # Retained on success and failure: another output name cannot buy retries.
    _create_private_once(marker, sealed, "EXECUTE_ALREADY_ADMITTED")
    return marker, sealed


def _verify_execute_admission(
    experiment_root: Path, authority: dict[str, Any], run_path: Path
) -> dict[str, Any]:
    _, value, _ = private_json(
        _execute_admission_path(experiment_root), "EXECUTE_ADMISSION", 256 * 1024
    )
    row = closed(
        value,
        {
            "schema", "status", "pilotAuthorityDigest", "protocolDigest",
            "experimentRootAuthority", "privateRunOutputName", "ownerPid",
            "ownerToken", "admissionDigest",
        },
        "EXECUTE_ADMISSION_INVALID",
    )
    unsigned = dict(row)
    declared = unsigned.pop("admissionDigest")
    if (
        row["schema"] != EXECUTE_ADMISSION_SCHEMA
        or row["status"] != "STARTED"
        or row["pilotAuthorityDigest"] != authority["authorityDigest"]
        or row["protocolDigest"] != authority["protocolDigest"]
        or row["experimentRootAuthority"] != authority["experimentRoot"]
        or row["privateRunOutputName"] != run_path.name
        or run_path.parent != experiment_root
        or type(row["ownerPid"]) is not int
        or row["ownerPid"] <= 0
        or not isinstance(row["ownerToken"], str)
        or re.fullmatch(r"[0-9a-f]{64}", row["ownerToken"]) is None
        or declared != authority_digest(unsigned)
    ):
        raise PilotError("EXECUTE_ADMISSION_INVALID")
    return row


def _broker_canary_ledger(
    ledger: Path,
    environment: dict[str, str],
    cache_roots: list[tuple[str, Path]],
) -> tuple[dict[str, Any], list[tuple[Path, bytes, str]]]:
    nonce = secrets.token_hex(16)
    rows: list[dict[str, Any]] = []
    specs: list[tuple[Path, bytes, str]] = []
    for label, root in cache_roots:
        try:
            root.mkdir(parents=True, exist_ok=True)
            resolved = root.resolve(strict=True)
        except OSError as error:
            raise PilotError("BROKER_AUDIT_CACHE_AUTHORITY_INVALID") from error
        if resolved != root or not root.is_dir():
            raise PilotError("BROKER_AUDIT_CACHE_AUTHORITY_INVALID")
        name = f".codeclew-s4k-cache-canary-{nonce}-{label.lower()}"
        path = root / name
        body = canonical_bytes(
            {
                "schema": "codeclew-kotlin-pilot-cache-canary/1.0",
                "nonce": nonce,
                "label": label,
                "rootDigest": authority_digest(os.fspath(root)),
            }
        ) + b"\n"
        body_digest = f"sha256:{hashlib.sha256(body).hexdigest()}"
        specs.append((path, body, body_digest))
        rows.append(
            {
                "label": label,
                "root": os.fspath(root),
                "path": os.fspath(path),
                "bodyDigest": body_digest,
                "device": None,
                "inode": None,
                "size": None,
            }
        )
    unsigned = {
        "schema": BROKER_CANARY_LEDGER_SCHEMA,
        "ownerPid": os.getpid(),
        "semanticEnvironmentDigest": authority_digest(environment),
        "sentinels": rows,
    }
    value = {**unsigned, "authorityDigest": authority_digest(unsigned)}
    _create_private_once(ledger, value, "BROKER_AUDIT_CANARY_LEDGER_FAILED")
    return value, specs


def _validate_broker_canary_ledger(
    value: Any,
    environment: dict[str, str],
    cache_roots: list[tuple[str, Path]],
) -> dict[str, Any]:
    row = closed(
        value,
        {
            "schema", "ownerPid", "semanticEnvironmentDigest", "sentinels",
            "authorityDigest",
        },
        "BROKER_AUDIT_CANARY_RECOVERY_FAILED",
    )
    unsigned = dict(row)
    declared = unsigned.pop("authorityDigest")
    sentinels = row["sentinels"]
    if (
        row["schema"] != BROKER_CANARY_LEDGER_SCHEMA
        or declared != authority_digest(unsigned)
        or type(row["ownerPid"]) is not int
        or row["ownerPid"] <= 0
        or row["semanticEnvironmentDigest"] != authority_digest(environment)
        or not isinstance(sentinels, list)
        or len(sentinels) != len(cache_roots)
    ):
        raise PilotError("BROKER_AUDIT_CANARY_RECOVERY_FAILED")
    for sentinel, (label, root) in zip(sentinels, cache_roots, strict=True):
        sentinel = closed(
            sentinel,
            {"label", "root", "path", "bodyDigest", "device", "inode", "size"},
            "BROKER_AUDIT_CANARY_RECOVERY_FAILED",
        )
        path = Path(sentinel["path"]) if isinstance(sentinel["path"], str) else Path(".")
        if (
            sentinel["label"] != label
            or sentinel["root"] != os.fspath(root)
            or not path.is_absolute()
            or path.parent != root
            or not path.name.startswith(".codeclew-s4k-cache-canary-")
            or not isinstance(sentinel["bodyDigest"], str)
            or SHA256.fullmatch(sentinel["bodyDigest"]) is None
            or any(
                sentinel[key] is not None
                and (type(sentinel[key]) is not int or sentinel[key] < 0)
                for key in {"device", "inode", "size"}
            )
            or (
                any(sentinel[key] is None for key in {"device", "inode", "size"})
                and any(sentinel[key] is not None for key in {"device", "inode", "size"})
            )
        ):
            raise PilotError("BROKER_AUDIT_CANARY_RECOVERY_FAILED")
    return row


def _remove_broker_cache_sentinels(
    ledger: Path,
    value: dict[str, Any],
    environment: dict[str, str],
    cache_roots: list[tuple[str, Path]],
) -> None:
    row = _validate_broker_canary_ledger(value, environment, cache_roots)
    for sentinel in row["sentinels"]:
        path = Path(sentinel["path"])
        try:
            metadata = os.lstat(path)
        except FileNotFoundError:
            continue
        except OSError as error:
            raise PilotError("BROKER_AUDIT_CANARY_RECOVERY_FAILED") from error
        if (
            stat.S_ISLNK(metadata.st_mode)
            or not stat.S_ISREG(metadata.st_mode)
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_uid != os.geteuid()
            or metadata.st_size <= 0
            or (
                sentinel["device"] is not None
                and (
                    metadata.st_dev != sentinel["device"]
                    or metadata.st_ino != sentinel["inode"]
                    or metadata.st_size != sentinel["size"]
                )
            )
            or file_digest(path, 4096) != sentinel["bodyDigest"]
        ):
            raise PilotError("BROKER_AUDIT_CANARY_RECOVERY_FAILED")
        try:
            path.unlink()
            directory = os.open(path.parent, os.O_RDONLY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        except OSError as error:
            raise PilotError("BROKER_AUDIT_CANARY_RECOVERY_FAILED") from error
    _remove_pending(ledger)


def _recover_broker_canaries(
    authority_path: Path, environment: dict[str, str], state_root: Path
) -> None:
    ledger = _broker_canary_ledger_path(authority_path)
    try:
        os.lstat(ledger)
    except FileNotFoundError:
        return
    except OSError as error:
        raise PilotError("BROKER_AUDIT_CANARY_RECOVERY_FAILED") from error
    _, value, _ = private_json(ledger, "BROKER_CANARY_LEDGER", 256 * 1024)
    owner = value.get("ownerPid")
    if type(owner) is int and owner != os.getpid():
        try:
            os.kill(owner, 0)
        except ProcessLookupError:
            pass
        except (PermissionError, OSError) as error:
            raise PilotError("BROKER_AUDIT_CANARY_RECOVERY_BUSY") from error
        else:
            raise PilotError("BROKER_AUDIT_CANARY_RECOVERY_BUSY")
    roots = _effective_cache_roots(
        environment, state_root, "BROKER_AUDIT_CACHE_AUTHORITY_INVALID"
    )
    _remove_broker_cache_sentinels(ledger, value, environment, roots)


@contextmanager
def _managed_state_write_canary(state_root: Path) -> Any:
    locks = state_root / "locks"
    try:
        locks.mkdir(parents=True, exist_ok=True)
        locks = locks.resolve(strict=True)
    except OSError as error:
        raise PilotError("BROKER_AUDIT_CANARY_FAILED") from error
    body = ("codeclew-s4k-managed-state-canary:" + secrets.token_hex(16)).encode("ascii")
    path = locks / f".codeclew-s4k-write-canary-{secrets.token_hex(16)}"
    fresh_output_target(path)
    try:
        yield path, body
    finally:
        metadata: os.stat_result | None
        try:
            metadata = os.lstat(path)
        except FileNotFoundError:
            metadata = None
        except OSError as error:
            raise PilotError("BROKER_AUDIT_CANARY_FAILED") from error
        if metadata is not None:
            if (
                not stat.S_ISREG(metadata.st_mode)
                or stat.S_IMODE(metadata.st_mode) != 0o600
                or metadata.st_uid != os.geteuid()
                or metadata.st_size != len(body)
                or path.read_bytes() != body
            ):
                raise PilotError("BROKER_AUDIT_CANARY_FAILED")
            try:
                path.unlink()
                _fsync_directory(path.parent, "BROKER_AUDIT_CANARY_FAILED")
            except OSError as error:
                raise PilotError("BROKER_AUDIT_CANARY_FAILED") from error


@contextmanager
def _broker_cache_canary_context(
    authority_path: Path,
    environment: dict[str, str],
    state_root: Path,
    cache_roots: list[tuple[str, Path]],
) -> Any:
    ledger = _broker_canary_ledger_path(authority_path)
    # Every experiment has a fresh private root.  Existing state is terminal
    # evidence of a prior/ambiguous run and is never imported or resumed.
    fresh_output_target(ledger)
    value, specs = _broker_canary_ledger(ledger, environment, cache_roots)
    paths: list[Path] = []
    try:
        for index, (path, body, body_digest) in enumerate(specs):
            flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
            descriptor = os.open(path, flags, 0o600)
            try:
                os.fchmod(descriptor, 0o600)
                written = os.write(descriptor, body)
                if written != len(body):
                    raise OSError("short cache canary write")
                os.fsync(descriptor)
                metadata = os.fstat(descriptor)
            finally:
                os.close(descriptor)
            if file_digest(path, 4096) != body_digest:
                raise PilotError("BROKER_AUDIT_CANARY_FAILED")
            value["sentinels"][index].update(
                {
                    "device": metadata.st_dev,
                    "inode": metadata.st_ino,
                    "size": metadata.st_size,
                }
            )
            unsigned = dict(value)
            unsigned.pop("authorityDigest")
            value["authorityDigest"] = authority_digest(unsigned)
            atomic_write(ledger, value, 0o600)
            paths.append(path)
        yield paths, authority_digest([row[2] for row in specs])
    except OSError as error:
        raise PilotError("BROKER_AUDIT_CANARY_FAILED") from error
    finally:
        _remove_broker_cache_sentinels(
            ledger, value, environment, cache_roots
        )


def _broker_audit(
    *,
    sandbox_exec: Path,
    clew: Path,
    git: Path,
    python: Path,
    python_framework: tuple[Path, str] | None,
    semantic_environment: dict[str, str],
    repositories: list[Path],
    sessions: list[dict[str, str]],
    authority_path: Path,
) -> dict[str, Any]:
    if sys.platform != "darwin":
        raise PilotError("BROKER_AUDIT_ADAPTER_UNAVAILABLE")
    state_root = _state_root(semantic_environment)
    cache_roots = _effective_cache_roots(
        semantic_environment, state_root, "BROKER_AUDIT_CACHE_AUTHORITY_INVALID"
    )
    observed_framework = _python_framework_executable(python)
    if observed_framework != python_framework:
        raise PilotError("EXECUTABLE_AUTHORITY_CHANGED")
    allowed = {
        clew,
        git,
        python,
        # Seatbelt matches the executable vnode selected by exec.  macOS may
        # enter the system shell through either literal even though user-space
        # path resolution reports /bin/sh, so bind both explicitly.
        Path("/bin/sh"),
        Path("/bin/bash"),
        Path("/usr/bin/dirname").resolve(strict=True),
    }
    python_link = python.parent / "python3"
    try:
        if python_link.resolve(strict=True) == python:
            allowed.add(python_link)
    except OSError:
        pass
    if python_framework is not None:
        allowed.add(python_framework[0])
    for session in sessions:
        runtime_key = session.get("runtimeKey")
        if not isinstance(runtime_key, str) or SHA256.fullmatch(runtime_key) is None:
            raise PilotError("BROKER_AUDIT_RUNTIME_INVALID")
        capsule = state_root / "runtimes" / runtime_key.removeprefix("sha256:") / "bin" / "clew"
        try:
            capsule = capsule.resolve(strict=True)
        except OSError as error:
            raise PilotError("BROKER_AUDIT_RUNTIME_INVALID") from error
        if not capsule.is_file() or not os.access(capsule, os.X_OK):
            raise PilotError("BROKER_AUDIT_RUNTIME_INVALID")
        allowed.add(capsule)
    denied_reads = {path for _, path in cache_roots}
    denied_writes: set[Path] = {path for _, path in cache_roots}
    for repository in repositories:
        resolved = repository.resolve(strict=True)
        denied_reads.update(
            {
                resolved / ".gradle",
                resolved / "build",
                resolved / "target",
                resolved / ".semantic-thread",
            }
        )
        denied_writes.add(resolved)
    with tempfile.TemporaryDirectory(prefix="codeclew-s4k-broker-canary-") as directory, _broker_cache_canary_context(
        authority_path, semantic_environment, state_root, cache_roots
    ) as (cache_canaries, cache_sentinel_digest), _managed_state_write_canary(
        state_root
    ) as (state_write_canary, state_write_body):
        canary_root = Path(directory).resolve(strict=True)
        os.chmod(canary_root, 0o700)
        canary = canary_root / "denied"
        canary.write_bytes(b"canary")
        os.chmod(canary, 0o600)
        denied_reads.add(canary)
        denied_writes.add(canary)
        process_rules = " ".join(
            f'(literal "{_seatbelt_path(path)}")'
            for path in sorted(allowed, key=os.fspath)
        )
        read_rules = "\n".join(
            f'(deny file-read* (subpath "{_seatbelt_path(path)}"))'
            for path in sorted(denied_reads, key=os.fspath)
        )
        write_rules = "\n".join(
            f'(deny file-write* (subpath "{_seatbelt_path(path)}"))'
            for path in sorted(denied_writes, key=os.fspath)
        )
        profile = "\n".join(
            [
                "(version 1)",
                "(allow default)",
                "(deny network*)",
                "(deny process-exec)",
                f"(allow process-exec {process_rules})",
                "(deny file-write*)",
                '(allow file-write* (literal "/dev/null"))',
                f'(allow file-write* (subpath "{_seatbelt_path(state_root)}"))',
                read_rules,
                write_rules,
            ]
        )

        def canary_run(command: list[str]) -> int:
            try:
                process = subprocess.Popen(
                    [os.fspath(sandbox_exec), "-p", profile, *command],
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    env=semantic_environment,
                    start_new_session=True,
                    close_fds=True,
                )
                try:
                    return_code = process.wait(timeout=10)
                except subprocess.TimeoutExpired as error:
                    _kill_group(process)
                    raise PilotError("BROKER_AUDIT_CANARY_FAILED") from error
                if _process_group_exists(process.pid):
                    _kill_group(process)
                    raise PilotError("BROKER_AUDIT_CANARY_FAILED")
                return return_code
            except OSError as error:
                raise PilotError("BROKER_AUDIT_CANARY_FAILED") from error

        network_script = (
            "import errno,socket,sys; "
            "\ntry: s=socket.socket(); s.connect(('127.0.0.1',9))"
            "\nexcept PermissionError as e: "
            "sys.exit(0 if e.errno in {errno.EPERM,errno.EACCES} else 3)"
            "\nexcept OSError: sys.exit(4)"
            "\nsys.exit(5)"
        )
        network_denied = canary_run([os.fspath(python), "-I", "-S", "-c", network_script]) == 0
        allowed_process = canary_run(
            [os.fspath(python), "-I", "-S", "-c", "import sys;sys.exit(0)"]
        ) == 0
        process_denied = canary_run(["/usr/bin/true"]) != 0
        read_script = (
            "import errno,sys; "
            "\ntry: open(sys.argv[1],'rb').read(1)"
            "\nexcept PermissionError as e: sys.exit(0 if e.errno==errno.EPERM else 3)"
            "\nexcept OSError: sys.exit(4)"
            "\nsys.exit(5)"
        )
        cache_denied = all(
            canary_run(
                [os.fspath(python), "-I", "-S", "-c", read_script, os.fspath(path)]
            ) == 0
            for path in cache_canaries
        )
        write_script = (
            "import errno,sys; "
            "\ntry: open(sys.argv[1],'wb').write(b'x')"
            "\nexcept PermissionError as e: sys.exit(0 if e.errno==errno.EPERM else 3)"
            "\nexcept OSError: sys.exit(4)"
            "\nsys.exit(5)"
        )
        write_denied = canary_run(
            [os.fspath(python), "-I", "-S", "-c", write_script, os.fspath(canary)]
        ) == 0
        state_write_script = (
            "import os,sys; body=bytes.fromhex(sys.argv[2]); "
            "fd=os.open(sys.argv[1],os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600); "
            "n=os.write(fd,body); os.fsync(fd); os.close(fd); "
            "sys.exit(0 if n==len(body) else 2)"
        )
        managed_state_write = canary_run(
            [
                os.fspath(python), "-I", "-S", "-c", state_write_script,
                os.fspath(state_write_canary), state_write_body.hex(),
            ]
        ) == 0
    if not (
        allowed_process
        and network_denied
        and process_denied
        and cache_denied
        and write_denied
        and managed_state_write
    ):
        raise PilotError("BROKER_AUDIT_CANARY_FAILED")
    if _python_framework_executable(python) != python_framework:
        raise PilotError("EXECUTABLE_AUTHORITY_CHANGED")
    return {
        "adapter": "MACOS_SEATBELT_V1",
        "profilePolicy": "GLOBAL_WRITE_DENY_MANAGED_STATE_ONLY_V1",
        "sandboxExecutable": os.fspath(sandbox_exec),
        "pythonFrameworkExecutable": (
            os.fspath(python_framework[0]) if python_framework is not None else None
        ),
        "pythonFrameworkDigest": (
            python_framework[1] if python_framework is not None else None
        ),
        "profile": profile,
        "profileDigest": authority_digest(profile),
        "allowedProcessCanaryPassed": True,
        "networkCanaryDenied": True,
        "processCanaryDenied": True,
        "cacheCanaryDenied": True,
        "cacheRootCanaryCount": len(cache_roots),
        "cacheSentinelDigest": cache_sentinel_digest,
        "writeCanaryDenied": True,
        "managedStateWriteCanaryPassed": True,
        "allowedWriteRoots": [
            {
                "label": "CODECLEW_MANAGED_STATE",
                "pathDigest": authority_digest(os.fspath(state_root)),
            }
        ],
        "cacheRoots": [
            {"label": label, "pathDigest": authority_digest(os.fspath(path))}
            for label, path in cache_roots
        ],
    }


def protocol_prompt(generic: str, task: descriptor_gate.Task, revisions: dict[str, str]) -> str:
    if not isinstance(generic, str) or not generic or len(generic.encode("utf-8")) > 16 * 1024:
        raise PilotError("INVALID_PRIVATE_BENCHMARK")
    return (
        f"{generic}\n\n"
        f"Task authority: taskId={task.task_id}; pairId={task.pair_id}; "
        f"scenario={task.scenario};\n"
        f"provider={task.provider}@{revisions[task.provider]};\n"
        f"consumer={task.consumer}@{revisions[task.consumer]}.\n\n"
        "Use only the closed local pilot tool. Begin with `pilot-tool capability`.\n"
        "Return only JSON conforming to the supplied schema.\n"
    )


def _manual_categories(benchmark_value: dict[str, Any], task_row: dict[str, Any]) -> list[dict[str, str]]:
    profile = task_row["manualCategoryProfile"]
    values = benchmark_value["manualVerificationProfiles"][profile]
    if not isinstance(values, list):
        raise PilotError("INVALID_PRIVATE_BENCHMARK")
    rows: list[dict[str, str]] = []
    for value in values:
        if not isinstance(value, str) or re.fullmatch(r"[A-Z][A-Z0-9_]{0,127}", value) is None:
            raise PilotError("INVALID_PRIVATE_BENCHMARK")
        required = value if value.startswith("VERIFY_") else f"VERIFY_{value}"
        category = value.removeprefix("VERIFY_")
        rows.append({"category": category, "requiredCheck": required})
    if len({(row["category"], row["requiredCheck"]) for row in rows}) != len(rows):
        raise PilotError("INVALID_PRIVATE_BENCHMARK")
    return sorted(rows, key=lambda row: (row["category"], row["requiredCheck"]))


def _validate_manual_verification(
    value: Any, expected_count: int, code: str
) -> list[dict[str, str]]:
    if not isinstance(value, list) or len(value) != expected_count:
        raise PilotError(code)
    rows: list[dict[str, str]] = []
    for raw in value:
        row = closed(raw, {"category", "requiredCheck"}, code)
        category = row["category"]
        required = row["requiredCheck"]
        if (
            not isinstance(category, str)
            or re.fullmatch(r"[A-Z][A-Z0-9_]{0,127}", category) is None
            or not isinstance(required, str)
            or required != f"VERIFY_{category}"
        ):
            raise PilotError(code)
        rows.append({"category": category, "requiredCheck": required})
    if rows != sorted(rows, key=lambda row: (row["category"], row["requiredCheck"])):
        raise PilotError(code)
    if len({row["category"] for row in rows}) != len(rows):
        raise PilotError(code)
    return rows


def validate_shape_oracle(
    value: dict[str, Any],
    corpus: descriptor_gate.Corpus,
    benchmark: descriptor_gate.Benchmark,
    benchmark_value: dict[str, Any],
    g1k_digest: str,
    runtime_digest: str,
    public_fixture_tree_oid: str,
    public_fixture_content_digest: str,
    compiler_environment_digest: str,
    git_digest: str,
    git_environment_digest: str,
    local_module_manifest_digest: str,
    maven_digest: str,
) -> dict[str, Any]:
    closed(value, {"schema", "authorityDigest", "sourceAuthority", "fixture", "tasks"}, "INVALID_SHAPE_ORACLE")
    unsigned = dict(value)
    declared = unsigned.pop("authorityDigest")
    if value["schema"] != PRIVATE_SHAPE_ORACLE_SCHEMA or declared != authority_digest(unsigned):
        raise PilotError("INVALID_SHAPE_ORACLE")
    source = closed(
        value["sourceAuthority"],
        {
            "privateCorpusDigest", "benchmarkDigest", "g1kEvidenceDigest",
            "runtimeDigest", "runtimeKey", "compilerProjectionSchema", "publicFixtureTreeOid",
            "publicFixtureContentDigest", "compilerEnvironmentDigest", "gitDigest",
            "gitEnvironmentDigest", "localModuleManifestDigest", "mavenDigest",
        },
        "INVALID_SHAPE_ORACLE",
    )
    if source != {
        "privateCorpusDigest": descriptor_gate.EXPECTED_CORPUS_DIGEST,
        "benchmarkDigest": descriptor_gate.EXPECTED_BENCHMARK_DIGEST,
        "g1kEvidenceDigest": g1k_digest,
        "runtimeDigest": runtime_digest,
        "runtimeKey": source["runtimeKey"],
        "gitDigest": git_digest,
        "gitEnvironmentDigest": git_environment_digest,
        "localModuleManifestDigest": local_module_manifest_digest,
        "compilerProjectionSchema": "codeclew-kotlin-callable-fact/1.0",
        "publicFixtureTreeOid": public_fixture_tree_oid,
        "publicFixtureContentDigest": public_fixture_content_digest,
        "compilerEnvironmentDigest": compiler_environment_digest,
        "mavenDigest": maven_digest,
    } or any(
        not isinstance(source[key], str) or SHA256.fullmatch(source[key]) is None
        for key in {"runtimeDigest", "runtimeKey"}
    ):
        raise PilotError("INVALID_SHAPE_ORACLE")
    fixture = value["fixture"]
    if not isinstance(fixture, list) or len(fixture) != 5:
        raise PilotError("INVALID_SHAPE_ORACLE")
    for row in fixture:
        _validate_exact_declaration(row, private=True)
    if len({authority_digest(row) for row in fixture}) != 5:
        raise PilotError("INVALID_SHAPE_ORACLE")
    tasks = value["tasks"]
    if not isinstance(tasks, list) or len(tasks) != 10:
        raise PilotError("INVALID_SHAPE_ORACLE")
    corpus_tasks = {task.task_id: task for task in corpus.tasks}
    benchmark_tasks = {row["taskId"]: row for row in benchmark_value["tasks"]}
    side_oracles = {(side.task_id, side.role): side for side in benchmark.sides}
    for expected, row in zip(corpus.tasks, tasks, strict=True):
        closed(row, {"taskId", "pairId", "manualVerification", "sides"}, "INVALID_SHAPE_ORACLE")
        if row["taskId"] != expected.task_id or row["pairId"] != expected.pair_id:
            raise PilotError("INVALID_SHAPE_ORACLE")
        manual = _manual_categories(benchmark_value, benchmark_tasks[expected.task_id])
        if row["manualVerification"] != manual:
            raise PilotError("INVALID_SHAPE_ORACLE")
        sides = row["sides"]
        if not isinstance(sides, list) or len(sides) != 2:
            raise PilotError("INVALID_SHAPE_ORACLE")
        slot_classes: set[str] = set()
        for role, side in zip(("provider", "consumer"), sides, strict=True):
            closed(
                side,
                {"role", "serviceAlias", "revision", "approvedFiles", "exactDeclarations"},
                "INVALID_SHAPE_ORACLE",
            )
            expected_alias = getattr(corpus_tasks[expected.task_id], role)
            source_side = side_oracles[(expected.task_id, role)]
            approved = [
                {"relativeFile": navigation.relative_file, "blobOid": navigation.blob_oid}
                for navigation in source_side.navigations
            ]
            if (
                side["role"] != role
                or side["serviceAlias"] != expected_alias
                or side["revision"] != source_side.revision
                or side["approvedFiles"] != approved
                or not isinstance(side["exactDeclarations"], list)
            ):
                raise PilotError("INVALID_SHAPE_ORACLE")
            allowed = {
                (declaration.kind, declaration.name)
                for navigation in source_side.navigations
                for declaration in navigation.declarations
            }
            approved_set = {(item["relativeFile"], item["blobOid"]) for item in approved}
            seen: set[str] = set()
            for declaration in side["exactDeclarations"]:
                _validate_exact_declaration(declaration, private=True)
                oracle_kind = "FUN" if declaration["declarationKind"] == "FUNCTION" else declaration["declarationKind"]
                if (
                    (oracle_kind, declaration["name"]) not in allowed
                    and not (declaration["descriptorClass"] == "TYPE" and any(name == declaration["name"] for _, name in allowed))
                ) or (declaration["relativeFile"], declaration["blobOid"]) not in approved_set:
                    raise PilotError("INVALID_SHAPE_ORACLE")
                identity = authority_digest(declaration)
                if identity in seen:
                    raise PilotError("INVALID_SHAPE_ORACLE")
                seen.add(identity)
                slot_classes.add(declaration["descriptorClass"])
        if slot_classes != {"CALLABLE", "TYPE"}:
            raise PilotError("INVALID_SHAPE_ORACLE")
    if sum(len(task["manualVerification"]) for task in tasks) != 74:
        raise PilotError("INVALID_SHAPE_ORACLE")
    return value


def _validate_exact_declaration(value: Any, *, private: bool) -> dict[str, Any]:
    row = closed(value, EXACT_FIELDS - {"shapeStatus"} if private else EXACT_FIELDS, "INVALID_DECLARATION")
    if private:
        row = dict(row)
        row["shapeStatus"] = "EXACT_PROJECTED_DECLARATION"
    if row["descriptorClass"] not in {"CALLABLE", "TYPE"} or row["declarationKind"] not in {
        "FUNCTION", "CONSTRUCTOR", "CLASS", "PROPERTY", "MUTABLE_PROPERTY"
    }:
        raise PilotError("INVALID_DECLARATION")
    for key in {"name", "ownerIdentity", "normalizedSignature", "relativeFile", "blobOid", "shapeDigest"}:
        if not isinstance(row[key], str) or not row[key]:
            raise PilotError("INVALID_DECLARATION")
    descriptor_gate.safe_relative_kotlin_file(row["relativeFile"])
    if descriptor_gate.GIT_BLOB_OID.fullmatch(row["blobOid"]) is None or SHA256.fullmatch(row["shapeDigest"]) is None:
        raise PilotError("INVALID_DECLARATION")
    source = closed(row["sourceRange"], {"startByte", "endByte"}, "INVALID_DECLARATION")
    if type(source["startByte"]) is not int or type(source["endByte"]) is not int or not 0 <= source["startByte"] < source["endByte"]:
        raise PilotError("INVALID_DECLARATION")
    if not private and row["shapeStatus"] != "EXACT_PROJECTED_DECLARATION":
        raise PilotError("INVALID_DECLARATION")
    return row


def _semantic_tool_executable(
    name: str, source: dict[str, str], code: str
) -> tuple[Path, str]:
    search_path = source.get("PATH")
    if (
        not isinstance(search_path, str)
        or not search_path
        or "\0" in search_path
    ):
        raise PilotError(code)
    try:
        raw = shutil.which(name, path=search_path)
    except (OSError, ValueError) as error:
        raise PilotError(code) from error
    if raw is None:
        raise PilotError(code)
    return executable(Path(raw), code)


def _semantic_path(
    python: Path, maven: Path, rustc: Path, cargo: Path
) -> str:
    return ":".join(
        dict.fromkeys(
            [
                os.fspath(python.parent),
                os.fspath(maven.parent),
                os.fspath(rustc.parent),
                os.fspath(cargo.parent),
                "/usr/bin",
                "/bin",
            ]
        )
    )


def _semantic_environment(python: Path, maven: Path) -> dict[str, str]:
    """Build the only environment allowed to influence private preparation."""

    ambient = dict(os.environ)
    rustc = _semantic_tool_executable(
        "rustc", ambient, "INVALID_RUSTC_EXECUTABLE"
    )
    cargo = _semantic_tool_executable(
        "cargo", ambient, "INVALID_CARGO_EXECUTABLE"
    )
    result = {
        key: value
        for key, value in ambient.items()
        if key in SEMANTIC_ENVIRONMENT_KEYS - {"PATH", "LANG", "LC_ALL"}
    }
    for key in SEMANTIC_PATH_KEYS & set(result):
        path = Path(result[key])
        if not path.is_absolute():
            raise PilotError("INVALID_SEMANTIC_ENVIRONMENT")
        result[key] = os.fspath(path.resolve(strict=False))
    result["PATH"] = _semantic_path(python, maven, rustc[0], cargo[0])
    if (
        _semantic_tool_executable(
            "rustc", result, "INVALID_RUSTC_EXECUTABLE"
        )
        != rustc
        or _semantic_tool_executable(
            "cargo", result, "INVALID_CARGO_EXECUTABLE"
        )
        != cargo
    ):
        raise PilotError("RUST_TOOLCHAIN_AUTHORITY_CHANGED")
    result["LANG"] = "C"
    result["LC_ALL"] = "C"
    _validate_semantic_environment(result, "INVALID_SEMANTIC_ENVIRONMENT")
    return result


def _verify_semantic_environment_authority(
    environment: dict[str, str], executables: dict[str, Any], code: str
) -> None:
    expected: dict[str, Path] = {}
    for name, unavailable_code in (
        ("rustc", "INVALID_RUSTC_EXECUTABLE"),
        ("cargo", "INVALID_CARGO_EXECUTABLE"),
    ):
        path_key = name
        digest_key = f"{name}Digest"
        path_value = executables.get(path_key)
        digest_value = executables.get(digest_key)
        if not isinstance(path_value, str) or not isinstance(digest_value, str):
            raise PilotError(code)
        path, digest = executable(Path(path_value), unavailable_code)
        if (
            os.fspath(path) != path_value
            or digest != digest_value
            or _semantic_tool_executable(name, environment, unavailable_code)
            != (path, digest)
        ):
            raise PilotError("EXECUTABLE_AUTHORITY_CHANGED")
        expected[name] = path
    python = executables.get("python")
    maven = executables.get("maven")
    if not isinstance(python, str) or not isinstance(maven, str):
        raise PilotError(code)
    if environment.get("PATH") != _semantic_path(
        Path(python), Path(maven), expected["rustc"], expected["cargo"]
    ):
        raise PilotError(code)


def _validate_semantic_environment(value: Any, code: str) -> dict[str, str]:
    if (
        not isinstance(value, dict)
        or not set(value).issubset(SEMANTIC_ENVIRONMENT_KEYS)
        or "HOME" not in value
        or "PATH" not in value
        or value.get("LANG") != "C"
        or value.get("LC_ALL") != "C"
        or any(
            not isinstance(key, str)
            or not isinstance(item, str)
            or not item
            or "\0" in item
            for key, item in value.items()
        )
    ):
        raise PilotError(code)
    for key in SEMANTIC_PATH_KEYS & set(value):
        path = Path(value[key])
        if not path.is_absolute() or value[key] != os.fspath(path.resolve(strict=False)):
            raise PilotError(code)
    return value


def _codex_environment(python: Path) -> dict[str, str]:
    result = {
        key: value
        for key, value in os.environ.items()
        if key in CODEX_ENVIRONMENT_KEYS - {"PATH", "SHELL"}
    }
    ambient_home = os.environ.get("HOME")
    if "CODEX_HOME" not in result:
        if not ambient_home:
            raise PilotError("INVALID_CODEX_ENVIRONMENT")
        result["CODEX_HOME"] = os.fspath(Path(ambient_home) / ".codex")
    for key in {"CODEX_HOME", "TMPDIR", "SSL_CERT_FILE", "SSL_CERT_DIR"} & set(result):
        path = Path(result[key])
        if not path.is_absolute():
            raise PilotError("INVALID_CODEX_ENVIRONMENT")
        result[key] = os.fspath(path.resolve(strict=False))
    result["PATH"] = f"{python.parent}:/usr/bin:/bin"
    result["SHELL"] = "/bin/sh"
    if (
        "CODEX_HOME" not in result
        or not set(result).issubset(CODEX_ENVIRONMENT_KEYS)
        or any(not value or "\0" in value for value in result.values())
    ):
        raise PilotError("INVALID_CODEX_ENVIRONMENT")
    return result


def _private_codex_auth_snapshot(
    auth_path: Path,
) -> tuple[bytes, dict[str, int]]:
    descriptor: int | None = None
    try:
        descriptor = os.open(
            auth_path,
            os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
        )
        metadata = os.fstat(descriptor)
        identity = _private_identity(metadata)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or identity["mode"] != 0o600
            or identity["uid"] != os.geteuid()
            or not 0 < identity["size"] <= 1024 * 1024
        ):
            raise PilotError("INVALID_CODEX_AUTHORITY")
        raw = bytearray()
        while len(raw) < identity["size"]:
            chunk = os.read(descriptor, identity["size"] - len(raw))
            if not chunk:
                break
            raw.extend(chunk)
        if (
            len(raw) != identity["size"]
            or _private_identity(os.fstat(descriptor)) != identity
        ):
            raise PilotError("INVALID_CODEX_AUTHORITY")
    except OSError as error:
        raise PilotError("CODEX_AUTH_UNAVAILABLE") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)
    try:
        value = json.loads(bytes(raw), object_pairs_hook=_duplicates)
    except (json.JSONDecodeError, UnicodeDecodeError, PilotError) as error:
        raise PilotError("INVALID_CODEX_AUTHORITY") from error
    if not isinstance(value, dict):
        raise PilotError("INVALID_CODEX_AUTHORITY")
    return bytes(raw), identity


def _private_codex_auth(auth_path: Path) -> bytes:
    return _private_codex_auth_snapshot(auth_path)[0]


def _raw_digest(raw: bytes) -> str:
    return f"sha256:{hashlib.sha256(raw).hexdigest()}"


def _remove_private_codex_node(path: Path) -> None:
    try:
        metadata = os.lstat(path)
    except FileNotFoundError:
        return
    except OSError as error:
        raise PilotError("CODEX_AUTH_CLEANUP_FAILED") from error
    if metadata.st_uid != os.geteuid():
        raise PilotError("CODEX_AUTH_CLEANUP_FAILED")
    try:
        if stat.S_ISDIR(metadata.st_mode):
            for child in path.iterdir():
                _remove_private_codex_node(child)
            path.rmdir()
        else:
            path.unlink()
    except OSError as error:
        raise PilotError("CODEX_AUTH_CLEANUP_FAILED") from error


def _private_codex_home_identity(private_home: Path) -> dict[str, int]:
    try:
        metadata = os.lstat(private_home)
        resolved = private_home.resolve(strict=True)
    except OSError as error:
        raise PilotError("INVALID_CODEX_AUTHORITY") from error
    private_identity = _private_identity(metadata)
    identity = {
        key: private_identity[key] for key in ("device", "inode", "mode", "uid")
    }
    if (
        resolved != private_home
        or not stat.S_ISDIR(metadata.st_mode)
        or identity["mode"] != 0o700
        or identity["uid"] != os.geteuid()
    ):
        raise PilotError("INVALID_CODEX_AUTHORITY")
    return identity


def _prune_private_codex_home(
    private_home: Path, expected_identity: dict[str, int]
) -> None:
    if _private_codex_home_identity(private_home) != expected_identity:
        raise PilotError("INVALID_CODEX_AUTHORITY")
    for child in private_home.iterdir():
        if child.name != "auth.json":
            _remove_private_codex_node(child)
    _private_codex_auth(private_home / "auth.json")
    if _private_codex_home_identity(private_home) != expected_identity:
        raise PilotError("INVALID_CODEX_AUTHORITY")


def _codex_auth_link_identity(identity: dict[str, int]) -> dict[str, int]:
    return {
        key: identity[key] for key in ("device", "inode", "mode", "uid")
    }


def _codex_auth_lease(lease_path: Path) -> dict[str, Any]:
    _, value, _ = private_json(
        lease_path, "CODEX_AUTH_LEASE", 256 * 1024
    )
    row = closed(
        value,
        {
            "schema", "status", "ownerPid", "sourceAuthPath",
            "sourceIdentity", "sourceDigest", "privateHomePath",
            "privateHomeIdentity", "authorityDigest",
        },
        "CODEX_AUTH_LEASE_INVALID",
    )
    unsigned = dict(row)
    declared = unsigned.pop("authorityDigest")
    source_path = Path(row["sourceAuthPath"])
    private_home = Path(row["privateHomePath"])
    if (
        row["schema"] != CODEX_AUTH_LEASE_SCHEMA
        or row["status"] != "ACTIVE"
        or type(row["ownerPid"]) is not int
        or row["ownerPid"] <= 0
        or not source_path.is_absolute()
        or not private_home.is_absolute()
        or private_home.parent != lease_path.parent
        or not private_home.name.startswith(".codeclew-s4k-codex-")
        or not isinstance(row["sourceIdentity"], dict)
        or set(row["sourceIdentity"])
        != {"device", "inode", "mode", "uid"}
        or not isinstance(row["privateHomeIdentity"], dict)
        or set(row["privateHomeIdentity"])
        != {"device", "inode", "mode", "uid"}
        or not isinstance(row["sourceDigest"], str)
        or SHA256.fullmatch(row["sourceDigest"]) is None
        or declared != authority_digest(unsigned)
    ):
        raise PilotError("CODEX_AUTH_LEASE_INVALID")
    return row


def _remove_codex_auth_lease(lease_path: Path) -> None:
    try:
        metadata = os.lstat(lease_path)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_uid != os.geteuid()
        ):
            raise PilotError("CODEX_AUTH_LEASE_INVALID")
        lease_path.unlink()
        _fsync_directory(lease_path.parent, "CODEX_AUTH_CLEANUP_FAILED")
    except OSError as error:
        raise PilotError("CODEX_AUTH_CLEANUP_FAILED") from error


def _codex_auth_incident_path(lease_path: Path) -> Path:
    return lease_path.with_name(f".{lease_path.name}.incident.json")


def _record_codex_auth_incident(
    lease_path: Path,
    lease: dict[str, Any],
    code: str,
    private_raw: bytes | None,
    private_identity: dict[str, int] | None,
    private_error: str | None,
    source_raw: bytes | None,
    source_identity: dict[str, int] | None,
    source_error: str | None,
) -> Path:
    incident_path = _codex_auth_incident_path(lease_path)
    unsigned = {
        "schema": CODEX_AUTH_INCIDENT_SCHEMA,
        "code": code,
        "cleanupPolicy": "REMOVE_PRIVATE_HOME_AND_LEASE",
        "leaseAuthorityDigest": lease["authorityDigest"],
        "expectedSourceIdentity": lease["sourceIdentity"],
        "observedSourceIdentity": (
            _codex_auth_link_identity(source_identity)
            if source_identity is not None
            else None
        ),
        "privateLinkIdentity": (
            _codex_auth_link_identity(private_identity)
            if private_identity is not None
            else None
        ),
        "expectedSourceDigest": lease["sourceDigest"],
        "observedSourceDigest": (
            _raw_digest(source_raw) if source_raw is not None else None
        ),
        "privateDigest": (
            _raw_digest(private_raw) if private_raw is not None else None
        ),
        "privateError": private_error,
        "sourceError": source_error,
    }
    value = {**unsigned, "authorityDigest": authority_digest(unsigned)}
    try:
        os.lstat(incident_path)
    except FileNotFoundError:
        _create_private_once(
            incident_path, value, "CODEX_AUTH_INCIDENT_CREATE_FAILED"
        )
    except OSError as error:
        raise PilotError("CODEX_AUTH_INCIDENT_CREATE_FAILED") from error
    else:
        _, existing, _ = private_json(
            incident_path, "CODEX_AUTH_INCIDENT", 256 * 1024
        )
        existing_unsigned = dict(existing)
        declared = existing_unsigned.pop("authorityDigest", None)
        if (
            existing.get("schema") != CODEX_AUTH_INCIDENT_SCHEMA
            or existing.get("leaseAuthorityDigest") != lease["authorityDigest"]
            or declared != authority_digest(existing_unsigned)
        ):
            raise PilotError("CODEX_AUTH_INCIDENT_INVALID")
    return incident_path


def _terminate_codex_auth_recovery(
    lease_path: Path,
    lease: dict[str, Any],
    private_home: Path,
    code: str,
    private_raw: bytes | None,
    private_identity: dict[str, int] | None,
    private_error: str | None,
    source_raw: bytes | None,
    source_identity: dict[str, int] | None,
    source_error: str | None,
) -> None:
    incident_error: PilotError | None = None
    try:
        _record_codex_auth_incident(
            lease_path,
            lease,
            code,
            private_raw,
            private_identity,
            private_error,
            source_raw,
            source_identity,
            source_error,
        )
    except PilotError as error:
        incident_error = error
    _remove_private_codex_node(private_home)
    _fsync_directory(private_home.parent, "CODEX_AUTH_CLEANUP_FAILED")
    _remove_codex_auth_lease(lease_path)
    if incident_error is not None:
        raise incident_error
    raise PilotError(code)


def _create_private_codex_home(
    source_home: Path, lease_path: Path
) -> tuple[Path, dict[str, int]]:
    if not source_home.is_absolute() or not lease_path.is_absolute():
        raise PilotError("INVALID_CODEX_ENVIRONMENT")
    source_auth = source_home / "auth.json"
    raw, source_identity = _private_codex_auth_snapshot(source_auth)
    source_link_identity = _codex_auth_link_identity(source_identity)
    source_digest = _raw_digest(raw)
    fresh_output_target(lease_path)
    private_home = Path(
        tempfile.mkdtemp(
            prefix=".codeclew-s4k-codex-",
            dir=os.fspath(lease_path.parent),
        )
    ).resolve(strict=True)
    lease_created = False
    try:
        os.chmod(private_home, 0o700)
        identity = _private_codex_home_identity(private_home)
        lease_unsigned = {
            "schema": CODEX_AUTH_LEASE_SCHEMA,
            "status": "ACTIVE",
            "ownerPid": os.getpid(),
            "sourceAuthPath": os.fspath(source_auth),
            "sourceIdentity": source_link_identity,
            "sourceDigest": source_digest,
            "privateHomePath": os.fspath(private_home),
            "privateHomeIdentity": identity,
        }
        _create_private_once(
            lease_path,
            {
                **lease_unsigned,
                "authorityDigest": authority_digest(lease_unsigned),
            },
            "CODEX_AUTH_LEASE_CREATE_FAILED",
        )
        lease_created = True
        try:
            os.link(
                source_auth,
                private_home / "auth.json",
                follow_symlinks=False,
            )
        except OSError as error:
            raise PilotError("CODEX_AUTH_LINK_FAILED") from error
        _fsync_directory(private_home, "CODEX_AUTH_COPY_FAILED")
        linked_raw, linked_identity = _private_codex_auth_snapshot(
            private_home / "auth.json"
        )
        current_raw, current_identity = _private_codex_auth_snapshot(source_auth)
        if (
            linked_raw != raw
            or current_raw != raw
            or _codex_auth_link_identity(linked_identity)
            != source_link_identity
            or _codex_auth_link_identity(current_identity)
            != source_link_identity
        ):
            raise PilotError("CODEX_AUTH_CONCURRENT_UPDATE")
        _prune_private_codex_home(private_home, identity)
        return private_home, identity
    except Exception as primary:
        try:
            _remove_private_codex_node(private_home)
            if lease_created:
                _remove_codex_auth_lease(lease_path)
        except PilotError as cleanup_error:
            raise cleanup_error from primary
        raise


def _recover_private_codex_home(lease_path: Path) -> None:
    lease = _codex_auth_lease(lease_path)
    if lease["ownerPid"] != os.getpid():
        try:
            os.kill(lease["ownerPid"], 0)
        except ProcessLookupError:
            pass
        except (PermissionError, OSError) as error:
            raise PilotError("CODEX_AUTH_RECOVERY_BUSY") from error
        else:
            raise PilotError("CODEX_AUTH_RECOVERY_BUSY")
    private_home = Path(lease["privateHomePath"])
    expected_identity = lease["privateHomeIdentity"]
    try:
        current_private_home_identity = _private_codex_home_identity(private_home)
    except PilotError as error:
        if error.code != "INVALID_CODEX_AUTHORITY" or private_home.exists():
            raise
        _remove_codex_auth_lease(lease_path)
        return
    if current_private_home_identity != expected_identity:
        raise PilotError("CODEX_AUTH_CLEANUP_FAILED")
    for child in private_home.iterdir():
        if child.name != "auth.json":
            _remove_private_codex_node(child)
    if _private_codex_home_identity(private_home) != expected_identity:
        raise PilotError("CODEX_AUTH_CLEANUP_FAILED")
    private_raw: bytes | None = None
    private_identity: dict[str, int] | None = None
    private_error: str | None = None
    source_raw: bytes | None = None
    source_identity: dict[str, int] | None = None
    source_error: str | None = None
    try:
        private_raw, private_identity = _private_codex_auth_snapshot(
            private_home / "auth.json"
        )
    except PilotError as error:
        private_error = error.code
    try:
        source_raw, source_identity = _private_codex_auth_snapshot(
            Path(lease["sourceAuthPath"])
        )
    except PilotError as error:
        source_error = error.code
    if private_error is not None or source_error is not None:
        _terminate_codex_auth_recovery(
            lease_path,
            lease,
            private_home,
            "CODEX_AUTH_RECOVERY_MATERIAL_INVALID",
            private_raw,
            private_identity,
            private_error,
            source_raw,
            source_identity,
            source_error,
        )
    if (
        private_raw is None
        or private_identity is None
        or source_raw is None
        or source_identity is None
    ):
        raise PilotError("CODEX_AUTH_RECOVERY_MATERIAL_INVALID")
    if (
        private_raw != source_raw
        or _codex_auth_link_identity(private_identity)
        != lease["sourceIdentity"]
        or _codex_auth_link_identity(source_identity)
        != lease["sourceIdentity"]
    ):
        _terminate_codex_auth_recovery(
            lease_path,
            lease,
            private_home,
            "CODEX_AUTH_CONCURRENT_UPDATE",
            private_raw,
            private_identity,
            None,
            source_raw,
            source_identity,
            None,
        )
    _remove_private_codex_node(private_home)
    _fsync_directory(private_home.parent, "CODEX_AUTH_CLEANUP_FAILED")
    _remove_codex_auth_lease(lease_path)


def _recover_codex_auth_lease_for_output(output: Path) -> Path:
    lease = output.with_name(f".{output.name}.codex-auth-lease.json")
    try:
        os.lstat(lease)
    except FileNotFoundError:
        return lease
    except OSError as error:
        raise PilotError("CODEX_AUTH_LEASE_INVALID") from error
    _recover_private_codex_home(lease)
    return lease


def _codex_permission_arguments(python: Path) -> list[str]:
    try:
        runtime_root = python.parent.parent.resolve(strict=True)
    except OSError as error:
        raise PilotError("INVALID_PYTHON_EXECUTABLE") from error
    if not runtime_root.is_dir() or not python.is_relative_to(runtime_root):
        raise PilotError("INVALID_PYTHON_EXECUTABLE")
    filesystem = {
        ":root": "deny",
        ":minimal": "read",
        ":workspace_roots": "write",
        os.fspath(runtime_root): "read",
    }
    inline = "{" + ",".join(
        f"{json.dumps(path)}={json.dumps(access)}"
        for path, access in filesystem.items()
    ) + "}"
    return [
        "-c", f"permissions.s4k.filesystem={inline}",
        "-c", "permissions.s4k.network.enabled=false",
        "-c", 'default_permissions="s4k"',
    ]


def _codex_permission_canary(
    codex: Path,
    python: Path,
    experiment_root: Path,
    base_environment: dict[str, str],
) -> dict[str, Any]:
    script = """
import errno
import pathlib
import socket
import sys

secret, marker = map(pathlib.Path, sys.argv[1:])

def denied_read(path):
    try:
        path.read_bytes()
    except PermissionError:
        return True
    return False

def denied_write(path):
    try:
        path.write_bytes(b"altered")
    except PermissionError:
        return True
    return False

network_denied = False
connection = socket.socket()
try:
    connection.connect(("127.0.0.1", 9))
except PermissionError:
    network_denied = True
except OSError as error:
    network_denied = error.errno == errno.EPERM
finally:
    connection.close()
marker.write_bytes(b"ok")
raise SystemExit(
    0 if denied_read(secret) and denied_write(secret) and network_denied else 47
)
"""
    with tempfile.TemporaryDirectory(
        prefix=".codeclew-s4k-canary-workspace-",
        dir=os.fspath(experiment_root),
    ) as workspace_directory, tempfile.TemporaryDirectory(
        prefix=".codeclew-s4k-canary-secret-",
        dir=os.fspath(experiment_root),
    ) as secret_directory:
        workspace = Path(workspace_directory).resolve(strict=True)
        secret_root = Path(secret_directory).resolve(strict=True)
        os.chmod(workspace, 0o700)
        os.chmod(secret_root, 0o700)
        secret = secret_root / "auth-canary.json"
        secret_raw = b'{"canary":"not-a-credential"}'
        secret.write_bytes(secret_raw)
        os.chmod(secret, 0o600)
        marker = workspace / "workspace-write"
        environment = dict(base_environment)
        environment["HOME"] = os.fspath(workspace)
        environment["TMPDIR"] = os.fspath(workspace)
        command = [
            os.fspath(codex),
            "sandbox",
            *_codex_permission_arguments(python),
            "-P", "s4k",
            "-C", os.fspath(workspace),
            os.fspath(python), "-I", "-S", "-c", script,
            os.fspath(secret), os.fspath(marker),
        ]
        try:
            process = subprocess.Popen(
                command,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
                close_fds=True,
                env=environment,
            )
            try:
                return_code = process.wait(timeout=10)
            except subprocess.TimeoutExpired as error:
                _kill_group(process)
                raise PilotError("CODEX_PERMISSION_CANARY_FAILED") from error
        except OSError as error:
            raise PilotError("CODEX_PERMISSION_CANARY_FAILED") from error
        if (
            return_code != 0
            or _process_group_exists(process.pid)
            or secret.read_bytes() != secret_raw
            or marker.read_bytes() != b"ok"
        ):
            _kill_group(process)
            raise PilotError("CODEX_PERMISSION_CANARY_FAILED")
    arguments = _codex_permission_arguments(python)
    return {
        "profile": "S4K_RESTRICTED_WORKSPACE_V1",
        "profileDigest": authority_digest(arguments),
        "credentialReadDenied": True,
        "credentialWriteDenied": True,
        "networkDenied": True,
        "workspaceWritePassed": True,
    }


def _codex_exec_command(
    authority: dict[str, Any],
    scratch: Path,
    schema_path: Path,
    answer_path: Path,
) -> list[str]:
    model = authority["model"]
    return [
        authority["executables"]["codex"],
        "-a", "never", "exec",
        "--strict-config", "--ephemeral", "--ignore-user-config", "--ignore-rules",
        "--skip-git-repo-check", "--json", "--color", "never",
        *_codex_permission_arguments(
            Path(authority["executables"]["python"])
        ),
        "-c", "project_doc_max_bytes=0",
        "-c", "skills.include_instructions=false",
        "-c", "skills.bundled.enabled=false",
        "-c", "include_apps_instructions=false",
        "-c", "include_collaboration_mode_instructions=false",
        "-c", f'model_reasoning_effort="{model["reasoningEffort"]}"',
        "--model", model["modelId"],
        "--cd", os.fspath(scratch),
        "--output-schema", os.fspath(schema_path),
        "--output-last-message", os.fspath(answer_path),
        "-",
    ]


def _private_arm_environment(
    base_environment: dict[str, str],
    scratch: Path,
    private_codex_home: Path,
    private_codex_home_identity: dict[str, int],
) -> dict[str, str]:
    try:
        metadata = scratch.stat()
        resolved = scratch.resolve(strict=True)
    except OSError as error:
        raise PilotError("INVALID_ARM_SCRATCH") from error
    if (
        resolved != scratch
        or not stat.S_ISDIR(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or metadata.st_uid != os.geteuid()
        or private_codex_home == scratch
        or private_codex_home.is_relative_to(scratch)
        or scratch.is_relative_to(private_codex_home)
    ):
        raise PilotError("INVALID_ARM_SCRATCH")
    _prune_private_codex_home(
        private_codex_home, private_codex_home_identity
    )
    environment = dict(base_environment)
    environment["HOME"] = os.fspath(scratch)
    environment["TMPDIR"] = os.fspath(scratch)
    environment["CODEX_HOME"] = os.fspath(private_codex_home)
    return environment


def _arm_scratch_locator(scratch: Path, task_id: str, arm: str) -> dict[str, Any]:
    try:
        metadata = scratch.stat()
    except OSError as error:
        raise PilotError("INVALID_ARM_SCRATCH") from error
    if (
        arm not in {"DEFAULT", "CODECLEW"}
        or re.fullmatch(r"task-[0-9]{2}", task_id) is None
        or not stat.S_ISDIR(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or metadata.st_uid != os.geteuid()
    ):
        raise PilotError("INVALID_ARM_SCRATCH")
    return {
        "taskId": task_id,
        "arm": arm,
        "path": os.fspath(scratch),
        "device": metadata.st_dev,
        "inode": metadata.st_ino,
    }


def _run_json(
    command: list[str],
    timeout_seconds: int,
    code: str,
    environment: dict[str, str],
) -> dict[str, Any]:
    """Run a preparation command with no ambient environment inheritance."""
    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
            close_fds=True,
            env=environment,
        )
    except OSError as error:
        raise PilotError(code) from error
    assert process.stdout is not None
    descriptor = process.stdout.fileno()
    os.set_blocking(descriptor, False)
    selector = selectors.DefaultSelector()
    selector.register(descriptor, selectors.EVENT_READ)
    raw_buffer = bytearray()
    maximum = descriptor_gate.MAX_CLEW_STDOUT_BYTES
    deadline = time.monotonic() + timeout_seconds
    eof = False
    try:
        while not eof:
            if time.monotonic() >= deadline:
                raise PilotError(code)
            events = selector.select(timeout=0.05)
            if not events and process.poll() is not None:
                events = [(None, None)]
            for _, _ in events:
                try:
                    chunk = os.read(
                        descriptor, min(65_536, maximum + 1 - len(raw_buffer))
                    )
                except BlockingIOError:
                    continue
                if not chunk:
                    eof = True
                    break
                raw_buffer.extend(chunk)
                if len(raw_buffer) > maximum:
                    raise PilotError(code)
        try:
            return_code = process.wait(timeout=0.25)
        except subprocess.TimeoutExpired as error:
            raise PilotError(code) from error
        if _process_group_exists(process.pid):
            raise PilotError(code)
    except PilotError as primary:
        if not _kill_group(process):
            raise PilotError("PROCESS_GROUP_RESIDUAL") from primary
        raise
    finally:
        selector.close()
        process.stdout.close()
    raw = bytes(raw_buffer)
    if return_code != 0 or not raw:
        raise PilotError(code)
    try:
        value = json.loads(raw, object_pairs_hook=_duplicates)
    except (json.JSONDecodeError, UnicodeDecodeError, PilotError) as error:
        raise PilotError(code) from error
    if not isinstance(value, dict):
        raise PilotError(code)
    return value


def _parse_thread_open(value: Any) -> tuple[str, str]:
    row = closed(value, {"schema", "status", "thread"}, "SEMANTIC_PREPARATION_FAILED")
    thread = row["thread"]
    if (
        row["schema"] != "codeclew-thread-open/1.0"
        or row["status"] != "OPEN"
        or not isinstance(thread, dict)
        or not isinstance(thread.get("threadId"), str)
        or not thread["threadId"].startswith("thread:")
        or not isinstance(thread.get("authorityDigest"), str)
        or SHA256.fullmatch(thread["authorityDigest"]) is None
    ):
        raise PilotError("SEMANTIC_PREPARATION_FAILED")
    return thread["threadId"], thread["authorityDigest"]


def prepare_semantic(
    corpus: descriptor_gate.Corpus,
    benchmark: descriptor_gate.Benchmark,
    git: Path,
    clew: Path,
    timeout_seconds: int,
    environment: dict[str, str],
    pending: Path,
    expected_runtime_key: str,
    owner_pid: int,
    owner_token: str,
) -> tuple[dict[str, str], dict[str, str], list[dict[str, Any]]]:
    """Prime exact member generations and open immutable pair threads.

    This work is intentionally outside every measured arm. Known resource IDs
    are closed and collected strictly. A crash/failure while an open request is
    in flight retains the private ledger and requires operator cleanup; the
    runner never guesses whether the resource was created.
    """

    sessions: dict[str, str] = {}
    session_rows: list[dict[str, str]] = []
    task_threads: dict[str, str] = {}
    task_thread_authorities: dict[str, str] = {}
    open_in_flight: dict[str, str] | None = None
    side_terms: dict[str, set[str]] = {service.alias: set() for service in corpus.services}
    for side in benchmark.sides:
        side_terms[side.service_alias].update(descriptor_gate.side_query_terms(side))
    _write_pending(
        pending, clew, environment, expected_runtime_key, task_threads, session_rows,
        "PREPARING", owner_pid=owner_pid, owner_token=owner_token, create=True,
    )
    try:
        for service in corpus.services:
            target_ref = descriptor_gate.pinned_target_ref(git, service)
            session_command = [
                os.fspath(clew), "session", "open",
                "--repo", os.fspath(service.repository), "--target-ref", target_ref,
                "--language", "kotlin", "--compilation", descriptor_gate.COMPILATION,
                "--generation-jobs", "1",
            ]
            open_in_flight = {
                "kind": "SESSION",
                "resourceKey": service.alias,
                "requestDigest": authority_digest(session_command),
            }
            _write_pending(
                pending, clew, environment, expected_runtime_key, task_threads,
                session_rows, "PREPARING", open_in_flight=open_in_flight,
                owner_pid=owner_pid, owner_token=owner_token,
            )
            opened = _run_json(
                session_command,
                timeout_seconds,
                "SEMANTIC_PREPARATION_FAILED",
                environment,
            )
            session = descriptor_gate.parse_session_open(opened, service, target_ref)
            if (
                session["runtimeMode"] != "RELEASE"
                or session["runtimeKey"] != expected_runtime_key
            ):
                raise PilotError("SEMANTIC_RUNTIME_AUTHORITY_DIVERGED")
            session_id = session["sessionId"]
            sessions[service.alias] = session_id
            session_rows.append(
                {
                    "serviceAlias": service.alias,
                    "sessionId": session_id,
                    "sessionAuthorityDigest": session["authorityDigest"],
                    "runtimeKey": session["runtimeKey"],
                    "runtimeMode": session["runtimeMode"],
                }
            )
            open_in_flight = None
            _write_pending(
                pending, clew, environment, expected_runtime_key, task_threads,
                session_rows, "PREPARING", owner_pid=owner_pid,
                owner_token=owner_token,
            )
            terms = sorted(side_terms[service.alias])[:16]
            if not terms:
                raise PilotError("SEMANTIC_PREPARATION_FAILED")
            _run_json(
                [
                    os.fspath(clew), "context", "create",
                    "--session", session_id,
                    "--intent", "prime frozen Kotlin descriptor pilot authority",
                    *[part for term in terms for part in ("--term", term)],
                    "--max-roots", "2",
                ],
                timeout_seconds,
                "SEMANTIC_PREPARATION_FAILED",
                environment,
            )
        for task in corpus.tasks:
            thread_command = [
                os.fspath(clew), "thread", "open",
                "--member", f"provider={sessions[task.provider]}",
                "--member", f"consumer={sessions[task.consumer]}",
                "--service-alias", f"provider={task.provider}",
                "--service-alias", f"consumer={task.consumer}",
            ]
            open_in_flight = {
                "kind": "THREAD",
                "resourceKey": task.task_id,
                "requestDigest": authority_digest(thread_command),
            }
            _write_pending(
                pending, clew, environment, expected_runtime_key, task_threads,
                session_rows, "PREPARING", open_in_flight=open_in_flight,
                owner_pid=owner_pid, owner_token=owner_token,
            )
            value = _run_json(
                thread_command,
                min(timeout_seconds, 300),
                "SEMANTIC_PREPARATION_FAILED",
                environment,
            )
            thread_id, thread_authority_digest = _parse_thread_open(value)
            task_threads[task.task_id] = thread_id
            task_thread_authorities[task.task_id] = thread_authority_digest
            open_in_flight = None
            _write_pending(
                pending, clew, environment, expected_runtime_key, task_threads,
                session_rows, "PREPARING", owner_pid=owner_pid,
                owner_token=owner_token,
            )
        runtime_keys = {row["runtimeKey"] for row in session_rows}
        if len(runtime_keys) != 1:
            raise PilotError("SEMANTIC_RUNTIME_AUTHORITY_DIVERGED")
        return task_threads, task_thread_authorities, session_rows
    except Exception as primary:
        if open_in_flight is not None:
            raise PilotError("OPERATOR_CLEANUP_REQUIRED") from primary
        try:
            _cleanup_semantic(
                clew, task_threads, session_rows, environment, pending=pending,
                runtime_key=expected_runtime_key,
            )
            _remove_pending(pending)
        except PilotError as cleanup_error:
            raise cleanup_error from primary
        raise


def _pending_path(authority_path: Path) -> Path:
    absolute = authority_path if authority_path.is_absolute() else Path.cwd() / authority_path
    return absolute.with_name(f".{absolute.name}.semantic-pending.json")


def _write_pending(
    pending: Path,
    clew: Path,
    environment: dict[str, str],
    runtime_key: str,
    task_threads: dict[str, str],
    sessions: list[dict[str, str]],
    status: str,
    *,
    open_in_flight: dict[str, str] | None = None,
    owner_pid: int | None = None,
    owner_token: str | None = None,
    create: bool = False,
) -> None:
    if status not in {"PREPARING", "ACTIVE", "CLEANING", "CLEANED"}:
        raise PilotError("SEMANTIC_RECOVERY_FAILED")
    if open_in_flight is not None and (
        not isinstance(open_in_flight, dict)
        or set(open_in_flight) != {"kind", "resourceKey", "requestDigest"}
        or open_in_flight["kind"] not in {"SESSION", "THREAD"}
        or not isinstance(open_in_flight["resourceKey"], str)
        or SAFE_MODEL.fullmatch(open_in_flight["resourceKey"]) is None
        or not isinstance(open_in_flight["requestDigest"], str)
        or SHA256.fullmatch(open_in_flight["requestDigest"]) is None
    ):
        raise PilotError("SEMANTIC_RECOVERY_FAILED")
    existing: dict[str, Any] | None = None
    try:
        os.lstat(pending)
    except FileNotFoundError:
        pass
    except OSError as error:
        raise PilotError("SEMANTIC_RECOVERY_FAILED") from error
    else:
        try:
            _, existing, _ = private_json(
                pending, "SEMANTIC_PENDING", 256 * 1024
            )
        except PilotError as error:
            raise PilotError("SEMANTIC_RECOVERY_FAILED") from error
    if existing is not None:
        if create:
            raise PilotError("SEMANTIC_RESOURCE_BUSY")
        existing_pid = existing.get("ownerPid")
        existing_token = existing.get("ownerToken")
        if (
            type(existing_pid) is not int
            or existing_pid <= 0
            or not isinstance(existing_token, str)
            or re.fullmatch(r"[0-9a-f]{64}", existing_token) is None
            or (owner_pid is not None and owner_pid != existing_pid)
            or (owner_token is not None and owner_token != existing_token)
        ):
            raise PilotError("SEMANTIC_RESOURCE_OWNER_MISMATCH")
        owner_pid = existing_pid
        owner_token = existing_token
    else:
        if owner_pid is None:
            owner_pid = os.getpid()
        if owner_token is None:
            owner_token = secrets.token_hex(32)
        if (
            type(owner_pid) is not int
            or owner_pid <= 0
            or not isinstance(owner_token, str)
            or re.fullmatch(r"[0-9a-f]{64}", owner_token) is None
        ):
            raise PilotError("SEMANTIC_RESOURCE_OWNER_MISMATCH")
    value = {
            "schema": "codeclew-kotlin-pilot-semantic-resource-ledger/5.0",
            "status": status,
            "ownerPid": owner_pid,
            "ownerToken": owner_token,
            "clew": os.fspath(clew),
            "clewDigest": file_digest(clew),
            "semanticEnvironment": environment,
            "runtimeKey": runtime_key,
            "threads": task_threads,
            "sessions": sessions,
            "openInFlight": open_in_flight,
        }
    if existing is None:
        _create_private_once(
            pending, value, "SEMANTIC_RESOURCE_CREATE_FAILED"
        )
    else:
        atomic_write(pending, value, 0o600)


def _remove_pending(pending: Path) -> None:
    try:
        pending.unlink()
        descriptor = os.open(pending.parent, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    except OSError as error:
        raise PilotError("SEMANTIC_RECOVERY_FAILED") from error


def _parse_thread_close(value: Any, thread_id: str) -> None:
    row = closed(
        value,
        {"schema", "threadId", "lifecycle"},
        "SEMANTIC_CLEANUP_FAILED",
    )
    lifecycle = closed(
        row["lifecycle"],
        {
            "schema", "threadId", "threadAuthorityDigest", "sequence",
            "previousEventHash", "status", "eventHash", "updatedUnixMs",
        },
        "SEMANTIC_CLEANUP_FAILED",
    )
    if (
        row["schema"] != "codeclew-thread-lifecycle-result/1.0"
        or row["threadId"] != thread_id
        or lifecycle["schema"] != "codeclew-thread-lifecycle-entry/1.0"
        or lifecycle["threadId"] != thread_id
        or lifecycle["status"] != "CLOSED"
        or not isinstance(lifecycle["threadAuthorityDigest"], str)
        or SHA256.fullmatch(lifecycle["threadAuthorityDigest"]) is None
        or type(lifecycle["sequence"]) is not int
        or lifecycle["sequence"] < 1
        or (
            lifecycle["previousEventHash"] is not None
            and (
                not isinstance(lifecycle["previousEventHash"], str)
                or SHA256.fullmatch(lifecycle["previousEventHash"]) is None
            )
        )
        or not isinstance(lifecycle["eventHash"], str)
        or SHA256.fullmatch(lifecycle["eventHash"]) is None
        or type(lifecycle["updatedUnixMs"]) is not int
        or lifecycle["updatedUnixMs"] <= 0
    ):
        raise PilotError("SEMANTIC_CLEANUP_FAILED")


def _parse_session_abort(value: Any, session_id: str) -> None:
    row = closed(
        value,
        {"schema", "lifecycle"},
        "SEMANTIC_CLEANUP_FAILED",
    )
    lifecycle = closed(
        row["lifecycle"],
        {
            "schema", "sessionId", "sessionAuthorityDigest", "sequence",
            "previousEventHash", "status", "eventHash", "updatedUnixMs",
        },
        "SEMANTIC_CLEANUP_FAILED",
    )
    if (
        row["schema"] != "codeclew-session-lifecycle-result/1.0"
        or lifecycle["schema"] != "codeclew-session-lifecycle-entry/1.0"
        or lifecycle["sessionId"] != session_id
        or lifecycle["status"] != "ABORTED"
        or not isinstance(lifecycle["sessionAuthorityDigest"], str)
        or SHA256.fullmatch(lifecycle["sessionAuthorityDigest"]) is None
        or type(lifecycle["sequence"]) is not int
        or lifecycle["sequence"] < 1
        or (
            lifecycle["previousEventHash"] is not None
            and (
                not isinstance(lifecycle["previousEventHash"], str)
                or SHA256.fullmatch(lifecycle["previousEventHash"]) is None
            )
        )
        or not isinstance(lifecycle["eventHash"], str)
        or SHA256.fullmatch(lifecycle["eventHash"]) is None
        or type(lifecycle["updatedUnixMs"]) is not int
        or lifecycle["updatedUnixMs"] <= 0
    ):
        raise PilotError("SEMANTIC_CLEANUP_FAILED")


def _parse_gc(value: Any, resource_kind: str, resource_id: str) -> None:
    if resource_kind == "thread":
        top = closed(
            value, {"schema", "threadId", "lifecycle"},
            "SEMANTIC_CLEANUP_FAILED",
        )
        if top["threadId"] != resource_id:
            raise PilotError("SEMANTIC_CLEANUP_FAILED")
    elif resource_kind == "session":
        top = closed(
            value, {"schema", "lifecycle"}, "SEMANTIC_CLEANUP_FAILED"
        )
    else:
        raise PilotError("SEMANTIC_CLEANUP_FAILED")
    identity = f"{resource_kind}Id"
    authority = f"{resource_kind}AuthorityDigest"
    lifecycle = closed(
        top["lifecycle"],
        {
            "schema", identity, authority, "sequence", "previousEventHash",
            "status", "eventHash", "updatedUnixMs",
        },
        "SEMANTIC_CLEANUP_FAILED",
    )
    previous = lifecycle["previousEventHash"]
    if (
        top["schema"] != f"codeclew-{resource_kind}-gc-result/1.0"
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
        raise PilotError("SEMANTIC_CLEANUP_FAILED")


def _cleanup_semantic(
    clew: Path,
    task_threads: dict[str, str],
    sessions: list[dict[str, str]],
    environment: dict[str, str],
    *,
    pending: Path | None = None,
    runtime_key: str | None = None,
) -> None:
    remaining_threads = dict(task_threads)
    remaining_sessions = list(sessions)
    if pending is not None and runtime_key is None:
        raise PilotError("SEMANTIC_CLEANUP_FAILED")

    def checkpoint(status: str) -> None:
        if pending is not None:
            assert runtime_key is not None
            _write_pending(
                pending, clew, environment, runtime_key, remaining_threads,
                remaining_sessions, status,
            )

    checkpoint("CLEANING")
    for task_id, thread_id in reversed(list(remaining_threads.items())):
        try:
            value = _run_json(
                [os.fspath(clew), "thread", "close", "--thread", thread_id],
                120,
                "SEMANTIC_CLEANUP_FAILED",
                environment,
            )
            _parse_thread_close(value, thread_id)
        except PilotError as error:
            if error.code == "PROCESS_GROUP_RESIDUAL":
                raise
            # Recovery after a crash may observe an already-closed or already-
            # collected resource.  Exact idempotent GC below is authoritative.
            pass
        collected = _run_json(
            [os.fspath(clew), "thread", "gc", "--thread", thread_id],
            120,
            "SEMANTIC_CLEANUP_FAILED",
            environment,
        )
        _parse_gc(collected, "thread", thread_id)
        remaining_threads.pop(task_id)
        checkpoint("CLEANING")
    for row in reversed(list(remaining_sessions)):
        try:
            value = _run_json(
                [os.fspath(clew), "session", "abort", "--session", row["sessionId"]],
                120,
                "SEMANTIC_CLEANUP_FAILED",
                environment,
            )
            _parse_session_abort(value, row["sessionId"])
        except PilotError as error:
            if error.code == "PROCESS_GROUP_RESIDUAL":
                raise
            pass
        collected = _run_json(
            [
                os.fspath(clew), "session", "gc", "--session",
                row["sessionId"],
            ],
            120,
            "SEMANTIC_CLEANUP_FAILED",
            environment,
        )
        _parse_gc(collected, "session", row["sessionId"])
        remaining_sessions.remove(row)
        checkpoint("CLEANING")
    checkpoint("CLEANED")


def _read_resource_ledger(
    pending: Path, clew: Path, code: str
) -> tuple[dict[str, Any], dict[str, str], list[dict[str, str]]]:
    try:
        metadata = os.lstat(pending)
    except OSError as error:
        raise PilotError(code) from error
    if not stat.S_ISREG(metadata.st_mode) or stat.S_IMODE(metadata.st_mode) != 0o600:
        raise PilotError(code)
    _, value, _ = private_json(pending, "SEMANTIC_PENDING", 256 * 1024)
    closed(
        value,
        {
            "schema", "status", "ownerPid", "ownerToken", "clew", "clewDigest", "semanticEnvironment",
            "runtimeKey", "threads", "sessions", "openInFlight",
        },
        code,
    )
    environment = _validate_semantic_environment(
        value["semanticEnvironment"], code
    )
    threads = value["threads"]
    sessions = value["sessions"]
    if (
        value["schema"]
        != "codeclew-kotlin-pilot-semantic-resource-ledger/5.0"
        or value["status"] not in {"PREPARING", "ACTIVE", "CLEANING", "CLEANED"}
        or type(value["ownerPid"]) is not int
        or value["ownerPid"] <= 0
        or not isinstance(value["ownerToken"], str)
        or re.fullmatch(r"[0-9a-f]{64}", value["ownerToken"]) is None
        or value["clew"] != os.fspath(clew)
        or value["clewDigest"] != file_digest(clew)
        or not isinstance(value["runtimeKey"], str)
        or SHA256.fullmatch(value["runtimeKey"]) is None
        or not isinstance(threads, dict)
        or any(
            not isinstance(task_id, str)
            or re.fullmatch(r"task-[0-9]{2}", task_id) is None
            or not isinstance(thread_id, str)
            or not thread_id.startswith("thread:")
            for task_id, thread_id in threads.items()
        )
        or not isinstance(sessions, list)
        or any(
            not isinstance(row, dict)
            or set(row)
            != {
                "serviceAlias", "sessionId", "sessionAuthorityDigest", "runtimeKey",
                "runtimeMode",
            }
            or not isinstance(row["serviceAlias"], str)
            or SAFE_MODEL.fullmatch(row["serviceAlias"]) is None
            or not isinstance(row["sessionId"], str)
            or not row["sessionId"].startswith("session:")
            or not isinstance(row["sessionAuthorityDigest"], str)
            or SHA256.fullmatch(row["sessionAuthorityDigest"]) is None
            or not isinstance(row["runtimeKey"], str)
            or SHA256.fullmatch(row["runtimeKey"]) is None
            or row["runtimeKey"] != value["runtimeKey"]
            or row["runtimeMode"] != "RELEASE"
            for row in sessions
        )
        or (
            value["openInFlight"] is not None
            and (
                not isinstance(value["openInFlight"], dict)
                or set(value["openInFlight"])
                != {"kind", "resourceKey", "requestDigest"}
                or value["openInFlight"].get("kind") not in {"SESSION", "THREAD"}
                or not isinstance(value["openInFlight"].get("resourceKey"), str)
                or SAFE_MODEL.fullmatch(value["openInFlight"]["resourceKey"])
                is None
                or not isinstance(value["openInFlight"].get("requestDigest"), str)
                or SHA256.fullmatch(value["openInFlight"]["requestDigest"]) is None
            )
        )
    ):
        raise PilotError(code)
    if value["status"] == "CLEANED" and (
        threads or sessions or value["openInFlight"] is not None
    ):
        raise PilotError(code)
    return value, dict(threads), list(sessions)


def _prepare_publication_path(authority_path: Path) -> Path:
    absolute = authority_path if authority_path.is_absolute() else Path.cwd() / authority_path
    return absolute.with_name(f".{absolute.name}.prepare-publication.json")


def _prepare_publication_creating_path(publication: Path) -> Path:
    return publication.with_name(f".{publication.name}.creating")


def _publication_owner_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except OSError as error:
        raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED") from error


def _valid_private_identity(value: Any) -> bool:
    return (
        isinstance(value, dict)
        and set(value) == {"device", "inode", "size", "mode", "uid"}
        and all(type(value[key]) is int for key in value)
        and value["device"] >= 0
        and value["inode"] > 0
        and 0 < value["size"] <= MAX_PRIVATE_BYTES
        and value["mode"] == 0o600
        and value["uid"] == os.geteuid()
    )


def _stage_prepare_value(
    final_path: Path,
    role: str,
    value: dict[str, Any],
    transaction_id: str,
) -> dict[str, Any]:
    raw = canonical_bytes(value) + b"\n"
    if not 0 < len(raw) <= MAX_PRIVATE_BYTES:
        raise PilotError("PREPARE_PUBLICATION_FAILED")
    stage = final_path.with_name(
        f".{final_path.name}.prepare-{transaction_id}-{role.lower()}.stage"
    )
    fresh_output_target(stage)
    descriptor: int | None = None
    identity: dict[str, int] | None = None
    try:
        descriptor = os.open(
            stage,
            os.O_WRONLY
            | os.O_CREAT
            | os.O_EXCL
            | getattr(os, "O_NOFOLLOW", 0),
            0o600,
        )
        offset = 0
        while offset < len(raw):
            offset += os.write(descriptor, raw[offset:])
        os.fsync(descriptor)
        identity = _private_identity(os.fstat(descriptor))
        if identity["size"] != len(raw) or identity["mode"] != 0o600:
            raise PilotError("PREPARE_PUBLICATION_FAILED")
        os.close(descriptor)
        descriptor = None
        _fsync_directory(stage.parent, "PREPARE_PUBLICATION_FAILED")
        return {
            "role": role,
            "finalPath": os.fspath(final_path),
            "stagePath": os.fspath(stage),
            "contentDigest": f"sha256:{hashlib.sha256(raw).hexdigest()}",
            "valueDigest": authority_digest(value),
            "stageIdentity": identity,
            "finalIdentity": None,
        }
    except (OSError, PilotError) as error:
        if descriptor is not None:
            try:
                os.close(descriptor)
            except OSError:
                pass
        if identity is not None:
            try:
                metadata = os.lstat(stage)
                if _private_identity(metadata) == identity:
                    os.unlink(stage)
                    _fsync_directory(stage.parent, "PREPARE_PUBLICATION_FAILED")
            except (OSError, PilotError):
                pass
        if isinstance(error, PilotError):
            raise
        raise PilotError("PREPARE_PUBLICATION_FAILED") from error


def _sealed_prepare_publication(unsigned: dict[str, Any]) -> dict[str, Any]:
    return {**unsigned, "ledgerDigest": authority_digest(unsigned)}


def _read_prepare_publication(
    path: Path,
    authority_path: Path,
    oracle_path: Path,
) -> dict[str, Any]:
    _, value, _ = private_json(path, "PREPARE_PUBLICATION", 256 * 1024)
    unsigned = dict(value)
    declared = unsigned.pop("ledgerDigest", None)
    closed(
        unsigned,
        {
            "schema", "status", "transactionId", "ownerPid", "ownerToken",
            "resourceLedgerPath", "resourceLedgerDigest", "outputs",
        },
        "PREPARE_PUBLICATION_RECOVERY_FAILED",
    )
    outputs = unsigned["outputs"]
    if (
        unsigned["schema"] != PREPARE_PUBLICATION_SCHEMA
        or unsigned["status"]
        not in {"STAGED", "PUBLISHING", "OUTPUTS_DURABLE"}
        or not isinstance(unsigned["transactionId"], str)
        or re.fullmatch(r"[0-9a-f]{64}", unsigned["transactionId"]) is None
        or type(unsigned["ownerPid"]) is not int
        or unsigned["ownerPid"] <= 0
        or not isinstance(unsigned["ownerToken"], str)
        or re.fullmatch(r"[0-9a-f]{64}", unsigned["ownerToken"]) is None
        or unsigned["resourceLedgerPath"]
        != os.fspath(_pending_path(authority_path))
        or not isinstance(unsigned["resourceLedgerDigest"], str)
        or SHA256.fullmatch(unsigned["resourceLedgerDigest"]) is None
        or not isinstance(outputs, list)
        or len(outputs) != 2
        or declared != authority_digest(unsigned)
    ):
        raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED")
    expected = [
        ("PILOT_ORACLE", oracle_path),
        ("PILOT_AUTHORITY", authority_path),
    ]
    for output, (role, final_path) in zip(outputs, expected, strict=True):
        if not isinstance(output, dict) or set(output) != {
            "role", "finalPath", "stagePath", "contentDigest", "valueDigest",
            "stageIdentity", "finalIdentity",
        }:
            raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED")
        stage_path = final_path.with_name(
            f".{final_path.name}.prepare-{unsigned['transactionId']}-{role.lower()}.stage"
        )
        if (
            output["role"] != role
            or output["finalPath"] != os.fspath(final_path)
            or output["stagePath"] != os.fspath(stage_path)
            or not isinstance(output["contentDigest"], str)
            or SHA256.fullmatch(output["contentDigest"]) is None
            or not isinstance(output["valueDigest"], str)
            or SHA256.fullmatch(output["valueDigest"]) is None
            or not _valid_private_identity(output["stageIdentity"])
            or (
                output["finalIdentity"] is not None
                and output["finalIdentity"] != output["stageIdentity"]
            )
        ):
            raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED")
    return value


def _begin_prepare_publication(
    authority_path: Path,
    authority: dict[str, Any],
    oracle_path: Path,
    oracle: dict[str, Any],
    pending: Path,
    clew: Path,
) -> tuple[Path, dict[str, Any]]:
    publication = fresh_output_target(_prepare_publication_path(authority_path))
    resource, _, _ = _read_resource_ledger(
        pending, clew, "PREPARE_PUBLICATION_FAILED"
    )
    if resource["status"] != "PREPARING" or resource["openInFlight"] is not None:
        raise PilotError("PREPARE_PUBLICATION_FAILED")
    transaction_id = secrets.token_hex(32)
    outputs: list[dict[str, Any]] = []
    try:
        outputs.append(
            _stage_prepare_value(oracle_path, "PILOT_ORACLE", oracle, transaction_id)
        )
        outputs.append(
            _stage_prepare_value(
                authority_path, "PILOT_AUTHORITY", authority, transaction_id
            )
        )
        unsigned = {
            "schema": PREPARE_PUBLICATION_SCHEMA,
            "status": "STAGED",
            "transactionId": transaction_id,
            "ownerPid": os.getpid(),
            "ownerToken": secrets.token_hex(32),
            "resourceLedgerPath": os.fspath(pending),
            "resourceLedgerDigest": file_digest(pending),
            "outputs": outputs,
        }
        ledger = _sealed_prepare_publication(unsigned)
        _create_once_private_json(
            publication, ledger, "PREPARE_PUBLICATION_FAILED"
        )
        return publication, ledger
    except Exception:
        for output in reversed(outputs):
            _unlink_exact_private_file(
                Path(output["stagePath"]),
                output["stageIdentity"],
                output["contentDigest"],
                output["valueDigest"],
                "PREPARE_PUBLICATION_FAILED",
            )
        raise


def _publication_file_exact(path: Path, output: dict[str, Any]) -> bool:
    return _read_exact_private_file(
        path,
        output["stageIdentity"],
        output["contentDigest"],
        output["valueDigest"],
    ) is not None


def _publication_file_missing(path: Path) -> bool:
    try:
        os.lstat(path)
        return False
    except FileNotFoundError:
        return True
    except OSError as error:
        raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED") from error


def _publication_can_finalize(ledger: dict[str, Any]) -> bool:
    for output in ledger["outputs"]:
        final = Path(output["finalPath"])
        stage = Path(output["stagePath"])
        if _publication_file_exact(final, output):
            continue
        if not _publication_file_missing(final):
            return False
        if not _publication_file_exact(stage, output):
            return False
    return True


def _write_prepare_publication(path: Path, ledger: dict[str, Any]) -> dict[str, Any]:
    unsigned = dict(ledger)
    unsigned.pop("ledgerDigest", None)
    sealed = _sealed_prepare_publication(unsigned)
    atomic_write(path, sealed, 0o600)
    return sealed


def _fsync_exact_private_file(path: Path, output: dict[str, Any], code: str) -> None:
    descriptor: int | None = None
    try:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        if _private_identity(os.fstat(descriptor)) != output["stageIdentity"]:
            raise PilotError(code)
        os.fsync(descriptor)
    except OSError as error:
        raise PilotError(code) from error
    finally:
        if descriptor is not None:
            os.close(descriptor)


def _publication_resource(
    ledger: dict[str, Any], clew: Path
) -> tuple[Path, dict[str, Any], dict[str, str], list[dict[str, str]]]:
    pending = Path(ledger["resourceLedgerPath"])
    resource, threads, sessions = _read_resource_ledger(
        pending, clew, "PREPARE_PUBLICATION_RECOVERY_FAILED"
    )
    preparing = dict(resource)
    preparing["status"] = "PREPARING"
    expected_digest = f"sha256:{hashlib.sha256(canonical_bytes(preparing) + b'\n').hexdigest()}"
    if (
        resource["status"] not in {"PREPARING", "ACTIVE"}
        or resource["openInFlight"] is not None
        or expected_digest != ledger["resourceLedgerDigest"]
    ):
        raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED")
    return pending, resource, threads, sessions


def _remove_prepare_publication(
    publication: Path, ledger: dict[str, Any], code: str
) -> None:
    try:
        _, current, raw = private_json(
            publication, "PREPARE_PUBLICATION", 256 * 1024
        )
        metadata = os.lstat(publication)
    except PilotError as error:
        raise PilotError(code) from error
    if current != ledger:
        raise PilotError(code)
    if not _unlink_exact_private_file(
        publication,
        _private_identity(metadata),
        f"sha256:{hashlib.sha256(raw).hexdigest()}",
        authority_digest(current),
        code,
    ):
        raise PilotError(code)


def _complete_prepare_publication(
    publication: Path, ledger: dict[str, Any], clew: Path
) -> None:
    pending, resource, threads, sessions = _publication_resource(ledger, clew)
    for index, output in enumerate(ledger["outputs"]):
        final = Path(output["finalPath"])
        stage = Path(output["stagePath"])
        if not _publication_file_exact(final, output):
            if not _publication_file_missing(final) or not _publication_file_exact(stage, output):
                raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED")
            try:
                os.link(stage, final, follow_symlinks=False)
            except FileExistsError:
                if not _publication_file_exact(final, output):
                    raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED")
            except OSError as error:
                raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED") from error
        _fsync_exact_private_file(final, output, "PREPARE_PUBLICATION_RECOVERY_FAILED")
        _fsync_directory(final.parent, "PREPARE_PUBLICATION_RECOVERY_FAILED")
        output["finalIdentity"] = dict(output["stageIdentity"])
        ledger["status"] = (
            "OUTPUTS_DURABLE" if index == len(ledger["outputs"]) - 1 else "PUBLISHING"
        )
        ledger = _write_prepare_publication(publication, ledger)
    if not all(
        _publication_file_exact(Path(output["finalPath"]), output)
        for output in ledger["outputs"]
    ):
        raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED")
    # Re-read after both links and directory fsyncs so publication cannot erase
    # a concurrent resource-ledger drift observed between staging and commit.
    current_pending, resource, threads, sessions = _publication_resource(
        ledger, clew
    )
    if current_pending != pending:
        raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED")
    if resource["status"] == "PREPARING":
        _write_pending(
            pending,
            clew,
            resource["semanticEnvironment"],
            resource["runtimeKey"],
            threads,
            sessions,
            "ACTIVE",
        )
    if not all(
        _publication_file_exact(Path(output["finalPath"]), output)
        for output in ledger["outputs"]
    ):
        raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED")
    for output in ledger["outputs"]:
        stage = Path(output["stagePath"])
        if _publication_file_missing(stage):
            continue
        if not _unlink_exact_private_file(
            stage,
            output["stageIdentity"],
            output["contentDigest"],
            output["valueDigest"],
            "PREPARE_PUBLICATION_RECOVERY_FAILED",
        ):
            raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED")
    _remove_prepare_publication(
        publication, ledger, "PREPARE_PUBLICATION_RECOVERY_FAILED"
    )


def _rollback_prepare_publication(
    publication: Path, ledger: dict[str, Any]
) -> None:
    for output in reversed(ledger["outputs"]):
        final = Path(output["finalPath"])
        if not _publication_file_missing(final):
            _unlink_exact_private_file(
                final,
                output["stageIdentity"],
                output["contentDigest"],
                output["valueDigest"],
                "PREPARE_PUBLICATION_RECOVERY_FAILED",
            )
        stage = Path(output["stagePath"])
        if not _publication_file_missing(stage) and not _unlink_exact_private_file(
            stage,
            output["stageIdentity"],
            output["contentDigest"],
            output["valueDigest"],
            "PREPARE_PUBLICATION_RECOVERY_FAILED",
        ):
            # A stage path is unguessable and transaction-owned.  If its exact
            # identity is gone, preserve the ledger for operator diagnosis.
            raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED")
    _remove_prepare_publication(
        publication, ledger, "PREPARE_PUBLICATION_RECOVERY_FAILED"
    )


def _publish_prepare_pair(
    authority_path: Path,
    authority: dict[str, Any],
    oracle_path: Path,
    oracle: dict[str, Any],
    pending: Path,
    clew: Path,
) -> None:
    publication, ledger = _begin_prepare_publication(
        authority_path, authority, oracle_path, oracle, pending, clew
    )
    try:
        _complete_prepare_publication(publication, ledger, clew)
    except Exception as primary:
        try:
            latest = _read_prepare_publication(
                publication, authority_path, oracle_path
            )
            _rollback_prepare_publication(publication, latest)
        except PilotError as rollback_error:
            raise rollback_error from primary
        raise


def _recover_prepare_publication(
    clew: Path, authority_path: Path, oracle_path: Path
) -> str | None:
    authority_target = _output_locator(authority_path)
    oracle_target = _output_locator(oracle_path)
    publication = _prepare_publication_path(authority_target)
    creating = _prepare_publication_creating_path(publication)
    try:
        os.lstat(publication)
        source = publication
    except FileNotFoundError:
        try:
            os.lstat(creating)
            source = creating
        except FileNotFoundError:
            return None
        except OSError as error:
            raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED") from error
    except OSError as error:
        raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED") from error
    ledger = _read_prepare_publication(
        source, authority_target, oracle_target
    )
    if _publication_owner_alive(ledger["ownerPid"]):
        raise PilotError("PREPARE_PUBLICATION_BUSY")
    if source == creating:
        try:
            os.link(creating, publication, follow_symlinks=False)
            _fsync_directory(
                publication.parent, "PREPARE_PUBLICATION_RECOVERY_FAILED"
            )
        except OSError as error:
            raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED") from error
    try:
        creating_metadata = os.lstat(creating)
    except FileNotFoundError:
        pass
    except OSError as error:
        raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED") from error
    else:
        try:
            if _private_identity(creating_metadata) != _private_identity(
                os.lstat(publication)
            ):
                raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED")
            os.unlink(creating)
            _fsync_directory(
                publication.parent, "PREPARE_PUBLICATION_RECOVERY_FAILED"
            )
        except OSError as error:
            raise PilotError("PREPARE_PUBLICATION_RECOVERY_FAILED") from error
    if _publication_can_finalize(ledger):
        _complete_prepare_publication(publication, ledger, clew)
        return "FINALIZED"
    _rollback_prepare_publication(publication, ledger)
    return "ROLLED_BACK"


def _recover_pending(clew: Path, authority_path: Path) -> None:
    pending = _pending_path(authority_path)
    try:
        os.lstat(pending)
    except FileNotFoundError:
        return
    except OSError as error:
        raise PilotError("SEMANTIC_RECOVERY_FAILED") from error
    value, threads, sessions = _read_resource_ledger(
        pending, clew, "SEMANTIC_RECOVERY_FAILED"
    )
    if value["openInFlight"] is not None:
        raise PilotError("OPERATOR_CLEANUP_REQUIRED")
    if value["status"] != "CLEANED":
        _cleanup_semantic(
            clew,
            threads,
            sessions,
            value["semanticEnvironment"],
            pending=pending,
            runtime_key=value["runtimeKey"],
        )
    _remove_pending(pending)


def _verify_resource_ledger(
    authority_path: Path, authority: dict[str, Any]
) -> tuple[Path, dict[str, Any]]:
    pending = _pending_path(authority_path)
    value, threads, sessions = _read_resource_ledger(
        pending, Path(authority["executables"]["clew"]),
        "SEMANTIC_RECOVERY_REQUIRED",
    )
    expected_threads = {
        task["taskId"]: task["thread"]["threadId"] for task in authority["tasks"]
    }
    if (
        value["status"] != "ACTIVE"
        or value["openInFlight"] is not None
        or value["ownerPid"] != authority["resourceOwner"]["ownerPid"]
        or value["ownerToken"] != authority["resourceOwner"]["ownerToken"]
        or value["semanticEnvironment"] != authority["semanticEnvironment"]
        or value["runtimeKey"]
        != next(iter({row["runtimeKey"] for row in authority["sessions"]}))
        or threads != expected_threads
        or sessions != authority["sessions"]
    ):
        raise PilotError("SEMANTIC_RECOVERY_REQUIRED")
    return pending, value


def _cleanup_project_resources(
    authority_path: Path, authority: dict[str, Any]
) -> Path:
    pending = _pending_path(authority_path)
    value, threads, sessions = _read_resource_ledger(
        pending,
        Path(authority["executables"]["clew"]),
        "SEMANTIC_RECOVERY_REQUIRED",
    )
    expected_threads = {
        task["taskId"]: task["thread"]["threadId"] for task in authority["tasks"]
    }
    expected_sessions = {row["sessionId"]: row for row in authority["sessions"]}
    if (
        value["semanticEnvironment"] != authority["semanticEnvironment"]
        or value["openInFlight"] is not None
        or value["ownerPid"] != authority["resourceOwner"]["ownerPid"]
        or value["ownerToken"] != authority["resourceOwner"]["ownerToken"]
        or value["runtimeKey"]
        != next(iter({row["runtimeKey"] for row in authority["sessions"]}))
        or any(expected_threads.get(task_id) != thread_id for task_id, thread_id in threads.items())
        or any(
            row.get("sessionId") not in expected_sessions
            or expected_sessions[row["sessionId"]] != row
            for row in sessions
        )
    ):
        raise PilotError("SEMANTIC_RECOVERY_REQUIRED")
    if value["status"] == "CLEANED":
        if threads or sessions:
            raise PilotError("SEMANTIC_RECOVERY_REQUIRED")
        return pending
    if value["status"] in {"PREPARING", "ACTIVE"} and (
        threads != expected_threads or sessions != authority["sessions"]
    ):
        raise PilotError("SEMANTIC_RECOVERY_REQUIRED")
    _cleanup_semantic(
        Path(authority["executables"]["clew"]),
        threads,
        sessions,
        authority["semanticEnvironment"],
        pending=pending,
        runtime_key=value["runtimeKey"],
    )
    return pending


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    experiment_root, experiment_authority = _experiment_root(args.experiment_root)
    _require_experiment_paths(
        experiment_root,
        [
            args.private_corpus,
            args.private_benchmark,
            args.private_shape_oracle,
            args.private_shape_attestation,
            args.shape_oracle_review_manifest,
            args.private_authority,
            args.private_oracle,
        ],
    )
    corpus_path, corpus_value, corpus_raw = private_json(args.private_corpus, "PRIVATE_CORPUS", 256 * 1024)
    benchmark_path, benchmark_value, benchmark_raw = private_json(args.private_benchmark, "PRIVATE_BENCHMARK", 4 * 1024 * 1024)
    shape_path, shape_value, _ = private_json(args.private_shape_oracle, "SHAPE_ORACLE")
    attestation_path, attestation, _ = private_json(
        args.private_shape_attestation, "SHAPE_ATTESTATION", 256 * 1024
    )
    review_path, review_manifest, _ = checked_json(
        args.shape_oracle_review_manifest, "SHAPE_ORACLE_REVIEW", 256 * 1024
    )
    g1k_path, g1k_value, _ = checked_json(args.g1k_evidence, "G1K_EVIDENCE")
    try:
        g1k_verifier.verify_value(g1k_value)
    except g1k_verifier.EvidenceError as error:
        raise PilotError("INVALID_G1K_EVIDENCE") from error
    corpus = descriptor_gate.parse_corpus(corpus_value)
    corpus_digest = authority_digest(corpus_value)
    if corpus_digest != descriptor_gate.EXPECTED_CORPUS_DIGEST:
        raise PilotError("INVALID_PRIVATE_CORPUS")
    benchmark = descriptor_gate.parse_benchmark(
        benchmark_value, benchmark_raw, corpus, corpus_digest
    )
    git, git_digest = git_executable()
    descriptor_gate.validate_oracle_files(git, corpus, benchmark)
    g1k_digest = authority_digest(g1k_value)
    clew, clew_digest = executable(args.clew, "INVALID_CLEW_EXECUTABLE")
    try:
        g1k_runtime_digest = g1k_value["executionAuthority"]["clewAuthority"]
    except (KeyError, TypeError) as error:
        raise PilotError("INVALID_G1K_EVIDENCE") from error
    if clew_digest != g1k_runtime_digest:
        raise PilotError("G1K_RUNTIME_AUTHORITY_CHANGED")
    python, python_digest = executable(Path(sys.executable), "INVALID_PYTHON_EXECUTABLE")
    if sys.version_info < (3, 11):
        raise PilotError("INVALID_PYTHON_EXECUTABLE")
    python_framework = _python_framework_executable(python)
    maven, maven_digest = maven_executable()
    semantic_environment = _semantic_environment(python, maven)
    rustc, rustc_digest = _semantic_tool_executable(
        "rustc", semantic_environment, "INVALID_RUSTC_EXECUTABLE"
    )
    cargo, cargo_digest = _semantic_tool_executable(
        "cargo", semantic_environment, "INVALID_CARGO_EXECUTABLE"
    )
    codex_environment = _codex_environment(python)
    builder, builder_digest = executable(
        args.shape_oracle_builder, "INVALID_SHAPE_ORACLE_BUILDER"
    )
    runner_path = Path(__file__).resolve(strict=True)
    runner_digest = file_digest(runner_path)
    module_manifest = _verify_local_module_authority(local_module_manifest())
    review_inputs = _run_json(
        [
            os.fspath(python),
            "-I",
            "-S",
            os.fspath(builder),
            "review-inputs",
            "--g1k-evidence",
            os.fspath(g1k_path),
            "--clew",
            os.fspath(clew),
            "--git",
            os.fspath(git),
            "--pilot-runner",
            os.fspath(runner_path),
            "--experiment-root",
            os.fspath(experiment_root),
        ],
        min(args.timeout_seconds, 300),
        "COMPILER_SHAPE_ORACLE_REVIEW_FAILED",
        semantic_environment,
    )
    review_input_fields = {
        "schema", "builderDigest", "pilotRunnerDigest", "g1kEvidenceDigest",
        "publicFixtureTreeOid", "publicFixtureContentDigest", "testDigest",
        "localModuleManifest", "gitDigest", "gitEnvironmentDigest", "mavenDigest",
    }
    if (
        set(review_inputs) != review_input_fields
        or review_inputs.get("schema")
        != "codeclew-kotlin-descriptor-shape-builder-review-inputs/1.0"
        or review_inputs.get("builderDigest") != builder_digest
        or review_inputs.get("pilotRunnerDigest") != runner_digest
        or review_inputs.get("g1kEvidenceDigest") != g1k_digest
        or not isinstance(review_inputs.get("publicFixtureTreeOid"), str)
        or descriptor_gate.GIT_BLOB_OID.fullmatch(review_inputs["publicFixtureTreeOid"])
        is None
        or not isinstance(review_inputs.get("publicFixtureContentDigest"), str)
        or SHA256.fullmatch(review_inputs["publicFixtureContentDigest"]) is None
        or not isinstance(review_inputs.get("testDigest"), str)
        or SHA256.fullmatch(review_inputs["testDigest"]) is None
        or review_inputs.get("localModuleManifest") != module_manifest
        or review_inputs.get("gitDigest") != git_digest
        or review_inputs.get("mavenDigest") != maven_digest
        or not isinstance(review_inputs.get("gitEnvironmentDigest"), str)
        or SHA256.fullmatch(review_inputs["gitEnvironmentDigest"]) is None
    ):
        raise PilotError("COMPILER_SHAPE_ORACLE_REVIEW_FAILED")
    review_unsigned = dict(review_manifest)
    review_manifest_digest = review_unsigned.pop("authorityDigest", None)
    if (
        set(review_unsigned)
        == {
            "schema", "builderDigest", "pilotRunnerDigest", "g1kEvidenceDigest",
            "publicFixtureTreeOid", "publicFixtureContentDigest", "testDigest",
            "localModuleManifest", "gitDigest", "gitEnvironmentDigest", "mavenDigest",
            "verdict", "findings",
        }
        and review_manifest.get("schema")
        == "codeclew-kotlin-descriptor-shape-builder-review/1.0"
    ):
        review_ingredients = {
            key: review_manifest[key]
            for key in review_input_fields - {"schema"}
        }
        expected_ingredients = {
            key: review_inputs[key]
            for key in review_input_fields - {"schema"}
        }
    else:
        raise PilotError("COMPILER_SHAPE_ORACLE_REVIEW_FAILED")
    if (
        review_manifest_digest != authority_digest(review_unsigned)
        or review_ingredients != expected_ingredients
        or review_manifest.get("verdict") != "PASS"
        or review_manifest.get("findings") != []
    ):
        raise PilotError("COMPILER_SHAPE_ORACLE_REVIEW_FAILED")
    if not isinstance(review_manifest_digest, str) or SHA256.fullmatch(review_manifest_digest) is None:
        raise PilotError("COMPILER_SHAPE_ORACLE_REVIEW_FAILED")
    review_manifest_file_digest = file_digest(review_path)
    shape = validate_shape_oracle(
        shape_value,
        corpus,
        benchmark,
        benchmark_value,
        g1k_digest,
        g1k_runtime_digest,
        review_inputs["publicFixtureTreeOid"],
        review_inputs["publicFixtureContentDigest"],
        authority_digest(semantic_environment),
        git_digest,
        review_inputs["gitEnvironmentDigest"],
        module_manifest["authorityDigest"],
        maven_digest,
    )
    attestation_unsigned = dict(attestation)
    attestation_digest = attestation_unsigned.pop("authorityDigest", None)
    if (
        set(attestation_unsigned)
        != {
            "schema", "shapeOracleDigest", "g1kEvidenceDigest", "runtimeDigest",
            "runtimeKey", "gitDigest", "gitEnvironmentDigest",
            "localModuleManifestDigest", "builderDigest", "compilerVerification",
            "reviewManifestDigest", "compilerEnvironmentDigest", "mavenDigest",
        }
        or attestation.get("schema") != PRIVATE_SHAPE_ATTESTATION_SCHEMA
        or attestation_digest != authority_digest(attestation_unsigned)
        or attestation.get("shapeOracleDigest") != shape["authorityDigest"]
        or attestation.get("g1kEvidenceDigest") != g1k_digest
        or attestation.get("runtimeDigest") != g1k_runtime_digest
        or attestation.get("runtimeKey") != shape["sourceAuthority"]["runtimeKey"]
        or attestation.get("gitDigest") != git_digest
        or attestation.get("gitEnvironmentDigest")
        != review_inputs["gitEnvironmentDigest"]
        or attestation.get("localModuleManifestDigest")
        != module_manifest["authorityDigest"]
        or attestation.get("builderDigest") != builder_digest
        or attestation.get("compilerVerification") != "PASS"
        or attestation.get("reviewManifestDigest") != review_manifest_digest
        or attestation.get("compilerEnvironmentDigest")
        != authority_digest(semantic_environment)
        or attestation.get("mavenDigest") != maven_digest
    ):
        raise PilotError("COMPILER_SHAPE_ORACLE_ATTESTATION_INVALID")
    builder_result = _run_json(
        [
            os.fspath(python), "-I", "-S", os.fspath(builder), "verify",
            "--shape-oracle", os.fspath(shape_path),
            "--attestation", os.fspath(attestation_path),
            "--g1k-evidence", os.fspath(g1k_path),
            "--clew", os.fspath(clew),
            "--git", os.fspath(git),
            "--private-corpus", os.fspath(corpus_path),
            "--private-benchmark", os.fspath(benchmark_path),
            "--review-manifest", os.fspath(review_path),
            "--pilot-runner", os.fspath(runner_path),
            "--experiment-root", os.fspath(experiment_root),
        ],
        min(args.timeout_seconds, 300),
        "COMPILER_SHAPE_ORACLE_BUILDER_FAILED",
        semantic_environment,
    )
    if builder_result != {
        "schema": "codeclew-kotlin-descriptor-shape-builder-result/1.0",
        "status": "PASS",
        "shapeOracleDigest": shape["authorityDigest"],
        "attestationDigest": attestation_digest,
    }:
        raise PilotError("COMPILER_SHAPE_ORACLE_BUILDER_FAILED")
    _require_maven_authority(maven_digest)

    codex, codex_digest = executable(args.codex, "INVALID_CODEX_EXECUTABLE")
    sandbox_exec, sandbox_exec_digest = executable(
        Path("/usr/bin/sandbox-exec"), "BROKER_AUDIT_ADAPTER_UNAVAILABLE"
    )
    broker_path = (runner_path.parent / "thread_kotlin_pilot_broker.py").resolve(strict=True)
    verifier_path = (runner_path.parent / "verify_thread_kotlin_pilot.py").resolve(strict=True)
    answer_schema_path = (runner_path.parent / "schemas" / "thread-kotlin-pilot-answer.schema.json").resolve(strict=True)
    try:
        warm_adapter_path = args.warm_audit_runner.resolve(strict=True)
    except OSError as error:
        raise PilotError("INVALID_WARM_AUDIT_ADAPTER") from error
    expected_warm_adapter = (runner_path.parent / "run_thread_kotlin_warm_audit.py").resolve(strict=True)
    if warm_adapter_path != expected_warm_adapter:
        raise PilotError("INVALID_WARM_AUDIT_ADAPTER")
    warm_adapter_digest = file_digest(warm_adapter_path)
    warm_fixture = _run_json(
        [
            os.fspath(python),
            "-I",
            "-S",
            os.fspath(warm_adapter_path),
            "fixture-digest",
            "--source-repo",
            os.fspath(clew.parent),
            "--clew",
            os.fspath(clew),
            "--git",
            os.fspath(git),
        ],
        min(args.timeout_seconds, 300),
        "WARM_FIXTURE_AUTHORITY_FAILED",
        semantic_environment,
    )
    if (
        set(warm_fixture)
        != {"schema", "status", "trackedTreeOid", "fixtureDigest"}
        or warm_fixture.get("schema")
        != "codeclew-kotlin-warm-fixture-digest/1.0"
        or warm_fixture.get("status") != "SEALED"
        or not isinstance(warm_fixture.get("trackedTreeOid"), str)
        or descriptor_gate.GIT_BLOB_OID.fullmatch(warm_fixture["trackedTreeOid"])
        is None
        or not isinstance(warm_fixture.get("fixtureDigest"), str)
        or SHA256.fullmatch(warm_fixture["fixtureDigest"]) is None
    ):
        raise PilotError("WARM_FIXTURE_AUTHORITY_FAILED")
    warm_fixture_digest = warm_fixture["fixtureDigest"]
    _require_maven_authority(maven_digest)
    if args.model is None or SAFE_MODEL.fullmatch(args.model) is None or args.reasoning_effort not in SAFE_REASONING:
        raise PilotError("INVALID_MODEL_CONFIGURATION")
    permission_canary = _codex_permission_canary(
        codex, python, experiment_root, codex_environment
    )
    model_configuration = {
        "modelId": args.model,
        "reasoningEffort": args.reasoning_effort,
        "sandbox": "S4K_RESTRICTED_WORKSPACE_NETWORK_DENIED",
        "approvalPolicy": "NEVER",
        "ephemeral": True,
        "userConfigIgnored": True,
        "rulesIgnored": True,
        "armHomePolicy": "MANAGED_ROOT_SCRATCH_0700",
        "credentialPolicy": "PRIVATE_HARDLINK_HOME_SOURCE_INODE",
        "permissionCanary": permission_canary,
        "environmentDigest": authority_digest(codex_environment),
    }
    model_digest = authority_digest(model_configuration)
    protocol_digest = authority_digest(PROTOCOL)
    shape_digest = shape["authorityDigest"]

    authority_target = _output_locator(args.private_authority)
    oracle_target = _output_locator(args.private_oracle)
    output_paths = [authority_target, oracle_target]
    pending = _output_locator(_pending_path(authority_target))
    publication_pending = _prepare_publication_path(authority_target)
    publication_creating = _prepare_publication_creating_path(publication_pending)
    broker_canary_pending = _broker_canary_ledger_path(authority_target)
    input_paths = [
        corpus_path, benchmark_path, shape_path, attestation_path, review_path, g1k_path
    ]
    require_distinct_paths(
        input_paths,
        output_paths
        + [
            pending,
            publication_pending,
            publication_creating,
            broker_canary_pending,
        ],
    )

    # S4K is fail-stop and single-owner.  A prior ledger/output means this
    # experiment root is terminal and must be quarantined; no phase imports,
    # resumes, or cleans an ambiguous predecessor.
    authority_target = fresh_output_target(authority_target)
    oracle_target = fresh_output_target(oracle_target)
    for terminal_artifact in (
        pending,
        publication_pending,
        publication_creating,
        broker_canary_pending,
    ):
        fresh_output_target(terminal_artifact)

    resource_owner = {"ownerPid": os.getpid(), "ownerToken": secrets.token_hex(32)}
    task_threads, task_thread_authorities, sessions = prepare_semantic(
        corpus,
        benchmark,
        git,
        clew,
        args.timeout_seconds,
        semantic_environment,
        pending,
        shape["sourceAuthority"]["runtimeKey"],
        resource_owner["ownerPid"],
        resource_owner["ownerToken"],
    )
    try:
        prepared_runtime_key = next(iter({row["runtimeKey"] for row in sessions}))
        if (
            prepared_runtime_key != shape["sourceAuthority"]["runtimeKey"]
            or prepared_runtime_key != attestation["runtimeKey"]
        ):
            raise PilotError("SEMANTIC_RUNTIME_AUTHORITY_DIVERGED")
        broker_audit = _broker_audit(
            sandbox_exec=sandbox_exec,
            clew=clew,
            git=git,
            python=python,
            python_framework=python_framework,
            semantic_environment=semantic_environment,
            repositories=[service.repository for service in corpus.services],
            sessions=sessions,
            authority_path=authority_target,
        )
    except Exception as primary:
        try:
            _cleanup_semantic(
                clew, task_threads, sessions, semantic_environment,
                pending=pending,
                runtime_key=shape["sourceAuthority"]["runtimeKey"],
            )
            _remove_pending(pending)
        except PilotError as cleanup_error:
            raise cleanup_error from primary
        raise
    revisions = {service.alias: service.revision for service in corpus.services}
    benchmark_tasks = {row["taskId"]: row for row in benchmark_value["tasks"]}
    tasks: list[dict[str, Any]] = []
    for task in corpus.tasks:
        benchmark_task = benchmark_tasks[task.task_id]
        generic = benchmark_value["promptProfiles"][benchmark_task["promptProfile"]]
        prompt = protocol_prompt(generic, task, revisions)
        tasks.append(
            {
                "taskId": task.task_id,
                "pairId": task.pair_id,
                "scenario": task.scenario,
                "provider": task.provider,
                "consumer": task.consumer,
                "providerRevision": revisions[task.provider],
                "consumerRevision": revisions[task.consumer],
                "prompt": prompt,
                "promptDigest": authority_digest(prompt),
                "manualVerification": _manual_categories(benchmark_value, benchmark_task),
                "thread": {
                    "threadId": task_threads[task.task_id],
                    "threadAuthorityDigest": task_thread_authorities[task.task_id],
                    "providerMember": "provider",
                    "consumerMember": "consumer",
                },
            }
        )
    oracle_unsigned = {
        "schema": PRIVATE_ORACLE_SCHEMA,
        "protocolDigest": protocol_digest,
        "shapeOracleDigest": shape_digest,
        "fixture": shape["fixture"],
        "tasks": shape["tasks"],
    }
    oracle = dict(oracle_unsigned)
    oracle["authorityDigest"] = authority_digest(oracle_unsigned)
    oracle_digest = oracle["authorityDigest"]
    arm_order = [
        {
            "taskId": task.task_id,
            "arms": ["DEFAULT", "CODECLEW"] if index % 2 == 1 else ["CODECLEW", "DEFAULT"],
        }
        for index, task in enumerate(corpus.tasks, 1)
    ]
    authority_unsigned = {
        "schema": PRIVATE_AUTHORITY_SCHEMA,
        "frozenAt": FROZEN_AT,
        "protocol": PROTOCOL,
        "protocolDigest": protocol_digest,
        "inputs": {
            "privateCorpusDigest": corpus_digest,
            "benchmarkDigest": benchmark.authority_digest,
            "g1kEvidenceDigest": g1k_digest,
            "shapeOracleDigest": shape_digest,
            "shapeOracleAttestationDigest": attestation_digest,
            "shapeOracleBuilderDigest": builder_digest,
            "shapeOracleReviewManifestDigest": review_manifest_digest,
            "shapeOracleReviewManifestFileDigest": review_manifest_file_digest,
            "pilotOracleDigest": oracle_digest,
            "answerSchemaDigest": file_digest(answer_schema_path),
            "runnerDigest": runner_digest,
            "brokerDigest": file_digest(broker_path),
            "publicVerifierDigest": file_digest(verifier_path),
            "localModuleManifest": module_manifest,
            "localModuleManifestDigest": module_manifest["authorityDigest"],
            "warmAuditAdapterDigest": warm_adapter_digest,
            "warmFixtureDigest": warm_fixture_digest,
        },
        "model": model_configuration,
        "modelConfigurationDigest": model_digest,
        "semanticEnvironment": semantic_environment,
        "codexEnvironment": codex_environment,
        "experimentRoot": experiment_authority,
        "resourceOwner": resource_owner,
        "brokerAudit": broker_audit,
        "executables": {
            "clew": os.fspath(clew),
            "clewDigest": clew_digest,
            "codex": os.fspath(codex),
            "codexDigest": codex_digest,
            "git": os.fspath(git),
            "gitDigest": git_digest,
            "maven": os.fspath(maven),
            "mavenDigest": maven_digest,
            "python": os.fspath(python),
            "pythonDigest": python_digest,
            "pythonFramework": (
                os.fspath(python_framework[0]) if python_framework is not None else None
            ),
            "pythonFrameworkDigest": (
                python_framework[1] if python_framework is not None else None
            ),
            "rustc": os.fspath(rustc),
            "rustcDigest": rustc_digest,
            "cargo": os.fspath(cargo),
            "cargoDigest": cargo_digest,
            "sandboxExec": os.fspath(sandbox_exec),
            "sandboxExecDigest": sandbox_exec_digest,
        },
        "budgets": BUDGETS,
        "taskOrder": [task.task_id for task in corpus.tasks],
        "armOrder": arm_order,
        "repositories": [
            {
                "serviceAlias": service.alias,
                "path": os.fspath(service.repository),
                "revision": service.revision,
            }
            for service in corpus.services
        ],
        "sessions": sessions,
        "tasks": tasks,
    }
    authority = dict(authority_unsigned)
    authority["authorityDigest"] = authority_digest(authority_unsigned)
    try:
        _require_maven_authority(maven_digest)
        _publish_prepare_pair(
            authority_target,
            authority,
            oracle_target,
            oracle,
            pending,
            clew,
        )
    except Exception as primary:
        try:
            _cleanup_semantic(
                clew, task_threads, sessions, semantic_environment,
                pending=pending,
                runtime_key=shape["sourceAuthority"]["runtimeKey"],
            )
            _remove_pending(pending)
        except PilotError as cleanup_error:
            raise cleanup_error from primary
        raise
    return {
        "schema": PRIVATE_AUTHORITY_SCHEMA,
        "status": "PREPARED",
        "authorityDigest": authority["authorityDigest"],
        "taskCount": 10,
        "armCount": 20,
    }


def verify_authority(value: dict[str, Any]) -> dict[str, Any]:
    unsigned = dict(value)
    declared = unsigned.pop("authorityDigest", None)
    required = {
        "schema", "frozenAt", "protocol", "protocolDigest", "inputs", "model",
        "modelConfigurationDigest", "semanticEnvironment", "codexEnvironment",
        "experimentRoot", "resourceOwner", "brokerAudit", "executables", "budgets", "taskOrder", "armOrder",
        "repositories", "sessions", "tasks",
    }
    if set(unsigned) != required or value.get("schema") != PRIVATE_AUTHORITY_SCHEMA or declared != authority_digest(unsigned):
        raise PilotError("INVALID_PILOT_AUTHORITY")
    if value["protocol"] != PROTOCOL or value["protocolDigest"] != authority_digest(PROTOCOL) or value["budgets"] != BUDGETS:
        raise PilotError("INVALID_PILOT_AUTHORITY")
    if value["taskOrder"] != [f"task-{index:02}" for index in range(1, 11)]:
        raise PilotError("INVALID_PILOT_AUTHORITY")
    expected_order = [
        {"taskId": f"task-{index:02}", "arms": ["DEFAULT", "CODECLEW"] if index % 2 else ["CODECLEW", "DEFAULT"]}
        for index in range(1, 11)
    ]
    if value["armOrder"] != expected_order or not isinstance(value["tasks"], list) or len(value["tasks"]) != 10:
        raise PilotError("INVALID_PILOT_AUTHORITY")
    model = value["model"]
    executable_rows = value.get("executables")
    python_value = (
        executable_rows.get("python")
        if isinstance(executable_rows, dict)
        else None
    )
    if not isinstance(python_value, str):
        raise PilotError("INVALID_PILOT_AUTHORITY")
    try:
        permission_profile_digest = authority_digest(
            _codex_permission_arguments(Path(python_value))
        )
    except PilotError as error:
        raise PilotError("INVALID_PILOT_AUTHORITY") from error
    if (
        not isinstance(model, dict)
        or set(model)
        != {
            "modelId", "reasoningEffort", "sandbox", "approvalPolicy", "ephemeral",
            "userConfigIgnored", "rulesIgnored", "armHomePolicy", "credentialPolicy",
            "permissionCanary", "environmentDigest",
        }
        or not isinstance(model["modelId"], str)
        or SAFE_MODEL.fullmatch(model["modelId"]) is None
        or model["reasoningEffort"] not in SAFE_REASONING
        or model["sandbox"] != "S4K_RESTRICTED_WORKSPACE_NETWORK_DENIED"
        or model["approvalPolicy"] != "NEVER"
        or model["armHomePolicy"] != "MANAGED_ROOT_SCRATCH_0700"
        or model["credentialPolicy"] != "PRIVATE_HARDLINK_HOME_SOURCE_INODE"
        or model["permissionCanary"]
        != {
            "profile": "S4K_RESTRICTED_WORKSPACE_V1",
            "profileDigest": permission_profile_digest,
            "credentialReadDenied": True,
            "credentialWriteDenied": True,
            "networkDenied": True,
            "workspaceWritePassed": True,
        }
        or any(model[key] is not True for key in {"ephemeral", "userConfigIgnored", "rulesIgnored"})
        or value["modelConfigurationDigest"] != authority_digest(model)
    ):
        raise PilotError("INVALID_PILOT_AUTHORITY")
    semantic_environment = _validate_semantic_environment(
        value["semanticEnvironment"], "INVALID_PILOT_AUTHORITY"
    )
    experiment = closed(
        value["experimentRoot"],
        {"path", "device", "inode"},
        "INVALID_PILOT_AUTHORITY",
    )
    if (
        not isinstance(experiment["path"], str)
        or type(experiment["device"]) is not int
        or type(experiment["inode"]) is not int
        or experiment["device"] < 0
        or experiment["inode"] <= 0
    ):
        raise PilotError("INVALID_PILOT_AUTHORITY")
    _, current_experiment = _experiment_root(Path(experiment["path"]))
    if current_experiment != experiment:
        raise PilotError("EXPERIMENT_ROOT_AUTHORITY_CHANGED")
    resource_owner = closed(
        value["resourceOwner"],
        {"ownerPid", "ownerToken"},
        "INVALID_PILOT_AUTHORITY",
    )
    if (
        type(resource_owner["ownerPid"]) is not int
        or resource_owner["ownerPid"] <= 0
        or not isinstance(resource_owner["ownerToken"], str)
        or re.fullmatch(r"[0-9a-f]{64}", resource_owner["ownerToken"]) is None
    ):
        raise PilotError("INVALID_PILOT_AUTHORITY")
    codex_environment = value["codexEnvironment"]
    if (
        not isinstance(codex_environment, dict)
        or not set(codex_environment).issubset(CODEX_ENVIRONMENT_KEYS)
        or "CODEX_HOME" not in codex_environment
        or codex_environment.get("SHELL") != "/bin/sh"
        or any(
            not isinstance(item, str) or not item or "\0" in item
            for item in codex_environment.values()
        )
        or model["environmentDigest"]
        != authority_digest(codex_environment)
    ):
        raise PilotError("INVALID_PILOT_AUTHORITY")
    for key in {"CODEX_HOME", "TMPDIR", "SSL_CERT_FILE", "SSL_CERT_DIR"} & set(codex_environment):
        path = Path(codex_environment[key])
        if not path.is_absolute() or codex_environment[key] != os.fspath(path.resolve(strict=False)):
            raise PilotError("INVALID_PILOT_AUTHORITY")
    broker_audit = value["brokerAudit"]
    if (
        not isinstance(broker_audit, dict)
        or set(broker_audit) != {
            "adapter", "profilePolicy", "sandboxExecutable", "profile", "profileDigest",
            "pythonFrameworkExecutable", "pythonFrameworkDigest",
            "allowedProcessCanaryPassed",
            "networkCanaryDenied", "processCanaryDenied", "cacheCanaryDenied",
            "cacheRootCanaryCount", "cacheSentinelDigest", "writeCanaryDenied",
            "managedStateWriteCanaryPassed", "cacheRoots", "allowedWriteRoots",
        }
        or broker_audit["adapter"] != "MACOS_SEATBELT_V1"
        or broker_audit["profilePolicy"]
        != "GLOBAL_WRITE_DENY_MANAGED_STATE_ONLY_V1"
        or not isinstance(broker_audit["profile"], str)
        or broker_audit["profileDigest"] != authority_digest(broker_audit["profile"])
        or any(
            broker_audit[key] is not True
            for key in {
                "allowedProcessCanaryPassed",
                "networkCanaryDenied", "processCanaryDenied", "cacheCanaryDenied",
                "writeCanaryDenied", "managedStateWriteCanaryPassed",
            }
        )
    ):
        raise PilotError("INVALID_PILOT_AUTHORITY")
    state_root = _state_root(semantic_environment)
    expected_cache_roots = [
        {"label": label, "pathDigest": authority_digest(os.fspath(path))}
        for label, path in _effective_cache_roots(
            semantic_environment, state_root, "INVALID_PILOT_AUTHORITY"
        )
    ]
    if broker_audit["cacheRoots"] != expected_cache_roots:
        raise PilotError("INVALID_PILOT_AUTHORITY")
    if (
        broker_audit["cacheRootCanaryCount"] != len(expected_cache_roots)
        or not isinstance(broker_audit["cacheSentinelDigest"], str)
        or SHA256.fullmatch(broker_audit["cacheSentinelDigest"]) is None
        or broker_audit["allowedWriteRoots"]
        != [
            {
                "label": "CODECLEW_MANAGED_STATE",
                "pathDigest": authority_digest(os.fspath(state_root)),
            }
        ]
        or "(deny file-write*)" not in broker_audit["profile"]
    ):
        raise PilotError("INVALID_PILOT_AUTHORITY")
    inputs = value["inputs"]
    if not isinstance(inputs, dict) or set(inputs) != {
        "privateCorpusDigest", "benchmarkDigest", "g1kEvidenceDigest", "shapeOracleDigest",
        "shapeOracleAttestationDigest", "shapeOracleBuilderDigest",
        "shapeOracleReviewManifestDigest", "shapeOracleReviewManifestFileDigest",
        "pilotOracleDigest",
        "answerSchemaDigest", "runnerDigest", "brokerDigest", "publicVerifierDigest",
        "localModuleManifest", "localModuleManifestDigest",
        "warmAuditAdapterDigest",
        "warmFixtureDigest",
    }:
        raise PilotError("INVALID_PILOT_AUTHORITY")
    if inputs["privateCorpusDigest"] != descriptor_gate.EXPECTED_CORPUS_DIGEST or inputs["benchmarkDigest"] != descriptor_gate.EXPECTED_BENCHMARK_DIGEST:
        raise PilotError("INVALID_PILOT_AUTHORITY")
    for key, digest in inputs.items():
        if key == "localModuleManifest":
            continue
        if not isinstance(digest, str) or SHA256.fullmatch(digest) is None:
            raise PilotError("INVALID_PILOT_AUTHORITY")
    module_manifest = _verify_local_module_authority(inputs["localModuleManifest"])
    if inputs["localModuleManifestDigest"] != module_manifest["authorityDigest"]:
        raise PilotError("LOCAL_MODULE_AUTHORITY_INVALID")
    runner_path = Path(__file__).resolve(strict=True)
    if (
        inputs["runnerDigest"] != file_digest(runner_path)
        or inputs["brokerDigest"] != file_digest(runner_path.parent / "thread_kotlin_pilot_broker.py")
        or inputs["publicVerifierDigest"]
        != file_digest(runner_path.parent / "verify_thread_kotlin_pilot.py")
        or inputs["answerSchemaDigest"] != file_digest(runner_path.parent / "schemas" / "thread-kotlin-pilot-answer.schema.json")
        or inputs["warmAuditAdapterDigest"] != file_digest(runner_path.parent / "run_thread_kotlin_warm_audit.py")
    ):
        raise PilotError("PILOT_IMPLEMENTATION_CHANGED")
    executables = value["executables"]
    if not isinstance(executables, dict) or set(executables) != {
        "clew", "clewDigest", "codex", "codexDigest", "git", "gitDigest",
        "maven", "mavenDigest", "python", "pythonDigest", "pythonFramework",
        "pythonFrameworkDigest", "rustc",
        "rustcDigest", "cargo", "cargoDigest", "sandboxExec",
        "sandboxExecDigest",
    }:
        raise PilotError("INVALID_PILOT_AUTHORITY")
    for name in ["clew", "codex", "git", "sandboxExec"]:
        path, digest = executable(Path(executables[name]), f"INVALID_{name.upper()}_EXECUTABLE")
        if os.fspath(path) != executables[name] or digest != executables[f"{name}Digest"]:
            raise PilotError("EXECUTABLE_AUTHORITY_CHANGED")
    _require_python_runtime_authority(executables)
    maven_path = _require_maven_authority(executables["mavenDigest"])
    if os.fspath(maven_path) != executables["maven"]:
        raise PilotError("EXECUTABLE_AUTHORITY_CHANGED")
    _verify_semantic_environment_authority(
        semantic_environment, executables, "INVALID_PILOT_AUTHORITY"
    )
    codex_path = f"{Path(executables['python']).parent}:/usr/bin:/bin"
    if codex_environment.get("PATH") != codex_path:
        raise PilotError("INVALID_PILOT_AUTHORITY")
    if (
        broker_audit["sandboxExecutable"] != executables["sandboxExec"]
        or broker_audit["pythonFrameworkExecutable"]
        != executables["pythonFramework"]
        or broker_audit["pythonFrameworkDigest"]
        != executables["pythonFrameworkDigest"]
    ):
        raise PilotError("EXECUTABLE_AUTHORITY_CHANGED")
    sessions = value["sessions"]
    if (
        not isinstance(sessions, list)
        or not sessions
        or any(
            not isinstance(row, dict)
            or set(row)
            != {
                "serviceAlias", "sessionId", "sessionAuthorityDigest", "runtimeKey",
                "runtimeMode",
            }
            or not isinstance(row["serviceAlias"], str)
            or SAFE_MODEL.fullmatch(row["serviceAlias"]) is None
            or not isinstance(row["sessionId"], str)
            or not row["sessionId"].startswith("session:")
            or not isinstance(row["sessionAuthorityDigest"], str)
            or SHA256.fullmatch(row["sessionAuthorityDigest"]) is None
            or not isinstance(row["runtimeKey"], str)
            or SHA256.fullmatch(row["runtimeKey"]) is None
            or row["runtimeMode"] != "RELEASE"
            for row in sessions
        )
        or len({row["serviceAlias"] for row in sessions}) != len(sessions)
        or len({row["sessionId"] for row in sessions}) != len(sessions)
        or len({row["runtimeKey"] for row in sessions}) != 1
    ):
        raise PilotError("INVALID_PILOT_AUTHORITY")
    thread_ids: set[str] = set()
    for task_index, (task, expected_order_row) in enumerate(
        zip(value["tasks"], expected_order, strict=True)
    ):
        task = closed(
            task,
            {
                "taskId", "pairId", "scenario", "provider", "consumer",
                "providerRevision", "consumerRevision", "prompt", "promptDigest",
                "manualVerification", "thread",
            },
            "INVALID_PILOT_AUTHORITY",
        )
        thread = closed(
            task["thread"],
            {
                "threadId", "threadAuthorityDigest", "providerMember",
                "consumerMember",
            },
            "INVALID_PILOT_AUTHORITY",
        )
        if (
            task["taskId"] != expected_order_row["taskId"]
            or not isinstance(task["pairId"], str)
            or not isinstance(task["prompt"], str)
            or task["promptDigest"] != authority_digest(task["prompt"])
            or not isinstance(task["manualVerification"], list)
            or not isinstance(thread["threadId"], str)
            or not thread["threadId"].startswith("thread:")
            or thread["threadId"] in thread_ids
            or not isinstance(thread["threadAuthorityDigest"], str)
            or SHA256.fullmatch(thread["threadAuthorityDigest"]) is None
            or thread["providerMember"] != "provider"
            or thread["consumerMember"] != "consumer"
        ):
            raise PilotError("INVALID_PILOT_AUTHORITY")
        _validate_manual_verification(
            task["manualVerification"],
            8 if task_index < 8 else 5,
            "INVALID_PILOT_AUTHORITY",
        )
        thread_ids.add(thread["threadId"])
    return value


def verify_oracle(value: dict[str, Any], authority: dict[str, Any]) -> dict[str, Any]:
    root = closed(
        value,
        {
            "schema", "protocolDigest", "shapeOracleDigest", "fixture", "tasks",
            "authorityDigest",
        },
        "INVALID_PILOT_ORACLE",
    )
    unsigned = dict(root)
    declared = unsigned.pop("authorityDigest")
    if (
        root["schema"] != PRIVATE_ORACLE_SCHEMA
        or declared != authority_digest(unsigned)
        or declared != authority["inputs"]["pilotOracleDigest"]
        or root["protocolDigest"] != authority["protocolDigest"]
        or root["shapeOracleDigest"] != authority["inputs"]["shapeOracleDigest"]
    ):
        raise PilotError("INVALID_PILOT_ORACLE")
    fixture = root["fixture"]
    if not isinstance(fixture, list) or len(fixture) != 5:
        raise PilotError("INVALID_PILOT_ORACLE")
    fixture_identities: set[str] = set()
    for declaration in fixture:
        validated = _validate_exact_declaration(declaration, private=True)
        identity = authority_digest(validated)
        if identity in fixture_identities:
            raise PilotError("INVALID_PILOT_ORACLE")
        fixture_identities.add(identity)

    tasks = root["tasks"]
    authority_tasks = authority["tasks"]
    if (
        not isinstance(tasks, list)
        or len(tasks) != 10
        or len(tasks) != len(authority_tasks)
    ):
        raise PilotError("INVALID_PILOT_ORACLE")
    manual_total = 0
    for task_index, (raw_task, expected) in enumerate(
        zip(tasks, authority_tasks, strict=True)
    ):
        task = closed(
            raw_task,
            {"taskId", "pairId", "manualVerification", "sides"},
            "INVALID_PILOT_ORACLE",
        )
        manual = task["manualVerification"]
        if (
            task["taskId"] != expected["taskId"]
            or task["pairId"] != expected["pairId"]
            or manual != expected["manualVerification"]
            or not isinstance(manual, list)
        ):
            raise PilotError("INVALID_PILOT_ORACLE")
        _validate_manual_verification(
            manual,
            8 if task_index < 8 else 5,
            "INVALID_PILOT_ORACLE",
        )
        manual_total += len(manual)
        sides = task["sides"]
        if not isinstance(sides, list) or len(sides) != 2:
            raise PilotError("INVALID_PILOT_ORACLE")
        descriptor_slots: set[str] = set()
        for raw_side, role in zip(sides, ("provider", "consumer"), strict=True):
            side = closed(
                raw_side,
                {
                    "role", "serviceAlias", "revision", "approvedFiles",
                    "exactDeclarations",
                },
                "INVALID_PILOT_ORACLE",
            )
            approved = side["approvedFiles"]
            declarations = side["exactDeclarations"]
            if (
                side["role"] != role
                or side["serviceAlias"] != expected[role]
                or side["revision"] != expected[f"{role}Revision"]
                or not isinstance(approved, list)
                or not approved
                or not isinstance(declarations, list)
                or not declarations
            ):
                raise PilotError("INVALID_PILOT_ORACLE")
            approved_rows: list[tuple[str, str]] = []
            for raw_file in approved:
                item = closed(
                    raw_file,
                    {"relativeFile", "blobOid"},
                    "INVALID_PILOT_ORACLE",
                )
                descriptor_gate.safe_relative_kotlin_file(item["relativeFile"])
                if (
                    not isinstance(item["blobOid"], str)
                    or descriptor_gate.GIT_BLOB_OID.fullmatch(item["blobOid"])
                    is None
                ):
                    raise PilotError("INVALID_PILOT_ORACLE")
                approved_rows.append((item["relativeFile"], item["blobOid"]))
            if len(approved_rows) != len(set(approved_rows)):
                raise PilotError("INVALID_PILOT_ORACLE")
            exact_identities: set[str] = set()
            for raw_declaration in declarations:
                declaration = _validate_exact_declaration(
                    raw_declaration, private=True
                )
                if (
                    (declaration["relativeFile"], declaration["blobOid"])
                    not in set(approved_rows)
                ):
                    raise PilotError("INVALID_PILOT_ORACLE")
                identity = authority_digest(declaration)
                if identity in exact_identities:
                    raise PilotError("INVALID_PILOT_ORACLE")
                exact_identities.add(identity)
                descriptor_slots.add(declaration["descriptorClass"])
        if descriptor_slots != {"CALLABLE", "TYPE"}:
            raise PilotError("INVALID_PILOT_ORACLE")
    if manual_total != 74:
        raise PilotError("INVALID_PILOT_ORACLE")
    return root


def validate_answer(value: Any, task: dict[str, Any], arm: str) -> dict[str, Any]:
    answer = closed(value, ANSWER_FIELDS, "ANSWER_SCHEMA_INVALID")
    if (
        answer["schema"] != ANSWER_SCHEMA
        or answer["taskId"] != task["taskId"]
        or answer["pairId"] != task["pairId"]
        or answer["arm"] != arm
        or answer["httpEndpointEquivalence"] not in {"NOT_CLAIMED", "EXACT_CLAIMED"}
        or answer["compatibility"] not in {"NOT_CLAIMED", "EXACT_CLAIMED"}
    ):
        raise PilotError("ANSWER_AUTHORITY_INVALID")
    relationship = closed(answer["relationship"], {"authority", "status"}, "ANSWER_SCHEMA_INVALID")
    if relationship["authority"] not in {"DECLARED_TOPOLOGY", "UNBOUND", "EXACT_RELATIONSHIP"} or relationship["status"] not in {"UNSURE", "EXACT"}:
        raise PilotError("ANSWER_SCHEMA_INVALID")
    members = answer["members"]
    if not isinstance(members, list) or len(members) != 2:
        raise PilotError("ANSWER_SCHEMA_INVALID")
    seen_roles: set[str] = set()
    for member in members:
        closed(member, {"role", "serviceAlias", "revision", "rankedFiles", "declarations"}, "ANSWER_SCHEMA_INVALID")
        role = member["role"]
        if role not in {"provider", "consumer"} or role in seen_roles:
            raise PilotError("ANSWER_SCHEMA_INVALID")
        seen_roles.add(role)
        expected_alias = task[role]
        if member["serviceAlias"] != expected_alias or member["revision"] != task[f"{role}Revision"]:
            raise PilotError("ANSWER_AUTHORITY_INVALID")
        ranked = member["rankedFiles"]
        if not isinstance(ranked, list) or len(ranked) > 10:
            raise PilotError("ANSWER_SCHEMA_INVALID")
        ranks: set[int] = set()
        files: set[tuple[str, str]] = set()
        for row in ranked:
            closed(row, {"rank", "relativeFile", "blobOid"}, "ANSWER_SCHEMA_INVALID")
            if type(row["rank"]) is not int or not 1 <= row["rank"] <= 10 or row["rank"] in ranks:
                raise PilotError("ANSWER_SCHEMA_INVALID")
            descriptor_gate.safe_relative_kotlin_file(row["relativeFile"])
            if descriptor_gate.GIT_BLOB_OID.fullmatch(row["blobOid"]) is None or (row["relativeFile"], row["blobOid"]) in files:
                raise PilotError("ANSWER_SCHEMA_INVALID")
            ranks.add(row["rank"])
            files.add((row["relativeFile"], row["blobOid"]))
        if ranks != set(range(1, len(ranked) + 1)):
            raise PilotError("ANSWER_SCHEMA_INVALID")
        declarations = member["declarations"]
        if not isinstance(declarations, list) or len(declarations) > 128:
            raise PilotError("ANSWER_SCHEMA_INVALID")
        identities: set[str] = set()
        for declaration in declarations:
            closed(declaration, EXACT_FIELDS, "ANSWER_SCHEMA_INVALID")
            if declaration["shapeStatus"] == "EXACT_PROJECTED_DECLARATION":
                _validate_exact_declaration(declaration, private=False)
            elif declaration["shapeStatus"] == "UNSURE":
                if any(declaration[key] is not None for key in {"ownerIdentity", "normalizedSignature", "shapeDigest"}):
                    raise PilotError("ANSWER_SCHEMA_INVALID")
                if (
                    declaration["descriptorClass"] not in {"CALLABLE", "TYPE"}
                    or declaration["declarationKind"]
                    not in {"FUNCTION", "CONSTRUCTOR", "CLASS", "PROPERTY", "MUTABLE_PROPERTY"}
                    or not isinstance(declaration["name"], str)
                    or re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,255}", declaration["name"])
                    is None
                    or not isinstance(declaration["blobOid"], str)
                    or descriptor_gate.GIT_BLOB_OID.fullmatch(declaration["blobOid"])
                    is None
                ):
                    raise PilotError("ANSWER_SCHEMA_INVALID")
                descriptor_gate.safe_relative_kotlin_file(declaration["relativeFile"])
                source = closed(declaration["sourceRange"], {"startByte", "endByte"}, "ANSWER_SCHEMA_INVALID")
                if type(source["startByte"]) is not int or type(source["endByte"]) is not int or not 0 <= source["startByte"] < source["endByte"]:
                    raise PilotError("ANSWER_SCHEMA_INVALID")
            else:
                raise PilotError("ANSWER_SCHEMA_INVALID")
            identity = authority_digest(declaration)
            if identity in identities:
                raise PilotError("ANSWER_SCHEMA_INVALID")
            identities.add(identity)
    if seen_roles != {"provider", "consumer"}:
        raise PilotError("ANSWER_SCHEMA_INVALID")
    manual = answer["manualVerification"]
    if not isinstance(manual, list) or len(manual) > 64:
        raise PilotError("ANSWER_SCHEMA_INVALID")
    seen_manual: set[tuple[str, str]] = set()
    for row in manual:
        closed(row, {"category", "status", "requiredCheck"}, "ANSWER_SCHEMA_INVALID")
        binding = (row["category"], row["requiredCheck"])
        if (
            not isinstance(binding[0], str)
            or re.fullmatch(r"[A-Z][A-Z0-9_]{0,127}", binding[0]) is None
            or not isinstance(binding[1], str)
            or re.fullmatch(r"VERIFY_[A-Z0-9_]{1,120}", binding[1]) is None
            or row["status"] != "UNSURE"
            or binding in seen_manual
        ):
            raise PilotError("ANSWER_SCHEMA_INVALID")
        seen_manual.add(binding)
    return answer


def _canonical_exact(value: dict[str, Any]) -> dict[str, Any]:
    row = dict(value)
    row.pop("shapeStatus", None)
    return row


def _approved_source_side_count(
    opened_windows: set[tuple[str, str, int, int]], oracle_task: dict[str, Any]
) -> int:
    return sum(
        any(
            alias == side["serviceAlias"]
            and blob_oid in {row["blobOid"] for row in side["approvedFiles"]}
            for alias, blob_oid, _, _ in opened_windows
        )
        for side in oracle_task["sides"]
    )


def score_answer(
    answer: dict[str, Any],
    task: dict[str, Any],
    oracle_task: dict[str, Any],
    runtime: dict[str, int],
    opened_windows: set[tuple[str, str, int, int]],
) -> dict[str, Any]:
    oracle_sides = {row["role"]: row for row in oracle_task["sides"]}
    members = {row["role"]: row for row in answer["members"]}
    declared_hits = 0
    file_hits = 0
    exact_matches: list[tuple[str, dict[str, Any]]] = []
    false_exact = 0
    for role in ["provider", "consumer"]:
        member = members[role]
        expected = oracle_sides[role]
        if member["serviceAlias"] == expected["serviceAlias"] and member["revision"] == expected["revision"]:
            declared_hits += 1
        approved = {(row["relativeFile"], row["blobOid"]) for row in expected["approvedFiles"]}
        if any((row["relativeFile"], row["blobOid"]) in approved and row["rank"] <= 10 for row in member["rankedFiles"]):
            file_hits += 1
        oracle_declarations = {
            authority_digest(row): row for row in expected["exactDeclarations"]
        }
        for declaration in member["declarations"]:
            if declaration["shapeStatus"] != "EXACT_PROJECTED_DECLARATION":
                continue
            normalized = _canonical_exact(declaration)
            identity = authority_digest(normalized)
            if identity in oracle_declarations:
                exact_matches.append((role, normalized))
            else:
                false_exact += 1
    callable_hit = any(row["descriptorClass"] == "CALLABLE" for _, row in exact_matches)
    type_hit = any(row["descriptorClass"] == "TYPE" for _, row in exact_matches)
    expected_manual = {
        (row["category"], row["requiredCheck"])
        for row in oracle_task["manualVerification"]
    }
    observed_manual = {
        (row["category"], row["requiredCheck"])
        for row in answer["manualVerification"]
        if row["status"] == "UNSURE"
    }
    manual_hits = len(expected_manual & observed_manual)
    relationship = answer["relationship"]
    declared_topology_bound = relationship == {
        "authority": "DECLARED_TOPOLOGY",
        "status": "UNSURE",
    }
    false_exact += int(
        relationship["authority"] == "EXACT_RELATIONSHIP"
        or relationship["status"] == "EXACT"
    )
    false_exact += int(answer["httpEndpointEquivalence"] == "EXACT_CLAIMED")
    false_exact += int(answer["compatibility"] == "EXACT_CLAIMED")
    source_sides = 0
    for role, expected in oracle_sides.items():
        approved_blobs = {
            (expected["serviceAlias"], row["blobOid"])
            for row in expected["approvedFiles"]
        }
        if any(
            (alias, blob_oid) in approved_blobs
            for alias, blob_oid, _, _ in opened_windows
        ):
            source_sides += 1
    claimed_anchors_bound = all(
        any(
            alias == oracle_sides[role]["serviceAlias"]
            and blob_oid == row["blobOid"]
            and start <= row["sourceRange"]["startByte"]
            and row["sourceRange"]["endByte"] <= end
            for alias, blob_oid, start, end in opened_windows
        )
        for role, row in exact_matches
    )
    # The public projection intentionally carries no source locators.  Its
    # side count therefore means "oracle-approved evidence with every claimed
    # exact range bound", so the checked verifier can independently enforce
    # the bounded-evidence boolean without learning private anchors.
    bound_source_sides = source_sides if claimed_anchors_bound else 0
    resource_pass = (
        runtime["elapsedMillis"] <= BUDGETS["wallMs"]
        and runtime["noncachedInputTokens"] <= BUDGETS["noncachedInputTokens"]
        and runtime["toolStarts"] <= BUDGETS["toolStarts"]
        and runtime["queryTerms"] <= BUDGETS["queryTerms"]
        and runtime["returnedFacts"] <= BUDGETS["returnedFacts"]
        and runtime["selectedFiles"] <= BUDGETS["selectedFiles"]
        and runtime["selectedFiles"] >= runtime["openedSourceFiles"]
        and runtime["sourceWindows"] <= BUDGETS["sourceWindows"]
        and runtime["agentVisibleEvidenceBytes"] <= BUDGETS["agentVisibleEvidenceBytes"]
        and runtime["answerBytes"] <= BUDGETS["answerBytes"]
        and runtime["contextCreates"] <= BUDGETS["contextCreates"]
        and runtime["contextExpansions"] <= BUDGETS["contextExpansions"]
        and runtime["maxSemanticCommandMillis"] <= BUDGETS["singleSemanticCommandMs"]
        and runtime["capabilityViolations"] == 0
        and runtime["budgetRefusals"] == 0
    )
    criteria = {
        "exactAuthority": declared_hits == 2
        and declared_topology_bound
        and (
            answer["arm"] == "DEFAULT"
            or (
                runtime["semanticContextCommands"] == 1
                and runtime["semanticCallablesCommands"] == 1
                and runtime["semanticImpactCommands"] >= 1
            )
        ),
        "approvedFileBothSides": file_hits == 2,
        "callableNavigation": callable_hit,
        "typeNavigation": type_hit,
        "boundedSourceEvidence": (
            bound_source_sides == 2
            and 0 < runtime["openedSourceBytes"] <= BUDGETS["agentVisibleEvidenceBytes"]
            and runtime["openedSourceFiles"] >= source_sides
            and runtime["selectedFiles"] <= BUDGETS["selectedFiles"]
            and 0 < runtime["sourceWindows"] <= BUDGETS["sourceWindows"]
            and runtime["agentVisibleEvidenceBytes"] >= runtime["openedSourceBytes"]
        ),
        "completeManualVerification": observed_manual == expected_manual,
        "zeroFalseExactClaims": false_exact == 0,
        "resourceBudgetsPass": resource_pass,
    }
    return {
        **runtime,
        "result": "PASS" if all(criteria.values()) else "FAIL",
        "criteria": criteria,
        "declaredMemberHits": declared_hits,
        "top10RelevantFileHits": file_hits,
        "descriptorSlotHits": int(callable_hit) + int(type_hit),
        "manualCategoryExpectedCount": len(expected_manual),
        "manualCategoryHits": manual_hits,
        "falseExactClaimCount": false_exact,
        "sourceEvidenceSideCount": bound_source_sides,
        "declaredTopologyBound": declared_topology_bound,
    }


def _broker_command(command: Any) -> bool:
    if not isinstance(command, str) or not command or "\n" in command:
        return False
    candidate = command
    try:
        outer = shlex.split(command, posix=True)
    except ValueError:
        return False
    if (
        len(outer) == 3
        and Path(outer[0]).name in {"sh", "bash", "zsh"}
        and outer[1] in {"-c", "-lc"}
    ):
        candidate = outer[2]
    if any(character in candidate for character in [";", "|", "&", ">", "<", "`", "\n", "\r", "$("]):
        return False
    try:
        tokens = shlex.split(candidate, posix=True)
    except ValueError:
        return False
    return bool(tokens) and tokens[0] == "pilot-tool" and tokens[1:2] in [
        ["capability"], ["tree"], ["show"], ["search"], ["read"],
        ["semantic-context"], ["semantic-callables"], ["semantic-impact"],
    ]


def _broker_operation(command: Any) -> str | None:
    """Return the closed broker operation for an already-audited command."""

    if not _broker_command(command):
        return None
    assert isinstance(command, str)
    outer = shlex.split(command, posix=True)
    candidate = (
        outer[2]
        if len(outer) == 3
        and Path(outer[0]).name in {"sh", "bash", "zsh"}
        and outer[1] in {"-c", "-lc"}
        else command
    )
    return shlex.split(candidate, posix=True)[1]


def _process_group_exists(process_group: int) -> bool:
    if process_group <= 1:
        return False
    try:
        os.killpg(process_group, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def _wait_process_group_gone(process_group: int, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while _process_group_exists(process_group):
        if time.monotonic() >= deadline:
            return False
        time.sleep(0.01)
    return True


def _kill_group(process: subprocess.Popen[Any]) -> bool:
    """Boundedly terminate the original session, even if its leader exited."""

    process_group = process.pid
    if _process_group_exists(process_group):
        try:
            os.killpg(process_group, signal.SIGTERM)
        except ProcessLookupError:
            pass
        if process.poll() is None:
            try:
                process.wait(timeout=0.25)
            except subprocess.TimeoutExpired:
                pass
        if not _wait_process_group_gone(process_group, 0.25):
            try:
                os.killpg(process_group, signal.SIGKILL)
            except ProcessLookupError:
                pass
            _wait_process_group_gone(process_group, 2.0)
    if process.poll() is None:
        try:
            process.wait(timeout=0.25)
        except subprocess.TimeoutExpired:
            try:
                process.kill()
            except OSError:
                pass
            try:
                process.wait(timeout=0.25)
            except subprocess.TimeoutExpired:
                pass
    return not _process_group_exists(process_group) and process.poll() is not None


def _interrupt_active_arm(_signum: int, _frame: Any) -> None:
    global _ACTIVE_PROCESS, _ACTIVE_BROKER_STOP
    if _ACTIVE_BROKER_STOP is not None:
        _ACTIVE_BROKER_STOP.set()
    if _ACTIVE_BROKER_SESSION is not None:
        _ACTIVE_BROKER_SESSION.terminate_active_children()
    if _ACTIVE_PROCESS is not None:
        _kill_group(_ACTIVE_PROCESS)
    if _ACTIVE_BROKER_THREAD is not None:
        _ACTIVE_BROKER_THREAD.join(timeout=5)
        if _ACTIVE_BROKER_THREAD.is_alive():
            raise PilotError("BROKER_SERVER_FAILED")
    raise PilotError("INTERRUPTED")


@contextmanager
def _arm_execution_guard(
    stop: threading.Event,
    server: threading.Thread,
    session: broker.BrokerSession,
    server_failure: list[str],
) -> Any:
    """Always tear down the model and broker capability contour."""

    global _ACTIVE_PROCESS, _ACTIVE_BROKER_STOP
    global _ACTIVE_BROKER_THREAD, _ACTIVE_BROKER_SESSION
    teardown_error: str | None = None
    try:
        yield
    finally:
        stop.set()
        session.terminate_active_children()
        if _ACTIVE_PROCESS is not None and not _kill_group(_ACTIVE_PROCESS):
            teardown_error = "PROCESS_RESIDUAL"
        server.join(timeout=5)
        if server.is_alive() or server_failure:
            teardown_error = "BROKER_SERVER_FAILED"
        _ACTIVE_PROCESS = None
        _ACTIVE_BROKER_STOP = None
        _ACTIVE_BROKER_THREAD = None
        _ACTIVE_BROKER_SESSION = None
        if teardown_error is not None:
            raise PilotError(teardown_error)


def _jsonl_event(raw: bytes) -> dict[str, Any]:
    if not raw or len(raw) > 4 * 1024 * 1024:
        raise PilotError("CODEX_JSONL_INVALID")
    try:
        value = json.loads(raw)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise PilotError("CODEX_JSONL_INVALID") from error
    if not isinstance(value, dict) or not isinstance(value.get("type"), str):
        raise PilotError("CODEX_JSONL_INVALID")
    return value


def _usage(event: dict[str, Any]) -> dict[str, int] | None:
    if event.get("type") != "turn.completed":
        return None
    usage = event.get("usage")
    fields = {
        "input_tokens",
        "cached_input_tokens",
        "cache_write_input_tokens",
        "output_tokens",
        "reasoning_output_tokens",
    }
    if not isinstance(usage, dict) or set(usage) != fields:
        raise PilotError("CODEX_USAGE_INVALID")
    result: dict[str, int] = {}
    for field in fields:
        value = usage[field]
        if type(value) is not int or value < 0:
            raise PilotError("CODEX_USAGE_INVALID")
        result[field] = value
    if result["cached_input_tokens"] > result["input_tokens"]:
        raise PilotError("CODEX_USAGE_INVALID")
    return result


def _started_item_policy(item: Any) -> tuple[int, bool]:
    """Return (command starts, allowed) for one Codex item.started payload."""

    if not isinstance(item, dict) or not isinstance(item.get("type"), str):
        return 0, False
    item_type = item["type"]
    if item_type == "command_execution":
        return 1, _broker_command(item.get("command"))
    # These item kinds cannot execute a capability.  Every current or future
    # active item kind is refused until this closed grammar is reviewed again.
    return 0, item_type in {"reasoning", "agent_message", "todo_list"}


def _failed_score(runtime: dict[str, int], arm: str) -> dict[str, Any]:
    resource_pass = (
        runtime["elapsedMillis"] <= BUDGETS["wallMs"]
        and runtime["noncachedInputTokens"] <= BUDGETS["noncachedInputTokens"]
        and runtime["toolStarts"] <= BUDGETS["toolStarts"]
        and runtime["queryTerms"] <= BUDGETS["queryTerms"]
        and runtime["returnedFacts"] <= BUDGETS["returnedFacts"]
        and runtime["selectedFiles"] <= BUDGETS["selectedFiles"]
        and runtime["selectedFiles"] >= runtime["openedSourceFiles"]
        and runtime["sourceWindows"] <= BUDGETS["sourceWindows"]
        and runtime["agentVisibleEvidenceBytes"] <= BUDGETS["agentVisibleEvidenceBytes"]
        and runtime["answerBytes"] <= BUDGETS["answerBytes"]
        and runtime["contextCreates"] <= BUDGETS["contextCreates"]
        and runtime["contextExpansions"] <= BUDGETS["contextExpansions"]
        and runtime["maxSemanticCommandMillis"] <= BUDGETS["singleSemanticCommandMs"]
        and runtime["capabilityViolations"] == 0
        and runtime["budgetRefusals"] == 0
    )
    bounded = (
        runtime["sourceEvidenceSideCount"] == 2
        and 0 < runtime["openedSourceBytes"] <= BUDGETS["agentVisibleEvidenceBytes"]
        and runtime["openedSourceFiles"] >= runtime["sourceEvidenceSideCount"]
        and runtime["selectedFiles"] <= BUDGETS["selectedFiles"]
        and 0 < runtime["sourceWindows"] <= BUDGETS["sourceWindows"]
        and runtime["agentVisibleEvidenceBytes"] >= runtime["openedSourceBytes"]
    )
    return {
        "result": "FAIL",
        "criteria": {
            "exactAuthority": False,
            "approvedFileBothSides": False,
            "callableNavigation": False,
            "typeNavigation": False,
            "boundedSourceEvidence": bounded,
            "completeManualVerification": False,
            "zeroFalseExactClaims": True,
            "resourceBudgetsPass": resource_pass,
        },
        "declaredMemberHits": 0,
        "top10RelevantFileHits": 0,
        "descriptorSlotHits": 0,
        "manualCategoryExpectedCount": 0,
        "manualCategoryHits": 0,
        "falseExactClaimCount": 0,
        "declaredTopologyBound": False,
        **runtime,
    }


def _runtime_metrics(
    elapsed_millis: int,
    tool_starts: int,
    usage: dict[str, int],
    broker_metrics: dict[str, int],
    answer_bytes: int,
    source_evidence_side_count: int,
) -> dict[str, int]:
    return {
        "elapsedMillis": elapsed_millis,
        "openedSourceBytes": broker_metrics["openedSourceBytes"],
        "openedSourceFiles": broker_metrics["openedSourceFiles"],
        "toolStarts": tool_starts,
        "noncachedInputTokens": usage["input_tokens"] - usage["cached_input_tokens"],
        "queryTerms": broker_metrics["queryTerms"],
        "returnedFacts": broker_metrics["returnedFacts"],
        "sourceWindows": broker_metrics["sourceWindows"],
        "agentVisibleEvidenceBytes": broker_metrics["agentVisibleEvidenceBytes"],
        "answerBytes": answer_bytes,
        "contextCreates": broker_metrics["contextCreates"],
        "contextExpansions": broker_metrics["contextExpansions"],
        "maxSemanticCommandMillis": broker_metrics["maxSemanticCommandMillis"],
        "selectedFiles": broker_metrics["selectedFiles"],
        "sourceEvidenceSideCount": source_evidence_side_count,
        "capabilityViolations": broker_metrics["capabilityViolations"],
        "budgetRefusals": broker_metrics["budgetRefusals"],
        "semanticContextCommands": broker_metrics["semanticContextCommands"],
        "semanticCallablesCommands": broker_metrics["semanticCallablesCommands"],
        "semanticImpactCommands": broker_metrics["semanticImpactCommands"],
    }


def run_arm(
    authority: dict[str, Any],
    oracle: dict[str, Any],
    task: dict[str, Any],
    arm: str,
    private_codex_home: Path,
    private_codex_home_identity: dict[str, int],
    on_scratch: Callable[[dict[str, Any]], None] | None = None,
) -> dict[str, Any]:
    global _ACTIVE_PROCESS, _ACTIVE_BROKER_STOP
    global _ACTIVE_BROKER_THREAD, _ACTIVE_BROKER_SESSION
    _require_python_runtime_authority(authority["executables"])
    oracle_task = next(row for row in oracle["tasks"] if row["taskId"] == task["taskId"])
    started = time.monotonic_ns()
    tool_starts = 0
    usages: list[dict[str, int]] = []
    codex_tool_ledger: list[dict[str, Any]] = []
    last_event_type: str | None = None
    jsonl_digest = hashlib.sha256()
    failure: str | None = None
    return_code: int | None = None
    scratch_locator: dict[str, Any] | None = None
    experiment_root = Path(authority["experimentRoot"]["path"])
    with tempfile.TemporaryDirectory(
        prefix=".codeclew-s4k-arm-",
        dir=os.fspath(experiment_root),
    ) as directory:
        scratch = Path(directory).resolve(strict=True)
        os.chmod(scratch, 0o700)
        scratch_locator = _arm_scratch_locator(scratch, task["taskId"], arm)
        if on_scratch is not None:
            on_scratch(scratch_locator)
        broker_tool = scratch / "pilot-tool"
        broker_tool.write_bytes((Path(__file__).parent / "thread_kotlin_pilot_broker.py").read_bytes())
        os.chmod(broker_tool, 0o700)
        schema_path = scratch / "answer.schema.json"
        schema_path.write_bytes((Path(__file__).parent / "schemas" / "thread-kotlin-pilot-answer.schema.json").read_bytes())
        os.chmod(schema_path, 0o600)
        answer_path = scratch / "answer.json"
        answer_path.touch(mode=0o600)
        request_directory = scratch / "broker-requests"
        response_directory = scratch / "broker-responses"
        request_directory.mkdir(mode=0o700)
        response_directory.mkdir(mode=0o700)
        token = secrets.token_hex(32)
        broker_session = broker.BrokerSession(authority, task["taskId"], arm)
        stop = threading.Event()
        server_failure: list[str] = []

        def serve_broker() -> None:
            try:
                broker.serve_directories(
                    request_directory,
                    response_directory,
                    broker_session,
                    token,
                    stop,
                )
            except Exception:
                server_failure.append("BROKER_SERVER_FAILED")
                stop.set()

        server = threading.Thread(
            target=serve_broker,
            daemon=True,
        )
        server.start()
        _ACTIVE_BROKER_THREAD = server
        _ACTIVE_BROKER_SESSION = broker_session
        environment = _private_arm_environment(
            authority["codexEnvironment"],
            scratch,
            private_codex_home,
            private_codex_home_identity,
        )
        python_directory = Path(authority["executables"]["python"]).parent
        environment["PATH"] = f"{scratch}:{python_directory}:/usr/bin:/bin"
        environment["ZDOTDIR"] = os.fspath(scratch)
        environment["CODECLEW_PILOT_BROKER_REQUESTS"] = os.fspath(request_directory)
        environment["CODECLEW_PILOT_BROKER_RESPONSES"] = os.fspath(response_directory)
        environment["CODECLEW_PILOT_BROKER_TOKEN"] = token
        command = _codex_exec_command(
            authority, scratch, schema_path, answer_path
        )
        with _arm_execution_guard(stop, server, broker_session, server_failure):
            try:
                process = subprocess.Popen(
                    command,
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                    start_new_session=True,
                    close_fds=True,
                    env=environment,
                )
            except OSError as error:
                stop.set()
                server.join(timeout=5)
                raise PilotError("CODEX_START_FAILED") from error
            _ACTIVE_PROCESS = process
            _ACTIVE_BROKER_STOP = stop
            assert process.stdin is not None and process.stdout is not None
            process.stdin.write(task["prompt"].encode("utf-8"))
            process.stdin.close()
            descriptor = process.stdout.fileno()
            os.set_blocking(descriptor, False)
            selector = selectors.DefaultSelector()
            selector.register(descriptor, selectors.EVENT_READ)
            buffer = bytearray()
            total_jsonl = 0
            deadline = started + BUDGETS["wallMs"] * 1_000_000
            eof = False

            def consume_chunk(chunk: bytes) -> None:
                nonlocal total_jsonl, buffer, tool_starts, failure, eof, last_event_type
                total_jsonl += len(chunk)
                if total_jsonl > MAX_JSONL_BYTES:
                    failure = "CODEX_JSONL_LIMIT"
                    _kill_group(process)
                    eof = True
                    return
                jsonl_digest.update(chunk)
                buffer.extend(chunk)
                while b"\n" in buffer:
                    line, _, remainder = buffer.partition(b"\n")
                    buffer = bytearray(remainder)
                    if not line:
                        continue
                    try:
                        event = _jsonl_event(line)
                        usage = _usage(event)
                    except PilotError as error:
                        failure = error.code
                        _kill_group(process)
                        eof = True
                        return
                    last_event_type = event["type"]
                    if usage is not None:
                        usages.append(usage)
                    if event["type"] != "item.started":
                        continue
                    item = event.get("item")
                    command_starts, allowed = _started_item_policy(item)
                    tool_starts += command_starts
                    if command_starts:
                        assert isinstance(item, dict)
                        command_value = item.get("command")
                        operation = _broker_operation(command_value)
                        codex_tool_ledger.append(
                            {
                                "sequence": tool_starts,
                                "operation": operation,
                                "commandDigest": authority_digest(command_value),
                            }
                        )
                        if tool_starts > BUDGETS["toolStarts"]:
                            failure = "TOOL_START_BUDGET_EXCEEDED"
                        elif not allowed:
                            failure = "CAPABILITY_VIOLATION"
                    elif not allowed:
                        failure = "CAPABILITY_VIOLATION"
                    if failure is not None:
                        _kill_group(process)
                        eof = True
                        return

            while not eof:
                if server_failure:
                    failure = server_failure[0]
                    _kill_group(process)
                    break
                if time.monotonic_ns() >= deadline:
                    failure = "WALL_BUDGET_EXCEEDED"
                    _kill_group(process)
                    break
                events = selector.select(timeout=0.1)
                if not events and process.poll() is not None:
                    try:
                        chunk = os.read(descriptor, 65_536)
                    except BlockingIOError:
                        chunk = b""
                    if not chunk:
                        eof = True
                        continue
                    consume_chunk(chunk)
                for _, _ in events:
                    try:
                        chunk = os.read(descriptor, 65_536)
                    except BlockingIOError:
                        continue
                    if not chunk:
                        eof = True
                        break
                    consume_chunk(chunk)
                    if eof:
                        break
            if buffer and failure is None:
                failure = "CODEX_JSONL_INVALID"
                _kill_group(process)
            if process.poll() is None:
                try:
                    return_code = process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    _kill_group(process)
                    failure = failure or "PROCESS_RESIDUAL"
                    return_code = process.returncode
            else:
                return_code = process.returncode
            if _process_group_exists(process.pid):
                _kill_group(process)
                failure = failure or "PROCESS_RESIDUAL"
        elapsed = max(1, (time.monotonic_ns() - started + 999_999) // 1_000_000)
        broker_metrics = broker_session.metrics.projection()
        if len(usages) != 1 or last_event_type != "turn.completed":
            failure = failure or "CODEX_USAGE_INVALID"
        arm_failure_class: str | None = None
        arm_failure_code: str | None = None
        if return_code != 0 and failure is None:
            arm_failure_class = "MODEL_EXIT"
            arm_failure_code = "CODEX_COMMAND_FAILED"
        answer_raw = b""
        if failure is None:
            try:
                answer_raw = _bounded_private_answer(answer_path)
            except PilotError as error:
                if arm_failure_class is None:
                    arm_failure_class = "MODEL_OUTPUT"
                    arm_failure_code = error.code
        usage = usages[0] if len(usages) == 1 else None
        if usage is None:
            raise PilotError(failure or "CODEX_USAGE_INVALID")
        broker_provenance = broker_session.audit_projection()
        if any(
            row.get("residualAfterCleanup") is True
            for row in broker_provenance["processGroupLedger"]
            if isinstance(row, dict)
        ):
            raise PilotError("BROKER_PROCESS_GROUP_RESIDUAL")
        broker_order = [
            (row.get("order"), row.get("operation"))
            for row in broker_provenance["orderedToolLedger"]
            if isinstance(row, dict)
        ]
        codex_order = [
            (row["sequence"], row["operation"]) for row in codex_tool_ledger
        ]
        if codex_order != broker_order:
            raise PilotError("BROKER_TOOL_LEDGER_MISMATCH")
        violation_codes = {
            row.get("code")
            for row in broker_provenance["violationLedger"]
            if isinstance(row, dict)
        }
        if violation_codes & BROKER_PROTOCOL_VIOLATIONS:
            raise PilotError("BROKER_PROTOCOL_VIOLATION")
        opened_windows = {
            (
                row["serviceAlias"],
                row["blobOid"],
                row["startByte"],
                row["endByte"],
            )
            for row in broker_provenance["sourceWindows"]
        }
        source_side_count = _approved_source_side_count(opened_windows, oracle_task)
        runtime = _runtime_metrics(
            elapsed,
            tool_starts,
            usage,
            broker_metrics,
            len(answer_raw),
            source_side_count,
        )
        if broker_metrics["budgetRefusals"]:
            arm_failure_class = "RESOURCE_LIMIT"
            arm_failure_code = "BROKER_RESOURCE_BUDGET"
        elif runtime["noncachedInputTokens"] > BUDGETS["noncachedInputTokens"]:
            arm_failure_class = "RESOURCE_LIMIT"
            arm_failure_code = "NONCACHED_TOKEN_BUDGET_EXCEEDED"
        elif broker_metrics["capabilityViolations"]:
            arm_failure_class = "PRODUCT_REFUSAL"
            arm_failure_code = next(
                (
                    row["code"]
                    for row in broker_provenance["violationLedger"]
                    if isinstance(row, dict)
                    and isinstance(row.get("code"), str)
                ),
                "BROKER_CAPABILITY_REFUSED",
            )
        answer: dict[str, Any] | None = None
        score: dict[str, Any]
        if failure is not None:
            raise PilotError(failure)
        if answer_raw:
            try:
                parsed = json.loads(answer_raw, object_pairs_hook=_duplicates)
                answer = validate_answer(parsed, task, arm)
            except (json.JSONDecodeError, UnicodeDecodeError, PilotError) as error:
                if arm_failure_class is None:
                    arm_failure_class = "MODEL_OUTPUT"
                    arm_failure_code = (
                        error.code
                        if isinstance(error, PilotError)
                        else "ANSWER_JSON_INVALID"
                    )
        if arm_failure_class is None:
            if answer is None:
                raise PilotError("ANSWER_AUDIT_INCONSISTENT")
            score = score_answer(
                answer,
                task,
                oracle_task,
                runtime,
                opened_windows,
            )
        else:
            score = _failed_score(runtime, arm)
            score["manualCategoryExpectedCount"] = len(
                oracle_task["manualVerification"]
            )
        arm_unsigned = {
            "taskId": task["taskId"],
            "pairId": task["pairId"],
            "arm": arm,
            "promptDigest": task["promptDigest"],
            "status": "COMPLETE" if arm_failure_class is None else "ARM_FAILURE",
            "failureClass": arm_failure_class,
            "failureCode": arm_failure_code,
            "answer": answer,
            "answerDigest": authority_digest(answer) if answer is not None else None,
            "jsonlDigest": f"sha256:{jsonl_digest.hexdigest()}",
            "modelReturnCode": return_code,
            "usage": usage,
            "elapsedMillis": elapsed,
            "answerBytes": len(answer_raw),
            "scratchLocator": scratch_locator,
            "codexToolLedger": codex_tool_ledger,
            "brokerProvenance": broker_provenance,
            "score": score,
        }
        return {**arm_unsigned, "armDigest": authority_digest(arm_unsigned)}


def execute(args: argparse.Namespace) -> dict[str, Any]:
    global _ACTIVE_PROCESS, _ACTIVE_BROKER_STOP
    global _ACTIVE_BROKER_THREAD, _ACTIVE_BROKER_SESSION
    experiment_root, _ = _experiment_root(args.experiment_root)
    _require_experiment_paths(
        experiment_root,
        [
            args.private_authority,
            args.private_oracle,
            args.implementation_review_manifest,
            args.private_output,
        ],
    )
    output_locator = _output_locator(args.private_output)
    codex_auth_lease = _recover_codex_auth_lease_for_output(output_locator)
    output = fresh_output_target(output_locator)
    authority_path, authority_value, _ = private_json(args.private_authority, "PILOT_AUTHORITY")
    oracle_path, oracle_value, _ = private_json(args.private_oracle, "PILOT_ORACLE")
    implementation_path, _, _ = checked_json(
        args.implementation_review_manifest, "IMPLEMENTATION_REVIEW", 256 * 1024
    )
    require_distinct_paths(
        [authority_path, oracle_path, implementation_path], [output]
    )
    authority = verify_authority(authority_value)
    _verify_resource_ledger(authority_path, authority)
    oracle = verify_oracle(oracle_value, authority)
    _, implementation_review_digest = _implementation_review(
        implementation_path, authority
    )
    run_owner = {"ownerPid": os.getpid(), "ownerToken": secrets.token_hex(32)}
    _admit_execute(experiment_root, authority, output, run_owner)
    runs: list[dict[str, Any]] = []
    active_arm: dict[str, Any] | None = None
    tasks = {task["taskId"]: task for task in authority["tasks"]}
    previous_sigint = signal.getsignal(signal.SIGINT)
    previous_sigterm = signal.getsignal(signal.SIGTERM)
    signal.signal(signal.SIGINT, _interrupt_active_arm)
    signal.signal(signal.SIGTERM, _interrupt_active_arm)
    created_unsigned = {
        "schema": PRIVATE_RUN_SCHEMA,
        "status": "CREATED",
        **run_owner,
        "authorityDigest": authority["authorityDigest"],
        "protocolDigest": authority["protocolDigest"],
        "implementationReviewManifestDigest": implementation_review_digest,
        "completedArmCount": 0,
        "failureCode": None,
        "activeArm": None,
        "arms": [],
    }
    _create_private_once(
        output,
        {**created_unsigned, "runDigest": authority_digest(created_unsigned)},
        "PRIVATE_RUN_CREATE_FAILED",
    )
    private_codex_home, private_codex_home_identity = (
        _create_private_codex_home(
            Path(authority["codexEnvironment"]["CODEX_HOME"]),
            codex_auth_lease,
        )
    )
    try:
        for order in authority["armOrder"]:
            for arm in order["arms"]:
                def checkpoint_arm(locator: dict[str, Any]) -> None:
                    nonlocal active_arm
                    active_arm = locator
                    running_unsigned = {
                        "schema": PRIVATE_RUN_SCHEMA,
                        "status": "RUNNING",
                        **run_owner,
                        "authorityDigest": authority["authorityDigest"],
                        "protocolDigest": authority["protocolDigest"],
                        "implementationReviewManifestDigest": implementation_review_digest,
                        "completedArmCount": len(runs),
                        "failureCode": None,
                        "activeArm": active_arm,
                        "arms": runs,
                    }
                    _write_owned_private_run(
                        output,
                        {
                            **running_unsigned,
                            "runDigest": authority_digest(running_unsigned),
                        },
                        run_owner["ownerPid"],
                        run_owner["ownerToken"],
                    )

                runs.append(
                    run_arm(
                        authority,
                        oracle,
                        tasks[order["taskId"]],
                        arm,
                        private_codex_home,
                        private_codex_home_identity,
                        checkpoint_arm,
                    )
                )
                active_arm = None
                running_unsigned = {
                    "schema": PRIVATE_RUN_SCHEMA,
                    "status": "RUNNING",
                    **run_owner,
                    "authorityDigest": authority["authorityDigest"],
                    "protocolDigest": authority["protocolDigest"],
                    "implementationReviewManifestDigest": implementation_review_digest,
                    "completedArmCount": len(runs),
                    "failureCode": None,
                    "activeArm": None,
                    "arms": runs,
                }
                _write_owned_private_run(
                    output,
                    {
                        **running_unsigned,
                        "runDigest": authority_digest(running_unsigned),
                    },
                    run_owner["ownerPid"],
                    run_owner["ownerToken"],
                )
    except PilotError as error:
        invalid_unsigned = {
            "schema": PRIVATE_RUN_SCHEMA,
            "status": "INVALID_RUN",
            **run_owner,
            "authorityDigest": authority["authorityDigest"],
            "protocolDigest": authority["protocolDigest"],
            "implementationReviewManifestDigest": implementation_review_digest,
            "completedArmCount": len(runs),
            "failureCode": error.code,
            "activeArm": active_arm,
            "arms": runs,
        }
        _write_owned_private_run(
            output,
            {**invalid_unsigned, "runDigest": authority_digest(invalid_unsigned)},
            run_owner["ownerPid"],
            run_owner["ownerToken"],
        )
        raise
    finally:
        _ACTIVE_PROCESS = None
        _ACTIVE_BROKER_STOP = None
        _ACTIVE_BROKER_THREAD = None
        _ACTIVE_BROKER_SESSION = None
        signal.signal(signal.SIGINT, previous_sigint)
        signal.signal(signal.SIGTERM, previous_sigterm)
        _recover_private_codex_home(codex_auth_lease)
    complete_unsigned = {
        "schema": PRIVATE_RUN_SCHEMA,
        "status": "COMPLETE",
        **run_owner,
        "authorityDigest": authority["authorityDigest"],
        "protocolDigest": authority["protocolDigest"],
        "implementationReviewManifestDigest": implementation_review_digest,
        "completedArmCount": len(runs),
        "failureCode": None,
        "activeArm": None,
        "arms": runs,
    }
    private = {
        **complete_unsigned,
        "runDigest": authority_digest(complete_unsigned),
    }
    _write_owned_private_run(
        output, private, run_owner["ownerPid"], run_owner["ownerToken"]
    )
    return {
        "schema": PRIVATE_RUN_SCHEMA,
        "status": "COMPLETE",
        "armCount": len(runs),
    }


def warm(args: argparse.Namespace) -> dict[str, Any]:
    """Admit a separately audited real warm run, never synthesize one.

    ``prepare`` seals the checked host adapter's bytes.  This phase verifies
    that exact adapter's attestation, canaries, state snapshots, and 30 samples
    before producing the smaller private projection consumed by ``project``.
    """

    experiment_root, _ = _experiment_root(args.experiment_root)
    _require_experiment_paths(
        experiment_root,
        [args.private_authority, args.private_attestation, args.private_output],
    )
    output = fresh_output_target(args.private_output)
    authority_path, authority_value, _ = private_json(args.private_authority, "PILOT_AUTHORITY")
    attestation_path, attestation, _ = private_json(args.private_attestation, "WARM_ATTESTATION")
    require_distinct_paths([authority_path, attestation_path], [output])
    authority = verify_authority(authority_value)
    _verify_resource_ledger(authority_path, authority)
    runtime_key = next(iter({row["runtimeKey"] for row in authority["sessions"]}))
    environment = authority["semanticEnvironment"]
    state_root = _state_root(environment)
    home = Path(environment["HOME"])
    cache_candidates = [
        ("CODECLEW_DEPENDENCY", state_root / "dependency-cache"),
        ("GRADLE", Path(environment.get("GRADLE_USER_HOME", os.fspath(home / ".gradle")))),
        ("MAVEN", Path(environment.get("MAVEN_USER_HOME", os.fspath(home / ".m2")))),
        ("CARGO", Path(environment.get("CARGO_HOME", os.fspath(home / ".cargo")))),
        ("RUSTUP", Path(environment.get("RUSTUP_HOME", os.fspath(home / ".rustup")))),
    ]
    resolved_home = home.resolve(strict=False)
    if any(
        not path.is_absolute()
        or path.resolve(strict=False) in {Path("/"), resolved_home}
        for _, path in cache_candidates
    ):
        raise PilotError("WARM_ATTESTATION_INVALID")
    effective_cache_roots = [
        {
            "label": label,
            "pathDigest": authority_digest(os.fspath(path.resolve(strict=False))),
        }
        for label, path in cache_candidates
    ]
    cache_body_digests = [
        "sha256:"
        + hashlib.sha256(
            (
                "codeclew-warm-cache-canary:"
                + authority["authorityDigest"]
                + ":"
                + row["label"]
                + "\n"
            ).encode("ascii")
        ).hexdigest()
        for row in effective_cache_roots
    ]
    expected_cache_sentinel_digest = authority_digest(cache_body_digests)

    unsigned = dict(attestation)
    declared = unsigned.pop("authorityDigest", None)
    expected = {
        "schema", "pilotAuthorityDigest", "protocolDigest", "clewDigest",
        "adapterDigest", "measurementClass", "profilePolicy", "profileDigest",
        "fixtureTreeDigest", "executionAuthority", "effectiveCacheRoots", "setup",
        "priming", "stateSnapshotBefore", "stateSnapshotAfter",
        "stateSnapshotUnchanged", "fixture", "samplesNanos", "stdoutDigests",
        "audit", "cleanupCompleted", "privateOutputMode",
    }
    if (
        set(unsigned) != expected
        or attestation.get("schema") != PRIVATE_WARM_ATTESTATION_SCHEMA
        or declared != authority_digest(unsigned)
        or attestation.get("pilotAuthorityDigest") != authority["authorityDigest"]
        or attestation.get("protocolDigest") != authority["protocolDigest"]
        or attestation.get("clewDigest") != authority["executables"]["clewDigest"]
        or attestation.get("adapterDigest") != authority["inputs"]["warmAuditAdapterDigest"]
        or attestation.get("measurementClass") != "MEASURED"
        or attestation.get("profilePolicy") != "GLOBAL_WRITE_DENY_STATE_LOCKS_ONLY_V1"
        or not isinstance(attestation.get("profileDigest"), str)
        or SHA256.fullmatch(attestation["profileDigest"]) is None
        or attestation.get("fixtureTreeDigest") != authority["inputs"]["warmFixtureDigest"]
        or attestation.get("effectiveCacheRoots") != effective_cache_roots
        or not isinstance(attestation.get("stateSnapshotBefore"), str)
        or SHA256.fullmatch(attestation["stateSnapshotBefore"]) is None
        or attestation.get("stateSnapshotAfter") != attestation.get("stateSnapshotBefore")
        or attestation.get("stateSnapshotUnchanged") is not True
        or attestation.get("cleanupCompleted") is not True
        or attestation.get("privateOutputMode") != "0600"
    ):
        raise PilotError("WARM_ATTESTATION_INVALID")
    execution = closed(
        attestation["executionAuthority"],
        {"pythonDigest", "stateRootDigest", "runtimeKey"},
        "WARM_ATTESTATION_INVALID",
    )
    if execution != {
        "pythonDigest": authority["executables"]["pythonDigest"],
        "stateRootDigest": authority_digest(os.fspath(state_root)),
        "runtimeKey": runtime_key,
    }:
        raise PilotError("WARM_ATTESTATION_INVALID")
    setup = closed(
        attestation["setup"],
        {
            "coldPreparationOutsideMeasuredInterval",
            "coldProcessAndCacheAccessPermitted",
            "coldNetworkAccessPermitted",
            "coldPreparationCommandCount",
            "coldPreparationElapsedNanos",
            "runtimeKey",
        },
        "WARM_ATTESTATION_INVALID",
    )
    if (
        setup["coldPreparationOutsideMeasuredInterval"] is not True
        or setup["coldProcessAndCacheAccessPermitted"] is not True
        or setup["coldNetworkAccessPermitted"] is not True
        or setup["coldPreparationCommandCount"] != 10
        or type(setup["coldPreparationElapsedNanos"]) is not int
        or setup["coldPreparationElapsedNanos"] <= 0
        or setup["runtimeKey"] != runtime_key
    ):
        raise PilotError("WARM_ATTESTATION_INVALID")
    priming = closed(
        attestation["priming"],
        {
            "commandCount", "elapsedNanos", "runtimeKey", "stdoutDigest",
            "profileDigest", "networkPolicy", "cacheReadPolicy", "processPolicy",
        },
        "WARM_ATTESTATION_INVALID",
    )
    if (
        priming["commandCount"] != 1
        or type(priming["elapsedNanos"]) is not int
        or priming["elapsedNanos"] <= 0
        or priming["runtimeKey"] != runtime_key
        or not isinstance(priming["stdoutDigest"], str)
        or SHA256.fullmatch(priming["stdoutDigest"]) is None
        or priming["profileDigest"] != attestation["profileDigest"]
        or priming["networkPolicy"] != "KERNEL_DENIED"
        or priming["cacheReadPolicy"] != "EFFECTIVE_ROOTS_DENIED"
        or priming["processPolicy"] != "SEALED_ALLOWLIST_ONLY"
    ):
        raise PilotError("WARM_ATTESTATION_INVALID")
    fixture = closed(
        attestation["fixture"],
        {"expectedShapeCount", "matchedShapeCount", "falseExactClaimCount"},
        "WARM_ATTESTATION_INVALID",
    )
    if (
        fixture["expectedShapeCount"] != 5
        or type(fixture["matchedShapeCount"]) is not int
        or not 0 <= fixture["matchedShapeCount"] <= 5
        or type(fixture["falseExactClaimCount"]) is not int
        or fixture["falseExactClaimCount"] < 0
    ):
        raise PilotError("WARM_ATTESTATION_INVALID")
    samples = attestation["samplesNanos"]
    digests = attestation["stdoutDigests"]
    if (
        not isinstance(samples, list)
        or len(samples) != 30
        or any(type(value) is not int or value <= 0 for value in samples)
        or not isinstance(digests, list)
        or len(digests) != 30
        or any(not isinstance(value, str) or SHA256.fullmatch(value) is None for value in digests)
        or set(digests) != {priming["stdoutDigest"]}
    ):
        raise PilotError("WARM_ATTESTATION_INVALID")
    audit = closed(
        attestation["audit"],
        {
            "adapter", "networkCanaryDenied", "processCanaryDenied",
            "cacheCanaryDenied", "writeCanaryDenied", "measuredDenialCount", "prohibitedProcessCount",
            "cacheAccessCount", "cacheRootCanaryCount", "cacheSentinelDigest",
        },
        "WARM_ATTESTATION_INVALID",
    )
    if (
        audit["adapter"] != "MACOS_SEATBELT_V1"
        or audit["networkCanaryDenied"] is not True
        or audit["processCanaryDenied"] is not True
        or audit["cacheCanaryDenied"] is not True
        or audit["writeCanaryDenied"] is not True
        or any(type(audit[key]) is not int or audit[key] != 0 for key in {
            "measuredDenialCount", "prohibitedProcessCount", "cacheAccessCount"
        })
        or audit["cacheRootCanaryCount"] != len(effective_cache_roots)
        or audit["cacheSentinelDigest"] != expected_cache_sentinel_digest
    ):
        raise PilotError("WARM_ATTESTATION_INVALID")
    private = {
        "schema": PRIVATE_WARM_SCHEMA,
        "status": "COMPLETE",
        "authorityDigest": authority["authorityDigest"],
        "protocolDigest": authority["protocolDigest"],
        "attestationDigest": declared,
        "fixture": fixture,
        "runCount": 30,
        "p95Rank": 29,
        "samplesNanos": samples,
        "p95Nanos": sorted(samples)[28],
        "stdoutByteIdentical": len(set(digests)) == 1,
        "networkDenied": True,
        "prohibitedProcessCount": 0,
        "cacheAccessCount": 0,
    }
    _create_private_once(output, private, "PRIVATE_WARM_CREATE_FAILED")
    return {
        "schema": PRIVATE_WARM_SCHEMA,
        "status": "COMPLETE",
        "runCount": 30,
        "p95Nanos": private["p95Nanos"],
    }


BROKER_METRIC_FIELDS = {
    "queryTerms", "returnedFacts", "selectedFiles", "sourceWindows",
    "openedSourceBytes", "openedSourceFiles", "agentVisibleEvidenceBytes",
    "contextCreates", "contextExpansions", "maxSemanticCommandMillis",
    "semanticContextCommands", "semanticCallablesCommands",
    "semanticImpactCommands", "capabilityViolations", "budgetRefusals",
}
BROKER_OPERATIONS = {
    "capability", "tree", "show", "search", "read", "semantic-context",
    "semantic-callables", "semantic-impact", "INVALID",
}


def _private_selected_file(value: Any, aliases: set[str]) -> tuple[str, str, str]:
    row = closed(
        value, {"serviceAlias", "relativeFile", "blobOid"},
        "INVALID_BROKER_PROVENANCE",
    )
    if row["serviceAlias"] not in aliases:
        raise PilotError("INVALID_BROKER_PROVENANCE")
    descriptor_gate.safe_relative_kotlin_file(row["relativeFile"])
    if (
        not isinstance(row["blobOid"], str)
        or descriptor_gate.GIT_BLOB_OID.fullmatch(row["blobOid"]) is None
    ):
        raise PilotError("INVALID_BROKER_PROVENANCE")
    return row["serviceAlias"], row["relativeFile"], row["blobOid"]


def _verify_broker_provenance(
    value: Any, task: dict[str, Any], arm: str
) -> tuple[dict[str, int], set[tuple[str, str, int, int]]]:
    audit = closed(
        value,
        {
            "schema", "taskId", "arm", "metrics", "queryTerms",
            "orderedToolLedger", "selectedFiles", "sourceWindows",
            "semanticTimingLedger", "violationLedger", "processGroupLedger",
        },
        "INVALID_BROKER_PROVENANCE",
    )
    if (
        audit["schema"] != "codeclew-kotlin-pilot-broker-audit/1.0"
        or audit["taskId"] != task["taskId"]
        or audit["arm"] != arm
    ):
        raise PilotError("INVALID_BROKER_PROVENANCE")
    metrics = closed(
        audit["metrics"], BROKER_METRIC_FIELDS, "INVALID_BROKER_PROVENANCE"
    )
    if any(type(item) is not int or item < 0 for item in metrics.values()):
        raise PilotError("INVALID_BROKER_PROVENANCE")
    terms = audit["queryTerms"]
    if (
        not isinstance(terms, list)
        or terms != sorted(set(terms))
        or any(
            not isinstance(term, str) or broker.SAFE_TERM.fullmatch(term) is None
            for term in terms
        )
    ):
        raise PilotError("INVALID_BROKER_PROVENANCE")
    aliases = {task["provider"], task["consumer"]}
    selected_rows = audit["selectedFiles"]
    if not isinstance(selected_rows, list):
        raise PilotError("INVALID_BROKER_PROVENANCE")
    selected = [_private_selected_file(row, aliases) for row in selected_rows]
    if selected != sorted(set(selected)):
        raise PilotError("INVALID_BROKER_PROVENANCE")

    source_rows = audit["sourceWindows"]
    if not isinstance(source_rows, list):
        raise PilotError("INVALID_BROKER_PROVENANCE")
    opened_files: set[tuple[str, str, str]] = set()
    opened_windows: set[tuple[str, str, int, int]] = set()
    charged_bytes = 0
    for index, item in enumerate(source_rows, 1):
        row = closed(
            item,
            {
                "order", "operation", "serviceAlias", "relativeFile", "blobOid",
                "startByte", "endByte", "chargedBytes", "visibleBytes",
                "sourceBytesDigest",
            },
            "INVALID_BROKER_PROVENANCE",
        )
        selected_identity = _private_selected_file(
            {
                "serviceAlias": row["serviceAlias"],
                "relativeFile": row["relativeFile"],
                "blobOid": row["blobOid"],
            },
            aliases,
        )
        if (
            row["order"] != index
            or row["operation"] not in BROKER_OPERATIONS - {"INVALID"}
            or type(row["startByte"]) is not int
            or type(row["endByte"]) is not int
            or not 0 <= row["startByte"] < row["endByte"]
            or row["chargedBytes"] != row["endByte"] - row["startByte"]
            or type(row["visibleBytes"]) is not int
            or not 0 <= row["visibleBytes"] <= row["chargedBytes"]
            or not isinstance(row["sourceBytesDigest"], str)
            or SHA256.fullmatch(row["sourceBytesDigest"]) is None
            or selected_identity not in set(selected)
        ):
            raise PilotError("INVALID_BROKER_PROVENANCE")
        opened_files.add(selected_identity)
        opened_windows.add(
            (
                row["serviceAlias"], row["blobOid"],
                row["startByte"], row["endByte"],
            )
        )
        charged_bytes += row["chargedBytes"]

    timing_rows = audit["semanticTimingLedger"]
    if not isinstance(timing_rows, list):
        raise PilotError("INVALID_BROKER_PROVENANCE")
    successful_semantic = {
        "semantic-context": 0,
        "semantic-callables": 0,
        "semantic-impact": 0,
    }
    successful_millis: list[int] = []
    for index, item in enumerate(timing_rows, 1):
        row = closed(
            item,
            {"order", "operation", "elapsedMillis", "status", "refusalCode"},
            "INVALID_BROKER_PROVENANCE",
        )
        if (
            row["order"] != index
            or row["operation"] not in successful_semantic
            or type(row["elapsedMillis"]) is not int
            or row["elapsedMillis"] <= 0
            or row["status"] not in {"OK", "REFUSED"}
            or (
                row["status"] == "OK"
                and row["refusalCode"] is not None
            )
            or (
                row["status"] == "REFUSED"
                and (
                    not isinstance(row["refusalCode"], str)
                    or re.fullmatch(r"[A-Z][A-Z0-9_]{0,63}", row["refusalCode"])
                    is None
                )
            )
        ):
            raise PilotError("INVALID_BROKER_PROVENANCE")
        if row["status"] == "OK":
            successful_semantic[row["operation"]] += 1
            successful_millis.append(row["elapsedMillis"])

    violation_rows = audit["violationLedger"]
    if not isinstance(violation_rows, list):
        raise PilotError("INVALID_BROKER_PROVENANCE")
    for index, item in enumerate(violation_rows, 1):
        row = closed(
            item, {"order", "operation", "code", "source"},
            "INVALID_BROKER_PROVENANCE",
        )
        if (
            row["order"] != index
            or not isinstance(row["operation"], str)
            or len(row["operation"].encode("utf-8")) > 128
            or not isinstance(row["code"], str)
            or re.fullmatch(r"[A-Z][A-Z0-9_]{0,63}", row["code"]) is None
            or not isinstance(row["source"], str)
            or re.fullmatch(r"[A-Z][A-Z0-9_]{0,63}", row["source"]) is None
        ):
            raise PilotError("INVALID_BROKER_PROVENANCE")

    process_rows = audit["processGroupLedger"]
    if not isinstance(process_rows, list):
        raise PilotError("INVALID_BROKER_PROVENANCE")
    for index, item in enumerate(process_rows, 1):
        row = closed(
            item,
            {
                "order", "kind", "purpose", "status", "returnCode", "timedOut",
                "interrupted", "stdoutOverflow", "groupLiveAtFinalize",
                "residualAfterCleanup",
            },
            "INVALID_BROKER_PROVENANCE",
        )
        if (
            row["order"] != index
            or row["kind"] not in {"GIT", "SEMANTIC"}
            or not isinstance(row["purpose"], str)
            or re.fullmatch(r"[A-Z][A-Z0-9_]{0,127}", row["purpose"]) is None
            or row["status"] not in {
                "OK", "INTERRUPTED", "TIMEOUT", "OUTPUT_LIMIT", "FAILED",
                "RESIDUAL",
            }
            or (row["returnCode"] is not None and type(row["returnCode"]) is not int)
            or any(
                type(row[key]) is not bool
                for key in {
                    "timedOut", "interrupted", "groupLiveAtFinalize",
                    "residualAfterCleanup", "stdoutOverflow",
                }
            )
            or (row["stdoutOverflow"] != (row["status"] == "OUTPUT_LIMIT"))
            or (
                row["status"] == "OK"
                and any(
                    row[key]
                    for key in {
                        "timedOut", "interrupted", "stdoutOverflow",
                        "groupLiveAtFinalize", "residualAfterCleanup",
                    }
                )
            )
        ):
            raise PilotError("INVALID_BROKER_PROVENANCE")
        # A leader that returned while its process group remained live is an
        # ambiguous capability boundary, even when the bounded teardown later
        # succeeded.  Such an arm cannot be downgraded to a scored refusal.
        if row["status"] == "RESIDUAL" or row["residualAfterCleanup"]:
            raise PilotError("BROKER_PROCESS_GROUP_RESIDUAL")

    tool_rows = audit["orderedToolLedger"]
    if not isinstance(tool_rows, list):
        raise PilotError("INVALID_BROKER_PROVENANCE")
    summed_metrics = {key: 0 for key in BROKER_METRIC_FIELDS}
    added_terms: list[str] = []
    added_selected: list[tuple[str, str, str]] = []
    linked_windows: list[int] = []
    linked_timings: list[int] = []
    linked_violations: list[int] = []
    for index, item in enumerate(tool_rows, 1):
        row = closed(
            item,
            {
                "order", "operation", "requestDigest", "responseDigest", "status",
                "refusalCode", "accountingDelta",
            },
            "INVALID_BROKER_PROVENANCE",
        )
        delta = closed(
            row["accountingDelta"],
            {
                "metrics", "queryTerms", "selectedFiles", "sourceWindowOrders",
                "semanticTimingOrders", "violationOrders",
            },
            "INVALID_BROKER_PROVENANCE",
        )
        delta_metrics = closed(
            delta["metrics"], BROKER_METRIC_FIELDS, "INVALID_BROKER_PROVENANCE"
        )
        if (
            row["order"] != index
            or row["operation"] not in BROKER_OPERATIONS
            or any(
                not isinstance(row[key], str) or SHA256.fullmatch(row[key]) is None
                for key in {"requestDigest", "responseDigest"}
            )
            or row["status"] not in {"OK", "REFUSED"}
            or (row["status"] == "OK" and row["refusalCode"] is not None)
            or (
                row["status"] == "REFUSED"
                and (
                    not isinstance(row["refusalCode"], str)
                    or re.fullmatch(r"[A-Z][A-Z0-9_]{0,63}", row["refusalCode"])
                    is None
                )
            )
            or any(type(number) is not int or number < 0 for number in delta_metrics.values())
            or not isinstance(delta["queryTerms"], list)
            or delta["queryTerms"] != sorted(set(delta["queryTerms"]))
            or any(term not in terms for term in delta["queryTerms"])
            or not isinstance(delta["selectedFiles"], list)
        ):
            raise PilotError("INVALID_BROKER_PROVENANCE")
        delta_selected = [
            _private_selected_file(item, aliases) for item in delta["selectedFiles"]
        ]
        if delta_selected != sorted(set(delta_selected)) or any(
            item not in selected for item in delta_selected
        ):
            raise PilotError("INVALID_BROKER_PROVENANCE")
        for key in {"sourceWindowOrders", "semanticTimingOrders", "violationOrders"}:
            orders = delta[key]
            if (
                not isinstance(orders, list)
                or any(type(order) is not int or order <= 0 for order in orders)
                or orders != sorted(set(orders))
            ):
                raise PilotError("INVALID_BROKER_PROVENANCE")
        for key in BROKER_METRIC_FIELDS:
            summed_metrics[key] += delta_metrics[key]
        added_terms.extend(delta["queryTerms"])
        added_selected.extend(delta_selected)
        linked_windows.extend(delta["sourceWindowOrders"])
        linked_timings.extend(delta["semanticTimingOrders"])
        linked_violations.extend(delta["violationOrders"])

    if (
        summed_metrics != metrics
        or sorted(added_terms) != terms
        or len(added_terms) != len(set(added_terms))
        or sorted(added_selected) != selected
        or len(added_selected) != len(set(added_selected))
        or linked_windows != list(range(1, len(source_rows) + 1))
        or linked_timings != list(range(1, len(timing_rows) + 1))
        or linked_violations != list(range(1, len(violation_rows) + 1))
        or metrics["queryTerms"] != len(terms)
        or metrics["selectedFiles"] != len(selected)
        or metrics["sourceWindows"] != len(source_rows)
        or metrics["openedSourceBytes"] != charged_bytes
        or metrics["openedSourceFiles"] != len(opened_files)
        or metrics["capabilityViolations"] != len(violation_rows)
        or metrics["semanticContextCommands"]
        != successful_semantic["semantic-context"]
        or metrics["semanticCallablesCommands"]
        != successful_semantic["semantic-callables"]
        or metrics["semanticImpactCommands"]
        != successful_semantic["semantic-impact"]
        or metrics["contextCreates"] != successful_semantic["semantic-context"]
        or metrics["contextExpansions"] != 0
        or metrics["maxSemanticCommandMillis"]
        != (max(successful_millis) if successful_millis else 0)
        or (arm == "DEFAULT" and timing_rows)
    ):
        raise PilotError("INVALID_BROKER_PROVENANCE")
    return dict(metrics), opened_windows


def _verify_private_run(
    value: dict[str, Any],
    authority: dict[str, Any],
    oracle: dict[str, Any],
    implementation_review_digest: str | None = None,
) -> list[dict[str, Any]]:
    unsigned_run = dict(value)
    declared_run_digest = unsigned_run.pop("runDigest", None)
    closed(
        unsigned_run,
        {
            "schema", "status", "authorityDigest", "protocolDigest",
            "ownerPid", "ownerToken",
            "implementationReviewManifestDigest", "completedArmCount",
            "failureCode", "activeArm", "arms",
        },
        "INVALID_PRIVATE_RUN",
    )
    if (
        not isinstance(declared_run_digest, str)
        or SHA256.fullmatch(declared_run_digest) is None
        or declared_run_digest != authority_digest(unsigned_run)
        or value["schema"] != PRIVATE_RUN_SCHEMA
        or value["status"] != "COMPLETE"
        or value["authorityDigest"] != authority["authorityDigest"]
        or value["protocolDigest"] != authority["protocolDigest"]
        or type(value["ownerPid"]) is not int
        or value["ownerPid"] <= 0
        or not isinstance(value["ownerToken"], str)
        or re.fullmatch(r"[0-9a-f]{64}", value["ownerToken"]) is None
        or not isinstance(value["implementationReviewManifestDigest"], str)
        or SHA256.fullmatch(value["implementationReviewManifestDigest"]) is None
        or (
            implementation_review_digest is not None
            and value["implementationReviewManifestDigest"]
            != implementation_review_digest
        )
        or value["completedArmCount"] != 20
        or value["failureCode"] is not None
        or value["activeArm"] is not None
        or not isinstance(value["arms"], list)
        or len(value["arms"]) != 20
    ):
        raise PilotError("INVALID_PRIVATE_RUN")
    expected = [
        (order["taskId"], arm)
        for order in authority["armOrder"]
        for arm in order["arms"]
    ]
    observed = [(row.get("taskId"), row.get("arm")) for row in value["arms"] if isinstance(row, dict)]
    if observed != expected:
        raise PilotError("INVALID_PRIVATE_RUN")
    tasks = {row["taskId"]: row for row in authority["tasks"]}
    oracle_tasks = {row["taskId"]: row for row in oracle["tasks"]}
    private_arm_fields = {
        "taskId", "pairId", "arm", "promptDigest", "status", "failureCode",
        "failureClass", "answer", "answerDigest", "jsonlDigest", "usage", "elapsedMillis",
        "modelReturnCode", "answerBytes", "scratchLocator", "codexToolLedger", "brokerProvenance",
        "score", "armDigest",
    }
    usage_fields = {
        "input_tokens", "cached_input_tokens", "cache_write_input_tokens",
        "output_tokens", "reasoning_output_tokens",
    }
    verified: list[dict[str, Any]] = []
    scratch_paths: set[str] = set()
    for row in value["arms"]:
        if not isinstance(row, dict) or set(row) != private_arm_fields:
            raise PilotError("INVALID_PRIVATE_RUN")
        arm_unsigned = dict(row)
        declared_arm_digest = arm_unsigned.pop("armDigest")
        task = tasks[row["taskId"]]
        if (
            not isinstance(declared_arm_digest, str)
            or SHA256.fullmatch(declared_arm_digest) is None
            or declared_arm_digest != authority_digest(arm_unsigned)
            or row["pairId"] != task["pairId"]
            or row["promptDigest"] != task["promptDigest"]
            or row["status"] not in {"COMPLETE", "ARM_FAILURE"}
            or not isinstance(row["usage"], dict)
            or set(row["usage"]) != usage_fields
            or any(type(item) is not int or item < 0 for item in row["usage"].values())
            or row["usage"]["cached_input_tokens"] > row["usage"]["input_tokens"]
            or not isinstance(row["jsonlDigest"], str)
            or SHA256.fullmatch(row["jsonlDigest"]) is None
            or type(row["modelReturnCode"]) is not int
            or type(row["elapsedMillis"]) is not int
            or row["elapsedMillis"] <= 0
            or type(row["answerBytes"]) is not int
            or not 0 <= row["answerBytes"] <= BUDGETS["answerBytes"]
        ):
            raise PilotError("INVALID_PRIVATE_RUN")
        locator = closed(
            row["scratchLocator"],
            {"taskId", "arm", "path", "device", "inode"},
            "INVALID_PRIVATE_RUN",
        )
        locator_path = (
            Path(locator["path"])
            if isinstance(locator["path"], str)
            else Path(".")
        )
        if (
            locator["taskId"] != row["taskId"]
            or locator["arm"] != row["arm"]
            or not locator_path.is_absolute()
            or locator["path"] != os.fspath(locator_path.resolve(strict=False))
            or not locator_path.name.startswith("codeclew-s4k-arm-")
            or type(locator["device"]) is not int
            or locator["device"] < 0
            or type(locator["inode"]) is not int
            or locator["inode"] <= 0
            or locator["path"] in scratch_paths
        ):
            raise PilotError("INVALID_PRIVATE_RUN")
        scratch_paths.add(locator["path"])
        tool_ledger = row["codexToolLedger"]
        if not isinstance(tool_ledger, list) or len(tool_ledger) > BUDGETS["toolStarts"]:
            raise PilotError("INVALID_PRIVATE_RUN")
        for index, item in enumerate(tool_ledger, 1):
            if (
                not isinstance(item, dict)
                or set(item) != {"sequence", "operation", "commandDigest"}
                or item["sequence"] != index
                or item["operation"] not in {
                    "capability", "tree", "show", "search", "read",
                    "semantic-context", "semantic-callables", "semantic-impact",
                }
                or not isinstance(item["commandDigest"], str)
                or SHA256.fullmatch(item["commandDigest"]) is None
            ):
                raise PilotError("INVALID_PRIVATE_RUN")
        broker_metrics, opened_windows = _verify_broker_provenance(
            row["brokerProvenance"], task, row["arm"]
        )
        violation_codes = {
            item["code"]
            for item in row["brokerProvenance"]["violationLedger"]
        }
        if violation_codes & BROKER_PROTOCOL_VIOLATIONS:
            raise PilotError("BROKER_PROTOCOL_VIOLATION")
        if [
            (item["sequence"], item["operation"]) for item in tool_ledger
        ] != [
            (item["order"], item["operation"])
            for item in row["brokerProvenance"]["orderedToolLedger"]
        ]:
            raise PilotError("BROKER_TOOL_LEDGER_MISMATCH")
        oracle_task = oracle_tasks[row["taskId"]]
        source_side_count = _approved_source_side_count(opened_windows, oracle_task)
        runtime = _runtime_metrics(
            row["elapsedMillis"],
            len(tool_ledger),
            row["usage"],
            broker_metrics,
            row["answerBytes"],
            source_side_count,
        )
        expected_failure: tuple[str, str] | None = None
        if broker_metrics["budgetRefusals"]:
            expected_failure = ("RESOURCE_LIMIT", "BROKER_RESOURCE_BUDGET")
        elif runtime["noncachedInputTokens"] > BUDGETS["noncachedInputTokens"]:
            expected_failure = (
                "RESOURCE_LIMIT", "NONCACHED_TOKEN_BUDGET_EXCEEDED"
            )
        elif broker_metrics["capabilityViolations"]:
            expected_failure = (
                "PRODUCT_REFUSAL",
                next(
                    (
                        item["code"]
                        for item in row["brokerProvenance"]["violationLedger"]
                    ),
                    "BROKER_CAPABILITY_REFUSED",
                ),
            )
        if row["status"] == "COMPLETE":
            if (
                expected_failure is not None
                or row["modelReturnCode"] != 0
                or row["failureCode"] is not None
                or row["failureClass"] is not None
                or not isinstance(row["answer"], dict)
                or not isinstance(row["answerDigest"], str)
                or SHA256.fullmatch(row["answerDigest"]) is None
                or row["answerDigest"] != authority_digest(row["answer"])
            ):
                raise PilotError("INVALID_PRIVATE_RUN")
            answer = validate_answer(row["answer"], task, row["arm"])
            recomputed = score_answer(
                answer,
                task,
                oracle_task,
                runtime,
                opened_windows,
            )
        elif (
            row["failureClass"] not in ARM_FAILURE_CLASSES
            or not isinstance(row["failureCode"], str)
            or re.fullmatch(r"[A-Z][A-Z0-9_]{0,63}", row["failureCode"]) is None
        ):
            raise PilotError("INVALID_PRIVATE_RUN")
        else:
            if expected_failure is not None:
                if (row["failureClass"], row["failureCode"]) != expected_failure:
                    raise PilotError("INVALID_PRIVATE_RUN")
            elif row["failureClass"] == "MODEL_EXIT":
                if row["modelReturnCode"] == 0 or row["failureCode"] != "CODEX_COMMAND_FAILED":
                    raise PilotError("INVALID_PRIVATE_RUN")
            elif row["failureClass"] == "MODEL_OUTPUT":
                if row["modelReturnCode"] != 0:
                    raise PilotError("INVALID_PRIVATE_RUN")
            else:
                raise PilotError("INVALID_PRIVATE_RUN")
            if row["answer"] is None:
                if row["answerDigest"] is not None:
                    raise PilotError("INVALID_PRIVATE_RUN")
            elif (
                not isinstance(row["answer"], dict)
                or not isinstance(row["answerDigest"], str)
                or SHA256.fullmatch(row["answerDigest"]) is None
                or row["answerDigest"] != authority_digest(row["answer"])
            ):
                raise PilotError("INVALID_PRIVATE_RUN")
            else:
                validate_answer(row["answer"], task, row["arm"])
            recomputed = _failed_score(runtime, row["arm"])
            recomputed["manualCategoryExpectedCount"] = len(
                oracle_task["manualVerification"]
            )
        if row["score"] != recomputed:
            raise PilotError("PRIVATE_SCORE_MISMATCH")
        try:
            public_verifier._verify_arm(
                recomputed,
                f'{row["taskId"]}/{row["arm"]}',
                len(task["manualVerification"]),
                arm=row["arm"],
            )
        except public_verifier.EvidenceError as error:
            raise PilotError("INVALID_PRIVATE_RUN") from error
        verified.append({**row, "recomputedScore": recomputed})
    return verified


def _verify_private_warm(value: dict[str, Any], authority: dict[str, Any]) -> dict[str, Any]:
    expected = {
        "schema", "status", "authorityDigest", "protocolDigest", "attestationDigest",
        "fixture", "runCount", "p95Rank", "samplesNanos", "p95Nanos",
        "stdoutByteIdentical", "networkDenied", "prohibitedProcessCount", "cacheAccessCount",
    }
    closed(value, expected, "INVALID_PRIVATE_WARM_RUN")
    fixture = value.get("fixture")
    if (
        value["schema"] != PRIVATE_WARM_SCHEMA
        or value["status"] != "COMPLETE"
        or value["authorityDigest"] != authority["authorityDigest"]
        or value["protocolDigest"] != authority["protocolDigest"]
        or not isinstance(value["attestationDigest"], str)
        or SHA256.fullmatch(value["attestationDigest"]) is None
        or value["runCount"] != 30
        or value["p95Rank"] != 29
        or not isinstance(value["samplesNanos"], list)
        or len(value["samplesNanos"]) != 30
        or any(type(sample) is not int or sample <= 0 for sample in value["samplesNanos"])
        or value["p95Nanos"] != sorted(value["samplesNanos"])[28]
        or not isinstance(fixture, dict)
        or set(fixture) != {"expectedShapeCount", "matchedShapeCount", "falseExactClaimCount"}
        or fixture["expectedShapeCount"] != 5
        or type(fixture["matchedShapeCount"]) is not int
        or not 0 <= fixture["matchedShapeCount"] <= 5
        or type(fixture["falseExactClaimCount"]) is not int
        or fixture["falseExactClaimCount"] < 0
        or type(value["stdoutByteIdentical"]) is not bool
        or value["networkDenied"] is not True
        or type(value["prohibitedProcessCount"]) is not int
        or value["prohibitedProcessCount"] != 0
        or type(value["cacheAccessCount"]) is not int
        or value["cacheAccessCount"] != 0
    ):
        raise PilotError("INVALID_PRIVATE_WARM_RUN")
    return value


def _aggregate(rows: list[dict[str, Any]]) -> dict[str, int]:
    return {
        "taskPassCount": sum(row["result"] == "PASS" for row in rows),
        "declaredMemberHitCount": sum(row["declaredMemberHits"] for row in rows),
        "top10RelevantFileHitCount": sum(row["top10RelevantFileHits"] for row in rows),
        "descriptorSlotHitCount": sum(row["descriptorSlotHits"] for row in rows),
        "manualCategoryHitCount": sum(row["manualCategoryHits"] for row in rows),
        "falseExactClaimCount": sum(row["falseExactClaimCount"] for row in rows),
        "totalElapsedMillis": sum(row["elapsedMillis"] for row in rows),
        "totalOpenedSourceBytes": sum(row["openedSourceBytes"] for row in rows),
        "totalOpenedSourceFiles": sum(row["openedSourceFiles"] for row in rows),
        "totalToolStarts": sum(row["toolStarts"] for row in rows),
        "totalNoncachedInputTokens": sum(row["noncachedInputTokens"] for row in rows),
    }


def _review_findings(value: Any, code: str) -> list[dict[str, str]]:
    if not isinstance(value, list) or len(value) > 256:
        raise PilotError(code)
    rows: list[dict[str, str]] = []
    identities: set[tuple[str, str]] = set()
    for item in value:
        row = closed(item, {"severity", "code"}, code)
        identity = (row["severity"], row["code"])
        if (
            row["severity"] not in {"P0", "P1", "P2", "P3"}
            or not isinstance(row["code"], str)
            or re.fullmatch(r"[A-Z][A-Z0-9_]{0,127}", row["code"]) is None
            or identity in identities
        ):
            raise PilotError(code)
        identities.add(identity)
        rows.append(row)
    if rows != sorted(rows, key=lambda row: (row["severity"], row["code"])):
        raise PilotError(code)
    if any(row["severity"] in {"P0", "P1"} for row in rows):
        raise PilotError(code)
    return rows


def _implementation_review(
    path: Path, authority: dict[str, Any]
) -> tuple[Path, str]:
    resolved, value, _ = checked_json(path, "IMPLEMENTATION_REVIEW", 256 * 1024)
    unsigned = dict(value)
    declared = unsigned.pop("authorityDigest", None)
    inputs = authority["inputs"]
    expected = {
        "schema": IMPLEMENTATION_REVIEW_SCHEMA,
        "protocolDigest": authority["protocolDigest"],
        "runnerDigest": inputs["runnerDigest"],
        "brokerDigest": inputs["brokerDigest"],
        "publicVerifierDigest": inputs["publicVerifierDigest"],
        "localModuleManifest": inputs["localModuleManifest"],
        "localModuleManifestDigest": inputs["localModuleManifestDigest"],
        "answerSchemaDigest": inputs["answerSchemaDigest"],
        "warmAuditAdapterDigest": inputs["warmAuditAdapterDigest"],
        "shapeOracleBuilderDigest": inputs["shapeOracleBuilderDigest"],
        "verdict": "PASS",
        "findings": value.get("findings"),
    }
    if set(unsigned) != set(expected) or unsigned != expected:
        raise PilotError("IMPLEMENTATION_REVIEW_INVALID")
    _review_findings(value["findings"], "IMPLEMENTATION_REVIEW_INVALID")
    if (
        not isinstance(declared, str)
        or SHA256.fullmatch(declared) is None
        or declared != authority_digest(unsigned)
    ):
        raise PilotError("IMPLEMENTATION_REVIEW_INVALID")
    return resolved, declared


def _value_review(
    path: Path,
    authority: dict[str, Any],
    run_digest: str,
    warm_attestation_digest: str,
    draft_metrics_digest: str,
) -> tuple[Path, str]:
    resolved, value, _ = checked_json(path, "VALUE_REVIEW", 256 * 1024)
    unsigned = dict(value)
    declared = unsigned.pop("authorityDigest", None)
    expected = {
        "schema": VALUE_REVIEW_SCHEMA,
        "pilotAuthorityDigest": authority["authorityDigest"],
        "runDigest": run_digest,
        "warmAttestationDigest": warm_attestation_digest,
        "draftMetricsDigest": draft_metrics_digest,
        "benchmarkDigest": authority["inputs"]["benchmarkDigest"],
        "verdict": "PASS",
        "findings": value.get("findings"),
    }
    if set(unsigned) != set(expected) or unsigned != expected:
        raise PilotError("VALUE_REVIEW_INVALID")
    _review_findings(value["findings"], "VALUE_REVIEW_INVALID")
    if (
        not isinstance(declared, str)
        or SHA256.fullmatch(declared) is None
        or declared != authority_digest(unsigned)
    ):
        raise PilotError("VALUE_REVIEW_INVALID")
    return resolved, declared


def _project_draft(
    authority: dict[str, Any],
    oracle_value: dict[str, Any],
    run_value: dict[str, Any],
    warm_value: dict[str, Any],
    implementation_review_digest: str,
) -> tuple[dict[str, Any], list[dict[str, Any]], list[dict[str, Any]]]:
    oracle = verify_oracle(oracle_value, authority)
    arms = _verify_private_run(
        run_value, authority, oracle, implementation_review_digest
    )
    warm_result = _verify_private_warm(warm_value, authority)
    by_key = {(row["taskId"], row["arm"]): row["recomputedScore"] for row in arms}
    task_rows: list[dict[str, Any]] = []
    defaults: list[dict[str, Any]] = []
    codeclew: list[dict[str, Any]] = []
    for index, (task, order) in enumerate(zip(authority["tasks"], authority["armOrder"], strict=True)):
        default = by_key[(task["taskId"], "DEFAULT")]
        clew = by_key[(task["taskId"], "CODECLEW")]
        expected_manual = 8 if index < 8 else 5
        if default["manualCategoryExpectedCount"] != expected_manual or clew["manualCategoryExpectedCount"] != expected_manual:
            raise PilotError("INVALID_PRIVATE_RUN")
        defaults.append(default)
        codeclew.append(clew)
        task_rows.append(
            {
                "taskId": task["taskId"],
                "pairId": task["pairId"],
                "armOrder": order["arms"],
                "default": default,
                "codeclew": clew,
            }
        )
    fixture_source = warm_result["fixture"]
    fixture = {
        **fixture_source,
        "result": "PASS"
        if fixture_source["matchedShapeCount"] == 5
        and fixture_source["falseExactClaimCount"] == 0
        else "FAIL",
    }
    comparison = {
            "taskDenominator": 10,
            "declaredMemberDenominator": 20,
            "top10RelevantFileDenominator": 20,
            "descriptorSlotDenominator": 20,
            "manualCategoryDenominator": 74,
            "default": _aggregate(defaults),
            "codeclew": _aggregate(codeclew),
            "taskResults": task_rows,
        }
    warm_audit = {
            key: warm_result[key]
            for key in {
                "runCount", "p95Rank", "samplesNanos", "p95Nanos",
                "stdoutByteIdentical", "networkDenied", "prohibitedProcessCount", "cacheAccessCount",
            }
        }
    metrics = {"fixture": fixture, "comparison": comparison, "warmAudit": warm_audit}
    unsigned = {
        "schema": PRIVATE_DRAFT_SCHEMA,
        "status": "DRAFT",
        "pilotAuthorityDigest": authority["authorityDigest"],
        "protocolDigest": authority["protocolDigest"],
        "runDigest": run_value["runDigest"],
        "warmAttestationDigest": warm_result["attestationDigest"],
        "implementationReviewManifestDigest": implementation_review_digest,
        "draftMetricsDigest": authority_digest(metrics),
        **metrics,
    }
    return {**unsigned, "draftDigest": authority_digest(unsigned)}, defaults, codeclew


def _verify_private_draft(value: Any, expected: dict[str, Any]) -> dict[str, Any]:
    fields = {
        "schema", "status", "pilotAuthorityDigest", "protocolDigest",
        "runDigest", "warmAttestationDigest", "draftMetricsDigest", "fixture",
        "implementationReviewManifestDigest", "comparison", "warmAudit",
        "draftDigest",
    }
    row = closed(value, fields, "PRIVATE_DRAFT_MISMATCH")
    unsigned = dict(row)
    declared = unsigned.pop("draftDigest")
    if (
        row["schema"] != PRIVATE_DRAFT_SCHEMA
        or row["status"] != "DRAFT"
        or not isinstance(declared, str)
        or SHA256.fullmatch(declared) is None
        or declared != authority_digest(unsigned)
        or row != expected
    ):
        raise PilotError("PRIVATE_DRAFT_MISMATCH")
    return row


def _project_private_inputs(
    args: argparse.Namespace,
) -> tuple[
    Path, dict[str, Any], Path, dict[str, Any], Path, dict[str, Any],
    Path, dict[str, Any], Path,
]:
    authority_path, authority_value, _ = private_json(
        args.private_authority, "PILOT_AUTHORITY"
    )
    authority = verify_authority(authority_value)
    oracle_path, oracle_value, _ = private_json(args.private_oracle, "PILOT_ORACLE")
    run_path, run_value, _ = private_json(args.private_run, "PILOT_RUN")
    _verify_execute_admission(
        Path(authority["experimentRoot"]["path"]), authority, run_path
    )
    warm_path, warm_value, _ = private_json(args.private_warm, "PILOT_WARM")
    resource_ledger = _cleanup_project_resources(authority_path, authority)
    return (
        authority_path, authority, oracle_path, oracle_value, run_path, run_value,
        warm_path, warm_value, resource_ledger,
    )


def _project_draft_phase(args: argparse.Namespace) -> dict[str, Any]:
    output = fresh_output_target(args.private_draft_output)
    (
        authority_path, authority, oracle_path, oracle_value, run_path, run_value,
        warm_path, warm_value, _resource_ledger,
    ) = _project_private_inputs(args)
    implementation_path, _, _ = checked_json(
        args.implementation_review_manifest, "IMPLEMENTATION_REVIEW", 256 * 1024
    )
    require_distinct_paths(
        [authority_path, oracle_path, run_path, warm_path, implementation_path],
        [output],
    )
    _, implementation_review_digest = _implementation_review(
        implementation_path, authority
    )
    draft, _, _ = _project_draft(
        authority,
        oracle_value,
        run_value,
        warm_value,
        implementation_review_digest,
    )
    _create_private_once(output, draft, "PRIVATE_DRAFT_CREATE_FAILED")
    return {
        "schema": PRIVATE_DRAFT_SCHEMA,
        "status": "DRAFT",
        "draftDigest": draft["draftDigest"],
        "draftMetricsDigest": draft["draftMetricsDigest"],
    }


def _project_publish_phase(args: argparse.Namespace) -> dict[str, Any]:
    output = fresh_output_target(args.checked_output)
    (
        authority_path, authority, oracle_path, oracle_value, run_path, run_value,
        warm_path, warm_value, resource_ledger,
    ) = _project_private_inputs(args)
    draft_path, supplied_draft, _ = private_json(
        args.private_draft, "PILOT_DRAFT"
    )
    implementation_path, _, _ = checked_json(
        args.implementation_review_manifest, "IMPLEMENTATION_REVIEW", 256 * 1024
    )
    value_path, _, _ = checked_json(
        args.value_review_manifest, "VALUE_REVIEW", 256 * 1024
    )
    require_distinct_paths(
        [
            authority_path, oracle_path, run_path, warm_path, draft_path,
            implementation_path, value_path,
        ],
        [output],
    )
    _, implementation_review_digest = _implementation_review(
        implementation_path, authority
    )
    draft, defaults, codeclew = _project_draft(
        authority,
        oracle_value,
        run_value,
        warm_value,
        implementation_review_digest,
    )
    _verify_private_draft(supplied_draft, draft)
    _, value_review_digest = _value_review(
        value_path,
        authority,
        draft["runDigest"],
        draft["warmAttestationDigest"],
        draft["draftMetricsDigest"],
    )
    evidence: dict[str, Any] = {
        "schema": public_verifier.SCHEMA,
        "status": "FAIL",
        "authority": {
            "privateCorpusDigest": authority["inputs"]["privateCorpusDigest"],
            "benchmarkDigest": authority["inputs"]["benchmarkDigest"],
            "protocolDigest": authority["protocolDigest"],
            "shapeOracleDigest": authority["inputs"]["shapeOracleDigest"],
            "modelConfigurationDigest": authority["modelConfigurationDigest"],
            "implementationReviewManifestDigest": implementation_review_digest,
            "valueReviewManifestDigest": value_review_digest,
        },
        "fixture": draft["fixture"],
        "comparison": draft["comparison"],
        "warmAudit": draft["warmAudit"],
        "verdict": {
            "benchmarkAggregatePass": False,
            "s4ComparativeGatePass": False,
            "implementationReview": "PASS",
            "valueReview": "PASS",
            "qualifiedAdoption": False,
            "qualification": "NOT_QUALIFIED",
            "failedGates": [],
        },
        "privacy": {
            "absolutePathCount": 0,
            "sourceBodyCount": 0,
            "privateIdentifierCount": 0,
            "credentialMatchCount": 0,
        },
    }
    failures, benchmark_pass, comparative_pass = public_verifier._expected_failed_gates(
        evidence, defaults, codeclew
    )
    qualified = benchmark_pass and comparative_pass
    evidence["status"] = "PASS" if qualified else "FAIL"
    evidence["verdict"].update(
        benchmarkAggregatePass=benchmark_pass,
        s4ComparativeGatePass=comparative_pass,
        qualifiedAdoption=qualified,
        qualification="LOCAL_KOTLIN_STRUCTURAL_NAVIGATION_PILOT" if qualified else "NOT_QUALIFIED",
        failedGates=failures,
    )
    try:
        public_verifier.verify_value(evidence)
    except public_verifier.EvidenceError as error:
        raise PilotError("PUBLIC_PROJECTION_INVALID") from error
    _create_private_once(output, evidence, "PUBLIC_PROJECTION_CREATE_FAILED", 0o644)
    _remove_pending(resource_ledger)
    return {
        "schema": public_verifier.SCHEMA,
        "status": evidence["status"],
        "qualifiedAdoption": qualified,
        "failedGates": failures,
    }


def project(args: argparse.Namespace) -> dict[str, Any]:
    experiment_root, _ = _experiment_root(args.experiment_root)
    private_paths = [
        args.private_authority,
        args.private_oracle,
        args.private_run,
        args.private_warm,
        args.implementation_review_manifest,
    ]
    if args.project_action == "draft":
        private_paths.append(args.private_draft_output)
    elif args.project_action == "publish":
        private_paths.extend(
            [args.private_draft, args.value_review_manifest]
        )
    _require_experiment_paths(experiment_root, private_paths)
    if args.project_action == "draft":
        return _project_draft_phase(args)
    if args.project_action == "publish":
        return _project_publish_phase(args)
    raise PilotError("INVALID_PROJECT_ACTION")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    phases = root.add_subparsers(dest="phase", required=True)
    prepare_parser = phases.add_parser("prepare")
    prepare_parser.add_argument("--experiment-root", type=Path, required=True)
    prepare_parser.add_argument("--private-corpus", type=Path, required=True)
    prepare_parser.add_argument("--private-benchmark", type=Path, required=True)
    prepare_parser.add_argument("--g1k-evidence", type=Path, required=True)
    prepare_parser.add_argument("--clew", type=Path, required=True)
    prepare_parser.add_argument("--codex", type=Path, default=Path("/opt/homebrew/bin/codex"))
    prepare_parser.add_argument("--model", required=True)
    prepare_parser.add_argument("--reasoning-effort", choices=sorted(SAFE_REASONING), default="high")
    prepare_parser.add_argument("--private-shape-oracle", type=Path, required=True)
    prepare_parser.add_argument("--private-shape-attestation", type=Path, required=True)
    prepare_parser.add_argument("--shape-oracle-review-manifest", type=Path, required=True)
    prepare_parser.add_argument("--shape-oracle-builder", type=Path, required=True)
    prepare_parser.add_argument("--warm-audit-runner", type=Path, required=True)
    prepare_parser.add_argument("--private-authority", type=Path, required=True)
    prepare_parser.add_argument("--private-oracle", type=Path, required=True)
    prepare_parser.add_argument("--timeout-seconds", type=int, default=1800)

    execute_parser = phases.add_parser("execute")
    execute_parser.add_argument("--experiment-root", type=Path, required=True)
    execute_parser.add_argument("--private-authority", type=Path, required=True)
    execute_parser.add_argument("--private-oracle", type=Path, required=True)
    execute_parser.add_argument(
        "--implementation-review-manifest", type=Path, required=True
    )
    execute_parser.add_argument("--private-output", type=Path, required=True)

    warm_parser = phases.add_parser("warm")
    warm_parser.add_argument("--experiment-root", type=Path, required=True)
    warm_parser.add_argument("--private-authority", type=Path, required=True)
    warm_parser.add_argument("--private-attestation", type=Path, required=True)
    warm_parser.add_argument("--private-output", type=Path, required=True)

    project_parser = phases.add_parser("project")
    project_actions = project_parser.add_subparsers(
        dest="project_action", required=True
    )

    def add_project_inputs(parser: argparse.ArgumentParser) -> None:
        parser.add_argument("--experiment-root", type=Path, required=True)
        parser.add_argument("--private-authority", type=Path, required=True)
        parser.add_argument("--private-oracle", type=Path, required=True)
        parser.add_argument("--private-run", type=Path, required=True)
        parser.add_argument("--private-warm", type=Path, required=True)

    draft_parser = project_actions.add_parser("draft")
    add_project_inputs(draft_parser)
    draft_parser.add_argument(
        "--implementation-review-manifest", type=Path, required=True
    )
    draft_parser.add_argument("--private-draft-output", type=Path, required=True)

    publish_parser = project_actions.add_parser("publish")
    add_project_inputs(publish_parser)
    publish_parser.add_argument("--private-draft", type=Path, required=True)
    publish_parser.add_argument(
        "--implementation-review-manifest", type=Path, required=True
    )
    publish_parser.add_argument("--value-review-manifest", type=Path, required=True)
    publish_parser.add_argument("--checked-output", type=Path, required=True)
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.phase == "prepare":
            if not 30 <= args.timeout_seconds <= 3600:
                raise PilotError("INVALID_PREPARATION_TIMEOUT")
            result = prepare(args)
        elif args.phase == "execute":
            result = execute(args)
        elif args.phase == "warm":
            result = warm(args)
        else:
            result = project(args)
    except PilotError as error:
        print(f"FAIL: {error.code}", file=sys.stderr)
        return 1
    except (descriptor_gate.GateError, g1k_verifier.EvidenceError, OSError):
        print(f"FAIL: S4K_{args.phase.upper()}_FAILED", file=sys.stderr)
        return 1
    except Exception:
        # Unexpected parser/type bugs must never emit a traceback containing
        # caller-owned locators.  The detailed exception remains deliberately
        # outside stdout/stderr authority.
        print(f"FAIL: S4K_{args.phase.upper()}_INTERNAL_FAILURE", file=sys.stderr)
        return 1
    try:
        print(canonical_bytes(result).decode("utf-8"))
    except Exception:
        print(f"FAIL: S4K_{args.phase.upper()}_INTERNAL_FAILURE", file=sys.stderr)
        return 1
    return 0 if result.get("status") not in {"FAIL", "INVALID_RUN"} else 1


if __name__ == "__main__":
    raise SystemExit(main())
