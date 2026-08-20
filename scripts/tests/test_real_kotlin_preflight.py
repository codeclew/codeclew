#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "real_kotlin_preflight.py"
SPEC = importlib.util.spec_from_file_location("real_kotlin_preflight", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
PREFLIGHT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREFLIGHT)


class RunContractTest(unittest.TestCase):
    def test_command_failure_retains_bounded_stdout_and_stderr(self) -> None:
        completed = subprocess.CompletedProcess(
            ["probe"], 1, stdout=b'{"error":"semantic detail"}\n', stderr=b"progress\n"
        )
        with self.assertRaises(PREFLIGHT.PreflightFailure) as raised:
            PREFLIGHT.require_success(completed, "SEMANTIC_BUILD_DISCOVERY")
        self.assertEqual(raised.exception.stage, "SEMANTIC_BUILD_DISCOVERY")
        self.assertIn("progress", str(raised.exception))
        self.assertIn("semantic detail", str(raised.exception))

    def test_cold_is_cache_optional(self) -> None:
        PREFLIGHT.validate_run_contract("cold", None, None)
        PREFLIGHT.validate_run_contract("cold", "callable:example/Foo#bar", None)

    def test_warm_requires_seed_and_state_root(self) -> None:
        with self.assertRaisesRegex(PREFLIGHT.PreflightFailure, "--seed-entity"):
            PREFLIGHT.validate_run_contract("warm", None, Path("/private/tmp/state"))
        with self.assertRaisesRegex(PREFLIGHT.PreflightFailure, "--state-root"):
            PREFLIGHT.validate_run_contract("warm", "callable:example/Foo#bar", None)

    def test_blank_seed_is_rejected_in_every_phase(self) -> None:
        for phase in ("cold", "warm"):
            with self.subTest(phase=phase):
                with self.assertRaisesRegex(PREFLIGHT.PreflightFailure, "nonblank"):
                    PREFLIGHT.validate_run_contract(phase, "  ", Path("/private/tmp/state"))

    def test_warm_never_creates_an_absent_state_root(self) -> None:
        with tempfile.TemporaryDirectory(prefix="real-kotlin-preflight-test-") as raw:
            base = Path(raw).resolve()
            missing = base / "missing"
            with self.assertRaisesRegex(PREFLIGHT.PreflightFailure, "already exist"):
                PREFLIGHT.verify_private_state_root(
                    missing, base / "repository", require_existing=True
                )
            self.assertFalse(missing.exists())

    def test_exact_warm_contract_is_accepted(self) -> None:
        PREFLIGHT.validate_run_contract(
            "warm", "callable:example/Foo#bar", Path("/private/tmp/state")
        )

    def test_gradle_cache_marker_is_bound_to_exact_seed_and_members(self) -> None:
        with tempfile.TemporaryDirectory(prefix="real-kotlin-preflight-test-") as raw:
            base = Path(raw).resolve()
            source = base / "seed"
            target = base / "target"
            source.mkdir()
            (target / "caches").mkdir(parents=True)
            (target / "wrapper").mkdir()
            marker = {
                "schema": PREFLIGHT.GRADLE_CACHE_MARKER_SCHEMA,
                "source": str(source),
                "members": ["caches", "wrapper"],
            }
            (target / PREFLIGHT.GRADLE_CACHE_MARKER).write_bytes(PREFLIGHT.canonical(marker))
            self.assertTrue(
                PREFLIGHT.gradle_cache_marker_matches(
                    target, source, ["caches", "wrapper"]
                )
            )
            self.assertFalse(
                PREFLIGHT.gradle_cache_marker_matches(
                    target, base / "other-seed", ["caches", "wrapper"]
                )
            )

    def test_isolated_gradle_environment_replaces_ambient_state_authorities(self) -> None:
        with mock.patch.dict(
            PREFLIGHT.os.environ,
            {
                "GRADLE_USER_HOME": "/forged/gradle",
                "CODECLEW_K1_BUILD_STATE_ROOT": "/forged/k1",
                "CODECLEW_K2_INDEX_ROOT": "/forged/k2",
                "CODECLEW_TEST_UNRELATED": "preserved",
            },
            clear=True,
        ):
            environment = PREFLIGHT.isolated_gradle_environment(Path("/private/tmp/gradle"))
        self.assertEqual(environment["GRADLE_USER_HOME"], "/private/tmp/gradle")
        self.assertNotIn("CODECLEW_K1_BUILD_STATE_ROOT", environment)
        self.assertNotIn("CODECLEW_K2_INDEX_ROOT", environment)
        self.assertEqual(environment["CODECLEW_TEST_UNRELATED"], "preserved")


if __name__ == "__main__":
    unittest.main()
