#!/usr/bin/env python3
"""Fast contract tests for stabilization plan enforcement."""

from __future__ import annotations

import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import stabilization_control as control  # noqa: E402
import stabilization_verifier as verifier  # noqa: E402


class StabilizationControlTest(unittest.TestCase):
    def setUp(self) -> None:
        control._DYNAMIC_AUTHORITY_CACHE.clear()
        self.temporary = tempfile.TemporaryDirectory()
        self.previous_home = os.environ.get("CODECLEW_CONTROL_HOME")
        os.environ["CODECLEW_CONTROL_HOME"] = self.temporary.name
        os.chmod(self.temporary.name, 0o700)
        self.plan = control.load_json(control.PLAN_PATH)
        self.model = control.validate_plan(self.plan)
        self.authority = control.authorities(self.plan)

    def tearDown(self) -> None:
        if self.previous_home is None:
            os.environ.pop("CODECLEW_CONTROL_HOME", None)
        else:
            os.environ["CODECLEW_CONTROL_HOME"] = self.previous_home
        self.temporary.cleanup()

    def test_current_plan_is_closed_and_acyclic(self) -> None:
        self.assertEqual(self.plan["schema"], "codeclew-stabilization-plan/2.0")
        self.assertEqual(self.model["order"][0], "S0")
        self.assertEqual(self.model["order"][-1], "R5")
        self.assertEqual(set(self.model["tiers"]), {f"L{index}" for index in range(8)})
        self.assertEqual(
            self.model["checks"]["s3-trusted-seed"]["timeoutPolicy"],
            "UNBOUNDED",
        )
        self.assertTrue(
            all(
                "timeoutPolicy" not in check
                for check_id, check in self.model["checks"].items()
                if check_id != "s3-trusted-seed"
            )
        )

    def test_unknown_timeout_policy_is_rejected(self) -> None:
        value = copy.deepcopy(self.plan)
        value["checks"][0]["timeoutPolicy"] = "SOMETIMES"
        with self.assertRaisesRegex(control.ControlError, "timeout policy"):
            control.validate_plan(value)

    def test_unbounded_timeout_is_rejected_outside_trusted_seed(self) -> None:
        value = copy.deepcopy(self.plan)
        target = next(check for check in value["checks"] if check["id"] == "r5-push-ci")
        target["timeoutPolicy"] = "UNBOUNDED"
        with self.assertRaisesRegex(control.ControlError, "timeout policy"):
            control.validate_plan(value)

    def test_dependency_cycle_is_rejected(self) -> None:
        value = copy.deepcopy(self.plan)
        value["steps"][0]["dependencies"] = ["R5"]
        with self.assertRaisesRegex(control.ControlError, "cycle"):
            control.validate_plan(value)

    def test_direct_expensive_gate_is_refused(self) -> None:
        environment = dict(os.environ)
        environment.pop("CODECLEW_PLAN_CAPABILITY", None)
        completed = subprocess.run(
            (
                sys.executable,
                "-I",
                "-S",
                str(ROOT / "scripts" / "stabilization_control.py"),
                "guard",
                "--gate",
                "cold-runtime",
            ),
            cwd=ROOT,
            env=environment,
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        self.assertEqual(completed.returncode, 2)
        self.assertIn(b"direct expensive gate execution is forbidden", completed.stdout)

    def test_capability_is_signed_and_single_use(self) -> None:
        capability = control.issue_capability(self.authority, "cold-runtime", 60)
        environment = dict(os.environ)
        environment["CODECLEW_PLAN_CAPABILITY"] = str(capability)
        command = (
            sys.executable,
            "-I",
            "-S",
            str(ROOT / "scripts" / "stabilization_control.py"),
            "guard",
            "--gate",
            "cold-runtime",
        )
        first = subprocess.run(command, cwd=ROOT, env=environment, check=False, stdout=subprocess.PIPE)
        second = subprocess.run(command, cwd=ROOT, env=environment, check=False, stdout=subprocess.PIPE)
        self.assertEqual(first.returncode, 0)
        self.assertEqual(second.returncode, 2)

    def test_failed_evidence_key_cannot_be_retried_blindly(self) -> None:
        model = copy.deepcopy(self.model)
        model["steps"]["S4"]["dependencies"] = []
        check = model["checks"]["s4-runtime-contracts"]
        evidence = control.evidence_digest(check, self.authority)
        path = control.receipt_path(self.authority, "s4-runtime-contracts", evidence)
        control.atomic_private_write(path, control.canonical({"status": "FAIL"}) + b"\n")
        with self.assertRaisesRegex(control.ControlError, "blind retry refused"):
            control.run_check(
                self.plan, model, self.authority, "S4", "s4-runtime-contracts"
            )

    def test_unverifiable_check_attempt_records_a_blind_retry_marker(self) -> None:
        model = copy.deepcopy(self.model)
        model["steps"]["S4"]["dependencies"] = []
        attempts = 0

        def reject(*_arguments):
            nonlocal attempts
            attempts += 1
            raise control.ControlError("authority changed during execution")

        with (
            mock.patch.object(
                control, "dynamic_authority_digest", return_value="sha256:" + "1" * 64
            ),
            mock.patch.object(
                control, "evidence_digest", return_value="sha256:" + "2" * 64
            ),
            mock.patch.object(control, "verified_receipt", side_effect=reject),
        ):
            with self.assertRaisesRegex(control.ControlError, "authority changed"):
                control.run_check(
                    self.plan, model, self.authority, "S4", "s4-runtime-contracts"
                )
            with self.assertRaisesRegex(control.ControlError, "blind retry refused"):
                control.run_check(
                    self.plan, model, self.authority, "S4", "s4-runtime-contracts"
                )
        self.assertEqual(attempts, 1)

    def test_controller_validate_is_machine_readable(self) -> None:
        completed = subprocess.run(
            (sys.executable, "-I", "-S", str(ROOT / "scripts" / "stabilization_control.py"), "validate"),
            cwd=ROOT,
            env=dict(os.environ),
            check=True,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            text=True,
        )
        value = json.loads(completed.stdout)
        self.assertEqual(value["status"], "PASS")
        self.assertEqual(value["schema"], "codeclew-stabilization-plan-validation/1.0")

    def test_global_authority_binds_native_python_runtime(self) -> None:
        with mock.patch.object(
            control,
            "native_python_runtime_authority",
            return_value={"runtime": "first"},
        ):
            first = control.authorities(self.plan)
        with mock.patch.object(
            control,
            "native_python_runtime_authority",
            return_value={"runtime": "second"},
        ):
            second = control.authorities(self.plan)
        self.assertNotEqual(first["controllerDigest"], second["controllerDigest"])
        self.assertNotEqual(first["verifierDigest"], second["verifierDigest"])
        self.assertEqual(first["planDigest"], second["planDigest"])

    def test_controller_authority_closes_over_dynamic_sources(self) -> None:
        source_digests = control.controller_source_digests()
        self.assertEqual(
            set(source_digests),
            {
                "bootstrap/clew_bootstrap.py",
                "scripts/stabilization_control.py",
                "scripts/trusted_seed_gc.py",
            },
        )
        first = control.authorities(self.plan)
        changed = dict(source_digests)
        changed["scripts/trusted_seed_gc.py"] = "sha256:" + "f" * 64
        with mock.patch.object(
            control, "controller_source_digests", return_value=changed
        ):
            second = control.authorities(self.plan)
        self.assertNotEqual(first["controllerDigest"], second["controllerDigest"])
        self.assertEqual(first["planDigest"], second["planDigest"])

    def test_controller_rejects_a_different_path_python(self) -> None:
        other = Path("/usr/bin/python3")
        if not other.exists() or other.resolve() == Path(sys.executable).resolve():
            self.skipTest("no distinct system Python is installed")
        completed = subprocess.run(
            (str(other), "-I", "-S", str(control.__file__), "validate"),
            cwd=ROOT,
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        self.assertEqual(completed.returncode, 2)
        self.assertIn(b"differs from the python3 command authority", completed.stdout)

    def test_timeout_reaps_the_owned_process_group(self) -> None:
        check = {
            "command": [sys.executable, "-I", "-S", "-c", "import time; time.sleep(30)"],
            "gate": None,
        }
        tier = {"budgetSeconds": 1}
        exit_code, duration, _stdout, _stderr = control.invoke(check, tier, self.authority)
        self.assertEqual(exit_code, 124)
        self.assertLess(duration, 7000)

    def test_unbounded_execution_waits_without_a_deadline(self) -> None:
        check = {
            "command": ["ignored"],
            "gate": None,
            "timeoutPolicy": "UNBOUNDED",
        }
        tier = {"budgetSeconds": 1}
        process = mock.Mock(pid=12345)
        process.wait.return_value = 0
        with mock.patch.object(control.subprocess, "Popen", return_value=process):
            exit_code, _duration, _stdout, _stderr = control.invoke(
                check, tier, self.authority
            )
        self.assertEqual(exit_code, 0)
        process.wait.assert_called_once_with(timeout=None)

    def test_unbounded_check_is_not_rejected_by_the_tier_duration_budget(self) -> None:
        check = self.model["checks"]["s3-trusted-seed"]
        tier = self.model["tiers"][check["tier"]]
        request = {
            "checkId": check["id"],
            "clean": True,
            "command": check["command"],
            "commandDigest": control.digest_bytes(control.canonical(check["command"])),
            "controllerDigest": self.authority["controllerDigest"],
            "durationMillis": (tier["budgetSeconds"] + 1) * 1000,
            "environmentDigest": "sha256:" + "1" * 64,
            "exitCode": 0,
            "inputDigest": "sha256:" + "2" * 64,
            "memoryBytes": control.memory_bytes(),
            "physicalCores": control.physical_cores(),
            "planDigest": self.authority["planDigest"],
            "sourceRevision": control.git("rev-parse", "HEAD"),
            "stderrDigest": "sha256:" + "3" * 64,
            "stdoutDigest": "sha256:" + "4" * 64,
            "stepId": check["step"],
            "tier": check["tier"],
            "verifierDigest": self.authority["verifierDigest"],
        }
        self.assertEqual(verifier.verify(request)["status"], "PASS")

    def test_failed_receipt_does_not_probe_a_missing_postcondition(self) -> None:
        model = copy.deepcopy(self.model)
        model["steps"]["S3"]["dependencies"] = []
        with (
            mock.patch.object(control, "dynamic_authority_digest", return_value="sha256:" + "1" * 64),
            mock.patch.object(control, "evidence_digest", return_value="sha256:" + "2" * 64),
            mock.patch.object(
                control,
                "verified_receipt",
                return_value={"exitCode": 124, "status": "FAIL"},
            ),
            mock.patch.object(control, "postcondition_authority_digest") as postcondition,
        ):
            with self.assertRaisesRegex(
                control.ControlError, "status FAIL and exit code 124"
            ):
                control.run_check(
                    self.plan, model, self.authority, "S3", "s3-trusted-seed"
                )
        postcondition.assert_not_called()

    def test_completion_fails_closed_when_required_check_authority_changes(self) -> None:
        step = "S0"
        authorities = {
            check_id: control.check_authority_digest(
                self.model["checks"][check_id], self.authority
            )
            for check_id in self.model["steps"][step]["requiredChecks"]
        }
        completion = {
            **self.authority,
            "checkAuthorities": authorities,
            "postconditionAuthorities": {
                check_id: control.postcondition_authority_digest(
                    self.model["checks"][check_id], refresh=True
                )
                for check_id in self.model["steps"][step]["requiredChecks"]
            },
            "receiptDigests": [],
            "schema": "codeclew-stabilization-step-completion/1.0",
            "sourceRevision": control.git("rev-parse", "HEAD"),
            "status": "COMPLETE",
            "stepId": step,
        }
        completion["completionDigest"] = control.digest_bytes(
            control.canonical(completion)
        )
        control.atomic_private_write(
            control.completion_path(self.authority, step),
            control.canonical(completion) + b"\n",
        )
        self.assertTrue(control.valid_completion(self.model, self.authority, step))

        changed = "sha256:" + "f" * 64
        with mock.patch.object(
            control, "check_authority_digest", return_value=changed
        ):
            self.assertFalse(
                control.valid_completion(self.model, self.authority, step)
            )

    def test_completion_reuse_does_not_require_transient_check_environment(self) -> None:
        check = self.model["checks"]["s0-recovery-baseline"]
        with mock.patch.dict(
            os.environ, {"CODECLEW_RECOVERY_MANIFEST": "/private/first.json"}
        ):
            first = control.check_authority_digest(check, self.authority)
            first_evidence = control.evidence_digest(check, self.authority)
        with mock.patch.dict(
            os.environ, {"CODECLEW_RECOVERY_MANIFEST": "/private/second.json"}
        ):
            second = control.check_authority_digest(check, self.authority)
            second_evidence = control.evidence_digest(check, self.authority)
        self.assertEqual(first, second)
        self.assertNotEqual(first_evidence, second_evidence)

    def test_s3_completion_requires_live_trusted_seed_postcondition(self) -> None:
        step = "S3"
        seed = "sha256:" + "3" * 64
        with mock.patch.object(
            control, "trusted_seed_authority_digest", return_value=seed
        ):
            authorities = {
                check_id: control.check_authority_digest(
                    self.model["checks"][check_id], self.authority
                )
                for check_id in self.model["steps"][step]["requiredChecks"]
            }
            postconditions = {
                check_id: control.postcondition_authority_digest(
                    self.model["checks"][check_id], refresh=True
                )
                for check_id in self.model["steps"][step]["requiredChecks"]
            }
            completion = {
                **self.authority,
                "checkAuthorities": authorities,
                "postconditionAuthorities": postconditions,
                "receiptDigests": [],
                "schema": "codeclew-stabilization-step-completion/1.0",
                "sourceRevision": control.git("rev-parse", "HEAD"),
                "status": "COMPLETE",
                "stepId": step,
            }
            completion["completionDigest"] = control.digest_bytes(
                control.canonical(completion)
            )
            control.atomic_private_write(
                control.completion_path(self.authority, step),
                control.canonical(completion) + b"\n",
            )
            self.assertTrue(
                control.valid_completion(self.model, self.authority, step)
            )
        control._DYNAMIC_AUTHORITY_CACHE.clear()
        with mock.patch.object(
            control,
            "trusted_seed_authority_digest",
            side_effect=control.ControlError("missing trusted seed"),
        ):
            self.assertFalse(
                control.valid_completion(self.model, self.authority, step)
            )

    def test_trusted_seed_replacement_invalidates_q2_completion(self) -> None:
        step = "Q2"
        before = "sha256:" + "1" * 64
        after = "sha256:" + "2" * 64
        with (
            mock.patch.object(
                control, "trusted_seed_authority_digest", return_value=before
            ),
            mock.patch.object(
                control,
                "native_gradle_environment_digest",
                return_value="sha256:" + "9" * 64,
            ),
        ):
            authorities = {
                check_id: control.check_authority_digest(
                    self.model["checks"][check_id], self.authority
                )
                for check_id in self.model["steps"][step]["requiredChecks"]
            }
            completion = {
                **self.authority,
                "checkAuthorities": authorities,
                "postconditionAuthorities": {
                    check_id: control.postcondition_authority_digest(
                        self.model["checks"][check_id], refresh=True
                    )
                    for check_id in self.model["steps"][step]["requiredChecks"]
                },
                "receiptDigests": [],
                "schema": "codeclew-stabilization-step-completion/1.0",
                "sourceRevision": control.git("rev-parse", "HEAD"),
                "status": "COMPLETE",
                "stepId": step,
            }
            completion["completionDigest"] = control.digest_bytes(
                control.canonical(completion)
            )
            control.atomic_private_write(
                control.completion_path(self.authority, step),
                control.canonical(completion) + b"\n",
            )
            self.assertTrue(
                control.valid_completion(self.model, self.authority, step)
            )
        control._DYNAMIC_AUTHORITY_CACHE.clear()
        with (
            mock.patch.object(
                control, "trusted_seed_authority_digest", return_value=after
            ),
            mock.patch.object(
                control,
                "native_gradle_environment_digest",
                return_value="sha256:" + "9" * 64,
            ),
        ):
            self.assertFalse(
                control.valid_completion(self.model, self.authority, step)
            )

    def test_command_refuses_dynamic_authority_change_before_receipt(self) -> None:
        check = self.model["checks"]["q2-multi-compilation"]
        tier = self.model["tiers"][check["tier"]]
        before = "sha256:" + "1" * 64
        after = "sha256:" + "2" * 64
        with (
            mock.patch.object(
                control,
                "invoke",
                return_value=(0, 1, "sha256:" + "3" * 64, "sha256:" + "4" * 64),
            ),
            mock.patch.object(
                control, "dynamic_authority_digest", return_value=after
            ),
            self.assertRaisesRegex(control.ControlError, "changed during execution"),
        ):
            control.verified_receipt(
                self.plan,
                self.authority,
                check,
                tier,
                "sha256:" + "5" * 64,
                before,
            )

    def test_command_refuses_tracked_or_repository_authority_change(self) -> None:
        check = self.model["checks"]["q2-multi-compilation"]
        tier = self.model["tiers"][check["tier"]]
        before = "sha256:" + "1" * 64
        with (
            mock.patch.object(
                control,
                "invoke",
                return_value=(0, 1, "sha256:" + "3" * 64, "sha256:" + "4" * 64),
            ),
            mock.patch.object(
                control, "dynamic_authority_digest", return_value=before
            ),
            mock.patch.object(control, "authorities", return_value=self.authority),
            mock.patch.object(
                control, "evidence_digest", return_value="sha256:" + "2" * 64
            ),
            self.assertRaisesRegex(control.ControlError, "repository authority changed"),
        ):
            control.verified_receipt(
                self.plan,
                self.authority,
                check,
                tier,
                "sha256:" + "5" * 64,
                before,
            )

    def test_dynamic_environment_file_changes_evidence_key(self) -> None:
        check = copy.deepcopy(self.model["checks"]["s0-recovery-baseline"])
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "recovery.json"
            manifest.write_text("first", encoding="utf-8")
            with mock.patch.dict(
                os.environ, {"CODECLEW_RECOVERY_MANIFEST": str(manifest)}
            ):
                first = control.dynamic_authority_digest(check, refresh=True)
                manifest.write_text("second", encoding="utf-8")
                second = control.dynamic_authority_digest(check, refresh=True)
        self.assertNotEqual(first, second)

    def test_native_gradle_environment_binds_tool_and_cache_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            tools = root / "tools"
            gradle = root / "gradle"
            cache_file = gradle / "caches" / "modules-2" / "files-2.1" / "artifact.jar"
            selection_index = (
                gradle
                / "caches"
                / "modules-2"
                / "metadata-2.107"
                / "module-artifact.bin"
            )
            volatile_lock = gradle / "caches" / "modules-2" / "modules-2.lock"
            tools.mkdir()
            cache_file.parent.mkdir(parents=True)
            selection_index.parent.mkdir(parents=True)
            java = tools / "java"
            git = tools / "git"
            tar = tools / "tar"
            java.write_bytes(b"java-tool")
            git.write_bytes(b"git-tool")
            tar.write_bytes(b"tar-tool")
            os.chmod(java, 0o700)
            os.chmod(git, 0o700)
            os.chmod(tar, 0o700)
            cache_file.write_bytes(b"first")
            selection_index.write_bytes(b"first-index")
            volatile_lock.write_bytes(b"first-lock")

            def which(name: str) -> str:
                return str({"git": git, "java": java, "tar": tar}[name])

            observation = subprocess.CompletedProcess(
                [str(java)],
                0,
                stdout=b"",
                stderr=f"java.home = {root}\nuser.home = {root}\njava.version = 21\n".encode(),
            )
            with (
                mock.patch.dict(
                    os.environ,
                    {
                        "GRADLE_USER_HOME": str(gradle),
                        "HOME": str(root),
                        "PATH": str(tools),
                    },
                    clear=False,
                ),
                mock.patch.object(control.shutil, "which", side_effect=which),
                mock.patch.object(control.subprocess, "run", return_value=observation),
                mock.patch.object(control, "qualification_tool_authority", return_value={}),
                mock.patch.object(control, "native_python_runtime_authority", return_value={}),
            ):
                first = control.native_gradle_environment_digest()
                volatile_lock.write_bytes(b"second-lock")
                lock_only = control.native_gradle_environment_digest()
                selection_index.write_bytes(b"second-index")
                metadata_changed = control.native_gradle_environment_digest()
                cache_file.write_bytes(b"second")
                second = control.native_gradle_environment_digest()
            self.assertEqual(first, lock_only)
            self.assertNotEqual(first, metadata_changed)
            self.assertNotEqual(first, second)

    def test_gradle_effective_home_includes_jvm_property_override(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            override = root / "gradle-override"
            with mock.patch.dict(
                os.environ,
                {"GRADLE_OPTS": f"-Dgradle.user.home={override}"},
                clear=False,
            ):
                homes = control.gradle_effective_homes(root)
            self.assertIn(override, homes)

    def test_native_java_prefers_java_home_over_path(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            home = root / "jdk"
            path_tools = root / "path"
            (home / "bin").mkdir(parents=True)
            path_tools.mkdir()
            selected = home / "bin" / "java"
            decoy = path_tools / "java"
            selected.write_bytes(b"selected-java")
            decoy.write_bytes(b"path-java")
            os.chmod(selected, 0o700)
            os.chmod(decoy, 0o700)
            actual_home = root / "actual-jdk"
            (actual_home / "bin").mkdir(parents=True)
            actual_java = actual_home / "bin" / "java"
            library = actual_home / "lib" / "server" / "libjvm.dylib"
            library.parent.mkdir(parents=True)
            library.write_bytes(b"first-runtime")
            actual_java.write_bytes(b"actual-java")
            os.chmod(actual_java, 0o700)
            selected.unlink()
            selected.symlink_to(actual_java)
            observation = subprocess.CompletedProcess(
                [str(actual_java)],
                0,
                stdout=b"",
                stderr=f"java.home = {actual_home}\nuser.home = {root}\njava.version = 21\n".encode(),
            )
            with (
                mock.patch.dict(
                    os.environ,
                    {"JAVA_HOME": str(home), "PATH": str(path_tools)},
                    clear=False,
                ),
                mock.patch.object(control.shutil, "which", return_value=str(decoy)),
                mock.patch.object(control.subprocess, "run", return_value=observation),
            ):
                java, _user_home, authority = control.native_java_authority()
                library.write_bytes(b"second-runtime")
                _java, _home, changed = control.native_java_authority()
            self.assertEqual(java, actual_java.resolve())
            self.assertEqual(authority["selection"], "JAVA_HOME")
            self.assertNotEqual(authority, changed)

    def test_native_maven_environment_binds_repository_and_distribution(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            tools = root / "tools"
            jdk = root / "jdk"
            maven = root / "maven"
            repository = root / "custom-repository"
            tools.mkdir()
            (jdk / "bin").mkdir(parents=True)
            (maven / "lib").mkdir(parents=True)
            repository.mkdir(parents=True)
            paths = {
                "java": jdk / "bin" / "java",
                "git": tools / "git",
                "tar": tools / "tar",
                "mvn": tools / "mvn",
            }
            for name, path in paths.items():
                path.write_bytes(name.encode())
                os.chmod(path, 0o700)
            (maven / "lib" / "maven-core.jar").write_bytes(b"distribution")
            artifact = repository / "artifact.jar"
            artifact.write_bytes(b"first")
            settings = root / "custom-settings.xml"
            settings.write_text(
                f"<settings><localRepository>{repository}</localRepository></settings>",
                encoding="utf-8",
            )

            def which(name: str) -> str:
                return str(paths[name])

            def observe(command, **_kwargs):
                if str(command[0]) == str(paths["java"].resolve()):
                    output = f"java.home = {jdk}\nuser.home = {root}\njava.version = 21\n"
                else:
                    output = f"Apache Maven 3.9\nMaven home: {maven}\n"
                return subprocess.CompletedProcess(command, 0, stdout=b"", stderr=output.encode())

            with (
                mock.patch.dict(
                    os.environ,
                    {
                        "HOME": str(root),
                        "JAVA_HOME": str(jdk),
                        "MAVEN_ARGS": f"-Dmaven.repo.local={repository} --settings {settings}",
                        "MAVEN_HOME": str(maven),
                        "PATH": str(tools),
                    },
                    clear=False,
                ),
                mock.patch.object(control.shutil, "which", side_effect=which),
                mock.patch.object(control.subprocess, "run", side_effect=observe),
                mock.patch.object(control, "qualification_tool_authority", return_value={}),
                mock.patch.object(control, "native_python_runtime_authority", return_value={}),
            ):
                first = control.native_maven_environment_digest()
                artifact.write_bytes(b"second")
                second = control.native_maven_environment_digest()
            self.assertNotEqual(first, second)

    def test_maven_jvm_user_home_override_is_an_effective_authority(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            override = root / "maven-home"
            with mock.patch.dict(
                os.environ,
                {"MAVEN_OPTS": f"-Duser.home={override}"},
                clear=False,
            ):
                _repositories, _settings, homes = control.maven_external_configuration(
                    None, root
                )
            self.assertEqual(homes, [override])

    def test_maven_settings_user_home_placeholder_binds_overridden_repository(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            tools = root / "tools"
            jdk = root / "jdk"
            maven = root / "maven"
            override = root / "overridden-home"
            repository = override / "repository"
            tools.mkdir()
            (jdk / "bin").mkdir(parents=True)
            (maven / "lib").mkdir(parents=True)
            (override / ".m2").mkdir(parents=True)
            repository.mkdir()
            paths = {
                "java": jdk / "bin" / "java",
                "mvn": tools / "mvn",
            }
            for name, path in paths.items():
                path.write_bytes(name.encode())
                os.chmod(path, 0o700)
            (maven / "lib" / "maven-core.jar").write_bytes(b"distribution")
            (override / ".m2" / "settings.xml").write_text(
                "<settings><localRepository>${user.home}/repository</localRepository></settings>",
                encoding="utf-8",
            )
            artifact = repository / "artifact.jar"
            artifact.write_bytes(b"first")

            def which(name: str) -> str:
                return str(paths[name])

            def observe(command, **_kwargs):
                if str(command[0]) == str(paths["java"].resolve()):
                    output = f"java.home = {jdk}\nuser.home = {root}\njava.version = 21\n"
                else:
                    output = f"Apache Maven 3.9\nMaven home: {maven}\n"
                return subprocess.CompletedProcess(command, 0, stdout=b"", stderr=output.encode())

            with (
                mock.patch.dict(
                    os.environ,
                    {
                        "HOME": str(root),
                        "JAVA_HOME": str(jdk),
                        "MAVEN_HOME": str(maven),
                        "MAVEN_OPTS": f"-Duser.home={override}",
                        "PATH": str(tools),
                    },
                    clear=True,
                ),
                mock.patch.object(control.shutil, "which", side_effect=which),
                mock.patch.object(control.subprocess, "run", side_effect=observe),
                mock.patch.object(control, "qualification_tool_authority", return_value={}),
                mock.patch.object(control, "native_python_runtime_authority", return_value={}),
            ):
                first = control.native_maven_environment_digest()
                artifact.write_bytes(b"second")
                second = control.native_maven_environment_digest()
            self.assertNotEqual(first, second)

    def test_q1_preparation_runs_before_reusable_check_evidence(self) -> None:
        model = copy.deepcopy(self.model)
        model["steps"]["Q1"]["dependencies"] = []
        events = []

        def invoke(check, _tier, _authority):
            events.append(("invoke", check["id"]))
            return 0, 1, "sha256:" + "1" * 64, "sha256:" + "2" * 64

        def verified(*_arguments):
            events.append(("verify", "q1-provider-models"))
            return {"status": "PASS"}

        with (
            mock.patch.object(control, "physical_cores", return_value=128),
            mock.patch.object(control, "memory_bytes", return_value=2**50),
            mock.patch.object(control, "is_clean", return_value=True),
            mock.patch.object(control, "dynamic_authority_digest", return_value="sha256:" + "3" * 64),
            mock.patch.object(control, "evidence_digest", return_value="sha256:" + "4" * 64),
            mock.patch.object(control, "authorities", return_value=self.authority),
            mock.patch.object(control, "invoke", side_effect=invoke),
            mock.patch.object(control, "verified_receipt", side_effect=verified),
        ):
            result = control.run_check(
                self.plan, model, self.authority, "Q1", "q1-provider-models"
            )
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(
            events,
            [("invoke", "q1-provider-models-prepare"), ("verify", "q1-provider-models")],
        )

    def test_failed_q1_preparation_refuses_blind_retry(self) -> None:
        model = copy.deepcopy(self.model)
        model["steps"]["Q1"]["dependencies"] = []
        attempts = 0

        def invoke(*_arguments):
            nonlocal attempts
            attempts += 1
            return 9, 1, "sha256:" + "1" * 64, "sha256:" + "2" * 64

        patches = (
            mock.patch.object(control, "physical_cores", return_value=128),
            mock.patch.object(control, "memory_bytes", return_value=2**50),
            mock.patch.object(control, "is_clean", return_value=True),
            mock.patch.object(control, "dynamic_authority_digest", return_value="sha256:" + "3" * 64),
            mock.patch.object(control, "evidence_digest", return_value="sha256:" + "4" * 64),
            mock.patch.object(control, "invoke", side_effect=invoke),
        )
        with patches[0], patches[1], patches[2], patches[3], patches[4], patches[5]:
            with self.assertRaisesRegex(control.ControlError, "preparation failed"):
                control.run_check(
                    self.plan, model, self.authority, "Q1", "q1-provider-models"
                )
            with self.assertRaisesRegex(control.ControlError, "blind retry refused"):
                control.run_check(
                    self.plan, model, self.authority, "Q1", "q1-provider-models"
                )
        self.assertEqual(attempts, 1)

    def test_trusted_seed_dynamic_authority_is_content_bound(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            lifecycle_locks = root / "locks"
            lifecycle_locks.mkdir(mode=0o700)
            lifecycle = lifecycle_locks / "lifecycle.lock"
            lifecycle.write_bytes(b"")
            os.chmod(lifecycle, 0o600)
            epoch = root / ("release-N-" + control.git("rev-parse", "HEAD"))
            runtime_key = "sha256:" + "2" * 64
            capsule = epoch / "parallel-state" / "v2" / "runtimes" / runtime_key[7:]
            capsule.mkdir(parents=True)
            os.chmod(epoch, 0o700)
            os.chmod(epoch / "parallel-state", 0o700)
            os.chmod(epoch / "parallel-state" / "v2", 0o700)
            os.chmod(epoch / "parallel-state" / "v2" / "runtimes", 0o700)
            core = capsule / "bin" / "clew"
            worker_file = capsule / "workers" / "kotlin24" / "worker.jar"
            core.parent.mkdir()
            worker_file.parent.mkdir(parents=True)
            core.write_bytes(b"core")
            worker_file.write_bytes(b"worker")
            os.chmod(core, 0o755)
            worker_row = {
                "mode": 0,
                "path": "worker.jar",
                "sha256": control.digest_bytes(b"worker"),
                "size": len(b"worker"),
            }
            manifest = {
                "artifactIds": ["clew"],
                "artifacts": {
                    "clew": {
                        "mode": 0o111,
                        "path": "bin/clew",
                        "sha256": control.digest_bytes(b"core"),
                        "size": len(b"core"),
                    }
                },
                "components": {"clew": "sha256:" + "3" * 64},
                "inputDigest": "sha256:" + "8" * 64,
                "manifestDigest": "",
                "mode": "RELEASE",
                "platformAuthority": {},
                "runtimeKey": runtime_key,
                "schema": "codeclew-runtime-capsule/4.0",
                "toolchainAuthority": {},
                "workerIds": ["kotlin24"],
                "workers": {
                    "kotlin24": {
                        "compilerVersion": "2.4.10",
                        "distribution": "workers/kotlin24",
                        "files": [worker_row],
                        "protocol": "semantic-thread.worker.v1",
                        "treeHash": control.runtime_tree_hash([worker_row]),
                    }
                },
            }
            manifest["manifestDigest"] = control.digest_bytes(
                control.canonical(manifest)
            )
            (capsule / "runtime.json").write_text(
                json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n",
                encoding="utf-8",
            )
            (capsule / "READY").write_text(runtime_key + "\n", encoding="ascii")
            seed = {
                "artifactHashes": {
                    "clew": manifest["artifacts"]["clew"]["sha256"]
                },
                "buildEvidenceDigests": ["sha256:" + "6" * 64],
                "manifestDigest": manifest["manifestDigest"],
                "mode": "RELEASE",
                "runtimeKey": runtime_key,
                "schema": "codeclew-trusted-release-seed/1.0",
                "sourceRevision": control.git("rev-parse", "HEAD"),
                "sourceTree": control.git("rev-parse", "HEAD^{tree}"),
                "stateEpoch": "sha256:" + "7" * 64,
                "workerTreeHashes": {
                    "kotlin24": manifest["workers"]["kotlin24"]["treeHash"]
                },
            }
            seed["seedDigest"] = control.digest_bytes(control.canonical(seed))
            seed_path = epoch / "seed.json"
            seed_path.write_text(
                json.dumps(seed, sort_keys=True, separators=(",", ":")) + "\n",
                encoding="utf-8",
            )
            os.chmod(seed_path, 0o400)
            locator = {
                "epoch": epoch.name,
                "generation": 1,
                "publicationDigest": "",
                "rollback": None,
                "runtimeKey": runtime_key,
                "schema": "codeclew-trusted-seed-locator/2.0",
                "seedDigest": seed["seedDigest"],
            }
            publication_unsigned = dict(locator)
            publication_unsigned.pop("publicationDigest")
            publication_unsigned["schema"] = (
                "codeclew-trusted-seed-publication/1.0"
            )
            locator["publicationDigest"] = control.digest_bytes(
                control.canonical(publication_unsigned)
            )
            publication = dict(locator)
            publication["schema"] = "codeclew-trusted-seed-publication/1.0"
            publication_path = epoch / "publication.json"
            publication_path.write_text(
                json.dumps(publication, sort_keys=True, separators=(",", ":"))
                + "\n",
                encoding="utf-8",
            )
            os.chmod(publication_path, 0o400)
            locator_path = root / "current.json"
            locator_path.write_text(
                json.dumps(locator, sort_keys=True, separators=(",", ":")) + "\n",
                encoding="utf-8",
            )
            os.chmod(locator_path, 0o600)
            for path in sorted(
                capsule.rglob("*"), key=lambda value: len(value.parts), reverse=True
            ):
                metadata = path.lstat()
                if path.is_dir():
                    os.chmod(path, 0o500)
                else:
                    os.chmod(path, 0o500 if metadata.st_mode & 0o111 else 0o400)
            os.chmod(capsule, 0o500)
            with mock.patch.dict(os.environ, {"CODECLEW_SEED_HOME": str(root)}):
                first = control.trusted_seed_authority_digest()
                os.chmod(core, 0o700)
                core.write_bytes(b"CORE")
                os.chmod(core, 0o500)
                with self.assertRaisesRegex(control.ControlError, "trusted seed authority"):
                    control.trusted_seed_authority_digest()
            self.assertTrue(first.startswith("sha256:"))

    def test_completed_steps_never_crosses_an_invalid_dependency(self) -> None:
        valid = {"S0", "S1", "S3", "S4"}
        with mock.patch.object(
            control,
            "valid_completion",
            side_effect=lambda _model, _authority, step: step in valid,
        ):
            self.assertEqual(
                control.completed_steps(self.model, self.authority),
                ["S0", "S1"],
            )


if __name__ == "__main__":
    unittest.main()
