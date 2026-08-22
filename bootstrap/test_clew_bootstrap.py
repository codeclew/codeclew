from __future__ import annotations

import importlib.util
import contextlib
import fcntl
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest
import sys
from unittest import mock


MODULE_PATH = Path(__file__).with_name("clew_bootstrap.py")
SPEC = importlib.util.spec_from_file_location("clew_bootstrap", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
bootstrap = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bootstrap)


class BootstrapAuthorityTest(unittest.TestCase):
    def test_build_group_shutdown_escalates_after_one_shared_deadline(self) -> None:
        first = mock.Mock(pid=101)
        second = mock.Mock(pid=202)
        with (
            mock.patch.object(bootstrap.os, "killpg") as killpg,
            mock.patch.object(
                bootstrap,
                "_process_group_exists",
                side_effect=lambda process_group: process_group == 202,
            ),
        ):
            bootstrap._terminate_process_groups(
                [first, second], grace_seconds=0, kill_wait_seconds=0
            )
        self.assertEqual(
            killpg.call_args_list,
            [
                mock.call(101, bootstrap.signal.SIGTERM),
                mock.call(202, bootstrap.signal.SIGTERM),
                mock.call(202, bootstrap.signal.SIGKILL),
            ],
        )
        first.wait.assert_called_once_with(timeout=0)
        second.wait.assert_called_once_with(timeout=0)

    def test_build_stage_exception_cancels_its_registered_process_group(self) -> None:
        process = mock.Mock(pid=303)
        process.poll.side_effect = RuntimeError("poll failed")
        supervisor = bootstrap.BuildProcessSupervisor()
        with (
            mock.patch.object(bootstrap.subprocess, "Popen", return_value=process) as popen,
            mock.patch.object(bootstrap, "_terminate_process_groups") as terminate,
            mock.patch.object(bootstrap, "progress"),
        ):
            with self.assertRaisesRegex(RuntimeError, "poll failed"):
                bootstrap.run_build_stage(
                    ["build-tool"], Path("/"), {}, "TEST_STAGE", supervisor
                )
        self.assertTrue(supervisor.cancelled())
        terminate.assert_called_once_with([process])
        self.assertTrue(popen.call_args.kwargs["start_new_session"])

    def test_pre_cancelled_build_never_spawns_a_process(self) -> None:
        supervisor = bootstrap.BuildProcessSupervisor()
        supervisor.request_cancel()
        with mock.patch.object(
            bootstrap.subprocess,
            "Popen",
            side_effect=AssertionError("cancelled stage spawned a process"),
        ):
            with self.assertRaisesRegex(bootstrap.BootstrapError, "cancelled"):
                bootstrap.run_build_stage(
                    ["build-tool"], Path("/"), {}, "TEST_STAGE", supervisor
                )

    def test_process_registered_after_cancel_is_stopped_immediately(self) -> None:
        supervisor = bootstrap.BuildProcessSupervisor()
        supervisor.request_cancel()
        process = mock.Mock(pid=404)
        with mock.patch.object(bootstrap, "_terminate_process_groups") as terminate:
            with self.assertRaisesRegex(bootstrap.BootstrapError, "cancelled"):
                supervisor.register(process)
        terminate.assert_called_once_with([process])

    def test_build_signal_requests_cancellation_and_restores_handler(self) -> None:
        supervisor = bootstrap.BuildProcessSupervisor()
        previous = bootstrap.signal.getsignal(bootstrap.signal.SIGTERM)
        with self.assertRaises(bootstrap.BootstrapInterrupted) as raised:
            with bootstrap.build_signal_scope(supervisor):
                handler = bootstrap.signal.getsignal(bootstrap.signal.SIGTERM)
                self.assertTrue(callable(handler))
                handler(bootstrap.signal.SIGTERM, None)
        self.assertEqual(raised.exception.signum, bootstrap.signal.SIGTERM)
        self.assertTrue(supervisor.cancelled())
        self.assertIs(bootstrap.signal.getsignal(bootstrap.signal.SIGTERM), previous)

    def test_parallel_stage_failure_cancels_sibling_before_executor_wait(self) -> None:
        sibling_started = bootstrap.threading.Event()
        sibling_observed_cancel = bootstrap.threading.Event()

        def stage(
            _arguments: list[str],
            _cwd: Path,
            _environment: dict[str, str],
            name: str,
            supervisor: bootstrap.BuildProcessSupervisor,
        ) -> None:
            if name == "CARGO_BINARIES":
                sibling_started.set()
                self.assertTrue(supervisor._cancelled.wait(timeout=1))
                sibling_observed_cancel.set()
                return
            self.assertTrue(sibling_started.wait(timeout=1))
            raise bootstrap.BootstrapError("Gradle failed")

        with (
            mock.patch.object(
                bootstrap,
                "runtime_build_plan",
                return_value={
                    "profile": "PARALLEL",
                    "parallel": True,
                    "cargoWorkers": 2,
                    "gradleWorkers": 2,
                    "inputWorkers": 4,
                    "packageWorkers": 3,
                    "memoryBudgetBytes": 8 * 1024**3,
                },
            ),
            mock.patch.object(bootstrap, "host_memory_bytes", return_value=16 * 1024**3),
            mock.patch.object(bootstrap, "run_build_stage", side_effect=stage),
        ):
            with self.assertRaisesRegex(bootstrap.BootstrapError, "Gradle failed"):
                bootstrap.build_toolchains(Path("/stage"), {})
        self.assertTrue(sibling_observed_cancel.is_set())

    def test_explicit_cold_build_profiles_control_all_parallelism(self) -> None:
        serial = bootstrap.runtime_build_plan(16, 32 * 1024**3, "SERIAL")
        parallel = bootstrap.runtime_build_plan(16, 32 * 1024**3, "PARALLEL")
        self.assertEqual(
            (serial["parallel"], serial["cargoWorkers"], serial["gradleWorkers"]),
            (False, 1, 1),
        )
        self.assertEqual((serial["inputWorkers"], serial["packageWorkers"]), (1, 1))
        self.assertEqual(
            (parallel["parallel"], parallel["cargoWorkers"], parallel["gradleWorkers"]),
            (True, 8, 8),
        )
        self.assertEqual((parallel["inputWorkers"], parallel["packageWorkers"]), (8, 3))
        with self.assertRaisesRegex(bootstrap.BootstrapError, "cannot admit"):
            bootstrap.runtime_build_plan(1, 4 * 1024**3, "PARALLEL")

    def test_real_cold_profiles_use_exact_cargo_and_gradle_argv(self) -> None:
        observed: list[tuple[str, list[str]]] = []

        def record(
            arguments: list[str],
            _cwd: Path,
            _environment: dict[str, str],
            name: str,
            _supervisor: bootstrap.BuildProcessSupervisor,
        ) -> None:
            observed.append((name, arguments))

        serial = bootstrap.runtime_build_plan(8, 32 * 1024**3, "SERIAL")
        with mock.patch.object(bootstrap, "run_build_stage", side_effect=record):
            bootstrap.build_toolchains(Path("/stage"), {}, serial)
        self.assertEqual([name for name, _ in observed], ["GRADLE_WORKERS", "CARGO_BINARIES"])
        gradle, cargo = (arguments for _, arguments in observed)
        self.assertNotIn("--parallel", gradle)
        self.assertIn("--no-build-cache", gradle)
        self.assertIn("--max-workers=1", gradle)
        self.assertEqual(cargo[-2:], ["--jobs", "1"])

        observed.clear()
        parallel = bootstrap.runtime_build_plan(8, 32 * 1024**3, "PARALLEL")
        with mock.patch.object(bootstrap, "run_build_stage", side_effect=record):
            bootstrap.build_toolchains(Path("/stage"), {}, parallel)
        self.assertEqual({name for name, _ in observed}, {"GRADLE_WORKERS", "CARGO_BINARIES"})
        gradle = next(arguments for name, arguments in observed if name == "GRADLE_WORKERS")
        cargo = next(arguments for name, arguments in observed if name == "CARGO_BINARIES")
        self.assertIn("--parallel", gradle)
        self.assertIn("--no-build-cache", gradle)
        self.assertIn("--max-workers=4", gradle)
        self.assertEqual(cargo[-2:], ["--jobs", "4"])

    def test_concurrent_runtime_lock_admits_exactly_one_publication(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            lock_path = root / "runtime.lock"
            publication = root / "READY"
            builders = root / "builders"
            children = []
            for _ in range(4):
                child = os.fork()
                if child == 0:
                    try:
                        with lock_path.open("a+b") as lock:
                            fcntl.flock(lock, fcntl.LOCK_EX)
                            if not publication.exists():
                                with builders.open("ab") as stream:
                                    stream.write(b"build\n")
                                    stream.flush()
                                    os.fsync(stream.fileno())
                                publication.write_text("ready\n")
                        os._exit(0)
                    except Exception:
                        os._exit(1)
                children.append(child)
            statuses = [os.waitpid(child, 0)[1] for child in children]
            self.assertTrue(all(os.waitstatus_to_exitcode(value) == 0 for value in statuses))
            self.assertEqual(builders.read_text().splitlines(), ["build"])

    def test_corruption_quarantine_self_test(self) -> None:
        bootstrap.bootstrap_self_test()

    def test_selected_closure_excludes_root_and_nested_legacy_state(self) -> None:
        self.assertFalse(bootstrap.selected_source(".semantic-thread/private"))
        self.assertFalse(
            bootstrap.selected_source("crates/clew/src/.semantic-thread/private.rs")
        )

    def test_warm_locator_never_hashes_or_executes_toolchains(self) -> None:
        with (
            mock.patch.object(
                bootstrap,
                "digest_file",
                side_effect=AssertionError("warm locator hashed a tool"),
            ),
            mock.patch.object(
                bootstrap,
                "run",
                side_effect=AssertionError("warm locator executed a tool"),
            ),
        ):
            authority = bootstrap.fast_toolchain_locator_authority()
        self.assertEqual(set(authority["executables"]), {"cargo", "java", "rustc"})

    def test_metadata_checkpoint_warm_path_never_runs_or_hashes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "source"
            state = root / "state"
            capsule = state / "runtimes" / ("1" * 64)
            checkpoint_directory = state / "runtimes" / "checkpoints"
            source.mkdir()
            capsule.mkdir(parents=True)
            checkpoint_directory.mkdir()
            subprocess.run(["git", "init", "-q"], cwd=source, check=True)
            (source / "Cargo.toml").write_text("[workspace]\n")
            subprocess.run(["git", "add", "Cargo.toml"], cwd=source, check=True)
            artifact = capsule / "clew"
            artifact.write_bytes(b"capsule")
            source_file = source / "Cargo.toml"
            source_metadata = source_file.stat()
            inputs = [{
                "path": "Cargo.toml",
                "size": source_metadata.st_size,
                "mode": source_metadata.st_mode & 0o111,
                "sha256": "sha256:" + "0" * 64,
            }]
            executable = Path(os.sys.executable).resolve()
            fast_tools = {
                "python": {"path": str(executable)},
                "executables": {
                    "cargo": {"path": str(executable)},
                    "java": {"path": str(executable)},
                    "rustc": {"path": str(executable)},
                },
                "jdkRelease": {"path": str(executable)},
            }
            path = bootstrap.checkpoint_path(state, source)
            bootstrap.write_checkpoint(
                path,
                source,
                capsule,
                "sha256:" + "1" * 64,
                "RELEASE",
                inputs,
                fast_tools,
            )
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            bootstrap.reset_audit_counters()
            with (
                mock.patch.object(
                    bootstrap,
                    "digest_file",
                    side_effect=AssertionError("warm checkpoint hashed bytes"),
                ),
                mock.patch.object(
                    bootstrap,
                    "run",
                    side_effect=AssertionError("warm checkpoint ran a process"),
                ),
            ):
                value = bootstrap.read_valid_checkpoint(path, source, state)
            self.assertIsNotNone(value)
            self.assertEqual(bootstrap._AUDIT_COUNTERS["processRuns"], 0)
            self.assertEqual(bootstrap._AUDIT_COUNTERS["digestFileCalls"], 0)
            self.assertGreater(bootstrap._AUDIT_COUNTERS["metadataChecks"], 0)
            (state / "locks").mkdir()
            state_descriptor = os.open(state, os.O_RDONLY | os.O_DIRECTORY)
            output = io.StringIO()
            try:
                with (
                    mock.patch.object(
                        bootstrap,
                        "digest_file",
                        side_effect=AssertionError("warm main hashed bytes"),
                    ),
                    mock.patch.object(
                        bootstrap,
                        "run",
                        side_effect=AssertionError("warm main ran a process"),
                    ),
                    mock.patch.object(
                        bootstrap,
                        "state_root",
                        return_value=(state, state_descriptor),
                    ),
                    mock.patch.object(
                        bootstrap,
                        "garbage_collect_runtime_capsules",
                        side_effect=AssertionError("checkpoint hit scanned runtime GC roots"),
                    ),
                    mock.patch.object(
                        sys,
                        "argv",
                        [
                            "clew_bootstrap.py",
                            "--source-root",
                            str(source),
                            "--bootstrap-warm-audit",
                        ],
                    ),
                    contextlib.redirect_stdout(output),
                ):
                    self.assertEqual(bootstrap.main(), 0)
            finally:
                os.close(state_descriptor)
            audit = json.loads(output.getvalue())
            self.assertEqual(audit["status"], "PASSED")
            self.assertEqual(audit["counters"]["processRuns"], 0)
            self.assertEqual(audit["counters"]["digestFileCalls"], 0)
            self.assertGreaterEqual(audit["counters"]["checkpointHits"], 1)
            malformed = json.loads(path.read_bytes())
            malformed["runtimeKey"] = "sha256:../../outside"
            malformed["capsule"] = str(root / "outside")
            path.write_bytes(bootstrap.canonical(malformed) + b"\n")
            os.chmod(path, 0o600)
            with mock.patch.object(
                bootstrap,
                "_metadata_matches",
                side_effect=AssertionError("malformed key reached metadata paths"),
            ):
                self.assertIsNone(
                    bootstrap.read_valid_checkpoint(path, source, state)
                )
            self.assertIsNone(bootstrap.read_checkpoint_candidate_key(path, state))

    def test_metadata_checkpoint_invalidates_source_and_capsule_mutations(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "source"
            state = root / "state"
            capsule = state / "runtimes" / ("2" * 64)
            (state / "runtimes" / "checkpoints").mkdir(parents=True)
            source.mkdir()
            capsule.mkdir(parents=True)
            subprocess.run(["git", "init", "-q"], cwd=source, check=True)
            source_file = source / "Cargo.toml"
            source_file.write_text("[workspace]\n")
            subprocess.run(["git", "add", "Cargo.toml"], cwd=source, check=True)
            artifact = capsule / "clew"
            artifact.write_bytes(b"capsule")
            executable = Path(os.sys.executable).resolve()
            fast_tools = {
                "python": {"path": str(executable)},
                "executables": {
                    "cargo": {"path": str(executable)},
                    "java": {"path": str(executable)},
                    "rustc": {"path": str(executable)},
                },
                "jdkRelease": {"path": str(executable)},
            }
            inputs = [{
                "path": "Cargo.toml",
                "size": source_file.stat().st_size,
                "mode": 0,
                "sha256": "sha256:" + "0" * 64,
            }]
            path = bootstrap.checkpoint_path(state, source)
            bootstrap.write_checkpoint(
                path, source, capsule, "sha256:" + "2" * 64,
                "DEVELOPMENT", inputs, fast_tools,
            )
            source_file.write_text("[workspace]\nmembers=[]\n")
            self.assertIsNone(bootstrap.read_valid_checkpoint(path, source, state))
            bootstrap.write_checkpoint(
                path, source, capsule, "sha256:" + "2" * 64,
                "DEVELOPMENT", inputs, fast_tools,
            )
            subprocess.run(["git", "add", "Cargo.toml"], cwd=source, check=True)
            self.assertIsNone(bootstrap.read_valid_checkpoint(path, source, state))
            bootstrap.write_checkpoint(
                path, source, capsule, "sha256:" + "2" * 64,
                "DEVELOPMENT", inputs, fast_tools,
            )
            subprocess.run(
                [
                    "git", "-c", "user.name=Codeclew Tests",
                    "-c", "user.email=tests@codeclew.invalid",
                    "commit", "-qm", "clean transition",
                ],
                cwd=source,
                check=True,
            )
            self.assertIsNone(bootstrap.read_valid_checkpoint(path, source, state))
            bootstrap.write_checkpoint(
                path, source, capsule, "sha256:" + "2" * 64,
                "RELEASE", inputs, fast_tools,
            )
            added = source / "crates" / "new" / "src" / "lib.rs"
            added.parent.mkdir(parents=True)
            added.write_text("pub fn added() {}\n")
            self.assertIsNone(bootstrap.read_valid_checkpoint(path, source, state))
            bootstrap.write_checkpoint(
                path, source, capsule, "sha256:" + "2" * 64,
                "DEVELOPMENT", inputs, fast_tools,
            )
            added.unlink()
            self.assertIsNone(bootstrap.read_valid_checkpoint(path, source, state))
            bootstrap.write_checkpoint(
                path, source, capsule, "sha256:" + "2" * 64,
                "DEVELOPMENT", inputs, fast_tools,
            )
            artifact.write_bytes(b"corrupt")
            self.assertIsNone(bootstrap.read_valid_checkpoint(path, source, state))
            bootstrap.write_checkpoint(
                path, source, capsule, "sha256:" + "2" * 64,
                "DEVELOPMENT", inputs, fast_tools,
            )
            (capsule / "unexpected").write_bytes(b"extra")
            self.assertIsNone(bootstrap.read_valid_checkpoint(path, source, state))

    def test_manifest_rechecks_full_closure_without_reading_legacy_state(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            (root / "Cargo.toml").write_text("[workspace]\n")
            source = root / "crates/example/src"
            source.mkdir(parents=True)
            (source / "lib.rs").write_text("pub fn one() {}\n")
            subprocess.run(
                ["git", "add", "Cargo.toml", "crates/example/src/lib.rs"],
                cwd=root,
                check=True,
            )
            legacy = root / ".semantic-thread"
            nested_legacy = source / ".semantic-thread"
            legacy.mkdir()
            nested_legacy.mkdir()
            (legacy / "poison").write_text("private")
            (nested_legacy / "poison.rs").write_text("private")
            try:
                legacy.chmod(0)
                nested_legacy.chmod(0)
                rows, _ = bootstrap.source_manifest(root)
                bootstrap.verify_source_manifest(root, rows)
                (source / "new.rs").write_text("pub fn two() {}\n")
                with self.assertRaisesRegex(
                    bootstrap.BootstrapError, "closure changed"
                ):
                    bootstrap.verify_source_manifest(root, rows)
            finally:
                legacy.chmod(stat.S_IRWXU)
                nested_legacy.chmod(stat.S_IRWXU)

    def test_executable_authority_is_independent_of_checkout_umask(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "source"
            stage = root / "stage"
            bootstrap_file = source / "bootstrap" / "clew_bootstrap.py"
            bootstrap_file.parent.mkdir(parents=True)
            bootstrap_file.write_text("#!/usr/bin/env python3\n")
            bootstrap_file.chmod(0o700)
            subprocess.run(["git", "init", "-q"], cwd=source, check=True)
            subprocess.run(
                ["git", "add", "bootstrap/clew_bootstrap.py"],
                cwd=source,
                check=True,
            )
            subprocess.run(
                [
                    "git", "-c", "user.name=Codeclew Tests",
                    "-c", "user.email=tests@codeclew.invalid",
                    "commit", "-qm", "fixture",
                ],
                cwd=source,
                check=True,
            )

            rows, development = bootstrap.source_manifest(source)
            self.assertFalse(development)
            self.assertEqual(rows[0]["mode"], 0o111)
            bootstrap.stage_inputs(source, stage, rows, workers=1)
            self.assertEqual(stat.S_IMODE(bootstrap_file.stat().st_mode), 0o700)
            self.assertEqual(
                stat.S_IMODE((stage / "bootstrap/clew_bootstrap.py").stat().st_mode),
                0o700,
            )

            bootstrap_file.chmod(0o755)
            bootstrap.verify_source_manifest(source, rows)
            bootstrap_file.chmod(0o600)
            with self.assertRaisesRegex(bootstrap.BootstrapError, "changed during bootstrap"):
                bootstrap.verify_source_manifest(source, rows)

    def test_build_outputs_are_private_and_injection_environment_is_removed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            stage = root / "source"
            stage.mkdir()
            gradle_home = root / "gradle-home"
            gradle_home.mkdir()
            with mock.patch.dict(
                os.environ,
                {
                    "RUSTFLAGS": "--cfg injected",
                    "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER": "injected",
                    "JAVA_TOOL_OPTIONS": "-javaagent:injected",
                    "GRADLE_OPTS": "-I injected.gradle",
                    "GRADLE_USER_HOME": str(gradle_home),
                },
                clear=False,
            ):
                environment = bootstrap.build_environment(stage, root)
            self.assertEqual(Path(environment["CARGO_TARGET_DIR"]), root / "cargo-target")
            self.assertEqual(environment["CARGO_INCREMENTAL"], "0")
            self.assertNotIn("RUSTFLAGS", environment)
            remaps = environment["CARGO_ENCODED_RUSTFLAGS"].split("\x1f")
            self.assertEqual(
                remaps[:3],
                [
                    f"--remap-path-prefix={Path.home()}=/codeclew/home",
                    f"--remap-path-prefix={root}=/codeclew/build",
                    f"--remap-path-prefix={stage}=/codeclew/source",
                ],
            )
            self.assertEqual(remaps[3:], [])
            self.assertNotIn(
                "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER", environment
            )
            self.assertNotIn("JAVA_TOOL_OPTIONS", environment)
            self.assertNotIn("GRADLE_OPTS", environment)

    def test_state_root_is_preopened_private_and_rejects_symlink_component(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory).resolve()
            with mock.patch.dict(
                os.environ, {"CODECLEW_HOME": str(parent / "state")}, clear=False
            ):
                root, descriptor = bootstrap.state_root()
            try:
                self.assertEqual(stat.S_IMODE(os.fstat(descriptor).st_mode), 0o700)
                self.assertEqual(root, parent / "state/v2")
            finally:
                os.close(descriptor)
            (parent / "real").mkdir()
            (parent / "link").symlink_to(parent / "real", target_is_directory=True)
            with self.assertRaisesRegex(
                bootstrap.BootstrapError, "physical normalized CODECLEW_HOME"
            ):
                bootstrap._open_private_tree(parent / "link/child")

    def test_cold_build_capacity_fails_before_build_tools_start(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            with mock.patch.object(
                bootstrap.shutil,
                "disk_usage",
                return_value=mock.Mock(free=bootstrap.MIN_COLD_BUILD_FREE_BYTES - 1),
            ):
                with self.assertRaisesRegex(
                    bootstrap.BootstrapError, "cold runtime build requires at least"
                ):
                    bootstrap.require_cold_build_capacity(root)

    def test_runtime_gc_retains_leases_and_two_newest_capsules(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            runtimes = root / "runtimes"
            locks = root / "locks"
            locators = runtimes / "locators"
            checkpoints = runtimes / "checkpoints"
            for path in [locks, locators, checkpoints]:
                path.mkdir(mode=0o700, parents=True, exist_ok=True)

            names = {
                "current": "1" * 64,
                "newest": "2" * 64,
                "second_newest": "3" * 64,
                "leased": "4" * 64,
                "removable": "5" * 64,
                "session": "8" * 64,
            }
            timestamps = {
                "current": 1,
                "newest": 5,
                "second_newest": 4,
                "leased": 3,
                "removable": 2,
                "session": 0,
            }
            for label, name in names.items():
                capsule = runtimes / name
                capsule.mkdir(mode=0o700)
                os.utime(capsule, ns=(timestamps[label], timestamps[label]))
                capsule.chmod(0o500)

            outside = root / "outside"
            outside.mkdir()
            poison = outside / "poison"
            poison.write_text("do not remove")
            removable_capsule = runtimes / names["removable"]
            removable_capsule.chmod(0o700)
            removable_nested = removable_capsule / "nested"
            removable_nested.mkdir()
            (removable_nested / "artifact").write_text("derived")
            (removable_nested / "external").symlink_to(poison)
            removable_nested.chmod(0o500)
            removable_capsule.chmod(0o500)
            os.utime(
                removable_capsule,
                ns=(timestamps["removable"], timestamps["removable"]),
            )
            symlink_name = "6" * 64
            (runtimes / symlink_name).symlink_to(outside, target_is_directory=True)

            removable_key = "sha256:" + names["removable"]
            for path in [locators / "old.json", checkpoints / "old.json"]:
                path.write_bytes(bootstrap.canonical({"runtimeKey": removable_key}) + b"\n")
                path.chmod(0o600)

            session_name = "gc-root"
            session_id = f"session:{session_name}"
            session = root / "sessions" / session_name
            session.mkdir(mode=0o700, parents=True)
            authority = session / "authority.json"
            authority.write_bytes(bootstrap.canonical({
                "schema": "codeclew-session/3.0",
                "sessionId": session_id,
                "authorityDigest": "sha256:" + "9" * 64,
                "runtimeKey": "sha256:" + names["session"],
            }) + b"\n")
            authority.chmod(0o600)

            leased_path = locks / f"runtime-{names['leased']}.lease"
            with leased_path.open("a+b") as leased:
                fcntl.flock(leased, fcntl.LOCK_SH)
                removed = bootstrap.garbage_collect_runtime_capsules(
                    root, "sha256:" + names["current"]
                )

            self.assertEqual(removed, [removable_key])
            for label in ["current", "newest", "second_newest", "leased", "session"]:
                self.assertTrue((runtimes / names[label]).is_dir())
            self.assertFalse((runtimes / names["removable"]).exists())
            self.assertFalse((locators / "old.json").exists())
            self.assertFalse((checkpoints / "old.json").exists())
            self.assertTrue((runtimes / symlink_name).is_symlink())
            self.assertEqual(poison.read_text(), "do not remove")

            stale_key = "sha256:" + "7" * 64
            stale_locator = locators / "stale.json"
            stale_locator.write_bytes(bootstrap.canonical({
                "schema": "codeclew-runtime-locator/2.0",
                "locatorKey": "locator",
                "runtimeKey": stale_key,
            }) + b"\n")
            self.assertIsNone(
                bootstrap.read_locator(stale_locator, "locator", root)
            )
            stale_checkpoint = checkpoints / "stale.json"
            stale_checkpoint.write_bytes(bootstrap.canonical({
                "schema": "codeclew-runtime-checkpoint/3.0",
                "runtimeKey": stale_key,
                "capsule": str(runtimes / ("7" * 64)),
            }) + b"\n")
            stale_checkpoint.chmod(0o600)
            self.assertIsNone(
                bootstrap.read_checkpoint_candidate_key(stale_checkpoint, root)
            )

    def test_session_runtime_root_uses_uuid_directory_and_exact_terminal_lifecycle(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            session_name = "01234567-89ab-cdef-0123-456789abcdef"
            session_id = f"session:{session_name}"
            session = root / "sessions" / session_name
            session.mkdir(mode=0o700, parents=True)
            runtime_key = "sha256:" + "8" * 64
            authority_digest = "sha256:" + "9" * 64
            (session / "authority.json").write_bytes(bootstrap.canonical({
                "schema": "codeclew-session/3.0",
                "sessionId": session_id,
                "authorityDigest": authority_digest,
                "runtimeKey": runtime_key,
            }) + b"\n")

            # Missing lifecycle fails open and retains the runtime.
            self.assertEqual(
                bootstrap._session_runtime_roots(root),
                {runtime_key.removeprefix("sha256:")},
            )

            lifecycle = {
                "schema": "codeclew-session-lifecycle-entry/1.0",
                "sessionId": session_id,
                "sessionAuthorityDigest": authority_digest,
                "sequence": 2,
                "previousEventHash": "sha256:" + "7" * 64,
                "status": "GARBAGE_COLLECTED",
                "eventHash": "",
                "updatedUnixMs": 1,
            }
            lifecycle["eventHash"] = bootstrap.digest_bytes(bootstrap.canonical(lifecycle))
            (session / "lifecycle.json").write_bytes(bootstrap.canonical(lifecycle))
            self.assertEqual(bootstrap._session_runtime_roots(root), set())

            lifecycle["status"] = "ABORTED"
            (session / "lifecycle.json").write_bytes(bootstrap.canonical(lifecycle))
            self.assertEqual(
                bootstrap._session_runtime_roots(root),
                {runtime_key.removeprefix("sha256:")},
            )


if __name__ == "__main__":
    unittest.main()
