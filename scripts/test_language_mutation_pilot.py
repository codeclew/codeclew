#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest


MODULE_PATH = Path(__file__).resolve().with_name("language_mutation_pilot.py")
SPEC = importlib.util.spec_from_file_location("codeclew_language_mutation_pilot", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
qualification = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = qualification
SPEC.loader.exec_module(qualification)


class LanguageMutationPilotTest(unittest.TestCase):
    def test_cases_cover_three_independent_changes_per_language(self) -> None:
        self.assertEqual(len(qualification.CASES), 6)
        self.assertEqual(
            [case.language for case in qualification.CASES].count("rust"), 3
        )
        self.assertEqual(
            [case.language for case in qualification.CASES].count("python"), 3
        )
        self.assertEqual(len({case.case_id for case in qualification.CASES}), 6)

    def test_native_validation_is_closed_by_language(self) -> None:
        for case in qualification.RUST_CASES:
            self.assertEqual(case.validation[0], "CARGO")
            self.assertEqual(qualification.native_command(case)[0], "cargo")
        for case in qualification.PYTHON_CASES:
            self.assertEqual(case.validation[:3], ("PYTHON", "-m", "unittest"))
            self.assertEqual(
                qualification.native_command(case)[:3],
                ["python3", "-m", "unittest"],
            )

    def test_generated_rust_sources_keep_both_tests_inside_the_test_module(self) -> None:
        for case in qualification.RUST_CASES:
            self.assertEqual(case.new_text.count("#[cfg(test)]"), 1)
            self.assertEqual(case.new_text.count("#[test]"), 2)
            self.assertTrue(case.new_text.endswith("}\n"))

    def test_public_summary_requires_all_six_cases(self) -> None:
        results = [
            {
                "caseId": case.case_id,
                "durationsMs": {"total": index},
                "errorCode": None,
                "status": "PASSED",
            }
            for index, case in enumerate(qualification.CASES, 1)
        ]
        summary = qualification.public_summary(results, "RELEASE", 1)
        self.assertEqual(summary["status"], "PASSED")
        self.assertEqual(summary["languages"]["rust"]["passed"], 3)
        self.assertEqual(summary["languages"]["python"]["passed"], 3)
        results[-1]["status"] = "FAILED"
        self.assertEqual(
            qualification.public_summary(results, "RELEASE", 1)["status"],
            "FAILED",
        )


if __name__ == "__main__":
    unittest.main()
