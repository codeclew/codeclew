#!/usr/bin/env python3

from __future__ import annotations

import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "task_apply_runner.py"
SPEC = importlib.util.spec_from_file_location("task_apply_runner", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)


FAKE_CLEW = r'''#!/usr/bin/env python3
import json
import os
from pathlib import Path
import signal
import sys
import time

root = Path(__file__).resolve().parent
mode = json.loads((root / "mode.json").read_text())
arguments = sys.argv[1:]
if "task-apply" in arguments:
    repository = Path(arguments[arguments.index("--repo") + 1])
    run_id = os.environ["CODECLEW_TASK_RUN_ID"]
    run_dir = repository / ".semantic-thread" / "task-runs" / run_id.removeprefix("task-run:")
    required = [
        run_dir / "request.json", run_dir / "context.json", run_dir / "edit-plan.json",
        run_dir / "run.lock", run_dir / "child.lock", run_dir / "child.json",
    ]
    if not all(path.is_file() for path in required):
        print(json.dumps({"schema": "semantic-error/0.1", "error": "runner record missing"}))
        raise SystemExit(2)
    with (root / "invocations.log").open("a", encoding="utf-8") as handle:
        handle.write("task-apply\n")
        handle.flush()
        os.fsync(handle.fileno())
    time.sleep(mode.get("sleep", 0))
    if mode.get("signal"):
        os.kill(os.getpid(), signal.SIGKILL)
    transaction_id = arguments[arguments.index("--transaction-id") + 1]
    output = Path(arguments[arguments.index("--output") + 1])
    terminal_status = mode.get("transactionStatus", "COMMITTED")
    transaction = {
        "schema": "semantic-transaction/0.1",
        "txId": transaction_id,
        "status": terminal_status,
        "finalCommit": "f" * 40 if terminal_status == "COMMITTED" else None,
        "candidateCommit": "c" * 40 if terminal_status == "VALIDATED_CONDITIONAL" else None,
    }
    output.write_text(json.dumps(transaction), encoding="utf-8")
    print(json.dumps({
        "schema": "semantic-task-apply-receipt/0.1",
        "status": terminal_status,
        "finalCommit": "f" * 40 if terminal_status == "COMMITTED" else None,
    }))
    raise SystemExit(0)
if "inspect" in arguments:
    transaction_id = arguments[arguments.index("--transaction-id") + 1]
    print(json.dumps({
        "schema": "semantic-ledger/0.1",
        "transactionId": transaction_id,
        "reconciledStatus": "COMMITTED",
    }))
    raise SystemExit(0)
print(json.dumps({"schema": "semantic-error/0.1", "error": "unexpected command"}))
raise SystemExit(2)
'''


class TaskApplyRunnerIntegrationTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="codeclew-task-runner-test-")
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        subprocess.run(["git", "init", "-q", "-b", "main"], cwd=self.repo, check=True)
        (self.repo / "seed.txt").write_text("seed\n", encoding="utf-8")
        subprocess.run(["git", "add", "seed.txt"], cwd=self.repo, check=True)
        subprocess.run(
            [
                "git",
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                "seed",
            ],
            cwd=self.repo,
            check=True,
        )
        self.context = self.root / "context.json"
        self.plan = self.root / "plan.json"
        self.context.write_bytes(b'{"context":1}\n')
        self.plan.write_bytes(b'{"operations":[]}\n')
        self.fake = self.root / "fake-clew"
        self.fake.write_text(FAKE_CLEW, encoding="utf-8")
        self.fake.chmod(0o755)
        self.mode = self.root / "mode.json"
        self.set_mode()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def set_mode(
        self,
        *,
        sleep: float = 0,
        send_signal: bool = False,
        transaction_status: str = "COMMITTED",
    ) -> None:
        self.mode.write_text(
            json.dumps(
                {
                    "sleep": sleep,
                    "signal": send_signal,
                    "transactionStatus": transaction_status,
                }
            ),
            encoding="utf-8",
        )

    def start_command(self) -> list[str]:
        return [
            os.sys.executable,
            str(SCRIPT),
            "start",
            "--clew",
            str(self.fake),
            "--repo",
            str(self.repo),
            "--context",
            str(self.context),
            "--edit-plan",
            str(self.plan),
            "--target-ref",
            "main",
            "--actor",
            "runner-test",
            "--compiler-index-root",
            str(self.root / "compiler-index"),
            "--handshake-seconds",
            "0.1",
        ]

    def invoke(self, command: list[str]) -> dict[str, object]:
        result = subprocess.run(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            text=True,
            timeout=10,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads(result.stdout)

    def status(self, start: dict[str, object]) -> dict[str, object]:
        return self.invoke(list(start["statusCommand"]))

    def await_terminal(
        self, start: dict[str, object], timeout: float = 5
    ) -> dict[str, object]:
        deadline = time.monotonic() + timeout
        current = start
        while not current["terminal"] and time.monotonic() < deadline:
            time.sleep(0.05)
            current = self.status(start)
        self.assertTrue(current["terminal"], current)
        return current

    def invocation_count(self) -> int:
        path = self.root / "invocations.log"
        return 0 if not path.exists() else len(path.read_text().splitlines())

    def test_concurrent_attach_runs_task_apply_exactly_once(self) -> None:
        self.set_mode(sleep=0.35)
        first_process = subprocess.Popen(
            self.start_command(), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
        )
        second_process = subprocess.Popen(
            self.start_command(), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
        )
        first_stdout, first_stderr = first_process.communicate(timeout=10)
        second_stdout, second_stderr = second_process.communicate(timeout=10)
        self.assertEqual(first_process.returncode, 0, first_stdout + first_stderr)
        self.assertEqual(second_process.returncode, 0, second_stdout + second_stderr)
        first = json.loads(first_stdout)
        second = json.loads(second_stdout)
        self.assertEqual(first["runId"], second["runId"])
        terminal = self.await_terminal(first)
        self.assertEqual(terminal["state"], "SUCCEEDED")
        self.assertEqual(self.invocation_count(), 1)
        attached = self.invoke(self.start_command())
        self.assertEqual(attached["state"], "SUCCEEDED")
        self.assertEqual(self.invocation_count(), 1)

    def test_supervisor_death_keeps_child_visible_and_recovers_completion(self) -> None:
        self.set_mode(sleep=0.8)
        started = self.invoke(self.start_command())
        run_dir = Path(started["detail"]["completionPath"]).parent
        deadline = time.monotonic() + 3
        child = None
        status = None
        while time.monotonic() < deadline:
            child_path = run_dir / "child.json"
            status_path = run_dir / "status.json"
            if child_path.exists() and status_path.exists():
                child = json.loads(child_path.read_text())
                status = json.loads(status_path.read_text())
                break
            time.sleep(0.02)
        self.assertIsNotNone(child)
        self.assertIsNotNone(status)
        os.kill(int(status["supervisorPid"]), signal.SIGKILL)
        deadline = time.monotonic() + 2
        unsupervised = None
        while time.monotonic() < deadline:
            current = self.status(started)
            if current["state"] == "RUNNING_UNSUPERVISED":
                unsupervised = current
                break
            time.sleep(0.02)
        self.assertIsNotNone(unsupervised)
        self.assertFalse(unsupervised["terminal"])
        terminal = self.await_terminal(started)
        self.assertEqual(terminal["state"], "SUCCEEDED")
        self.assertEqual(self.invocation_count(), 1)

    def test_signal_without_terminal_artifact_is_unknown_and_never_retried(self) -> None:
        self.set_mode(send_signal=True)
        started = self.invoke(self.start_command())
        terminal = self.await_terminal(started)
        self.assertEqual(terminal["state"], "UNKNOWN_REQUIRES_INSPECTION")
        inspect = terminal["transactionInspectCommand"]
        self.assertEqual(inspect[-2:], ["--transaction-id", terminal["transactionId"]])
        attached = self.invoke(self.start_command())
        self.assertEqual(attached["state"], "UNKNOWN_REQUIRES_INSPECTION")
        self.assertEqual(self.invocation_count(), 1)

    def test_conditional_validation_is_terminal_without_claiming_publication(self) -> None:
        self.set_mode(transaction_status="VALIDATED_CONDITIONAL")
        started = self.invoke(self.start_command())
        terminal = self.await_terminal(started)
        self.assertEqual(terminal["state"], "VALIDATED_CONDITIONAL")
        self.assertTrue(terminal["terminal"])
        completion = terminal["detail"]["completion"]
        self.assertEqual(completion["transactionStatus"], "VALIDATED_CONDITIONAL")
        self.assertIsNone(completion["taskApplyResult"]["finalCommit"])
        self.assertEqual(self.invocation_count(), 1)

    def test_request_digest_binds_exact_input_bytes(self) -> None:
        first = self.invoke(self.start_command())
        self.await_terminal(first)
        self.context.write_bytes(b'{\n  "context": 1\n}\n')
        second = self.invoke(self.start_command())
        self.assertNotEqual(first["runId"], second["runId"])
        self.await_terminal(second)
        self.assertEqual(self.invocation_count(), 2)

    def test_transaction_binding_is_create_only(self) -> None:
        arguments = argparse.Namespace(
            clew=str(self.fake),
            repo=str(self.repo),
            context=str(self.context),
            edit_plan=str(self.plan),
            target_ref="main",
            actor="runner-test",
            compiler_index_root=None,
            allow_legacy_heuristic=False,
        )
        request, _ = RUNNER._prepare_request(arguments)
        transaction_uuid = request["transactionId"].removeprefix("tx:")
        binding = (
            self.repo
            / ".semantic-thread"
            / "task-runs"
            / "transactions"
            / f"{transaction_uuid}.json"
        )
        binding.write_text('{"forged":true}\n', encoding="utf-8")
        with self.assertRaisesRegex(RUNNER.RunnerError, "different bytes"):
            RUNNER._prepare_request(arguments)


if __name__ == "__main__":
    unittest.main()
