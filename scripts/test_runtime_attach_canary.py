#!/usr/bin/env python3

from __future__ import annotations

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import runtime_attach_canary as canary


class RuntimeAttachCanaryTest(unittest.TestCase):
    def setUp(self) -> None:
        self.expected = {
            "runtimeKey": "sha256:" + "a" * 64,
            "runtimeManifestDigest": "sha256:" + "b" * 64,
            "version": "v1.2.3",
        }

    def test_matching_selector_is_retained_without_private_paths(self) -> None:
        value = {
            "productVersion": "1.2.3",
            "runtimeKey": self.expected["runtimeKey"],
            "runtimeManifestDigest": self.expected["runtimeManifestDigest"],
            "runtimeMode": "RELEASE",
        }
        result = canary.selector_result("installed-locator", value, 12, self.expected)
        self.assertEqual(result["status"], "ATTACHED")
        self.assertNotIn("path", canary.canonical(result).decode().lower())

    def test_mismatched_selector_fails_closed(self) -> None:
        value = {
            "productVersion": "1.2.3",
            "runtimeKey": "sha256:" + "c" * 64,
            "runtimeManifestDigest": self.expected["runtimeManifestDigest"],
            "runtimeMode": "RELEASE",
        }
        with self.assertRaisesRegex(canary.CanaryError, "mismatched runtime"):
            canary.selector_result("installed-locator", value, 1, self.expected)

    def test_stale_contract_is_a_typed_negative_control(self) -> None:
        result = canary.stale_result(
            {"schema": "codeclew-capabilities/1.0", "runtimeMode": "RELEASE"},
            self.expected,
        )
        self.assertEqual(result["status"], "EXPECTED_MISMATCH")
        self.assertEqual(result["code"], "RUNTIME_ATTACH_MISMATCH")


if __name__ == "__main__":
    unittest.main()
