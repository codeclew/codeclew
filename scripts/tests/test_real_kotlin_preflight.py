#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
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
    @staticmethod
    def semantic_index_with_descriptors(*descriptors: dict[str, object]) -> bytes:
        return PREFLIGHT.canonical(
            {
                "schema": "semantic-index-result/0.1",
                "declarationDescriptors": {"descriptors": list(descriptors)},
            }
        )

    def test_semantic_index_summary_uses_last_complete_json_document(self) -> None:
        result = {
            "schema": "semantic-index-result/0.1",
            "declarationDescriptorHash": "sha256:" + "a" * 64,
            "declarationRelationHash": "sha256:" + "b" * 64,
            "persistentIndexHash": "sha256:" + "c" * 64,
            "workerIndexHash": "sha256:" + "d" * 64,
            "compilerIndex": {
                "status": "FAILED_RECOVERABLE",
                "valid": False,
                "fallbackUsed": True,
            },
            "projectModelCache": {"status": "EXTRACTED_NOT_PUBLISHED"},
        }
        raw = PREFLIGHT.canonical({"event": "request_completed"}) + json.dumps(
            result, indent=2
        ).encode()
        self.assertEqual(
            PREFLIGHT.semantic_index_summary(raw),
            {
                "compilerIndexStatus": "FAILED_RECOVERABLE",
                "compilerIndexValid": False,
                "fallbackUsed": True,
                "projectModelCacheStatus": "EXTRACTED_NOT_PUBLISHED",
            },
        )

    def test_semantic_index_summary_accepts_explicitly_unavailable_profile(self) -> None:
        result = {
            "schema": "semantic-index-result/0.1",
            "declarationDescriptorHash": "sha256:" + "a" * 64,
            "declarationRelationHash": "sha256:" + "b" * 64,
            "persistentIndexHash": "sha256:" + "c" * 64,
            "workerIndexHash": "sha256:" + "d" * 64,
            "projectModelCache": {"status": "PERSISTENT_HIT"},
        }
        self.assertEqual(
            PREFLIGHT.semantic_index_summary(PREFLIGHT.canonical(result)),
            {
                "compilerIndexStatus": "UNAVAILABLE_FOR_TOOLCHAIN",
                "compilerIndexValid": None,
                "fallbackUsed": None,
                "projectModelCacheStatus": "PERSISTENT_HIT",
            },
        )

    def test_persistent_backend_capability_is_version_explicit(self) -> None:
        self.assertTrue(PREFLIGHT.supports_persistent_compiler_index("2.1.21"))
        self.assertFalse(PREFLIGHT.supports_persistent_compiler_index("2.3.0"))
        self.assertEqual(
            PREFLIGHT.project_compiler_version(
                PREFLIGHT.canonical(
                    {"schema": "semantic-project/0.1", "compilerVersion": "2.3.0"}
                )
            ),
            "2.3.0",
        )

    def test_cached_warm_seed_requires_exact_callable_identity(self) -> None:
        canonical = "callable:example/Foo.call#jvm:call()V"
        self.assertEqual(PREFLIGHT.canonical_cached_seed(canonical), canonical)
        self.assertEqual(
            PREFLIGHT.canonical_cached_seed("property:example/Foo.value"),
            "property:example/Foo.value",
        )
        with self.assertRaisesRegex(PREFLIGHT.PreflightFailure, "cold receipt"):
            PREFLIGHT.canonical_cached_seed("callable:example/Foo.call")

    def test_last_json_object_is_linear_over_large_pretty_payload(self) -> None:
        event = PREFLIGHT.canonical({"event": "request_completed"})
        payload = {
            "schema": "semantic-index-result/0.1",
            "declarationDescriptors": {
                "descriptors": [
                    {"symbolIdentity": f"callable:example/Foo.call{index}"}
                    for index in range(5_000)
                ]
            },
        }
        raw = event + json.dumps(payload, indent=2).encode()
        self.assertEqual(PREFLIGHT.last_json_object(raw), payload)

    def test_persistent_profile_rejects_fallback_and_nonpublished_model(self) -> None:
        failed = {
            "compilerIndexStatus": "FAILED_RECOVERABLE",
            "compilerIndexValid": False,
            "fallbackUsed": True,
            "projectModelCacheStatus": "EXTRACTED_NOT_PUBLISHED",
        }
        with self.assertRaisesRegex(PREFLIGHT.PreflightFailure, "legacy fallback"):
            PREFLIGHT.require_persistent_reuse(failed, "cold")
        unpublished = {
            "compilerIndexStatus": "COLD_FULL",
            "compilerIndexValid": True,
            "fallbackUsed": False,
            "projectModelCacheStatus": "EXTRACTED_NOT_PUBLISHED",
        }
        with self.assertRaisesRegex(PREFLIGHT.PreflightFailure, "not published"):
            PREFLIGHT.require_persistent_reuse(unpublished, "cold")

    def test_warm_profile_requires_both_persistent_hits(self) -> None:
        valid = {
            "compilerIndexStatus": "UNCHANGED_HIT",
            "compilerIndexValid": True,
            "fallbackUsed": False,
            "projectModelCacheStatus": "PERSISTENT_HIT",
        }
        PREFLIGHT.require_persistent_reuse(valid, "warm")
        for field, replacement in (
            ("compilerIndexStatus", "COLD_FULL"),
            ("projectModelCacheStatus", "EXTRACTED_PUBLISHED"),
        ):
            with self.subTest(field=field):
                changed = {**valid, field: replacement}
                with self.assertRaises(PREFLIGHT.PreflightFailure):
                    PREFLIGHT.require_persistent_reuse(changed, "warm")

    def test_semantic_index_summary_rejects_unsealed_result(self) -> None:
        with self.assertRaises(PREFLIGHT.PreflightFailure):
            PREFLIGHT.semantic_index_summary(b'{"schema":"semantic-index-result/0.1"}\n')

    def test_seed_is_resolved_to_one_proven_canonical_entity(self) -> None:
        canonical = (
            "callable:example/Foo.decode#jvm:decode(Ljava/lang/String;)Ljava/lang/Object;"
        )
        raw = self.semantic_index_with_descriptors(
            {
                "resolution": "PROVEN",
                "symbolIdentity": canonical,
                "compilerCallableId": "example/Foo.decode",
            },
            {
                "resolution": "UNKNOWN",
                "symbolIdentity": "callable:example/Foo.decode",
                "compilerCallableId": "example/Foo.decode",
            },
        )
        self.assertEqual(
            PREFLIGHT.canonical_seed_entity(raw, "callable:example/Foo.decode"),
            canonical,
        )
        self.assertEqual(PREFLIGHT.canonical_seed_entity(raw, canonical), canonical)
        self.assertIsNone(PREFLIGHT.canonical_seed_entity(raw, None))

    def test_missing_or_ambiguous_seed_fails_before_agent_launch(self) -> None:
        missing = self.semantic_index_with_descriptors()
        with self.assertRaisesRegex(PREFLIGHT.PreflightFailure, "exactly one"):
            PREFLIGHT.canonical_seed_entity(missing, "callable:example/Missing.call")
        ambiguous = self.semantic_index_with_descriptors(
            {
                "resolution": "PROVEN",
                "symbolIdentity": "callable:example/Foo.call#jvm:call(I)V",
                "compilerCallableId": "example/Foo.call",
            },
            {
                "resolution": "PROVEN",
                "symbolIdentity": "callable:example/Foo.call#jvm:call(Ljava/lang/String;)V",
                "compilerCallableId": "example/Foo.call",
            },
        )
        with self.assertRaisesRegex(PREFLIGHT.PreflightFailure, "exactly one"):
            PREFLIGHT.canonical_seed_entity(ambiguous, "callable:example/Foo.call")

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

    def test_cold_is_unbounded_by_default_and_explicit_budget_is_honored(self) -> None:
        self.assertIsNone(PREFLIGHT.effective_budget("cold", None))
        self.assertEqual(PREFLIGHT.effective_budget("cold", 180.0), 180.0)

    def test_warm_is_always_capped_at_one_minute(self) -> None:
        self.assertEqual(PREFLIGHT.effective_budget("warm", None), 60.0)
        self.assertEqual(PREFLIGHT.effective_budget("warm", 180.0), 60.0)
        self.assertEqual(PREFLIGHT.effective_budget("warm", 12.0), 12.0)
        with self.assertRaisesRegex(PREFLIGHT.PreflightFailure, "positive"):
            PREFLIGHT.effective_budget("cold", 0.0)

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

    def test_gradle_daemon_reset_is_bound_to_repo_local_cache(self) -> None:
        self.assertEqual(
            PREFLIGHT.gradle_daemon_stop_argv(Path("/repo/gradlew"), Path("/repo/.gradle")),
            ["/repo/gradlew", "--gradle-user-home", "/repo/.gradle", "--stop"],
        )

    def test_java_launch_authority_is_bound_directly_to_java_home(self) -> None:
        with tempfile.TemporaryDirectory(prefix="real-kotlin-java-authority-") as raw:
            root = Path(raw).resolve()
            home = root / "jdk"
            java = home / "bin" / "java"
            java.parent.mkdir(parents=True)
            java.write_text("#!/bin/sh\n", encoding="utf-8")
            java.chmod(0o700)
            with mock.patch.dict(PREFLIGHT.os.environ, {"JAVA_HOME": str(home)}, clear=True):
                self.assertEqual(PREFLIGHT.java_launch_authority(), (home, java))
            with mock.patch.dict(PREFLIGHT.os.environ, {}, clear=True):
                with self.assertRaisesRegex(PREFLIGHT.PreflightFailure, "must be explicit"):
                    PREFLIGHT.java_launch_authority()


if __name__ == "__main__":
    unittest.main()
