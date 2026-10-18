#!/usr/bin/env python3
"""Check that compatibility failures retain facts and their original test log."""

from contextlib import redirect_stderr, redirect_stdout
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location(
    "compatibility_gate", Path(__file__).with_name("kotlin-compatibility-gate.py")
)
GATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GATE)


class CompatibilityArtifactTests(unittest.TestCase):
    def exercise(self, exit_code):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "row"
            result = {"normalizedFactsDigest": "sha256:example"}
            log = "compiler test output\n" + GATE.RESULT_PREFIX + json.dumps(result) + "\n"

            def run(command, **kwargs):
                facts = Path(kwargs["env"]["CODECLEW_KOTLIN_QUALIFICATION_FACTS_OUTPUT"])
                self.assertTrue(facts.is_absolute())
                facts.write_text('{"files":[]}')
                return subprocess.CompletedProcess(command, exit_code, stdout=log)

            stdout, stderr = io.StringIO(), io.StringIO()
            with (
                mock.patch("sys.argv", ["gate", "--row", "k24-exact", "--output", str(output)]),
                mock.patch.object(GATE, "prepare_fixture"),
                mock.patch.object(GATE.subprocess, "run", side_effect=run),
                redirect_stdout(stdout),
                redirect_stderr(stderr),
            ):
                self.assertEqual(GATE.main(), exit_code)

            self.assertEqual((output / "test.log").read_text(), log)
            self.assertEqual(json.loads((output / "normalized-facts.json").read_text()), {"files": []})
            if exit_code:
                self.assertIn("Kotlin compatibility row failed: k24-exact", stderr.getvalue())
                self.assertIn(log, stderr.getvalue())
            else:
                self.assertEqual(json.loads(stdout.getvalue())["row"], "k24-exact")

    def test_failed_row_retains_artifacts_and_failure(self):
        self.exercise(23)

    def test_successful_row_retains_artifacts_and_result(self):
        self.exercise(0)


if __name__ == "__main__":
    unittest.main()
