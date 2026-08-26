#!/usr/bin/env python3
"""Closed local broker used by the S4K Kotlin descriptor value pilot.

The model-facing process is only an atomic private-mailbox client.  Repository locators,
managed Codeclew identifiers, subprocesses, and accounting stay in the
runner-owned server.  Import this module from ``run_thread_kotlin_pilot.py`` to
create a :class:`BrokerSession`; execute it as ``pilot-tool`` for the client.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import secrets
import selectors
import signal
import stat
import subprocess
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


REQUEST_SCHEMA = "codeclew-kotlin-pilot-broker-request/1.0"
RESPONSE_SCHEMA = "codeclew-kotlin-pilot-broker-response/1.0"
SAFE_ALIAS = re.compile(r"^service-[0-9]{2}$")
SAFE_TERM = re.compile(r"^[A-Za-z_][A-Za-z0-9_./$-]{0,255}$")
SAFE_SUBJECT = re.compile(r"^[A-Za-z_][A-Za-z0-9_./$<>():;\[\]-]{0,4095}$")
GIT_OID = re.compile(r"^[0-9a-f]{40}(?:[0-9a-f]{24})?$")
MAX_FRAME_BYTES = 9 * 1024 * 1024
MAX_SOURCE_WINDOW_BYTES = 64 * 1024
MAX_GIT_OBJECT_BYTES = 8 * 1024 * 1024
MAX_TOOL_STDOUT_BYTES = 256 * 1024
IMPACT_BUDGETS = {
    "maxFindings": 4_096,
    "maxObligations": 4_096,
    "maxSourceWindows": 32,
    "maxSourceWindowBytes": 256 * 1024,
    "maxDerivedCasObjects": 64,
    "maxRetainedClosureBytes": 64 * 1024 * 1024,
    "maxStdoutBytes": 64 * 1024,
}
SAFE_REQUEST_NAME = re.compile(r"^([0-9]{20})-([0-9a-f]{32})\.json$")
CONTEXT_RESULT_SCHEMA = "codeclew-thread-context-result/1.0"
CALLABLE_RESULT_SCHEMA = "codeclew-thread-callables-result/1.0"
IMPACT_RESULT_SCHEMA = "codeclew-thread-impact-result/1.0"
MANAGED_VALUE_PREFIX = re.compile(
    r"^(?:thread(?::|-context:|-callables:|-impact:)|session:|context:|cas:)",
    re.IGNORECASE,
)
MODEL_PRIVATE_LOCATOR = re.compile(
    r"(?:file:(?://)?|(?:thread-context|thread-callables|thread-impact|thread|context|cas):sha256:[0-9a-f]{16,}|session:[A-Za-z0-9._:-]{16,})",
    re.IGNORECASE,
)
MODEL_PRIVATE_ABSOLUTE = re.compile(
    r"(?:^|[\s\"'`=:(])/(?:Users|home|private|tmp|var/folders|Volumes|workspace|workspaces|repo)(?:/|$)"
)
MODEL_EMBEDDED_ABSOLUTE = re.compile(
    r"(?:^|[\s\"'`=:(])/(?:[A-Za-z0-9._~+-]+/)+[A-Za-z0-9._~+-]+(?:[/\s\"'`,;:)\]}]|$)"
)
MODEL_SOURCE_TEXT_KEYS = {"content", "text", "sourceText", "snippet"}
SHA256_VALUE = re.compile(r"^sha256:[0-9a-f]{64}$")
MODEL_SAFE_DIGEST_KEYS = {"shapeDigest", "exactShapeDigest"}
MODEL_SAFE_ID_KEYS = {
    "taskId", "pairId", "fileId", "compilerCallableId", "compilerClassId",
    "targetCallableId",
}
MODEL_DROP_KEYS = {
    "threadId", "threadAuthorityDigest", "threadContextBindingDigest",
    "contextId", "contextAuthorityDigest", "contextDigest", "factSetId",
    "factSetAuthorityDigest", "impactId", "authorityDigest", "bindingDigest",
    "evidenceDigest", "evidenceRef", "queryIndexRef", "contentRef",
    "dependencyEvidenceRef", "factSetEvidenceRef", "consultedQueryShardRefs",
    "directCasClosure", "sessionId", "sessionAuthorityDigest", "repositoryKey",
    "repositoryNamespace", "targetRepositoryNamespace", "snapshotId",
    "repositorySnapshot", "generation", "generationId", "generationRef",
    "compilation", "compilations", "queryIndex", "payloadRef", "inputPayloadRef",
    "factShardRef", "inputFactKey", "factId", "findingId", "findingIds",
    "windowId", "termsDigest", "opaquePayloadDigest", "profileDigest",
}
_OMIT = object()


class BrokerError(RuntimeError):
    """A path-free refusal intended to be returned to the model."""

    def __init__(self, code: str):
        if re.fullmatch(r"[A-Z][A-Z0-9_]{0,63}", code) is None:
            code = "BROKER_INTERNAL_FAILURE"
        super().__init__(code)
        self.code = code


def canonical_bytes(value: Any) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")


def _sha256_digest(raw: bytes) -> str:
    return f"sha256:{hashlib.sha256(raw).hexdigest()}"


def _cas_object_digest(object_schema: str, raw: bytes) -> str:
    digest = hashlib.sha256()
    digest.update(b"codeclew-cas/v2\0")
    digest.update(object_schema.encode("utf-8"))
    digest.update(b"\0")
    digest.update(raw)
    return f"sha256:{digest.hexdigest()}"


def _closed_object(
    value: Any,
    required: set[str],
    optional: set[str] = frozenset(),
) -> dict[str, Any]:
    if not isinstance(value, dict) or not required.issubset(value) or not set(value).issubset(
        required | set(optional)
    ):
        raise BrokerError("SEMANTIC_OUTPUT_INVALID")
    return value


def _model_safe_value(
    value: Any,
    key: str | None = None,
    depth: int = 0,
    forbidden_locators: tuple[str, ...] = (),
) -> Any:
    if depth > 48:
        raise BrokerError("SEMANTIC_OUTPUT_INVALID")
    if isinstance(value, dict):
        projected: dict[str, Any] = {}
        for child_key, child in value.items():
            if not isinstance(child_key, str):
                raise BrokerError("SEMANTIC_OUTPUT_INVALID")
            # JSON keys are model-visible bytes too.  They never represent
            # source bodies, so locator-shaped keys are always a refusal.
            _refuse_model_locator_string(
                child_key, None, forbidden_locators, allow_source_text=False
            )
            if child_key in MODEL_DROP_KEYS:
                continue
            lowered = child_key.lower()
            if (
                (lowered.endswith("digest") and child_key not in MODEL_SAFE_DIGEST_KEYS)
                or (lowered.endswith("ref") and child_key != "relativeFile")
                or (
                    lowered.endswith("id")
                    and child_key not in MODEL_SAFE_ID_KEYS
                    and child_key not in {"symbolIdentity", "ownerIdentity"}
                )
            ):
                continue
            safe = _model_safe_value(
                child, child_key, depth + 1, forbidden_locators
            )
            if safe is not _OMIT:
                projected[child_key] = safe
        return projected
    if isinstance(value, list):
        projected_values = []
        for child in value:
            safe = _model_safe_value(child, key, depth + 1, forbidden_locators)
            if safe is not _OMIT:
                projected_values.append(safe)
        return projected_values
    if isinstance(value, str):
        stripped = value.strip()
        if MANAGED_VALUE_PREFIX.match(value):
            return _OMIT
        if SHA256_VALUE.fullmatch(value) and key not in MODEL_SAFE_DIGEST_KEYS:
            return _OMIT
        _refuse_model_locator_string(
            value,
            key,
            forbidden_locators,
            allow_source_text=key in MODEL_SOURCE_TEXT_KEYS,
        )
        return value
    if value is None or isinstance(value, (bool, int, float)):
        return value
    raise BrokerError("SEMANTIC_OUTPUT_INVALID")


def _refuse_model_locator_string(
    value: str,
    key: str | None,
    forbidden_locators: tuple[str, ...],
    *,
    allow_source_text: bool,
) -> None:
    stripped = value.strip()
    if (
        "\0" in value
        or MODEL_PRIVATE_LOCATOR.search(value)
        or MODEL_PRIVATE_ABSOLUTE.search(value)
        or (stripped and Path(stripped).is_absolute())
        or any(locator and locator in value for locator in forbidden_locators)
        or (
            not allow_source_text
            and MODEL_EMBEDDED_ABSOLUTE.search(value) is not None
        )
    ):
        raise BrokerError("MODEL_VISIBLE_LOCATOR_REFUSED")


def _assert_model_visible(
    value: Any,
    forbidden_locators: tuple[str, ...],
    depth: int = 0,
    key: str | None = None,
) -> None:
    """Recursively refuse locators in every byte returned to the model."""

    if depth > 48:
        raise BrokerError("MODEL_VISIBLE_LOCATOR_REFUSED")
    if isinstance(value, dict):
        for key, child in value.items():
            if not isinstance(key, str):
                raise BrokerError("MODEL_VISIBLE_LOCATOR_REFUSED")
            _refuse_model_locator_string(
                key, None, forbidden_locators, allow_source_text=False
            )
            _assert_model_visible(
                child, forbidden_locators, depth + 1, key=key
            )
        return
    if isinstance(value, list):
        for child in value:
            _assert_model_visible(
                child, forbidden_locators, depth + 1, key=key
            )
        return
    if isinstance(value, str):
        # Reuse the same recursive string policy without applying projection.
        if MANAGED_VALUE_PREFIX.match(value) or MODEL_PRIVATE_LOCATOR.search(value):
            raise BrokerError("MODEL_VISIBLE_LOCATOR_REFUSED")
        _refuse_model_locator_string(
            value,
            key,
            forbidden_locators,
            allow_source_text=key in MODEL_SOURCE_TEXT_KEYS,
        )
        return
    if value is None or isinstance(value, (bool, int, float)):
        return
    raise BrokerError("MODEL_VISIBLE_LOCATOR_REFUSED")


def safe_relative_path(value: Any, *, kotlin_only: bool = False) -> str:
    if (
        not isinstance(value, str)
        or not value
        or len(value.encode("utf-8")) > 4096
        or value.startswith("/")
        or "\\" in value
        or "\0" in value
    ):
        raise BrokerError("INVALID_RELATIVE_PATH")
    parts = value.split("/")
    if any(part in {"", ".", ".."} for part in parts):
        raise BrokerError("INVALID_RELATIVE_PATH")
    if kotlin_only and not value.endswith((".kt", ".kts")):
        raise BrokerError("INVALID_RELATIVE_PATH")
    return value


def _private_directory(raw: str) -> Path:
    if not raw.startswith("/"):
        raise BrokerError("BROKER_CAPABILITY_MISSING")
    path = Path(raw)
    try:
        metadata = os.lstat(path)
        resolved = path.resolve(strict=True)
    except OSError as error:
        raise BrokerError("BROKER_CAPABILITY_MISSING") from error
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or metadata.st_uid != os.getuid()
        or resolved != path
    ):
        raise BrokerError("BROKER_CAPABILITY_MISSING")
    return path


def _read_private_message(path: Path) -> dict[str, Any]:
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise BrokerError("BROKER_FRAME_INVALID") from error
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_uid != os.getuid()
            or not 0 < metadata.st_size <= MAX_FRAME_BYTES + 1
        ):
            raise BrokerError("BROKER_FRAME_INVALID")
        chunks = bytearray()
        while len(chunks) <= MAX_FRAME_BYTES:
            chunk = os.read(descriptor, min(65_536, MAX_FRAME_BYTES + 1 - len(chunks)))
            if not chunk:
                break
            chunks.extend(chunk)
    finally:
        os.close(descriptor)
    raw = bytes(chunks)
    try:
        value = json.loads(raw)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise BrokerError("BROKER_FRAME_INVALID") from error
    if not isinstance(value, dict) or raw != canonical_bytes(value) + b"\n":
        raise BrokerError("BROKER_FRAME_INVALID")
    return value


def _write_private_message(path: Path, value: Any) -> None:
    raw = canonical_bytes(value) + b"\n"
    if len(raw) > MAX_FRAME_BYTES:
        raise BrokerError("BROKER_FRAME_LIMIT")
    temporary = path.with_name(f".{path.name}.{secrets.token_hex(8)}.tmp")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(temporary, flags, 0o600)
        try:
            offset = 0
            while offset < len(raw):
                offset += os.write(descriptor, raw[offset:])
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        os.link(temporary, path, follow_symlinks=False)
        temporary.unlink()
        directory_descriptor = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory_descriptor)
        finally:
            os.close(directory_descriptor)
    except OSError as error:
        try:
            temporary.unlink()
        except OSError:
            pass
        raise BrokerError("BROKER_TRANSPORT_FAILED") from error


@dataclass(frozen=True)
class RepositoryBinding:
    alias: str
    path: Path
    revision: str


@dataclass(frozen=True, order=True)
class SelectedFileProvenance:
    service_alias: str
    relative_file: str
    blob_oid: str

    def projection(self) -> dict[str, Any]:
        return {
            "serviceAlias": self.service_alias,
            "relativeFile": self.relative_file,
            "blobOid": self.blob_oid,
        }


@dataclass(frozen=True)
class SourceWindowProvenance:
    service_alias: str
    relative_file: str
    blob_oid: str
    start_byte: int
    end_byte: int
    visible_bytes: int
    source_bytes_digest: str

    @property
    def charged_bytes(self) -> int:
        return self.end_byte - self.start_byte

    def projection(self, *, order: int, operation: str) -> dict[str, Any]:
        return {
            "order": order,
            "operation": operation,
            "serviceAlias": self.service_alias,
            "relativeFile": self.relative_file,
            "blobOid": self.blob_oid,
            "startByte": self.start_byte,
            "endByte": self.end_byte,
            "chargedBytes": self.charged_bytes,
            "visibleBytes": self.visible_bytes,
            "sourceBytesDigest": self.source_bytes_digest,
        }


@dataclass
class BrokerMetrics:
    query_terms: set[str] = field(default_factory=set)
    selected_files: set[tuple[str, str]] = field(default_factory=set)
    opened_blobs: set[tuple[str, str]] = field(default_factory=set)
    returned_facts: int = 0
    source_windows: int = 0
    opened_source_bytes: int = 0
    agent_visible_evidence_bytes: int = 0
    context_creates: int = 0
    context_expansions: int = 0
    max_semantic_command_millis: int = 0
    semantic_context_commands: int = 0
    semantic_callables_commands: int = 0
    semantic_impact_commands: int = 0
    capability_violations: int = 0
    budget_refusals: int = 0
    selected_file_provenance: set[SelectedFileProvenance] = field(default_factory=set)
    opened_file_provenance: set[SelectedFileProvenance] = field(default_factory=set)
    source_window_ledger: list[dict[str, Any]] = field(default_factory=list)
    semantic_timing_ledger: list[dict[str, Any]] = field(default_factory=list)
    ordered_tool_ledger: list[dict[str, Any]] = field(default_factory=list)
    violation_ledger: list[dict[str, Any]] = field(default_factory=list)
    process_group_ledger: list[dict[str, Any]] = field(default_factory=list)

    def projection(self) -> dict[str, int]:
        return {
            "queryTerms": len(self.query_terms),
            "returnedFacts": self.returned_facts,
            "selectedFiles": len(self.selected_file_provenance),
            "sourceWindows": self.source_windows,
            "openedSourceBytes": self.opened_source_bytes,
            "openedSourceFiles": len(self.opened_file_provenance),
            "agentVisibleEvidenceBytes": self.agent_visible_evidence_bytes,
            "contextCreates": self.context_creates,
            "contextExpansions": self.context_expansions,
            "maxSemanticCommandMillis": self.max_semantic_command_millis,
            "semanticContextCommands": self.semantic_context_commands,
            "semanticCallablesCommands": self.semantic_callables_commands,
            "semanticImpactCommands": self.semantic_impact_commands,
            "capabilityViolations": self.capability_violations,
            "budgetRefusals": self.budget_refusals,
        }

    def record_violation(self, code: str, operation: str, source: str) -> None:
        self.capability_violations += 1
        self.violation_ledger.append(
            {
                "order": len(self.violation_ledger) + 1,
                "operation": operation,
                "code": code,
                "source": source,
            }
        )

    def record_process_group(
        self,
        *,
        kind: str,
        purpose: str,
        status: str,
        return_code: int | None,
        timed_out: bool,
        interrupted: bool,
        stdout_overflow: bool,
        group_live_at_finalize: bool,
        residual_after_cleanup: bool,
    ) -> None:
        self.process_group_ledger.append(
            {
                "order": len(self.process_group_ledger) + 1,
                "kind": kind,
                "purpose": purpose,
                "status": status,
                "returnCode": return_code,
                "timedOut": timed_out,
                "interrupted": interrupted,
                "stdoutOverflow": stdout_overflow,
                "groupLiveAtFinalize": group_live_at_finalize,
                "residualAfterCleanup": residual_after_cleanup,
            }
        )


class BrokerSession:
    """One task/arm broker with pre-return resource enforcement."""

    def __init__(self, authority: dict[str, Any], task_id: str, arm: str):
        if arm not in {"DEFAULT", "CODECLEW"}:
            raise BrokerError("INVALID_ARM")
        tasks = authority.get("tasks")
        repositories = authority.get("repositories")
        sessions = authority.get("sessions")
        budgets = authority.get("budgets")
        if (
            not isinstance(tasks, list)
            or not isinstance(repositories, list)
            or not isinstance(sessions, list)
            or not isinstance(budgets, dict)
        ):
            raise BrokerError("INVALID_PILOT_AUTHORITY")
        matching = [row for row in tasks if isinstance(row, dict) and row.get("taskId") == task_id]
        if len(matching) != 1:
            raise BrokerError("INVALID_TASK_AUTHORITY")
        self.task = matching[0]
        self.arm = arm
        self.budgets = budgets
        self.clew = Path(authority.get("executables", {}).get("clew", ""))
        self.git = Path(authority.get("executables", {}).get("git", ""))
        audit = authority.get("brokerAudit")
        self.sandbox_exec: Path | None = None
        self.sandbox_profile: str | None = None
        if audit is not None:
            if (
                not isinstance(audit, dict)
                or audit.get("adapter") != "MACOS_SEATBELT_V1"
                or not isinstance(audit.get("sandboxExecutable"), str)
                or not isinstance(audit.get("profile"), str)
            ):
                raise BrokerError("INVALID_PILOT_AUTHORITY")
            self.sandbox_exec = Path(audit["sandboxExecutable"])
            self.sandbox_profile = audit["profile"]
            if not self.sandbox_exec.is_absolute() or not self.sandbox_exec.is_file():
                raise BrokerError("INVALID_PILOT_AUTHORITY")
        semantic_environment = authority.get("semanticEnvironment")
        if not isinstance(semantic_environment, dict) or any(
            not isinstance(key, str) or not isinstance(value, str) or "\0" in value
            for key, value in semantic_environment.items()
        ):
            raise BrokerError("INVALID_PILOT_AUTHORITY")
        self.semantic_environment = dict(semantic_environment)
        if not self.git.is_absolute() or not self.git.is_file():
            raise BrokerError("INVALID_PILOT_AUTHORITY")
        self.repositories: dict[str, RepositoryBinding] = {}
        for row in repositories:
            if not isinstance(row, dict) or set(row) != {"serviceAlias", "path", "revision"}:
                raise BrokerError("INVALID_PILOT_AUTHORITY")
            alias = row["serviceAlias"]
            path = Path(row["path"])
            revision = row["revision"]
            if (
                not isinstance(alias, str)
                or SAFE_ALIAS.fullmatch(alias) is None
                or not path.is_absolute()
                or not path.is_dir()
                or not isinstance(revision, str)
                or GIT_OID.fullmatch(revision) is None
                or alias in self.repositories
            ):
                raise BrokerError("INVALID_PILOT_AUTHORITY")
            self.repositories[alias] = RepositoryBinding(alias, path, revision)
        expected = {self.task.get("provider"), self.task.get("consumer")}
        if None in expected or not expected.issubset(self.repositories):
            raise BrokerError("INVALID_TASK_AUTHORITY")
        self.sessions: dict[str, dict[str, str]] = {}
        for row in sessions:
            if (
                not isinstance(row, dict)
                or set(row)
                != {
                    "serviceAlias", "sessionId", "sessionAuthorityDigest",
                    "runtimeKey", "runtimeMode",
                }
                or not isinstance(row["serviceAlias"], str)
                or row["serviceAlias"] in self.sessions
                or not isinstance(row["sessionId"], str)
                or not row["sessionId"].startswith("session:")
                or not isinstance(row["sessionAuthorityDigest"], str)
                or SHA256_VALUE.fullmatch(row["sessionAuthorityDigest"]) is None
            ):
                raise BrokerError("INVALID_PILOT_AUTHORITY")
            self.sessions[row["serviceAlias"]] = row
        if not expected.issubset(self.sessions):
            raise BrokerError("INVALID_TASK_AUTHORITY")
        thread = self.task.get("thread")
        if (
            not isinstance(thread, dict)
            or set(thread)
            != {
                "threadId", "threadAuthorityDigest", "providerMember",
                "consumerMember",
            }
            or not isinstance(thread["threadId"], str)
            or not thread["threadId"].startswith("thread:")
            or not isinstance(thread["threadAuthorityDigest"], str)
            or SHA256_VALUE.fullmatch(thread["threadAuthorityDigest"]) is None
            or thread["providerMember"] != "provider"
            or thread["consumerMember"] != "consumer"
        ):
            raise BrokerError("INVALID_TASK_AUTHORITY")
        locators: set[str] = {
            os.fspath(binding.path) for binding in self.repositories.values()
        }
        for raw in authority.get("executables", {}).values():
            if isinstance(raw, str) and Path(raw).is_absolute():
                locators.add(raw)
        for raw in self.semantic_environment.values():
            if isinstance(raw, str) and Path(raw).is_absolute():
                locators.add(raw)
        locators.add(thread["threadId"])
        locators.update(row["sessionId"] for row in self.sessions.values())
        self.model_forbidden_locators = tuple(
            sorted((item for item in locators if item), key=lambda item: (-len(item), item))
        )
        self.metrics = BrokerMetrics()
        self.context_id: str | None = None
        self.context_authority_digest: str | None = None
        self.fact_set_id: str | None = None
        self.fact_set_authority_digest: str | None = None
        self._stop_event: threading.Event | None = None
        self._active_process_lock = threading.Lock()
        self._active_process: subprocess.Popen[Any] | None = None

    def bind_stop_event(self, stop: threading.Event) -> None:
        if self._stop_event is not None and self._stop_event is not stop:
            raise BrokerError("BROKER_SESSION_ALREADY_BOUND")
        self._stop_event = stop

    def terminate_active_children(self) -> None:
        with self._active_process_lock:
            process = self._active_process
        if process is not None:
            _terminate_group(process)

    def audit_projection(self) -> dict[str, Any]:
        """Return the closed, path-free evidence needed to recompute accounting."""

        metrics = self.metrics.projection()
        expected_opened_files = {
            (
                row["serviceAlias"],
                row["relativeFile"],
                row["blobOid"],
            )
            for row in self.metrics.source_window_ledger
        }
        if (
            metrics["queryTerms"] != len(self.metrics.query_terms)
            or metrics["selectedFiles"] != len(self.metrics.selected_file_provenance)
            or metrics["sourceWindows"] != len(self.metrics.source_window_ledger)
            or metrics["openedSourceBytes"]
            != sum(row["chargedBytes"] for row in self.metrics.source_window_ledger)
            or metrics["openedSourceFiles"] != len(expected_opened_files)
            or expected_opened_files
            != {
                (row.service_alias, row.relative_file, row.blob_oid)
                for row in self.metrics.opened_file_provenance
            }
            or metrics["capabilityViolations"] != len(self.metrics.violation_ledger)
            or any(
                row["order"] != index
                for ledger in (
                    self.metrics.ordered_tool_ledger,
                    self.metrics.source_window_ledger,
                    self.metrics.semantic_timing_ledger,
                    self.metrics.violation_ledger,
                    self.metrics.process_group_ledger,
                )
                for index, row in enumerate(ledger, 1)
            )
        ):
            raise BrokerError("BROKER_AUDIT_INCONSISTENT")
        return {
            "schema": "codeclew-kotlin-pilot-broker-audit/1.0",
            "taskId": self.task["taskId"],
            "arm": self.arm,
            "metrics": metrics,
            "queryTerms": sorted(self.metrics.query_terms),
            "orderedToolLedger": list(self.metrics.ordered_tool_ledger),
            "selectedFiles": [
                row.projection()
                for row in sorted(self.metrics.selected_file_provenance)
            ],
            "sourceWindows": list(self.metrics.source_window_ledger),
            "semanticTimingLedger": list(self.metrics.semantic_timing_ledger),
            "violationLedger": list(self.metrics.violation_ledger),
            "processGroupLedger": list(self.metrics.process_group_ledger),
        }

    def _audited(self, command: list[str]) -> list[str]:
        # Protocol authorities always provide the sealed adapter.  The direct
        # form exists only for focused BrokerSession unit fixtures that do not
        # pass through run_thread_kotlin_pilot.verify_authority().
        if self.sandbox_exec is None or self.sandbox_profile is None:
            return command
        return [
            os.fspath(self.sandbox_exec),
            "-p",
            self.sandbox_profile,
            *command,
        ]

    def _repository(self, alias: Any) -> RepositoryBinding:
        if not isinstance(alias, str) or alias not in {
            self.task["provider"], self.task["consumer"]
        }:
            raise BrokerError("MEMBER_OUTSIDE_TASK")
        return self.repositories[alias]

    def _run_child(
        self,
        command: list[str],
        *,
        environment: dict[str, str],
        timeout_seconds: float,
        maximum_stdout_bytes: int,
        kind: str,
        purpose: str,
        failure_code: str,
    ) -> tuple[bytes, int]:
        if maximum_stdout_bytes < 0:
            raise BrokerError("INVALID_PILOT_AUTHORITY")
        process: subprocess.Popen[Any] | None = None
        stdout = bytearray()
        timed_out = False
        interrupted = False
        stdout_overflow = False
        stream_closed = False
        communication_error: OSError | None = None
        selector: selectors.BaseSelector | None = None
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
            with self._active_process_lock:
                self._active_process = process
            if process.stdout is None:
                raise OSError("child stdout pipe is unavailable")
            selector = selectors.DefaultSelector()
            selector.register(process.stdout, selectors.EVENT_READ)
            deadline = time.monotonic() + timeout_seconds
            while True:
                if self._stop_event is not None and self._stop_event.is_set():
                    interrupted = True
                    break
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    timed_out = True
                    break
                try:
                    if stream_closed:
                        if process.poll() is not None:
                            break
                        if self._stop_event is not None:
                            self._stop_event.wait(min(0.1, remaining))
                        else:
                            time.sleep(min(0.1, remaining))
                        continue
                    events = selector.select(min(0.1, remaining))
                    for key, _ in events:
                        remaining_capacity = maximum_stdout_bytes - len(stdout)
                        chunk = os.read(
                            key.fd,
                            min(65_536, max(1, remaining_capacity + 1)),
                        )
                        if not chunk:
                            selector.unregister(key.fileobj)
                            stream_closed = True
                            continue
                        stdout.extend(chunk)
                        if len(stdout) > maximum_stdout_bytes:
                            stdout_overflow = True
                            break
                    if stdout_overflow:
                        break
                    if process.poll() is not None and not events and not stream_closed:
                        # EOF is immediately readable when all writers closed. A
                        # quiet open pipe after leader exit proves a residual or
                        # escaped writer; fail closed instead of waiting for it.
                        break
                except OSError as error:
                    communication_error = error
                    break
        except OSError as error:
            communication_error = error
        finally:
            if selector is not None:
                selector.close()
            if process is not None and process.stdout is not None:
                process.stdout.close()
            with self._active_process_lock:
                if self._active_process is process:
                    self._active_process = None

        if process is None:
            raise BrokerError(failure_code) from communication_error

        completed_normally = (
            not timed_out
            and not interrupted
            and not stdout_overflow
            and communication_error is None
            and stream_closed
            and process.poll() is not None
        )
        group_live = _process_group_exists(process.pid)
        residual_after_cleanup = False
        if group_live or not completed_normally:
            _terminate_group(process)
            residual_after_cleanup = _process_group_exists(process.pid)
        return_code = process.poll()
        if return_code is None:
            try:
                return_code = process.wait(timeout=0.25)
            except subprocess.TimeoutExpired:
                _terminate_group(process)
                residual_after_cleanup = _process_group_exists(process.pid)
                return_code = process.poll()

        status = "OK"
        if interrupted:
            status = "INTERRUPTED"
        elif timed_out:
            status = "TIMEOUT"
        elif stdout_overflow:
            status = "OUTPUT_LIMIT"
        elif communication_error is not None:
            status = "FAILED"
        elif group_live or not stream_closed:
            status = "RESIDUAL"
        self.metrics.record_process_group(
            kind=kind,
            purpose=purpose,
            status=status,
            return_code=return_code,
            timed_out=timed_out,
            interrupted=interrupted,
            stdout_overflow=stdout_overflow,
            group_live_at_finalize=group_live,
            residual_after_cleanup=residual_after_cleanup,
        )
        if residual_after_cleanup or (completed_normally and group_live):
            raise BrokerError("BROKER_PROCESS_GROUP_RESIDUAL")
        if not completed_normally:
            raise BrokerError(failure_code) from communication_error
        return bytes(stdout), int(return_code if return_code is not None else -1)

    def _git(
        self,
        repository: RepositoryBinding,
        arguments: list[str],
        maximum: int = MAX_TOOL_STDOUT_BYTES,
        *,
        allowed_return_codes: frozenset[int] = frozenset({0}),
        purpose: str = "GIT_QUERY",
    ) -> bytes:
        command = self._audited(
            [os.fspath(self.git), "-C", os.fspath(repository.path), *arguments]
        )
        stdout, return_code = self._run_child(
            command,
            environment={
                "PATH": "/usr/bin:/bin",
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_CONFIG_GLOBAL": "/dev/null",
                "GIT_CONFIG_SYSTEM": "/dev/null",
                "GIT_NO_REPLACE_OBJECTS": "1",
                "GIT_TERMINAL_PROMPT": "0",
                "LC_ALL": "C",
            },
            timeout_seconds=30,
            maximum_stdout_bytes=maximum,
            kind="GIT",
            purpose=purpose,
            failure_code="GIT_AUTHORITY_UNAVAILABLE",
        )
        if return_code not in allowed_return_codes or len(stdout) > maximum:
            raise BrokerError("GIT_AUTHORITY_UNAVAILABLE")
        return stdout

    def _blob_oid(self, repository: RepositoryBinding, relative_file: str) -> str:
        raw = self._git(
            repository,
            ["rev-parse", "--verify", f"{repository.revision}:{relative_file}"],
            256,
            purpose="GIT_BLOB_IDENTITY",
        )
        try:
            value = raw.decode("ascii").strip()
        except UnicodeDecodeError as error:
            raise BrokerError("GIT_AUTHORITY_UNAVAILABLE") from error
        if GIT_OID.fullmatch(value) is None:
            raise BrokerError("GIT_AUTHORITY_UNAVAILABLE")
        return value

    def _file_bytes(self, repository: RepositoryBinding, relative_file: str) -> tuple[str, bytes]:
        blob_oid = self._blob_oid(repository, relative_file)
        raw = self._git(
            repository,
            ["show", f"{repository.revision}:{relative_file}"],
            MAX_GIT_OBJECT_BYTES + 1,
            purpose="GIT_BLOB_READ",
        )
        if len(raw) > MAX_GIT_OBJECT_BYTES:
            raise BrokerError("SOURCE_OBJECT_LIMIT")
        return blob_oid, raw

    def _selected_file(
        self,
        repository: RepositoryBinding,
        relative_file: str,
        blob_oid: str,
    ) -> SelectedFileProvenance:
        return SelectedFileProvenance(repository.alias, relative_file, blob_oid)

    def _source_window(
        self,
        repository: RepositoryBinding,
        relative_file: str,
        blob_oid: str,
        raw: bytes,
        start_byte: int,
        end_byte: int,
        visible_bytes: int,
    ) -> SourceWindowProvenance:
        if (
            start_byte < 0
            or end_byte <= start_byte
            or end_byte > len(raw)
            or visible_bytes < 0
        ):
            raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
        return SourceWindowProvenance(
            repository.alias,
            relative_file,
            blob_oid,
            start_byte,
            end_byte,
            visible_bytes,
            _sha256_digest(raw[start_byte:end_byte]),
        )

    def _line_source_window(
        self,
        repository: RepositoryBinding,
        relative_file: str,
        blob_oid: str,
        raw: bytes,
        start_line: int,
        end_line: int,
        visible_text: str,
    ) -> SourceWindowProvenance:
        try:
            source = raw.decode("utf-8")
        except UnicodeDecodeError as error:
            raise BrokerError("SOURCE_NOT_UTF8") from error
        if start_line < 1 or end_line < start_line:
            raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
        raw_parts = raw.split(b"\n")
        raw_lines = [
            part + (b"\n" if index < len(raw_parts) - 1 else b"")
            for index, part in enumerate(raw_parts)
        ]
        if raw.endswith(b"\n"):
            raw_lines.pop()
        text_lines = source.split("\n")
        if source.endswith("\n"):
            text_lines.pop()
        text_lines = [line[:-1] if line.endswith("\r") else line for line in text_lines]
        logical_count = max(1, len(text_lines))
        if end_line > logical_count or not raw_lines:
            raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
        expected = "\n".join(text_lines[start_line - 1 : end_line])
        whole_file = start_line == 1 and end_line == logical_count and visible_text == source
        start_byte = sum(len(line) for line in raw_lines[: start_line - 1])
        raw_end_byte = sum(len(line) for line in raw_lines[:end_line])
        if raw_lines[end_line - 1].endswith(b"\n"):
            raw_end_byte -= 1
        try:
            raw_visible = raw[start_byte:raw_end_byte].decode("utf-8")
        except UnicodeDecodeError as error:
            raise BrokerError("SOURCE_NOT_UTF8") from error
        if not whole_file and visible_text not in {expected, raw_visible}:
            raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
        if whole_file:
            end_byte = len(raw)
        elif visible_text == raw_visible:
            end_byte = raw_end_byte
        else:
            final_line = raw_lines[end_line - 1]
            if final_line.endswith(b"\n"):
                final_line = final_line[:-1]
                if final_line.endswith(b"\r"):
                    final_line = final_line[:-1]
            end_byte = sum(len(line) for line in raw_lines[: end_line - 1]) + len(final_line)
        return self._source_window(
            repository,
            relative_file,
            blob_oid,
            raw,
            start_byte,
            end_byte,
            len(visible_text.encode("utf-8")),
        )

    @staticmethod
    def _verify_content_ref(reference: Any, raw: bytes) -> None:
        if not isinstance(reference, dict):
            raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
        object_schema = reference.get("objectSchema")
        digest = reference.get("digest")
        size = reference.get("size")
        if (
            not isinstance(object_schema, str)
            or not object_schema
            or digest != _cas_object_digest(object_schema, raw)
            or type(size) is not int
            or size != len(raw)
        ):
            raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")

    def _limits(self) -> dict[str, int]:
        required = {
            "queryTerms",
            "returnedFacts",
            "selectedFiles",
            "sourceWindows",
            "agentVisibleEvidenceBytes",
            "contextCreates",
            "contextExpansions",
            "singleSemanticCommandMs",
        }
        if set(self.budgets).issuperset(required) is False:
            raise BrokerError("INVALID_PILOT_AUTHORITY")
        result: dict[str, int] = {}
        for key in required:
            value = self.budgets[key]
            if type(value) is not int or value < 0:
                raise BrokerError("INVALID_PILOT_AUTHORITY")
            result[key] = value
        return result

    def _commit_response(
        self,
        payload: Any,
        *,
        terms: tuple[str, ...] = (),
        files: tuple[SelectedFileProvenance, ...] = (),
        windows: tuple[SourceWindowProvenance, ...] = (),
        returned_facts: int = 0,
        context_creates: int = 0,
        context_expansions: int = 0,
        semantic_millis: int = 0,
        semantic_view: str | None = None,
    ) -> dict[str, Any]:
        response = {
            "schema": RESPONSE_SCHEMA,
            "status": "OK",
            "operation": payload["operation"],
            "result": payload["result"],
        }
        _assert_model_visible(response, self.model_forbidden_locators)
        rendered = canonical_bytes(response) + b"\n"
        limits = self._limits()
        projected_terms = self.metrics.query_terms | set(terms)
        projected_file_provenance = self.metrics.selected_file_provenance | set(files)
        projected_files = self.metrics.selected_files | {
            (row.service_alias, row.blob_oid) for row in files
        }
        projected_opened_files = self.metrics.opened_file_provenance | {
            SelectedFileProvenance(row.service_alias, row.relative_file, row.blob_oid)
            for row in windows
        }
        projected_blobs = self.metrics.opened_blobs | {
            (row.service_alias, row.blob_oid) for row in windows
        }
        checks = {
            "queryTerms": len(projected_terms),
            "returnedFacts": self.metrics.returned_facts + returned_facts,
            "selectedFiles": len(projected_file_provenance),
            "sourceWindows": self.metrics.source_windows + len(windows),
            "agentVisibleEvidenceBytes": self.metrics.agent_visible_evidence_bytes
            + len(rendered),
            "contextCreates": self.metrics.context_creates + context_creates,
            "contextExpansions": self.metrics.context_expansions + context_expansions,
            "singleSemanticCommandMs": semantic_millis,
        }
        if any(checks[key] > limits[key] for key in checks):
            self.metrics.budget_refusals += 1
            raise BrokerError("BROKER_RESOURCE_BUDGET")
        self.metrics.query_terms = projected_terms
        self.metrics.selected_files = projected_files
        self.metrics.selected_file_provenance = projected_file_provenance
        self.metrics.opened_file_provenance = projected_opened_files
        self.metrics.opened_blobs = projected_blobs
        self.metrics.returned_facts += returned_facts
        self.metrics.source_windows += len(windows)
        self.metrics.opened_source_bytes += sum(row.charged_bytes for row in windows)
        operation = payload["operation"]
        for row in windows:
            self.metrics.source_window_ledger.append(
                row.projection(
                    order=len(self.metrics.source_window_ledger) + 1,
                    operation=operation,
                )
            )
        self.metrics.agent_visible_evidence_bytes += len(rendered)
        self.metrics.context_creates += context_creates
        self.metrics.context_expansions += context_expansions
        self.metrics.max_semantic_command_millis = max(
            self.metrics.max_semantic_command_millis, semantic_millis
        )
        if semantic_view == "context":
            self.metrics.semantic_context_commands += 1
        elif semantic_view == "callables":
            self.metrics.semantic_callables_commands += 1
        elif semantic_view == "impact":
            self.metrics.semantic_impact_commands += 1
        return response

    def handle(self, request: dict[str, Any]) -> dict[str, Any]:
        if request.get("schema") != REQUEST_SCHEMA or not isinstance(request.get("operation"), str):
            self.metrics.record_violation(
                "INVALID_BROKER_REQUEST", "INVALID", "REQUEST_GRAMMAR"
            )
            raise BrokerError("INVALID_BROKER_REQUEST")
        operation = request["operation"]
        if operation == "capability":
            if set(request) != {"schema", "operation"}:
                raise BrokerError("INVALID_BROKER_REQUEST")
            semantic = (
                ["context", "callables", "impact"]
                if self.arm == "CODECLEW"
                else []
            )
            result = {
                "arm": self.arm,
                "common": ["capability", "tree", "search", "show", "read"],
                "semantic": semantic,
                "grammar": {
                    "capability": "capability",
                    "tree": "tree --member SERVICE_ALIAS [--prefix RELATIVE_PREFIX] [--limit 1..12]",
                    "search": "search --member SERVICE_ALIAS --term TERM [--limit 1..128]",
                    "show": "show --member SERVICE_ALIAS --file RELATIVE_KOTLIN_FILE",
                    "read": "read --member SERVICE_ALIAS --file RELATIVE_KOTLIN_FILE --start-byte N --end-byte N",
                    **(
                        {
                            "context": "semantic-context --term TERM [--term TERM ...]",
                            "callables": "semantic-callables --term TERM [--term TERM ...]",
                            "impact": "semantic-impact --subject-kind full-symbol|callable-family|token --subject SUBJECT [--member THREAD_MEMBER_ALIAS]",
                        }
                        if self.arm == "CODECLEW"
                        else {}
                    ),
                },
                "budgets": self.budgets,
            }
            return self._commit_response(
                {"operation": operation, "result": result}
            )
        if operation == "tree":
            return self._tree(request)
        if operation == "search":
            return self._search(request)
        if operation == "show":
            return self._show(request)
        if operation == "read":
            return self._read(request)
        if operation.startswith("semantic-"):
            if self.arm != "CODECLEW":
                raise BrokerError("ARM_CAPABILITY_UNAVAILABLE")
            return self._semantic(operation.removeprefix("semantic-"), request)
        self.metrics.record_violation(
            "UNKNOWN_BROKER_OPERATION", operation, "REQUEST_GRAMMAR"
        )
        raise BrokerError("UNKNOWN_BROKER_OPERATION")

    def _tree(self, request: dict[str, Any]) -> dict[str, Any]:
        if set(request) != {"schema", "operation", "member", "prefix", "limit"}:
            raise BrokerError("INVALID_BROKER_REQUEST")
        repository = self._repository(request["member"])
        prefix = request["prefix"]
        if prefix:
            prefix = safe_relative_path(prefix)
        limit = request["limit"]
        if type(limit) is not int or not 1 <= limit <= 12:
            raise BrokerError("INVALID_BROKER_REQUEST")
        arguments = ["ls-tree", "-r", "--name-only", repository.revision]
        if prefix:
            arguments.extend(["--", prefix])
        raw = self._git(repository, arguments, purpose="GIT_TREE")
        try:
            paths = [line for line in raw.decode("utf-8").splitlines() if line.endswith((".kt", ".kts"))]
        except UnicodeDecodeError as error:
            raise BrokerError("GIT_AUTHORITY_UNAVAILABLE") from error
        rows = []
        for path in paths[:limit]:
            relative = safe_relative_path(path, kotlin_only=True)
            rows.append({"relativeFile": relative, "blobOid": self._blob_oid(repository, relative)})
        files = tuple(
            self._selected_file(repository, row["relativeFile"], row["blobOid"])
            for row in rows
        )
        return self._commit_response(
            {"operation": "tree", "result": {"files": rows, "truncated": len(paths) > limit}},
            files=files,
            returned_facts=len(rows),
        )

    def _show(self, request: dict[str, Any]) -> dict[str, Any]:
        if set(request) != {"schema", "operation", "member", "file"}:
            raise BrokerError("INVALID_BROKER_REQUEST")
        repository = self._repository(request["member"])
        relative = safe_relative_path(request["file"], kotlin_only=True)
        blob_oid, raw = self._file_bytes(repository, relative)
        return self._commit_response(
            {
                "operation": "show",
                "result": {"relativeFile": relative, "blobOid": blob_oid, "size": len(raw)},
            },
            files=(self._selected_file(repository, relative, blob_oid),),
            returned_facts=1,
        )

    def _read(self, request: dict[str, Any]) -> dict[str, Any]:
        if set(request) != {"schema", "operation", "member", "file", "startByte", "endByte"}:
            raise BrokerError("INVALID_BROKER_REQUEST")
        repository = self._repository(request["member"])
        relative = safe_relative_path(request["file"], kotlin_only=True)
        start = request["startByte"]
        end = request["endByte"]
        if (
            type(start) is not int
            or type(end) is not int
            or start < 0
            or end <= start
            or end - start > MAX_SOURCE_WINDOW_BYTES
        ):
            raise BrokerError("INVALID_SOURCE_WINDOW")
        blob_oid, raw = self._file_bytes(repository, relative)
        if end > len(raw):
            raise BrokerError("INVALID_SOURCE_WINDOW")
        selected = raw[start:end]
        try:
            content = selected.decode("utf-8")
        except UnicodeDecodeError as error:
            raise BrokerError("SOURCE_NOT_UTF8") from error
        return self._commit_response(
            {
                "operation": "read",
                "result": {
                    "relativeFile": relative,
                    "blobOid": blob_oid,
                    "startByte": start,
                    "endByte": end,
                    "content": content,
                },
            },
            files=(self._selected_file(repository, relative, blob_oid),),
            windows=(
                self._source_window(
                    repository,
                    relative,
                    blob_oid,
                    raw,
                    start,
                    end,
                    len(selected),
                ),
            ),
            returned_facts=1,
        )

    def _search(self, request: dict[str, Any]) -> dict[str, Any]:
        if set(request) != {"schema", "operation", "member", "term", "limit"}:
            raise BrokerError("INVALID_BROKER_REQUEST")
        repository = self._repository(request["member"])
        term = request["term"]
        limit = request["limit"]
        if not isinstance(term, str) or SAFE_TERM.fullmatch(term) is None or type(limit) is not int or not 1 <= limit <= 128:
            raise BrokerError("INVALID_SEARCH_REQUEST")
        stdout = self._git(
            repository,
            [
                "grep", "-n", "-I", "-F", "-z", "-e", term,
                repository.revision, "--", "*.kt", "*.kts",
            ],
            allowed_return_codes=frozenset({0, 1}),
            purpose="GIT_SEARCH",
        )
        matches: list[tuple[str, int, str]] = []
        cursor = 0
        try:
            while cursor < len(stdout):
                path_end = stdout.index(b"\0", cursor)
                line_end = stdout.index(b"\0", path_end + 1)
                text_end = stdout.index(b"\n", line_end + 1)
                header = stdout[cursor:path_end].decode("utf-8")
                revision, separator, relative = header.partition(":")
                if not separator or revision != repository.revision:
                    raise BrokerError("GIT_AUTHORITY_UNAVAILABLE")
                line_number = int(stdout[path_end + 1 : line_end].decode("ascii"))
                text = stdout[line_end + 1 : text_end].decode("utf-8")
                matches.append((relative, line_number, text))
                cursor = text_end + 1
        except (ValueError, UnicodeDecodeError) as error:
            raise BrokerError("GIT_AUTHORITY_UNAVAILABLE") from error
        rows: list[dict[str, Any]] = []
        files: set[SelectedFileProvenance] = set()
        windows: list[SourceWindowProvenance] = []
        cached_files: dict[str, tuple[str, bytes]] = {}
        for raw_relative, line_number, text in matches[:limit]:
            relative = safe_relative_path(raw_relative, kotlin_only=True)
            if relative not in cached_files:
                cached_files[relative] = self._file_bytes(repository, relative)
            blob_oid, raw = cached_files[relative]
            selected_file = self._selected_file(repository, relative, blob_oid)
            files.add(selected_file)
            windows.append(
                self._line_source_window(
                    repository,
                    relative,
                    blob_oid,
                    raw,
                    line_number,
                    line_number,
                    text,
                )
            )
            rows.append(
                {
                    "relativeFile": relative,
                    "blobOid": blob_oid,
                    "line": line_number,
                    "text": text,
                }
            )
        return self._commit_response(
            {
                "operation": "search",
                "result": {"matches": rows, "truncated": len(matches) > limit},
            },
            terms=(term,),
            files=tuple(files),
            windows=tuple(windows),
            returned_facts=len(rows),
        )

    def _semantic_model_projection(
        self, view: str, value: dict[str, Any]
    ) -> dict[str, Any]:
        if view == "context":
            projected = self._context_model_projection(value)
        elif view == "callables":
            projected = self._callables_model_projection(value)
        elif view == "impact":
            projected = self._impact_model_projection(value)
        else:
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        safe = _model_safe_value(
            projected, forbidden_locators=self.model_forbidden_locators
        )
        if not isinstance(safe, dict) or safe != projected:
            # Schema-specific constructors must already have removed every
            # managed authority. A second-pass change means their allowlist is
            # incomplete, so fail closed instead of silently weakening it.
            raise BrokerError("SEMANTIC_MODEL_PROJECTION_INVALID")
        return projected

    @staticmethod
    def _semantic_digest(value: Any) -> str:
        if not isinstance(value, str) or SHA256_VALUE.fullmatch(value) is None:
            raise BrokerError("SEMANTIC_AUTHORITY_BINDING_INVALID")
        return value

    @classmethod
    def _semantic_cas_reference(cls, value: Any) -> dict[str, Any]:
        row = _closed_object(value, {"schema", "objectSchema", "digest", "size"})
        if (
            not isinstance(row["schema"], str)
            or not row["schema"]
            or not isinstance(row["objectSchema"], str)
            or not row["objectSchema"]
            or cls._semantic_digest(row["digest"]) != row["digest"]
            or type(row["size"]) is not int
            or row["size"] < 0
        ):
            raise BrokerError("SEMANTIC_AUTHORITY_BINDING_INVALID")
        return row

    def _semantic_member_authority(
        self, thread: dict[str, Any]
    ) -> dict[str, dict[str, str]]:
        return {
            thread["providerMember"]: {
                "serviceAlias": self.task["provider"],
                "sessionId": self.sessions[self.task["provider"]]["sessionId"],
                "sessionAuthorityDigest": self.sessions[self.task["provider"]][
                    "sessionAuthorityDigest"
                ],
            },
            thread["consumerMember"]: {
                "serviceAlias": self.task["consumer"],
                "sessionId": self.sessions[self.task["consumer"]]["sessionId"],
                "sessionAuthorityDigest": self.sessions[self.task["consumer"]][
                    "sessionAuthorityDigest"
                ],
            },
        }

    def _impact_binding_digest(
        self, request: dict[str, Any], thread: dict[str, Any]
    ) -> str:
        kind = request.get("subjectKind")
        subject_value = request.get("subject")
        member = request.get("member")
        if kind == "full-symbol":
            subject: dict[str, Any] = {
                "kind": "FULL_SYMBOL",
                "symbolIdentity": subject_value,
            }
            if member is not None:
                subject["memberAlias"] = member
        elif kind == "callable-family":
            subject = {"kind": "CALLABLE_FAMILY", "callableId": subject_value}
        elif kind == "token":
            subject = {"kind": "TOKEN", "term": subject_value}
        else:
            raise BrokerError("SEMANTIC_REQUEST_BINDING_INVALID")
        product_request = {
            "factSetAuthorityDigest": self.fact_set_authority_digest,
            "pairId": self.task["pairId"],
            "subject": subject,
            "budgets": IMPACT_BUDGETS,
        }
        pair = {
            "pairId": self.task["pairId"],
            "providerMember": thread["providerMember"],
            "consumerMember": thread["consumerMember"],
            "relationshipAuthority": "DECLARED_TOPOLOGY",
            "dependencyEvidenceRef": None,
        }
        return _sha256_digest(
            canonical_bytes(
                {
                    "schema": "codeclew-kotlin-thread-impact-binding/1.0",
                    "factSetAuthorityDigest": self.fact_set_authority_digest,
                    "request": product_request,
                    "pair": pair,
                }
            )
        )

    def _validate_semantic_bindings(
        self,
        view: str,
        value: dict[str, Any],
        thread: dict[str, Any],
        terms: tuple[str, ...],
        request: dict[str, Any],
    ) -> dict[str, str]:
        """Bind every semantic root and nested projection to this exact arm."""

        expected_members = self._semantic_member_authority(thread)
        thread_id = thread["threadId"]
        thread_digest = thread["threadAuthorityDigest"]
        if view == "context":
            root = _closed_object(
                value,
                {
                    "schema", "threadId", "threadAuthorityDigest", "contextId",
                    "contextAuthorityDigest", "evidenceDigest", "evidenceRef",
                    "context",
                },
            )
            context = _closed_object(
                root["context"],
                {
                    "schema", "threadId", "threadAuthorityDigest", "contextId",
                    "contextAuthorityDigest", "task", "members", "matches",
                    "sources", "completeness", "publicationPolicy",
                    "verificationObligations", "obligationCount",
                    "obligationsTruncated", "truncated",
                },
            )
            context_digest = self._semantic_digest(root["contextAuthorityDigest"])
            evidence_digest = self._semantic_digest(root["evidenceDigest"])
            evidence_ref = self._semantic_cas_reference(root["evidenceRef"])
            context_id = root["contextId"]
            if (
                root["schema"] != CONTEXT_RESULT_SCHEMA
                or root["threadId"] != thread_id
                or root["threadAuthorityDigest"] != thread_digest
                or not isinstance(context_id, str)
                or context_id != f"thread-context:{context_digest}"
                or evidence_ref["digest"] != evidence_digest
                or context["schema"] != "codeclew-thread-context-projection/1.0"
                or context["threadId"] != thread_id
                or context["threadAuthorityDigest"] != thread_digest
                or context["contextId"] != context_id
                or context["contextAuthorityDigest"] != context_digest
            ):
                raise BrokerError("SEMANTIC_AUTHORITY_BINDING_INVALID")
            task = _closed_object(context["task"], {"intent", "terms"})
            normalized_terms = sorted(set(terms))
            if (
                task["intent"] != "frozen Kotlin descriptor pilot navigation"
                or task["terms"] != normalized_terms
            ):
                raise BrokerError("SEMANTIC_REQUEST_BINDING_INVALID")
            member_rows = context["members"]
            if not isinstance(member_rows, list) or len(member_rows) != 2:
                raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
            observed_members: dict[str, dict[str, Any]] = {}
            for raw in member_rows:
                row = _closed_object(
                    raw,
                    {
                        "memberAlias", "serviceAlias", "sessionId", "language",
                        "compilations", "contextId", "contextDigest",
                        "evidenceDigest",
                    },
                )
                alias = row["memberAlias"]
                expected = expected_members.get(alias)
                if (
                    expected is None
                    or alias in observed_members
                    or row["serviceAlias"] != expected["serviceAlias"]
                    or row["sessionId"] != expected["sessionId"]
                    or row["language"] != "language:kotlin"
                    or not isinstance(row["compilations"], list)
                    or not row["compilations"]
                    or any(
                        not isinstance(item, str) or not item
                        for item in row["compilations"]
                    )
                    or not isinstance(row["contextId"], str)
                    or not row["contextId"].startswith("context:")
                ):
                    raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
                self._semantic_digest(row["contextDigest"])
                self._semantic_digest(row["evidenceDigest"])
                observed_members[alias] = row
            if set(observed_members) != set(expected_members):
                raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")

            def bind_member_wrapper(raw: Any) -> None:
                if not isinstance(raw, dict):
                    raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
                alias = raw.get("memberAlias")
                member = observed_members.get(alias)
                expected = expected_members.get(alias)
                if member is None or expected is None:
                    raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
                for key, expected_value in {
                    "serviceAlias": expected["serviceAlias"],
                    "sessionId": expected["sessionId"],
                    "sessionAuthorityDigest": expected["sessionAuthorityDigest"],
                    "language": "language:kotlin",
                    "contextId": member["contextId"],
                    "contextDigest": member["contextDigest"],
                    "evidenceDigest": member["evidenceDigest"],
                }.items():
                    if raw.get(key) != expected_value:
                        raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")

            for key in ("matches", "sources"):
                rows = context[key]
                if not isinstance(rows, list):
                    raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
                for raw in rows:
                    bind_member_wrapper(raw)
            obligations = context["verificationObligations"]
            if not isinstance(obligations, list):
                raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
            for raw in obligations:
                if not isinstance(raw, dict):
                    raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
                alias = raw.get("memberAlias")
                member = observed_members.get(alias)
                expected = expected_members.get(alias)
                if (
                    member is None
                    or expected is None
                    or raw.get("serviceAlias") != expected["serviceAlias"]
                    or raw.get("sessionId") != expected["sessionId"]
                    or raw.get("contextId") != member["contextId"]
                ):
                    raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
            if (
                type(context["obligationCount"]) is not int
                or context["obligationCount"] < len(obligations)
                or type(context["obligationsTruncated"]) is not bool
                or type(context["truncated"]) is not bool
                or (
                    context["obligationsTruncated"] is False
                    and context["obligationCount"] != len(obligations)
                )
            ):
                raise BrokerError("SEMANTIC_OUTPUT_INVALID")
            return {
                "contextId": context_id,
                "contextAuthorityDigest": context_digest,
            }

        if view == "callables":
            if self.context_id is None or self.context_authority_digest is None:
                raise BrokerError("SEMANTIC_SEQUENCE_INVALID")
            root = _closed_object(
                value,
                {
                    "schema", "threadId", "threadAuthorityDigest", "contextId",
                    "contextAuthorityDigest", "factSetId", "authorityDigest",
                    "evidenceRef", "queryIndexRef", "callables",
                },
            )
            projection = _closed_object(
                root["callables"],
                {
                    "schema", "factSetId", "authorityDigest", "bindingDigest",
                    "threadId", "threadContextId", "tasks", "pairs", "members",
                    "counts", "completeness", "queryIndexRef", "evidenceRef",
                },
            )
            authority = self._semantic_digest(root["authorityDigest"])
            fact_set_id = root["factSetId"]
            evidence_ref = self._semantic_cas_reference(root["evidenceRef"])
            query_ref = self._semantic_cas_reference(root["queryIndexRef"])
            if (
                root["schema"] != CALLABLE_RESULT_SCHEMA
                or root["threadId"] != thread_id
                or root["threadAuthorityDigest"] != thread_digest
                or root["contextId"] != self.context_id
                or root["contextAuthorityDigest"] != self.context_authority_digest
                or not isinstance(fact_set_id, str)
                or fact_set_id != f"thread-callables:{authority}"
                or projection["schema"]
                != "codeclew-kotlin-callable-fact-set-projection/1.0"
                or projection["factSetId"] != fact_set_id
                or projection["authorityDigest"] != authority
                or projection["threadId"] != thread_id
                or projection["threadContextId"] != self.context_id
                or projection["evidenceRef"] != evidence_ref
                or projection["queryIndexRef"] != query_ref
            ):
                raise BrokerError("SEMANTIC_AUTHORITY_BINDING_INVALID")
            self._semantic_digest(projection["bindingDigest"])
            normalized_terms = sorted(set(terms))
            tasks = projection["tasks"]
            if not isinstance(tasks, list) or len(tasks) != 1:
                raise BrokerError("SEMANTIC_REQUEST_BINDING_INVALID")
            task = _closed_object(
                tasks[0], {"taskId", "pairId", "termCount", "termsDigest"}
            )
            if (
                task["taskId"] != self.task["taskId"]
                or task["pairId"] != self.task["pairId"]
                or task["termCount"] != len(normalized_terms)
                or task["termsDigest"] != _sha256_digest(canonical_bytes(normalized_terms))
            ):
                raise BrokerError("SEMANTIC_REQUEST_BINDING_INVALID")
            pairs = projection["pairs"]
            if not isinstance(pairs, list) or len(pairs) != 1:
                raise BrokerError("SEMANTIC_REQUEST_BINDING_INVALID")
            pair = _closed_object(
                pairs[0],
                {
                    "pairId", "providerMember", "consumerMember",
                    "relationshipAuthority", "dependencyEvidenceRef",
                },
            )
            if (
                pair["pairId"] != self.task["pairId"]
                or pair["providerMember"] != thread["providerMember"]
                or pair["consumerMember"] != thread["consumerMember"]
                or pair["relationshipAuthority"] != "DECLARED_TOPOLOGY"
                or pair["dependencyEvidenceRef"] is not None
            ):
                raise BrokerError("SEMANTIC_REQUEST_BINDING_INVALID")
            members = projection["members"]
            if not isinstance(members, list) or len(members) != 2:
                raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
            observed = set()
            for raw in members:
                member = _closed_object(
                    raw,
                    {
                        "memberAlias", "serviceAlias", "repositoryNamespace",
                        "compilations",
                    },
                )
                alias = member["memberAlias"]
                expected = expected_members.get(alias)
                if (
                    expected is None
                    or (alias, member["serviceAlias"]) in observed
                    or member["serviceAlias"] != expected["serviceAlias"]
                    or not isinstance(member["repositoryNamespace"], str)
                    or not member["repositoryNamespace"]
                    or not isinstance(member["compilations"], list)
                ):
                    raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
                observed.add((alias, member["serviceAlias"]))
            if observed != {
                (alias, expected["serviceAlias"])
                for alias, expected in expected_members.items()
            }:
                raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
            counts = _closed_object(
                projection["counts"],
                {
                    "visitedInputFacts", "visitedInputPayloadBytes", "declarations",
                    "uses", "boundaries", "total", "exactDeclarations", "exactUses",
                },
            )
            if (
                any(type(item) is not int or item < 0 for item in counts.values())
                or counts["total"]
                != counts["declarations"] + counts["uses"] + counts["boundaries"]
                or counts["exactDeclarations"] > counts["declarations"]
                or counts["exactUses"] > counts["uses"]
            ):
                raise BrokerError("SEMANTIC_OUTPUT_INVALID")
            return {
                "factSetId": fact_set_id,
                "factSetAuthorityDigest": authority,
            }

        if view == "impact":
            if self.fact_set_id is None or self.fact_set_authority_digest is None:
                raise BrokerError("SEMANTIC_SEQUENCE_INVALID")
            root = _closed_object(
                value,
                {
                    "schema", "threadId", "threadAuthorityDigest", "factSetId",
                    "factSetAuthorityDigest", "impactId", "authorityDigest",
                    "evidenceRef", "impact",
                },
            )
            impact = _closed_object(
                root["impact"],
                {
                    "schema", "impactId", "authorityDigest", "bindingDigest",
                    "factSetAuthorityDigest", "pairId", "subjectKind",
                    "relationshipAuthority", "shapeStatus", "certainty", "members",
                    "findingCount", "sourceWindowCount", "obligationCount",
                    "findingsTruncated", "sourceWindowsTruncated", "findings",
                    "publicFindingsTruncated", "obligations", "sourceWindows",
                    "evidenceRef",
                },
            )
            impact_authority = self._semantic_digest(root["authorityDigest"])
            evidence_ref = self._semantic_cas_reference(root["evidenceRef"])
            expected_binding_digest = self._impact_binding_digest(request, thread)
            expected_kind = {
                "full-symbol": "FULL_SYMBOL",
                "callable-family": "CALLABLE_FAMILY",
                "token": "TOKEN",
            }[request["subjectKind"]]
            if (
                root["schema"] != IMPACT_RESULT_SCHEMA
                or root["threadId"] != thread_id
                or root["threadAuthorityDigest"] != thread_digest
                or root["factSetId"] != self.fact_set_id
                or root["factSetAuthorityDigest"] != self.fact_set_authority_digest
                or root["impactId"] != f"thread-impact:{impact_authority}"
                or impact["schema"] != "codeclew-kotlin-thread-impact-projection/1.0"
                or impact["impactId"] != root["impactId"]
                or impact["authorityDigest"] != impact_authority
                or impact["bindingDigest"] != expected_binding_digest
                or impact["factSetAuthorityDigest"] != self.fact_set_authority_digest
                or impact["pairId"] != self.task["pairId"]
                or impact["subjectKind"] != expected_kind
                or impact["relationshipAuthority"] != "DECLARED_TOPOLOGY"
                or impact["certainty"] != "UNSURE"
                or impact["evidenceRef"] != evidence_ref
            ):
                raise BrokerError("SEMANTIC_AUTHORITY_BINDING_INVALID")
            self._semantic_digest(impact["bindingDigest"])
            if impact["shapeStatus"] not in {
                "EXACT_PROJECTED_SHAPE_EQUAL", "EXACT_PROJECTED_SHAPE_DELTA",
                "UNSURE", "NOT_COMPARABLE",
            }:
                raise BrokerError("SEMANTIC_OUTPUT_INVALID")
            members = impact["members"]
            expected_member_sides = {
                ("PROVIDER", thread["providerMember"]),
                ("CONSUMER", thread["consumerMember"]),
            }
            if not isinstance(members, list) or len(members) != 2:
                raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
            observed_member_sides = set()
            for raw in members:
                member = _closed_object(
                    raw,
                    {
                        "side", "memberAlias", "observed", "matchedFindingCount",
                        "selectedFindingCount", "declarationCount", "useCount",
                        "boundaryCount",
                    },
                    {"exactShapeDigest"},
                )
                observed_member_sides.add((member["side"], member["memberAlias"]))
                for key in (
                    "matchedFindingCount", "selectedFindingCount", "declarationCount",
                    "useCount", "boundaryCount",
                ):
                    if type(member[key]) is not int or member[key] < 0:
                        raise BrokerError("SEMANTIC_OUTPUT_INVALID")
                if type(member["observed"]) is not bool:
                    raise BrokerError("SEMANTIC_OUTPUT_INVALID")
                if "exactShapeDigest" in member:
                    self._semantic_digest(member["exactShapeDigest"])
            if observed_member_sides != expected_member_sides:
                raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
            collections = {
                "findings": "findingCount",
                "sourceWindows": "sourceWindowCount",
                "obligations": "obligationCount",
            }
            for key, count_key in collections.items():
                if (
                    not isinstance(impact[key], list)
                    or impact[count_key] != len(impact[key])
                ):
                    raise BrokerError("SEMANTIC_OUTPUT_INVALID")
            if any(
                impact[key] is not False
                for key in (
                    "findingsTruncated", "sourceWindowsTruncated",
                    "publicFindingsTruncated",
                )
            ):
                raise BrokerError("SEMANTIC_OUTPUT_INVALID")
            for raw in impact["findings"]:
                if not isinstance(raw, dict) or (
                    raw.get("side"), raw.get("memberAlias")
                ) not in expected_member_sides:
                    raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
            for raw in impact["sourceWindows"]:
                if not isinstance(raw, dict) or (
                    raw.get("side"), raw.get("memberAlias")
                ) not in expected_member_sides:
                    raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
            for raw in impact["obligations"]:
                if not isinstance(raw, dict):
                    raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
                alias = raw.get("memberAlias")
                if alias is not None and alias not in expected_members:
                    raise BrokerError("SEMANTIC_MEMBER_BINDING_INVALID")
            return {}
        raise BrokerError("SEMANTIC_OUTPUT_INVALID")

    def _context_model_projection(self, value: dict[str, Any]) -> dict[str, Any]:
        root = _closed_object(
            value,
            {
                "schema", "threadId", "threadAuthorityDigest", "contextId",
                "contextAuthorityDigest", "evidenceDigest", "evidenceRef", "context",
            },
        )
        if root["schema"] != CONTEXT_RESULT_SCHEMA:
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        context = _closed_object(
            root["context"],
            {
                "schema", "threadId", "threadAuthorityDigest", "contextId",
                "contextAuthorityDigest", "task", "members", "matches", "sources",
                "completeness", "publicationPolicy", "verificationObligations",
                "obligationCount", "obligationsTruncated", "truncated",
            },
        )
        if context["schema"] != "codeclew-thread-context-projection/1.0":
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        task = _closed_object(context["task"], {"intent", "terms"})
        members = []
        if not isinstance(context["members"], list):
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        for raw in context["members"]:
            row = _closed_object(
                raw,
                {
                    "memberAlias", "serviceAlias", "sessionId", "language",
                    "compilations", "contextId", "contextDigest", "evidenceDigest",
                },
            )
            members.append(
                {
                    "memberAlias": row["memberAlias"],
                    "serviceAlias": row["serviceAlias"],
                    "language": row["language"],
                }
            )
        matches = []
        if not isinstance(context["matches"], list):
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        for raw in context["matches"]:
            row = _closed_object(
                raw,
                {
                    "compilation", "factKey", "domainUri", "payloadRef", "payload",
                    "memberAlias", "serviceAlias", "sessionId", "sessionAuthorityDigest",
                    "language", "contextId", "contextDigest", "evidenceDigest",
                },
            )
            payload = _model_safe_value(
                row["payload"], forbidden_locators=self.model_forbidden_locators
            )
            if payload is _OMIT:
                payload = {}
            matches.append(
                {
                    "memberAlias": row["memberAlias"],
                    "serviceAlias": row["serviceAlias"],
                    "language": row["language"],
                    "domainUri": row["domainUri"],
                    "payload": payload,
                }
            )
        sources = []
        if not isinstance(context["sources"], list):
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        for raw in context["sources"]:
            row = _closed_object(
                raw,
                {
                    "fileId", "contentRef", "startLine", "endLine", "text", "windows",
                    "completeFile", "memberAlias", "serviceAlias", "sessionId",
                    "sessionAuthorityDigest", "language", "contextId", "contextDigest",
                    "evidenceDigest", "compilations", "threadTruncated",
                },
            )
            if not isinstance(row["windows"], list):
                raise BrokerError("SEMANTIC_OUTPUT_INVALID")
            source_windows = []
            for raw_window in row["windows"]:
                window = _closed_object(raw_window, {"startLine", "endLine", "text"})
                source_windows.append(dict(window))
            sources.append(
                {
                    "memberAlias": row["memberAlias"],
                    "serviceAlias": row["serviceAlias"],
                    "fileId": row["fileId"],
                    "windows": source_windows,
                    "completeFile": row["completeFile"],
                    "threadTruncated": row["threadTruncated"],
                }
            )
        completeness = dict(
            _closed_object(
                context["completeness"],
                {"status", "support", "certainty", "coverage", "unmatchedTerms", "memberCount"},
            )
        )
        _closed_object(
            context["publicationPolicy"],
            {"mode", "status", "automaticPublication"},
        )
        obligations = []
        if not isinstance(context["verificationObligations"], list):
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        for raw in context["verificationObligations"]:
            wrapper = _closed_object(
                raw,
                {"memberAlias", "serviceAlias", "sessionId", "contextId", "obligation"},
            )
            obligation = _closed_object(
                wrapper["obligation"],
                {"id", "code", "subject", "requiredCheckSet", "publicationBlocking"},
            )
            obligations.append(
                {
                    "memberAlias": wrapper["memberAlias"],
                    "serviceAlias": wrapper["serviceAlias"],
                    "code": obligation["code"],
                    "subject": obligation["subject"],
                    "requiredCheckSet": obligation["requiredCheckSet"],
                    "publicationBlocking": obligation["publicationBlocking"],
                }
            )
        return {
            "schema": "codeclew-kotlin-pilot-semantic-context/1.0",
            "task": dict(task),
            "members": members,
            "matches": matches,
            "sources": sources,
            "completeness": completeness,
            "verificationObligations": obligations,
            "obligationCount": context["obligationCount"],
            "obligationsTruncated": context["obligationsTruncated"],
            "truncated": context["truncated"],
        }

    def _callables_model_projection(self, value: dict[str, Any]) -> dict[str, Any]:
        root = _closed_object(
            value,
            {
                "schema", "threadId", "threadAuthorityDigest", "contextId",
                "contextAuthorityDigest", "factSetId", "authorityDigest", "evidenceRef",
                "queryIndexRef", "callables",
            },
        )
        if root["schema"] != CALLABLE_RESULT_SCHEMA:
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        callables = _closed_object(
            root["callables"],
            {
                "schema", "factSetId", "authorityDigest", "bindingDigest", "threadId",
                "threadContextId", "tasks", "pairs", "members", "counts", "completeness",
                "queryIndexRef", "evidenceRef",
            },
        )
        if callables["schema"] != "codeclew-kotlin-callable-fact-set-projection/1.0":
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        tasks = []
        if not isinstance(callables["tasks"], list):
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        for raw in callables["tasks"]:
            row = _closed_object(raw, {"taskId", "pairId", "termCount", "termsDigest"})
            tasks.append(
                {key: row[key] for key in ("taskId", "pairId", "termCount")}
            )
        pairs = []
        if not isinstance(callables["pairs"], list):
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        for raw in callables["pairs"]:
            row = _closed_object(
                raw,
                {
                    "pairId", "providerMember", "consumerMember",
                    "relationshipAuthority", "dependencyEvidenceRef",
                },
            )
            pairs.append(
                {
                    key: row[key]
                    for key in (
                        "pairId", "providerMember", "consumerMember", "relationshipAuthority"
                    )
                }
            )
        members = []
        if not isinstance(callables["members"], list):
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        for raw in callables["members"]:
            row = _closed_object(
                raw,
                {"memberAlias", "serviceAlias", "repositoryNamespace", "compilations"},
            )
            members.append(
                {"memberAlias": row["memberAlias"], "serviceAlias": row["serviceAlias"]}
            )
        counts = dict(
            _closed_object(
                callables["counts"],
                {
                    "visitedInputFacts", "visitedInputPayloadBytes", "declarations", "uses",
                    "boundaries", "total", "exactDeclarations", "exactUses",
                },
            )
        )
        completeness = dict(
            _closed_object(
                callables["completeness"], {"coverage", "certainty", "obligationCount"}
            )
        )
        return {
            "schema": "codeclew-kotlin-pilot-semantic-callables/1.0",
            "tasks": tasks,
            "pairs": pairs,
            "members": members,
            "counts": counts,
            "completeness": completeness,
        }

    def _source_anchor_projection(self, value: Any) -> dict[str, Any]:
        anchor = _closed_object(value, {"path", "contentRef"}, {"start", "end"})
        if ("start" in anchor) != ("end" in anchor):
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        return {
            key: anchor[key]
            for key in ("path", "start", "end")
            if key in anchor
        }

    def _impact_finding_projection(self, value: Any) -> dict[str, Any]:
        finding = _closed_object(
            value,
            {"findingId", "side", "memberAlias", "factId", "authority", "detail"},
            {"shapeDigest", "source"},
        )
        detail = _closed_object(finding["detail"], {"kind", "detail"})
        kind = detail["kind"]
        if kind == "DECLARATION":
            body = _closed_object(
                detail["detail"],
                {"declarationKind", "symbolIdentity", "projectedShape"},
                {"compilerCallableId", "compilerClassId"},
            )
            projected_body = dict(body)
            projected_body["projectedShape"] = _model_safe_value(
                body["projectedShape"],
                forbidden_locators=self.model_forbidden_locators,
            )
        elif kind == "USE":
            body = _closed_object(
                detail["detail"],
                {"relationKind", "sourceOwner", "targetCallableId", "targetResolution"},
                {"targetSymbolIdentity", "targetRepositoryNamespace"},
            )
            projected_body = {
                key: body[key]
                for key in (
                    "relationKind", "sourceOwner", "targetCallableId",
                    "targetSymbolIdentity", "targetResolution",
                )
                if key in body
            }
        elif kind == "BOUNDARY":
            body = _closed_object(
                detail["detail"], {"stage", "code", "requiredChecks"}, {"subject"}
            )
            projected_body = dict(body)
        else:
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        projected = {
            "side": finding["side"],
            "memberAlias": finding["memberAlias"],
            "authority": finding["authority"],
            "detail": {"kind": kind, "detail": projected_body},
        }
        if "shapeDigest" in finding:
            projected["shapeDigest"] = finding["shapeDigest"]
        if "source" in finding:
            projected["source"] = self._source_anchor_projection(finding["source"])
        return projected

    def _impact_model_projection(self, value: dict[str, Any]) -> dict[str, Any]:
        root = _closed_object(
            value,
            {
                "schema", "threadId", "threadAuthorityDigest", "factSetId",
                "factSetAuthorityDigest", "impactId", "authorityDigest", "evidenceRef", "impact",
            },
        )
        if root["schema"] != IMPACT_RESULT_SCHEMA:
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        impact = _closed_object(
            root["impact"],
            {
                "schema", "impactId", "authorityDigest", "bindingDigest",
                "factSetAuthorityDigest", "pairId", "subjectKind", "relationshipAuthority",
                "shapeStatus", "certainty", "members", "findingCount", "sourceWindowCount",
                "obligationCount", "findingsTruncated", "sourceWindowsTruncated", "findings",
                "publicFindingsTruncated", "obligations", "sourceWindows", "evidenceRef",
            },
        )
        if impact["schema"] != "codeclew-kotlin-thread-impact-projection/1.0":
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        members = []
        if not isinstance(impact["members"], list):
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        for raw in impact["members"]:
            row = _closed_object(
                raw,
                {
                    "side", "memberAlias", "observed", "matchedFindingCount",
                    "selectedFindingCount", "declarationCount", "useCount", "boundaryCount",
                },
                {"exactShapeDigest"},
            )
            members.append(dict(row))
        if not isinstance(impact["findings"], list):
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        findings = [self._impact_finding_projection(row) for row in impact["findings"]]
        obligations = []
        if not isinstance(impact["obligations"], list):
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        for raw in impact["obligations"]:
            row = _closed_object(raw, {"code"}, {"memberAlias", "factId", "requiredCheck"})
            obligations.append(
                {key: row[key] for key in ("code", "memberAlias", "requiredCheck") if key in row}
            )
        source_windows = []
        if not isinstance(impact["sourceWindows"], list):
            raise BrokerError("SEMANTIC_OUTPUT_INVALID")
        for raw in impact["sourceWindows"]:
            row = _closed_object(
                raw,
                {"windowId", "side", "memberAlias", "anchor", "spanBytes", "findingIds"},
            )
            source_windows.append(
                {
                    "side": row["side"],
                    "memberAlias": row["memberAlias"],
                    "source": self._source_anchor_projection(row["anchor"]),
                    "spanBytes": row["spanBytes"],
                }
            )
        return {
            "schema": "codeclew-kotlin-pilot-semantic-impact/1.0",
            "pairId": impact["pairId"],
            "subjectKind": impact["subjectKind"],
            "relationshipAuthority": impact["relationshipAuthority"],
            "shapeStatus": impact["shapeStatus"],
            "certainty": impact["certainty"],
            "members": members,
            "findingCount": impact["findingCount"],
            "sourceWindowCount": impact["sourceWindowCount"],
            "obligationCount": impact["obligationCount"],
            "findingsTruncated": impact["findingsTruncated"],
            "sourceWindowsTruncated": impact["sourceWindowsTruncated"],
            "publicFindingsTruncated": impact["publicFindingsTruncated"],
            "findings": findings,
            "obligations": obligations,
            "sourceWindows": source_windows,
        }

    def _semantic(self, view: str, request: dict[str, Any]) -> dict[str, Any]:
        thread = self.task.get("thread")
        if not isinstance(thread, dict) or set(thread) != {
            "threadId", "threadAuthorityDigest", "providerMember", "consumerMember"
        }:
            raise BrokerError("SEMANTIC_AUTHORITY_NOT_PREPARED")
        if not self.clew.is_absolute() or not self.clew.is_file():
            raise BrokerError("SEMANTIC_AUTHORITY_NOT_PREPARED")
        command: list[str]
        terms: tuple[str, ...] = ()
        if view in {"context", "callables"}:
            if set(request) != {"schema", "operation", "terms"}:
                raise BrokerError("INVALID_BROKER_REQUEST")
            raw_terms = request["terms"]
            if not isinstance(raw_terms, list) or not raw_terms or len(raw_terms) > 16:
                raise BrokerError("INVALID_SEMANTIC_REQUEST")
            terms = tuple(raw_terms)
            if len(set(terms)) != len(terms) or any(
                not isinstance(term, str) or SAFE_TERM.fullmatch(term) is None for term in terms
            ):
                raise BrokerError("INVALID_SEMANTIC_REQUEST")
        if view == "context":
            if self.context_id is not None:
                raise BrokerError("SEMANTIC_CONTEXT_ALREADY_CREATED")
            command = [
                os.fspath(self.clew), "thread", "context",
                "--thread", thread["threadId"],
                "--intent", "frozen Kotlin descriptor pilot navigation",
                *[part for term in terms for part in ("--term", term)],
                "--max-roots", "2",
            ]
        elif view == "callables":
            if self.context_id is None or self.fact_set_id is not None:
                raise BrokerError("SEMANTIC_SEQUENCE_INVALID")
            command = [
                os.fspath(self.clew), "thread", "callables",
                "--thread", thread["threadId"], "--context", self.context_id,
                "--task-id", self.task["taskId"], "--pair-id", self.task["pairId"],
                "--provider", thread["providerMember"], "--consumer", thread["consumerMember"],
                *[part for term in terms for part in ("--term", term)],
            ]
        elif view == "impact":
            expected = {"schema", "operation", "subjectKind", "subject", "member"}
            if set(request) != expected or self.fact_set_id is None:
                raise BrokerError("SEMANTIC_SEQUENCE_INVALID")
            kind = request["subjectKind"]
            subject = request["subject"]
            member = request["member"]
            if kind not in {"full-symbol", "callable-family", "token"} or not isinstance(subject, str) or SAFE_SUBJECT.fullmatch(subject) is None:
                raise BrokerError("INVALID_SEMANTIC_REQUEST")
            if kind == "full-symbol":
                if member not in {thread["providerMember"], thread["consumerMember"]}:
                    raise BrokerError("INVALID_SEMANTIC_REQUEST")
            elif member is not None:
                raise BrokerError("INVALID_SEMANTIC_REQUEST")
            terms = (subject,)
            command = [
                os.fspath(self.clew), "thread", "impact",
                "--thread", thread["threadId"], "--fact-set", self.fact_set_id,
                "--pair-id", self.task["pairId"], "--subject-kind", kind,
                "--subject", subject,
            ]
            if member is not None:
                command.extend(["--member", member])
        else:
            raise BrokerError("UNKNOWN_BROKER_OPERATION")
        started = time.monotonic_ns()
        operation = f"semantic-{view}"
        millis = 0
        try:
            stdout, return_code = self._run_child(
                self._audited(command),
                environment=self.semantic_environment,
                timeout_seconds=self._limits()["singleSemanticCommandMs"] / 1000,
                maximum_stdout_bytes=MAX_TOOL_STDOUT_BYTES,
                kind="SEMANTIC",
                purpose=operation.upper().replace("-", "_"),
                failure_code="SEMANTIC_COMMAND_FAILED",
            )
            millis = max(
                1, (time.monotonic_ns() - started + 999_999) // 1_000_000
            )
            if return_code != 0 or not stdout or len(stdout) > MAX_TOOL_STDOUT_BYTES:
                raise BrokerError("SEMANTIC_COMMAND_FAILED")
            try:
                value = json.loads(stdout)
            except (json.JSONDecodeError, UnicodeDecodeError) as error:
                raise BrokerError("SEMANTIC_OUTPUT_INVALID") from error
            if not isinstance(value, dict):
                raise BrokerError("SEMANTIC_OUTPUT_INVALID")
            bindings = self._validate_semantic_bindings(
                view, value, thread, terms, request
            )
            model_value = self._semantic_model_projection(view, value)
            _, semantic_files, semantic_windows = self._semantic_accounting(
                view, value, thread
            )
            facts = _semantic_fact_count(model_value)
            response = self._commit_response(
                {"operation": operation, "result": model_value},
                terms=terms,
                files=tuple(semantic_files),
                windows=tuple(semantic_windows),
                returned_facts=facts,
                context_creates=1 if view == "context" else 0,
                semantic_millis=millis,
                semantic_view=view,
            )
            if view == "context":
                self.context_id = bindings["contextId"]
                self.context_authority_digest = bindings[
                    "contextAuthorityDigest"
                ]
            if view == "callables":
                self.fact_set_id = bindings["factSetId"]
                self.fact_set_authority_digest = bindings[
                    "factSetAuthorityDigest"
                ]
        except BrokerError as error:
            if millis == 0:
                millis = max(
                    1, (time.monotonic_ns() - started + 999_999) // 1_000_000
                )
            self.metrics.semantic_timing_ledger.append(
                {
                    "order": len(self.metrics.semantic_timing_ledger) + 1,
                    "operation": operation,
                    "elapsedMillis": millis,
                    "status": "REFUSED",
                    "refusalCode": error.code,
                }
            )
            raise
        self.metrics.semantic_timing_ledger.append(
            {
                "order": len(self.metrics.semantic_timing_ledger) + 1,
                "operation": operation,
                "elapsedMillis": millis,
                "status": "OK",
                "refusalCode": None,
            }
        )
        return response

    def _semantic_repository(
        self, thread: dict[str, Any], member: Any
    ) -> RepositoryBinding:
        if not isinstance(member, str):
            raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
        if member in {thread["providerMember"], self.task["provider"]}:
            return self.repositories[self.task["provider"]]
        if member in {thread["consumerMember"], self.task["consumer"]}:
            return self.repositories[self.task["consumer"]]
        raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")

    def _semantic_accounting(
        self, view: str, value: dict[str, Any], thread: dict[str, Any]
    ) -> tuple[int, set[SelectedFileProvenance], list[SourceWindowProvenance]]:
        files: set[SelectedFileProvenance] = set()
        windows: list[SourceWindowProvenance] = []
        window_keys: set[tuple[str, str, str, int, int]] = set()
        cache: dict[tuple[str, str], tuple[RepositoryBinding, str, bytes]] = {}

        def pinned(member: Any, path: Any) -> tuple[RepositoryBinding, str, str, bytes]:
            repository = self._semantic_repository(thread, member)
            relative = safe_relative_path(path, kotlin_only=True)
            key = (repository.alias, relative)
            if key not in cache:
                blob_oid, raw = self._file_bytes(repository, relative)
                cache[key] = (repository, blob_oid, raw)
            bound, blob_oid, raw = cache[key]
            selected = self._selected_file(bound, relative, blob_oid)
            files.add(selected)
            return bound, relative, blob_oid, raw

        def add_window(window: SourceWindowProvenance) -> None:
            key = (
                window.service_alias,
                window.relative_file,
                window.blob_oid,
                window.start_byte,
                window.end_byte,
            )
            if key not in window_keys:
                window_keys.add(key)
                windows.append(window)

        if view == "context":
            context = value.get("context")
            sources = context.get("sources") if isinstance(context, dict) else None
            if not isinstance(sources, list):
                raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
            for source in sources:
                if not isinstance(source, dict):
                    raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
                member = source.get("memberAlias")
                repository, relative, blob_oid, raw = pinned(
                    member, source.get("fileId")
                )
                expected_service = source.get("serviceAlias")
                if expected_service is not None and expected_service != repository.alias:
                    raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
                self._verify_content_ref(source.get("contentRef"), raw)
                source_windows = source.get("windows")
                if not isinstance(source_windows, list) or not source_windows:
                    raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
                previous_end = 0
                for row in source_windows:
                    if not isinstance(row, dict) or set(row) != {
                        "startLine", "endLine", "text"
                    }:
                        raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
                    start_line = row["startLine"]
                    end_line = row["endLine"]
                    text = row["text"]
                    if (
                        type(start_line) is not int
                        or type(end_line) is not int
                        or not isinstance(text, str)
                        or start_line <= previous_end
                    ):
                        raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
                    previous_end = end_line
                    add_window(
                        self._line_source_window(
                            repository,
                            relative,
                            blob_oid,
                            raw,
                            start_line,
                            end_line,
                            text,
                        )
                    )

        def visit(current: Any, member: str | None = None) -> None:
            if isinstance(current, dict):
                candidate_member = current.get("memberAlias")
                if isinstance(candidate_member, str):
                    member = candidate_member
                path = next(
                    (
                        current[key]
                        for key in ("path", "relativeFile", "fileId", "file")
                        if isinstance(current.get(key), str)
                    ),
                    None,
                )
                pinned_value: tuple[RepositoryBinding, str, str, bytes] | None = None
                if path is not None:
                    if member is None:
                        raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
                    pinned_value = pinned(member, path)
                reference = current.get("contentRef")
                # Only `SourceAnchor` uses the byte-range `path`/`contentRef`
                # pair. Context source rows use `fileId` plus line windows and
                # are accounted above, without charging an extra whole file.
                if reference is not None and isinstance(current.get("path"), str):
                    assert pinned_value is not None
                    repository, relative, blob_oid, raw = pinned_value
                    self._verify_content_ref(reference, raw)
                    start = current.get("start")
                    end = current.get("end")
                    if (start is None) != (end is None):
                        raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
                    if start is None:
                        start, end = 0, len(raw)
                    if type(start) is not int or type(end) is not int:
                        raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
                    try:
                        raw[:start].decode("utf-8")
                        raw[start:end].decode("utf-8")
                    except (UnicodeDecodeError, ValueError) as error:
                        raise BrokerError(
                            "SEMANTIC_SOURCE_PROVENANCE_INVALID"
                        ) from error
                    add_window(
                        self._source_window(
                            repository,
                            relative,
                            blob_oid,
                            raw,
                            start,
                            end,
                            0,
                        )
                    )
                anchor = current.get("anchor")
                if isinstance(anchor, dict) and "spanBytes" in current:
                    start = anchor.get("start")
                    end = anchor.get("end")
                    if start is None and end is None:
                        anchor_path = anchor.get("path")
                        if member is None:
                            raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
                        _, _, _, anchor_raw = pinned(member, anchor_path)
                        expected_span = len(anchor_raw)
                    elif type(start) is int and type(end) is int and end > start:
                        expected_span = end - start
                    else:
                        raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
                    if current["spanBytes"] != expected_span:
                        raise BrokerError("SEMANTIC_SOURCE_PROVENANCE_INVALID")
                for child in current.values():
                    visit(child, member)
            elif isinstance(current, list):
                for child in current:
                    visit(child, member)

        visit(value)
        facts = _semantic_fact_count(value)
        return facts, files, windows


def _semantic_fact_count(value: Any) -> int:
    facts = 0

    def visit(current: Any) -> None:
        nonlocal facts
        if isinstance(current, dict):
            for key, child in current.items():
                if key in {
                    "facts", "matches", "declarations", "uses", "boundaries",
                    "observations", "obligations", "verificationObligations", "findings",
                } and isinstance(child, list):
                    facts += len(child)
                visit(child)
        elif isinstance(current, list):
            for child in current:
                visit(child)

    visit(value)
    return facts


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


def _terminate_group(process: subprocess.Popen[Any]) -> bool:
    """Terminate the entire original session, even after its leader exited."""

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
            if process.poll() is None:
                try:
                    process.wait(timeout=0.25)
                except subprocess.TimeoutExpired:
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


def serve_directories(
    request_directory: Path,
    response_directory: Path,
    session: BrokerSession,
    capability_token: str,
    stop: threading.Event,
) -> None:
    """Serve atomic private request/response files inside the arm scratch."""

    if re.fullmatch(r"[0-9a-f]{64}", capability_token) is None:
        raise BrokerError("BROKER_CAPABILITY_INVALID")
    requests = _private_directory(os.fspath(request_directory))
    responses = _private_directory(os.fspath(response_directory))
    session.bind_stop_event(stop)
    last_sequence = 0
    seen_nonces: set[str] = set()
    try:
        while not stop.is_set():
            handled = False
            try:
                candidates = sorted(requests.iterdir(), key=lambda row: row.name)
            except OSError:
                if stop.is_set():
                    return
                raise
            for path in candidates:
                match = SAFE_REQUEST_NAME.fullmatch(path.name)
                if match is None:
                    continue
                handled = True
                sequence = int(match.group(1))
                nonce = match.group(2)
                metrics_before = session.metrics.projection()
                terms_before = set(session.metrics.query_terms)
                selected_before = set(session.metrics.selected_file_provenance)
                source_windows_before = len(session.metrics.source_window_ledger)
                timing_before = len(session.metrics.semantic_timing_ledger)
                violations_before = len(session.metrics.violation_ledger)
                capability_before = session.metrics.capability_violations
                request_value: Any = {}
                operation = "INVALID"
                try:
                    envelope = _read_private_message(path)
                    request_value = envelope.get("request")
                    if isinstance(request_value, dict) and isinstance(
                        request_value.get("operation"), str
                    ):
                        operation = request_value["operation"]
                    if (
                        set(envelope) != {"token", "nonce", "sequence", "request"}
                        or envelope["token"] != capability_token
                        or envelope["nonce"] != nonce
                        or envelope["sequence"] != sequence
                        or sequence <= last_sequence
                        or nonce in seen_nonces
                        or not isinstance(request_value, dict)
                    ):
                        raise BrokerError("BROKER_CAPABILITY_INVALID")
                    last_sequence = sequence
                    seen_nonces.add(nonce)
                    response = session.handle(request_value)
                except BrokerError as error:
                    if (
                        error.code != "BROKER_RESOURCE_BUDGET"
                        and session.metrics.capability_violations == capability_before
                    ):
                        session.metrics.record_violation(
                            error.code, operation, "BROKER_REFUSAL"
                        )
                    response = {
                        "schema": RESPONSE_SCHEMA,
                        "status": "REFUSED",
                        "code": error.code,
                    }
                try:
                    path.unlink()
                except OSError:
                    session.metrics.record_violation(
                        "BROKER_TRANSPORT_FAILED", operation, "REQUEST_UNLINK"
                    )
                    response = {
                        "schema": RESPONSE_SCHEMA,
                        "status": "REFUSED",
                        "code": "BROKER_TRANSPORT_FAILED",
                    }
                response_published = False
                try:
                    _write_private_message(responses / path.name, response)
                    response_published = True
                except BrokerError:
                    # A pre-existing response proves this request name was already served.
                    session.metrics.record_violation(
                        "BROKER_TRANSPORT_FAILED", operation, "RESPONSE_PUBLISH"
                    )
                    response = {
                        "schema": RESPONSE_SCHEMA,
                        "status": "REFUSED",
                        "code": "BROKER_TRANSPORT_FAILED",
                    }
                if response_published and response.get("status") == "REFUSED":
                    session.metrics.agent_visible_evidence_bytes += len(
                        canonical_bytes(response) + b"\n"
                    )
                metrics_after = session.metrics.projection()
                selected_added = sorted(
                    session.metrics.selected_file_provenance - selected_before
                )
                source_window_orders = list(
                    range(
                        source_windows_before + 1,
                        len(session.metrics.source_window_ledger) + 1,
                    )
                )
                timing_orders = list(
                    range(
                        timing_before + 1,
                        len(session.metrics.semantic_timing_ledger) + 1,
                    )
                )
                violation_orders = list(
                    range(
                        violations_before + 1,
                        len(session.metrics.violation_ledger) + 1,
                    )
                )
                request_digest = _sha256_digest(
                    canonical_bytes(request_value) + b"\n"
                )
                response_digest = _sha256_digest(canonical_bytes(response) + b"\n")
                session.metrics.ordered_tool_ledger.append(
                    {
                        "order": len(session.metrics.ordered_tool_ledger) + 1,
                        "operation": operation,
                        "requestDigest": request_digest,
                        "responseDigest": response_digest,
                        "status": response.get("status", "REFUSED"),
                        "refusalCode": response.get("code"),
                        "accountingDelta": {
                            "metrics": {
                                key: metrics_after[key] - metrics_before[key]
                                for key in sorted(metrics_after)
                            },
                            "queryTerms": sorted(
                                session.metrics.query_terms - terms_before
                            ),
                            "selectedFiles": [
                                row.projection() for row in selected_added
                            ],
                            "sourceWindowOrders": source_window_orders,
                            "semanticTimingOrders": timing_orders,
                            "violationOrders": violation_orders,
                        },
                    }
                )
            if not handled:
                stop.wait(0.01)
    finally:
        session.terminate_active_children()


def _client_request(arguments: argparse.Namespace) -> dict[str, Any]:
    request: dict[str, Any] = {"schema": REQUEST_SCHEMA, "operation": arguments.operation}
    if arguments.operation == "capability":
        return request
    if arguments.operation == "tree":
        request.update(member=arguments.member, prefix=arguments.prefix, limit=arguments.limit)
    elif arguments.operation == "show":
        request.update(member=arguments.member, file=arguments.file)
    elif arguments.operation == "search":
        request.update(member=arguments.member, term=arguments.term, limit=arguments.limit)
    elif arguments.operation == "read":
        request.update(
            member=arguments.member,
            file=arguments.file,
            startByte=arguments.start_byte,
            endByte=arguments.end_byte,
        )
    elif arguments.operation in {"semantic-context", "semantic-callables"}:
        request["terms"] = arguments.terms
    elif arguments.operation == "semantic-impact":
        request.update(
            subjectKind=arguments.subject_kind,
            subject=arguments.subject,
            member=arguments.member,
        )
    return request


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="operation", required=True)
    commands.add_parser("capability")
    tree = commands.add_parser("tree")
    tree.add_argument("--member", required=True)
    tree.add_argument("--prefix", default="")
    tree.add_argument("--limit", type=int, default=12)
    show = commands.add_parser("show")
    show.add_argument("--member", required=True)
    show.add_argument("--file", required=True)
    search = commands.add_parser("search")
    search.add_argument("--member", required=True)
    search.add_argument("--term", required=True)
    search.add_argument("--limit", type=int, default=20)
    read = commands.add_parser("read")
    read.add_argument("--member", required=True)
    read.add_argument("--file", required=True)
    read.add_argument("--start-byte", type=int, required=True)
    read.add_argument("--end-byte", type=int, required=True)
    for name in ["semantic-context", "semantic-callables"]:
        semantic = commands.add_parser(name)
        semantic.add_argument("--term", dest="terms", action="append", required=True)
    impact = commands.add_parser("semantic-impact")
    impact.add_argument(
        "--subject-kind", choices=["full-symbol", "callable-family", "token"], required=True
    )
    impact.add_argument("--subject", required=True)
    impact.add_argument("--member")
    return root


def main(argv: list[str] | None = None) -> int:
    arguments = parser().parse_args(argv)
    request_raw = os.environ.get("CODECLEW_PILOT_BROKER_REQUESTS")
    response_raw = os.environ.get("CODECLEW_PILOT_BROKER_RESPONSES")
    capability_token = os.environ.get("CODECLEW_PILOT_BROKER_TOKEN")
    if (
        request_raw is None
        or response_raw is None
        or capability_token is None
        or re.fullmatch(r"[0-9a-f]{64}", capability_token) is None
    ):
        print(canonical_bytes({"schema": RESPONSE_SCHEMA, "status": "REFUSED", "code": "BROKER_CAPABILITY_MISSING"}).decode())
        return 2
    request_path: Path | None = None
    response_path: Path | None = None
    try:
        requests = _private_directory(request_raw)
        responses = _private_directory(response_raw)
        nonce = secrets.token_hex(16)
        sequence = time.monotonic_ns()
        name = f"{sequence:020d}-{nonce}.json"
        request_path = requests / name
        response_path = responses / name
        _write_private_message(
            request_path,
            {
                "token": capability_token,
                "nonce": nonce,
                "sequence": sequence,
                "request": _client_request(arguments),
            },
        )
        deadline = time.monotonic() + 65
        while True:
            try:
                os.lstat(response_path)
            except FileNotFoundError:
                if time.monotonic() >= deadline:
                    raise BrokerError("BROKER_TRANSPORT_FAILED")
                time.sleep(0.01)
                continue
            response = _read_private_message(response_path)
            break
    except (OSError, BrokerError):
        print(canonical_bytes({"schema": RESPONSE_SCHEMA, "status": "REFUSED", "code": "BROKER_TRANSPORT_FAILED"}).decode())
        return 2
    finally:
        for path in [request_path, response_path]:
            if path is not None:
                try:
                    path.unlink()
                except OSError:
                    pass
    print(canonical_bytes(response).decode("utf-8"))
    return 0 if response.get("status") == "OK" else 1


if __name__ == "__main__":
    raise SystemExit(main())
