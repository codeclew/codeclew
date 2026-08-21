#!/usr/bin/env python3
"""Durable, idempotent launcher for one long-running ``clew task-apply``.

The public ``start`` command returns after a short handshake.  A detached
supervisor owns the operation lock and writes an immutable completion receipt.
``status`` never starts or retries work and never invokes the mutating
``clew tx inspect`` recovery path; when an interrupted outcome cannot be proven
from the task artifact it returns UNKNOWN_REQUIRES_INSPECTION with the exact
inspection argv.
"""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import secrets
import stat
import subprocess
import sys
import time
from typing import Any, BinaryIO
import uuid


REQUEST_SCHEMA = "semantic-task-apply-run-request/0.1"
LAUNCH_SCHEMA = "semantic-task-apply-run-launch/0.1"
CHILD_SCHEMA = "semantic-task-apply-run-child/0.1"
STATUS_SCHEMA = "semantic-task-apply-run-status/0.1"
COMPLETION_SCHEMA = "semantic-task-apply-run-completion/0.1"
TRANSACTION_BINDING_SCHEMA = "semantic-task-apply-transaction-binding/0.1"
RUNNER_VERSION = "0.1"
TRANSACTION_NAMESPACE = uuid.UUID("847e5933-83e1-515f-b756-3d4f944e56f3")
MAX_RESULT_BYTES = 16 * 1024 * 1024


class RunnerError(RuntimeError):
    pass


def canonical(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def sha256_bytes(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return "sha256:" + digest.hexdigest()


def _fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _safe_regular_bytes(path: Path, label: str, maximum: int | None = None) -> bytes:
    try:
        metadata = path.lstat()
    except FileNotFoundError as error:
        raise RunnerError(f"{label} is missing: {path}") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise RunnerError(f"{label} must be a regular non-symlink file: {path}")
    if maximum is not None and metadata.st_size > maximum:
        raise RunnerError(f"{label} exceeds {maximum} bytes: {path}")
    return path.read_bytes()


def _ensure_directory(path: Path, label: str) -> None:
    try:
        path.mkdir(mode=0o700, parents=True)
        return
    except FileExistsError:
        pass
    if path.exists() or path.is_symlink():
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
            raise RunnerError(f"{label} must be a real directory: {path}")
        return
    raise RunnerError(f"{label} could not be created safely: {path}")


def _write_new(path: Path, value: bytes, label: str) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = path.parent / (
        f".{path.name}.create-{os.getpid()}-{secrets.token_hex(8)}"
    )
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(temporary, flags, 0o600)
    try:
        with os.fdopen(descriptor, "wb", closefd=False) as handle:
            handle.write(value)
            handle.flush()
            os.fsync(handle.fileno())
    finally:
        os.close(descriptor)
    published = False
    try:
        try:
            # A hard-link publication is create-only and exposes either the
            # complete fsynced object or no object, never a partially written
            # request to a concurrent attach.
            os.link(temporary, path)
            published = True
        except FileExistsError:
            existing = _safe_regular_bytes(path, label)
            if existing != value:
                raise RunnerError(f"{label} already exists with different bytes: {path}")
    finally:
        temporary.unlink(missing_ok=True)
    if published:
        _fsync_directory(path.parent)


def _atomic_replace(path: Path, value: bytes, label: str) -> None:
    if path.exists() or path.is_symlink():
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
            raise RunnerError(f"{label} target is unsafe: {path}")
    temporary = path.parent / (
        f".{path.name}.tmp-{os.getpid()}-{secrets.token_hex(8)}"
    )
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(temporary, flags, 0o600)
    try:
        with os.fdopen(descriptor, "wb", closefd=False) as handle:
            handle.write(value)
            handle.flush()
            os.fsync(handle.fileno())
    finally:
        os.close(descriptor)
    os.replace(temporary, path)
    _fsync_directory(path.parent)


def _write_new_json(path: Path, value: Any, label: str) -> None:
    _write_new(path, canonical(value) + b"\n", label)


def _atomic_json(path: Path, value: Any, label: str) -> None:
    _atomic_replace(path, canonical(value) + b"\n", label)


def _read_json(path: Path, label: str, maximum: int | None = None) -> Any:
    raw = _safe_regular_bytes(path, label, maximum)
    try:
        return json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RunnerError(f"{label} is not valid JSON: {path}") from error


def _read_optional_json(path: Path, label: str, maximum: int | None = None) -> Any | None:
    if not path.exists() and not path.is_symlink():
        return None
    return _read_json(path, label, maximum)


def _open_lock(path: Path) -> BinaryIO:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    flags = os.O_RDWR | os.O_CREAT
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(path, flags, 0o600)
    metadata = os.fstat(descriptor)
    if not stat.S_ISREG(metadata.st_mode):
        os.close(descriptor)
        raise RunnerError(f"lock is not a regular file: {path}")
    return os.fdopen(descriptor, "a+b", buffering=0)


def _try_lock(handle: BinaryIO) -> bool:
    try:
        fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        return True
    except BlockingIOError:
        return False


def _unlock(handle: BinaryIO) -> None:
    fcntl.flock(handle.fileno(), fcntl.LOCK_UN)


def _git(repo: Path, *arguments: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), *arguments],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        text=True,
    )
    if result.returncode != 0:
        detail = result.stderr.strip().replace("\n", " ")[:512]
        raise RunnerError(f"git {' '.join(arguments)} failed: {detail}")
    return result.stdout.strip()


def _repository_identity(provided: Path) -> dict[str, str]:
    provided = provided.expanduser().resolve(strict=True)
    if not provided.is_dir():
        raise RunnerError(f"repository is not a directory: {provided}")
    top_level = Path(_git(provided, "rev-parse", "--show-toplevel")).resolve(strict=True)
    raw_git_dir = Path(_git(top_level, "rev-parse", "--git-dir"))
    git_dir = (
        raw_git_dir if raw_git_dir.is_absolute() else top_level / raw_git_dir
    ).resolve(strict=True)
    raw_common = Path(_git(top_level, "rev-parse", "--git-common-dir"))
    common_dir = (
        raw_common if raw_common.is_absolute() else top_level / raw_common
    ).resolve(strict=True)
    return {
        "topLevel": str(top_level),
        "gitDir": str(git_dir),
        "gitCommonDir": str(common_dir),
    }


def _regular_executable(provided: Path, label: str) -> Path:
    path = provided.expanduser().resolve(strict=True)
    _safe_regular_bytes(path, label)
    if not os.access(path, os.X_OK):
        raise RunnerError(f"{label} is not executable: {path}")
    return path


def _input_bytes(provided: Path, label: str) -> tuple[Path, bytes]:
    path = provided.expanduser().resolve(strict=True)
    raw = _safe_regular_bytes(path, label)
    try:
        parsed = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RunnerError(f"{label} is not valid JSON: {path}") from error
    if not isinstance(parsed, dict):
        raise RunnerError(f"{label} must contain one JSON object: {path}")
    return path, raw


def _normalized_target_ref(value: str) -> str:
    value = value.strip()
    if not value or any(character.isspace() for character in value):
        raise RunnerError("target ref must be a non-empty ref without whitespace")
    return value if value.startswith("refs/") else f"refs/heads/{value}"


def _absolute_optional_path(value: str | None) -> str | None:
    if value is None:
        return None
    return os.path.abspath(os.path.expanduser(value))


def _state_root(repo: Path) -> Path:
    semantic = repo / ".semantic-thread"
    _ensure_directory(semantic, "semantic state root")
    root = semantic / "task-runs"
    _ensure_directory(root, "task run state root")
    _ensure_directory(root / "transactions", "task transaction binding root")
    return root


def _request_paths(root: Path, digest_hex: str) -> tuple[Path, dict[str, Path]]:
    run_dir = root / digest_hex
    _ensure_directory(run_dir, "task run directory")
    paths = {
        "request": run_dir / "request.json",
        "context": run_dir / "context.json",
        "plan": run_dir / "edit-plan.json",
        "runLock": run_dir / "run.lock",
        "childLock": run_dir / "child.lock",
        "launch": run_dir / "launch.json",
        "child": run_dir / "child.json",
        "status": run_dir / "status.json",
        "completion": run_dir / "completion.json",
        "transaction": run_dir / "transaction.json",
        "stdout": run_dir / "stdout.log",
        "stderr": run_dir / "stderr.log",
        "supervisor": run_dir / "supervisor.log",
        "execError": run_dir / "exec-error.json",
    }
    for key in ("runLock", "childLock"):
        with _open_lock(paths[key]):
            pass
    return run_dir, paths


def _transaction_id(digest_hex: str) -> str:
    return "tx:" + str(uuid.uuid5(TRANSACTION_NAMESPACE, digest_hex))


def _prepare_request(arguments: argparse.Namespace) -> tuple[dict[str, Any], Path]:
    repository = _repository_identity(Path(arguments.repo))
    repo = Path(repository["topLevel"])
    clew = _regular_executable(Path(arguments.clew), "clew binary")
    runner = Path(__file__).resolve(strict=True)
    python = Path(sys.executable).resolve(strict=True)
    context_source, context = _input_bytes(Path(arguments.context), "task context")
    plan_source, plan = _input_bytes(Path(arguments.edit_plan), "edit plan")
    actor = arguments.actor.strip()
    if not actor:
        raise RunnerError("actor must not be empty")
    identity = {
        "schema": "semantic-task-apply-run-identity/0.1",
        "runnerVersion": RUNNER_VERSION,
        "repository": repository,
        "context": {"bytes": len(context), "sha256": sha256_bytes(context)},
        "editPlan": {"bytes": len(plan), "sha256": sha256_bytes(plan)},
        "targetRef": _normalized_target_ref(arguments.target_ref),
        "actor": actor,
        "options": {
            "allowLegacyHeuristic": bool(arguments.allow_legacy_heuristic),
            "compilerIndexRoot": _absolute_optional_path(arguments.compiler_index_root),
        },
        "clew": {"path": str(clew), "sha256": sha256_file(clew)},
        "runner": {"path": str(runner), "sha256": sha256_file(runner)},
        "python": {"path": str(python)},
    }
    digest = sha256_bytes(canonical(identity))
    digest_hex = digest.removeprefix("sha256:")
    run_id = "task-run:" + digest_hex
    transaction_id = _transaction_id(digest_hex)
    root = _state_root(repo)
    run_dir, paths = _request_paths(root, digest_hex)
    request = {
        "schema": REQUEST_SCHEMA,
        "requestDigest": digest,
        "runId": run_id,
        "transactionId": transaction_id,
        "identity": identity,
        "sources": {
            "context": str(context_source),
            "editPlan": str(plan_source),
        },
        "artifacts": {key: str(path) for key, path in paths.items()},
    }
    _write_new(paths["context"], context, "immutable task context")
    _write_new(paths["plan"], plan, "immutable edit plan")
    _write_new_json(paths["request"], request, "immutable task request")
    binding = {
        "schema": TRANSACTION_BINDING_SCHEMA,
        "transactionId": transaction_id,
        "requestDigest": digest,
        "runId": run_id,
    }
    binding_path = root / "transactions" / f"{transaction_id.removeprefix('tx:')}.json"
    _write_new_json(binding_path, binding, "immutable transaction binding")
    return request, run_dir


def _paths_from_request(request: dict[str, Any]) -> dict[str, Path]:
    artifacts = request.get("artifacts")
    if not isinstance(artifacts, dict):
        raise RunnerError("task request has no artifact map")
    return {key: Path(value) for key, value in artifacts.items() if isinstance(value, str)}


def _load_request(repo: Path, run_id: str) -> tuple[dict[str, Any], Path]:
    if not run_id.startswith("task-run:"):
        raise RunnerError("run id must be task-run:<64 lowercase hex>")
    digest_hex = run_id.removeprefix("task-run:")
    if len(digest_hex) != 64 or any(character not in "0123456789abcdef" for character in digest_hex):
        raise RunnerError("run id must be task-run:<64 lowercase hex>")
    repository = _repository_identity(repo)
    root = _state_root(Path(repository["topLevel"]))
    run_dir = root / digest_hex
    if not run_dir.is_dir() or run_dir.is_symlink():
        raise RunnerError(f"task run not found: {run_id}")
    request = _read_json(run_dir / "request.json", "task request", MAX_RESULT_BYTES)
    if not isinstance(request, dict) or request.get("runId") != run_id:
        raise RunnerError("task request/run id mismatch")
    _verify_request(request, run_dir)
    return request, run_dir


def _verify_request(request: dict[str, Any], run_dir: Path) -> None:
    if request.get("schema") != REQUEST_SCHEMA or not isinstance(request.get("identity"), dict):
        raise RunnerError("task request schema mismatch")
    digest = sha256_bytes(canonical(request["identity"]))
    digest_hex = digest.removeprefix("sha256:")
    if request.get("requestDigest") != digest:
        raise RunnerError("task request digest mismatch")
    if request.get("runId") != "task-run:" + digest_hex:
        raise RunnerError("task request run id mismatch")
    if request.get("transactionId") != _transaction_id(digest_hex):
        raise RunnerError("task request transaction id mismatch")
    expected_artifacts = _request_paths(_state_root(Path(request["identity"]["repository"]["topLevel"])), digest_hex)[1]
    if request.get("artifacts") != {key: str(path) for key, path in expected_artifacts.items()}:
        raise RunnerError("task request artifact paths mismatch")
    paths = _paths_from_request(request)
    for key, identity_key in (("context", "context"), ("plan", "editPlan")):
        raw = _safe_regular_bytes(paths[key], f"immutable {identity_key}")
        expected = request["identity"][identity_key]
        if expected != {"bytes": len(raw), "sha256": sha256_bytes(raw)}:
            raise RunnerError(f"immutable {identity_key} bytes do not match request")
    if sha256_file(Path(request["identity"]["clew"]["path"])) != request["identity"]["clew"]["sha256"]:
        raise RunnerError("clew binary changed after task request creation")


def _status_command(request: dict[str, Any]) -> list[str]:
    identity = request["identity"]
    return [
        identity["python"]["path"],
        identity["runner"]["path"],
        "status",
        "--repo",
        identity["repository"]["topLevel"],
        "--run-id",
        request["runId"],
    ]


def _inspect_command(request: dict[str, Any]) -> list[str]:
    identity = request["identity"]
    command = [identity["clew"]["path"]]
    compiler_index_root = identity["options"]["compilerIndexRoot"]
    if compiler_index_root is not None:
        command += ["--compiler-index-root", compiler_index_root]
    return command + [
        "tx",
        "inspect",
        "--repo",
        identity["repository"]["topLevel"],
        "--transaction-id",
        request["transactionId"],
    ]


def _task_apply_command(request: dict[str, Any]) -> list[str]:
    identity = request["identity"]
    paths = _paths_from_request(request)
    command = [identity["clew"]["path"]]
    compiler_index_root = identity["options"]["compilerIndexRoot"]
    if compiler_index_root is not None:
        command += ["--compiler-index-root", compiler_index_root]
    command += [
        "task-apply",
        "--repo",
        identity["repository"]["topLevel"],
        "--context",
        str(paths["context"]),
        "--edit-plan",
        str(paths["plan"]),
        "--target-ref",
        identity["targetRef"],
        "--actor",
        identity["actor"],
        "--transaction-id",
        request["transactionId"],
        "--output",
        str(paths["transaction"]),
    ]
    if identity["options"]["allowLegacyHeuristic"]:
        command.append("--allow-legacy-heuristic")
    return command


def _status_envelope(
    request: dict[str, Any], state: str, *, attached: bool, detail: dict[str, Any] | None = None
) -> dict[str, Any]:
    return {
        "schema": STATUS_SCHEMA,
        "runId": request["runId"],
        "requestDigest": request["requestDigest"],
        "transactionId": request["transactionId"],
        "state": state,
        "terminal": state in {
            "SUCCEEDED",
            "FAILED",
            "UNKNOWN_REQUIRES_INSPECTION",
        },
        "attached": attached,
        "statusCommand": _status_command(request),
        "transactionInspectCommand": _inspect_command(request),
        "detail": detail,
    }


def _public_status(request: dict[str, Any], run_dir: Path, *, attached: bool) -> dict[str, Any]:
    paths = _paths_from_request(request)
    completion = _read_optional_json(paths["completion"], "task completion", MAX_RESULT_BYTES)
    if isinstance(completion, dict):
        return _status_envelope(
            request,
            str(completion.get("state", "UNKNOWN_REQUIRES_INSPECTION")),
            attached=attached,
            detail={"completion": completion, "completionPath": str(paths["completion"])},
        )
    status = _read_optional_json(paths["status"], "task status", MAX_RESULT_BYTES)
    if isinstance(status, dict):
        return _status_envelope(
            request,
            str(status.get("state", "RUNNING")),
            attached=attached,
            detail={"status": status, "completionPath": str(paths["completion"])},
        )
    launch = _read_optional_json(paths["launch"], "task launch", MAX_RESULT_BYTES)
    return _status_envelope(
        request,
        "STARTING" if launch is not None else "ACCEPTED",
        attached=attached,
        detail={"completionPath": str(paths["completion"])},
    )


def _publish_status(request: dict[str, Any], state: str, **detail: Any) -> None:
    paths = _paths_from_request(request)
    value = {
        "schema": STATUS_SCHEMA,
        "runId": request["runId"],
        "requestDigest": request["requestDigest"],
        "transactionId": request["transactionId"],
        "state": state,
        "updatedUnixNs": time.time_ns(),
        **detail,
    }
    _atomic_json(paths["status"], value, "task status")


def _optional_artifact(path: Path, label: str) -> dict[str, Any] | None:
    if not path.exists() and not path.is_symlink():
        return None
    raw = _safe_regular_bytes(path, label)
    return {"path": str(path), "bytes": len(raw), "sha256": sha256_bytes(raw)}


def _parsed_result(path: Path) -> dict[str, Any] | None:
    if not path.exists() and not path.is_symlink():
        return None
    value = _read_json(path, "task stdout", MAX_RESULT_BYTES)
    return value if isinstance(value, dict) else None


def _completion(
    request: dict[str, Any],
    *,
    state: str,
    started_ns: int | None,
    finished_ns: int,
    exit_code: int | None,
    reason: str | None,
) -> dict[str, Any]:
    paths = _paths_from_request(request)
    transaction_value = _read_optional_json(
        paths["transaction"], "task transaction artifact", MAX_RESULT_BYTES
    )
    stdout_value = None
    try:
        stdout_value = _parsed_result(paths["stdout"])
    except RunnerError:
        pass
    return {
        "schema": COMPLETION_SCHEMA,
        "runId": request["runId"],
        "requestDigest": request["requestDigest"],
        "transactionId": request["transactionId"],
        "state": state,
        "reason": reason,
        "exitCode": exit_code,
        "startedUnixNs": started_ns,
        "finishedUnixNs": finished_ns,
        "wallMilliseconds": (
            None if started_ns is None else max(0, (finished_ns - started_ns) // 1_000_000)
        ),
        "taskApplyResult": stdout_value,
        "transactionStatus": (
            transaction_value.get("status") if isinstance(transaction_value, dict) else None
        ),
        "artifacts": {
            "stdout": _optional_artifact(paths["stdout"], "task stdout"),
            "stderr": _optional_artifact(paths["stderr"], "task stderr"),
            "transaction": _optional_artifact(
                paths["transaction"], "task transaction artifact"
            ),
        },
        "transactionInspectCommand": _inspect_command(request),
    }


def _publish_completion(request: dict[str, Any], completion: dict[str, Any]) -> None:
    paths = _paths_from_request(request)
    _write_new_json(paths["completion"], completion, "immutable task completion")
    _publish_status(
        request,
        str(completion["state"]),
        completionPath=str(paths["completion"]),
    )


def _completion_after_exit(
    request: dict[str, Any], started_ns: int, exit_code: int
) -> dict[str, Any]:
    paths = _paths_from_request(request)
    transaction = _read_optional_json(
        paths["transaction"], "task transaction artifact", MAX_RESULT_BYTES
    )
    if isinstance(transaction, dict) and transaction.get("status") == "COMMITTED":
        state = "SUCCEEDED"
        reason = None if exit_code == 0 else "COMMITTED_WITH_NONZERO_PROCESS_EXIT"
    else:
        parsed = None
        try:
            parsed = _parsed_result(paths["stdout"])
        except RunnerError:
            pass
        if exit_code >= 0 and isinstance(parsed, dict) and parsed.get("schema") == "semantic-error/0.1":
            state = "FAILED"
            reason = "TASK_APPLY_REPORTED_ERROR"
        elif exit_code == 0:
            state = "UNKNOWN_REQUIRES_INSPECTION"
            reason = "ZERO_EXIT_WITHOUT_COMMITTED_TRANSACTION_ARTIFACT"
        else:
            state = "UNKNOWN_REQUIRES_INSPECTION"
            reason = "TASK_APPLY_EXITED_WITHOUT_PROVABLE_TERMINAL_ARTIFACT"
    return _completion(
        request,
        state=state,
        started_ns=started_ns,
        finished_ns=time.time_ns(),
        exit_code=exit_code,
        reason=reason,
    )


def _lock_is_held(path: Path) -> bool:
    with _open_lock(path) as handle:
        if not _try_lock(handle):
            return True
        _unlock(handle)
        return False


def _status_after_owner_exit(request: dict[str, Any], run_dir: Path) -> dict[str, Any]:
    paths = _paths_from_request(request)
    completion = _read_optional_json(paths["completion"], "task completion", MAX_RESULT_BYTES)
    if isinstance(completion, dict):
        return _public_status(request, run_dir, attached=True)
    if _lock_is_held(paths["childLock"]):
        child = _read_optional_json(paths["child"], "task child", MAX_RESULT_BYTES)
        _publish_status(
            request,
            "RUNNING_UNSUPERVISED",
            child=child,
            note="child lifetime lock is still held; tx inspect is unsafe",
        )
        return _public_status(request, run_dir, attached=True)
    transaction = _read_optional_json(
        paths["transaction"], "task transaction artifact", MAX_RESULT_BYTES
    )
    if isinstance(transaction, dict) and transaction.get("status") == "COMMITTED":
        status = _read_optional_json(paths["status"], "task status", MAX_RESULT_BYTES)
        started_ns = status.get("startedUnixNs") if isinstance(status, dict) else None
        completion = _completion(
            request,
            state="SUCCEEDED",
            started_ns=started_ns if isinstance(started_ns, int) else None,
            finished_ns=time.time_ns(),
            exit_code=None,
            reason="COMPLETION_RECOVERED_FROM_COMMITTED_TRANSACTION_ARTIFACT",
        )
    else:
        parsed = None
        try:
            parsed = _parsed_result(paths["stdout"])
        except RunnerError:
            pass
        if isinstance(parsed, dict) and parsed.get("schema") == "semantic-error/0.1":
            state = "FAILED"
            reason = "COMPLETION_RECOVERED_FROM_TASK_ERROR_OUTPUT"
        else:
            state = "UNKNOWN_REQUIRES_INSPECTION"
            reason = "RUN_OWNER_EXITED_WITHOUT_PROVABLE_TERMINAL_ARTIFACT"
        status = _read_optional_json(paths["status"], "task status", MAX_RESULT_BYTES)
        started_ns = status.get("startedUnixNs") if isinstance(status, dict) else None
        completion = _completion(
            request,
            state=state,
            started_ns=started_ns if isinstance(started_ns, int) else None,
            finished_ns=time.time_ns(),
            exit_code=None,
            reason=reason,
        )
    _publish_completion(request, completion)
    return _public_status(request, run_dir, attached=True)


def start(arguments: argparse.Namespace) -> dict[str, Any]:
    request, run_dir = _prepare_request(arguments)
    paths = _paths_from_request(request)
    launch_token = secrets.token_hex(32)
    launch_time = time.time_ns()
    with _open_lock(paths["runLock"]) as lock:
        if not _try_lock(lock):
            return _public_status(request, run_dir, attached=True)
        lock_transferred = False
        try:
            if paths["completion"].exists():
                return _public_status(request, run_dir, attached=True)
            if paths["launch"].exists():
                return _status_after_owner_exit(request, run_dir)
            command = [
                request["identity"]["python"]["path"],
                request["identity"]["runner"]["path"],
                "_run",
                "--repo",
                request["identity"]["repository"]["topLevel"],
                "--run-id",
                request["runId"],
                "--launch-token",
                launch_token,
                "--launched-unix-ns",
                str(launch_time),
                "--run-lock-fd",
                str(lock.fileno()),
            ]
            with paths["supervisor"].open("ab", buffering=0) as supervisor_log:
                process = subprocess.Popen(
                    command,
                    cwd=request["identity"]["repository"]["topLevel"],
                    stdin=subprocess.DEVNULL,
                    stdout=supervisor_log,
                    stderr=supervisor_log,
                    start_new_session=True,
                    close_fds=True,
                    pass_fds=(lock.fileno(),),
                )
            # The supervisor inherited the same flock-bearing open file
            # description. Closing our copy (without LOCK_UN) hands ownership
            # over without a gap in which status could abort an unstarted run.
            lock_transferred = True
            launch = {
                "schema": LAUNCH_SCHEMA,
                "runId": request["runId"],
                "requestDigest": request["requestDigest"],
                "transactionId": request["transactionId"],
                "launchToken": launch_token,
                "launchedUnixNs": launch_time,
                "supervisorPid": process.pid,
            }
            _write_new_json(paths["launch"], launch, "immutable task launch")
            _publish_status(
                request,
                "STARTING",
                supervisorPid=process.pid,
                launchedUnixNs=launch_time,
            )
        finally:
            if not lock_transferred:
                _unlock(lock)
    deadline = time.monotonic() + max(0.0, arguments.handshake_seconds)
    while time.monotonic() < deadline:
        status = _read_optional_json(paths["status"], "task status", MAX_RESULT_BYTES)
        completion = _read_optional_json(paths["completion"], "task completion", MAX_RESULT_BYTES)
        if completion is not None or (
            isinstance(status, dict) and status.get("state") not in {"STARTING", "ACCEPTED"}
        ):
            break
        time.sleep(0.05)
    return _public_status(request, run_dir, attached=False)


def status(arguments: argparse.Namespace) -> dict[str, Any]:
    request, run_dir = _load_request(Path(arguments.repo), arguments.run_id)
    paths = _paths_from_request(request)
    with _open_lock(paths["runLock"]) as lock:
        if not _try_lock(lock):
            return _public_status(request, run_dir, attached=True)
        try:
            return _status_after_owner_exit(request, run_dir)
        finally:
            _unlock(lock)


def _run(arguments: argparse.Namespace) -> None:
    with os.fdopen(arguments.run_lock_fd, "a+b", buffering=0) as lock:
        if not stat.S_ISREG(os.fstat(lock.fileno()).st_mode):
            raise RunnerError("inherited task run lock is not a regular file")
        request, _ = _load_request(Path(arguments.repo), arguments.run_id)
        paths = _paths_from_request(request)
        launch = {
            "schema": LAUNCH_SCHEMA,
            "runId": request["runId"],
            "requestDigest": request["requestDigest"],
            "transactionId": request["transactionId"],
            "launchToken": arguments.launch_token,
            "launchedUnixNs": arguments.launched_unix_ns,
            "supervisorPid": os.getpid(),
        }
        _write_new_json(paths["launch"], launch, "immutable task launch")
        if paths["completion"].exists():
            return
        started_ns = time.time_ns()
        _publish_status(
            request,
            "RUNNING",
            supervisorPid=os.getpid(),
            startedUnixNs=started_ns,
        )
        child_token = secrets.token_hex(32)
        command = [
            request["identity"]["python"]["path"],
            request["identity"]["runner"]["path"],
            "_exec",
            "--repo",
            request["identity"]["repository"]["topLevel"],
            "--run-id",
            request["runId"],
            "--launch-token",
            arguments.launch_token,
            "--child-token",
            child_token,
        ]
        try:
            with paths["stdout"].open("wb", buffering=0) as stdout_handle, paths[
                "stderr"
            ].open("wb", buffering=0) as stderr_handle:
                child = subprocess.Popen(
                    command,
                    cwd=request["identity"]["repository"]["topLevel"],
                    stdin=subprocess.DEVNULL,
                    stdout=stdout_handle,
                    stderr=stderr_handle,
                    start_new_session=True,
                    close_fds=True,
                )
                _publish_status(
                    request,
                    "RUNNING",
                    supervisorPid=os.getpid(),
                    childPid=child.pid,
                    childToken=child_token,
                    startedUnixNs=started_ns,
                )
                exit_code = child.wait()
            while _lock_is_held(paths["childLock"]):
                _publish_status(
                    request,
                    "DRAINING",
                    supervisorPid=os.getpid(),
                    childPid=child.pid,
                    childToken=child_token,
                    startedUnixNs=started_ns,
                    note="task process exited; inherited child lifetime lock is still held",
                )
                time.sleep(0.05)
            completion = _completion_after_exit(request, started_ns, exit_code)
            _publish_completion(request, completion)
        except BaseException as error:
            child_record = _read_optional_json(paths["child"], "task child", MAX_RESULT_BYTES)
            if _lock_is_held(paths["childLock"]):
                _publish_status(
                    request,
                    "RUNNING_UNSUPERVISED",
                    child=child_record,
                    startedUnixNs=started_ns,
                    supervisorError=str(error)[:1000],
                )
                return
            completion = _completion(
                request,
                state="UNKNOWN_REQUIRES_INSPECTION" if child_record else "FAILED",
                started_ns=started_ns,
                finished_ns=time.time_ns(),
                exit_code=None,
                reason=(
                    "SUPERVISOR_FAILED_AFTER_CHILD_START"
                    if child_record
                    else "SUPERVISOR_FAILED_BEFORE_CHILD_START"
                ),
            )
            completion["supervisorError"] = str(error)[:1000]
            _publish_completion(request, completion)


def _exec(arguments: argparse.Namespace) -> None:
    request, _ = _load_request(Path(arguments.repo), arguments.run_id)
    paths = _paths_from_request(request)
    launch = _read_json(paths["launch"], "task launch", MAX_RESULT_BYTES)
    if launch.get("launchToken") != arguments.launch_token:
        raise RunnerError("child launch token mismatch")
    command = _task_apply_command(request)
    child_lock = _open_lock(paths["childLock"])
    fcntl.flock(child_lock.fileno(), fcntl.LOCK_EX)
    os.set_inheritable(child_lock.fileno(), True)
    child = {
        "schema": CHILD_SCHEMA,
        "runId": request["runId"],
        "requestDigest": request["requestDigest"],
        "transactionId": request["transactionId"],
        "launchToken": arguments.launch_token,
        "childToken": arguments.child_token,
        "pid": os.getpid(),
        "startedUnixNs": time.time_ns(),
        "commandSha256": sha256_bytes(canonical(command)),
    }
    _write_new_json(paths["child"], child, "immutable task child")
    environment = os.environ.copy()
    environment["CODECLEW_TASK_RUN_ID"] = request["runId"]
    environment["CODECLEW_TASK_RUN_CHILD_TOKEN"] = arguments.child_token
    try:
        os.execve(command[0], command, environment)
    except OSError as error:
        _write_new_json(
            paths["execError"],
            {
                "schema": "semantic-task-apply-run-exec-error/0.1",
                "runId": request["runId"],
                "transactionId": request["transactionId"],
                "error": str(error)[:1000],
            },
            "immutable task exec error",
        )
        raise


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    start_parser = commands.add_parser("start", help="start or attach to one durable task apply")
    start_parser.add_argument("--clew", required=True)
    start_parser.add_argument("--repo", required=True)
    start_parser.add_argument("--context", required=True)
    start_parser.add_argument("--edit-plan", required=True)
    start_parser.add_argument("--target-ref", required=True)
    start_parser.add_argument("--actor", default="semantic-task-agent")
    start_parser.add_argument("--compiler-index-root")
    start_parser.add_argument("--allow-legacy-heuristic", action="store_true")
    start_parser.add_argument("--handshake-seconds", type=float, default=2.0)
    status_parser = commands.add_parser("status", help="read one run without retrying it")
    status_parser.add_argument("--repo", required=True)
    status_parser.add_argument("--run-id", required=True)
    for name in ("_run", "_exec"):
        internal = commands.add_parser(name, help=argparse.SUPPRESS)
        internal.add_argument("--repo", required=True)
        internal.add_argument("--run-id", required=True)
        internal.add_argument("--launch-token", required=True)
        if name == "_run":
            internal.add_argument("--launched-unix-ns", required=True, type=int)
            internal.add_argument("--run-lock-fd", required=True, type=int)
        else:
            internal.add_argument("--child-token", required=True)
    return parser


def main() -> int:
    arguments = _parser().parse_args()
    try:
        if arguments.command == "start":
            result = start(arguments)
        elif arguments.command == "status":
            result = status(arguments)
        elif arguments.command == "_run":
            _run(arguments)
            return 0
        else:
            _exec(arguments)
            return 0
        sys.stdout.buffer.write(canonical(result) + b"\n")
        return 0
    except RunnerError as error:
        sys.stdout.buffer.write(
            canonical(
                {
                    "schema": "semantic-task-apply-runner-error/0.1",
                    "error": str(error),
                }
            )
            + b"\n"
        )
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
