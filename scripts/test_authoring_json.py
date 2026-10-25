#!/usr/bin/env python3
"""Run the packaged syntax check against independently synthetic artifacts."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
HELPER = ROOT / "skills/codeclew/scripts/check_json.py"


class AuthoringJSONTest(unittest.TestCase):
    def run_check(self, raw, *args, copied=False):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            helper = HELPER
            if copied:
                helper = directory / "check_json.py"
                shutil.copyfile(HELPER, helper)
            path = directory / "answer.json"
            if raw is not None:
                path.write_bytes(raw)
            result = subprocess.run([sys.executable, "-I", "-S", str(helper), str(path), *args],
                                    capture_output=True, text=True, cwd=directory)
            if raw is not None:
                self.assertEqual(path.read_bytes(), raw)
            return result.returncode, json.loads(result.stdout)

    def test_nested_valid_artifact_is_unchanged_and_does_not_gain_native_or_meaning_authority(self):
        raw = b'{"fields":[{"name":"correlationId","type":"String"},{"name":"resultId","type":"Long"}],"steps":{"children":[],"otherwise":[]}}\n'
        code, result = self.run_check(raw)
        self.assertEqual(code, 0)
        self.assertEqual(result["status"], "VALID_JSON")
        self.assertEqual(result["bytes"], len(raw))
        self.assertEqual(result["nativeValidation"], "NOT_PERFORMED")
        self.assertEqual(result["meaningValidation"], "NOT_PERFORMED")

    def test_duplicates_at_any_depth_are_errors_instead_of_last_value_wins(self):
        for raw in [b'{"evidence":["s1"],"evidence":["s2"]}',
                    b'{"rows":[{"meaning":{"evidence":[],"evidence":["s1"]}}]}']:
            with self.subTest(raw=raw):
                code, result = self.run_check(raw)
                self.assertEqual(code, 2)
                self.assertEqual(result, {"status":"INVALID_JSON","reason":"DUPLICATE_KEY"})

    def test_non_json_numbers_trailing_data_malformed_utf8_and_missing_input_fail(self):
        for raw in [b'{"n":NaN}', b'{"n":Infinity}', b'{"n":-Infinity}',
                    b'{} {}', b'{"x":', b'\xff', None]:
            with self.subTest(raw=raw):
                code, result = self.run_check(raw)
                self.assertEqual(code, 2)
                self.assertEqual(result["status"], "INVALID_JSON")

    def test_repeated_names_in_distinct_objects_and_literal_text_are_valid(self):
        code, result = self.run_check(b'{"a":{"name":"same"},"b":{"name":"same"},"text":"NaN"}')
        self.assertEqual(code, 0)
        self.assertEqual(result["status"], "VALID_JSON")

    def test_diagnostic_does_not_echo_keys_values_or_paths(self):
        code, result = self.run_check(b'{"synthetic-sensitive-name":"value-a","synthetic-sensitive-name":"value-b"}')
        self.assertEqual(code, 2)
        self.assertNotIn("synthetic-sensitive-name", json.dumps(result))
        self.assertNotIn("value-a", json.dumps(result))

    def test_helper_budget_is_distinct_from_native_schema_validation(self):
        code, result = self.run_check(b'{"text":"synthetic"}', "--max-bytes", "4")
        self.assertEqual(code, 2)
        self.assertEqual(result["reason"], "HELPER_BYTE_LIMIT")

    def test_copied_helper_runs_without_repository_imports_or_dependencies(self):
        code, result = self.run_check(b'{"synthetic":true}', copied=True)
        self.assertEqual(code, 0)
        self.assertEqual(result["status"], "VALID_JSON")


if __name__ == "__main__":
    unittest.main()
