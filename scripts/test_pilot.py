#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
from pathlib import Path
import subprocess
import shutil
import sys
import unittest


MODULE_PATH = Path(__file__).resolve().with_name("pilot.py")
SPEC = importlib.util.spec_from_file_location("codeclew_pilot", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
pilot = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = pilot
SPEC.loader.exec_module(pilot)


class PilotTest(unittest.TestCase):
    def test_execute_cases_is_canonical_and_fail_fast(self) -> None:
        calls: list[str] = []

        def execute(case: pilot.PilotCase) -> tuple[dict[str, object], str]:
            calls.append(case.case_id)
            if len(calls) == 2:
                raise pilot.PilotFailure("CHANGE_OPEN_FAILED")
            return (
                {
                    "caseId": case.case_id,
                    "durationsMs": {"total": 1},
                    "errorCode": None,
                    "status": "PASSED",
                },
                "DEVELOPMENT",
            )

        results, mode = pilot.execute_cases(pilot.CASES, execute)
        self.assertEqual(calls, ["total-boundary", "classify-edge"])
        self.assertEqual(mode, "DEVELOPMENT")
        self.assertEqual(results[-1]["errorCode"], "CHANGE_OPEN_FAILED")
        self.assertEqual(results[-1]["status"], "FAILED")

    def test_public_summary_is_bounded_and_rejects_paths(self) -> None:
        results = [
            {
                "caseId": case.case_id,
                "durationsMs": {"total": index},
                "errorCode": None,
                "status": "PASSED",
            }
            for index, case in enumerate(pilot.CASES, 1)
        ]
        summary = pilot.public_summary(results, "RELEASE", 7)
        self.assertEqual(summary["status"], "PASSED")
        self.assertEqual(summary["aggregate"], {"attempted": 3, "passed": 3, "total": 3})
        results[0]["errorCode"] = "/private/repository"
        with self.assertRaises(ValueError):
            pilot.public_summary(results, "RELEASE", 7)

    def test_failure_after_start_cancels_before_abort_and_gc(self) -> None:
        calls: list[tuple[str, ...]] = []

        def invoke(
            arguments: list[str], **_kwargs: object
        ) -> tuple[subprocess.CompletedProcess[str], dict[str, object]]:
            calls.append(tuple(arguments))
            completed = subprocess.CompletedProcess(arguments, 0, "{}", "")
            if arguments[:2] == ["change", "status"]:
                return completed, {"run": {"status": "PREPARING"}}
            if arguments[:2] == ["task-run", "cancel"]:
                return completed, {"run": {"status": "CANCELLED"}}
            return completed, {}

        pilot.cleanup_case(
            "session:authority", "run:authority", {}, invoke=invoke
        )
        self.assertEqual(
            calls,
            [
                ("change", "status", "--run", "run:authority"),
                ("task-run", "cancel", "--run", "run:authority"),
                ("session", "abort", "--session", "session:authority"),
                ("session", "gc", "--session", "session:authority"),
            ],
        )

    def test_ready_failure_preserves_recovery_authority(self) -> None:
        completed = subprocess.CompletedProcess([], 0, "{}", "")

        def invoke(
            _arguments: list[str], **_kwargs: object
        ) -> tuple[subprocess.CompletedProcess[str], dict[str, object]]:
            return completed, {"run": {"status": "READY_TO_PUBLISH_CONDITIONAL"}}

        with self.assertRaises(pilot.PilotFailure) as caught:
            pilot.cleanup_case(
                "session:authority", "run:authority", {}, invoke=invoke
            )
        self.assertEqual(caught.exception.code, "PILOT_RECOVERY_REQUIRED")
        self.assertEqual(caught.exception.session_id, "session:authority")
        self.assertEqual(caught.exception.run_id, "run:authority")

    def test_signal_becomes_typed_failure_for_finally_cleanup(self) -> None:
        with self.assertRaises(pilot.PilotFailure) as caught:
            pilot.interrupt_as_failure(15, None)
        self.assertEqual(caught.exception.code, "PILOT_SIGNALLED")

    def test_recovery_preserves_repository_workspace(self) -> None:
        with pilot.PilotWorkspace() as disposable:
            assert disposable.path is not None
            disposable_path = disposable.path
        self.assertFalse(disposable_path.exists())

        with pilot.PilotWorkspace() as recovery:
            assert recovery.path is not None
            recovery_path = recovery.path
            recovery.preserve = True
        self.assertTrue(recovery_path.is_dir())
        shutil.rmtree(recovery_path)


if __name__ == "__main__":
    unittest.main()
