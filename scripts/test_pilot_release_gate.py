#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest


MODULE_PATH = Path(__file__).resolve().with_name("pilot_release_gate.py")
SPEC = importlib.util.spec_from_file_location("codeclew_pilot_release_gate", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
gate = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = gate
SPEC.loader.exec_module(gate)
KEY = b"k" * 32


def pilot_case(index: int) -> dict[str, object]:
    return gate.attest_case({
        "caseId": f"case-{index:02d}",
        "durationsMs": {"open": 1, "prepareToReady": 2, "publish": 1, "total": 4},
        "errorCode": None,
        "evidenceDigest": "sha256:" + f"{index:064x}",
        "idempotentRetry": True,
        "outcome": "PUBLISHED",
        "preparedWithoutManualCleanup": True,
        "privateDataLeak": False,
        "projectClass": gate.PROJECT_CLASS,
        "recoveryResolved": True,
        "runtimeMode": "RELEASE",
        "schema": "codeclew-pilot-case/1.0",
        "sourcePreservedBeforePublish": True,
        "typedOutcome": True,
        "validationPassed": True,
    }, KEY)


def case_set() -> dict[str, object]:
    return {
        "cases": [pilot_case(index) for index in range(20)],
        "schema": "codeclew-pilot-case-set/1.0",
    }


def resign(evidence: dict[str, object]) -> None:
    evidence["cases"] = [gate.attest_case(case, KEY) for case in evidence["cases"]]


class PilotReleaseGateTest(unittest.TestCase):
    def test_twenty_successful_cases_pass_without_exposing_ids(self) -> None:
        decision = gate.evaluate(case_set(), KEY)
        self.assertEqual(decision["status"], "PASS")
        self.assertEqual(decision["decision"], "SIGNED_RELEASE_ELIGIBLE")
        self.assertEqual(decision["metrics"]["cases"], 20)
        self.assertNotIn("case-00", str(decision))

    def test_threshold_and_absolute_invariants_fail_closed(self) -> None:
        evidence = case_set()
        for index in [0, 1]:
            evidence["cases"][index]["outcome"] = "FAILED"
            evidence["cases"][index]["errorCode"] = "COMPILE_FAILED"
            evidence["cases"][index]["preparedWithoutManualCleanup"] = False
            evidence["cases"][index]["validationPassed"] = False
        evidence["cases"][2]["sourcePreservedBeforePublish"] = False
        resign(evidence)
        decision = gate.evaluate(evidence, KEY)
        self.assertEqual(decision["status"], "FAIL")
        self.assertFalse(decision["criteria"]["preparedWithoutManualCleanup"])
        self.assertFalse(decision["criteria"]["sourcePreservedBeforePublish"])

    def test_non_published_case_requires_typed_error(self) -> None:
        evidence = case_set()
        evidence["cases"][0]["outcome"] = "FAILED"
        resign(evidence)
        with self.assertRaises(gate.GateInputError):
            gate.evaluate(evidence, KEY)
        evidence["cases"][0]["errorCode"] = "COMPILE_FAILED"
        evidence["cases"][0]["preparedWithoutManualCleanup"] = False
        evidence["cases"][0]["validationPassed"] = False
        resign(evidence)
        decision = gate.evaluate(evidence, KEY)
        self.assertEqual(decision["status"], "PASS")

    def test_private_case_set_must_be_external_physical_and_0600(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / "cases.json"
            path.write_bytes(gate.canonical(case_set()))
            path.chmod(0o600)
            self.assertEqual(
                gate.private_case_set(path)["schema"],
                "codeclew-pilot-case-set/1.0",
            )
            path.chmod(0o644)
            with self.assertRaises(gate.GateInputError):
                gate.private_case_set(path)

    def test_duplicate_or_wrong_case_count_is_invalid(self) -> None:
        evidence = case_set()
        evidence["cases"].pop()
        with self.assertRaises(gate.GateInputError):
            gate.evaluate(evidence, KEY)
        evidence = case_set()
        evidence["cases"][1]["caseId"] = evidence["cases"][0]["caseId"]
        resign(evidence)
        with self.assertRaises(gate.GateInputError):
            gate.evaluate(evidence, KEY)

    def test_development_case_cannot_authorize_signed_release(self) -> None:
        evidence = case_set()
        evidence["cases"][0]["runtimeMode"] = "DEVELOPMENT"
        resign(evidence)
        decision = gate.evaluate(evidence, KEY)
        self.assertEqual(decision["status"], "FAIL")
        self.assertFalse(decision["criteria"]["releaseRuntime"])

    def test_receipt_is_new_private_and_digest_bound(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / "decision.json"
            digest = gate.write_private_decision(path, gate.evaluate(case_set(), KEY))
            self.assertTrue(digest.startswith("sha256:"))
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(gate.GateInputError):
                gate.write_private_decision(path, gate.evaluate(case_set(), KEY))

    def test_zero_publications_cannot_authorize_release(self) -> None:
        evidence = case_set()
        for case in evidence["cases"]:
            case["outcome"] = "VALIDATED_CONDITIONAL"
            case["errorCode"] = "INCOMPLETE_SEMANTIC_ANALYSIS"
        resign(evidence)
        decision = gate.evaluate(evidence, KEY)
        self.assertEqual(decision["status"], "FAIL")
        self.assertFalse(decision["criteria"]["publishedCases"])

    def test_shipped_style_assertion_without_attestation_is_invalid(self) -> None:
        evidence = case_set()
        del evidence["cases"][0]["attestation"]
        with self.assertRaises(gate.GateInputError):
            gate.evaluate(evidence, KEY)

    def test_replayed_evidence_digest_is_invalid(self) -> None:
        evidence = case_set()
        repeated = evidence["cases"][0]["evidenceDigest"]
        for case in evidence["cases"]:
            case["evidenceDigest"] = repeated
        resign(evidence)
        with self.assertRaises(gate.GateInputError):
            gate.evaluate(evidence, KEY)


if __name__ == "__main__":
    unittest.main()
