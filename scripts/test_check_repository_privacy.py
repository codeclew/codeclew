#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest


MODULE_PATH = Path(__file__).resolve().with_name("check_repository_privacy.py")
SPEC = importlib.util.spec_from_file_location("codeclew_repository_privacy", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
privacy = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = privacy
SPEC.loader.exec_module(privacy)


class RepositoryPrivacyTest(unittest.TestCase):
    def test_pilot_results_are_forbidden_even_when_force_added(self) -> None:
        self.assertEqual(
            privacy.path_rules("docs/pilot/results/case-001.json"),
            ["private-generated-path"],
        )
        self.assertEqual(privacy.path_rules("docs/pilot/case-template.json"), [])

    def test_pilot_case_schema_is_forbidden_outside_exact_template(self) -> None:
        template = (MODULE_PATH.parent.parent / privacy.PILOT_CASE_TEMPLATE).read_bytes()
        self.assertNotIn(
            "filled-pilot-case",
            privacy.blob_rules(template, privacy.PILOT_CASE_TEMPLATE),
        )
        self.assertIn(
            "filled-pilot-case",
            privacy.blob_rules(template, "private-case.json"),
        )
        pretty = template.replace(b'"schema":"', b'"schema": "')
        self.assertIn(
            "filled-pilot-case",
            privacy.blob_rules(pretty, "pretty-private-case.json"),
        )
        changed = template.replace(
            b'"outcome":"RECORDER_OUTPUT_REQUIRED"', b'"outcome":"FAILED"'
        )
        self.assertIn(
            "filled-pilot-case",
            privacy.blob_rules(changed, privacy.PILOT_CASE_TEMPLATE),
        )
        for schema in [
            "codeclew-pilot-attestation-key/1.0",
            "codeclew-pilot-case-set/1.0",
            "codeclew-pilot-release-decision/1.0",
            "codeclew-pilot-source-snapshot/1.0",
        ]:
            evidence = (f'{{"schema":"{schema}"}}\n').encode()
            self.assertIn(
                "filled-pilot-case",
                privacy.blob_rules(evidence, "arbitrary.json"),
            )


if __name__ == "__main__":
    unittest.main()
