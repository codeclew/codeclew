#!/usr/bin/env python3
"""Run the bounded Kotlin 2.4/Gradle Codeclew pilot contour."""

from __future__ import annotations

from dataclasses import dataclass
import argparse
import io
import json
import os
from pathlib import Path
import re
import signal
import shutil
import subprocess
import tarfile
import tempfile
import time
from typing import Callable


ROOT = Path(__file__).resolve().parent.parent
TERMINAL = {
    "READY_TO_PUBLISH",
    "READY_TO_PUBLISH_CONDITIONAL",
    "VALIDATED_CONDITIONAL",
    "FAILED",
    "WORKTREE_RECOVERY_REQUIRED",
    "CANCELLED",
}
ERROR_CODE = re.compile(r"^[A-Z][A-Z0-9_]{0,63}$")


class PilotFailure(Exception):
    def __init__(
        self, code: str, *, session_id: str | None = None, run_id: str | None = None
    ) -> None:
        super().__init__(code)
        self.code = code
        self.session_id = session_id
        self.run_id = run_id


class PilotWorkspace:
    def __init__(self) -> None:
        self.path: Path | None = None
        self.preserve = False

    def __enter__(self) -> "PilotWorkspace":
        self.path = Path(tempfile.mkdtemp(prefix="codeclew-pilot-")).resolve()
        return self

    def __exit__(self, _type: object, _value: object, _traceback: object) -> None:
        if not self.preserve and self.path is not None:
            # Runtime capsules are deliberately sealed read-only.  The pilot owns
            # this entire 0700 workspace, so restore directory write permission
            # before removing it without weakening capsules in persistent state.
            for directory, _children, _files in os.walk(
                self.path, topdown=True, followlinks=False
            ):
                os.chmod(directory, 0o700, follow_symlinks=False)
            shutil.rmtree(self.path)


def interrupt_as_failure(_signum: int, _frame: object) -> None:
    raise PilotFailure("PILOT_SIGNALLED")


@dataclass(frozen=True)
class PilotCase:
    case_id: str
    intent: str
    term: str
    old_text: str
    new_text: str
    test_file: str
    test_text: str


CASES = (
    PilotCase(
        "total-boundary",
        "define the zero-base total result and add its exact test",
        "com.acme.total",
        "    return value\n}",
        "    return if (base == 0) 1 else value\n}",
        "src/test/kotlin/com/acme/CodeclewTotalPilotTest.kt",
        """package com.acme

import kotlin.test.Test
import kotlin.test.assertEquals

class CodeclewTotalPilotTest {
    @Test fun zeroBaseHasExplicitResult() { assertEquals(1, total(0, false)) }
}
""",
    ),
    PilotCase(
        "classify-edge",
        "define the minimum integer classification and add its exact test",
        "com.acme.classify",
        'fun classify(value: Int): String = when {\n    value < 0 -> "negative"',
        'fun classify(value: Int): String = when {\n    value == Int.MIN_VALUE -> "minimum"\n    value < 0 -> "negative"',
        "src/test/kotlin/com/acme/CodeclewClassifyPilotTest.kt",
        """package com.acme

import kotlin.test.Test
import kotlin.test.assertEquals

class CodeclewClassifyPilotTest {
    @Test fun minimumHasExplicitClass() { assertEquals("minimum", classify(Int.MIN_VALUE)) }
}
""",
    ),
    PilotCase(
        "counter-step",
        "change Counter increment to a two-unit step and add its exact test",
        "com.acme.Counter.increment",
        "    fun increment(): Int {\n        value += 1\n        return value\n    }",
        "    fun increment(): Int {\n        value += 2\n        return value\n    }",
        "src/test/kotlin/com/acme/CodeclewCounterPilotTest.kt",
        """package com.acme

import kotlin.test.Test
import kotlin.test.assertEquals

class CodeclewCounterPilotTest {
    @Test fun incrementUsesTwoUnitStep() { assertEquals(5, Counter(3).increment()) }
}
""",
    ),
)


def millis(started: float) -> int:
    return max(0, round((time.monotonic() - started) * 1000))


def command(
    arguments: list[str],
    *,
    cwd: Path,
    environment: dict[str, str],
    error_code: str,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        arguments,
        cwd=cwd,
        env=environment,
        check=False,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if check and completed.returncode != 0:
        raise PilotFailure(error_code)
    return completed


def clew(
    arguments: list[str],
    *,
    environment: dict[str, str],
    error_code: str,
    check: bool = True,
) -> tuple[subprocess.CompletedProcess[str], dict[str, object]]:
    completed = command(
        [str(ROOT / "clew"), *arguments],
        cwd=ROOT,
        environment=environment,
        error_code=error_code,
        check=check,
    )
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise PilotFailure("INVALID_CLEW_OUTPUT") from error
    if not isinstance(value, dict):
        raise PilotFailure("INVALID_CLEW_OUTPUT")
    return completed, value


def git(repository: Path, *arguments: str) -> str:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=repository,
        check=False,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if completed.returncode != 0:
        raise PilotFailure("GIT_FAILED")
    return completed.stdout.strip()


def extract_fixture(repository: Path) -> None:
    archive = subprocess.run(
        ["git", "archive", "HEAD:fixtures/kotlin-basic"],
        cwd=ROOT,
        check=True,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
    ).stdout
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as stream:
        for member in stream.getmembers():
            path = Path(member.name)
            if member.issym() or member.islnk() or path.is_absolute() or ".." in path.parts:
                raise PilotFailure("UNSAFE_FIXTURE")
        stream.extractall(repository)
    os.chmod(repository / "gradlew", 0o755)


def source_authorities(context: dict[str, object]) -> dict[str, dict[str, object]]:
    projection = context.get("context")
    if not isinstance(projection, dict) or not isinstance(projection.get("sources"), list):
        raise PilotFailure("INVALID_CONTEXT")
    return {
        str(row["fileId"]): row["contentRef"]
        for row in projection["sources"]
        if isinstance(row, dict) and "fileId" in row and isinstance(row.get("contentRef"), dict)
    }


def require(condition: bool, code: str) -> None:
    if not condition:
        raise PilotFailure(code)


def _run_case(
    case: PilotCase,
    repository: Path,
    plan_path: Path,
    environment: dict[str, str],
    authority: dict[str, object],
) -> tuple[dict[str, object], str]:
    total_started = time.monotonic()
    baseline = git(repository, "rev-parse", "HEAD")
    require(git(repository, "rev-parse", "main") == baseline, "INVALID_BASELINE_REF")
    baseline_started = time.monotonic()
    command(
        [str(repository / "gradlew"), "test", "--no-daemon", "--quiet"],
        cwd=repository,
        environment=environment,
        error_code="NATIVE_BASELINE_FAILED",
    )
    native_baseline_ms = millis(baseline_started)

    open_started = time.monotonic()
    _, opened = clew(
        [
            "change",
            "open",
            "--repo",
            str(repository),
            "--target-ref",
            "main",
            "--language",
            "kotlin",
            "--compilation",
            ":/main",
            "--intent",
            case.intent,
            "--term",
            case.term,
            "--max-roots",
            "4",
        ],
        environment=environment,
        error_code="CHANGE_OPEN_FAILED",
    )
    open_ms = millis(open_started)
    session_row = opened.get("session")
    context = opened.get("context")
    require(isinstance(session_row, dict) and isinstance(context, dict), "INVALID_CHANGE_OPEN")
    session = str(session_row["sessionId"])
    authority["session"] = session
    runtime_mode = str(session_row["runtimeMode"])
    context_id = str(context["contextId"])
    sources = source_authorities(context)
    main_file = "src/main/kotlin/com/acme/Samples.kt"
    require(main_file in sources, "MISSING_SOURCE_AUTHORITY")

    plan = {
        "schema": "codeclew-task-plan/2.0",
        "operations": [
            {
                "kind": "REPLACE_TEXT",
                "opId": f"{case.case_id}-replace",
                "target": {"fileId": main_file, "contentRef": sources[main_file]},
                "oldText": case.old_text,
                "newText": case.new_text,
            },
            {
                "kind": "CREATE_FILE",
                "opId": f"{case.case_id}-test",
                "target": {"fileId": case.test_file},
                "text": case.test_text,
            },
        ],
        "validation": [{"launcher": "GRADLE", "args": ["test", "--no-daemon", "--quiet"]}],
    }
    plan_path.write_text(
        json.dumps(plan, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    prepare_started = time.monotonic()
    prepare_arguments = [
        "change",
        "prepare",
        "--session",
        session,
        "--context",
        context_id,
        "--plan",
        str(plan_path),
    ]
    _, prepared = clew(
        prepare_arguments,
        environment=environment,
        error_code="CHANGE_PREPARE_FAILED",
    )
    run_row = prepared.get("run")
    require(isinstance(run_row, dict), "INVALID_CHANGE_PREPARE")
    run = str(run_row["runId"])
    authority["run"] = run
    _, repeated = clew(
        prepare_arguments,
        environment=environment,
        error_code="CHANGE_PREPARE_RETRY_FAILED",
    )
    require(isinstance(repeated.get("run"), dict), "INVALID_CHANGE_PREPARE")
    require(repeated["run"]["runId"] == run, "NON_IDEMPOTENT_PREPARE")

    deadline = time.monotonic() + 180
    while True:
        _, status = clew(
            ["change", "status", "--run", run],
            environment=environment,
            error_code="CHANGE_STATUS_FAILED",
        )
        status_run = status.get("run")
        require(isinstance(status_run, dict), "INVALID_CHANGE_STATUS")
        run_status = str(status_run["status"])
        if run_status in TERMINAL:
            break
        if time.monotonic() >= deadline:
            raise PilotFailure("PREPARE_TIMEOUT")
        time.sleep(0.2)
    prepare_to_ready_ms = millis(prepare_started)
    require(run_status == "READY_TO_PUBLISH_CONDITIONAL", "NOT_READY_CONDITIONAL")
    require(git(repository, "status", "--porcelain") == "", "SOURCE_MUTATED_BEFORE_PUBLISH")
    require(git(repository, "rev-parse", "HEAD") == baseline, "SOURCE_REF_MOVED")
    require(git(repository, "rev-parse", "main") == baseline, "TARGET_REF_MOVED")
    candidate = status.get("candidate")
    require(isinstance(candidate, dict), "MISSING_CANDIDATE")
    obligations = candidate.get("qualifiedObligations")
    require(isinstance(obligations, list) and bool(obligations), "MISSING_OBLIGATIONS")
    approval_ids = sorted(str(row["approvalId"]) for row in obligations if isinstance(row, dict))
    require(len(approval_ids) == len(obligations), "INVALID_OBLIGATIONS")

    refused, refusal = clew(
        ["change", "publish", "--session", session, "--run", run],
        environment=environment,
        error_code="STRICT_PUBLISH_EXECUTION_FAILED",
        check=False,
    )
    require(refused.returncode != 0, "STRICT_PUBLISH_NOT_REFUSED")
    error = refusal.get("error")
    require(
        isinstance(error, dict) and error.get("code") == "INCOMPLETE_SEMANTIC_ANALYSIS",
        "INVALID_STRICT_REFUSAL",
    )
    publish_arguments = [
        "change",
        "publish",
        "--session",
        session,
        "--run",
        run,
        "--allow-conditional",
        "--prepared-authority-digest",
        str(candidate["preparedAuthorityDigest"]),
    ]
    for approval_id in approval_ids:
        publish_arguments.extend(["--acknowledge-obligation", approval_id])
    publish_started = time.monotonic()
    _, published = clew(
        publish_arguments,
        environment=environment,
        error_code="CONDITIONAL_PUBLISH_FAILED",
    )
    _, repeated_publish = clew(
        publish_arguments,
        environment=environment,
        error_code="PUBLISH_RETRY_FAILED",
    )
    publish_ms = millis(publish_started)
    published_run = published.get("run")
    repeated_run = repeated_publish.get("run")
    require(
        isinstance(published_run, dict)
        and isinstance(repeated_run, dict)
        and published_run.get("status") == "PUBLISHED_CONDITIONAL"
        and repeated_run.get("finalCommit") == published_run.get("finalCommit"),
        "INVALID_PUBLISH_RESULT",
    )
    require(
        git(repository, "rev-list", "--count", f"{baseline}..HEAD") == "1",
        "UNEXPECTED_COMMIT_COUNT",
    )
    changed = git(
        repository, "diff", "--name-status", "--no-renames", baseline, "HEAD"
    ).splitlines()
    require(
        changed == [f"M\t{main_file}", f"A\t{case.test_file}"],
        "UNEXPECTED_WRITE_SET",
    )
    command(
        [str(repository / "gradlew"), "test", "--no-daemon", "--quiet"],
        cwd=repository,
        environment=environment,
        error_code="NATIVE_POST_TEST_FAILED",
    )
    clew(
        ["session", "close", "--session", session],
        environment=environment,
        error_code="SESSION_CLOSE_FAILED",
    )
    clew(
        ["session", "gc", "--session", session],
        environment=environment,
        error_code="SESSION_GC_FAILED",
    )
    authority["cleaned"] = True
    return (
        {
            "caseId": case.case_id,
            "durationsMs": {
                "nativeBaseline": native_baseline_ms,
                "open": open_ms,
                "prepareToReady": prepare_to_ready_ms,
                "publish": publish_ms,
                "total": millis(total_started),
            },
            "errorCode": None,
            "status": "PASSED",
        },
        runtime_mode,
    )


def _cleanup_case(
    session: str,
    run: str | None,
    environment: dict[str, str],
    invoke: Callable[..., tuple[subprocess.CompletedProcess[str], dict[str, object]]] = clew,
) -> None:
    status_name = None
    if run is not None:
        _, status = invoke(
            ["change", "status", "--run", run],
            environment=environment,
            error_code="CLEANUP_STATUS_FAILED",
        )
        row = status.get("run")
        if not isinstance(row, dict):
            raise PilotFailure(
                "PILOT_RECOVERY_REQUIRED", session_id=session, run_id=run
            )
        status_name = str(row.get("status"))
        if status_name in {"CREATED", "PREPARING"}:
            try:
                _, cancelled = invoke(
                    ["task-run", "cancel", "--run", run],
                    environment=environment,
                    error_code="CLEANUP_CANCEL_FAILED",
                )
                cancelled_row = cancelled.get("run")
                status_name = (
                    str(cancelled_row.get("status"))
                    if isinstance(cancelled_row, dict)
                    else None
                )
            except PilotFailure:
                _, latest = invoke(
                    ["change", "status", "--run", run],
                    environment=environment,
                    error_code="CLEANUP_STATUS_FAILED",
                )
                latest_row = latest.get("run")
                status_name = (
                    str(latest_row.get("status"))
                    if isinstance(latest_row, dict)
                    else None
                )
        safe_terminal = {
            "CANCELLED",
            "FAILED",
            "VALIDATED_CONDITIONAL",
            "PUBLISHED",
            "PUBLISHED_CONDITIONAL",
        }
        if status_name not in safe_terminal:
            raise PilotFailure(
                "PILOT_RECOVERY_REQUIRED", session_id=session, run_id=run
            )
    terminal = "close" if status_name in {"PUBLISHED", "PUBLISHED_CONDITIONAL"} else "abort"
    try:
        invoke(
            ["session", terminal, "--session", session],
            environment=environment,
            error_code="SESSION_TERMINAL_CLEANUP_FAILED",
        )
        invoke(
            ["session", "gc", "--session", session],
            environment=environment,
            error_code="SESSION_GC_FAILED",
        )
    except PilotFailure as error:
        raise PilotFailure(
            "PILOT_RECOVERY_REQUIRED", session_id=session, run_id=run
        ) from error


def cleanup_case(
    session: str,
    run: str | None,
    environment: dict[str, str],
    invoke: Callable[..., tuple[subprocess.CompletedProcess[str], dict[str, object]]] = clew,
) -> None:
    try:
        _cleanup_case(session, run, environment, invoke)
    except PilotFailure as error:
        if error.code == "PILOT_RECOVERY_REQUIRED":
            raise
        raise PilotFailure(
            "PILOT_RECOVERY_REQUIRED", session_id=session, run_id=run
        ) from error


def run_case(
    case: PilotCase,
    repository: Path,
    plan_path: Path,
    environment: dict[str, str],
) -> tuple[dict[str, object], str]:
    authority: dict[str, object] = {"session": None, "run": None, "cleaned": False}
    try:
        return _run_case(case, repository, plan_path, environment, authority)
    finally:
        session = authority["session"]
        if isinstance(session, str) and not authority["cleaned"]:
            run = authority["run"]
            cleanup_case(
                session,
                run if isinstance(run, str) else None,
                environment,
            )


def execute_cases(
    cases: tuple[PilotCase, ...],
    execute: Callable[[PilotCase], tuple[dict[str, object], str]],
) -> tuple[list[dict[str, object]], str | None]:
    results = []
    runtime_mode = None
    for case in cases:
        try:
            result, observed_mode = execute(case)
        except PilotFailure as error:
            failure: dict[str, object] = {
                "caseId": case.case_id,
                "durationsMs": {},
                "errorCode": error.code,
                "status": "FAILED",
            }
            if error.session_id is not None:
                failure["recoveryAuthority"] = {
                    "runId": error.run_id,
                    "sessionId": error.session_id,
                }
            results.append(failure)
            break
        if runtime_mode is None:
            runtime_mode = observed_mode
        elif runtime_mode != observed_mode:
            results.append(
                {
                    "caseId": case.case_id,
                    "durationsMs": {},
                    "errorCode": "RUNTIME_MODE_CHANGED",
                    "status": "FAILED",
                }
            )
            break
        results.append(result)
    return results, runtime_mode


def public_summary(
    results: list[dict[str, object]], runtime_mode: str | None, prime_ms: int
) -> dict[str, object]:
    passed = sum(row.get("status") == "PASSED" for row in results)
    value = {
        "aggregate": {"attempted": len(results), "passed": passed, "total": len(CASES)},
        "cases": results,
        "primeMs": prime_ms,
        "runtimeMode": runtime_mode,
        "schema": "codeclew-pilot/1.0",
        "status": "PASSED" if passed == len(CASES) else "FAILED",
    }
    validate_public_value(value)
    return value


def validate_public_value(value: object) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if not isinstance(key, str) or len(key) > 64:
                raise ValueError("invalid public key")
            validate_public_value(child)
    elif isinstance(value, list):
        if len(value) > len(CASES):
            raise ValueError("public list exceeds pilot bound")
        for child in value:
            validate_public_value(child)
    elif isinstance(value, str):
        if len(value) > 128 or "\n" in value or Path(value).is_absolute():
            raise ValueError("private or unbounded public string")
        if value.endswith("_FAILED") or value in {"RUNTIME_MODE_CHANGED"}:
            if not ERROR_CODE.fullmatch(value):
                raise ValueError("invalid error code")


def main() -> int:
    signal.signal(signal.SIGINT, interrupt_as_failure)
    signal.signal(signal.SIGTERM, interrupt_as_failure)
    parser = argparse.ArgumentParser()
    parser.add_argument("--reuse-primed-runtime", action="store_true")
    arguments = parser.parse_args()
    with PilotWorkspace() as workspace:
        assert workspace.path is not None
        root = workspace.path
        configured_state = os.environ.get("CODECLEW_HOME")
        state = Path(configured_state).resolve() if configured_state else root / "state"
        state.mkdir(mode=0o700, parents=True, exist_ok=True)
        environment = {**os.environ, "CODECLEW_HOME": str(state)}
        prime_ms = 0
        if not arguments.reuse_primed_runtime:
            prime_started = time.monotonic()
            command(
                [str(ROOT / "clew"), "--version"],
                cwd=ROOT,
                environment=environment,
                error_code="RUNTIME_PRIME_FAILED",
            )
            prime_ms = millis(prime_started)

        def execute(case: PilotCase) -> tuple[dict[str, object], str]:
            repository = root / case.case_id
            repository.mkdir(mode=0o700)
            extract_fixture(repository)
            git(repository, "init", "-q", "-b", "main")
            git(repository, "config", "user.name", "Codeclew Pilot")
            git(repository, "config", "user.email", "pilot@codeclew.invalid")
            git(repository, "add", ".")
            git(repository, "commit", "-q", "-m", "baseline")
            return run_case(case, repository, root / f"{case.case_id}.json", environment)

        results, runtime_mode = execute_cases(CASES, execute)
        recovery_required = any(
            row.get("errorCode") == "PILOT_RECOVERY_REQUIRED" for row in results
        )
        workspace.preserve = recovery_required
        summary = public_summary(results, runtime_mode, prime_ms)
        exit_code = (
            0 if summary["status"] == "PASSED" else (2 if recovery_required else 1)
        )
    # Emit exactly one result only after disposable-workspace cleanup succeeds.
    print(json.dumps(summary, sort_keys=True, separators=(",", ":")))
    return exit_code


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except PilotFailure as error:
        row: dict[str, object] = {
            "caseId": "bootstrap",
            "durationsMs": {},
            "errorCode": error.code,
            "status": "FAILED",
        }
        if error.session_id is not None:
            row["recoveryAuthority"] = {
                "runId": error.run_id,
                "sessionId": error.session_id,
            }
        failure = [row]
        print(json.dumps(public_summary(failure, None, 0), sort_keys=True, separators=(",", ":")))
        raise SystemExit(2 if error.code == "PILOT_RECOVERY_REQUIRED" else 1) from None
    except Exception:
        failure = [{
            "caseId": "internal",
            "durationsMs": {},
            "errorCode": "PILOT_INTERNAL_FAILED",
            "status": "FAILED",
        }]
        print(json.dumps(public_summary(failure, None, 0), sort_keys=True, separators=(",", ":")))
        raise SystemExit(1) from None
