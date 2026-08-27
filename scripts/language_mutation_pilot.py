#!/usr/bin/env python3
"""Qualify the bounded conditional Rust and Python mutation contours."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import sys
import tarfile
import time


ROOT = Path(__file__).resolve().parent.parent
PILOT_PATH = Path(__file__).resolve().with_name("pilot.py")
SPEC = importlib.util.spec_from_file_location("codeclew_shared_pilot", PILOT_PATH)
assert SPEC is not None and SPEC.loader is not None
pilot = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = pilot
SPEC.loader.exec_module(pilot)


@dataclass(frozen=True)
class LanguageCase:
    case_id: str
    language: str
    fixture: str
    compilations: tuple[str, ...]
    intent: str
    term: str
    source_file: str
    old_text: str
    new_text: str
    test_file: str | None
    test_text: str | None
    validation: tuple[str, ...]
    expected_changes: tuple[str, ...]


RUST_SOURCE = """pub struct Counter {
    value: i32,
}

impl Counter {
    pub fn new(value: i32) -> Self {
        Self { value }
    }

    pub fn increment(&mut self) -> i32 {
        self.value += 1;
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::Counter;

    #[test]
    fn increment_uses_one_unit_step() {
        assert_eq!(Counter::new(3).increment(), 4);
    }
}
"""


def rust_source(method: str, test: str) -> str:
    with_method = RUST_SOURCE.replace(
        "}\n\n#[cfg(test)]",
        f"{method}}}\n\n#[cfg(test)]",
        1,
    )
    assert with_method.endswith("}\n")
    return with_method[:-2] + test + "}\n"


PYTHON_SOURCE = """class Counter:
    def __init__(self, value: int) -> None:
        self.value = value

    def increment(self) -> int:
        self.value += 1
        return self.value
"""


RUST_CASES = (
    LanguageCase(
        case_id="rust-decrement",
        language="rust",
        fixture="rust-basic",
        compilations=(
            "cargo:Cargo.toml#codeclew-rust-basic#lib#codeclew_rust_basic",
        ),
        intent="add a one-unit decrement operation with a focused native test",
        term="Counter",
        source_file="src/lib.rs",
        old_text=RUST_SOURCE,
        new_text=rust_source(
            """
    pub fn decrement(&mut self) -> i32 {
        self.value -= 1;
        self.value
    }
""",
            """
    #[test]
    fn decrement_uses_one_unit_step() {
        assert_eq!(Counter::new(3).decrement(), 2);
    }
""",
        ),
        test_file=None,
        test_text=None,
        validation=("CARGO", "test", "--quiet"),
        expected_changes=("M\tsrc/lib.rs",),
    ),
    LanguageCase(
        case_id="rust-increment-by",
        language="rust",
        fixture="rust-basic",
        compilations=(
            "cargo:Cargo.toml#codeclew-rust-basic#lib#codeclew_rust_basic",
        ),
        intent="add an explicit increment-by operation with a focused native test",
        term="Counter",
        source_file="src/lib.rs",
        old_text=RUST_SOURCE,
        new_text=rust_source(
            """
    pub fn increment_by(&mut self, amount: i32) -> i32 {
        self.value += amount;
        self.value
    }
""",
            """
    #[test]
    fn increment_by_uses_the_requested_amount() {
        assert_eq!(Counter::new(3).increment_by(4), 7);
    }
""",
        ),
        test_file=None,
        test_text=None,
        validation=("CARGO", "test", "--quiet"),
        expected_changes=("M\tsrc/lib.rs",),
    ),
    LanguageCase(
        case_id="rust-reset",
        language="rust",
        fixture="rust-basic",
        compilations=(
            "cargo:Cargo.toml#codeclew-rust-basic#lib#codeclew_rust_basic",
        ),
        intent="add an explicit reset operation with a focused native test",
        term="Counter",
        source_file="src/lib.rs",
        old_text=RUST_SOURCE,
        new_text=rust_source(
            """
    pub fn reset(&mut self) -> i32 {
        self.value = 0;
        self.value
    }
""",
            """
    #[test]
    fn reset_returns_zero() {
        assert_eq!(Counter::new(3).reset(), 0);
    }
""",
        ),
        test_file=None,
        test_text=None,
        validation=("CARGO", "test", "--quiet"),
        expected_changes=("M\tsrc/lib.rs",),
    ),
)


def python_test(method: str, expression: str, expected: int) -> str:
    return f"""import unittest

from counter import Counter


class CounterMutationPilotTest(unittest.TestCase):
    def test_{method}(self) -> None:
        self.assertEqual({expression}, {expected})
"""


PYTHON_CASES = (
    LanguageCase(
        case_id="python-decrement",
        language="python",
        fixture="python-basic",
        compilations=("python:.#counter", "python:.#tests"),
        intent="add a one-unit decrement operation with a focused native test",
        term="Counter",
        source_file="counter/__init__.py",
        old_text=PYTHON_SOURCE,
        new_text=PYTHON_SOURCE
        + """

    def decrement(self) -> int:
        self.value -= 1
        return self.value
""",
        test_file="tests/test_decrement_pilot.py",
        test_text=python_test("decrement", "Counter(3).decrement()", 2),
        validation=("PYTHON", "-m", "unittest", "discover", "-s", "tests", "-q"),
        expected_changes=(
            "M\tcounter/__init__.py",
            "A\ttests/test_decrement_pilot.py",
        ),
    ),
    LanguageCase(
        case_id="python-increment-by",
        language="python",
        fixture="python-basic",
        compilations=("python:.#counter", "python:.#tests"),
        intent="add an explicit increment-by operation with a focused native test",
        term="Counter",
        source_file="counter/__init__.py",
        old_text=PYTHON_SOURCE,
        new_text=PYTHON_SOURCE
        + """

    def increment_by(self, amount: int) -> int:
        self.value += amount
        return self.value
""",
        test_file="tests/test_increment_by_pilot.py",
        test_text=python_test("increment_by", "Counter(3).increment_by(4)", 7),
        validation=("PYTHON", "-m", "unittest", "discover", "-s", "tests", "-q"),
        expected_changes=(
            "M\tcounter/__init__.py",
            "A\ttests/test_increment_by_pilot.py",
        ),
    ),
    LanguageCase(
        case_id="python-reset",
        language="python",
        fixture="python-basic",
        compilations=("python:.#counter", "python:.#tests"),
        intent="add an explicit reset operation with a focused native test",
        term="Counter",
        source_file="counter/__init__.py",
        old_text=PYTHON_SOURCE,
        new_text=PYTHON_SOURCE
        + """

    def reset(self) -> int:
        self.value = 0
        return self.value
""",
        test_file="tests/test_reset_pilot.py",
        test_text=python_test("reset", "Counter(3).reset()", 0),
        validation=("PYTHON", "-m", "unittest", "discover", "-s", "tests", "-q"),
        expected_changes=(
            "M\tcounter/__init__.py",
            "A\ttests/test_reset_pilot.py",
        ),
    ),
)

CASES = (*RUST_CASES, *PYTHON_CASES)


def extract_fixture(repository: Path, fixture: str) -> None:
    archive = pilot.command(
        ["git", "archive", f"HEAD:fixtures/{fixture}"],
        cwd=ROOT,
        environment=dict(os.environ),
        error_code="FIXTURE_ARCHIVE_FAILED",
    ).stdout.encode()
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as stream:
        for member in stream.getmembers():
            relative = Path(member.name)
            if (
                member.issym()
                or member.islnk()
                or relative.is_absolute()
                or ".." in relative.parts
            ):
                raise pilot.PilotFailure("UNSAFE_FIXTURE")
        stream.extractall(repository)


def native_command(case: LanguageCase) -> list[str]:
    if case.language == "rust":
        return ["cargo", "test", "--quiet"]
    return ["python3", "-m", "unittest", "discover", "-s", "tests", "-q"]


def run_case(
    case: LanguageCase,
    repository: Path,
    plan_path: Path,
    environment: dict[str, str],
) -> tuple[dict[str, object], str]:
    authority: dict[str, str | bool | None] = {
        "session": None,
        "run": None,
        "cleaned": False,
    }
    total_started = time.monotonic()
    try:
        baseline = pilot.git(repository, "rev-parse", "HEAD")
        baseline_started = time.monotonic()
        pilot.command(
            native_command(case),
            cwd=repository,
            environment=environment,
            error_code="NATIVE_BASELINE_FAILED",
        )
        native_baseline_ms = pilot.millis(baseline_started)
        open_arguments = [
            "change",
            "open",
            "--repo",
            str(repository),
            "--target-ref",
            "main",
            "--language",
            case.language,
        ]
        for compilation in case.compilations:
            open_arguments.extend(["--compilation", compilation])
        open_arguments.extend(
            [
                "--intent",
                case.intent,
                "--term",
                case.term,
                "--max-roots",
                "4",
            ]
        )
        open_started = time.monotonic()
        _, opened = pilot.clew(
            open_arguments,
            environment=environment,
            error_code="CHANGE_OPEN_FAILED",
        )
        open_ms = pilot.millis(open_started)
        session_row = opened.get("session")
        context = opened.get("context")
        pilot.require(
            isinstance(session_row, dict) and isinstance(context, dict),
            "INVALID_CHANGE_OPEN",
        )
        session = str(session_row["sessionId"])
        authority["session"] = session
        context_id = str(context["contextId"])
        sources = pilot.source_authorities(context)
        pilot.require(case.source_file in sources, "MISSING_SOURCE_AUTHORITY")
        operations: list[dict[str, object]] = [
            {
                "kind": "REPLACE_TEXT",
                "opId": f"{case.case_id}-source",
                "target": {
                    "fileId": case.source_file,
                    "contentRef": sources[case.source_file],
                },
                "oldText": case.old_text,
                "newText": case.new_text,
            }
        ]
        if case.test_file is not None and case.test_text is not None:
            operations.append(
                {
                    "kind": "CREATE_FILE",
                    "opId": f"{case.case_id}-test",
                    "target": {"fileId": case.test_file},
                    "text": case.test_text,
                }
            )
        launcher, *validation_arguments = case.validation
        plan = {
            "schema": "codeclew-task-plan/2.0",
            "operations": operations,
            "validation": [
                {"launcher": launcher, "args": validation_arguments}
            ],
        }
        plan_path.write_text(
            json.dumps(plan, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
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
        prepare_started = time.monotonic()
        _, prepared = pilot.clew(
            prepare_arguments,
            environment=environment,
            error_code="CHANGE_PREPARE_FAILED",
        )
        run_row = prepared.get("run")
        pilot.require(isinstance(run_row, dict), "INVALID_CHANGE_PREPARE")
        run = str(run_row["runId"])
        authority["run"] = run
        _, repeated = pilot.clew(
            prepare_arguments,
            environment=environment,
            error_code="CHANGE_PREPARE_RETRY_FAILED",
        )
        pilot.require(
            isinstance(repeated.get("run"), dict)
            and repeated["run"]["runId"] == run,
            "NON_IDEMPOTENT_PREPARE",
        )
        deadline = time.monotonic() + 180
        while True:
            _, status = pilot.clew(
                ["change", "status", "--run", run],
                environment=environment,
                error_code="CHANGE_STATUS_FAILED",
            )
            status_run = status.get("run")
            pilot.require(isinstance(status_run, dict), "INVALID_CHANGE_STATUS")
            run_status = str(status_run["status"])
            if run_status in pilot.TERMINAL:
                break
            if time.monotonic() >= deadline:
                raise pilot.PilotFailure("PREPARE_TIMEOUT")
            time.sleep(0.2)
        prepare_ms = pilot.millis(prepare_started)
        pilot.require(
            run_status == "READY_TO_PUBLISH_CONDITIONAL",
            "NOT_READY_CONDITIONAL",
        )
        pilot.require(
            pilot.git(repository, "status", "--porcelain") == "",
            "SOURCE_MUTATED_BEFORE_PUBLISH",
        )
        pilot.require(
            pilot.git(repository, "rev-parse", "HEAD") == baseline
            and pilot.git(repository, "rev-parse", "main") == baseline,
            "SOURCE_REF_MOVED",
        )
        candidate = status.get("candidate")
        pilot.require(isinstance(candidate, dict), "MISSING_CANDIDATE")
        obligations = candidate.get("qualifiedObligations")
        pilot.require(
            isinstance(obligations, list) and bool(obligations),
            "MISSING_OBLIGATIONS",
        )
        approval_ids = sorted(
            str(row["approvalId"]) for row in obligations if isinstance(row, dict)
        )
        pilot.require(len(approval_ids) == len(obligations), "INVALID_OBLIGATIONS")
        refused, refusal = pilot.clew(
            ["change", "publish", "--session", session, "--run", run],
            environment=environment,
            error_code="STRICT_PUBLISH_EXECUTION_FAILED",
            check=False,
        )
        pilot.require(refused.returncode != 0, "STRICT_PUBLISH_NOT_REFUSED")
        refusal_error = refusal.get("error")
        pilot.require(
            isinstance(refusal_error, dict)
            and refusal_error.get("code") == "INCOMPLETE_SEMANTIC_ANALYSIS",
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
        _, published = pilot.clew(
            publish_arguments,
            environment=environment,
            error_code="CONDITIONAL_PUBLISH_FAILED",
        )
        _, repeated_publish = pilot.clew(
            publish_arguments,
            environment=environment,
            error_code="PUBLISH_RETRY_FAILED",
        )
        publish_ms = pilot.millis(publish_started)
        published_run = published.get("run")
        repeated_run = repeated_publish.get("run")
        pilot.require(
            isinstance(published_run, dict)
            and isinstance(repeated_run, dict)
            and published_run.get("status") == "PUBLISHED_CONDITIONAL"
            and repeated_run.get("finalCommit") == published_run.get("finalCommit"),
            "INVALID_PUBLISH_RESULT",
        )
        changed = tuple(
            pilot.git(
                repository,
                "diff",
                "--name-status",
                "--no-renames",
                baseline,
                "HEAD",
            ).splitlines()
        )
        pilot.require(changed == case.expected_changes, "UNEXPECTED_WRITE_SET")
        pilot.require(
            pilot.git(repository, "rev-list", "--count", f"{baseline}..HEAD")
            == "1",
            "UNEXPECTED_COMMIT_COUNT",
        )
        pilot.command(
            native_command(case),
            cwd=repository,
            environment=environment,
            error_code="NATIVE_POST_TEST_FAILED",
        )
        pilot.clew(
            ["session", "close", "--session", session],
            environment=environment,
            error_code="SESSION_CLOSE_FAILED",
        )
        pilot.clew(
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
                    "prepareToReady": prepare_ms,
                    "publish": publish_ms,
                    "total": pilot.millis(total_started),
                },
                "errorCode": None,
                "status": "PASSED",
            },
            str(session_row["runtimeMode"]),
        )
    finally:
        session_value = authority["session"]
        if isinstance(session_value, str) and not authority["cleaned"]:
            run_value = authority["run"]
            pilot.cleanup_case(
                session_value,
                run_value if isinstance(run_value, str) else None,
                environment,
            )


def public_summary(
    results: list[dict[str, object]], runtime_mode: str | None, prime_ms: int
) -> dict[str, object]:
    per_language = {}
    for language in ("python", "rust"):
        rows = [row for row in results if str(row.get("caseId", "")).startswith(language)]
        per_language[language] = {
            "attempted": len(rows),
            "passed": sum(row.get("status") == "PASSED" for row in rows),
            "total": 3,
        }
    passed = sum(row.get("status") == "PASSED" for row in results)
    value = {
        "aggregate": {"attempted": len(results), "passed": passed, "total": len(CASES)},
        "cases": results,
        "languages": per_language,
        "primeMs": prime_ms,
        "runtimeMode": runtime_mode,
        "schema": "codeclew-language-mutation-pilot/1.0",
        "status": "PASSED" if passed == len(CASES) else "FAILED",
    }
    pilot.validate_public_value(value, max_list=len(CASES))
    return value


def main() -> int:
    signal.signal(signal.SIGINT, pilot.interrupt_as_failure)
    signal.signal(signal.SIGTERM, pilot.interrupt_as_failure)
    parser = argparse.ArgumentParser()
    parser.add_argument("--reuse-primed-runtime", action="store_true")
    arguments = parser.parse_args()
    with pilot.PilotWorkspace() as workspace:
        assert workspace.path is not None
        root = workspace.path
        configured_state = os.environ.get("CODECLEW_HOME")
        state = Path(configured_state).resolve() if configured_state else root / "state"
        state.mkdir(mode=0o700, parents=True, exist_ok=True)
        environment = {**os.environ, "CODECLEW_HOME": str(state)}
        prime_ms = 0
        if not arguments.reuse_primed_runtime:
            prime_started = time.monotonic()
            pilot.command(
                [str(ROOT / "clew"), "--version"],
                cwd=ROOT,
                environment=environment,
                error_code="RUNTIME_PRIME_FAILED",
            )
            prime_ms = pilot.millis(prime_started)

        def execute(case: LanguageCase) -> tuple[dict[str, object], str]:
            repository = root / case.case_id
            repository.mkdir(mode=0o700)
            extract_fixture(repository, case.fixture)
            pilot.git(repository, "init", "-q", "-b", "main")
            pilot.git(repository, "config", "user.name", "Codeclew Pilot")
            pilot.git(repository, "config", "user.email", "pilot@codeclew.invalid")
            pilot.git(repository, "add", ".")
            pilot.git(repository, "commit", "-q", "-m", "baseline")
            return run_case(
                case,
                repository,
                root / f"{case.case_id}.json",
                environment,
            )

        results, runtime_mode = pilot.execute_cases(CASES, execute)
        recovery_required = any(
            row.get("errorCode") == "PILOT_RECOVERY_REQUIRED" for row in results
        )
        workspace.preserve = recovery_required
        summary = public_summary(results, runtime_mode, prime_ms)
        exit_code = 0 if summary["status"] == "PASSED" else (2 if recovery_required else 1)
    print(json.dumps(summary, sort_keys=True, separators=(",", ":")))
    return exit_code


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except pilot.PilotFailure as error:
        result = {
            "caseId": "bootstrap",
            "durationsMs": {},
            "errorCode": error.code,
            "status": "FAILED",
        }
        print(json.dumps(public_summary([result], None, 0), sort_keys=True, separators=(",", ":")))
        raise SystemExit(2 if error.code == "PILOT_RECOVERY_REQUIRED" else 1) from None
    except Exception:
        result = {
            "caseId": "internal",
            "durationsMs": {},
            "errorCode": "PILOT_INTERNAL_FAILED",
            "status": "FAILED",
        }
        print(json.dumps(public_summary([result], None, 0), sort_keys=True, separators=(",", ":")))
        raise SystemExit(1) from None
