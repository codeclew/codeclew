#!/usr/bin/env python3
"""Produce the audited private 30-run S4K warm attestation on macOS."""

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
from pathlib import Path
from typing import Any, Callable

sys.path.insert(0, os.fspath(Path(__file__).resolve().parent))

import run_thread_kotlin_descriptor_gate as descriptor_gate
import run_thread_kotlin_pilot as pilot


ADAPTER = "MACOS_SEATBELT_V1"
PROFILE_POLICY = "GLOBAL_WRITE_DENY_STATE_LOCKS_ONLY_V1"
MEASUREMENT_CLASS = "MEASURED"
RESOURCE_LEDGER_SCHEMA = "codeclew-kotlin-warm-resource-ledger/2.0"
TERMS = ["publicDescriptor", "overloadedDescriptor", "genericDescriptor", "Envelope"]
SUBJECT = "com/acme/publicDescriptor"
MAX_STDOUT_BYTES = 64 * 1024
MAX_STATE_ENTRIES = 1_000_000
SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")


class WarmAuditError(RuntimeError):
    """A locator-free refusal from the audited warm executor."""

    def __init__(self, code: str):
        if re.fullmatch(r"[A-Z][A-Z0-9_]{0,63}", code) is None:
            code = "WARM_AUDIT_INTERNAL_FAILURE"
        super().__init__(code)
        self.code = code


def _run(
    command: list[str],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    timeout: int = 120,
    maximum: int = MAX_STDOUT_BYTES,
) -> bytes:
    try:
        process = subprocess.Popen(
            command,
            cwd=cwd,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            close_fds=True,
        )
    except OSError as error:
        raise WarmAuditError("WARM_SUBPROCESS_FAILED") from error
    stdout, stderr = _bounded_capture(
        process,
        timeout=timeout,
        stdout_limit=maximum,
        stderr_limit=MAX_STDOUT_BYTES,
        failure_code="WARM_SUBPROCESS_FAILED",
    )
    if process.returncode != 0 or stderr or len(stdout) > maximum:
        raise WarmAuditError("WARM_SUBPROCESS_FAILED")
    return stdout


def _run_json(
    command: list[str],
    timeout: int = 1800,
    env: dict[str, str] | None = None,
) -> dict[str, Any]:
    raw = _run(command, timeout=timeout, env=env)
    try:
        value = json.loads(raw, object_pairs_hook=pilot._duplicates)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise WarmAuditError("WARM_JSON_INVALID") from error
    if not isinstance(value, dict) or raw != pilot.canonical_bytes(value) + b"\n":
        raise WarmAuditError("WARM_JSON_INVALID")
    return value


def _terminate_process_group(process: subprocess.Popen[Any], failure_code: str) -> None:
    """Terminate and prove disappearance of the entire private process group."""

    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    except OSError as error:
        raise WarmAuditError(failure_code) from error
    if process.poll() is None:
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            pass
    deadline = time.monotonic() + 2.0
    while time.monotonic() < deadline and _process_group_exists(process.pid):
        time.sleep(0.02)
    if _process_group_exists(process.pid):
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except OSError as error:
            raise WarmAuditError(failure_code) from error
    if process.poll() is None:
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            raise WarmAuditError(failure_code)
    deadline = time.monotonic() + 2.0
    while time.monotonic() < deadline and _process_group_exists(process.pid):
        time.sleep(0.02)
    if _process_group_exists(process.pid):
        raise WarmAuditError(failure_code)


def _process_group_exists(process_group: int) -> bool:
    try:
        os.killpg(process_group, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def _bounded_capture(
    process: subprocess.Popen[Any],
    *,
    timeout: int,
    stdout_limit: int,
    stderr_limit: int,
    failure_code: str,
) -> tuple[bytes, bytes]:
    """Drain both pipes concurrently with hard byte/time/process-group bounds."""

    if process.stdout is None or process.stderr is None:
        _terminate_process_group(process, "SANDBOX_RESIDUAL_PROCESS_CLEANUP_FAILED")
        raise WarmAuditError(failure_code)
    streams = {process.stdout: bytearray(), process.stderr: bytearray()}
    limits = {process.stdout: stdout_limit, process.stderr: stderr_limit}
    selector = selectors.DefaultSelector()
    deadline = time.monotonic() + timeout
    primary: WarmAuditError | None = None
    try:
        for stream in streams:
            os.set_blocking(stream.fileno(), False)
            selector.register(stream, selectors.EVENT_READ)
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                primary = WarmAuditError(failure_code)
                break
            events = selector.select(min(remaining, 0.05))
            if not events and process.poll() is not None:
                # A final nonblocking read is still required: the leader can
                # exit after filling a pipe but before the selector wakes.
                events = [
                    (key, selectors.EVENT_READ)
                    for key in list(selector.get_map().values())
                ]
            for key, _ in events:
                stream = key.fileobj
                try:
                    chunk = os.read(stream.fileno(), 65_536)
                except BlockingIOError:
                    continue
                except OSError:
                    primary = WarmAuditError(failure_code)
                    break
                if not chunk:
                    selector.unregister(stream)
                    continue
                buffer = streams[stream]
                if len(buffer) + len(chunk) > limits[stream]:
                    primary = WarmAuditError(failure_code)
                    break
                buffer.extend(chunk)
            if process.poll() is not None and _process_group_exists(process.pid):
                _terminate_process_group(
                    process, "SANDBOX_RESIDUAL_PROCESS_CLEANUP_FAILED"
                )
                raise WarmAuditError("SANDBOX_RESIDUAL_PROCESS")
            if primary is not None:
                break
        if primary is None:
            remaining = max(0.0, deadline - time.monotonic())
            try:
                process.wait(timeout=remaining)
            except subprocess.TimeoutExpired:
                primary = WarmAuditError(failure_code)
        if primary is None and _process_group_exists(process.pid):
            _terminate_process_group(
                process, "SANDBOX_RESIDUAL_PROCESS_CLEANUP_FAILED"
            )
            raise WarmAuditError("SANDBOX_RESIDUAL_PROCESS")
        if primary is not None:
            _terminate_process_group(
                process, "SANDBOX_RESIDUAL_PROCESS_CLEANUP_FAILED"
            )
            raise primary
        return bytes(streams[process.stdout]), bytes(streams[process.stderr])
    except BaseException as error:
        try:
            _terminate_process_group(
                process, "SANDBOX_RESIDUAL_PROCESS_CLEANUP_FAILED"
            )
        except WarmAuditError as cleanup_error:
            raise cleanup_error from error
        raise
    finally:
        selector.close()
        process.stdout.close()
        process.stderr.close()


def _git_env() -> dict[str, str]:
    return {
        "PATH": "/usr/bin:/bin",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_CONFIG_SYSTEM": "/dev/null",
        "GIT_NO_REPLACE_OBJECTS": "1",
        "GIT_TERMINAL_PROMPT": "0",
        "LC_ALL": "C",
    }


def _safe_relative(value: str) -> Path:
    try:
        checked = descriptor_gate.safe_relative_kotlin_file(value) if value.endswith((".kt", ".kts")) else value
    except descriptor_gate.GateError as error:
        raise WarmAuditError("FIXTURE_TREE_INVALID") from error
    if (
        not isinstance(checked, str)
        or not checked
        or checked.startswith("/")
        or "\\" in checked
        or "\0" in checked
        or any(part in {"", ".", ".."} for part in checked.split("/"))
    ):
        raise WarmAuditError("FIXTURE_TREE_INVALID")
    return Path(checked)


def _copy_tracked_fixture(source: Path, destination: Path, git: Path) -> tuple[str, str]:
    head = _run(
        [os.fspath(git), "-C", os.fspath(source), "rev-parse", "--verify", "HEAD^{commit}"],
        env=_git_env(),
        maximum=128,
    ).decode("ascii").strip()
    if descriptor_gate.GIT_OID.fullmatch(head) is None:
        raise WarmAuditError("FIXTURE_TREE_INVALID")
    tree = _run(
        [os.fspath(git), "-C", os.fspath(source), "rev-parse", "--verify", "HEAD:fixtures/kotlin-basic"],
        env=_git_env(),
        maximum=128,
    ).decode("ascii").strip()
    if descriptor_gate.GIT_OID.fullmatch(tree) is None:
        raise WarmAuditError("FIXTURE_TREE_INVALID")
    listing = _run(
        [os.fspath(git), "-C", os.fspath(source), "ls-tree", "-r", "-z", tree],
        env=_git_env(),
        maximum=2 * 1024 * 1024,
    )
    entries = [entry for entry in listing.split(b"\0") if entry]
    if not entries or len(entries) > 128:
        raise WarmAuditError("FIXTURE_TREE_INVALID")
    destination.mkdir(mode=0o700)
    digest_rows: list[dict[str, Any]] = []
    for entry in entries:
        try:
            metadata, raw_name = entry.split(b"\t", 1)
            mode, kind, oid = metadata.decode("ascii").split(" ")
            name = raw_name.decode("utf-8")
        except (ValueError, UnicodeDecodeError) as error:
            raise WarmAuditError("FIXTURE_TREE_INVALID") from error
        if kind != "blob" or mode not in {"100644", "100755"} or descriptor_gate.GIT_BLOB_OID.fullmatch(oid) is None:
            raise WarmAuditError("FIXTURE_TREE_INVALID")
        relative = _safe_relative(name)
        target = destination / relative
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        body = _run(
            [os.fspath(git), "-C", os.fspath(source), "cat-file", "blob", oid],
            env=_git_env(),
            maximum=16 * 1024 * 1024,
        )
        digest_rows.append(
            {
                "mode": mode,
                "name": name,
                "size": len(body),
                "blobDigest": f"sha256:{hashlib.sha256(body).hexdigest()}",
            }
        )
        target.write_bytes(body)
        os.chmod(target, 0o755 if mode == "100755" else 0o600)
    digest_rows.sort(key=lambda row: row["name"])
    return tree, pilot.authority_digest(digest_rows)


def _commit_fixture(repository: Path, git: Path) -> str:
    env = _git_env()
    env.update(
        {
            "GIT_AUTHOR_NAME": "Codeclew Pilot",
            "GIT_COMMITTER_NAME": "Codeclew Pilot",
            "GIT_AUTHOR_EMAIL": "pilot" + "@" + "invalid",
            "GIT_COMMITTER_EMAIL": "pilot" + "@" + "invalid",
            "GIT_AUTHOR_DATE": "2000-01-01T00:00:00+0000",
            "GIT_COMMITTER_DATE": "2000-01-01T00:00:00+0000",
        }
    )
    _run([os.fspath(git), "init", "-q", "-b", "pilot"], cwd=repository, env=env)
    _run([os.fspath(git), "add", "--all"], cwd=repository, env=env)
    _run([os.fspath(git), "commit", "-q", "-m", "fixture"], cwd=repository, env=env)
    oid = _run(
        [os.fspath(git), "rev-parse", "--verify", "HEAD^{commit}"],
        cwd=repository,
        env=env,
        maximum=128,
    ).decode("ascii").strip()
    if descriptor_gate.GIT_OID.fullmatch(oid) is None:
        raise WarmAuditError("FIXTURE_COMMIT_INVALID")
    return oid


def _session(value: dict[str, Any], repository: Path, revision: str) -> dict[str, Any]:
    service = descriptor_gate.Service("service-01", "fixture", repository, revision)
    try:
        result = descriptor_gate.parse_session_open(value, service, "refs/heads/pilot")
    except descriptor_gate.GateError as error:
        raise WarmAuditError("FIXTURE_SESSION_INVALID") from error
    if result["runtimeMode"] != "RELEASE":
        raise WarmAuditError("WARM_RELEASE_RUNTIME_REQUIRED")
    return result


def _open_fixture_sessions(
    clew: Path,
    repositories: dict[str, Path],
    revisions: dict[str, str],
    sessions: dict[str, dict[str, Any]],
    environment: dict[str, str],
    expected_runtime_key: str,
    record_open_start: Callable[[str, str], None],
    record_session: Callable[[str], None],
) -> None:
    for alias in ["provider", "consumer", "observer"]:
        record_open_start(
            "SESSION",
            pilot.authority_digest(
                {
                    "alias": alias,
                    "revision": revisions[alias],
                    "targetRef": "refs/heads/pilot",
                    "language": "kotlin",
                    "compilation": descriptor_gate.COMPILATION,
                    "generationJobs": 1,
                }
            ),
        )
        value = _run_json(
            [
                os.fspath(clew), "session", "open",
                "--repo", os.fspath(repositories[alias]),
                "--target-ref", "refs/heads/pilot",
                "--language", "kotlin",
                "--compilation", descriptor_gate.COMPILATION,
                "--generation-jobs", "1",
            ],
            env=environment,
        )
        sessions[alias] = _session(value, repositories[alias], revisions[alias])
        record_session(sessions[alias]["sessionId"])
    keys = {row["runtimeKey"] for row in sessions.values()}
    if keys != {expected_runtime_key}:
        raise WarmAuditError("FIXTURE_RUNTIME_MISMATCH")


def _thread(
    clew: Path, sessions: dict[str, dict[str, Any]], environment: dict[str, str]
) -> str:
    command = [os.fspath(clew), "thread", "open"]
    for alias in ["provider", "consumer", "observer"]:
        command.extend(["--member", f"{alias}={sessions[alias]['sessionId']}"])
        command.extend(["--service-alias", f"{alias}={alias}"])
    value = _run_json(command, timeout=300, env=environment)
    try:
        return pilot._parse_thread_open(value)
    except pilot.PilotError as error:
        raise WarmAuditError("FIXTURE_THREAD_INVALID") from error


def _context_and_fact_set(
    clew: Path, thread_id: str, environment: dict[str, str]
) -> tuple[str, str]:
    context = _run_json(
        [
            os.fspath(clew), "thread", "context",
            "--thread", thread_id,
            "--intent", "warm Kotlin descriptor navigation audit",
            *[part for term in TERMS for part in ("--term", term)],
            "--max-roots", "2",
        ],
        timeout=300,
        env=environment,
    )
    context_id = context.get("contextId")
    if not isinstance(context_id, str) or not context_id.startswith("thread-context:sha256:"):
        raise WarmAuditError("FIXTURE_CONTEXT_INVALID")
    callables = _run_json(
        [
            os.fspath(clew), "thread", "callables",
            "--thread", thread_id,
            "--context", context_id,
            "--task-id", "task-warm",
            "--pair-id", "pair-warm",
            "--provider", "provider",
            "--consumer", "consumer",
            *[part for term in TERMS for part in ("--term", term)],
        ],
        timeout=300,
        env=environment,
    )
    fact_set = callables.get("factSetId")
    if not isinstance(fact_set, str) or not fact_set.startswith("thread-callables:sha256:"):
        raise WarmAuditError("FIXTURE_FACT_SET_INVALID")
    return context_id, fact_set


def _impact_command(clew: Path, thread_id: str, fact_set: str, subject: str) -> list[str]:
    return [
        os.fspath(clew), "thread", "impact",
        "--thread", thread_id,
        "--fact-set", fact_set,
        "--pair-id", "pair-warm",
        "--subject-kind", "callable-family" if subject == SUBJECT else "token",
        "--subject", subject,
    ]


IMPACT_OBLIGATION_CODES = {
    "SUBJECT_NOT_OBSERVED_IN_MEMBER",
    "PROJECTED_DECLARATION_NOT_OBSERVED",
    "DISAMBIGUATE_OVERLOAD_SET",
    "COMPLETE_DESCRIPTOR_SCOPE",
    "VERIFY_DECLARATION_EVIDENCE",
    "VERIFY_USE_EVIDENCE",
    "VERIFY_RELATED_BOUNDARY",
    "VERIFY_BOUNDARY_CHECK",
    "VERIFY_FACT_SET_BOUNDARY_SCOPE",
    "VERIFY_RELATIONSHIP_AUTHORITY",
    "RESOLVE_NAVIGATION_SUBJECT",
    "NARROW_OR_EXPAND_QUERY",
}
MEMBER_SIDES = {"provider": "PROVIDER", "consumer": "CONSUMER"}


def _impact_digest(value: Any) -> str:
    if not isinstance(value, str) or SHA256.fullmatch(value) is None:
        raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    return value


def _impact_cas(value: Any, object_schema: str | None = None) -> dict[str, Any]:
    if (
        not isinstance(value, dict)
        or set(value) != {"schema", "objectSchema", "digest", "size"}
        or value.get("schema") != "codeclew-cas-object/2.0"
        or not isinstance(value.get("objectSchema"), str)
        or not value["objectSchema"]
        or (object_schema is not None and value["objectSchema"] != object_schema)
        or type(value.get("size")) is not int
        or value["size"] <= 0
    ):
        raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    _impact_digest(value.get("digest"))
    return value


def _impact_source(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    fields = set(value)
    if fields not in (
        {"path", "contentRef"},
        {"path", "start", "end", "contentRef"},
    ):
        raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    if not isinstance(value.get("path"), str):
        raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    try:
        descriptor_gate.safe_relative_kotlin_file(value.get("path"))
    except descriptor_gate.GateError as error:
        raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID") from error
    if ("start" in value) and (
        type(value["start"]) is not int
        or type(value["end"]) is not int
        or not 0 <= value["start"] < value["end"]
    ):
        raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    _impact_cas(value.get("contentRef"), "codeclew-repository-input-blob/2.0")
    return value


def _impact_detail(value: Any, finding: dict[str, Any]) -> tuple[str, dict[str, Any]]:
    if (
        not isinstance(value, dict)
        or set(value) != {"kind", "detail"}
        or value.get("kind") not in {"DECLARATION", "USE", "BOUNDARY"}
        or not isinstance(value.get("detail"), dict)
    ):
        raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    kind = value["kind"]
    detail = value["detail"]
    if kind == "DECLARATION":
        declaration_kind = detail.get("declarationKind")
        fields = {"declarationKind", "symbolIdentity", "projectedShape"}
        if not isinstance(declaration_kind, str):
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
        if declaration_kind == "CLASS":
            fields.add("compilerClassId")
        elif declaration_kind in {"FUNCTION", "PROPERTY", "MUTABLE_PROPERTY"}:
            fields.add("compilerCallableId")
        elif declaration_kind == "CONSTRUCTOR":
            fields.update({"compilerCallableId", "compilerClassId"})
        else:
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
        projected = detail.get("projectedShape")
        if (
            set(detail) != fields
            or not isinstance(detail.get("symbolIdentity"), str)
            or not detail["symbolIdentity"]
            or not isinstance(projected, dict)
            or projected.get("declarationKind") != declaration_kind
            or projected.get("symbolIdentity") != detail["symbolIdentity"]
            or projected.get("compilerCallableId") != detail.get("compilerCallableId")
            or projected.get("compilerClassId") != detail.get("compilerClassId")
        ):
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
        shape_digest = finding.get("shapeDigest")
        if "shapeDigest" in finding:
            _impact_digest(shape_digest)
            if pilot.authority_digest(projected) != shape_digest:
                raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
        if finding.get("authority") == "EXACT_PROJECTED_DECLARATION" and (
            shape_digest is None or "source" not in finding
        ):
            raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
    elif kind == "USE":
        fields = {
            "relationKind", "sourceOwner", "targetCallableId", "targetResolution"
        }
        optional = {"targetSymbolIdentity", "targetRepositoryNamespace"}
        if (
            not fields <= set(detail) <= fields | optional
            or detail.get("targetResolution") not in {"EXACT_SYMBOL", "CALLABLE_FAMILY"}
            or any(
                not isinstance(detail.get(key), str) or not detail[key]
                for key in fields - {"targetResolution"}
            )
            or any(
                not isinstance(detail[key], str) or not detail[key]
                for key in optional & set(detail)
            )
            or "shapeDigest" in finding
        ):
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    else:
        fields = {"stage", "code", "requiredChecks"}
        if "subject" in detail:
            fields.add("subject")
        checks = detail.get("requiredChecks")
        if (
            set(detail) != fields
            or any(
                not isinstance(detail.get(key), str) or not detail[key]
                for key in fields - {"requiredChecks"}
            )
            or not isinstance(checks, list)
            or len(checks) > 4096
            or any(not isinstance(check, str) or not check for check in checks)
            or len(checks) != len(set(checks))
            or "shapeDigest" in finding
        ):
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    return kind, detail


def _validate_warm_impact_result(
    value: Any,
    *,
    thread_id: str,
    fact_set: str,
    subject: str,
    expected_descriptor: dict[str, Any],
) -> dict[str, Any]:
    top_fields = {
        "schema", "threadId", "threadAuthorityDigest", "factSetId",
        "factSetAuthorityDigest", "impactId", "authorityDigest", "evidenceRef",
        "impact",
    }
    projection_fields = {
        "schema", "impactId", "authorityDigest", "bindingDigest",
        "factSetAuthorityDigest", "pairId", "subjectKind",
        "relationshipAuthority", "shapeStatus", "certainty", "members",
        "findingCount", "sourceWindowCount", "obligationCount",
        "findingsTruncated", "sourceWindowsTruncated", "findings",
        "publicFindingsTruncated", "obligations", "sourceWindows", "evidenceRef",
    }
    if not isinstance(value, dict) or set(value) != top_fields:
        raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    projection = value.get("impact")
    if not isinstance(projection, dict) or set(projection) != projection_fields:
        raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    evidence = value.get("evidenceRef")
    _impact_cas(evidence, "codeclew-kotlin-thread-impact-evidence/1.0")
    if projection.get("evidenceRef") != evidence:
        raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
    expected_subject_kind = "CALLABLE_FAMILY" if subject == SUBJECT else "TOKEN"
    authority = value.get("authorityDigest")
    impact_id = value.get("impactId")
    if (
        value.get("schema") != "codeclew-thread-impact-result/1.0"
        or value.get("threadId") != thread_id
        or value.get("factSetId") != fact_set
        or not isinstance(value.get("threadAuthorityDigest"), str)
        or SHA256.fullmatch(value["threadAuthorityDigest"]) is None
        or not isinstance(value.get("factSetAuthorityDigest"), str)
        or SHA256.fullmatch(value["factSetAuthorityDigest"]) is None
        or not isinstance(authority, str)
        or SHA256.fullmatch(authority) is None
        or impact_id != f"thread-impact:{authority}"
        or projection.get("schema") != "codeclew-kotlin-thread-impact-projection/1.0"
        or projection.get("impactId") != impact_id
        or projection.get("authorityDigest") != authority
        or projection.get("factSetAuthorityDigest") != value["factSetAuthorityDigest"]
        or not isinstance(projection.get("bindingDigest"), str)
        or SHA256.fullmatch(projection["bindingDigest"]) is None
        or projection.get("pairId") != "pair-warm"
        or projection.get("subjectKind") != expected_subject_kind
    ):
        raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
    for field in ("members", "findings", "obligations", "sourceWindows"):
        if not isinstance(projection.get(field), list):
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    for field in ("findingCount", "sourceWindowCount", "obligationCount"):
        if type(projection.get(field)) is not int or projection[field] < 0:
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    for field in ("findingsTruncated", "sourceWindowsTruncated", "publicFindingsTruncated"):
        if type(projection.get(field)) is not bool:
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    if (
        projection["findingCount"] != len(projection["findings"])
        or projection["sourceWindowCount"] != len(projection["sourceWindows"])
        or projection["obligationCount"] != len(projection["obligations"])
        or projection["findingsTruncated"] is not False
        or projection["sourceWindowsTruncated"] is not False
        or projection["publicFindingsTruncated"] is not False
    ):
        raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
    if (
        projection.get("relationshipAuthority") != "DECLARED_TOPOLOGY"
        or projection.get("shapeStatus") != "EXACT_PROJECTED_SHAPE_EQUAL"
        or projection.get("certainty") != "UNSURE"
    ):
        raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")

    members = projection["members"]
    if len(members) != 2:
        raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
    member_rows: dict[str, dict[str, Any]] = {}
    for member in members:
        required = {
            "side", "memberAlias", "observed", "matchedFindingCount",
            "selectedFindingCount", "declarationCount", "useCount", "boundaryCount",
        }
        if not isinstance(member, dict) or not required <= set(member) <= required | {"exactShapeDigest"}:
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
        alias = member.get("memberAlias")
        if (
            not isinstance(alias, str)
            or alias not in MEMBER_SIDES
            or member.get("side") != MEMBER_SIDES[alias]
            or alias in member_rows
        ):
            raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
        for field in (
            "matchedFindingCount", "selectedFindingCount", "declarationCount",
            "useCount", "boundaryCount",
        ):
            if type(member.get(field)) is not int or member[field] < 0:
                raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
        if type(member.get("observed")) is not bool:
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
        exact_shape = member.get("exactShapeDigest")
        if "exactShapeDigest" in member:
            _impact_digest(exact_shape)
        member_rows[alias] = member
    if set(member_rows) != set(MEMBER_SIDES):
        raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")

    findings_by_id: dict[str, dict[str, Any]] = {}
    fact_ids: set[str] = set()
    kind_counts = {
        alias: {"DECLARATION": 0, "USE": 0, "BOUNDARY": 0}
        for alias in MEMBER_SIDES
    }
    target_findings: dict[str, dict[str, Any]] = {}
    for finding in projection["findings"]:
        required = {"findingId", "side", "memberAlias", "factId", "authority", "detail"}
        optional = {"shapeDigest", "source"}
        if (
            not isinstance(finding, dict)
            or not required <= set(finding) <= required | optional
            or not isinstance(finding.get("authority"), str)
            or finding.get("authority")
            not in {"EXACT_PROJECTED_DECLARATION", "NAVIGATION_ONLY", "UNSURE"}
        ):
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
        finding_id = _impact_digest(finding.get("findingId"))
        fact_id = _impact_digest(finding.get("factId"))
        alias = finding.get("memberAlias")
        if (
            not isinstance(alias, str)
            or alias not in MEMBER_SIDES
            or finding.get("side") != MEMBER_SIDES[alias]
            or finding_id in findings_by_id
            or fact_id in fact_ids
        ):
            raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
        if "source" in finding:
            _impact_source(finding["source"])
        kind, detail = _impact_detail(finding["detail"], finding)
        if finding["authority"] == "EXACT_PROJECTED_DECLARATION" and kind != "DECLARATION":
            raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
        findings_by_id[finding_id] = finding
        fact_ids.add(fact_id)
        kind_counts[alias][kind] += 1
        if kind == "DECLARATION":
            projected = detail["projectedShape"]
            if _name(projected, detail["declarationKind"]) == "publicDescriptor":
                if (
                    finding["authority"] != "EXACT_PROJECTED_DECLARATION"
                    or alias in target_findings
                ):
                    raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
                target_findings[alias] = finding

    for alias, member in member_rows.items():
        counts = kind_counts[alias]
        total = sum(counts.values())
        if (
            member["selectedFindingCount"] != total
            or member["matchedFindingCount"] != total
            or member["declarationCount"] != counts["DECLARATION"]
            or member["useCount"] != counts["USE"]
            or member["boundaryCount"] != counts["BOUNDARY"]
            or member["observed"] != (total > 0)
        ):
            raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")

    if (
        not isinstance(expected_descriptor, dict)
        or expected_descriptor.get("descriptorClass") != "CALLABLE"
        or expected_descriptor.get("declarationKind") != "FUNCTION"
        or expected_descriptor.get("name") != "publicDescriptor"
        or set(target_findings) != set(MEMBER_SIDES)
    ):
        raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
    expected_range = expected_descriptor.get("sourceRange")
    if not isinstance(expected_range, dict):
        raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
    target_sources: list[dict[str, Any]] = []
    for alias, finding in target_findings.items():
        detail = finding["detail"]["detail"]
        projected = detail["projectedShape"]
        source = finding["source"]
        if (
            finding.get("shapeDigest") != expected_descriptor.get("shapeDigest")
            or detail.get("declarationKind") != expected_descriptor["declarationKind"]
            or detail.get("symbolIdentity") != expected_descriptor.get("normalizedSignature")
            or projected.get("ownerIdentity") != expected_descriptor.get("ownerIdentity")
            or source.get("path") != expected_descriptor.get("relativeFile")
            or source.get("start") != expected_range.get("startByte")
            or source.get("end") != expected_range.get("endByte")
            or member_rows[alias].get("exactShapeDigest") != expected_descriptor.get("shapeDigest")
        ):
            raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
        target_sources.append(source)
    if target_sources[0] != target_sources[1]:
        raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")

    obligations_seen: set[str] = set()
    relationship_obligation = False
    for obligation in projection["obligations"]:
        if not isinstance(obligation, dict) or not {"code"} <= set(obligation) <= {
            "code", "memberAlias", "factId", "requiredCheck"
        }:
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
        code = obligation.get("code")
        if not isinstance(code, str) or code not in IMPACT_OBLIGATION_CODES:
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
        alias = obligation.get("memberAlias")
        if "memberAlias" in obligation and (
            not isinstance(alias, str) or alias not in MEMBER_SIDES
        ):
            raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
        fact_id = obligation.get("factId")
        if "factId" in obligation and (_impact_digest(fact_id) not in fact_ids):
            raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
        required_check = obligation.get("requiredCheck")
        if "requiredCheck" in obligation and (
            not isinstance(required_check, str) or not required_check
        ):
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
        obligation_digest = pilot.authority_digest(obligation)
        if obligation_digest in obligations_seen:
            raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
        obligations_seen.add(obligation_digest)
        relationship_obligation |= code == "VERIFY_RELATIONSHIP_AUTHORITY"
    if not relationship_obligation:
        raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")

    sourced_findings = {
        finding_id for finding_id, finding in findings_by_id.items() if "source" in finding
    }
    window_findings: set[str] = set()
    window_ids: set[str] = set()
    for window in projection["sourceWindows"]:
        if not isinstance(window, dict) or set(window) != {
            "windowId", "side", "memberAlias", "anchor", "spanBytes", "findingIds"
        }:
            raise WarmAuditError("MEASURED_OUTPUT_SCHEMA_INVALID")
        window_id = _impact_digest(window.get("windowId"))
        alias = window.get("memberAlias")
        anchor = _impact_source(window.get("anchor"))
        finding_ids = window.get("findingIds")
        if (
            window_id in window_ids
            or not isinstance(alias, str)
            or alias not in MEMBER_SIDES
            or window.get("side") != MEMBER_SIDES[alias]
            or type(window.get("spanBytes")) is not int
            or window["spanBytes"] <= 0
            or not isinstance(finding_ids, list)
            or not finding_ids
            or any(not isinstance(finding_id, str) for finding_id in finding_ids)
            or len(finding_ids) != len(set(finding_ids))
        ):
            raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
        expected_span = (
            anchor["end"] - anchor["start"]
            if "start" in anchor
            else anchor["contentRef"]["size"]
        )
        if window["spanBytes"] != expected_span:
            raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
        for finding_id in finding_ids:
            _impact_digest(finding_id)
            finding = findings_by_id.get(finding_id)
            if (
                finding is None
                or finding.get("memberAlias") != alias
                or finding.get("side") != window["side"]
                or finding.get("source") != anchor
                or finding_id in window_findings
            ):
                raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
            window_findings.add(finding_id)
        window_ids.add(window_id)
    if window_findings != sourced_findings:
        raise WarmAuditError("MEASURED_OUTPUT_BINDING_INVALID")
    return value


def _name(projected: dict[str, Any], declaration_kind: str) -> str:
    key = "compilerClassId" if declaration_kind == "CLASS" else "compilerCallableId"
    value = projected.get(key)
    if not isinstance(value, str) or not value:
        raise WarmAuditError("FIXTURE_SHAPE_INVALID")
    return re.split(r"[/.]", value)[-1]


def _exact_fixture_rows(
    impacts: list[dict[str, Any]], repository: Path, revision: str, git: Path
) -> list[dict[str, Any]]:
    rows: dict[str, dict[str, Any]] = {}
    for value in impacts:
        projection = value.get("impact")
        findings = projection.get("findings") if isinstance(projection, dict) else None
        if not isinstance(findings, list):
            raise WarmAuditError("FIXTURE_SHAPE_INVALID")
        for finding in findings:
            if (
                not isinstance(finding, dict)
                or finding.get("memberAlias") != "provider"
                or finding.get("authority") != "EXACT_PROJECTED_DECLARATION"
            ):
                continue
            detail_outer = finding.get("detail")
            source = finding.get("source")
            if (
                not isinstance(detail_outer, dict)
                or detail_outer.get("kind") != "DECLARATION"
                or not isinstance(detail_outer.get("detail"), dict)
                or not isinstance(source, dict)
            ):
                raise WarmAuditError("FIXTURE_SHAPE_INVALID")
            detail = detail_outer["detail"]
            projected = detail.get("projectedShape")
            kind = detail.get("declarationKind")
            signature = detail.get("symbolIdentity")
            path = source.get("path")
            start = source.get("start")
            end = source.get("end")
            shape_digest = finding.get("shapeDigest")
            if (
                not isinstance(projected, dict)
                or kind not in {"FUNCTION", "CONSTRUCTOR", "CLASS", "PROPERTY", "MUTABLE_PROPERTY"}
                or not isinstance(signature, str)
                or not isinstance(path, str)
                or type(start) is not int
                or type(end) is not int
                or not 0 <= start < end
                or not isinstance(shape_digest, str)
                or SHA256.fullmatch(shape_digest) is None
            ):
                raise WarmAuditError("FIXTURE_SHAPE_INVALID")
            compiler_identity_key = "compilerClassId" if kind == "CLASS" else "compilerCallableId"
            if (
                detail.get(compiler_identity_key) != projected.get(compiler_identity_key)
                or detail.get("symbolIdentity") != projected.get("symbolIdentity")
                or kind != projected.get("declarationKind")
                or pilot.authority_digest(projected) != shape_digest
            ):
                raise WarmAuditError("FIXTURE_SHAPE_INVALID")
            relative = descriptor_gate.safe_relative_kotlin_file(path)
            blob_oid = _run(
                [
                    os.fspath(git), "-C", os.fspath(repository), "rev-parse",
                    "--verify", f"{revision}:{relative}",
                ],
                env=_git_env(),
                maximum=128,
            ).decode("ascii").strip()
            row = {
                "descriptorClass": "TYPE" if kind == "CLASS" else "CALLABLE",
                "declarationKind": kind,
                "name": _name(projected, kind),
                "ownerIdentity": projected.get("ownerIdentity"),
                "normalizedSignature": signature,
                "shapeDigest": shape_digest,
                "relativeFile": relative,
                "blobOid": blob_oid,
                "sourceRange": {"startByte": start, "endByte": end},
            }
            try:
                pilot._validate_exact_declaration(row, private=True)
            except pilot.PilotError as error:
                raise WarmAuditError("FIXTURE_SHAPE_INVALID") from error
            rows[pilot.authority_digest(row)] = row
    return [rows[key] for key in sorted(rows)]


def _fixture_identity(row: dict[str, Any]) -> tuple[Any, ...]:
    source = row.get("sourceRange")
    start = source.get("startByte") if isinstance(source, dict) else None
    end = source.get("endByte") if isinstance(source, dict) else None
    return (
        row.get("descriptorClass"),
        row.get("declarationKind"),
        row.get("name"),
        row.get("ownerIdentity"),
        row.get("relativeFile"),
        start,
        end,
    )


def _fixture_comparison(
    actual: list[dict[str, Any]], expected: list[dict[str, Any]]
) -> tuple[int, int]:
    identities = {_fixture_identity(row) for row in expected}
    if len(expected) != 5 or len(identities) != 5:
        raise WarmAuditError("FIXTURE_CONTOUR_INVALID")
    target_set = {
        (row.get("descriptorClass"), row.get("declarationKind"), row.get("name"))
        for row in expected
    }
    actual_set = {
        pilot.authority_digest(row)
        for row in actual
        if (row.get("descriptorClass"), row.get("declarationKind"), row.get("name"))
        in target_set
    }
    expected_set = {pilot.authority_digest(row) for row in expected}
    return len(actual_set & expected_set), len(actual_set - expected_set)


def _state_root(environment: dict[str, str]) -> Path:
    explicit = environment.get("CODECLEW_HOME")
    if explicit:
        root = Path(explicit)
    elif environment.get("XDG_CACHE_HOME"):
        root = Path(environment["XDG_CACHE_HOME"]) / "codeclew"
    else:
        home = environment.get("HOME")
        if home is None:
            raise WarmAuditError("STATE_AUTHORITY_UNAVAILABLE")
        root = Path(home) / ".cache" / "codeclew"
    if not root.is_absolute():
        raise WarmAuditError("STATE_AUTHORITY_UNAVAILABLE")
    try:
        resolved = (root / "v2").resolve(strict=True)
    except OSError as error:
        raise WarmAuditError("STATE_AUTHORITY_UNAVAILABLE") from error
    return resolved


def _state_snapshot(root: Path) -> str:
    digest = hashlib.sha256()
    entry_count = 0

    def record(path: Path, name: str) -> None:
        nonlocal entry_count
        metadata = os.lstat(path)
        row: dict[str, Any] = {
            "name": name,
            "device": metadata.st_dev,
            "inode": metadata.st_ino,
            "mode": stat.S_IFMT(metadata.st_mode) | stat.S_IMODE(metadata.st_mode),
            "size": metadata.st_size,
            "modifiedNanos": metadata.st_mtime_ns,
            "changedNanos": metadata.st_ctime_ns,
        }
        if stat.S_ISLNK(metadata.st_mode):
            row["linkDigest"] = pilot.authority_digest(os.readlink(path))
        entry_count += 1
        if entry_count > MAX_STATE_ENTRIES:
            raise WarmAuditError("STATE_SNAPSHOT_LIMIT")
        digest.update(pilot.canonical_bytes(row))
        digest.update(b"\n")

    record(root, ".")
    for current, directories, files in os.walk(root, followlinks=False):
        directories.sort()
        files.sort()
        relative_root = Path(current).relative_to(root)
        if relative_root.parts[:1] == ("locks",):
            directories[:] = []
            continue
        if not relative_root.parts:
            directories[:] = [name for name in directories if name != "locks"]
        for name in sorted(directories + files):
            path = Path(current) / name
            relative = path.relative_to(root).as_posix()
            if relative.split("/", 1)[0] == "locks":
                continue
            record(path, relative)
    return f"sha256:{digest.hexdigest()}"


def _sb(value: Path) -> str:
    raw = os.fspath(value)
    if not raw.startswith("/") or "\n" in raw or "\r" in raw or "\0" in raw:
        raise WarmAuditError("SANDBOX_PATH_INVALID")
    return raw.replace("\\", "\\\\").replace('"', '\\"')


def _python(authority: dict[str, Any]) -> tuple[Path, Path]:
    link = Path(authority["executables"]["python"])
    try:
        if not link.is_absolute():
            raise OSError
        resolved = link.resolve(strict=True)
    except OSError as error:
        raise WarmAuditError("PYTHON_AUTHORITY_UNAVAILABLE") from error
    return link, resolved


def _profile(
    clew: Path,
    capsule: Path,
    fixture_repositories: list[Path],
    state_root: Path,
    python: tuple[Path, Path],
    cache_roots: list[tuple[str, Path]],
) -> str:
    python_link, python_resolved = python
    shell_link = Path("/bin/sh")
    try:
        shell_resolved = shell_link.resolve(strict=True)
    except OSError as error:
        raise WarmAuditError("SANDBOX_SHELL_UNAVAILABLE") from error
    allowed = {
        clew.resolve(strict=True),
        capsule.resolve(strict=True),
        shell_link,
        shell_resolved,
        Path("/usr/bin/dirname"),
        python_link,
        python_resolved,
    }
    # On macOS, execve("/bin/sh", ...) asks Seatbelt to execute /bin/bash as
    # the selected sh variant even though /bin/sh is not a filesystem symlink.
    bash_variant = Path("/bin/bash")
    if bash_variant.is_file() and os.access(bash_variant, os.X_OK):
        allowed.add(bash_variant.resolve(strict=True))
    denied = {
        *[repository.resolve(strict=True) for repository in fixture_repositories],
        *[path for _, path in cache_roots],
        clew.parent / ".gradle",
        clew.parent / "target",
        clew.parent / ".semantic-thread",
    }
    process_rules = " ".join(f'(literal "{_sb(path)}")' for path in sorted(allowed, key=os.fspath))
    read_rules = "\n".join(f'(deny file-read* (subpath "{_sb(path)}"))' for path in sorted(denied, key=os.fspath))
    return (
        "(version 1)\n"
        "(allow default)\n"
        "(deny network*)\n"
        "(deny process-exec)\n"
        f"(allow process-exec {process_rules})\n"
        "(deny file-write*)\n"
        f'(allow file-write* (subpath "{_sb(state_root / "locks")}"))\n'
        f"{read_rules}\n"
    )


def _sandbox(
    sandbox_exec: Path,
    profile: str,
    command: list[str],
    *,
    env: dict[str, str],
    timeout: int = 60,
) -> tuple[int, bytes, bytes, int]:
    started = time.monotonic_ns()
    try:
        process = subprocess.Popen(
            [os.fspath(sandbox_exec), "-p", profile, *command],
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            close_fds=True,
        )
    except OSError as error:
        raise WarmAuditError("SANDBOX_EXECUTION_FAILED") from error
    stdout, stderr = _bounded_capture(
        process,
        timeout=timeout,
        stdout_limit=MAX_STDOUT_BYTES,
        stderr_limit=MAX_STDOUT_BYTES,
        failure_code="SANDBOX_EXECUTION_FAILED",
    )
    elapsed = max(1, time.monotonic_ns() - started)
    if len(stdout) > MAX_STDOUT_BYTES or len(stderr) > MAX_STDOUT_BYTES:
        raise WarmAuditError("SANDBOX_OUTPUT_LIMIT")
    return process.returncode, stdout, stderr, elapsed


def _canaries(
    sandbox_exec: Path,
    profile: str,
    cache_files: list[Path],
    source_fixture_file: Path,
    write_file: Path,
    python: tuple[Path, Path],
    environment: dict[str, str],
) -> tuple[bool, bool, bool, bool, bool]:
    python_link, _ = python
    network_script = (
        "import errno,socket,sys; s=socket.socket();\n"
        "try: s.connect(('127.0.0.1',9))\n"
        "except OSError as e: sys.exit(0 if e.errno in (errno.EPERM,errno.EACCES) else 2)\n"
        "sys.exit(3)"
    )
    network = _sandbox(
        sandbox_exec,
        profile,
        [os.fspath(python_link), "-I", "-S", "-c", network_script],
        env=environment,
    )[0] == 0
    shell_usable = _sandbox(
        sandbox_exec, profile, ["/bin/sh", "-c", "exit 0"], env=environment
    )[0] == 0
    process_denied = shell_usable and _sandbox(
        sandbox_exec, profile, ["/usr/bin/true"], env=environment
    )[0] != 0
    read_script = (
        "import errno,sys\n"
        "try: open(sys.argv[1],'rb').read(1)\n"
        "except PermissionError as e: sys.exit(0 if e.errno in (errno.EPERM,errno.EACCES) else 3)\n"
        "except OSError: sys.exit(4)\n"
        "sys.exit(5)\n"
    )
    cache_results = [
        _sandbox(
            sandbox_exec,
            profile,
            [os.fspath(python_link), "-I", "-S", "-c", read_script, os.fspath(cache_file)],
            env=environment,
        )[0] == 0
        for cache_file in cache_files
    ]
    cache_denied = bool(cache_results) and all(cache_results)
    source_fixture_denied = _sandbox(
        sandbox_exec,
        profile,
        [
            os.fspath(python_link), "-I", "-S", "-c", read_script,
            os.fspath(source_fixture_file),
        ],
        env=environment,
    )[0] == 0
    write_script = (
        "import errno,sys\n"
        "try: open(sys.argv[1],'ab').write(b'x')\n"
        "except PermissionError as e: sys.exit(0 if e.errno in (errno.EPERM,errno.EACCES) else 3)\n"
        "except OSError: sys.exit(4)\n"
        "sys.exit(5)\n"
    )
    write_denied = _sandbox(
        sandbox_exec,
        profile,
        [os.fspath(python_link), "-I", "-S", "-c", write_script, os.fspath(write_file)],
        env=environment,
    )[0] == 0
    return network, process_denied, cache_denied, source_fixture_denied, write_denied


def _sealed_environment(authority: dict[str, Any]) -> dict[str, str]:
    source = authority["semanticEnvironment"]
    allowed = {
        "HOME", "CODECLEW_HOME", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE",
        "XDG_CACHE_HOME", "JAVA_HOME", "SSL_CERT_FILE", "SSL_CERT_DIR",
        "GRADLE_USER_HOME", "MAVEN_USER_HOME", "CARGO_HOME", "RUSTUP_HOME",
        "PATH",
    }
    if not isinstance(source, dict) or not set(source).issubset(allowed):
        raise WarmAuditError("SEALED_ENVIRONMENT_INVALID")
    if "HOME" not in source or any(
        not isinstance(value, str) or not value or "\0" in value
        for value in source.values()
    ):
        raise WarmAuditError("SEALED_ENVIRONMENT_INVALID")
    path_keys = {
        "HOME", "CODECLEW_HOME", "TMPDIR", "XDG_CACHE_HOME", "JAVA_HOME",
        "SSL_CERT_FILE", "SSL_CERT_DIR", "GRADLE_USER_HOME", "MAVEN_USER_HOME",
        "CARGO_HOME", "RUSTUP_HOME",
    }
    for key in path_keys & set(source):
        path = Path(source[key])
        if (
            not path.is_absolute()
            or source[key] != os.fspath(path.resolve(strict=False))
        ):
            raise WarmAuditError("SEALED_ENVIRONMENT_INVALID")
    python = Path(authority["executables"]["python"])
    sealed_path = f"{python.parent}:/usr/bin:/bin"
    if source.get("PATH") != sealed_path:
        raise WarmAuditError("SEALED_ENVIRONMENT_INVALID")
    environment = dict(source)
    # Locale is protocol policy, not ambient semantic authority.  Dropping
    # LC_CTYPE also prevents macOS/Python from re-injecting a host locale into
    # otherwise identical compiler invocations.
    environment.pop("LC_CTYPE", None)
    environment["LANG"] = "C"
    environment["LC_ALL"] = "C"
    environment["PATH"] = sealed_path
    return environment


def _cache_roots(environment: dict[str, str], state_root: Path) -> list[tuple[str, Path]]:
    home = Path(environment["HOME"])
    candidates = [
        ("CODECLEW_DEPENDENCY", state_root / "dependency-cache"),
        ("GRADLE", Path(environment.get("GRADLE_USER_HOME", os.fspath(home / ".gradle")))),
        ("MAVEN", Path(environment.get("MAVEN_USER_HOME", os.fspath(home / ".m2")))),
        ("CARGO", Path(environment.get("CARGO_HOME", os.fspath(home / ".cargo")))),
        ("RUSTUP", Path(environment.get("RUSTUP_HOME", os.fspath(home / ".rustup")))),
    ]
    result: list[tuple[str, Path]] = []
    for label, raw in candidates:
        if not raw.is_absolute():
            raise WarmAuditError("CACHE_AUTHORITY_INVALID")
        path = raw.resolve(strict=False)
        if path in {Path("/"), home.resolve(strict=False)}:
            raise WarmAuditError("CACHE_AUTHORITY_INVALID")
        result.append((label, path))
    return result


def _cache_sentinel_specs(
    roots: list[tuple[str, Path]], authority_digest: str
) -> list[tuple[Path, bytes, str]]:
    specs: list[tuple[Path, bytes, str]] = []
    for label, root in roots:
        body = (
            "codeclew-warm-cache-canary:" + authority_digest + ":" + label + "\n"
        ).encode("ascii")
        name = (
            ".codeclew-warm-canary-"
            + authority_digest.removeprefix("sha256:")[:12]
            + "-"
            + label.lower().replace("_", "-")
        )
        specs.append(
            (root / name, body, f"sha256:{hashlib.sha256(body).hexdigest()}")
        )
    return specs


def _cache_sentinels(
    specs: list[tuple[Path, bytes, str]],
    begin: Callable[[Path, str], None],
    record: Callable[[dict[str, Any]], None],
) -> tuple[list[Path], str]:
    paths: list[Path] = []
    body_digests: list[str] = []
    for path, body, body_digest in specs:
        if not path.parent.is_dir():
            raise WarmAuditError("CACHE_CANARY_UNAVAILABLE")
        begin(path, body_digest)
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
        if hasattr(os, "O_NOFOLLOW"):
            flags |= os.O_NOFOLLOW
        descriptor: int | None = None
        try:
            descriptor = os.open(path, flags, 0o600)
            os.fchmod(descriptor, 0o600)
            offset = 0
            while offset < len(body):
                offset += os.write(descriptor, body[offset:])
            os.fsync(descriptor)
            metadata = os.fstat(descriptor)
            identity = {
                "path": os.fspath(path),
                "bodyDigest": body_digest,
                "device": metadata.st_dev,
                "inode": metadata.st_ino,
                "size": metadata.st_size,
            }
            if (
                not stat.S_ISREG(metadata.st_mode)
                or stat.S_IMODE(metadata.st_mode) != 0o600
                or metadata.st_uid != os.geteuid()
                or metadata.st_size != len(body)
            ):
                raise WarmAuditError("CACHE_CANARY_UNAVAILABLE")
        except OSError as error:
            raise WarmAuditError("CACHE_CANARY_UNAVAILABLE") from error
        finally:
            if descriptor is not None:
                os.close(descriptor)
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        record(identity)
        paths.append(path)
        body_digests.append(body_digest)
    return paths, pilot.authority_digest(body_digests)


def _ledger_path(output: Path) -> Path:
    return output.with_name(f".{output.name}.resources-pending.json")


def _ledger_value(
    authority: dict[str, Any],
    *,
    session_ids: list[str],
    thread_id: str | None,
    open_in_flight: dict[str, str] | None,
    temporary_root: dict[str, Any],
    sentinels: list[dict[str, Any]],
    sentinel_in_flight: dict[str, str] | None,
) -> dict[str, Any]:
    unsigned = {
        "schema": RESOURCE_LEDGER_SCHEMA,
        "ownerPid": os.getpid(),
        "pilotAuthorityDigest": authority["authorityDigest"],
        "adapterDigest": pilot.file_digest(Path(__file__).resolve(strict=True)),
        "clewDigest": authority["executables"]["clewDigest"],
        "sessionIds": session_ids,
        "threadId": thread_id,
        "openInFlight": open_in_flight,
        "temporaryRoot": temporary_root,
        "sentinels": sentinels,
        "sentinelInFlight": sentinel_in_flight,
    }
    return {**unsigned, "authorityDigest": pilot.authority_digest(unsigned)}


def _validate_ledger(
    value: Any,
    authority: dict[str, Any],
    sentinel_specs: list[tuple[Path, bytes, str]],
) -> dict[str, Any]:
    fields = {
        "schema", "authorityDigest", "ownerPid", "pilotAuthorityDigest",
        "adapterDigest", "clewDigest", "sessionIds", "threadId", "openInFlight",
        "temporaryRoot", "sentinels", "sentinelInFlight",
    }
    if not isinstance(value, dict) or set(value) != fields:
        raise WarmAuditError("WARM_RESOURCE_LEDGER_INVALID")
    unsigned = dict(value)
    declared = unsigned.pop("authorityDigest")
    expected_specs = {
        os.fspath(path): (len(body), body_digest)
        for path, body, body_digest in sentinel_specs
    }
    expected_spec_order = [os.fspath(path) for path, _, _ in sentinel_specs]
    sessions = value.get("sessionIds")
    thread_id = value.get("threadId")
    open_in_flight = value.get("openInFlight")
    temporary_root = value.get("temporaryRoot")
    sentinel_in_flight = value.get("sentinelInFlight")
    if open_in_flight is not None and (
        not isinstance(open_in_flight, dict)
        or set(open_in_flight) != {"kind", "requestDigest"}
        or not isinstance(open_in_flight.get("kind"), str)
        or open_in_flight.get("kind") not in {"SESSION", "THREAD"}
        or not isinstance(open_in_flight.get("requestDigest"), str)
        or SHA256.fullmatch(open_in_flight["requestDigest"]) is None
    ):
        raise WarmAuditError("WARM_RESOURCE_LEDGER_INVALID")
    if (
        not isinstance(temporary_root, dict)
        or set(temporary_root) != {"path", "device", "inode"}
        or not isinstance(temporary_root.get("path"), str)
        or not Path(temporary_root["path"]).is_absolute()
        or Path(temporary_root["path"]).parent
        != Path(tempfile.gettempdir()).resolve(strict=True)
        or not Path(temporary_root["path"]).name.startswith("codeclew-s4k-warm-")
        or type(temporary_root.get("device")) is not int
        or type(temporary_root.get("inode")) is not int
        or temporary_root["device"] <= 0
        or temporary_root["inode"] <= 0
    ):
        raise WarmAuditError("WARM_RESOURCE_LEDGER_INVALID")
    if sentinel_in_flight is not None and (
        not isinstance(sentinel_in_flight, dict)
        or set(sentinel_in_flight) != {"path", "bodyDigest"}
        or not isinstance(sentinel_in_flight.get("path"), str)
        or sentinel_in_flight.get("path") not in expected_specs
        or sentinel_in_flight.get("bodyDigest")
        != expected_specs[sentinel_in_flight["path"]][1]
    ):
        raise WarmAuditError("WARM_RESOURCE_LEDGER_INVALID")
    sentinels = value.get("sentinels")
    if not isinstance(sentinels, list) or len(sentinels) > len(expected_specs):
        raise WarmAuditError("WARM_RESOURCE_LEDGER_INVALID")
    seen_paths: set[str] = set()
    for row in sentinels:
        if (
            not isinstance(row, dict)
            or set(row) != {"path", "bodyDigest", "device", "inode", "size"}
            or not isinstance(row.get("path"), str)
            or row.get("path") not in expected_specs
            or row["path"] in seen_paths
            or row.get("bodyDigest") != expected_specs[row["path"]][1]
            or type(row.get("device")) is not int
            or type(row.get("inode")) is not int
            or type(row.get("size")) is not int
            or row["device"] <= 0
            or row["inode"] <= 0
            or row["size"] != expected_specs[row["path"]][0]
        ):
            raise WarmAuditError("WARM_RESOURCE_LEDGER_INVALID")
        seen_paths.add(row["path"])
    if [row["path"] for row in sentinels] != expected_spec_order[: len(sentinels)]:
        raise WarmAuditError("WARM_RESOURCE_LEDGER_INVALID")
    if sentinel_in_flight is not None and sentinel_in_flight["path"] in seen_paths:
        raise WarmAuditError("WARM_RESOURCE_LEDGER_INVALID")
    if sentinel_in_flight is not None and (
        len(sentinels) >= len(expected_spec_order)
        or sentinel_in_flight["path"] != expected_spec_order[len(sentinels)]
    ):
        raise WarmAuditError("WARM_RESOURCE_LEDGER_INVALID")
    if (
        value.get("schema") != RESOURCE_LEDGER_SCHEMA
        or not isinstance(declared, str)
        or declared != pilot.authority_digest(unsigned)
        or type(value.get("ownerPid")) is not int
        or value["ownerPid"] <= 0
        or value.get("pilotAuthorityDigest") != authority["authorityDigest"]
        or value.get("adapterDigest") != pilot.file_digest(Path(__file__).resolve(strict=True))
        or value.get("clewDigest") != authority["executables"]["clewDigest"]
        or not isinstance(sessions, list)
        or len(sessions) > 3
        or any(
            not isinstance(item, str)
            or descriptor_gate.SESSION_ID.fullmatch(item) is None
            for item in sessions
        )
        or len(set(sessions)) != len(sessions)
        or (thread_id is not None and (not isinstance(thread_id, str) or not thread_id.startswith("thread:")))
        or (thread_id is not None and len(sessions) != 3)
        or (
            open_in_flight is not None
            and open_in_flight["kind"] == "THREAD"
            and (len(sessions) != 3 or thread_id is not None)
        )
        or (
            open_in_flight is not None
            and open_in_flight["kind"] == "SESSION"
            and (len(sessions) >= 3 or thread_id is not None)
        )
    ):
        raise WarmAuditError("WARM_RESOURCE_LEDGER_INVALID")
    return value


def _temporary_root_identity(root: Path) -> dict[str, Any]:
    metadata = os.lstat(root)
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or metadata.st_uid != os.geteuid()
        or root.parent != Path(tempfile.gettempdir()).resolve(strict=True)
        or not root.name.startswith("codeclew-s4k-warm-")
    ):
        raise WarmAuditError("WARM_TEMPORARY_ROOT_INVALID")
    return {
        "path": os.fspath(root),
        "device": metadata.st_dev,
        "inode": metadata.st_ino,
    }


def _temporary_root_exists(value: dict[str, Any]) -> bool:
    path = Path(value["path"])
    try:
        metadata = os.lstat(path)
    except FileNotFoundError:
        return False
    except OSError as error:
        raise WarmAuditError("WARM_TEMPORARY_ROOT_INVALID") from error
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or metadata.st_uid != os.geteuid()
        or (metadata.st_dev, metadata.st_ino) != (value["device"], value["inode"])
    ):
        raise WarmAuditError("WARM_TEMPORARY_ROOT_INVALID")
    return True


def _remove_temporary_root(value: dict[str, Any]) -> None:
    if not _temporary_root_exists(value):
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
        raise WarmAuditError("WARM_TEMPORARY_ROOT_CLEANUP_FAILED") from error


def _create_ledger(path: Path, value: dict[str, Any]) -> None:
    raw = pilot.canonical_bytes(value) + b"\n"
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
        raise WarmAuditError("WARM_RESOURCE_LEDGER_CREATE_FAILED") from error


def _update_ledger(path: Path, value: dict[str, Any]) -> None:
    try:
        pilot.atomic_write(path, value, 0o600)
    except pilot.PilotError as error:
        raise WarmAuditError("WARM_RESOURCE_LEDGER_UPDATE_FAILED") from error


def _remove_recovery_sentinels(
    records: list[dict[str, Any]],
    specs: list[tuple[Path, bytes, str]],
    *,
    allow_missing: bool,
) -> None:
    bodies = {
        os.fspath(path): (body, body_digest)
        for path, body, body_digest in specs
    }
    failed = False
    verified: list[tuple[Path, dict[str, Any]]] = []
    for record in records:
        raw_path = record.get("path") if isinstance(record, dict) else None
        expected = bodies.get(raw_path) if isinstance(raw_path, str) else None
        if expected is None:
            failed = True
            continue
        path = Path(raw_path)
        body, body_digest = expected
        descriptor: int | None = None
        try:
            metadata = os.lstat(path)
            if (
                stat.S_ISLNK(metadata.st_mode)
                or not stat.S_ISREG(metadata.st_mode)
                or stat.S_IMODE(metadata.st_mode) != 0o600
                or metadata.st_uid != os.geteuid()
                or metadata.st_dev != record.get("device")
                or metadata.st_ino != record.get("inode")
                or metadata.st_size != record.get("size")
                or metadata.st_size != len(body)
                or record.get("bodyDigest") != body_digest
            ):
                raise OSError
            descriptor = os.open(
                path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
            )
            opened = os.fstat(descriptor)
            if (
                opened.st_dev != metadata.st_dev
                or opened.st_ino != metadata.st_ino
                or opened.st_mode != metadata.st_mode
                or opened.st_size != metadata.st_size
            ):
                raise OSError
            observed = bytearray()
            while len(observed) <= len(body):
                chunk = os.read(descriptor, len(body) + 1 - len(observed))
                if not chunk:
                    break
                observed.extend(chunk)
            after = os.fstat(descriptor)
            if (
                after.st_dev != metadata.st_dev
                or after.st_ino != metadata.st_ino
                or after.st_size != metadata.st_size
                or bytes(observed) != body
                or f"sha256:{hashlib.sha256(observed).hexdigest()}" != body_digest
            ):
                raise OSError
            verified.append((path, record))
        except FileNotFoundError:
            if not allow_missing:
                failed = True
        except OSError:
            failed = True
        finally:
            if descriptor is not None:
                os.close(descriptor)
    if failed:
        raise WarmAuditError("CACHE_CANARY_CLEANUP_FAILED")
    for path, record in verified:
        try:
            metadata = os.lstat(path)
            if (
                not stat.S_ISREG(metadata.st_mode)
                or metadata.st_dev != record["device"]
                or metadata.st_ino != record["inode"]
                or metadata.st_size != record["size"]
            ):
                raise OSError
            path.unlink()
            directory = os.open(path.parent, os.O_RDONLY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        except OSError as error:
            raise WarmAuditError("CACHE_CANARY_CLEANUP_FAILED") from error


def _remove_ledger(path: Path) -> None:
    try:
        metadata = os.lstat(path)
        if (
            stat.S_ISLNK(metadata.st_mode)
            or not stat.S_ISREG(metadata.st_mode)
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_uid != os.geteuid()
        ):
            raise OSError
        path.unlink()
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except OSError as error:
        raise WarmAuditError("WARM_RESOURCE_LEDGER_REMOVE_FAILED") from error


def _create_output_once(path: Path, value: dict[str, Any]) -> dict[str, int]:
    raw = pilot.canonical_bytes(value) + b"\n"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor: int | None = None
    try:
        descriptor = os.open(path, flags, 0o600)
        os.fchmod(descriptor, 0o600)
        offset = 0
        while offset < len(raw):
            offset += os.write(descriptor, raw[offset:])
        os.fsync(descriptor)
        metadata = os.fstat(descriptor)
        identity = {
            "device": metadata.st_dev,
            "inode": metadata.st_ino,
            "size": metadata.st_size,
        }
        if (
            not stat.S_ISREG(metadata.st_mode)
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_uid != os.geteuid()
            or metadata.st_size != len(raw)
        ):
            raise OSError
        os.close(descriptor)
        descriptor = None
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        return identity
    except FileExistsError as error:
        raise WarmAuditError("WARM_OUTPUT_ALREADY_EXISTS") from error
    except OSError as error:
        # Never remove a possibly durable output inode here.  The retained
        # resource ledger is the fail-stop recovery authority.
        raise WarmAuditError("WARM_OUTPUT_CREATE_FAILED") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)


def _validate_output_exact(
    path: Path,
    value: dict[str, Any],
    identity: dict[str, int],
) -> None:
    raw = pilot.canonical_bytes(value) + b"\n"
    descriptor: int | None = None
    try:
        metadata = os.lstat(path)
        if (
            stat.S_ISLNK(metadata.st_mode)
            or not stat.S_ISREG(metadata.st_mode)
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_uid != os.geteuid()
            or metadata.st_dev != identity["device"]
            or metadata.st_ino != identity["inode"]
            or metadata.st_size != identity["size"]
            or metadata.st_size != len(raw)
        ):
            raise OSError
        descriptor = os.open(
            path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
        )
        opened = os.fstat(descriptor)
        if (
            opened.st_dev != metadata.st_dev
            or opened.st_ino != metadata.st_ino
            or opened.st_size != metadata.st_size
        ):
            raise OSError
        observed = bytearray()
        while len(observed) <= len(raw):
            chunk = os.read(descriptor, len(raw) + 1 - len(observed))
            if not chunk:
                break
            observed.extend(chunk)
        after = os.fstat(descriptor)
        if (
            after.st_dev != metadata.st_dev
            or after.st_ino != metadata.st_ino
            or after.st_size != metadata.st_size
            or bytes(observed) != raw
        ):
            raise OSError
        parsed = json.loads(bytes(observed), object_pairs_hook=pilot._duplicates)
        if parsed != value or pilot.canonical_bytes(parsed) + b"\n" != bytes(observed):
            raise OSError
    except (OSError, json.JSONDecodeError, UnicodeDecodeError, pilot.PilotError) as error:
        raise WarmAuditError("WARM_OUTPUT_VALIDATION_FAILED") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)


def _pid_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def _recover_ledger(
    path: Path,
    authority: dict[str, Any],
    sentinel_specs: list[tuple[Path, bytes, str]],
    clew: Path,
    environment: dict[str, str],
) -> None:
    try:
        os.lstat(path)
    except FileNotFoundError:
        return
    except OSError as error:
        raise WarmAuditError("WARM_RESOURCE_LEDGER_INVALID") from error
    try:
        _, raw, _ = pilot.private_json(path, "WARM_RESOURCE_LEDGER", 256 * 1024)
    except pilot.PilotError as error:
        raise WarmAuditError("WARM_RESOURCE_LEDGER_INVALID") from error
    value = _validate_ledger(raw, authority, sentinel_specs)
    if _pid_alive(value["ownerPid"]):
        raise WarmAuditError("WARM_AUDIT_ALREADY_RUNNING")
    if (
        value["openInFlight"] is not None
        or value["sentinelInFlight"] is not None
        or _temporary_root_exists(value["temporaryRoot"])
    ):
        # The product has no request-id/status endpoint that can prove the ID
        # of a create interrupted between child commit and parent checkpoint.
        # Preserve the private locator and require explicit operator cleanup.
        raise WarmAuditError("OPERATOR_CLEANUP_REQUIRED")
    failures = 0
    try:
        _remove_recovery_sentinels(
            value["sentinels"], sentinel_specs, allow_missing=True
        )
    except WarmAuditError:
        failures += 1
    thread_id = value["threadId"]
    if thread_id is not None:
        try:
            _close_and_gc(clew, "thread", thread_id, environment)
        except WarmAuditError:
            failures += 1
    for session_id in reversed(value["sessionIds"]):
        try:
            _close_and_gc(clew, "session", session_id, environment)
        except WarmAuditError:
            failures += 1
    if failures:
        raise WarmAuditError("WARM_RESOURCE_RECOVERY_FAILED")
    _remove_ledger(path)


def _capsule(state_root: Path, runtime_key: str) -> Path:
    if not isinstance(runtime_key, str) or SHA256.fullmatch(runtime_key) is None:
        raise WarmAuditError("RUNTIME_AUTHORITY_INVALID")
    path = state_root / "runtimes" / runtime_key.removeprefix("sha256:") / "bin" / "clew"
    try:
        resolved = path.resolve(strict=True)
        metadata = resolved.stat()
    except OSError as error:
        raise WarmAuditError("RUNTIME_AUTHORITY_INVALID") from error
    if not stat.S_ISREG(metadata.st_mode) or not os.access(resolved, os.X_OK):
        raise WarmAuditError("RUNTIME_AUTHORITY_INVALID")
    return resolved


def _runtime_key(authority: dict[str, Any]) -> str:
    sessions = authority.get("sessions")
    if not isinstance(sessions, list) or not sessions:
        raise WarmAuditError("RUNTIME_AUTHORITY_INVALID")
    keys = {row.get("runtimeKey") for row in sessions if isinstance(row, dict)}
    if len(keys) != 1:
        raise WarmAuditError("RUNTIME_AUTHORITY_INVALID")
    key = next(iter(keys))
    if not isinstance(key, str) or SHA256.fullmatch(key) is None:
        raise WarmAuditError("RUNTIME_AUTHORITY_INVALID")
    return key


def _validate_lifecycle(
    value: Any,
    *,
    resource_kind: str,
    resource_id: str,
    status: str,
    garbage_collection: bool,
) -> None:
    lifecycle_fields = {
        "schema", f"{resource_kind}Id", f"{resource_kind}AuthorityDigest",
        "sequence", "previousEventHash", "status", "eventHash", "updatedUnixMs",
    }
    lifecycle_key = f"{resource_kind}Id"
    authority_key = f"{resource_kind}AuthorityDigest"
    if resource_kind == "thread":
        top_fields = {"schema", "threadId", "lifecycle"}
        expected_schema = (
            "codeclew-thread-gc-result/1.0"
            if garbage_collection
            else "codeclew-thread-lifecycle-result/1.0"
        )
        if not isinstance(value, dict) or set(value) != top_fields or value.get("threadId") != resource_id:
            raise WarmAuditError("WARM_CLEANUP_RESULT_INVALID")
    elif resource_kind == "session":
        top_fields = {"schema", "lifecycle"}
        expected_schema = (
            "codeclew-session-gc-result/1.0"
            if garbage_collection
            else "codeclew-session-lifecycle-result/1.0"
        )
        if not isinstance(value, dict) or set(value) != top_fields:
            raise WarmAuditError("WARM_CLEANUP_RESULT_INVALID")
    else:
        raise WarmAuditError("WARM_CLEANUP_RESULT_INVALID")
    lifecycle = value.get("lifecycle")
    previous = lifecycle.get("previousEventHash") if isinstance(lifecycle, dict) else None
    if (
        value.get("schema") != expected_schema
        or not isinstance(lifecycle, dict)
        or set(lifecycle) != lifecycle_fields
        or lifecycle.get("schema") != f"codeclew-{resource_kind}-lifecycle-entry/1.0"
        or lifecycle.get(lifecycle_key) != resource_id
        or not isinstance(lifecycle.get(authority_key), str)
        or SHA256.fullmatch(lifecycle[authority_key]) is None
        or type(lifecycle.get("sequence")) is not int
        or lifecycle["sequence"] < (2 if garbage_collection else 1)
        or not isinstance(previous, str)
        or SHA256.fullmatch(previous) is None
        or lifecycle.get("status") != status
        or not isinstance(lifecycle.get("eventHash"), str)
        or SHA256.fullmatch(lifecycle["eventHash"]) is None
        or type(lifecycle.get("updatedUnixMs")) is not int
        or lifecycle["updatedUnixMs"] < 0
    ):
        raise WarmAuditError("WARM_CLEANUP_RESULT_INVALID")


def _close_and_gc(
    clew: Path,
    resource_kind: str,
    resource_id: str,
    environment: dict[str, str],
) -> None:
    closed_status = "CLOSED"
    try:
        closed = _run_json(
            [
                os.fspath(clew), resource_kind, "close",
                f"--{resource_kind}", resource_id,
            ],
            120,
            environment,
        )
        _validate_lifecycle(
            closed,
            resource_kind=resource_kind,
            resource_id=resource_id,
            status=closed_status,
            garbage_collection=False,
        )
    except WarmAuditError:
        # A crash after a successful GC but before the ledger update makes
        # `close` an invalid backwards transition.  Exact idempotent GC below
        # is the sole accepted recovery for that case.
        pass
    collected = _run_json(
        [
            os.fspath(clew), resource_kind, "gc",
            f"--{resource_kind}", resource_id,
        ],
        120,
        environment,
    )
    _validate_lifecycle(
        collected,
        resource_kind=resource_kind,
        resource_id=resource_id,
        status="GARBAGE_COLLECTED",
        garbage_collection=True,
    )


def _cleanup(
    clew: Path,
    thread_id: str | None,
    sessions: dict[str, dict[str, Any]],
    environment: dict[str, str],
) -> None:
    failures = 0
    if thread_id is not None:
        try:
            _close_and_gc(clew, "thread", thread_id, environment)
        except WarmAuditError:
            failures += 1
    for value in reversed(list(sessions.values())):
        try:
            session_id = value["sessionId"]
            _close_and_gc(clew, "session", session_id, environment)
        except (KeyError, WarmAuditError):
            failures += 1
    if failures:
        raise WarmAuditError("WARM_CLEANUP_FAILED")


def run_audit(args: argparse.Namespace) -> dict[str, Any]:
    _, authority_value, _ = pilot.private_json(args.private_authority, "PILOT_AUTHORITY")
    _, oracle_value, _ = pilot.private_json(args.private_oracle, "PILOT_ORACLE")
    authority = pilot.verify_authority(authority_value)
    oracle = pilot.verify_oracle(oracle_value, authority)
    source = args.source_repo.resolve(strict=True)
    clew = Path(authority["executables"]["clew"]).resolve(strict=True)
    if source != clew.parent:
        raise WarmAuditError("FIXTURE_SOURCE_AUTHORITY_MISMATCH")
    git = Path(authority["executables"]["git"])
    sandbox_exec = Path(authority["executables"]["sandboxExec"])
    if not sandbox_exec.is_file() or sys.platform != "darwin":
        raise WarmAuditError("AUDITED_HOST_ADAPTER_UNAVAILABLE")
    environment = _sealed_environment(authority)
    python = _python(authority)
    state_root = _state_root(environment)
    expected_runtime_key = _runtime_key(authority)
    capsule = _capsule(state_root, expected_runtime_key)
    cache_roots = _cache_roots(environment, state_root)
    sentinel_specs = _cache_sentinel_specs(cache_roots, authority["authorityDigest"])
    sealed_fixture_digest = authority["inputs"].get("warmFixtureDigest")
    if not isinstance(sealed_fixture_digest, str) or SHA256.fullmatch(sealed_fixture_digest) is None:
        raise WarmAuditError("FIXTURE_SOURCE_AUTHORITY_MISMATCH")
    target = pilot.output_target(args.private_output, True)
    resource_ledger = _ledger_path(target)
    _recover_ledger(
        resource_ledger, authority, sentinel_specs, clew, environment
    )
    try:
        os.lstat(target)
    except FileNotFoundError:
        pass
    except OSError as error:
        raise WarmAuditError("WARM_OUTPUT_ALREADY_EXISTS") from error
    else:
        raise WarmAuditError("WARM_OUTPUT_ALREADY_EXISTS")
    root = Path(tempfile.mkdtemp(prefix="codeclew-s4k-warm-")).resolve(strict=True)
    os.chmod(root, 0o700)
    temporary_root = _temporary_root_identity(root)
    sessions: dict[str, dict[str, Any]] = {}
    thread_id: str | None = None
    ledger_sessions: list[str] = []
    open_in_flight: dict[str, str] | None = None
    sentinel_records: list[dict[str, Any]] = []
    sentinel_in_flight: dict[str, str] | None = None
    try:
        _create_ledger(
            resource_ledger,
            _ledger_value(
                authority,
                session_ids=ledger_sessions,
                thread_id=None,
                open_in_flight=open_in_flight,
                temporary_root=temporary_root,
                sentinels=sentinel_records,
                sentinel_in_flight=sentinel_in_flight,
            ),
        )
    except Exception:
        _remove_temporary_root(temporary_root)
        raise

    def persist_ledger() -> None:
        _update_ledger(
            resource_ledger,
            _ledger_value(
                authority,
                session_ids=ledger_sessions,
                thread_id=thread_id,
                open_in_flight=open_in_flight,
                temporary_root=temporary_root,
                sentinels=sentinel_records,
                sentinel_in_flight=sentinel_in_flight,
            ),
        )

    def record_open_start(kind: str, request_digest: str) -> None:
        nonlocal open_in_flight
        open_in_flight = {"kind": kind, "requestDigest": request_digest}
        persist_ledger()

    def record_session(session_id: str) -> None:
        nonlocal open_in_flight
        ledger_sessions.append(session_id)
        open_in_flight = None
        persist_ledger()

    def record_sentinel_start(path: Path, body_digest: str) -> None:
        nonlocal sentinel_in_flight
        sentinel_in_flight = {
            "path": os.fspath(path),
            "bodyDigest": body_digest,
        }
        persist_ledger()

    def record_sentinel(identity: dict[str, Any]) -> None:
        nonlocal sentinel_in_flight
        sentinel_records.append(identity)
        sentinel_in_flight = None
        persist_ledger()

    # From this point on, every failure is deliberately fail-stop: no
    # best-effort cleanup is attempted.  The 0600 ledger and 0700 root remain
    # the exact operator authority, without leaking either locator to output.
    repositories: dict[str, Path] = {}
    revisions: dict[str, str] = {}
    fixture_tree: str | None = None
    for alias in ["provider", "consumer", "observer"]:
        repository = root / alias
        observed_tree, observed_digest = _copy_tracked_fixture(source, repository, git)
        if observed_digest != sealed_fixture_digest:
            raise WarmAuditError("FIXTURE_SOURCE_AUTHORITY_MISMATCH")
        fixture_tree = fixture_tree or observed_tree
        if observed_tree != fixture_tree:
            raise WarmAuditError("FIXTURE_TREE_MISMATCH")
        repositories[alias] = repository
        revisions[alias] = _commit_fixture(repository, git)

    setup_started = time.monotonic_ns()
    _open_fixture_sessions(
        clew, repositories, revisions, sessions, environment,
        expected_runtime_key, record_open_start, record_session,
    )
    record_open_start(
        "THREAD",
        pilot.authority_digest(
            {
                "members": [
                    {"alias": alias, "sessionId": sessions[alias]["sessionId"]}
                    for alias in ["provider", "consumer", "observer"]
                ]
            }
        ),
    )
    thread_id = _thread(clew, sessions, environment)
    open_in_flight = None
    persist_ledger()
    _, fact_set = _context_and_fact_set(clew, thread_id, environment)
    fixture_impacts = [
        _run_json(
            _impact_command(clew, thread_id, fact_set, term),
            timeout=300,
            env=environment,
        )
        for term in TERMS
    ]
    actual_fixture = _exact_fixture_rows(
        fixture_impacts, repositories["provider"], revisions["provider"], git
    )
    expected_fixture = oracle["fixture"]
    matched, false_exact = _fixture_comparison(actual_fixture, expected_fixture)
    expected_public = [
        row for row in expected_fixture
        if row.get("descriptorClass") == "CALLABLE"
        and row.get("declarationKind") == "FUNCTION"
        and row.get("name") == "publicDescriptor"
    ]
    actual_public = [
        row for row in actual_fixture
        if row.get("descriptorClass") == "CALLABLE"
        and row.get("declarationKind") == "FUNCTION"
        and row.get("name") == "publicDescriptor"
    ]
    if (
        matched != len(expected_fixture)
        or false_exact != 0
        or len(expected_public) != 1
        or actual_public != expected_public
    ):
        raise WarmAuditError("FIXTURE_SHAPE_INVALID")
    expected_descriptor = expected_public[0]
    setup_elapsed = max(1, time.monotonic_ns() - setup_started)
    original_fixture = (source / "fixtures" / "kotlin-basic").resolve(strict=True)
    source_fixture_file = (
        original_fixture / _safe_relative(expected_descriptor["relativeFile"])
    ).resolve(strict=True)
    try:
        source_fixture_file.relative_to(original_fixture)
    except ValueError as error:
        raise WarmAuditError("FIXTURE_SOURCE_AUTHORITY_MISMATCH") from error
    profile = _profile(
        clew, capsule, [original_fixture, *repositories.values()], state_root,
        python, cache_roots,
    )
    write_canary = root / "write-canary"
    write_canary.write_bytes(b"")
    os.chmod(write_canary, 0o600)
    cache_sentinels, cache_sentinel_digest = _cache_sentinels(
        sentinel_specs, record_sentinel_start, record_sentinel
    )
    (
        network_denied,
        process_denied,
        cache_denied,
        source_fixture_denied,
        write_denied,
    ) = _canaries(
        sandbox_exec, profile, cache_sentinels, source_fixture_file,
        write_canary.resolve(strict=True), python, environment,
    )
    if not (
        network_denied
        and process_denied
        and cache_denied
        and source_fixture_denied
        and write_denied
    ):
        raise WarmAuditError("SANDBOX_CANARY_FAILED")
    _remove_recovery_sentinels(
        sentinel_records, sentinel_specs, allow_missing=False
    )
    command = _impact_command(clew, thread_id, fact_set, SUBJECT)
    prime_code, prime_stdout, prime_stderr, prime_elapsed = _sandbox(
        sandbox_exec, profile, command, env=environment, timeout=60
    )
    if prime_code != 0 or prime_stderr:
        raise WarmAuditError("WARM_PRIMING_FAILED")
    try:
        prime_value = json.loads(prime_stdout, object_pairs_hook=pilot._duplicates)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise WarmAuditError("WARM_PRIMING_FAILED") from error
    if prime_stdout != pilot.canonical_bytes(prime_value) + b"\n":
        raise WarmAuditError("WARM_PRIMING_FAILED")
    _validate_warm_impact_result(
        prime_value,
        thread_id=thread_id,
        fact_set=fact_set,
        subject=SUBJECT,
        expected_descriptor=expected_descriptor,
    )
    before = _state_snapshot(state_root)
    samples: list[int] = []
    stdout_digests: list[str] = []
    measured_denials = 0
    for _ in range(30):
        return_code, stdout, stderr, elapsed = _sandbox(
            sandbox_exec, profile, command, env=environment, timeout=60
        )
        if return_code != 0 or stderr:
            measured_denials += 1
            raise WarmAuditError("MEASURED_SANDBOX_FAILURE")
        try:
            value = json.loads(stdout, object_pairs_hook=pilot._duplicates)
        except (json.JSONDecodeError, UnicodeDecodeError) as error:
            raise WarmAuditError("MEASURED_OUTPUT_INVALID") from error
        if stdout != pilot.canonical_bytes(value) + b"\n":
            raise WarmAuditError("MEASURED_OUTPUT_INVALID")
        _validate_warm_impact_result(
            value,
            thread_id=thread_id,
            fact_set=fact_set,
            subject=SUBJECT,
            expected_descriptor=expected_descriptor,
        )
        samples.append(elapsed)
        stdout_digests.append(f"sha256:{hashlib.sha256(stdout).hexdigest()}")
    prime_stdout_digest = f"sha256:{hashlib.sha256(prime_stdout).hexdigest()}"
    if set(stdout_digests) != {prime_stdout_digest}:
        raise WarmAuditError("MEASURED_OUTPUT_CHANGED_AFTER_PRIME")
    after = _state_snapshot(state_root)
    if before != after:
        raise WarmAuditError("WARM_STATE_MUTATED")
    pending_attestation = {
        "schema": pilot.PRIVATE_WARM_ATTESTATION_SCHEMA,
        "pilotAuthorityDigest": authority["authorityDigest"],
        "protocolDigest": authority["protocolDigest"],
        "clewDigest": authority["executables"]["clewDigest"],
        "adapterDigest": pilot.file_digest(Path(__file__).resolve(strict=True)),
        "measurementClass": MEASUREMENT_CLASS,
        "profilePolicy": PROFILE_POLICY,
        "profileDigest": pilot.authority_digest(profile),
        "fixtureTreeDigest": sealed_fixture_digest,
        "executionAuthority": {
            "pythonDigest": authority["executables"]["pythonDigest"],
            "stateRootDigest": pilot.authority_digest(os.fspath(state_root)),
            "runtimeKey": expected_runtime_key,
        },
        "effectiveCacheRoots": [
            {"label": label, "pathDigest": pilot.authority_digest(os.fspath(path))}
            for label, path in cache_roots
        ],
        "setup": {
            "coldPreparationOutsideMeasuredInterval": True,
            "coldProcessAndCacheAccessPermitted": True,
            "coldNetworkAccessPermitted": True,
            "coldPreparationCommandCount": 6 + len(TERMS),
            "coldPreparationElapsedNanos": setup_elapsed,
            "runtimeKey": expected_runtime_key,
        },
        "priming": {
            "commandCount": 1,
            "elapsedNanos": prime_elapsed,
            "runtimeKey": expected_runtime_key,
            "stdoutDigest": prime_stdout_digest,
            "profileDigest": pilot.authority_digest(profile),
            "networkPolicy": "KERNEL_DENIED",
            "cacheReadPolicy": "EFFECTIVE_ROOTS_DENIED",
            "processPolicy": "SEALED_ALLOWLIST_ONLY",
        },
        "stateSnapshotBefore": before,
        "stateSnapshotAfter": after,
        "stateSnapshotUnchanged": before == after,
        "fixture": {
            "expectedShapeCount": len(expected_fixture),
            "matchedShapeCount": matched,
            "falseExactClaimCount": false_exact,
        },
        "samplesNanos": samples,
        "stdoutDigests": stdout_digests,
        "audit": {
            "adapter": ADAPTER,
            "networkCanaryDenied": network_denied,
            "processCanaryDenied": process_denied,
            "cacheCanaryDenied": cache_denied,
            "writeCanaryDenied": write_denied,
            "measuredDenialCount": measured_denials,
            "prohibitedProcessCount": 0,
            "cacheAccessCount": 0,
            "cacheRootCanaryCount": len(cache_roots),
            "cacheSentinelDigest": cache_sentinel_digest,
        },
    }
    if open_in_flight is not None or sentinel_in_flight is not None:
        raise WarmAuditError("OPERATOR_CLEANUP_REQUIRED")
    _cleanup(clew, thread_id, sessions, environment)
    _remove_recovery_sentinels(
        sentinel_records, sentinel_specs, allow_missing=True
    )
    _remove_temporary_root(temporary_root)
    pending_attestation["cleanupCompleted"] = True
    pending_attestation["privateOutputMode"] = "0600"
    unsigned = dict(pending_attestation)
    pending_attestation["authorityDigest"] = pilot.authority_digest(unsigned)
    output_identity = _create_output_once(target, pending_attestation)
    _validate_output_exact(target, pending_attestation, output_identity)
    _remove_ledger(resource_ledger)
    return {
        "schema": pilot.PRIVATE_WARM_ATTESTATION_SCHEMA,
        "status": MEASUREMENT_CLASS,
        "runCount": 30,
    }


def self_test() -> None:
    if not _safe_relative("src/main/kotlin/Sample.kt").as_posix().endswith("Sample.kt"):
        raise AssertionError("relative path validation failed")
    try:
        _safe_relative("../secret.kt")
    except WarmAuditError:
        pass
    else:
        raise AssertionError("escaping fixture path was accepted")
    sample = {"a": 1, "b": [2, 3]}
    if pilot.authority_digest(sample) != pilot.authority_digest(json.loads(json.dumps(sample))):
        raise AssertionError("canonical authority is unstable")
    contour = [
        {
            "descriptorClass": "CALLABLE",
            "declarationKind": "FUNCTION",
            "name": f"shape-{index}",
            "ownerIdentity": "owner",
            "normalizedSignature": f"signature-{index}",
            "shapeDigest": f"digest-{index}",
        }
        for index in range(5)
    ]
    outside = dict(contour[0], name="outside", shapeDigest="conflict")
    if _fixture_comparison([*contour, outside], contour) != (5, 0):
        raise AssertionError("out-of-contour fixture changed conflict counts")
    conflict = dict(contour[0], shapeDigest="conflict")
    if _fixture_comparison([*contour[1:], conflict], contour) != (4, 1):
        raise AssertionError("in-contour fixture conflict was not counted")
    extra_exact = dict(contour[0], ownerIdentity="wrong-owner", shapeDigest="extra")
    if _fixture_comparison([*contour, extra_exact], contour) != (5, 1):
        raise AssertionError("extra exact target claim was not counted")
    python_path = Path(sys.executable).resolve(strict=True)
    sealed_path = f"{python_path.parent}:/usr/bin:/bin"
    normalized_home = os.fspath(Path(tempfile.gettempdir()).resolve(strict=True))
    environment_authority = {
        "semanticEnvironment": {"HOME": normalized_home, "PATH": sealed_path},
        "executables": {"python": os.fspath(python_path)},
    }
    expected_environment = {
        "HOME": normalized_home,
        "PATH": sealed_path,
        "LANG": "C",
        "LC_ALL": "C",
    }
    if _sealed_environment(environment_authority) != expected_environment:
        raise AssertionError("missing locale was not normalized")
    for partial_key in ("LANG", "LC_ALL", "LC_CTYPE"):
        partial_locale = json.loads(json.dumps(environment_authority))
        partial_locale["semanticEnvironment"][partial_key] = "host-partial"
        if _sealed_environment(partial_locale) != expected_environment:
            raise AssertionError("partial locale was not normalized")
    hostile_locale = json.loads(json.dumps(environment_authority))
    hostile_locale["semanticEnvironment"].update(
        {"LANG": "hostile", "LC_ALL": "partial", "LC_CTYPE": "injected"}
    )
    if _sealed_environment(hostile_locale) != expected_environment:
        raise AssertionError("hostile/partial locale remained semantic authority")
    injected = json.loads(json.dumps(environment_authority))
    injected["semanticEnvironment"]["PYTHONPATH"] = tempfile.gettempdir()
    try:
        _sealed_environment(injected)
    except WarmAuditError:
        pass
    else:
        raise AssertionError("environment injection variable was accepted")
    temporary = Path(tempfile.gettempdir()).resolve(strict=True)
    profile = _profile(
        Path(__file__).resolve(strict=True), python_path, [temporary], temporary,
        (python_path, python_path), [("TEST", temporary / "cache")],
    )
    if (
        "(deny network*)" not in profile
        or "(deny process-exec)" not in profile
        or "(deny file-write*)" not in profile
        or os.fspath(Path("/bin/sh").resolve(strict=True)) not in profile
        or (Path("/bin/bash").exists() and "/bin/bash" not in profile)
    ):
        raise AssertionError("sandbox profile is not fail closed")
    digest = "sha256:" + "1" * 64
    content_digest = "sha256:" + "2" * 64
    evidence = {
        "schema": "codeclew-cas-object/2.0",
        "objectSchema": "codeclew-kotlin-thread-impact-evidence/1.0",
        "digest": digest,
        "size": 1,
    }
    content_ref = {
        "schema": "codeclew-cas-object/2.0",
        "objectSchema": "codeclew-repository-input-blob/2.0",
        "digest": content_digest,
        "size": 100,
    }
    source_anchor = {
        "path": "src/main/kotlin/com/acme/RelationFacts.kt",
        "start": 20,
        "end": 30,
        "contentRef": content_ref,
    }
    projected_shape = {
        "declarationKind": "FUNCTION",
        "symbolIdentity": "function:com/acme/publicDescriptor#jvm:(Ljava/lang/String;)Ljava/lang/String;",
        "compilerCallableId": "com/acme/publicDescriptor",
        "ownerIdentity": "package:com/acme",
    }
    shape_digest = pilot.authority_digest(projected_shape)
    expected_descriptor = {
        "descriptorClass": "CALLABLE",
        "declarationKind": "FUNCTION",
        "name": "publicDescriptor",
        "ownerIdentity": "package:com/acme",
        "normalizedSignature": projected_shape["symbolIdentity"],
        "shapeDigest": shape_digest,
        "relativeFile": source_anchor["path"],
        "blobOid": "1" * 40,
        "sourceRange": {"startByte": 20, "endByte": 30},
    }
    findings = []
    members = []
    source_windows = []
    for index, (alias, side) in enumerate(MEMBER_SIDES.items(), 3):
        finding_id = "sha256:" + str(index) * 64
        fact_id = "sha256:" + str(index + 2) * 64
        findings.append(
            {
                "findingId": finding_id,
                "side": side,
                "memberAlias": alias,
                "factId": fact_id,
                "authority": "EXACT_PROJECTED_DECLARATION",
                "shapeDigest": shape_digest,
                "source": source_anchor,
                "detail": {
                    "kind": "DECLARATION",
                    "detail": {
                        "declarationKind": "FUNCTION",
                        "symbolIdentity": projected_shape["symbolIdentity"],
                        "compilerCallableId": projected_shape["compilerCallableId"],
                        "projectedShape": projected_shape,
                    },
                },
            }
        )
        members.append(
            {
                "side": side,
                "memberAlias": alias,
                "observed": True,
                "matchedFindingCount": 1,
                "selectedFindingCount": 1,
                "declarationCount": 1,
                "useCount": 0,
                "boundaryCount": 0,
                "exactShapeDigest": shape_digest,
            }
        )
        source_windows.append(
            {
                "windowId": "sha256:" + str(index + 4) * 64,
                "side": side,
                "memberAlias": alias,
                "anchor": source_anchor,
                "spanBytes": 10,
                "findingIds": [finding_id],
            }
        )
    impact_id = f"thread-impact:{digest}"
    projection = {
        "schema": "codeclew-kotlin-thread-impact-projection/1.0",
        "impactId": impact_id,
        "authorityDigest": digest,
        "bindingDigest": digest,
        "factSetAuthorityDigest": digest,
        "pairId": "pair-warm",
        "subjectKind": "CALLABLE_FAMILY",
        "relationshipAuthority": "DECLARED_TOPOLOGY",
        "shapeStatus": "EXACT_PROJECTED_SHAPE_EQUAL",
        "certainty": "UNSURE",
        "members": members,
        "findingCount": 2,
        "sourceWindowCount": 2,
        "obligationCount": 1,
        "findingsTruncated": False,
        "sourceWindowsTruncated": False,
        "findings": findings,
        "publicFindingsTruncated": False,
        "obligations": [{"code": "VERIFY_RELATIONSHIP_AUTHORITY"}],
        "sourceWindows": source_windows,
        "evidenceRef": evidence,
    }
    impact_result = {
        "schema": "codeclew-thread-impact-result/1.0",
        "threadId": "thread:test",
        "threadAuthorityDigest": digest,
        "factSetId": "thread-callables:test",
        "factSetAuthorityDigest": digest,
        "impactId": impact_id,
        "authorityDigest": digest,
        "evidenceRef": evidence,
        "impact": projection,
    }
    _validate_warm_impact_result(
        impact_result,
        thread_id="thread:test",
        fact_set="thread-callables:test",
        subject=SUBJECT,
        expected_descriptor=expected_descriptor,
    )

    def expect_invalid_impact(candidate: dict[str, Any], label: str) -> None:
        try:
            _validate_warm_impact_result(
                candidate,
                thread_id="thread:test",
                fact_set="thread-callables:test",
                subject=SUBJECT,
                expected_descriptor=expected_descriptor,
            )
        except WarmAuditError:
            return
        raise AssertionError(f"{label} measured impact was accepted")

    substituted = json.loads(json.dumps(impact_result))
    substituted["impact"]["subjectKind"] = "TOKEN"
    expect_invalid_impact(substituted, "substituted")
    truncated = json.loads(json.dumps(impact_result))
    truncated["impact"]["findingsTruncated"] = True
    expect_invalid_impact(truncated, "truncated")
    empty = json.loads(json.dumps(impact_result))
    empty["impact"].update(
        {
            "members": [],
            "findingCount": 0,
            "sourceWindowCount": 0,
            "obligationCount": 0,
            "findings": [],
            "obligations": [],
            "sourceWindows": [],
        }
    )
    expect_invalid_impact(empty, "empty")
    stale = json.loads(json.dumps(impact_result))
    stale["impact"]["findings"][0]["source"]["start"] = 19
    expect_invalid_impact(stale, "stale-source")
    malformed = json.loads(json.dumps(impact_result))
    del malformed["impact"]["findings"][0]["detail"]["detail"]["symbolIdentity"]
    expect_invalid_impact(malformed, "malformed-nested")
    wrong_target = json.loads(json.dumps(impact_result))
    for finding in wrong_target["impact"]["findings"]:
        projected = finding["detail"]["detail"]["projectedShape"]
        projected["compilerCallableId"] = "com/acme/wrongTarget"
        projected["symbolIdentity"] = "function:com/acme/wrongTarget#jvm:()V"
        finding["detail"]["detail"]["compilerCallableId"] = projected["compilerCallableId"]
        finding["detail"]["detail"]["symbolIdentity"] = projected["symbolIdentity"]
        finding["shapeDigest"] = pilot.authority_digest(projected)
    expect_invalid_impact(wrong_target, "wrong-target")
    thread_lifecycle = {
        "schema": "codeclew-thread-lifecycle-result/1.0",
        "threadId": "thread:test",
        "lifecycle": {
            "schema": "codeclew-thread-lifecycle-entry/1.0",
            "threadId": "thread:test",
            "threadAuthorityDigest": digest,
            "sequence": 1,
            "previousEventHash": digest,
            "status": "CLOSED",
            "eventHash": digest,
            "updatedUnixMs": 1,
        },
    }
    _validate_lifecycle(
        thread_lifecycle,
        resource_kind="thread",
        resource_id="thread:test",
        status="CLOSED",
        garbage_collection=False,
    )
    substituted_lifecycle = json.loads(json.dumps(thread_lifecycle))
    substituted_lifecycle["lifecycle"]["threadId"] = "thread:other"
    try:
        _validate_lifecycle(
            substituted_lifecycle,
            resource_kind="thread",
            resource_id="thread:test",
            status="CLOSED",
            garbage_collection=False,
        )
    except WarmAuditError:
        pass
    else:
        raise AssertionError("substituted cleanup lifecycle was accepted")
    git_raw = shutil.which("git")
    if git_raw is None:
        raise AssertionError("git is required for the recursive fixture test")
    git = Path(git_raw).resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="codeclew-s4k-warm-self-test-") as directory:
        root = Path(directory).resolve(strict=True)
        os.chmod(root, 0o700)
        temporary_root = _temporary_root_identity(root)
        source = root / "source"
        nested = source / "fixtures" / "kotlin-basic" / "src" / "main" / "Sample.kt"
        nested.parent.mkdir(parents=True)
        nested.write_text("fun sample() = Unit\n", encoding="utf-8")
        _commit_fixture(source, git)
        destination = root / "destination"
        _, fixture_digest = _copy_tracked_fixture(source, destination, git)
        if (destination / "src" / "main" / "Sample.kt").read_bytes() != nested.read_bytes():
            raise AssertionError("tracked fixture copy is not recursive")
        second = root / "destination-two"
        if _copy_tracked_fixture(source, second, git)[1] != fixture_digest:
            raise AssertionError("tracked fixture digest is not deterministic")
        state = root / "state"
        (state / "locks").mkdir(parents=True)
        (state / "tmp").mkdir()
        (state / "objects").mkdir()
        lock = state / "locks" / "test.lock"
        data = state / "objects" / "nested" / "data"
        data.parent.mkdir()
        lock.write_bytes(b"a")
        data.write_bytes(b"a")
        before = _state_snapshot(state)
        lock.write_bytes(b"changed")
        if _state_snapshot(state) != before:
            raise AssertionError("lock-only state was included in the snapshot")
        metadata = data.stat()
        time.sleep(0.01)
        data.write_bytes(b"b")
        os.utime(data, ns=(metadata.st_atime_ns, metadata.st_mtime_ns))
        if _state_snapshot(state) == before:
            raise AssertionError("nested same-size restored-mtime mutation was not detected")
        tmp_data = state / "tmp" / "data"
        current = _state_snapshot(state)
        tmp_data.write_bytes(b"x")
        if _state_snapshot(state) == current:
            raise AssertionError("tmp state was incorrectly excluded")
        cache_roots: list[tuple[str, Path]] = []
        for label in ["CODECLEW_DEPENDENCY", "GRADLE", "MAVEN", "CARGO", "RUSTUP"]:
            cache = root / "cache" / label.lower()
            cache.mkdir(parents=True)
            cache_roots.append((label, cache))
        specs = _cache_sentinel_specs(cache_roots, pilot.authority_digest(sample))
        fake_authority = {
            "authorityDigest": pilot.authority_digest("warm-authority"),
            "executables": {"clewDigest": pilot.authority_digest("warm-clew")},
        }
        ledger = root / ".warm.resources-pending.json"
        test_records: list[dict[str, Any]] = []
        test_in_flight: dict[str, str] | None = None

        def test_ledger_value() -> dict[str, Any]:
            return _ledger_value(
                fake_authority,
                session_ids=[],
                thread_id=None,
                open_in_flight=None,
                temporary_root=temporary_root,
                sentinels=test_records,
                sentinel_in_flight=test_in_flight,
            )

        initial = _ledger_value(
            fake_authority,
            session_ids=[],
            thread_id=None,
            open_in_flight=None,
            temporary_root=temporary_root,
            sentinels=[],
            sentinel_in_flight=None,
        )
        _create_ledger(ledger, initial)
        _, stored, _ = pilot.private_json(ledger, "WARM_RESOURCE_LEDGER", 256 * 1024)
        _validate_ledger(stored, fake_authority, specs)

        def test_begin(path: Path, body_digest: str) -> None:
            nonlocal test_in_flight
            test_in_flight = {"path": os.fspath(path), "bodyDigest": body_digest}
            _update_ledger(ledger, test_ledger_value())

        def test_record(identity: dict[str, Any]) -> None:
            nonlocal test_in_flight
            test_records.append(identity)
            test_in_flight = None
            _update_ledger(ledger, test_ledger_value())

        sentinels, sentinel_digest = _cache_sentinels(
            specs, test_begin, test_record
        )
        if len(sentinels) != 5 or SHA256.fullmatch(sentinel_digest) is None:
            raise AssertionError("cache sentinels are incomplete")
        _, stored, _ = pilot.private_json(ledger, "WARM_RESOURCE_LEDGER", 256 * 1024)
        _validate_ledger(stored, fake_authority, specs)
        _remove_recovery_sentinels(
            test_records, specs, allow_missing=False
        )
        if any(path.exists() for path in sentinels):
            raise AssertionError("cache sentinels were not cleaned")
        updated = _ledger_value(
            fake_authority,
            session_ids=["session:test-a", "session:test-b", "session:test-c"],
            thread_id="thread:test",
            open_in_flight=None,
            temporary_root=temporary_root,
            sentinels=test_records,
            sentinel_in_flight=None,
        )
        _update_ledger(ledger, updated)
        _, stored, _ = pilot.private_json(ledger, "WARM_RESOURCE_LEDGER", 256 * 1024)
        _validate_ledger(stored, fake_authority, specs)
        _remove_ledger(ledger)
        if ledger.exists():
            raise AssertionError("resource ledger was not removed")

        preexisting_path, preexisting_body, _ = specs[0]
        preexisting_path.write_bytes(b"")
        os.chmod(preexisting_path, 0o600)
        try:
            _cache_sentinels(specs[:1], lambda *_: None, lambda *_: None)
        except WarmAuditError:
            pass
        else:
            raise AssertionError("preexisting empty cache file was overwritten")
        if preexisting_path.read_bytes() != b"":
            raise AssertionError("preexisting empty cache file was mutated")

        prefix_path, prefix_body, _ = specs[1]
        prefix_path.write_bytes(prefix_body[:7])
        os.chmod(prefix_path, 0o600)
        try:
            _cache_sentinels(specs[1:2], lambda *_: None, lambda *_: None)
        except WarmAuditError:
            pass
        else:
            raise AssertionError("preexisting prefix cache file was overwritten")
        if prefix_path.read_bytes() != prefix_body[:7]:
            raise AssertionError("preexisting prefix cache file was mutated")

        exact_path, exact_body, exact_digest = specs[2]
        exact_records: list[dict[str, Any]] = []
        _cache_sentinels(
            [(exact_path, exact_body, exact_digest)],
            lambda *_: None,
            exact_records.append,
        )
        exact_path.unlink()
        exact_path.write_bytes(exact_body)
        os.chmod(exact_path, 0o600)
        try:
            _remove_recovery_sentinels(
                exact_records,
                [(exact_path, exact_body, exact_digest)],
                allow_missing=False,
            )
        except WarmAuditError:
            pass
        else:
            raise AssertionError("replacement cache inode was deleted")
        if exact_path.read_bytes() != exact_body:
            raise AssertionError("replacement cache inode was mutated")

        output = root / "warm-output.json"
        output_value = {"schema": "self-test/1.0", "status": "PASS"}
        output_identity = _create_output_once(output, output_value)
        _validate_output_exact(output, output_value, output_identity)
        try:
            _create_output_once(output, {"schema": "replacement/1.0"})
        except WarmAuditError as error:
            if error.code != "WARM_OUTPUT_ALREADY_EXISTS":
                raise
        else:
            raise AssertionError("warm output was overwritten")
        _validate_output_exact(output, output_value, output_identity)

        sandbox_path = Path("/usr/bin/sandbox-exec")
        if sys.platform == "darwin" and sandbox_path.is_file():
            denied_source = root / "denied-source"
            denied_source.mkdir()
            denied_source_file = denied_source / "Source.kt"
            denied_source_file.write_bytes(b"source")
            denied_cache = root / "denied-cache"
            denied_cache.mkdir()
            denied_cache_file = denied_cache / "canary"
            denied_cache_file.write_bytes(b"cache")
            sandbox_state = root / "sandbox-state"
            (sandbox_state / "locks").mkdir(parents=True)
            denied_write = root / "denied-write"
            denied_write.write_bytes(b"")
            sandbox_profile = _profile(
                Path(__file__).resolve(strict=True),
                python_path,
                [denied_source],
                sandbox_state,
                (python_path, python_path),
                [("TEST", denied_cache)],
            )
            sandbox_environment = {
                "HOME": normalized_home,
                "PATH": sealed_path,
                "LANG": "C",
                "LC_ALL": "C",
            }
            if not all(
                _canaries(
                    sandbox_path,
                    sandbox_profile,
                    [denied_cache_file],
                    denied_source_file,
                    denied_write,
                    (python_path, python_path),
                    sandbox_environment,
                )
            ):
                raise AssertionError("actual Seatbelt canaries did not fail closed")

    try:
        _run(
            [
                os.fspath(python_path), "-I", "-S", "-c",
                "import os;os.write(1,b'x'*65537)",
            ],
            timeout=5,
            maximum=1024,
        )
    except WarmAuditError as error:
        if error.code != "WARM_SUBPROCESS_FAILED":
            raise
    else:
        raise AssertionError("oversized subprocess output crossed the hard cap")

    dual_megabytes = 2 * 1024 * 1024
    dual_script = (
        "import sys;"
        f"sys.stdout.buffer.write(b'o'*{dual_megabytes});sys.stdout.buffer.flush();"
        f"sys.stderr.buffer.write(b'e'*{dual_megabytes});sys.stderr.buffer.flush()"
    )
    dual_process = subprocess.Popen(
        [os.fspath(python_path), "-I", "-S", "-c", dual_script],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
        close_fds=True,
    )
    dual_stdout, dual_stderr = _bounded_capture(
        dual_process,
        timeout=10,
        stdout_limit=dual_megabytes,
        stderr_limit=dual_megabytes,
        failure_code="WARM_SUBPROCESS_FAILED",
    )
    if (
        dual_stdout != b"o" * dual_megabytes
        or dual_stderr != b"e" * dual_megabytes
        or _process_group_exists(dual_process.pid)
    ):
        raise AssertionError("selector capture did not drain bounded dual output")

    continuous_script = (
        "import os\n"
        "while True:\n"
        " os.write(1,b'o'*65536)\n"
        " os.write(2,b'e'*65536)\n"
    )
    continuous = subprocess.Popen(
        [os.fspath(python_path), "-I", "-S", "-c", continuous_script],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
        close_fds=True,
    )
    try:
        _bounded_capture(
            continuous,
            timeout=5,
            stdout_limit=256 * 1024,
            stderr_limit=256 * 1024,
            failure_code="WARM_SUBPROCESS_FAILED",
        )
    except WarmAuditError as error:
        if error.code != "WARM_SUBPROCESS_FAILED":
            raise
    else:
        raise AssertionError("continuous dual output crossed the hard cap")
    if _process_group_exists(continuous.pid):
        raise AssertionError("continuous-output process group survived refusal")

    with tempfile.TemporaryDirectory(prefix="codeclew-warm-process-test-") as directory:
        pid_file = Path(directory) / "child.pid"
        child_script = (
            "import pathlib,subprocess,sys; "
            "p=subprocess.Popen([sys.executable,'-I','-S','-c','import time;time.sleep(30)'],"
            "stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); "
            "pathlib.Path(sys.argv[1]).write_text(str(p.pid),encoding='ascii')"
        )
        try:
            _run(
                [
                    os.fspath(python_path), "-I", "-S", "-c", child_script,
                    os.fspath(pid_file),
                ],
                timeout=5,
            )
        except WarmAuditError as error:
            if error.code != "SANDBOX_RESIDUAL_PROCESS":
                raise
        else:
            raise AssertionError("successful cold command left an accepted process group")
        child_pid = int(pid_file.read_text(encoding="ascii"))
    child_alive = True
    for _ in range(50):
        try:
            os.kill(child_pid, 0)
        except OSError:
            child_alive = False
            break
        time.sleep(0.02)
    if child_alive:
        raise AssertionError("residual process-group child survived termination")


def fixture_digest(args: argparse.Namespace) -> dict[str, Any]:
    source = args.source_repo.resolve(strict=True)
    clew, _ = pilot.executable(args.clew, "INVALID_CLEW_EXECUTABLE")
    git, _ = pilot.executable(args.git, "INVALID_GIT_EXECUTABLE")
    if source != clew.parent:
        raise WarmAuditError("FIXTURE_SOURCE_AUTHORITY_MISMATCH")
    with tempfile.TemporaryDirectory(prefix="codeclew-warm-fixture-digest-") as directory:
        tree, digest = _copy_tracked_fixture(source, Path(directory) / "fixture", git)
    return {
        "schema": "codeclew-kotlin-warm-fixture-digest/1.0",
        "status": "SEALED",
        "trackedTreeOid": tree,
        "fixtureDigest": digest,
    }


def fixture_parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser(description="Seal the tracked warm fixture")
    value.add_argument("--source-repo", type=Path, required=True)
    value.add_argument("--clew", type=Path, required=True)
    value.add_argument("--git", type=Path, required=True)
    return value


def parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser(description=__doc__)
    value.add_argument("--private-authority", type=Path)
    value.add_argument("--private-oracle", type=Path)
    value.add_argument("--source-repo", type=Path)
    value.add_argument("--private-output", type=Path)
    value.add_argument("--self-test", action="store_true")
    return value


def main(argv: list[str] | None = None) -> int:
    raw_arguments = list(sys.argv[1:] if argv is None else argv)
    try:
        if raw_arguments[:1] == ["fixture-digest"]:
            result = fixture_digest(fixture_parser().parse_args(raw_arguments[1:]))
            print(pilot.canonical_bytes(result).decode("utf-8"))
            return 0
        args = parser().parse_args(raw_arguments)
        if args.self_test:
            self_test()
            print(json.dumps({"adapter": ADAPTER, "selfTest": "PASS"}, sort_keys=True))
            return 0
        for name in ["private_authority", "private_oracle", "source_repo", "private_output"]:
            if getattr(args, name) is None:
                parser().error(f"--{name.replace('_', '-')} is required")
        result = run_audit(args)
    except WarmAuditError as error:
        print(f"FAIL: {error.code}", file=sys.stderr)
        return 1
    except (
        pilot.PilotError,
        descriptor_gate.GateError,
        OSError,
        TypeError,
        ValueError,
        KeyError,
        IndexError,
    ):
        print("FAIL: S4K_WARM_AUDIT_FAILED", file=sys.stderr)
        return 1
    print(pilot.canonical_bytes(result).decode("utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
