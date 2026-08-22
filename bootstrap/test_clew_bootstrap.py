from __future__ import annotations

import importlib.util
import fcntl
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).with_name("clew_bootstrap.py")
SPEC = importlib.util.spec_from_file_location("clew_bootstrap", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
bootstrap = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bootstrap)


class BootstrapAuthorityTest(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
