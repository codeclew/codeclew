#!/usr/bin/env python3
from __future__ import annotations

import json
import fcntl
import os
from pathlib import Path
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import cold_multicore_gate as gate

TEST_LOGICAL_CORES = 8
TEST_MEMORY_BYTES = 16 * 1024**3


def sha(character: str) -> str:
    return "sha256:" + character * 64


def build_authority() -> dict[str, object]:
    return {
        "artifactIds": ["clew"],
        "componentIds": ["clew", "kotlin21", "kotlin23", "kotlin24"],
        "inputDigest": sha("1"),
        "mode": "RELEASE",
        "runtimeKey": sha("2"),
        "schema": "codeclew-dependency-cache-authority/1.0",
        "status": "PASS",
        "toolchainDigest": sha("3"),
        "workerIds": ["kotlin21", "kotlin23", "kotlin24"],
    }


def warm_audit() -> dict[str, object]:
    return {
        "capsuleBuildInvoked": False,
        "coldToolchainInvoked": False,
        "counters": {
            "checkpointHits": 1,
            "checkpointMisses": 0,
            "digestFileCalls": 0,
            "metadataChecks": 12,
            "processRuns": 0,
        },
        "forbiddenWarmProcesses": ["cargo", "rustc", "gradle", "maven"],
        "schema": "codeclew-bootstrap-warm-audit/2.0",
        "status": "PASSED",
    }


def build_plan(profile: str, critical: int) -> tuple[dict[str, object], dict[str, int]]:
    resources = gate.expected_build_resources(
        TEST_LOGICAL_CORES, TEST_MEMORY_BYTES, profile
    )
    if profile == "SERIAL":
        gradle = max(1, critical * 3 // 5)
        cargo = critical - gradle
        recomputed = gradle + cargo
    else:
        gradle = critical
        cargo = max(1, critical * 4 // 5)
        recomputed = max(gradle, cargo)
    stage_wall = {
        "CAPSULE_ASSEMBLY": 3,
        "CAPSULE_SEAL_AND_VERIFY": 4,
        "CARGO_BINARIES": cargo,
        "COMPONENT_PUBLICATION": 5,
        "GRADLE_WORKERS": gradle,
        "INPUT_STAGING": 6,
    }
    return (
        {
            **resources,
            "parallel": profile == "PARALLEL",
            "profile": profile,
            "stageWallMillis": {
                "CARGO_BINARIES": cargo,
                "GRADLE_WORKERS": gradle,
            },
            "toolchainCriticalPathMillis": recomputed,
            "toolchainStages": list(gate.TOOLCHAIN_STAGES),
            "toolchainWallMillis": recomputed + 2,
        },
        stage_wall,
    )


def load_authority(boot: str, offset: int) -> dict[str, object]:
    return {
        "after": {
            "capturedMonotonicNanos": 1_000_000 + offset + 1,
            "loadAverage": [1.0, 1.5, 2.0],
        },
        "before": {
            "capturedMonotonicNanos": 1_000_000 + offset,
            "loadAverage": [1.0, 1.5, 2.0],
        },
        "bootAuthorityDigest": boot,
        "physicalCores": 8,
        "schema": gate.LOAD_SCHEMA,
    }


def arms_from_measurements(
    measurements: list[tuple[int, int]],
    *,
    cohort: str = sha("4"),
    cohort_authority: dict[str, object] | None = None,
) -> list[dict[str, object]]:
    if cohort_authority is None:
        common = {
            "artifactIds": ["clew"],
            "bootAuthorityDigest": sha("5"),
            "buildAuthorityDigest": sha("6"),
            "cacheSeedDigest": sha("7"),
            "cohortId": cohort,
            "componentIds": ["clew", "kotlin21", "kotlin23", "kotlin24"],
            "hostAuthorityDigest": sha("8"),
            "logicalCores": TEST_LOGICAL_CORES,
            "physicalCores": 8,
            "qualificationCores": 8,
            "sourceRevision": "a" * 40,
            "totalMemoryBytes": TEST_MEMORY_BYTES,
            "workerIds": ["kotlin21", "kotlin23", "kotlin24"],
        }
    else:
        common = {
            "artifactIds": cohort_authority["buildAuthority"]["artifactIds"],
            "bootAuthorityDigest": cohort_authority["bootAuthorityDigest"],
            "buildAuthorityDigest": cohort_authority["buildAuthorityDigest"],
            "cacheSeedDigest": cohort_authority["cacheSeedDigest"],
            "cohortId": cohort_authority["cohortId"],
            "componentIds": cohort_authority["buildAuthority"]["componentIds"],
            "hostAuthorityDigest": cohort_authority["hostAuthorityDigest"],
            "logicalCores": cohort_authority["hostAuthority"]["logicalCores"],
            "physicalCores": cohort_authority["hostAuthority"]["physicalCores"],
            "qualificationCores": cohort_authority["hostAuthority"]["qualificationCores"],
            "sourceRevision": cohort_authority["sourceRevision"],
            "totalMemoryBytes": cohort_authority["hostAuthority"]["totalMemoryBytes"],
            "workerIds": cohort_authority["buildAuthority"]["workerIds"],
        }
    predecessor = None
    values = []
    for sequence, ((arm_id, block, order, profile), (critical, outer)) in enumerate(
        zip(gate.ARMS, measurements), 1
    ):
        plan, stages = build_plan(profile, critical)
        capsule = critical + sum(
            value for name, value in stages.items() if name not in gate.TOOLCHAIN_STAGES
        ) + 10
        if outer < capsule:
            outer = capsule
        value: dict[str, object] = {
            **common,
            "armDigest": "",
            "armId": arm_id,
            "artifactHashes": {"clew": sha("9")},
            "block": block,
            "buildPlan": plan,
            "capsuleWallMillis": capsule,
            "componentHits": [],
            "componentMisses": common["componentIds"],
            "criticalPathMillis": critical,
            "loadAuthority": load_authority(str(common["bootAuthorityDigest"]), sequence),
            "manifestDigest": sha("b"),
            "order": order,
            "outerWallMillis": outer,
            "predecessorArmDigest": predecessor,
            "profile": profile,
            "runtimeKey": sha("2"),
            "schema": gate.ARM_SCHEMA,
            "sequence": sequence,
            "stageWallMillis": stages,
            "status": "PASS",
            "warmAudit": warm_audit(),
            "workerTreeHashes": {
                "kotlin21": sha("a"), "kotlin23": sha("d"), "kotlin24": sha("c")
            },
        }
        value["armDigest"] = gate._arm_digest(value)
        predecessor = str(value["armDigest"])
        values.append(value)
    return values


def persist_arms(cohort: gate.CohortHandle, arms: list[dict[str, object]]) -> None:
    for sequence, ((arm_id, _block, _order, _profile), arm) in enumerate(
        zip(gate.ARMS, arms), 1
    ):
        gate.immutable_json(cohort.arms / f"{sequence:02d}-{arm_id}.json", arm)


def final_report(
    cohort: gate.CohortHandle, arms: list[dict[str, object]]
) -> dict[str, object]:
    return {
        **gate.aggregate(
            arms,
            str(cohort.authority["sourceRevision"]),
            int(cohort.authority["hostAuthority"]["qualificationCores"]),
            cohort.authority,
        ),
        "cleanupStatus": "PASSED",
    }


def cold_evidence(profile: str = "SERIAL", critical: int = 100) -> dict[str, object]:
    plan, stages = build_plan(profile, critical)
    sequential = sum(
        value for name, value in stages.items() if name not in gate.TOOLCHAIN_STAGES
    )
    return {
        "artifactHashes": {"clew": sha("9")},
        "buildPlan": plan,
        "componentHits": [],
        "componentMisses": ["clew", "kotlin21", "kotlin23", "kotlin24"],
        "manifestDigest": sha("b"),
        "mode": "RELEASE",
        "runtimeKey": sha("2"),
        "schema": "codeclew-real-cold-build-evidence/1.0",
        "stageWallMillis": stages,
        "status": "MEASURED",
        "wallMillis": critical + sequential + 10,
        "workerTreeHashes": {
            "kotlin21": sha("a"), "kotlin23": sha("d"), "kotlin24": sha("c")
        },
    }


def prime_evidence(authority: dict[str, object]) -> dict[str, object]:
    return {
        **authority,
        "stageWallMillis": {
            "CARGO_DEPENDENCIES": 20,
            "GRADLE_DEPENDENCIES": 30,
        },
        "status": "PRIMED",
        "wallMillis": 55,
    }


def host_fixture() -> dict[str, object]:
    resources = {
        "cpu": {
            "affinityCores": 8,
            "cgroupQuotaCores": None,
            "cpusetCores": None,
            "onlineCores": 8,
        },
        "logicalCores": TEST_LOGICAL_CORES,
        "memory": {
            "cgroupLimitBytes": None,
            "physicalBytes": TEST_MEMORY_BYTES,
        },
        "schema": "codeclew-effective-host-resources/1.0",
        "totalMemoryBytes": TEST_MEMORY_BYTES,
    }
    return {
        "logicalCores": TEST_LOGICAL_CORES,
        "machine": "test",
        "physicalCores": 8,
        "platform": "Test",
        "qualificationCores": 8,
        "release": "1",
        "resourceAuthority": resources,
        "schema": gate.HOST_SCHEMA,
        "totalMemoryBytes": TEST_MEMORY_BYTES,
    }


class ColdMulticoreGateTests(unittest.TestCase):
    def test_unqualified_host_cannot_satisfy_formal_zero_exit_gate(self) -> None:
        self.assertNotEqual(gate.UNQUALIFIED_EXIT_CODE, 0)

    def test_matched_blocks_pass_on_stable_critical_path_speedup(self) -> None:
        arms = arms_from_measurements([(100, 200), (50, 120), (55, 125), (100, 200)])
        report = gate.aggregate(arms, "a" * 40, 8)
        self.assertTrue(report["accepted"])
        self.assertEqual(report["status"], "PASSED")
        self.assertAlmostEqual(
            report["measurements"]["criticalPathGeometricMeanRatio"],
            (0.5 * 0.55) ** 0.5,
            places=6,
        )

    def test_large_order_interaction_is_typed_noisy(self) -> None:
        arms = arms_from_measurements([(100, 200), (30, 130), (80, 180), (100, 200)])
        report = gate.aggregate(arms, "a" * 40, 8)
        self.assertFalse(report["accepted"])
        self.assertEqual(report["status"], "FAILED_NOISY")

    def test_identity_mismatch_fails_before_performance_acceptance(self) -> None:
        arms = arms_from_measurements([(100, 200), (50, 150), (50, 150), (100, 200)])
        arms[-1]["manifestDigest"] = sha("d")
        arms[-1]["armDigest"] = gate._arm_digest(arms[-1])
        report = gate.aggregate(arms, "a" * 40, 8)
        self.assertFalse(report["accepted"])
        self.assertEqual(report["status"], "FAILED_NONDETERMINISTIC_CAPSULE")

    def test_strict_evidence_recomputes_critical_path_and_rejects_forgery(self) -> None:
        common = {
            "artifactIds": ["clew"],
            "componentIds": ["clew", "kotlin21", "kotlin23", "kotlin24"],
            "logicalCores": TEST_LOGICAL_CORES,
            "runtimeKey": sha("2"),
            "totalMemoryBytes": TEST_MEMORY_BYTES,
            "workerIds": ["kotlin21", "kotlin23", "kotlin24"],
        }
        evidence = cold_evidence()
        _value, critical = gate.validate_cold_evidence(evidence, "SERIAL", common)
        self.assertEqual(critical, 100)

        forged = json.loads(json.dumps(evidence))
        forged["buildPlan"]["toolchainCriticalPathMillis"] = 1
        with self.assertRaisesRegex(gate.GateError, "independently reproducible"):
            gate.validate_cold_evidence(forged, "SERIAL", common)

        missing = json.loads(json.dumps(evidence))
        del missing["stageWallMillis"]["CARGO_BINARIES"]
        with self.assertRaises(gate.GateError):
            gate.validate_cold_evidence(missing, "SERIAL", common)

        none_identity = json.loads(json.dumps(evidence))
        none_identity["manifestDigest"] = None
        with self.assertRaises(gate.GateError):
            gate.validate_cold_evidence(none_identity, "SERIAL", common)

    def test_evidence_rejects_worker_overclaim_memory_forgery_and_path_identifiers(self) -> None:
        common = {
            "artifactIds": ["clew"],
            "componentIds": ["clew", "kotlin21", "kotlin23", "kotlin24"],
            "logicalCores": TEST_LOGICAL_CORES,
            "runtimeKey": sha("2"),
            "totalMemoryBytes": TEST_MEMORY_BYTES,
            "workerIds": ["kotlin21", "kotlin23", "kotlin24"],
        }
        evidence = cold_evidence("PARALLEL", 100)
        evidence["buildPlan"]["cargoWorkers"] = TEST_LOGICAL_CORES
        with self.assertRaisesRegex(gate.GateError, "resource authority"):
            gate.validate_cold_evidence(evidence, "PARALLEL", common)

        evidence = cold_evidence("SERIAL", 100)
        evidence["buildPlan"]["memoryBudgetBytes"] += 1
        with self.assertRaisesRegex(gate.GateError, "resource authority"):
            gate.validate_cold_evidence(evidence, "SERIAL", common)

        evidence = cold_evidence("PARALLEL", 100)
        evidence["buildPlan"]["gradleHeapBytes"] -= 1
        with self.assertRaisesRegex(gate.GateError, "resource authority"):
            gate.validate_cold_evidence(evidence, "PARALLEL", common)

        evidence = cold_evidence("SERIAL", 100)
        evidence["artifactHashes"] = {"../../private/clew": sha("9")}
        with self.assertRaises(gate.GateError):
            gate.validate_cold_evidence(evidence, "SERIAL", common)

        evidence = cold_evidence("SERIAL", 100)
        del evidence["workerTreeHashes"]["kotlin21"]
        with self.assertRaises(gate.GateError):
            gate.validate_cold_evidence(evidence, "SERIAL", common)

        evidence = cold_evidence("SERIAL", 100)
        evidence["artifactHashes"]["extra"] = sha("e")
        with self.assertRaises(gate.GateError):
            gate.validate_cold_evidence(evidence, "SERIAL", common)

        unsafe_authority = build_authority()
        unsafe_authority["componentIds"] = ["../private-worker"]
        with self.assertRaises(gate.GateError):
            gate.validate_build_authority(unsafe_authority)
        unsafe_authority["componentIds"] = ["clew", 7]
        with self.assertRaises(gate.GateError):
            gate.validate_build_authority(unsafe_authority)

    def test_exact_warm_audit_rejects_missing_counter_and_cold_miss(self) -> None:
        self.assertEqual(gate.validate_warm_audit(warm_audit())["status"], "PASSED")
        missing = warm_audit()
        del missing["counters"]["checkpointHits"]
        with self.assertRaises(gate.GateError):
            gate.validate_warm_audit(missing)
        cold = warm_audit()
        cold["capsuleBuildInvoked"] = True
        with self.assertRaises(gate.GateError):
            gate.validate_warm_audit(cold)

    def test_prime_evidence_exactly_matches_build_authority_and_timings(self) -> None:
        authority = build_authority()
        evidence = prime_evidence(authority)
        self.assertEqual(gate.validate_prime_evidence(evidence, authority), evidence)
        wrong = json.loads(json.dumps(evidence))
        wrong["toolchainDigest"] = sha("f")
        with self.assertRaisesRegex(gate.GateError, "build authority"):
            gate.validate_prime_evidence(wrong, authority)
        missing = json.loads(json.dumps(evidence))
        del missing["stageWallMillis"]["CARGO_DEPENDENCIES"]
        with self.assertRaises(gate.GateError):
            gate.validate_prime_evidence(missing, authority)
        forged = json.loads(json.dumps(evidence))
        forged["wallMillis"] = 1
        with self.assertRaisesRegex(gate.GateError, "timing authority"):
            gate.validate_prime_evidence(forged, authority)

    def test_immutable_arm_receipt_has_digest_mode_fsync_and_no_overwrite(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / "arm.json"
            value = arms_from_measurements(
                [(100, 200), (50, 150), (50, 150), (100, 200)]
            )[0]
            expected = {
                name: value[name]
                for name in (
                    "armId", "block", "cacheSeedDigest", "cohortId", "order",
                    "predecessorArmDigest", "profile", "sequence",
                )
            }
            with mock.patch.object(gate.os, "fsync", wraps=os.fsync) as fsync:
                gate.immutable_json(path, value)
            self.assertGreaterEqual(fsync.call_count, 2)
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o400)
            self.assertEqual(gate.read_arm(path, expected), value)
            original = path.read_bytes()
            with self.assertRaisesRegex(gate.GateError, "already exists"):
                gate.immutable_json(path, {**value, "status": "FORGED"})
            self.assertEqual(path.read_bytes(), original)
            path.chmod(0o600)
            self.assertIsNone(gate.read_arm(path, expected))

    def test_receipt_read_refuses_path_replacement_during_read(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            root.chmod(0o700)
            path = root / "receipt.json"
            replacement = root / "replacement.json"
            value = {
                "armCount": 4,
                "cohortId": sha("1"),
                "finalArmDigest": sha("2"),
                "reportDigest": sha("3"),
                "schema": gate.COHORT_COMPLETE_SCHEMA,
                "status": "COMPLETE",
            }
            path.write_bytes(gate.canonical(value) + b"\n")
            replacement.write_bytes(gate.canonical(value) + b"\n")
            path.chmod(0o400)
            replacement.chmod(0o400)
            real_read = os.read
            replaced = False

            def replace_after_read(descriptor: int, length: int) -> bytes:
                nonlocal replaced
                block = real_read(descriptor, length)
                if not replaced:
                    replaced = True
                    os.replace(replacement, path)
                return block

            with mock.patch.object(gate.os, "read", side_effect=replace_after_read):
                self.assertIsNone(
                    gate._read_exact_json(path, gate.COHORT_COMPLETE_FIELDS)
                )

    def test_receipt_read_refuses_hardlink_alias(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            root.chmod(0o700)
            path = root / "receipt.json"
            alias = root / "alias.json"
            value = {
                "armCount": 4,
                "cohortId": sha("1"),
                "finalArmDigest": sha("2"),
                "reportDigest": sha("3"),
                "schema": gate.COHORT_COMPLETE_SCHEMA,
                "status": "COMPLETE",
            }
            path.write_bytes(gate.canonical(value) + b"\n")
            path.chmod(0o400)
            os.link(path, alias)
            self.assertIsNone(
                gate._read_exact_json(path, gate.COHORT_COMPLETE_FIELDS)
            )

    def test_read_and_lock_verify_do_not_create_missing_state(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            missing_parent = root / "missing" / "state"
            self.assertIsNone(
                gate._read_exact_json(
                    missing_parent / "receipt.json", gate.COHORT_COMPLETE_FIELDS
                )
            )
            self.assertFalse((root / "missing").exists())

            lock_parent = root / "lock-parent"
            lock_parent.mkdir(mode=0o700)
            lock = gate._open_private_lock(
                lock_parent / "cohort.lock", create=False, nonblocking=True
            )
            self.assertIsNotNone(lock)
            assert lock is not None
            renamed = root / "renamed"
            lock_parent.rename(renamed)
            with self.assertRaisesRegex(gate.GateError, "disappeared"):
                lock.verify()
            self.assertFalse(lock_parent.exists())
            lock.close()

    def test_private_directory_refuses_relative_and_symlinked_ancestors(self) -> None:
        with self.assertRaisesRegex(gate.GateError, "normalized"):
            gate.private_directory(Path("relative/state"))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            root.chmod(0o700)
            physical = root / "physical"
            physical.mkdir(mode=0o700)
            link = root / "link"
            link.symlink_to(physical, target_is_directory=True)
            with self.assertRaisesRegex(gate.GateError, "symlink"):
                gate.private_directory(link / "state")

    def test_whole_cohort_is_unique_and_receipts_cannot_mix(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runs = Path(directory).resolve()
            host = host_fixture()
            boot = {"identity": "boot", "platform": "Test", "schema": gate.BOOT_SCHEMA}
            first = gate.create_cohort(runs, "a" * 40, sha("7"), host, boot, build_authority())
            second = gate.create_cohort(runs, "a" * 40, sha("7"), host, boot, build_authority())
            try:
                self.assertNotEqual(first.authority["cohortId"], second.authority["cohortId"])
                self.assertNotEqual(first.path, second.path)
                self.assertEqual(stat.S_IMODE((first.path / "cohort.json").stat().st_mode), 0o400)
                arm = arms_from_measurements(
                    [(100, 200), (50, 150), (50, 150), (100, 200)],
                    cohort=str(first.authority["cohortId"]),
                )[0]
                receipt = first.arms / "01.json"
                gate.immutable_json(receipt, arm)
                self.assertIsNone(
                    gate.read_arm(receipt, {"cohortId": second.authority["cohortId"]})
                )
            finally:
                second.close()
                first.close()

    def test_lock_replacement_is_rejected_after_flock(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            root.chmod(0o700)
            path = root / "authority.lock"
            real_flock = fcntl.flock

            def replace_then_lock(descriptor: int, operation: int) -> None:
                replacement = root / "replacement.lock"
                replacement.write_bytes(b"")
                replacement.chmod(0o600)
                os.replace(replacement, path)
                real_flock(descriptor, operation)

            with (
                mock.patch.object(gate.fcntl, "flock", side_effect=replace_then_lock),
                self.assertRaisesRegex(gate.GateError, "directory lock authority changed"),
            ):
                gate._open_private_lock(path, create=True)

    def test_held_lock_pair_refuses_primary_or_companion_replacement(self) -> None:
        for replaced_name in ("authority.lock", "authority.lock.authority"):
            with self.subTest(replaced_name=replaced_name):
                with tempfile.TemporaryDirectory() as directory:
                    root = Path(directory).resolve()
                    root.chmod(0o700)
                    path = root / "authority.lock"
                    holder = gate._open_private_lock(path, create=True)
                    self.assertIsNotNone(holder)
                    target = root / replaced_name
                    target.write_bytes(b"")
                    target.chmod(0o600)
                    try:
                        with self.assertRaises(gate.GateError):
                            holder.verify()
                        with self.assertRaises(gate.GateError):
                            gate._open_private_lock(
                                path, create=True, nonblocking=True
                            )
                    finally:
                        holder.close()

    def test_existing_cache_seed_is_resolved_through_content_locator(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            object_path = root / "control" / "qualification" / "cold-runtime" / "cache-seeds" / "objects" / ("d" * 64)
            manifest = {
                "apparentBytes": 1,
                "contentDigest": sha("d"),
                "entries": 1,
                "schema": "codeclew-cold-cache-seed/2.0",
            }
            with (
                mock.patch.object(gate, "recover_seed", return_value=(object_path, manifest)) as recover,
                mock.patch.object(gate, "publish_seed", side_effect=AssertionError("existing seed republished")),
                mock.patch.object(
                    gate,
                    "create_seed_candidate",
                    side_effect=AssertionError("existing seed created a candidate"),
                ),
            ):
                observed = gate.prepare_cache_seed(
                    root / "source",
                    root / "work",
                    root / "control",
                    build_authority(),
                    Path.home(),
                )
            self.assertEqual(observed, (object_path, manifest))
            recover.assert_called_once()
            self.assertEqual(recover.call_args.args[1], sha("2"))

    def test_concurrent_prepare_singleflights_prime_for_one_authority(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            authority = build_authority()
            store = (
                root / "control" / "qualification" / "cold-runtime" / "cache-seeds"
            )
            object_path = store / "objects" / ("d" * 64)
            manifest = {
                "apparentBytes": 1,
                "contentDigest": sha("d"),
                "entries": 1,
                "schema": "codeclew-cold-cache-seed/2.0",
            }
            state: dict[str, object] = {"published": False, "primeCalls": 0}
            state_lock = threading.Lock()

            def recover(_store: Path, _key: str):
                with state_lock:
                    if state["published"]:
                        return object_path, manifest
                return None

            def prime(*_arguments, **_keywords):
                with state_lock:
                    state["primeCalls"] = int(state["primeCalls"]) + 1
                payload = gate.canonical(prime_evidence(authority)) + b"\n"
                return gate.RunResult(0, b"", False, payload, False)

            def publish(candidate: Path, _store: Path, _key: str):
                shutil.rmtree(candidate)
                with state_lock:
                    state["published"] = True
                return object_path, manifest

            barrier = threading.Barrier(3)
            results = []
            failures = []

            def prepare(index: int) -> None:
                try:
                    barrier.wait()
                    results.append(
                        gate.prepare_cache_seed(
                            root / "source",
                            root / f"work-{index}",
                            root / "control",
                            authority,
                            Path.home(),
                        )
                    )
                except BaseException as error:
                    failures.append(error)

            with (
                mock.patch.object(gate, "recover_seed", side_effect=recover),
                mock.patch.object(gate, "run", side_effect=prime),
                mock.patch.object(gate, "publish_seed", side_effect=publish),
            ):
                threads = [threading.Thread(target=prepare, args=(index,)) for index in range(2)]
                for thread in threads:
                    thread.start()
                barrier.wait()
                for thread in threads:
                    thread.join(timeout=10)
                    self.assertFalse(thread.is_alive())
            self.assertEqual(failures, [])
            self.assertEqual(results, [(object_path, manifest), (object_path, manifest)])
            self.assertEqual(state["primeCalls"], 1)

    def test_prepare_recovers_interrupted_publication_without_prime(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            authority = build_authority()
            recovered = (root / "store" / "objects" / ("d" * 64), {"contentDigest": sha("d")})
            with (
                mock.patch.object(gate, "recover_seed", return_value=recovered) as recover,
                mock.patch.object(
                    gate,
                    "create_seed_candidate",
                    side_effect=AssertionError("recovered seed created a candidate"),
                ),
                mock.patch.object(gate, "run", side_effect=AssertionError("recovered seed primed")),
            ):
                observed = gate.prepare_cache_seed(
                    root / "source",
                    root / "work",
                    root / "control",
                    authority,
                    Path.home(),
                )
            self.assertEqual(observed, recovered)
            recover.assert_called_once()

    def test_publish_failure_discards_candidate_without_masking_error(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            authority = build_authority()
            payload = gate.canonical(prime_evidence(authority)) + b"\n"

            def cleanup(path: str, _timeout: int, _identity: tuple[int, int]) -> bool:
                shutil.rmtree(path)
                return True

            with (
                mock.patch.object(gate, "recover_seed", return_value=None),
                mock.patch.object(
                    gate, "run", return_value=gate.RunResult(0, b"", False, payload, False)
                ),
                mock.patch.object(
                    gate,
                    "publish_seed",
                    side_effect=gate.CacheAuthorityError("publish failed"),
                ),
                mock.patch.object(gate, "bounded_gate_cleanup", side_effect=cleanup),
                self.assertRaisesRegex(gate.CacheAuthorityError, "publish failed"),
            ):
                gate.prepare_cache_seed(
                    root / "source",
                    root / "work",
                    root / "control",
                    authority,
                    Path.home(),
                )
            store = root / "control" / "qualification" / "cold-runtime" / "cache-seeds"
            self.assertEqual(list(store.glob(".candidate-*")), [])

    def test_prime_failure_discards_candidate_without_masking_error(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            authority = build_authority()

            def cleanup(path: str, _timeout: int, _identity: tuple[int, int]) -> bool:
                shutil.rmtree(path)
                return True

            with (
                mock.patch.object(gate, "recover_seed", return_value=None),
                mock.patch.object(
                    gate,
                    "run",
                    return_value=gate.RunResult(
                        1, b"prime failed", False, b"", False
                    ),
                ),
                mock.patch.object(
                    gate,
                    "publish_seed",
                    side_effect=AssertionError("failed prime was published"),
                ),
                mock.patch.object(gate, "bounded_gate_cleanup", side_effect=cleanup),
                self.assertRaisesRegex(gate.GateError, "dependency cache prime failed"),
            ):
                gate.prepare_cache_seed(
                    root / "source",
                    root / "work",
                    root / "control",
                    authority,
                    Path.home(),
                )
            store = root / "control" / "qualification" / "cold-runtime" / "cache-seeds"
            self.assertEqual(list(store.glob(".candidate-*")), [])

    def test_aggregate_rejects_none_identity_and_broken_predecessor(self) -> None:
        arms = arms_from_measurements([(100, 200), (50, 150), (50, 150), (100, 200)])
        arms[0]["artifactHashes"] = None
        arms[0]["armDigest"] = gate._arm_digest(arms[0])
        with self.assertRaises(gate.GateError):
            gate.aggregate(arms, "a" * 40, 8)

        arms = arms_from_measurements([(100, 200), (50, 150), (50, 150), (100, 200)])
        arms[2]["predecessorArmDigest"] = sha("f")
        arms[2]["armDigest"] = gate._arm_digest(arms[2])
        with self.assertRaisesRegex(gate.GateError, "predecessor"):
            gate.aggregate(arms, "a" * 40, 8)

        arms = arms_from_measurements([(100, 200), (50, 150), (50, 150), (100, 200)])
        for arm in arms:
            arm["loadAuthority"]["physicalCores"] = 1
            arm["armDigest"] = gate._arm_digest(arm)
        with self.assertRaises(gate.GateError):
            gate.aggregate(arms, "a" * 40, 8)

    def test_run_streams_and_truncates_both_outputs(self) -> None:
        program = (
            "import os\n"
            f"data=b'x'*{gate.MAX_PROCESS_OUTPUT * 2}\n"
            "os.write(1,data)\n"
            "os.write(2,data)\n"
        )
        result = gate.run([sys.executable, "-c", program], Path.cwd(), os.environ.copy())
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout), gate.MAX_PROCESS_OUTPUT)
        self.assertEqual(len(result.stderr), gate.MAX_PROCESS_OUTPUT)
        self.assertTrue(result.stdout_truncated)
        self.assertTrue(result.stderr_truncated)

    def test_failure_diagnostic_persists_only_redacted_envelope(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            control = Path(directory).resolve() / "control"
            control.mkdir(mode=0o700)
            personal_path = b"/" + b"Users/" + b"private/person"
            secret = b"token=super-secret " + personal_path + b" \x00\xff"
            value = gate._failure_value(
                gate.GateError("TEST_FAILURE", "failed", secret), control
            )
            self.assertEqual(value["diagnosticStatus"], "STORED_PRIVATE")
            diagnostic_digest = str(value["diagnosticDigest"])
            stored = (
                control
                / "diagnostics"
                / "cold-runtime"
                / f"{diagnostic_digest.removeprefix('sha256:')}.stderr"
            ).read_bytes()
            self.assertNotIn(b"super-secret", stored)
            self.assertNotIn(personal_path, stored)
            self.assertNotIn(b"\x00\xff", stored)
            envelope = json.loads(stored)
            self.assertEqual(envelope["status"], "REDACTED")
            self.assertEqual(envelope["byteCount"], len(secret))

    @unittest.skipUnless(hasattr(os, "killpg"), "POSIX process groups required")
    def test_controller_group_cancellation_reaches_worker_and_descendant(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "pids.json"
            worker_program = (
                "import json,os,pathlib,subprocess,sys,time\n"
                "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'])\n"
                "pathlib.Path(sys.argv[1]).write_text(json.dumps({"
                "'workerPid':os.getpid(),'workerPgid':os.getpgrp(),"
                "'childPid':child.pid,'childPgid':os.getpgid(child.pid)}))\n"
                "time.sleep(60)\n"
            )
            controller_program = (
                "import os,pathlib,sys\n"
                f"sys.path.insert(0,{str(Path(__file__).resolve().parent)!r})\n"
                "import cold_multicore_gate as gate\n"
                f"gate.run([sys.executable,'-c',{worker_program!r},sys.argv[1]],pathlib.Path.cwd(),os.environ.copy())\n"
            )
            controller = subprocess.Popen(
                [sys.executable, "-c", controller_program, str(marker)],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            identities = {"workerPgid": controller.pid}
            try:
                deadline = time.monotonic() + 10
                while not marker.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertTrue(marker.exists())
                identities = json.loads(marker.read_text())
                self.assertEqual(identities["workerPgid"], identities["workerPid"])
                self.assertEqual(identities["childPgid"], identities["workerPid"])
                os.kill(controller.pid, signal.SIGTERM)
                controller.wait(timeout=10)
                deadline = time.monotonic() + 5
                while time.monotonic() < deadline:
                    status = subprocess.run(
                        ["/bin/ps", "-p", str(identities["childPid"]), "-o", "stat="],
                        check=False,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.DEVNULL,
                        text=True,
                    ).stdout.strip()
                    if not status or status.startswith("Z"):
                        break
                    time.sleep(0.05)
                self.assertTrue(not status or status.startswith("Z"))
            finally:
                if marker.exists():
                    try:
                        os.killpg(int(identities["workerPgid"]), signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                try:
                    controller.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    pass
        source = Path(gate.__file__).read_text(encoding="utf-8")
        run_source = source[source.index("def run("):source.index("def _diagnostic(")]
        self.assertIn("_spawn_command", run_source)
        self.assertIn("_terminate_process_group", run_source)

    @unittest.skipUnless(hasattr(os, "killpg"), "POSIX process groups required")
    def test_signal_after_leader_exit_still_reaps_residual_group(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            child_marker = root / "child.pid"
            gap_marker = root / "gap"
            worker_program = (
                "import pathlib,subprocess,sys\n"
                "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'])\n"
                "pathlib.Path(sys.argv[1]).write_text(str(child.pid))\n"
            )
            controller_program = (
                "import os,pathlib,sys,time\n"
                f"sys.path.insert(0,{str(Path(__file__).resolve().parent)!r})\n"
                "import cold_multicore_gate as gate\n"
                "real_probe=gate._process_group_exists\n"
                "first=True\n"
                "def probe(group):\n"
                " global first\n"
                " if first:\n"
                "  first=False\n"
                "  pathlib.Path(sys.argv[2]).write_text('gap')\n"
                "  time.sleep(30)\n"
                " return real_probe(group)\n"
                "gate._process_group_exists=probe\n"
                f"gate.run([sys.executable,'-c',{worker_program!r},sys.argv[1]],pathlib.Path.cwd(),os.environ.copy())\n"
            )
            controller = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    controller_program,
                    str(child_marker),
                    str(gap_marker),
                ],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            child_pid = None
            child_group = None
            try:
                deadline = time.monotonic() + 10
                while (
                    (not child_marker.exists() or not gap_marker.exists())
                    and time.monotonic() < deadline
                ):
                    time.sleep(0.02)
                self.assertTrue(child_marker.exists())
                self.assertTrue(gap_marker.exists())
                child_pid = int(child_marker.read_text())
                child_group = os.getpgid(child_pid)
                os.kill(controller.pid, signal.SIGTERM)
                controller.wait(timeout=10)
                self.assertNotEqual(controller.returncode, -signal.SIGTERM)
                deadline = time.monotonic() + 5
                status = ""
                while time.monotonic() < deadline:
                    status = subprocess.run(
                        ["/bin/ps", "-p", str(child_pid), "-o", "stat="],
                        check=False,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.DEVNULL,
                        text=True,
                    ).stdout.strip()
                    if not status or status.startswith("Z"):
                        break
                    time.sleep(0.05)
                self.assertTrue(not status or status.startswith("Z"))
            finally:
                if child_group is not None:
                    try:
                        os.killpg(child_group, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                try:
                    controller.kill()
                except ProcessLookupError:
                    pass
                try:
                    controller.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    pass

    @unittest.skipUnless(hasattr(os, "killpg"), "POSIX process groups required")
    def test_repeated_signal_during_termination_cannot_abandon_group(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            child_marker = root / "child.json"
            termination_marker = root / "terminating"
            child_program = (
                "import signal,time\n"
                "signal.signal(signal.SIGTERM,signal.SIG_IGN)\n"
                "time.sleep(60)\n"
            )
            worker_program = (
                "import json,os,pathlib,signal,subprocess,sys,time\n"
                "signal.signal(signal.SIGTERM,signal.SIG_IGN)\n"
                f"child=subprocess.Popen([sys.executable,'-c',{child_program!r}])\n"
                "pathlib.Path(sys.argv[1]).write_text(json.dumps({"
                "'childPid':child.pid,'processGroup':os.getpgrp()}))\n"
                "time.sleep(60)\n"
            )
            controller_program = (
                "import os,pathlib,sys,time\n"
                f"sys.path.insert(0,{str(Path(__file__).resolve().parent)!r})\n"
                "import cold_multicore_gate as gate\n"
                "real_terminate=gate._terminate_process_group\n"
                "def terminate(process):\n"
                " pathlib.Path(sys.argv[2]).write_text('terminating')\n"
                " time.sleep(1)\n"
                " return real_terminate(process)\n"
                "gate._terminate_process_group=terminate\n"
                f"gate.run([sys.executable,'-c',{worker_program!r},sys.argv[1]],pathlib.Path.cwd(),os.environ.copy())\n"
            )
            controller = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    controller_program,
                    str(child_marker),
                    str(termination_marker),
                ],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            process_group = None
            child_pid = None
            try:
                deadline = time.monotonic() + 10
                while not child_marker.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertTrue(child_marker.exists())
                identity = json.loads(child_marker.read_text())
                child_pid = int(identity["childPid"])
                process_group = int(identity["processGroup"])
                os.kill(controller.pid, signal.SIGTERM)
                deadline = time.monotonic() + 10
                while not termination_marker.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertTrue(termination_marker.exists())
                os.kill(controller.pid, signal.SIGTERM)
                controller.wait(timeout=15)
                self.assertNotEqual(controller.returncode, -signal.SIGTERM)
                status = subprocess.run(
                    ["/bin/ps", "-p", str(child_pid), "-o", "stat="],
                    check=False,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                    text=True,
                ).stdout.strip()
                self.assertTrue(not status or status.startswith("Z"))
            finally:
                if process_group is not None:
                    try:
                        os.killpg(process_group, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                try:
                    controller.kill()
                except ProcessLookupError:
                    pass
                try:
                    controller.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    pass

    @unittest.skipUnless(hasattr(signal, "pthread_sigmask"), "pthread masks required")
    def test_signal_at_success_handoff_cannot_be_swallowed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            handoff = root / "handoff"
            survived = root / "survived"
            controller_program = (
                "import os,pathlib,signal,sys,time\n"
                f"sys.path.insert(0,{str(Path(__file__).resolve().parent)!r})\n"
                "import cold_multicore_gate as gate\n"
                "real_mask=signal.pthread_sigmask\n"
                "blocks=0\n"
                "def mask(how,values):\n"
                " global blocks\n"
                " if how==signal.SIG_BLOCK:\n"
                "  blocks+=1\n"
                "  if blocks==2:\n"
                "   pathlib.Path(sys.argv[1]).write_text('handoff')\n"
                "   time.sleep(30)\n"
                " return real_mask(how,values)\n"
                "gate.signal.pthread_sigmask=mask\n"
                "gate.run([sys.executable,'-c','pass'],pathlib.Path.cwd(),os.environ.copy())\n"
                "pathlib.Path(sys.argv[2]).write_text('survived')\n"
            )
            controller = subprocess.Popen(
                [sys.executable, "-c", controller_program, str(handoff), str(survived)],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            try:
                deadline = time.monotonic() + 10
                while not handoff.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertTrue(handoff.exists())
                os.kill(controller.pid, signal.SIGTERM)
                controller.wait(timeout=10)
                self.assertNotEqual(controller.returncode, 0)
                self.assertFalse(survived.exists())
            finally:
                try:
                    controller.kill()
                except ProcessLookupError:
                    pass
                try:
                    controller.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    pass

    def test_frozen_source_has_no_object_store_attachment(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            repository = root / "repository"
            repository.mkdir()
            subprocess.run(["git", "init", "-q"], cwd=repository, check=True)
            (repository / "file.txt").write_text("value\n", encoding="utf-8")
            subprocess.run(["git", "add", "file.txt"], cwd=repository, check=True)
            environment = {
                **os.environ,
                "GIT_AUTHOR_NAME": "Tests", "GIT_AUTHOR_EMAIL": "tests@example.invalid",
                "GIT_COMMITTER_NAME": "Tests", "GIT_COMMITTER_EMAIL": "tests@example.invalid",
            }
            subprocess.run(["git", "commit", "-qm", "seed"], cwd=repository, env=environment, check=True)
            revision = subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=repository, text=True
            ).strip()
            work = root / "work"
            work.mkdir()
            source = gate.frozen_source(repository, work, revision)
            self.assertFalse((source / ".git" / "objects" / "info" / "alternates").exists())
            repository.rename(root / "repository-moved")
            observed = subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=source, text=True
            ).strip()
            self.assertEqual(observed, revision)

    def test_cleanup_refuses_symlink_and_outside_target(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            parent = root / "parent"
            outside = root / "outside"
            parent.mkdir(mode=0o700)
            outside.mkdir(mode=0o700)
            marker = outside / "marker"
            marker.write_text("keep", encoding="utf-8")
            metadata = outside.lstat()
            with mock.patch.object(gate, "bounded_gate_cleanup") as cleanup:
                self.assertFalse(
                    gate.cleanup_owned_tree(
                        outside, parent, (metadata.st_dev, metadata.st_ino)
                    )
                )
                link = parent / "link"
                link.symlink_to(outside)
                link_metadata = link.lstat()
                self.assertFalse(
                    gate.cleanup_owned_tree(
                        link, parent, (link_metadata.st_dev, link_metadata.st_ino)
                    )
                )
                cleanup.assert_not_called()
            self.assertEqual(marker.read_text(encoding="utf-8"), "keep")

    def test_stale_recovery_uses_bounded_cleanup_and_retains_completed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runs = Path(directory).resolve()
            host = host_fixture()
            boot = {"identity": "boot", "platform": "Test", "schema": gate.BOOT_SCHEMA}
            incomplete = gate.create_cohort(
                runs, "a" * 40, sha("7"), host, boot, build_authority()
            )
            incomplete_path = incomplete.path
            completed = gate.create_cohort(
                runs, "a" * 40, sha("7"), host, boot, build_authority()
            )
            completed_path = completed.path
            completed_arms = arms_from_measurements(
                [(100, 200), (50, 150), (50, 150), (100, 200)],
                cohort_authority=completed.authority,
            )
            persist_arms(completed, completed_arms)
            gate.complete_cohort(
                completed,
                completed_arms,
                final_report(completed, completed_arms),
            )
            complete_receipt = completed.path / "COMPLETE.json"
            interrupted_journal = (
                completed.path
                / f".COMPLETE.json.{os.getpid()}.{'a' * 24}.tmp"
            )
            os.link(complete_receipt, interrupted_journal)
            self.assertEqual(complete_receipt.stat().st_nlink, 2)
            completed.close()
            incomplete.close()

            calls = []

            def cleanup(path: str, _timeout: int, _identity: tuple[int, int]) -> bool:
                calls.append(Path(path))
                shutil.rmtree(path)
                return True

            with mock.patch.object(gate, "bounded_gate_cleanup", side_effect=cleanup):
                gate.recover_stale_runs(runs)
            self.assertEqual(calls, [incomplete_path])
            self.assertFalse(incomplete_path.exists())
            self.assertTrue(completed_path.exists())
            self.assertFalse(interrupted_journal.exists())
            self.assertEqual(complete_receipt.stat().st_nlink, 1)

    def test_gate_admission_recovers_stale_work_and_refuses_unknown_entries(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            gate_base = Path(directory).resolve()
            gate_base.chmod(0o700)
            stale = gate_base / "run.abcdef"
            stale.mkdir(mode=0o700)
            calls: list[Path] = []

            def cleanup(path: str, _timeout: int, _identity: tuple[int, int]) -> bool:
                calls.append(Path(path))
                shutil.rmtree(path)
                return True

            with mock.patch.object(gate, "bounded_gate_cleanup", side_effect=cleanup):
                admission = gate.acquire_gate_admission(gate_base)
            try:
                self.assertEqual(calls, [stale])
                self.assertFalse(stale.exists())
                contender = gate._open_private_lock(
                    gate_base / "admission.lock", create=True, nonblocking=True
                )
                self.assertIsNone(contender)
            finally:
                admission.close()

            unknown = gate_base / "privacy-capsule.secret"
            unknown.mkdir(mode=0o700)
            with self.assertRaisesRegex(gate.GateError, "unknown cold gate work entry"):
                gate.acquire_gate_admission(gate_base)

    def test_completion_is_published_only_after_successful_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runs = Path(directory).resolve()
            runs.chmod(0o700)
            cohort = gate.create_cohort(
                runs,
                "a" * 40,
                sha("7"),
                host_fixture(),
                {"identity": "boot", "platform": "Test", "schema": gate.BOOT_SCHEMA},
                build_authority(),
            )
            arms = arms_from_measurements(
                [(100, 200), (50, 150), (50, 150), (100, 200)],
                cohort_authority=cohort.authority,
            )
            persist_arms(cohort, arms)
            report = final_report(cohort, arms)
            try:
                self.assertFalse(
                    gate.publish_completion_if_eligible(
                        cohort,
                        arms,
                        report,
                        measurements_complete=True,
                        cleanup_ok=False,
                    )
                )
                self.assertFalse((cohort.path / "COMPLETE.json").exists())
                self.assertTrue(
                    gate.publish_completion_if_eligible(
                        cohort,
                        arms,
                        report,
                        measurements_complete=True,
                        cleanup_ok=True,
                    )
                )
                receipt = gate._read_exact_json(
                    cohort.path / "COMPLETE.json", gate.COHORT_COMPLETE_FIELDS
                )
                self.assertIsNotNone(receipt)
                self.assertEqual(receipt["reportDigest"], gate.digest(report))
            finally:
                cohort.close()

    def test_completion_refuses_unpersisted_or_minimal_arm_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runs = Path(directory).resolve()
            runs.chmod(0o700)
            cohort = gate.create_cohort(
                runs,
                "a" * 40,
                sha("7"),
                host_fixture(),
                {"identity": "boot", "platform": "Test", "schema": gate.BOOT_SCHEMA},
                build_authority(),
            )
            minimal = [
                {"cohortId": cohort.authority["cohortId"], "armDigest": sha(str(i))}
                for i in range(4)
            ]
            report = {
                "accepted": True,
                "cleanupStatus": "PASSED",
                "schema": gate.REPORT_SCHEMA,
                "sourceRevision": "a" * 40,
                "status": "PASSED",
            }
            try:
                with self.assertRaisesRegex(gate.GateError, "receipt closure"):
                    gate.complete_cohort(cohort, minimal, report)
                self.assertFalse((cohort.path / "COMPLETE.json").exists())
                self.assertFalse((cohort.path / "REPORT.json").exists())
            finally:
                cohort.close()

    def test_cleanup_failure_overrides_success_but_preserves_primary_failure(self) -> None:
        success = {"accepted": True, "schema": gate.REPORT_SCHEMA, "status": "PASSED"}
        value, code = gate.apply_cleanup_outcome(success, 0, False)
        self.assertEqual(code, 1)
        self.assertEqual(value["failureStage"], "CLEANUP_WORKTREE")
        self.assertEqual(value["status"], "FAILED_INCOMPLETE")

        primary = {
            "accepted": False,
            "failureStage": "ARM_EVIDENCE",
            "schema": gate.REPORT_SCHEMA,
            "status": "FAILED_INCOMPLETE",
        }
        value, code = gate.apply_cleanup_outcome(primary, 1, False)
        self.assertEqual(code, 1)
        self.assertEqual(value["failureStage"], "ARM_EVIDENCE")
        self.assertEqual(value["cleanupStatus"], "FAILED")

    def test_isolated_environment_partitions_all_mutable_homes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            environment = gate.isolated_environment(
                root / "cache", root / "state", Path.home()
            )
            self.assertEqual(environment["HOME"], str(root / "cache"))
            self.assertEqual(environment["CARGO_HOME"], str(root / "cache" / ".cargo"))
            self.assertEqual(environment["GRADLE_USER_HOME"], str(root / "cache" / ".gradle"))
            self.assertEqual(environment["CODECLEW_HOME"], str(root / "state"))
            self.assertNotEqual(environment["CARGO_HOME"], environment["GRADLE_USER_HOME"])


if __name__ == "__main__":
    unittest.main()
