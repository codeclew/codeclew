#!/usr/bin/env python3
"""Derive one private pilot case record from managed Codeclew artifacts."""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import secrets


ROOT = Path(__file__).resolve().parent.parent
MAX_ARTIFACT_BYTES = 16 * 1024 * 1024
CASE_ID = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}$")
OID = re.compile(r"^[0-9a-f]{40,64}$")
PRIVATE_PATH = re.compile(
    br"/(?:Users|home|tmp|var|private|workspace|workspaces|mnt|Volumes|opt|srv|data|run|nix)/[^\s\"']+"
)
EMAIL = re.compile(br"[A-Z0-9._%+-]+@([A-Z0-9.-]+\.[A-Z]{2,})", re.IGNORECASE)
SECRET_PATTERNS = (
    re.compile(br"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    re.compile(br"(?:AKIA|ASIA)[0-9A-Z]{16}"),
    re.compile(br"gh[pousr]_[A-Za-z0-9_]{20,}"),
    re.compile(br"xox[baprs]-[A-Za-z0-9-]{10,}"),
    re.compile(br"(?:sk-proj-[A-Za-z0-9_-]{20,}|sk-[A-Za-z0-9]{32,})"),
    re.compile(br"https?://[^\s/:@]+:[^\s@/]+@"),
    re.compile(br"authorization[\"'=:\s]+bearer\s+[A-Za-z0-9_./+\-]{8,}", re.IGNORECASE),
)


class RecordError(Exception):
    pass


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def read_private_key(path: Path) -> bytes:
    if not path.is_absolute():
        raise RecordError("attestation key path must be absolute")
    try:
        metadata = path.lstat()
        resolved = path.resolve(strict=True)
    except OSError as error:
        raise RecordError("attestation key is unavailable") from error
    if (
        resolved != path
        or not stat.S_ISREG(metadata.st_mode)
        or metadata.st_mode & 0o077
        or metadata.st_size > 1024
    ):
        raise RecordError("attestation key must be a physical private file")
    try:
        resolved.relative_to(ROOT)
    except ValueError:
        pass
    else:
        raise RecordError("attestation key must be outside the repository")
    try:
        value = json.loads(path.read_bytes())
        encoded = value.get("keyHex") if isinstance(value, dict) else None
        if (
            not isinstance(value, dict)
            or set(value) != {"keyHex", "schema"}
            or value.get("schema") != "codeclew-pilot-attestation-key/1.0"
            or not isinstance(encoded, str)
            or not re.fullmatch(r"[0-9a-f]{64}", encoded)
        ):
            raise RecordError("attestation key schema is invalid")
        return bytes.fromhex(encoded)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise RecordError("attestation key schema is invalid") from error


def pilot_id(key: bytes) -> str:
    return "sha256:" + hashlib.sha256(key).hexdigest()


def attest_case(case: dict[str, object], key: bytes, evidence_digest: str) -> dict[str, object]:
    result = dict(case)
    result["evidenceDigest"] = evidence_digest
    result["pilotId"] = pilot_id(key)
    result["attestation"] = "hmac-sha256:" + hmac.new(
        key, canonical(result), hashlib.sha256
    ).hexdigest()
    return result


def read_private_json(path: Path) -> tuple[dict[str, object], bytes]:
    if not path.is_absolute():
        raise RecordError("artifact path must be absolute")
    try:
        metadata = path.lstat()
        resolved = path.resolve(strict=True)
    except OSError as error:
        raise RecordError("artifact is unavailable") from error
    if resolved != path or not stat.S_ISREG(metadata.st_mode) or metadata.st_mode & 0o077:
        raise RecordError("artifact must be a physical private file")
    try:
        resolved.relative_to(ROOT)
    except ValueError:
        pass
    else:
        raise RecordError("artifact must be outside the repository")
    if metadata.st_size <= 0 or metadata.st_size > MAX_ARTIFACT_BYTES:
        raise RecordError("artifact is empty or oversized")
    try:
        data = path.read_bytes()
        value = json.loads(data)
    except (OSError, json.JSONDecodeError, UnicodeDecodeError) as error:
        raise RecordError("artifact is not valid JSON") from error
    if not isinstance(value, dict):
        raise RecordError("artifact must be an object")
    return value, data


def run_row(value: dict[str, object]) -> dict[str, object]:
    row = value.get("run")
    required = [
        "contextId", "ledgerHead", "planId", "requestDigest", "runId",
        "sessionId", "transactionId",
    ]
    if (
        not isinstance(row, dict)
        or any(not isinstance(row.get(key), str) for key in required)
        or not isinstance(row.get("sequence"), int)
    ):
        raise RecordError("artifact has no run authority")
    return row


def same_run_request(left: dict[str, object], right: dict[str, object]) -> bool:
    return all(
        left.get(key) == right.get(key)
        for key in ["contextId", "planId", "requestDigest", "sessionId", "transactionId"]
    )


def require_schema(value: dict[str, object], expected: str) -> None:
    if value.get("schema") != expected:
        raise RecordError("artifact schema is invalid")


def validation_passed(terminal: dict[str, object]) -> bool:
    candidate = terminal.get("candidate")
    if not isinstance(candidate, dict):
        return False
    evidence = candidate.get("validationEvidence")
    return (
        isinstance(evidence, list)
        and bool(evidence)
        and all(
            isinstance(row, dict)
            and row.get("launcher") == "GRADLE"
            and row.get("success") is True
            for row in evidence
        )
    )


def typed_failure(row: dict[str, object], status: str) -> str | None:
    failure = row.get("failure")
    if isinstance(failure, dict) and isinstance(failure.get("code"), str):
        return str(failure["code"])
    return {
        "CANCELLED": "CANCELLED",
        "VALIDATED_CONDITIONAL": "INCOMPLETE_SEMANTIC_ANALYSIS",
        "WORKTREE_RECOVERY_REQUIRED": "WORKTREE_RECOVERY_REQUIRED",
    }.get(status)


def derive_case(
    *,
    case_id: str,
    opened: dict[str, object],
    prepared_first: dict[str, object],
    prepared_retry: dict[str, object],
    terminal: dict[str, object],
    published_first: dict[str, object] | None,
    published_retry: dict[str, object] | None,
    durations: dict[str, int],
    repository_facts: dict[str, object],
    prepublish_snapshot: dict[str, object],
    manual_cleanup_used: bool,
    private_data_leak: bool,
) -> dict[str, object]:
    if not CASE_ID.fullmatch(case_id):
        raise RecordError("case ID is invalid")
    require_schema(opened, "codeclew-change-open/1.0")
    require_schema(prepared_first, "codeclew-change-prepare/1.0")
    require_schema(prepared_retry, "codeclew-change-prepare/1.0")
    require_schema(terminal, "codeclew-change-status/1.0")
    require_schema(prepublish_snapshot, "codeclew-pilot-source-snapshot/1.0")
    session = opened.get("session")
    context_result = opened.get("context")
    projection = context_result.get("context") if isinstance(context_result, dict) else None
    compiler_versions = (
        projection.get("compilerVersions") if isinstance(projection, dict) else None
    )
    if (
        not isinstance(session, dict)
        or session.get("baseRevision") != session.get("targetOid")
        or not isinstance(session.get("baseRevision"), str)
        or not OID.fullmatch(str(session.get("baseRevision")))
        or session.get("runtimeMode") not in {"DEVELOPMENT", "RELEASE"}
        or not isinstance(session.get("sessionId"), str)
        or not isinstance(session.get("repositoryKey"), str)
        or not re.fullmatch(r"[0-9a-f]{64}", str(session.get("repositoryKey")))
        or not isinstance(session.get("targetRef"), str)
        or not isinstance(session.get("compilations"), list)
        or len(session["compilations"]) != 1
        or session.get("modelCachePolicy") != "NON_CACHEABLE"
        or not isinstance(compiler_versions, dict)
        or set(compiler_versions) != set(session["compilations"])
        or set(compiler_versions.values()) != {"2.4.10"}
    ):
        raise RecordError("open artifact authority is invalid")
    first_run = run_row(prepared_first)
    retry_run = run_row(prepared_retry)
    terminal_run = run_row(terminal)
    run_id = first_run["runId"]
    session_id = session["sessionId"]
    if (
        not isinstance(context_result, dict)
        or context_result.get("schema") != "codeclew-context-result/2.0"
        or context_result.get("sessionId") != session_id
        or context_result.get("contextId") != first_run.get("contextId")
    ):
        raise RecordError("open context differs from run authority")
    for artifact, row in [
        (prepared_first, first_run), (prepared_retry, retry_run)
    ]:
        if any(
            artifact.get(key) != row.get(key)
            for key in ["sessionId", "contextId", "planId"]
        ):
            raise RecordError("prepare result differs from run authority")
    if any(
        row.get("sessionId") != session_id or not same_run_request(first_run, row)
        for row in [retry_run, terminal_run]
    ):
        raise RecordError("run belongs to a different session")
    idempotent = retry_run["runId"] == run_id == terminal_run["runId"]
    status = str(terminal_run.get("status"))
    candidate = terminal.get("candidate")
    candidate_commit = candidate.get("candidateCommit") if isinstance(candidate, dict) else None
    prepared = isinstance(candidate_commit, str) and OID.fullmatch(candidate_commit) is not None
    candidate_digest = (
        candidate.get("preparedAuthorityDigest") if isinstance(candidate, dict) else None
    )
    if prepared and (
        terminal_run.get("candidateCommit") != candidate_commit
        or terminal_run.get("preparedAuthorityDigest") != candidate_digest
    ):
        raise RecordError("terminal public candidate differs from run authority")
    validated = validation_passed(terminal)
    final_commit = None
    if published_first is not None or published_retry is not None:
        if published_first is None or published_retry is None:
            raise RecordError("publication retry evidence is incomplete")
        require_schema(published_first, "codeclew-change-publish/1.0")
        require_schema(published_retry, "codeclew-change-publish/1.0")
        first_published_run = run_row(published_first)
        retry_published_run = run_row(published_retry)
        if any(
            row.get("sessionId") != session_id
            or row["runId"] != run_id
            or not same_run_request(first_run, row)
            for row in [first_published_run, retry_published_run]
        ):
            raise RecordError("publication run authority differs from preparation")
        if first_published_run.get("status") not in {"PUBLISHED", "PUBLISHED_CONDITIONAL"}:
            raise RecordError("publication artifact is not published")
        final_commit = first_published_run.get("finalCommit")
        outcome = str(first_published_run["status"])
        expected_ready = (
            "READY_TO_PUBLISH_CONDITIONAL"
            if outcome == "PUBLISHED_CONDITIONAL" else "READY_TO_PUBLISH"
        )
        if (
            not isinstance(final_commit, str)
            or OID.fullmatch(final_commit) is None
            or final_commit != candidate_commit
            or status != expected_ready
            or any(
                row.get("candidateCommit") != candidate_commit
                or row.get("preparedAuthorityDigest") != candidate_digest
                for row in [first_published_run, retry_published_run]
            )
        ):
            raise RecordError("publication does not match prepared candidate authority")
        idempotent = (
            idempotent
            and retry_published_run.get("status") == outcome
            and retry_published_run.get("finalCommit") == final_commit
        )
        error_code = None
    else:
        outcome = "RECOVERY_REQUIRED" if status == "WORKTREE_RECOVERY_REQUIRED" else status
        if outcome not in {"VALIDATED_CONDITIONAL", "FAILED", "CANCELLED", "RECOVERY_REQUIRED"}:
            raise RecordError("terminal artifact requires publication evidence")
        error_code = typed_failure(terminal_run, status)
        if error_code is None:
            raise RecordError("non-published outcome has no typed error")
    base = session["baseRevision"]
    binding = managed_snapshot_binding(opened, terminal)
    authority_matches = (
        prepublish_snapshot.get("repoAuthority") == repository_facts.get("repoAuthority")
        and session["repositoryKey"] == repository_facts.get("repositoryKey")
        and prepublish_snapshot.get("targetRef") == session["targetRef"]
        and repository_facts.get("targetRef") == session["targetRef"]
        and prepublish_snapshot.get("buildSystem") == "GRADLE_WRAPPER"
        and repository_facts.get("buildSystem") == "GRADLE_WRAPPER"
        and all(prepublish_snapshot.get(key) == value for key, value in binding.items())
    )
    source_preserved = (
        authority_matches
        and prepublish_snapshot.get("baseRevision") == base
        and prepublish_snapshot.get("head") == base
        and prepublish_snapshot.get("targetOid") == base
        and prepublish_snapshot.get("clean") is True
    )
    expected_final = base if final_commit is None else final_commit
    post_state_valid = (
        authority_matches
        and repository_facts.get("head") == expected_final
        and repository_facts.get("targetOid") == expected_final
        and repository_facts.get("clean") is True
        and repository_facts.get("commitCount") == (0 if final_commit is None else 1)
    )
    if not post_state_valid:
        raise RecordError("repository state contradicts the managed outcome")
    recovery_resolved = outcome != "RECOVERY_REQUIRED"
    return {
        "caseId": case_id,
        "durationsMs": durations,
        "errorCode": error_code,
        "idempotentRetry": idempotent,
        "outcome": outcome,
        "preparedWithoutManualCleanup": prepared and not manual_cleanup_used,
        "privateDataLeak": private_data_leak,
        "projectClass": "kotlin24-gradle-single-compilation",
        "recoveryResolved": recovery_resolved,
        "runtimeMode": session["runtimeMode"],
        "schema": "codeclew-pilot-case/1.0",
        "sourcePreservedBeforePublish": source_preserved,
        "typedOutcome": error_code is not None or final_commit is not None,
        "validationPassed": validated,
    }


def git(repository: Path, *arguments: str) -> str:
    completed = subprocess.run(
        ["git", *arguments], cwd=repository, check=False,
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        text=True,
    )
    if completed.returncode != 0:
        raise RecordError("Git authority is unavailable")
    return completed.stdout.strip()


def repository_facts(repository: Path, target_ref: str, base: str) -> dict[str, object]:
    resolved = repository.resolve(strict=True)
    if resolved != repository or not repository.is_dir():
        raise RecordError("repository path must be physical and canonical")
    head = git(repository, "rev-parse", "HEAD")
    qualified_ref = git(repository, "rev-parse", "--symbolic-full-name", target_ref)
    if not qualified_ref.startswith("refs/"):
        raise RecordError("target ref is not a qualified Git ref")
    target_oid = git(repository, "rev-parse", qualified_ref)
    clean = git(repository, "status", "--porcelain=v1", "--untracked-files=all") == ""
    count = int(git(repository, "rev-list", "--count", f"{base}..{head}"))
    common_dir = Path(git(repository, "rev-parse", "--path-format=absolute", "--git-common-dir"))
    common_metadata = common_dir.stat()
    wrapper = git(repository, "ls-files", "--stage", "--", "gradlew").split()
    if len(wrapper) < 4 or wrapper[0] != "100755" or wrapper[-1] != "gradlew":
        raise RecordError("supported pilot requires a tracked executable Gradle wrapper")
    object_format = git(repository, "rev-parse", "--show-object-format")
    root_commit = git(repository, "rev-list", "--max-parents=0", "HEAD").splitlines()[0]
    authority = hashlib.sha256()
    authority.update(b"codeclew-repo/v1\0")
    authority.update(common_metadata.st_dev.to_bytes(8, "big"))
    authority.update(common_metadata.st_ino.to_bytes(8, "big"))
    authority.update(b"\0")
    authority.update(object_format.encode())
    authority.update(b"\0")
    authority.update(root_commit.encode())
    repository_key = authority.hexdigest()
    return {
        "clean": clean,
        "buildSystem": "GRADLE_WRAPPER",
        "commitCount": count,
        "head": head,
        "repoAuthority": "sha256:" + repository_key,
        "repositoryKey": repository_key,
        "targetOid": target_oid,
        "targetRef": qualified_ref,
    }


def managed_snapshot_binding(
    opened: dict[str, object], terminal: dict[str, object]
) -> dict[str, object]:
    require_schema(opened, "codeclew-change-open/1.0")
    require_schema(terminal, "codeclew-change-status/1.0")
    session = opened.get("session")
    row = run_row(terminal)
    candidate = terminal.get("candidate")
    if not isinstance(session, dict):
        raise RecordError("snapshot has no managed authority")
    prepared_digest = (
        candidate.get("preparedAuthorityDigest") if isinstance(candidate, dict) else None
    )
    candidate_commit = candidate.get("candidateCommit") if isinstance(candidate, dict) else None
    ready = row.get("status") in {
        "READY_TO_PUBLISH", "READY_TO_PUBLISH_CONDITIONAL", "VALIDATED_CONDITIONAL"
    }
    if (
        row.get("sessionId") != session.get("sessionId")
        or not isinstance(session.get("repositoryKey"), str)
        or (ready and (
            not isinstance(prepared_digest, str)
            or not prepared_digest.startswith("sha256:")
            or not isinstance(candidate_commit, str)
            or OID.fullmatch(candidate_commit) is None
        ))
        or row.get("candidateCommit") != candidate_commit
        or row.get("preparedAuthorityDigest") != prepared_digest
    ):
        raise RecordError("snapshot managed authority is invalid")
    return {
        "candidateCommit": candidate_commit,
        "contextId": row["contextId"],
        "ledgerHead": row["ledgerHead"],
        "planId": row["planId"],
        "preparedAuthorityDigest": prepared_digest,
        "repositoryKey": session["repositoryKey"],
        "requestDigest": row["requestDigest"],
        "runId": row["runId"],
        "runSequence": row["sequence"],
        "sessionId": row["sessionId"],
        "transactionId": row["transactionId"],
    }


def source_snapshot(
    facts: dict[str, object], base: str, binding: dict[str, object]
) -> dict[str, object]:
    result = {
        "baseRevision": base,
        "buildSystem": facts["buildSystem"],
        "clean": facts["clean"],
        "head": facts["head"],
        "repoAuthority": facts["repoAuthority"],
        "physicalRepositoryKey": facts["repositoryKey"],
        "schema": "codeclew-pilot-source-snapshot/1.0",
        "targetOid": facts["targetOid"],
        "targetRef": facts["targetRef"],
    }
    result.update(binding)
    return result


def artifact_leaks(data: list[bytes], repository: Path) -> bool:
    repository_bytes = str(repository).encode()
    for value in data:
        if (
            repository_bytes in value
            or PRIVATE_PATH.search(value)
            or any(pattern.search(value) for pattern in SECRET_PATTERNS)
        ):
            return True
        if any(not match.group(1).lower().endswith(b".invalid") for match in EMAIL.finditer(value)):
            return True
    return False


def write_private_record(path: Path, record: dict[str, object]) -> str:
    return write_private_bytes(path, canonical(record) + b"\n")


def write_private_bytes(path: Path, data: bytes) -> str:
    if not path.is_absolute() or path.exists() or path.parent.resolve(strict=True) != path.parent:
        raise RecordError("output must be a new physical absolute path")
    try:
        path.resolve().relative_to(ROOT)
    except ValueError:
        pass
    else:
        raise RecordError("output must be outside the repository")
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
    except Exception:
        try:
            os.unlink(path)
        except OSError:
            pass
        raise
    return "sha256:" + hashlib.sha256(data).hexdigest()


def record(arguments: argparse.Namespace) -> str:
    key = read_private_key(arguments.attestation_key)
    opened, opened_bytes = read_private_json(arguments.opened)
    first, first_bytes = read_private_json(arguments.prepared_first)
    retry, retry_bytes = read_private_json(arguments.prepared_retry)
    terminal, terminal_bytes = read_private_json(arguments.terminal)
    snapshot, snapshot_bytes = read_private_json(arguments.prepublish_snapshot)
    published_first, published_first_bytes = (
        read_private_json(arguments.published_first)
        if arguments.published_first is not None else (None, b"")
    )
    published_retry, published_retry_bytes = (
        read_private_json(arguments.published_retry)
        if arguments.published_retry is not None else (None, b"")
    )
    session = opened.get("session")
    if not isinstance(session, dict) or not isinstance(session.get("baseRevision"), str):
        raise RecordError("open artifact has no base authority")
    repository = arguments.repo.resolve(strict=True)
    facts = repository_facts(repository, arguments.target_ref, str(session["baseRevision"]))
    durations = {
        "open": arguments.open_ms,
        "prepareToReady": arguments.prepare_ms,
        "publish": arguments.publish_ms,
        "total": arguments.total_ms,
    }
    if (
        any(value < 0 or value > 86_400_000 for value in durations.values())
        or durations["total"]
        < sum(durations[key] for key in ["open", "prepareToReady", "publish"])
    ):
        raise RecordError("duration is invalid")
    raw = [opened_bytes, first_bytes, retry_bytes, terminal_bytes, snapshot_bytes,
           published_first_bytes, published_retry_bytes]
    result = derive_case(
        case_id=arguments.case_id, opened=opened, prepared_first=first,
        prepared_retry=retry, terminal=terminal, published_first=published_first,
        published_retry=published_retry, durations=durations,
        repository_facts=facts, prepublish_snapshot=snapshot,
        manual_cleanup_used=arguments.manual_cleanup_used,
        private_data_leak=artifact_leaks(raw, repository),
    )
    artifact_names = [
        "opened", "preparedFirst", "preparedRetry", "terminal", "snapshot",
        "publishedFirst", "publishedRetry",
    ]
    evidence_manifest = {
        "artifactDigests": {
            name: "sha256:" + hashlib.sha256(data).hexdigest()
            for name, data in zip(artifact_names, raw, strict=True)
        },
        "repositoryFacts": facts,
        "schema": "codeclew-pilot-case-evidence-binding/1.0",
    }
    evidence_digest = "sha256:" + hashlib.sha256(canonical(evidence_manifest)).hexdigest()
    return write_private_record(
        arguments.output, attest_case(result, key, evidence_digest)
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    key_parser = commands.add_parser("keygen")
    key_parser.add_argument("--output", type=Path, required=True)
    snapshot_parser = commands.add_parser("snapshot")
    snapshot_parser.add_argument("--repo", type=Path, required=True)
    snapshot_parser.add_argument("--opened", type=Path, required=True)
    snapshot_parser.add_argument("--terminal", type=Path, required=True)
    snapshot_parser.add_argument("--output", type=Path, required=True)
    record_parser = commands.add_parser("record")
    record_parser.add_argument("--case-id", required=True)
    record_parser.add_argument("--repo", type=Path, required=True)
    record_parser.add_argument("--target-ref", required=True)
    record_parser.add_argument("--opened", type=Path, required=True)
    record_parser.add_argument("--prepared-first", type=Path, required=True)
    record_parser.add_argument("--prepared-retry", type=Path, required=True)
    record_parser.add_argument("--terminal", type=Path, required=True)
    record_parser.add_argument("--prepublish-snapshot", type=Path, required=True)
    record_parser.add_argument("--published-first", type=Path)
    record_parser.add_argument("--published-retry", type=Path)
    record_parser.add_argument("--open-ms", type=int, required=True)
    record_parser.add_argument("--prepare-ms", type=int, required=True)
    record_parser.add_argument("--publish-ms", type=int, required=True)
    record_parser.add_argument("--total-ms", type=int, required=True)
    record_parser.add_argument("--manual-cleanup-used", action="store_true")
    record_parser.add_argument("--attestation-key", type=Path, required=True)
    record_parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    try:
        if arguments.command == "keygen":
            key = secrets.token_bytes(32)
            digest = write_private_record(arguments.output, {
                "keyHex": key.hex(),
                "schema": "codeclew-pilot-attestation-key/1.0",
            })
            result = {
                "keyDigest": digest,
                "pilotId": pilot_id(key),
                "status": "KEY_RECORDED",
            }
        elif arguments.command == "snapshot":
            opened, _ = read_private_json(arguments.opened)
            terminal, _ = read_private_json(arguments.terminal)
            session = opened.get("session")
            if not isinstance(session, dict):
                raise RecordError("open artifact has no session authority")
            repository = arguments.repo.resolve(strict=True)
            facts = repository_facts(
                repository, str(session.get("targetRef")), str(session.get("baseRevision"))
            )
            if facts.get("repositoryKey") != session.get("repositoryKey"):
                raise RecordError("open session belongs to a different physical repository")
            digest = write_private_record(
                arguments.output,
                source_snapshot(
                    facts, str(session.get("baseRevision")),
                    managed_snapshot_binding(opened, terminal),
                ),
            )
            result = {"snapshotDigest": digest, "status": "SNAPSHOT_RECORDED"}
        else:
            digest = record(arguments)
            result = {"caseDigest": digest, "status": "RECORDED"}
    except (OSError, ValueError, RecordError):
        print(json.dumps({"errorCode":"INVALID_CASE_EVIDENCE",
            "schema":"codeclew-pilot-case-record-result/1.0","status":"INVALID"},
            sort_keys=True, separators=(",", ":")))
        return 2
    result["schema"] = "codeclew-pilot-case-record-result/1.0"
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
