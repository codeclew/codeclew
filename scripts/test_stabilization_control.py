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
        first = control.run_check(self.plan, model, self.authority, "S3", "s3-trusted-seed")
        self.assertEqual(first["status"], "FAIL")
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


if __name__ == "__main__":
    unittest.main()
