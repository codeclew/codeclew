#!/usr/bin/env python3
"""One acceptance-bearing Kotlin 2.4/Gradle conditional publish smoke."""

from __future__ import annotations

import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import time


ROOT = Path(__file__).resolve().parent.parent
TERMINAL = {
    "READY_TO_PUBLISH",
    "READY_TO_PUBLISH_CONDITIONAL",
    "VALIDATED_CONDITIONAL",
    "FAILED",
    "WORKTREE_RECOVERY_REQUIRED",
    "CANCELLED",
}


def command(
    arguments: list[str],
    *,
    cwd: Path,
    environment: dict[str, str],
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
        raise AssertionError(
            f"command {Path(arguments[0]).name} failed with {completed.returncode}: "
            f"{completed.stdout[-2000:]} {completed.stderr[-2000:]}"
        )
    return completed


def clew(
    arguments: list[str],
    *,
    environment: dict[str, str],
    check: bool = True,
) -> tuple[subprocess.CompletedProcess[str], dict[str, object]]:
    completed = command(
        [str(ROOT / "clew"), *arguments],
        cwd=ROOT,
        environment=environment,
        check=check,
    )
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise AssertionError("clew stdout is not one JSON object") from error
    return completed, value


def git(repository: Path, *arguments: str) -> str:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=repository,
        check=True,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
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
                raise AssertionError("fixture archive contains an unsafe member")
        stream.extractall(repository)
    os.chmod(repository / "gradlew", 0o755)


def source_authorities(context: dict[str, object]) -> dict[str, dict[str, object]]:
    projection = context["context"]
    assert isinstance(projection, dict)
    rows = projection["sources"]
    assert isinstance(rows, list)
    return {
        str(row["fileId"]): row["contentRef"]
        for row in rows
        if isinstance(row, dict) and "fileId" in row and "contentRef" in row
    }


def assert_private_output(value: object, forbidden: list[Path]) -> None:
    encoded = json.dumps(value, sort_keys=True)
    for path in forbidden:
        assert str(path) not in encoded, "public JSON contains a private absolute path"


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="codeclew-usability-") as temporary:
        temporary_root = Path(temporary).resolve()
        repository = temporary_root / "product"
        state = temporary_root / "state"
        repository.mkdir(mode=0o700)
        state.mkdir(mode=0o700)
        extract_fixture(repository)
        git(repository, "init", "-q", "-b", "main")
        git(repository, "config", "user.name", "Codeclew Smoke")
        git(repository, "config", "user.email", "codeclew-smoke@localhost")
        git(repository, "add", ".")
        git(repository, "commit", "-q", "-m", "baseline")
        base = git(repository, "rev-parse", "HEAD")
        environment = dict(os.environ)
        environment["CODECLEW_HOME"] = str(state)

        command(
            [str(repository / "gradlew"), "test", "--no-daemon", "--quiet"],
            cwd=repository,
            environment=environment,
        )
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
                "define the zero-base total result and add its exact test",
                "--term",
                "com.acme.total",
                "--max-roots",
                "4",
            ],
            environment=environment,
        )
        session = str(opened["session"]["sessionId"])
        runtime_mode = str(opened["session"]["runtimeMode"])
        assert runtime_mode in {"DEVELOPMENT", "RELEASE"}
        context = opened["context"]
        assert isinstance(context, dict)
        context_id = str(context["contextId"])
        completeness = context["completeness"]
        assert isinstance(completeness, dict)
        assert completeness["status"] == "CONDITIONAL_TASK"
        assert completeness["certainty"] == "UNSURE"
        assert_private_output(context, [repository, state])

        sources = source_authorities(context)
        main_file = "src/main/kotlin/com/acme/Samples.kt"
        test_file = "src/test/kotlin/com/acme/CodeclewTotalTest.kt"
        assert main_file in sources, sorted(sources)
        plan = {
            "schema": "codeclew-task-plan/2.0",
            "operations": [
                {
                    "kind": "REPLACE_TEXT",
                    "opId": "increment-total-result",
                    "target": {"fileId": main_file, "contentRef": sources[main_file]},
                    "oldText": "    return value\n}",
                    "newText": "    return if (base == 0) 1 else value\n}",
                },
                {
                    "kind": "CREATE_FILE",
                    "opId": "create-zero-total-test",
                    "target": {"fileId": test_file},
                    "text": (
                        "package com.acme\n\n"
                        "import kotlin.test.Test\n"
                        "import kotlin.test.assertEquals\n\n"
                        "class CodeclewTotalTest {\n"
                        "    @Test fun zeroBaseHasExplicitResult() { assertEquals(1, total(0, false)) }\n"
                        "}\n"
                    ),
                },
            ],
            "validation": [
                {
                    "launcher": "GRADLE",
                    "args": ["test", "--no-daemon", "--quiet"],
                }
            ],
        }
        plan_path = temporary_root / "plan.json"
        plan_path.write_text(
            json.dumps(plan, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        _, first_start = clew(
            [
                "change",
                "prepare",
                "--session",
                session,
                "--context",
                context_id,
                "--plan",
                str(plan_path),
            ],
            environment=environment,
        )
        plan_id = str(first_start["planId"])
        run = str(first_start["run"]["runId"])
        _, second_start = clew(
            [
                "change",
                "prepare",
                "--session",
                session,
                "--context",
                context_id,
                "--plan",
                str(plan_path),
            ],
            environment=environment,
        )
        assert second_start["planId"] == plan_id
        assert second_start["run"]["runId"] == run

        deadline = time.monotonic() + 180
        status: dict[str, object]
        while True:
            _, status = clew(["change", "status", "--run", run], environment=environment)
            run_status = str(status["run"]["status"])
            if run_status in TERMINAL:
                break
            if time.monotonic() >= deadline:
                raise AssertionError("task run did not reach a ready state")
            time.sleep(0.2)
        assert run_status == "READY_TO_PUBLISH_CONDITIONAL", status
        assert git(repository, "rev-parse", "HEAD") == base
        assert git(repository, "status", "--porcelain") == ""
        candidate = status["candidate"]
        assert isinstance(candidate, dict)
        assert candidate["diff"]["overLimit"] is False
        patch = str(candidate["diff"]["patch"])
        assert "base == 0" in patch and "zeroBaseHasExplicitResult" in patch
        obligations = candidate["qualifiedObligations"]
        assert isinstance(obligations, list) and obligations
        approval_ids = [str(item["approvalId"]) for item in obligations]
        assert approval_ids == sorted(set(approval_ids))
        assert_private_output(status, [repository, state])

        refused, refused_value = clew(
            ["change", "publish", "--session", session, "--run", run],
            environment=environment,
            check=False,
        )
        assert refused.returncode != 0
        assert refused_value["error"]["code"] == "INCOMPLETE_SEMANTIC_ANALYSIS"
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
        _, published = clew(publish_arguments, environment=environment)
        assert published["run"]["status"] == "PUBLISHED_CONDITIONAL"
        _, published_again = clew(publish_arguments, environment=environment)
        assert published_again["run"]["finalCommit"] == published["run"]["finalCommit"]
        assert git(repository, "rev-list", "--count", f"{base}..HEAD") == "1"
        changed = git(repository, "diff", "--name-only", base, "HEAD").splitlines()
        assert changed == [main_file, test_file]
        command(
            [str(repository / "gradlew"), "test", "--no-daemon", "--quiet"],
            cwd=repository,
            environment=environment,
        )
        clew(["session", "close", "--session", session], environment=environment)
        clew(["session", "gc", "--session", session], environment=environment)
        assert len(git(repository, "worktree", "list", "--porcelain").split("worktree ")) == 2
        assert not (repository / ".semantic-thread").exists()

        print(
            json.dumps(
                {
                    "runtimeMode": runtime_mode,
                    "schema": "codeclew-usability-smoke/1.0",
                    "status": "PASSED",
                },
                sort_keys=True,
                separators=(",", ":"),
            )
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
