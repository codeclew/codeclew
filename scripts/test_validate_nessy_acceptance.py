#!/usr/bin/env python3
"""Contract tests for scripts/validate_nessy_acceptance.py."""

from __future__ import annotations

import json
import shutil
import tempfile
import uuid
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUNS_ROOT = ROOT / ".nessy" / "a2a-runs"
CHECKER = ROOT / "scripts" / "validate_nessy_acceptance.py"


def load_checker():
    import importlib.util
    spec = importlib.util.spec_from_file_location("vna", CHECKER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def make_plan(**overrides):
    plan = {
        "version": 1,
        "runId": "synth-run",
        "workspace": "/tmp/synth",
        "title": "Synthetic run",
        "goal": "Prove the checker accepts a conforming run.",
        "steps": [
            {
                "id": "step-a",
                "description": "A step.",
                "commands": ["true"],
                "acceptanceCriteria": ["A criterion is met."],
            }
        ],
        "acceptanceCriteria": ["Every criterion is met."],
        "constraints": {"forbiddenActions": ["git push"]},
        "executionLimits": {"maxWallTimeSeconds": 60, "maxToolCalls": 10},
        "verificationCommands": ["true"],
        "reportContract": {"schemaVersion": 1},
    }
    plan.update(overrides)
    return plan


def make_status(**overrides):
    status = {
        "version": 1,
        "runId": "synth-run",
        "status": "completed",
        "startedAt": "2026-09-17T00:00:00Z",
        "updatedAt": "2026-09-17T00:01:00Z",
        "finishedAt": "2026-09-17T00:01:00Z",
        "currentStepId": "step-a",
    }
    status.update(overrides)
    return status


def make_report(**overrides):
    report = {
        "version": 1,
        "runId": "synth-run",
        "status": "completed",
        "startedAt": "2026-09-17T00:00:00Z",
        "finishedAt": "2026-09-17T00:01:00Z",
        "summary": "All done.",
        "steps": [
            {
                "id": "step-a",
                "status": "completed",
                "summary": "Step done.",
                "filesInspected": ["a.txt"],
                "filesChanged": ["b.txt"],
                "unmetCriteria": [],
            }
        ],
        "verification": [
            {"command": "true", "exitCode": 0, "outputSummary": "ok"}
        ],
        "filesChanged": ["b.txt"],
        "unmetAcceptanceCriteria": [],
        "followUps": [],
        "blockingQuestion": None,
    }
    report.update(overrides)
    return report


def make_events():
    return [
        {"version": 1, "event": "started", "runId": "synth-run", "at": "2026-09-17T00:00:00Z"},
        {"version": 1, "event": "step-start", "runId": "synth-run", "at": "2026-09-17T00:00:01Z", "step": "step-a"},
        {"version": 1, "event": "command", "runId": "synth-run", "at": "2026-09-17T00:00:02Z", "phase": "step", "step": "step-a", "command": "true", "exitCode": 0, "outputSummary": "ok"},
        {"version": 1, "event": "step-end", "runId": "synth-run", "at": "2026-09-17T00:00:03Z", "step": "step-a", "status": "completed"},
        {"version": 1, "event": "verification-start", "runId": "synth-run", "at": "2026-09-17T00:00:04Z"},
        {"version": 1, "event": "command", "runId": "synth-run", "at": "2026-09-17T00:00:05Z", "phase": "verification", "command": "true", "exitCode": 0, "outputSummary": "ok"},
        {"version": 1, "event": "verification-end", "runId": "synth-run", "at": "2026-09-17T00:00:06Z", "status": "completed"},
        {"version": 1, "event": "completed", "runId": "synth-run", "at": "2026-09-17T00:00:07Z"},
    ]


class CheckerTest(unittest.TestCase):
    def setUp(self) -> None:
        # Unique test-owned root under the runs directory. The test itself
        # creates it via mkdtemp, so tearDown never removes a directory the
        # current test did not create.
        self._run = Path(tempfile.mkdtemp(prefix="synth-", dir=str(RUNS_ROOT)))

    def tearDown(self) -> None:
        shutil.rmtree(self._run, ignore_errors=True)

    def _write(self, run_dir: Path, **artifacts) -> None:
        run_dir.mkdir(parents=True, exist_ok=True)
        for name, value in artifacts.items():
            (run_dir / f"{name}.json").write_text(json.dumps(value))
        (run_dir / "events.jsonl").write_text(
            "".join(json.dumps(e) + "\n" for e in make_events())
        )

    def _run_checker(self, *args: str) -> tuple[int, str]:
        import subprocess
        proc = subprocess.run(
            ["python3", "-I", "-S", str(CHECKER), *args],
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        return proc.returncode, proc.stdout + proc.stderr

    def test_conforming_synthetic_run_is_accepted(self) -> None:
        self._write(self._run, plan=make_plan(), status=make_status(), report=make_report())
        code, out = self._run_checker(self._run)
        self.assertEqual(code, 0, out)
        self.assertIn("VALID", out)

    def test_missing_files_inspected_is_rejected(self) -> None:
        report = make_report()
        del report["steps"][0]["filesInspected"]
        self._write(self._run, plan=make_plan(), status=make_status(), report=report)
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("filesInspected", out)

    def test_missing_top_level_files_changed_is_rejected(self) -> None:
        report = make_report()
        del report["filesChanged"]
        self._write(self._run, plan=make_plan(), status=make_status(), report=report)
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("filesChanged", out)

    def test_unsupported_extra_property_is_rejected(self) -> None:
        report = make_report()
        report["scenarioEvidence"] = {"x": 1}
        self._write(self._run, plan=make_plan(), status=make_status(), report=report)
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("scenarioEvidence", out)

    def test_unsupported_schema_version_property_is_rejected(self) -> None:
        report = make_report()
        report["schemaVersion"] = 1
        self._write(self._run, plan=make_plan(), status=make_status(), report=report)
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("schemaVersion", out)

    def test_empty_output_summary_is_rejected(self) -> None:
        report = make_report()
        report["verification"][0]["outputSummary"] = ""
        self._write(self._run, plan=make_plan(), status=make_status(), report=report)
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("outputSummary", out)

    def test_nonzero_verification_is_rejected(self) -> None:
        report = make_report()
        report["verification"][0]["exitCode"] = 1
        self._write(self._run, plan=make_plan(), status=make_status(), report=report)
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("exitCode", out)

    def test_omitted_verification_is_rejected(self) -> None:
        report = make_report()
        report["verification"] = []
        self._write(self._run, plan=make_plan(), status=make_status(), report=report)
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("verification", out)

    def test_mismatched_command_events_are_rejected(self) -> None:
        self._write(self._run, plan=make_plan(), status=make_status(), report=make_report())
        events_path = self._run / "events.jsonl"
        lines = events_path.read_text().splitlines()
        kept = [l for l in lines if '"phase": "verification"' not in l or '"event": "command"' not in l]
        events_path.write_text("\n".join(kept) + "\n")
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("verification command event", out)

    def test_nonempty_step_unmet_criteria_is_rejected(self) -> None:
        report = make_report()
        report["steps"][0]["unmetCriteria"] = ["not met"]
        self._write(self._run, plan=make_plan(), status=make_status(), report=report)
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("unmet criteria", out)

    def test_step_id_order_mismatch_is_rejected(self) -> None:
        plan = make_plan()
        plan["steps"][0]["id"] = "step-b"
        report = make_report()
        report["steps"][0]["id"] = "step-a"
        self._write(self._run, plan=plan, status=make_status(), report=report)
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("step IDs", out)

    def test_unsupported_schema_keyword_fails(self) -> None:
        plan = make_plan()
        plan["_bogusKeyword"] = True
        self._write(self._run, plan=plan, status=make_status(), report=make_report())
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)

    def test_preexisting_sentinel_is_not_deleted(self) -> None:
        # A sentinel the test itself creates must survive a checker run and a
        # full tearDown of other test fixtures.
        sentinel = RUNS_ROOT / ("sentinel-" + uuid.uuid4().hex)
        sentinel.mkdir(parents=True, exist_ok=True)
        marker = sentinel / "keep.txt"
        marker.write_text("keep me")
        try:
            self._write(self._run, plan=make_plan(), status=make_status(), report=make_report())
            self.assertEqual(self._run_checker(self._run)[0], 0)
            self.assertTrue(sentinel.exists())
            self.assertEqual(marker.read_text(), "keep me")
        finally:
            shutil.rmtree(sentinel, ignore_errors=True)

    def test_two_simultaneous_isolated_fixtures(self) -> None:
        first = Path(tempfile.mkdtemp(prefix="synth-", dir=str(RUNS_ROOT)))
        second = Path(tempfile.mkdtemp(prefix="synth-", dir=str(RUNS_ROOT)))
        try:
            self._write(first, plan=make_plan(), status=make_status(), report=make_report())
            self._write(second, plan=make_plan(), status=make_status(), report=make_report())
            # Write distinct content into the second fixture before checking the
            # first, then verify the second is unchanged and isolated.
            (second / "extra.txt").write_text("second-only")
            self.assertEqual(self._run_checker(first)[0], 0)
            self.assertEqual(self._run_checker(second)[0], 0)
            self.assertTrue((second / "extra.txt").exists())
            self.assertEqual((second / "extra.txt").read_text(), "second-only")
            self.assertFalse((first / "extra.txt").exists())
        finally:
            shutil.rmtree(first, ignore_errors=True)
            shutil.rmtree(second, ignore_errors=True)

    def test_event_schema_missing_conditional_required_fails(self) -> None:
        # The installed event.schema.json asserts required fields inside
        # conditionals without an explicit "type". A step-start without `step`
        # must fail schema validation.
        events = make_events()
        events[1] = {"version": 1, "event": "step-start", "runId": "synth-run", "at": "2026-09-17T00:00:01Z"}
        self._write(self._run, plan=make_plan(), status=make_status(), report=make_report())
        (self._run / "events.jsonl").write_text("".join(json.dumps(e) + "\n" for e in events))
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("step", out)

    def test_event_run_id_mismatch_fails(self) -> None:
        events = make_events()
        events[0]["runId"] = "other-run"
        self._write(self._run, plan=make_plan(), status=make_status(), report=make_report())
        (self._run / "events.jsonl").write_text("".join(json.dumps(e) + "\n" for e in events))
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("runId", out)

    def test_unbalanced_step_lifecycle_fails(self) -> None:
        events = [e for e in make_events() if e.get("event") != "step-end"]
        self._write(self._run, plan=make_plan(), status=make_status(), report=make_report())
        (self._run / "events.jsonl").write_text("".join(json.dumps(e) + "\n" for e in events))
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("step-start and step-end", out)

    def test_completed_with_failed_step_fails(self) -> None:
        report = make_report()
        report["steps"][0]["status"] = "failed"
        self._write(self._run, plan=make_plan(), status=make_status(), report=report)
        code, out = self._run_checker(self._run)
        self.assertNotEqual(code, 0)
        self.assertIn("non-completed step", out)

    def test_true_unsupported_schema_keyword_fails(self) -> None:
        vna = load_checker()
        schema = {"type": "object", "properties": {"x": {"type": "string"}}, "bogusKeyword": True}
        errors: list[str] = []
        validator = vna.MiniSchema(schema, errors)
        with self.assertRaises(vna.SchemaFailure):
            validator.validate({"x": "y"}, schema, "$", {})

    def test_failed_report_with_omitted_commands_passes_structural(self) -> None:
        # A truthful failed report may list only executed verification commands.
        plan = make_plan()
        plan["verificationCommands"] = ["true", "false"]
        report = make_report(status="failed")
        report["steps"][0]["status"] = "failed"
        report["verification"] = [{"command": "true", "exitCode": 0, "outputSummary": "ok"}]
        report["unmetAcceptanceCriteria"] = ["something unmet"]
        report["blockingQuestion"] = None
        status = make_status(status="failed")
        events = make_events()
        # Drop the second (fabricated) verification command and the completed end.
        events = [e for e in events if e.get("event") != "completed"]
        events.append({"version": 1, "event": "failed", "runId": "synth-run", "at": "2026-09-17T00:00:07Z"})
        self._write(self._run, plan=plan, status=status, report=report)
        (self._run / "events.jsonl").write_text("".join(json.dumps(e) + "\n" for e in events))
        code, out = self._run_checker(self._run)
        self.assertEqual(code, 0, out)

    def _write_failed_run(self) -> None:
        plan = make_plan()
        plan["verificationCommands"] = []
        report = make_report(status="failed")
        report["steps"][0]["status"] = "failed"
        report["steps"][0]["filesInspected"] = []
        report["verification"] = []
        report["unmetAcceptanceCriteria"] = ["unmet"]
        report["blockingQuestion"] = None
        status = make_status(status="failed")
        events = [
            {"version": 1, "event": "started", "runId": "synth-run", "at": "2026-09-17T00:00:00Z"},
            {"version": 1, "event": "failed", "runId": "synth-run", "at": "2026-09-17T00:00:07Z"},
        ]
        self._write(self._run, plan=plan, status=status, report=report)
        (self._run / "events.jsonl").write_text("".join(json.dumps(e) + "\n" for e in events))

    def test_run_alias_form_is_accepted(self) -> None:
        self._write(self._run, plan=make_plan(), status=make_status(), report=make_report())
        code, out = self._run_checker("--run", str(self._run))
        self.assertEqual(code, 0, out)

    def test_require_completed_passes_on_completed(self) -> None:
        self._write(self._run, plan=make_plan(), status=make_status(), report=make_report())
        code, out = self._run_checker("--require-completed", str(self._run))
        self.assertEqual(code, 0, out)

    def test_require_completed_fails_on_failed(self) -> None:
        self._write_failed_run()
        # Structural validation alone succeeds.
        code, _ = self._run_checker(str(self._run))
        self.assertEqual(code, 0)
        # --require-completed must reject a non-completed status even if valid.
        code, out = self._run_checker("--require-completed", str(self._run))
        self.assertEqual(code, 3)
        self.assertIn("NOT COMPLETED", out)

    def test_unknown_option_rejected(self) -> None:
        code, _ = self._run_checker("--bogus")
        self.assertEqual(code, 2)

    def test_missing_run_argument_rejected(self) -> None:
        code, _ = self._run_checker()
        self.assertEqual(code, 2)

    def test_safe_path_rejection(self) -> None:
        outside = Path(tempfile.mkdtemp()) / "not-a-run"
        code, out = self._run_checker(str(outside))
        self.assertNotEqual(code, 0)
        self.assertIn("a2a-runs", out)


if __name__ == "__main__":
    unittest.main(verbosity=2)