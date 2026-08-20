#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "real_kotlin_preflight.py"
SPEC = importlib.util.spec_from_file_location("real_kotlin_preflight", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
PREFLIGHT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREFLIGHT)


class RunContractTest(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
