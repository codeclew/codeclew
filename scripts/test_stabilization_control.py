#!/usr/bin/env python3
"""Fast contract tests for stabilization plan enforcement."""

from __future__ import annotations

import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import stabilization_control as control  # noqa: E402


class StabilizationControlTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.previous_home = os.environ.get("CODECLEW_CONTROL_HOME")
        os.environ["CODECLEW_CONTROL_HOME"] = self.temporary.name
        os.chmod(self.temporary.name, 0o700)
        self.plan = control.load_json(control.PLAN_PATH)
        self.model = control.validate_plan(self.plan)
        self.authority = control.authorities(self.plan)

    def tearDown(self) -> None:
        if self.previous_home is None:
            os.environ.pop("CODECLEW_CONTROL_HOME", None)
        else:
            os.environ["CODECLEW_CONTROL_HOME"] = self.previous_home
        self.temporary.cleanup()

    def test_current_plan_is_closed_and_acyclic(self) -> None:
        self.assertEqual(self.model["order"][0], "S0")
        self.assertEqual(self.model["order"][-1], "R5")
        self.assertEqual(set(self.model["tiers"]), {f"L{index}" for index in range(8)})

    def test_dependency_cycle_is_rejected(self) -> None:
        value = copy.deepcopy(self.plan)
        value["steps"][0]["dependencies"] = ["R5"]
        with self.assertRaisesRegex(control.ControlError, "cycle"):
            control.validate_plan(value)

    def test_direct_expensive_gate_is_refused(self) -> None:
        environment = dict(os.environ)
        environment.pop("CODECLEW_PLAN_CAPABILITY", None)
        completed = subprocess.run(
            (
                sys.executable,
                "-I",
                "-S",
                str(ROOT / "scripts" / "stabilization_control.py"),
                "guard",
                "--gate",
                "cold-runtime",
            ),
            cwd=ROOT,
            env=environment,
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        self.assertEqual(completed.returncode, 2)
        self.assertIn(b"direct expensive gate execution is forbidden", completed.stdout)

    def test_capability_is_signed_and_single_use(self) -> None:
        capability = control.issue_capability(self.authority, "cold-runtime", 60)
        environment = dict(os.environ)
        environment["CODECLEW_PLAN_CAPABILITY"] = str(capability)
        command = (
            sys.executable,
            "-I",
            "-S",
            str(ROOT / "scripts" / "stabilization_control.py"),
            "guard",
            "--gate",
            "cold-runtime",
        )
        first = subprocess.run(command, cwd=ROOT, env=environment, check=False, stdout=subprocess.PIPE)
        second = subprocess.run(command, cwd=ROOT, env=environment, check=False, stdout=subprocess.PIPE)
        self.assertEqual(first.returncode, 0)
        self.assertEqual(second.returncode, 2)

    def test_failed_evidence_key_cannot_be_retried_blindly(self) -> None:
        model = copy.deepcopy(self.model)
        model["steps"]["S3"]["dependencies"] = []
        check = model["checks"]["s3-trusted-seed"]
        evidence = control.evidence_digest(check, self.authority)
        path = control.receipt_path(self.authority, "s3-trusted-seed", evidence)
        control.atomic_private_write(path, control.canonical({"status": "FAIL"}) + b"\n")
        with self.assertRaisesRegex(control.ControlError, "blind retry refused"):
            control.run_check(self.plan, model, self.authority, "S3", "s3-trusted-seed")

    def test_controller_validate_is_machine_readable(self) -> None:
        completed = subprocess.run(
            (sys.executable, "-I", "-S", str(ROOT / "scripts" / "stabilization_control.py"), "validate"),
            cwd=ROOT,
            env=dict(os.environ),
            check=True,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            text=True,
        )
        value = json.loads(completed.stdout)
        self.assertEqual(value["status"], "PASS")
        self.assertEqual(value["schema"], "codeclew-stabilization-plan-validation/1.0")

    def test_timeout_reaps_the_owned_process_group(self) -> None:
        check = {
            "command": [sys.executable, "-I", "-S", "-c", "import time; time.sleep(30)"],
            "gate": None,
        }
        tier = {"budgetSeconds": 1}
        exit_code, duration, _stdout, _stderr = control.invoke(check, tier, self.authority)
        self.assertEqual(exit_code, 124)
        self.assertLess(duration, 7000)

    def test_completion_fails_closed_when_required_check_authority_changes(self) -> None:
        step = "S0"
        authorities = {
            check_id: control.check_authority_digest(
                self.model["checks"][check_id], self.authority
            )
            for check_id in self.model["steps"][step]["requiredChecks"]
        }
        completion = {
            **self.authority,
            "checkAuthorities": authorities,
            "receiptDigests": [],
            "schema": "codeclew-stabilization-step-completion/1.0",
            "sourceRevision": control.git("rev-parse", "HEAD"),
            "status": "COMPLETE",
            "stepId": step,
        }
        completion["completionDigest"] = control.digest_bytes(
            control.canonical(completion)
        )
        control.atomic_private_write(
            control.completion_path(self.authority, step),
            control.canonical(completion) + b"\n",
        )
        self.assertTrue(control.valid_completion(self.model, self.authority, step))

        changed = "sha256:" + "f" * 64
        with mock.patch.object(
            control, "check_authority_digest", return_value=changed
        ):
            self.assertFalse(
                control.valid_completion(self.model, self.authority, step)
            )

    def test_completion_reuse_does_not_require_transient_check_environment(self) -> None:
        check = self.model["checks"]["s0-recovery-baseline"]
        with mock.patch.dict(
            os.environ, {"CODECLEW_RECOVERY_MANIFEST": "/private/first.json"}
        ):
            first = control.check_authority_digest(check, self.authority)
            first_evidence = control.evidence_digest(check, self.authority)
        with mock.patch.dict(
            os.environ, {"CODECLEW_RECOVERY_MANIFEST": "/private/second.json"}
        ):
            second = control.check_authority_digest(check, self.authority)
            second_evidence = control.evidence_digest(check, self.authority)
        self.assertEqual(first, second)
        self.assertNotEqual(first_evidence, second_evidence)

    def test_dynamic_environment_file_changes_evidence_key(self) -> None:
        check = copy.deepcopy(self.model["checks"]["s0-recovery-baseline"])
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "recovery.json"
            manifest.write_text("first", encoding="utf-8")
            with mock.patch.dict(
                os.environ, {"CODECLEW_RECOVERY_MANIFEST": str(manifest)}
            ):
                first = control.dynamic_authority_digest(check)
                manifest.write_text("second", encoding="utf-8")
                second = control.dynamic_authority_digest(check)
        self.assertNotEqual(first, second)

    def test_completed_steps_never_crosses_an_invalid_dependency(self) -> None:
        valid = {"S0", "S1", "S3", "S4"}
        with mock.patch.object(
            control,
            "valid_completion",
            side_effect=lambda _model, _authority, step: step in valid,
        ):
            self.assertEqual(
                control.completed_steps(self.model, self.authority),
                ["S0", "S1"],
            )


if __name__ == "__main__":
    unittest.main()
