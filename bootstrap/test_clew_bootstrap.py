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
import time
import unittest
import sys
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import host_resources


MODULE_PATH = Path(__file__).with_name("clew_bootstrap.py")
SPEC = importlib.util.spec_from_file_location("clew_bootstrap", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
bootstrap = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bootstrap)


def write_minimal_registry(
    source: Path, *, input_files: list[str], input_roots: list[str]
) -> None:
    registry = {
        "components": [{
            "buildContract": {
                "artifactName": "clew",
                "binary": "clew",
                "executor": "CARGO",
                "package": "clew",
            },
            "componentId": "clew",
            "componentKind": "core-binary",
            "inputFiles": sorted(input_files),
            "inputRoots": sorted(input_roots),
            "optionalInputRoots": [],
            "toolchainKeys": ["platform", "rust"],
        }],
        "schema": bootstrap.COMPONENT_REGISTRY_SCHEMA,
    }
    target = source / "bootstrap/runtime_components.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(bootstrap.canonical(registry) + b"\n")


class BootstrapAuthorityTest(unittest.TestCase):
    def test_session_cleanup_uses_its_retained_capsule_without_source_bootstrap(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            session_name = "01234567-89ab-cdef-0123-456789abcdef"
            session_id = f"session:{session_name}"
            runtime_key = "sha256:" + "8" * 64
            session = root / "sessions" / session_name
            capsule = root / "runtimes" / runtime_key.removeprefix("sha256:")
            locks = root / "locks"
            session.mkdir(mode=0o700, parents=True)
            capsule.mkdir(mode=0o500, parents=True)
            locks.mkdir(mode=0o700)
            authority = session / "authority.json"
            authority.write_bytes(bootstrap.canonical({
                "schema": "codeclew-session/5.0",
                "sessionId": session_id,
                "authorityDigest": "sha256:" + "9" * 64,
                "runtimeKey": runtime_key,
                "runtimeMode": "RELEASE",
            }) + b"\n")
            authority.chmod(0o600)

            self.assertEqual(
                bootstrap.cleanup_session_id(
                    ["session", "gc", "--force", "--session", session_id]
                ),
                session_id,
            )
            self.assertIsNone(
                bootstrap.cleanup_session_id(
                    ["session", "inspect", "--session", session_id]
                )
            )
            with mock.patch.object(
                bootstrap,
                "verify_capsule",
                return_value={"mode": "RELEASE"},
            ) as verify:
                key, selected, lease = bootstrap.sealed_session_cleanup_runtime(
                    root, session_id
                )
            try:
                self.assertEqual(key, runtime_key)
                self.assertEqual(selected, capsule)
                verify.assert_called_once_with(capsule, runtime_key)
            finally:
                lease.close()

    def test_session_cleanup_rejects_unsafe_authority_and_runtime_mode(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            session_name = "01234567-89ab-cdef-0123-456789abcdef"
            session_id = f"session:{session_name}"
            session = root / "sessions" / session_name
            session.mkdir(mode=0o700, parents=True)
            (root / "runtimes").mkdir()
            (root / "locks").mkdir()
            authority = session / "authority.json"
            authority.write_bytes(bootstrap.canonical({
                "schema": "codeclew-session/5.0",
                "sessionId": session_id,
                "authorityDigest": "sha256:" + "9" * 64,
                "runtimeKey": "sha256:" + "8" * 64,
                "runtimeMode": "AMBIENT",
            }) + b"\n")
            authority.chmod(0o600)
            with self.assertRaisesRegex(
                bootstrap.BootstrapError, "session cleanup authority is invalid"
            ):
                bootstrap.sealed_session_cleanup_runtime(root, session_id)

    def test_effective_resources_honor_affinity_cpuset_quota_and_memory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            cgroup = root / "cgroup"
            proc = root / "proc"
            cgroup.mkdir()
            (proc / "self").mkdir(parents=True)
            (proc / "self/cgroup").write_text("0::/\n", encoding="ascii")
            (cgroup / "cpu.max").write_text("200000 100000\n", encoding="ascii")
            (cgroup / "cpuset.cpus.effective").write_text("0-5\n", encoding="ascii")
            (cgroup / "memory.max").write_text(str(4 * 1024**3), encoding="ascii")

            def sysconf(name: str) -> int:
                return 4 * 1024**3 if name == "SC_PHYS_PAGES" else 4

            with (
                mock.patch.object(host_resources.os, "cpu_count", return_value=16),
                mock.patch.object(
                    host_resources.os,
                    "sched_getaffinity",
                    return_value=set(range(8)),
                    create=True,
                ),
                mock.patch.object(host_resources.os, "sysconf", side_effect=sysconf),
            ):
                authority = host_resources.effective_host_resources(cgroup, proc)
            self.assertEqual(authority["logicalCores"], 2)
            self.assertEqual(authority["totalMemoryBytes"], 4 * 1024**3)
            self.assertEqual(authority["cpu"]["cpusetCores"], 6)
            self.assertEqual(authority["memory"]["physicalBytes"], 16 * 1024**3)

    def test_effective_resources_take_nested_cgroup_ancestor_minima(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            cgroup = root / "cgroup"
            leaf = cgroup / "user.slice/session.scope"
            leaf.mkdir(parents=True)
            proc = root / "proc"
            (proc / "self").mkdir(parents=True)
            (proc / "self/cgroup").write_text(
                "0::/user.slice/session.scope\n", encoding="ascii"
            )
            (cgroup / "cpu.max").write_text("max 100000\n", encoding="ascii")
            (cgroup / "memory.max").write_text("max\n", encoding="ascii")
            (cgroup / "user.slice/cpu.max").write_text(
                "300000 100000\n", encoding="ascii"
            )
            (cgroup / "user.slice/memory.max").write_text(
                str(6 * 1024**3), encoding="ascii"
            )
            (leaf / "cpu.max").write_text("200000 100000\n", encoding="ascii")
            (leaf / "memory.max").write_text(str(4 * 1024**3), encoding="ascii")
            (leaf / "cpuset.cpus.effective").write_text("2-5\n", encoding="ascii")

            def sysconf(name: str) -> int:
                return 4 * 1024**3 if name == "SC_PHYS_PAGES" else 4

            with (
                mock.patch.object(host_resources.os, "cpu_count", return_value=16),
                mock.patch.object(
                    host_resources.os,
                    "sched_getaffinity",
                    return_value=set(range(8)),
                    create=True,
                ),
                mock.patch.object(host_resources.os, "sysconf", side_effect=sysconf),
            ):
                authority = host_resources.effective_host_resources(cgroup, proc)
            self.assertEqual(authority["logicalCores"], 2)
            self.assertEqual(authority["totalMemoryBytes"], 4 * 1024**3)

    def test_effective_resources_resolve_v1_controller_memberships(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            cgroup = root / "cgroup"
            proc = root / "proc"
            (proc / "self").mkdir(parents=True)
            (proc / "self/cgroup").write_text(
                "2:cpu,cpuacct:/jobs/42\n3:memory:/jobs/42\n4:cpuset:/jobs/42\n",
                encoding="ascii",
            )
            cpu = cgroup / "cpu,cpuacct/jobs/42"
            memory = cgroup / "memory/jobs/42"
            cpuset = cgroup / "cpuset/jobs/42"
            for path in (cpu, memory, cpuset):
                path.mkdir(parents=True)
            (cpu / "cpu.cfs_quota_us").write_text("200000", encoding="ascii")
            (cpu / "cpu.cfs_period_us").write_text("100000", encoding="ascii")
            (memory / "memory.limit_in_bytes").write_text(
                str(3 * 1024**3), encoding="ascii"
            )
            (cpuset / "cpuset.cpus").write_text("0-3", encoding="ascii")

            def sysconf(name: str) -> int:
                return 2 * 1024**3 if name == "SC_PHYS_PAGES" else 8

            with (
                mock.patch.object(host_resources.os, "cpu_count", return_value=16),
                mock.patch.object(
                    host_resources.os,
                    "sched_getaffinity",
                    return_value=set(range(8)),
                    create=True,
                ),
                mock.patch.object(host_resources.os, "sysconf", side_effect=sysconf),
            ):
                authority = host_resources.effective_host_resources(cgroup, proc)
            self.assertEqual(authority["logicalCores"], 2)
            self.assertEqual(authority["totalMemoryBytes"], 3 * 1024**3)

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
        self.assertIs(popen.call_args.kwargs["stderr"], subprocess.DEVNULL)

    def test_build_stage_keeps_tool_output_out_of_typed_telemetry(self) -> None:
        program = (
            "import os\n"
            "os.write(1,b'untyped-stdout\\n')\n"
            "os.write(2,b'untyped-stderr\\n')\n"
        )
        telemetry = io.StringIO()
        with contextlib.redirect_stderr(telemetry):
            bootstrap.run_build_stage(
                [sys.executable, "-c", program],
                Path("/"),
                {},
                "TELEMETRY_TEST",
            )
        rows = [json.loads(line) for line in telemetry.getvalue().splitlines()]
        self.assertEqual(
            [(row["event"], row["stage"]) for row in rows],
            [
                ("STAGE_STARTED", "TELEMETRY_TEST"),
                ("STAGE_COMPLETED", "TELEMETRY_TEST"),
            ],
        )

    @unittest.skipUnless(hasattr(os, "killpg"), "POSIX process groups required")
    def test_successful_build_stage_refuses_and_stops_residual_descendant(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "child.pid"
            program = (
                "import pathlib,subprocess,sys\n"
                "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'])\n"
                "pathlib.Path(sys.argv[1]).write_text(str(child.pid))\n"
            )
            with mock.patch.object(bootstrap, "progress"):
                with self.assertRaisesRegex(
                    bootstrap.BootstrapError, "residual process group"
                ):
                    bootstrap.run_build_stage(
                        [sys.executable, "-c", program, str(marker)],
                        Path(directory),
                        os.environ.copy(),
                        "RESIDUAL_TEST",
                    )
            self.assertTrue(marker.exists())
            child_pid = int(marker.read_text())
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

    def test_discard_private_tree_unlinks_symlink_without_touching_target(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory).resolve()
            outside = base / "outside"
            outside.mkdir(mode=0o700)
            marker = outside / "marker.bin"
            marker.write_bytes(b"private-bytes\x00\xff")
            marker.chmod(0o640)
            outside.chmod(0o750)
            expected_file_mode = stat.S_IMODE(marker.stat().st_mode)
            expected_directory_mode = stat.S_IMODE(outside.stat().st_mode)

            temporary = base / "temporary"
            temporary.mkdir(mode=0o700)
            nested = temporary / "nested"
            nested.mkdir(mode=0o700)
            (nested / "external").symlink_to(marker)

            bootstrap.discard_private_tree(temporary)

            self.assertFalse(temporary.exists())
            self.assertEqual(marker.read_bytes(), b"private-bytes\x00\xff")
            self.assertEqual(stat.S_IMODE(marker.stat().st_mode), expected_file_mode)
            self.assertEqual(
                stat.S_IMODE(outside.stat().st_mode), expected_directory_mode
            )

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
                    "gradleHeapBytes": 3 * 1024**3,
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
                bootstrap.build_toolchains(
                    Path("/stage"), {}, gradle_tasks=[":adapter:installDist"]
                )
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
            (True, 5, 5),
        )
        self.assertEqual((parallel["inputWorkers"], parallel["packageWorkers"]), (8, 8))
        self.assertEqual(parallel["gradleHeapBytes"], 8 * 1024**3)
        non_capped = bootstrap.runtime_build_plan(8, 16 * 1024**3, "PARALLEL")
        self.assertEqual(non_capped["gradleHeapBytes"] % 1024**2, 0)
        self.assertIn(
            f"-Xmx{non_capped['gradleHeapBytes'] // 1024**2}m",
            bootstrap.gradle_jvm_options(non_capped),
        )
        constrained = bootstrap.runtime_build_plan(16, 8 * 1024**3, "AUTO")
        self.assertFalse(constrained["parallel"])
        self.assertEqual(constrained["gradleWorkers"], 1)
        self.assertGreaterEqual(
            constrained["gradleHeapBytes"], bootstrap.GRADLE_MIN_HEAP_BYTES
        )
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
            bootstrap.build_toolchains(
                Path("/stage"), {}, serial, gradle_tasks=[":adapter:installDist"]
            )
        self.assertEqual([name for name, _ in observed], ["GRADLE_WORKERS", "CARGO_BINARIES"])
        gradle, cargo = (arguments for _, arguments in observed)
        self.assertNotIn("--parallel", gradle)
        self.assertIn("--no-watch-fs", gradle)
        self.assertIn("-Dorg.gradle.daemon.idletimeout=1000", gradle)
        self.assertIn(
            "-Dorg.gradle.daemon.registry.base=/.codeclew-gradle-daemon", gradle
        )
        self.assertIn("--no-build-cache", gradle)
        self.assertIn("--offline", gradle)
        self.assertIn("-Pkotlin.compiler.execution.strategy=in-process", gradle)
        self.assertIn("--max-workers=1", gradle)
        self.assertIn("--frozen", cargo)
        self.assertEqual(cargo[-2:], ["--jobs", "1"])

        observed.clear()
        parallel = bootstrap.runtime_build_plan(8, 32 * 1024**3, "PARALLEL")
        with mock.patch.object(bootstrap, "run_build_stage", side_effect=record):
            bootstrap.build_toolchains(
                Path("/stage"), {}, parallel, gradle_tasks=[":adapter:installDist"]
            )
        self.assertEqual({name for name, _ in observed}, {"GRADLE_WORKERS", "CARGO_BINARIES"})
        gradle = next(arguments for name, arguments in observed if name == "GRADLE_WORKERS")
        cargo = next(arguments for name, arguments in observed if name == "CARGO_BINARIES")
        self.assertIn("--parallel", gradle)
        self.assertIn("--no-watch-fs", gradle)
        self.assertIn("--no-build-cache", gradle)
        self.assertIn("--offline", gradle)
        self.assertIn("-Pkotlin.compiler.execution.strategy=in-process", gradle)
        self.assertIn("--max-workers=4", gradle)
        self.assertIn(
            "-Dorg.gradle.jvmargs=-Xms256m -Xmx8192m "
            "-XX:MaxMetaspaceSize=1024m -XX:MaxDirectMemorySize=512m "
            "-XX:+ExitOnOutOfMemoryError",
            gradle,
        )
        self.assertIn("--frozen", cargo)
        self.assertEqual(cargo[-2:], ["--jobs", "4"])

        with self.assertRaisesRegex(bootstrap.BootstrapError, "must be absolute"):
            bootstrap.build_toolchains(
                Path("relative-stage"),
                {},
                serial,
                cargo_required=False,
                gradle_tasks=[":adapter:installDist"],
            )

    def test_component_misses_start_only_the_required_toolchain(self) -> None:
        observed: list[str] = []

        def record(
            _arguments: list[str],
            _cwd: Path,
            _environment: dict[str, str],
            name: str,
            _supervisor: bootstrap.BuildProcessSupervisor,
        ) -> None:
            observed.append(name)

        plan = bootstrap.runtime_build_plan(8, 32 * 1024**3, "PARALLEL")
        with mock.patch.object(bootstrap, "run_build_stage", side_effect=record):
            result = bootstrap.build_toolchains(
                Path("/stage"),
                {},
                plan,
                cargo_required=False,
                gradle_tasks=[":zeta:installDist"],
            )
        self.assertEqual(observed, ["GRADLE_WORKERS"])
        self.assertEqual(result["toolchainStages"], ["GRADLE_WORKERS"])
        self.assertEqual(set(result["stageWallMillis"]), {"GRADLE_WORKERS"})
        self.assertGreaterEqual(result["toolchainWallMillis"], 0)

        observed.clear()
        with mock.patch.object(bootstrap, "run_build_stage", side_effect=record):
            result = bootstrap.build_toolchains(
                Path("/stage"), {}, plan, cargo_required=True, gradle_tasks=[]
            )
        self.assertEqual(observed, ["CARGO_BINARIES"])
        self.assertEqual(result["toolchainStages"], ["CARGO_BINARIES"])
        self.assertEqual(set(result["stageWallMillis"]), {"CARGO_BINARIES"})

    def test_dependency_prime_uses_online_fetch_and_in_process_gradle(self) -> None:
        source = Path("/source")
        root = Path("/state")
        inputs = [{"mode": 0, "path": "Cargo.toml", "sha256": "sha256:" + "1" * 64, "size": 1}]
        tools = {"jdk": {}, "platform": {}, "python": {}, "rust": {}}
        specs = [
            {
                "buildContract": {"executor": "GRADLE", "task": ":zeta:installDist"}
            }
        ]
        observed: list[tuple[str, list[str]]] = []

        def record(arguments, _cwd, _environment, name, _supervisor):
            observed.append((name, arguments))

        with (
            mock.patch.object(bootstrap, "dependency_cache_authority", return_value={
                "artifactIds": ["clew"],
                "componentIds": ["zeta"],
                "inputDigest": "sha256:" + "2" * 64,
                "mode": "RELEASE",
                "runtimeKey": "sha256:" + "3" * 64,
                "schema": "codeclew-dependency-cache-authority/1.0",
                "status": "PASS",
                "toolchainDigest": "sha256:" + "4" * 64,
                "workerIds": ["zeta"],
            }),
            mock.patch.object(bootstrap, "source_manifest", return_value=(inputs, False)),
            mock.patch.object(bootstrap, "load_component_registry", return_value={}),
            mock.patch.object(bootstrap, "toolchain_authority", return_value=tools),
            mock.patch.object(bootstrap, "runtime_component_specs", return_value=specs),
            mock.patch.object(bootstrap.tempfile, "mkdtemp", return_value="/state/tmp/prime"),
            mock.patch.object(bootstrap, "runtime_build_plan", return_value={
                "cargoWorkers": 4,
                "gradleHeapBytes": 3 * 1024**3,
                "gradleWorkers": 4,
                "inputWorkers": 4,
                "memoryBudgetBytes": 1,
                "packageWorkers": 4,
                "parallel": True,
                "profile": "PARALLEL",
            }),
            mock.patch.object(bootstrap, "stage_inputs"),
            mock.patch.object(bootstrap, "build_environment", return_value={}),
            mock.patch.object(bootstrap, "run_build_stage", side_effect=record),
            mock.patch.object(bootstrap, "verify_source_manifest"),
            mock.patch.object(bootstrap, "discard_private_tree"),
        ):
            evidence = bootstrap.prime_dependency_cache(source, root)
        self.assertEqual([name for name, _ in observed], [
            "CARGO_DEPENDENCIES", "GRADLE_DEPENDENCIES"
        ])
        cargo = observed[0][1]
        gradle = observed[1][1]
        self.assertEqual(cargo, ["cargo", "fetch", "--locked"])
        self.assertNotIn("--offline", gradle)
        self.assertIn("-Pkotlin.compiler.execution.strategy=in-process", gradle)
        self.assertIn(
            "-Dorg.gradle.daemon.registry.base=/state/tmp/prime/"
            ".codeclew-gradle-daemon",
            gradle,
        )
        self.assertEqual(evidence["status"], "PRIMED")

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

    def test_component_authority_is_closed_relevant_and_language_neutral(self) -> None:
        core = {
            "mode": 0,
            "path": "crates/core/src/lib.rs",
            "sha256": "sha256:" + "1" * 64,
            "size": 3,
        }
        adapter = {
            "mode": 0,
            "path": "adapters/zeta/main.zeta",
            "sha256": "sha256:" + "2" * 64,
            "size": 5,
        }
        tools = {"compiler": "sha256:" + "3" * 64}
        contract = {"entrypoint": "bin/zeta", "protocol": "adapter:v1"}
        first = bootstrap.component_authority(
            "RELEASE", "language-adapter", "language:zeta", [adapter, core], tools, contract
        )
        reordered = bootstrap.component_authority(
            "RELEASE", "language-adapter", "language:zeta", [core, adapter], tools, contract
        )
        self.assertEqual(first, reordered)
        development = bootstrap.component_authority(
            "DEVELOPMENT", "language-adapter", "language:zeta", [core, adapter], tools, contract
        )
        self.assertNotEqual(first["componentKey"], development["componentKey"])

        core_only = bootstrap.component_authority(
            "RELEASE", "core-binary", "clew", [core], tools, {"entrypoint": "bin/clew"}
        )
        changed_adapter = dict(adapter)
        changed_adapter["sha256"] = "sha256:" + "4" * 64
        same_core = bootstrap.component_authority(
            "RELEASE", "core-binary", "clew", [core], tools, {"entrypoint": "bin/clew"}
        )
        self.assertEqual(core_only, same_core)
        self.assertNotEqual(
            first["componentKey"],
            bootstrap.component_authority(
                "RELEASE",
                "language-adapter",
                "language:zeta",
                [core, changed_adapter],
                tools,
                contract,
            )["componentKey"],
        )
        with self.assertRaisesRegex(bootstrap.BootstrapError, "unsupported value"):
            bootstrap.component_authority(
                "RELEASE", "core-binary", "clew", [core], {"ratio": 0.5}, contract
            )
        tuple_authority = bootstrap.component_authority(
            "RELEASE",
            "core-binary",
            "clew",
            [core],
            {"libc": ("libSystem", "")},
            {"entrypoint": "bin/clew"},
        )
        self.assertTrue(
            bootstrap.valid_runtime_key(tuple_authority["componentKey"])
        )

    def test_component_publish_verify_materialize_and_quarantine(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            for relative in ["runtimes/components", "locks", "tmp", "quarantine"]:
                (root / relative).mkdir(mode=0o700, parents=True, exist_ok=True)
            output = root / "output"
            (output / "bin").mkdir(parents=True)
            executable = output / "bin/zeta"
            executable.write_bytes(b"executable")
            executable.chmod(0o700)
            (output / "metadata.json").write_bytes(b"{}\n")
            authority = bootstrap.component_authority(
                "RELEASE",
                "language-adapter",
                "language:zeta",
                [{
                    "mode": 0,
                    "path": "adapters/zeta/main.zeta",
                    "sha256": "sha256:" + "1" * 64,
                    "size": 3,
                }],
                {"compiler": "sha256:" + "2" * 64},
                {"entrypoint": "bin/zeta", "protocol": "adapter:v1"},
            )
            component, published = bootstrap.publish_component(root, authority, output)
            self.assertTrue(published)
            self.assertEqual(
                bootstrap.verify_component(root, authority["componentKey"], authority)[0],
                component,
            )
            materialized = root / "materialized"
            rows = bootstrap.materialize_component(
                root, authority["componentKey"], materialized, authority
            )
            self.assertEqual([row["path"] for row in rows], ["bin/zeta", "metadata.json"])
            self.assertEqual(stat.S_IMODE((materialized / "bin/zeta").stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE((materialized / "metadata.json").stat().st_mode), 0o600)

            component.chmod(0o700)
            (component / "files").chmod(0o700)
            corrupted = component / "files/bin/zeta"
            corrupted.chmod(0o700)
            corrupted.write_bytes(b"corrupt")
            rebuilt, republished = bootstrap.publish_component(root, authority, output)
            self.assertTrue(republished)
            self.assertEqual(rebuilt, component)
            bootstrap.verify_component(root, authority["componentKey"], authority)
            quarantined = [
                path
                for path in (root / "quarantine").iterdir()
                if path.is_dir() and path.name.startswith("component-")
            ]
            self.assertEqual(len(quarantined), 1)
            record = quarantined[0].with_suffix(".json")
            self.assertEqual(stat.S_IMODE(record.stat().st_mode), 0o600)

    @unittest.skipUnless(hasattr(os, "fork"), "component singleflight requires POSIX")
    def test_component_publish_is_process_singleflight(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            for relative in ["runtimes/components", "locks", "tmp", "quarantine", "results"]:
                (root / relative).mkdir(mode=0o700, parents=True, exist_ok=True)
            output = root / "output"
            output.mkdir()
            (output / "core").write_bytes(b"core")
            authority = bootstrap.component_authority(
                "RELEASE",
                "core-binary",
                "clew",
                [{
                    "mode": 0,
                    "path": "crates/clew/src/main.rs",
                    "sha256": "sha256:" + "5" * 64,
                    "size": 4,
                }],
                {"rustc": "sha256:" + "6" * 64},
                {"entrypoint": "core"},
            )
            children = []
            for index in range(4):
                child = os.fork()
                if child == 0:
                    try:
                        _component, published = bootstrap.publish_component(
                            root, authority, output
                        )
                        (root / "results" / str(index)).write_text(
                            "published" if published else "reused"
                        )
                        os._exit(0)
                    except Exception:
                        os._exit(1)
                children.append(child)
            statuses = [os.waitpid(child, 0)[1] for child in children]
            self.assertTrue(
                all(os.waitstatus_to_exitcode(value) == 0 for value in statuses)
            )
            results = sorted(path.read_text() for path in (root / "results").iterdir())
            self.assertEqual(results.count("published"), 1)
            self.assertEqual(results.count("reused"), 3)
            bootstrap.verify_component(root, authority["componentKey"], authority)

    def test_runtime_component_registry_partitions_real_relevant_inputs(self) -> None:
        repository = MODULE_PATH.parent.parent
        registry = bootstrap.load_component_registry(repository)
        inputs, _development = bootstrap.source_manifest(repository)
        tools = {
            "jdk": {"digest": "sha256:" + "1" * 64},
            "platform": {"digest": "sha256:" + "2" * 64},
            "rust": {"digest": "sha256:" + "3" * 64},
        }
        first = bootstrap.runtime_component_specs(
            "RELEASE", inputs, tools, registry
        )
        self.assertEqual(
            [spec["componentId"] for spec in first],
            ["clew", "kotlin23", "kotlin24"],
        )
        unrelated = [dict(row) for row in inputs]
        bootstrap_row = next(
            row for row in unrelated if row["path"] == "bootstrap/clew_bootstrap.py"
        )
        bootstrap_row["sha256"] = "sha256:" + "4" * 64
        second = bootstrap.runtime_component_specs(
            "RELEASE", unrelated, tools, registry
        )
        self.assertEqual(
            [spec["authority"]["componentKey"] for spec in first],
            [spec["authority"]["componentKey"] for spec in second],
        )

        changed = [dict(row) for row in inputs]
        kotlin23_row = next(
            row
            for row in changed
            if str(row["path"]).startswith("workers/kotlin23/src/main/")
        )
        kotlin23_row["sha256"] = "sha256:" + "6" * 64
        third = bootstrap.runtime_component_specs("RELEASE", changed, tools, registry)
        changed_ids = {
            before["componentId"]
            for before, after in zip(first, third, strict=True)
            if before["authority"]["componentKey"]
            != after["authority"]["componentKey"]
        }
        self.assertEqual(changed_ids, {"kotlin23"})

    def test_component_registry_accepts_a_new_gradle_language_without_core_changes(self) -> None:
        repository = MODULE_PATH.parent.parent
        registry = bootstrap.load_component_registry(repository)
        registry = json.loads(json.dumps(registry))
        registry["components"].append({
            "buildContract": {
                "compilerVersion": "1.0",
                "distribution": "adapters/zeta/build/install/zeta",
                "executor": "GRADLE",
                "manifest": "adapters/zeta/manifest.json",
                "protocol": "semantic-thread.worker.v1",
                "runtimeName": "zeta",
                "task": ":adapters:zeta:installDist",
            },
            "componentId": "zeta",
            "componentKind": "language-adapter",
            "inputFiles": ["build.gradle.kts"],
            "inputRoots": ["adapters/zeta/src/main"],
            "optionalInputRoots": [],
            "toolchainKeys": ["jdk", "platform"],
        })
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory).resolve()
            (source / "bootstrap").mkdir()
            (source / "bootstrap/runtime_components.json").write_bytes(
                bootstrap.canonical(registry) + b"\n"
            )
            zeta_source = source / "adapters/zeta/src/main/zeta/Main.zeta"
            zeta_source.parent.mkdir(parents=True)
            zeta_source.write_text("language zeta\n", encoding="utf-8")
            for relative in [
                "Cargo.lock", "Cargo.toml", "build.gradle.kts", "build.gradle.kts",
                "clew", "gradle/wrapper/gradle-wrapper.jar",
                "gradle/wrapper/gradle-wrapper.properties", "gradlew", "gradlew.bat",
                "rust-toolchain.toml", "schemas/worker.proto", "settings.gradle.kts",
            ]:
                target = source / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                if not target.exists():
                    target.write_text("fixture\n", encoding="utf-8")
            # Existing registry roots must be nonempty; the new zeta root is the
            # regression under test and is deliberately outside workers/**.
            for component in registry["components"]:
                for relative in component["inputFiles"]:
                    target = source / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    if not target.exists():
                        target.write_text("fixture\n", encoding="utf-8")
                for root in component["inputRoots"]:
                    target = source / root / "Fixture.txt"
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text("fixture\n", encoding="utf-8")
            subprocess.run(["git", "init", "-q"], cwd=source, check=True)
            subprocess.run(["git", "add", "."], cwd=source, check=True)
            environment = {
                **os.environ,
                "GIT_AUTHOR_NAME": "Tests",
                "GIT_AUTHOR_EMAIL": "tests@example.invalid",
                "GIT_COMMITTER_NAME": "Tests",
                "GIT_COMMITTER_EMAIL": "tests@example.invalid",
            }
            subprocess.run(
                ["git", "commit", "-qm", "fixture"], cwd=source, env=environment, check=True
            )
            loaded = bootstrap.load_component_registry(source)
            inputs, development = bootstrap.source_manifest(source)
            specs = bootstrap.runtime_component_specs(
                "RELEASE",
                inputs,
                {
                    "jdk": {"digest": "jdk"},
                    "platform": {"digest": "platform"},
                    "python": {"digest": "python"},
                    "rust": {"digest": "rust"},
                },
                loaded,
            )
        self.assertEqual(loaded["components"][-1]["componentId"], "zeta")
        self.assertFalse(development)
        self.assertIn("adapters/zeta/src/main/zeta/Main.zeta", {row["path"] for row in inputs})
        self.assertEqual(specs[-1]["componentId"], "zeta")

    def test_capsule_assembly_all_hit_runs_no_stage_or_toolchain(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "source"
            source.mkdir()
            for relative in [
                "runtimes/components",
                "locks",
                "tmp",
                "quarantine",
            ]:
                (root / relative).mkdir(mode=0o700, parents=True, exist_ok=True)
            core_output = root / "core-output"
            core_output.mkdir()
            core_binary = core_output / "clew"
            core_binary.write_bytes(b"core")
            core_binary.chmod(0o700)
            adapter_output = root / "adapter-output"
            (adapter_output / "bin").mkdir(parents=True)
            adapter_binary = adapter_output / "bin/zeta"
            adapter_binary.write_bytes(b"adapter")
            adapter_binary.chmod(0o700)
            input_row = {
                "mode": 0,
                "path": "input",
                "sha256": "sha256:" + "1" * 64,
                "size": 1,
            }
            core_authority = bootstrap.component_authority(
                "RELEASE",
                "core-binary",
                "clew",
                [input_row],
                {"rust": "sha256:" + "2" * 64},
                {"artifactName": "clew", "binary": "clew", "executor": "CARGO", "package": "clew"},
            )
            adapter_authority = bootstrap.component_authority(
                "RELEASE",
                "language-adapter",
                "zeta",
                [input_row],
                {"compiler": "sha256:" + "3" * 64},
                {
                    "compilerVersion": "1.0",
                    "distribution": "adapters/zeta",
                    "executor": "GRADLE",
                    "manifest": "manifests/zeta.json",
                    "protocol": "semantic-thread.worker.v1",
                    "runtimeName": "zeta",
                    "task": ":zeta:installDist",
                },
            )
            bootstrap.publish_component(root, core_authority, core_output)
            bootstrap.publish_component(root, adapter_authority, adapter_output)
            specs = [
                {
                    "authority": core_authority,
                    "buildContract": {
                        "artifactName": "clew",
                        "binary": "clew",
                        "executor": "CARGO",
                        "package": "clew",
                    },
                    "componentId": "clew",
                    "componentKind": "core-binary",
                },
                {
                    "authority": adapter_authority,
                    "buildContract": {
                        "compilerVersion": "1.0",
                        "distribution": "adapters/zeta",
                        "executor": "GRADLE",
                        "manifest": "manifests/zeta.json",
                        "protocol": "semantic-thread.worker.v1",
                        "runtimeName": "zeta",
                        "task": ":zeta:installDist",
                    },
                    "componentId": "zeta",
                    "componentKind": "language-adapter",
                },
            ]
            plan = {
                "cargoWorkers": 1,
                "gradleHeapBytes": 3 * 1024**3,
                "gradleWorkers": 1,
                "inputWorkers": 1,
                "memoryBudgetBytes": 1,
                "packageWorkers": 2,
                "parallel": True,
                "profile": "AUTO",
            }
            evidence = {}
            runtime_key = "sha256:" + "9" * 64
            tools = {
                "jdk": {},
                "platform": {},
                "python": {},
                "rust": {},
            }
            with (
                mock.patch.object(bootstrap, "runtime_build_plan", return_value=plan),
                mock.patch.object(bootstrap, "load_component_registry", return_value={}),
                mock.patch.object(bootstrap, "runtime_component_specs", return_value=specs),
                mock.patch.object(bootstrap, "verify_source_manifest"),
                mock.patch.object(
                    bootstrap,
                    "stage_inputs",
                    side_effect=AssertionError("component hit staged source"),
                ),
                mock.patch.object(
                    bootstrap,
                    "build_environment",
                    side_effect=AssertionError("component hit prepared a build environment"),
                ),
                mock.patch.object(
                    bootstrap,
                    "build_toolchains",
                    side_effect=AssertionError("component hit started a toolchain"),
                ),
            ):
                capsule = bootstrap.build_capsule(
                    source,
                    root,
                    runtime_key,
                    "RELEASE",
                    [input_row],
                    tools,
                    evidence=evidence,
                )
            self.assertEqual(evidence["componentHits"], ["clew", "zeta"])
            self.assertEqual(evidence["componentMisses"], [])
            self.assertEqual(evidence["buildPlan"]["toolchainStages"], [])
            manifest = bootstrap.verify_capsule(capsule, runtime_key)
            self.assertEqual(
                manifest["components"],
                {
                    "clew": core_authority["componentKey"],
                    "zeta": adapter_authority["componentKey"],
                },
            )
            capsule_core = capsule / "bin" / "clew"
            os.chmod(capsule_core, 0o400)
            with self.assertRaisesRegex(
                bootstrap.BootstrapError, "executable authority mismatch"
            ):
                bootstrap.verify_capsule(capsule, runtime_key)
            os.chmod(capsule_core, 0o500)

    def test_selected_closure_excludes_root_and_nested_legacy_state(self) -> None:
        registry = bootstrap.load_component_registry(MODULE_PATH.parent.parent)
        self.assertFalse(bootstrap.selected_source(".semantic-thread/private", registry))
        self.assertFalse(
            bootstrap.selected_source(
                "crates/clew/src/.semantic-thread/private.rs", registry
            )
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

    def test_sealed_runtime_seed_leases_external_capsule_without_copying_state(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "source"
            source.mkdir(mode=0o700)
            lifecycle_locks = root / "locks"
            lifecycle_locks.mkdir(mode=0o700)
            lifecycle_path = lifecycle_locks / "lifecycle.lock"
            lifecycle_path.write_bytes(b"")
            os.chmod(lifecycle_path, 0o600)
            epoch = root / ("release-N-" + "1" * 40)
            state = epoch / "parallel-state" / "v2"
            key = "sha256:" + "2" * 64
            capsule = state / "runtimes" / key.removeprefix("sha256:")
            capsule.mkdir(parents=True)
            (state / "locks").mkdir()
            os.chmod(epoch, 0o700)
            os.chmod(epoch / "parallel-state", 0o700)
            os.chmod(state, 0o700)
            os.chmod(state / "locks", 0o700)
            manifest = {
                "artifactHashes": {"clew": "sha256:" + "3" * 64},
                "manifestDigest": "sha256:" + "4" * 64,
                "workerTreeHashes": {"kotlin24": "sha256:" + "5" * 64},
            }
            seed = {
                "artifactHashes": manifest["artifactHashes"],
                "buildEvidenceDigests": ["sha256:" + "6" * 64],
                "manifestDigest": manifest["manifestDigest"],
                "mode": "RELEASE",
                "runtimeKey": key,
                "schema": "codeclew-trusted-release-seed/1.0",
                "sourceRevision": "a" * 40,
                "sourceTree": "b" * 40,
                "stateEpoch": "sha256:" + "7" * 64,
                "workerTreeHashes": manifest["workerTreeHashes"],
            }
            seed["seedDigest"] = bootstrap.digest_bytes(bootstrap.canonical(seed))
            seed_path = epoch / "seed.json"
            seed_path.write_bytes(bootstrap.canonical(seed) + b"\n")
            os.chmod(seed_path, 0o400)
            verified = {
                "artifacts": {
                    "clew": {"sha256": manifest["artifactHashes"]["clew"]},
                },
                "manifestDigest": manifest["manifestDigest"],
                "mode": "RELEASE",
                "workers": {
                    "kotlin24": {"treeHash": manifest["workerTreeHashes"]["kotlin24"]},
                },
            }

            def git_authority(arguments, _source):
                return ("a" * 40 if arguments[-1] == "HEAD" else "b" * 40).encode() + b"\n"

            lease_path = state / "locks" / f"runtime-{key[7:]}.lease"
            with (
                mock.patch.dict(
                    os.environ, {"CODECLEW_RUNTIME_SEED": str(seed_path)}, clear=False
                ),
                mock.patch.object(bootstrap, "run", side_effect=git_authority),
                mock.patch.object(bootstrap, "verify_capsule", return_value=verified),
                self.assertRaisesRegex(bootstrap.BootstrapError, "lease is unsafe"),
            ):
                bootstrap.sealed_runtime_seed(source)
            self.assertFalse(lease_path.exists())
            lease_path.write_bytes(b"")
            os.chmod(lease_path, 0o600)

            def verify_under_lease(_capsule, _key):
                with lifecycle_path.open("rb") as lifecycle_contender:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(
                            lifecycle_contender,
                            fcntl.LOCK_EX | fcntl.LOCK_NB,
                        )
                with lease_path.open("a+b") as contender:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
                return verified

            with (
                mock.patch.dict(
                    os.environ, {"CODECLEW_RUNTIME_SEED": str(seed_path)}, clear=False
                ),
                mock.patch.object(bootstrap, "run", side_effect=git_authority),
                mock.patch.object(bootstrap, "verify_capsule", side_effect=verify_under_lease),
            ):
                actual_key, actual_capsule, lease = bootstrap.sealed_runtime_seed(source)
            try:
                self.assertEqual(actual_key, key)
                self.assertEqual(actual_capsule, capsule)
                self.assertFalse((root / "trial-state" / "runtimes").exists())
            finally:
                lease.close()

            def wrong_git_authority(arguments, _source):
                return ("c" * 40 if arguments[-1] == "HEAD" else "b" * 40).encode() + b"\n"

            with (
                mock.patch.dict(
                    os.environ, {"CODECLEW_RUNTIME_SEED": str(seed_path)}, clear=False
                ),
                mock.patch.object(bootstrap, "run", side_effect=wrong_git_authority),
                mock.patch.object(bootstrap, "verify_capsule", return_value=verified),
                self.assertRaisesRegex(
                    bootstrap.BootstrapError, "source authority mismatch"
                ),
            ):
                bootstrap.sealed_runtime_seed(source)

            with (
                mock.patch.dict(
                    os.environ, {"CODECLEW_RUNTIME_SEED": str(seed_path)}, clear=False
                ),
                mock.patch.object(bootstrap, "run", side_effect=git_authority),
                mock.patch.object(
                    bootstrap, "verify_capsule", side_effect=bootstrap.BootstrapError("bad")
                ),
                self.assertRaises(bootstrap.BootstrapError),
            ):
                bootstrap.sealed_runtime_seed(source)
            with lease_path.open("a+b") as contender:
                fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
                fcntl.flock(contender, fcntl.LOCK_UN)

            lease_path.unlink()
            victim = root / "victim"
            victim.write_bytes(b"unchanged")
            os.chmod(victim, 0o644)
            lease_path.symlink_to(victim)
            with (
                mock.patch.dict(
                    os.environ, {"CODECLEW_RUNTIME_SEED": str(seed_path)}, clear=False
                ),
                mock.patch.object(bootstrap, "run", side_effect=git_authority),
                mock.patch.object(bootstrap, "verify_capsule", return_value=verified),
                self.assertRaisesRegex(bootstrap.BootstrapError, "lease is unsafe"),
            ):
                bootstrap.sealed_runtime_seed(source)
            self.assertEqual(victim.read_bytes(), b"unchanged")
            self.assertEqual(stat.S_IMODE(victim.stat().st_mode), 0o644)

    def test_minimal_release_source_is_closed_and_seed_bound(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source"
            bootstrap_directory = source / "bootstrap"
            bootstrap_directory.mkdir(parents=True, mode=0o700)
            launcher = source / "clew"
            launcher.write_bytes(b"#!/bin/sh\nexit 0\n")
            os.chmod(launcher, 0o500)
            module = bootstrap_directory / "clew_bootstrap.py"
            module.write_bytes(b"pass\n")
            os.chmod(module, 0o400)
            rows = []
            for path in (module, launcher):
                metadata = path.stat()
                rows.append({
                    "mode": 0o111 if metadata.st_mode & 0o111 else 0,
                    "path": path.relative_to(source).as_posix(),
                    "sha256": bootstrap.digest_file(path),
                    "size": metadata.st_size,
                })
            rows.sort(key=lambda row: row["path"])
            manifest = {
                "files": rows,
                "manifestDigest": "",
                "schema": bootstrap.RELEASE_SOURCE_SCHEMA,
                "sourceRevision": "a" * 40,
                "sourceTree": "b" * 40,
            }
            manifest["manifestDigest"] = bootstrap.digest_bytes(
                bootstrap.canonical(manifest)
            )
            manifest_path = source / "release-source.json"
            manifest_path.write_bytes(bootstrap.canonical(manifest) + b"\n")
            os.chmod(manifest_path, 0o400)
            seed = {
                "sourcePayloadDigest": manifest["manifestDigest"],
                "sourceRevision": manifest["sourceRevision"],
                "sourceTree": manifest["sourceTree"],
            }
            bootstrap._verify_release_source(source, seed)

            os.chmod(module, 0o600)
            module.write_bytes(b"tampered\n")
            os.chmod(module, 0o400)
            with self.assertRaisesRegex(
                bootstrap.BootstrapError, "file authority mismatch"
            ):
                bootstrap._verify_release_source(source, seed)

            os.chmod(module, 0o600)
            module.write_bytes(b"pass\n")
            os.chmod(module, 0o400)
            extra = source / "unexpected"
            extra.write_bytes(b"private")
            os.chmod(extra, 0o400)
            with self.assertRaisesRegex(
                bootstrap.BootstrapError, "closure mismatch"
            ):
                bootstrap._verify_release_source(source, seed)

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
            checkpoint = json.loads(path.read_bytes())
            self.assertNotIn("PATH", checkpoint["environment"])
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            bootstrap.reset_audit_counters()
            with (
                mock.patch.dict(
                    os.environ,
                    {"PATH": str(root / "no-toolchain-bin")},
                    clear=False,
                ),
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
                    mock.patch.dict(
                        os.environ,
                        {"PATH": str(root / "no-toolchain-bin")},
                        clear=False,
                    ),
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

    def test_checkpoint_miss_revalidates_sealed_capsule_without_toolchain_routes(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "source"
            state = root / "state"
            locks = state / "locks"
            checkpoints = state / "runtimes" / "checkpoints"
            source.mkdir()
            locks.mkdir(parents=True)
            checkpoints.mkdir(parents=True)
            inputs = [{
                "mode": 0,
                "path": "Cargo.toml",
                "sha256": "sha256:" + "0" * 64,
                "size": 12,
            }]
            tools = {
                "jdk": {"releaseSha256": "sha256:" + "1" * 64},
                "platform": {"machine": "test", "system": "test"},
                "python": {"version": "3.14"},
                "rust": {"cargoVersion": "cargo test"},
            }
            key = bootstrap.runtime_key("RELEASE", inputs, tools)
            capsule = state / "runtimes" / key.removeprefix("sha256:")
            capsule.mkdir()
            checkpoint = bootstrap.checkpoint_path(state, source)
            checkpoint.write_bytes(bootstrap.canonical({
                "capsule": str(capsule),
                "runtimeKey": key,
                "schema": "codeclew-runtime-checkpoint/3.0",
            }) + bytes([10]))
            checkpoint.chmod(0o600)
            manifest = {
                "inputDigest": bootstrap.digest_bytes(bootstrap.canonical(inputs)),
                "mode": "RELEASE",
                "platformAuthority": tools["platform"],
                "toolchainAuthority": {
                    name: tools[name] for name in ("python", "rust", "jdk")
                },
            }
            state_descriptor = os.open(state, os.O_RDONLY | os.O_DIRECTORY)
            output = io.StringIO()
            try:
                with (
                    mock.patch.object(
                        bootstrap, "verify_capsule", return_value=manifest
                    ) as verify,
                    mock.patch.object(
                        bootstrap, "source_manifest", return_value=(inputs, False)
                    ),
                    mock.patch.object(
                        bootstrap,
                        "fast_toolchain_locator_authority",
                        side_effect=AssertionError("warm miss probed toolchain routes"),
                    ),
                    mock.patch.object(
                        bootstrap,
                        "toolchain_authority",
                        side_effect=AssertionError("warm miss invoked toolchains"),
                    ),
                    mock.patch.object(
                        bootstrap,
                        "state_root",
                        return_value=(state, state_descriptor),
                    ),
                    mock.patch.object(
                        bootstrap,
                        "garbage_collect_runtime_capsules",
                        side_effect=AssertionError("revalidated capsule ran GC"),
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
            self.assertFalse(audit["coldToolchainInvoked"])
            self.assertFalse(audit["capsuleBuildInvoked"])
            self.assertEqual(audit["counters"]["checkpointMisses"], 1)
            verify.assert_called_once_with(capsule, key)

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
            write_minimal_registry(
                root, input_files=["Cargo.toml"], input_roots=["crates"]
            )
            subprocess.run(
                ["git", "add", "Cargo.toml", "crates/example/src/lib.rs", "bootstrap"],
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
            write_minimal_registry(
                source,
                input_files=["bootstrap/clew_bootstrap.py"],
                input_roots=["bootstrap"],
            )
            subprocess.run(["git", "init", "-q"], cwd=source, check=True)
            subprocess.run(
                ["git", "add", "bootstrap"],
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
            self.assertEqual(environment["HOME"], "/codeclew/home")
            self.assertEqual(environment["USER"], "codeclew")
            self.assertEqual(environment["LOGNAME"], "codeclew")
            self.assertEqual(environment["XDG_CONFIG_HOME"], "/codeclew/config")
            self.assertTrue(Path(environment["CARGO_HOME"]).is_absolute())
            self.assertTrue(Path(environment["RUSTUP_HOME"]).is_absolute())
            self.assertNotIn("RUSTFLAGS", environment)
            remaps = environment["CARGO_ENCODED_RUSTFLAGS"].split("\x1f")
            self.assertEqual(remaps[0], f"--remap-path-prefix={Path.home()}=/codeclew/home")
            self.assertEqual(
                remaps[1],
                f"--remap-path-prefix={environment['CARGO_HOME']}=/codeclew/cargo-home",
            )
            self.assertEqual(
                remaps[2],
                f"--remap-path-prefix={environment['RUSTUP_HOME']}=/codeclew/rustup-home",
            )
            self.assertEqual(remaps[3:], [
                f"--remap-path-prefix={root}=/codeclew/build",
                f"--remap-path-prefix={stage}=/codeclew/source",
            ])
            self.assertNotIn(
                "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER", environment
            )
            self.assertNotIn("JAVA_TOOL_OPTIONS", environment)
            self.assertNotIn("GRADLE_OPTS", environment)

    def test_capsule_privacy_scan_detects_paths_across_stream_chunks(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            capsule = root / "capsule"
            private = root / "private-build"
            artifact = capsule / "bin" / "clew"
            artifact.parent.mkdir(parents=True)
            marker = str(private).encode()
            artifact.write_bytes(b"x" * (1024 * 1024 - len(marker) // 2) + marker + b"tail")
            with self.assertRaisesRegex(
                bootstrap.BootstrapError, "contains a private build path"
            ):
                bootstrap.verify_capsule_has_no_private_paths(capsule, [private])

            artifact.write_bytes(b"binary with /codeclew/source only")
            bootstrap.verify_capsule_has_no_private_paths(capsule, [private])

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

    def test_cold_build_reclaims_unreachable_capsules_before_capacity_gate(self) -> None:
        events: list[str] = []
        with (
            mock.patch.object(
                bootstrap,
                "garbage_collect_runtime_capsules",
                side_effect=lambda _root, _key: events.append("gc"),
            ),
            mock.patch.object(
                bootstrap,
                "require_cold_build_capacity",
                side_effect=lambda _root: events.append("capacity"),
            ),
        ):
            bootstrap.prepare_cold_build_capacity(Path("/private/state"), "sha256:" + "1" * 64)
        self.assertEqual(events, ["gc", "capacity"])

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
                "schema": "codeclew-session/5.0",
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

            legacy_authority = json.loads((session / "authority.json").read_bytes())
            legacy_authority["schema"] = "codeclew-session/3.0"
            (session / "authority.json").write_bytes(bootstrap.canonical(legacy_authority))
            self.assertEqual(bootstrap._session_runtime_roots(root), set())

            legacy_authority["schema"] = "codeclew-session/5.0"
            (session / "authority.json").write_bytes(bootstrap.canonical(legacy_authority))

            lifecycle["status"] = "ABORTED"
            (session / "lifecycle.json").write_bytes(bootstrap.canonical(lifecycle))
            self.assertEqual(
                bootstrap._session_runtime_roots(root),
                {runtime_key.removeprefix("sha256:")},
            )


if __name__ == "__main__":
    unittest.main()
