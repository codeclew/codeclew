#!/usr/bin/env python3
from __future__ import annotations

import fcntl
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import trusted_seed_gc


REV_A = "a" * 40
REV_B = "b" * 40
DIGEST = "sha256:" + "c" * 64


class TrustedSeedGcTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name).resolve() / "seeds"
        self.root.mkdir(mode=0o700)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def epoch(self, name: str) -> Path:
        path = self.root / name
        path.mkdir(mode=0o700)
        payload = path / "payload"
        payload.write_bytes(b"derived")
        payload.chmod(0o600)
        return path

    def locator(self, epoch: str) -> None:
        epoch_root = self.root / epoch
        if not epoch_root.exists():
            epoch_root = self.root / (".gc-" + epoch)
        seed_path = epoch_root / "seed.json"
        if not seed_path.exists():
            self.add_valid_seed(epoch_root, epoch.removeprefix("release-N-"))
        seed = json.loads(seed_path.read_bytes())
        locator = trusted_seed_gc._current_locator(
            {
                "epoch": epoch,
                "runtimeKey": seed["runtimeKey"],
                "seedDigest": seed["seedDigest"],
            },
            None,
        )
        publication = epoch_root / "publication.json"
        publication.write_text(
            json.dumps(
                trusted_seed_gc._publication_record_for_locator(locator),
                sort_keys=True,
                separators=(",", ":"),
            )
            + "\n",
            encoding="utf-8",
        )
        publication.chmod(0o400)
        path = self.root / "current.json"
        path.write_text(
            json.dumps(locator, sort_keys=True, separators=(",", ":"))
            + "\n",
            encoding="utf-8",
        )
        path.chmod(0o600)

    def collect(self, *protected: str) -> dict[str, int | str]:
        return trusted_seed_gc.collect(str(self.root), list(protected))

    def add_valid_seed(self, epoch: Path, revision: str) -> None:
        core_bytes = b"core"
        worker_bytes = b"worker"
        core_digest = "sha256:" + hashlib.sha256(core_bytes).hexdigest()
        worker_digest = "sha256:" + hashlib.sha256(worker_bytes).hexdigest()
        worker_row = {
            "mode": 0,
            "path": "worker.jar",
            "sha256": worker_digest,
            "size": len(worker_bytes),
        }
        tree_hasher = hashlib.sha256()
        for field in ("path", "mode", "size", "sha256"):
            tree_hasher.update(str(worker_row[field]).encode())
            tree_hasher.update(b"\0")
        worker_tree = "sha256:" + tree_hasher.hexdigest()
        manifest_digest = None
        for state_name in ("parallel-state",):
            capsule = epoch / state_name / "v2" / "runtimes" / DIGEST[7:]
            core = capsule / "bin" / "clew"
            worker = capsule / "workers" / "kotlin24" / "worker.jar"
            core.parent.mkdir(parents=True)
            worker.parent.mkdir(parents=True)
            core.write_bytes(core_bytes)
            worker.write_bytes(worker_bytes)
            core.chmod(0o500)
            worker.chmod(0o400)
            manifest = {
                "artifactIds": ["clew"],
                "artifacts": {
                    "clew": {
                        "mode": 0o111,
                        "path": "bin/clew",
                        "sha256": core_digest,
                        "size": len(core_bytes),
                    }
                },
                "components": {"clew": DIGEST, "kotlin24": DIGEST},
                "inputDigest": DIGEST,
                "manifestDigest": "",
                "mode": "RELEASE",
                "platformAuthority": {},
                "runtimeKey": DIGEST,
                "schema": "codeclew-runtime-capsule/4.0",
                "toolchainAuthority": {},
                "workerIds": ["kotlin24"],
                "workers": {
                    "kotlin24": {
                        "compilerVersion": "2.4.10",
                        "distribution": "workers/kotlin24",
                        "files": [worker_row],
                        "protocol": "semantic-thread.worker.v1",
                        "treeHash": worker_tree,
                    }
                },
            }
            manifest["manifestDigest"] = "sha256:" + hashlib.sha256(
                json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()
            ).hexdigest()
            manifest_digest = manifest["manifestDigest"]
            runtime_manifest = capsule / "runtime.json"
            runtime_manifest.write_text(
                json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n",
                encoding="utf-8",
            )
            runtime_manifest.chmod(0o400)
            ready = capsule / "READY"
            ready.write_text(DIGEST + "\n", encoding="ascii")
            ready.chmod(0o400)
            for directory in sorted(
                (path for path in capsule.rglob("*") if path.is_dir()),
                key=lambda path: len(path.parts),
                reverse=True,
            ):
                directory.chmod(0o500)
            capsule.chmod(0o500)
            for parent in (
                epoch / state_name,
                epoch / state_name / "v2",
                epoch / state_name / "v2" / "runtimes",
            ):
                parent.chmod(0o700)
        assert manifest_digest is not None
        value = {
            "artifactHashes": {"clew": core_digest},
            "buildEvidenceDigests": [DIGEST],
            "manifestDigest": manifest_digest,
            "mode": "RELEASE",
            "runtimeKey": DIGEST,
            "schema": "codeclew-trusted-release-seed/1.0",
            "sourceRevision": revision,
            "sourceTree": "8" * 40,
            "stateEpoch": DIGEST,
            "workerTreeHashes": {"kotlin24": worker_tree},
        }
        value["seedDigest"] = "sha256:" + hashlib.sha256(
            json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
        ).hexdigest()
        seed = epoch / "seed.json"
        seed.write_text(
            json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        seed.chmod(0o400)

    def current(self) -> str:
        name = "release-N-" + "f" * 40
        if not (self.root / name).exists():
            epoch = self.epoch(name)
            self.add_valid_seed(epoch, "f" * 40)
        self.locator(name)
        return name

    def test_retains_current_and_explicit_protect_but_deletes_stale(self) -> None:
        current = f"release-N-{REV_A}"
        protected = f"release-N-{REV_B}"
        stale = "release-N-" + "d" * 40
        for name in (current, protected, stale):
            self.epoch(name)
        self.locator(current)
        report = self.collect(protected)
        self.assertTrue((self.root / current).is_dir())
        self.assertTrue((self.root / protected).is_dir())
        self.assertFalse((self.root / stale).exists())
        self.assertEqual(report["deletedEpochs"], 1)
        self.assertEqual(report["protectedEpochs"], 2)

    def test_deletes_failed_epoch_and_ignores_unknown_names(self) -> None:
        failed = f"failed-{REV_A}-123"
        unknown = self.epoch("release-N-not-a-revision")
        self.epoch(failed)
        self.current()
        report = self.collect()
        self.assertFalse((self.root / failed).exists())
        self.assertTrue(unknown.exists())
        self.assertEqual(report["deletedEpochs"], 1)

    def test_busy_build_lock_and_lease_are_retained(self) -> None:
        self.current()
        for index, suffix in enumerate(("lock", "lease")):
            with self.subTest(suffix=suffix):
                epoch = self.epoch("release-N-" + str(index + 1) * 40)
                lock = epoch / f"runtime.{suffix}"
                lock.write_bytes(b"")
                lock.chmod(0o600)
                with lock.open("rb") as stream:
                    fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                    report = self.collect()
                self.assertTrue(epoch.exists())
                self.assertEqual(report["skippedBusy"], 1)
                for child in epoch.iterdir():
                    child.unlink()
                epoch.rmdir()

    def test_live_external_gitdir_is_retained(self) -> None:
        epoch = self.epoch(f"release-N-{REV_A}")
        outside = self.root.parent / "live-gitdir"
        outside.mkdir()
        git = epoch / ".git"
        git.write_text(f"gitdir: {outside}\n", encoding="utf-8")
        git.chmod(0o600)
        self.current()
        report = self.collect()
        self.assertTrue(epoch.exists())
        self.assertEqual(report["skippedGitdir"], 1)

    def test_missing_external_gitdir_is_safe(self) -> None:
        epoch = self.epoch(f"release-N-{REV_A}")
        missing = self.root.parent / "missing-gitdir"
        git = epoch / ".git"
        git.write_text(f"gitdir: {missing}\n", encoding="utf-8")
        git.chmod(0o600)
        self.current()
        report = self.collect()
        self.assertFalse(epoch.exists())
        self.assertEqual(report["deletedEpochs"], 1)

    def test_symlink_special_file_and_unsafe_modes_are_refused(self) -> None:
        symlink_epoch = self.epoch(f"release-N-{REV_A}")
        (symlink_epoch / "link").symlink_to(self.root.parent)
        fifo_epoch = self.epoch(f"release-N-{REV_B}")
        os.mkfifo(fifo_epoch / "pipe", 0o600)
        public_epoch = self.epoch("release-N-" + "e" * 40)
        public_epoch.chmod(0o755)
        self.current()
        report = self.collect()
        self.assertEqual(report["skippedUnsafe"], 3)
        self.root.chmod(0o755)
        with self.assertRaises(trusted_seed_gc.AuthorityRefusal):
            self.collect()

    def test_invalid_locator_and_protect_name_fail_closed(self) -> None:
        locator = self.root / "current.json"
        locator.write_text("{}", encoding="utf-8")
        locator.chmod(0o600)
        with self.assertRaises(trusted_seed_gc.AuthorityRefusal):
            self.collect()
        locator.unlink()
        with self.assertRaises(trusted_seed_gc.AuthorityRefusal):
            self.collect("../escape")

    def test_missing_locator_performs_zero_deletion(self) -> None:
        stale = self.epoch(f"release-N-{REV_A}")
        report = self.collect()
        self.assertEqual(report["status"], "NO_CURRENT")
        self.assertTrue(stale.exists())

    def test_protected_crash_tombstone_is_restored(self) -> None:
        epoch = f"release-N-{REV_A}"
        tombstone = self.epoch(".gc-" + epoch)
        self.locator(epoch)
        report = self.collect()
        self.assertFalse(tombstone.exists())
        self.assertTrue((self.root / epoch).is_dir())
        self.assertEqual(report["recoveredTombstones"], 1)

    def test_unprotected_crash_tombstone_is_reclaimed(self) -> None:
        epoch = f"release-N-{REV_A}"
        tombstone = self.epoch(".gc-" + epoch)
        self.current()
        report = self.collect()
        self.assertFalse(tombstone.exists())
        self.assertEqual(report["deletedEpochs"], 1)

    def test_lock_created_after_initial_scan_prevents_deletion(self) -> None:
        epoch_name = f"release-N-{REV_A}"
        epoch = self.epoch(epoch_name)
        self.current()
        original = trusted_seed_gc._scan_candidate
        held = []

        def scan_then_admit(root_fd, name, root_value):
            snapshot = original(root_fd, name, root_value)
            if name == epoch_name and not held:
                lock = epoch / "late-build.lock"
                lock.write_bytes(b"")
                lock.chmod(0o600)
                stream = lock.open("rb")
                fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                held.append(stream)
            return snapshot

        try:
            with mock.patch.object(trusted_seed_gc, "_scan_candidate", side_effect=scan_then_admit):
                report = self.collect()
        finally:
            for stream in held:
                stream.close()
        self.assertTrue(epoch.exists())
        self.assertEqual(report["skippedBusy"], 1)

    def test_mid_delete_failure_leaves_recoverable_tombstone(self) -> None:
        epoch_name = f"release-N-{REV_A}"
        self.epoch(epoch_name)
        self.current()
        original = trusted_seed_gc._delete_directory_contents
        failed = False

        def fail_once(descriptor, device):
            nonlocal failed
            if not failed:
                failed = True
                raise OSError("injected")
            return original(descriptor, device)

        with mock.patch.object(trusted_seed_gc, "_delete_directory_contents", side_effect=fail_once):
            first = self.collect()
        self.assertEqual(first["retainedTombstones"], 1)
        self.assertTrue((self.root / (".gc-" + epoch_name)).exists())
        second = self.collect()
        self.assertFalse((self.root / (".gc-" + epoch_name)).exists())
        self.assertEqual(second["deletedEpochs"], 1)

    def test_publish_candidate_atomically_updates_locator(self) -> None:
        previous = self.current()
        candidate = self.epoch(".candidate.ABC123")
        self.add_valid_seed(candidate, "9" * 40)
        epoch = "release-N-" + "9" * 40
        report = trusted_seed_gc.publish(str(self.root), epoch, candidate.name)
        locator = json.loads((self.root / "current.json").read_bytes())
        self.assertEqual(report["status"], "PASS")
        self.assertEqual(locator["epoch"], epoch)
        self.assertFalse(candidate.exists())
        self.assertTrue((self.root / epoch).is_dir())
        rollback = locator["rollback"]
        self.assertEqual(rollback["epoch"], previous)
        self.assertEqual(locator["generation"], 2)
        self.assertTrue((self.root / previous).is_dir())

        next_candidate = self.epoch(".candidate.DEF456")
        self.add_valid_seed(next_candidate, "7" * 40)
        next_epoch = "release-N-" + "7" * 40
        trusted_seed_gc.publish(str(self.root), next_epoch, next_candidate.name)
        locator = json.loads((self.root / "current.json").read_bytes())
        rollback = locator["rollback"]
        self.assertEqual(locator["epoch"], next_epoch)
        self.assertEqual(rollback["epoch"], epoch)
        self.assertEqual(locator["generation"], 3)
        self.assertTrue((self.root / next_epoch).is_dir())
        self.assertTrue((self.root / epoch).is_dir())
        self.assertFalse((self.root / previous).exists())

    def test_direct_state_runner_holds_lifecycle_through_process_exit(self) -> None:
        current = self.current()
        candidate = self.epoch(".candidate.RUN123")
        self.add_valid_seed(candidate, "9" * 40)
        target = "release-N-" + "9" * 40
        process_entered = threading.Event()
        release_process = threading.Event()
        runner_result: list[object] = []
        publisher_result: list[object] = []
        captured_environment: list[dict[str, str]] = []

        class FakeProcess:
            def __init__(self, _command, *, env, **_kwargs):
                self.pid = 987654321
                captured_environment.append(env)

            def wait(self, timeout=None):
                if timeout is not None:
                    raise AssertionError("unexpected bounded wait")
                process_entered.set()
                if not release_process.wait(5):
                    raise AssertionError("test process was not released")
                return 0

        def run_direct() -> None:
            try:
                runner_result.append(
                    trusted_seed_gc.run_current_state(
                        str(self.root),
                        "parallel-state",
                        current.removeprefix("release-N-"),
                        "8" * 40,
                        ["fake-clew"],
                    )
                )
            except BaseException as error:
                runner_result.append(error)

        def run_publisher() -> None:
            try:
                publisher_result.append(
                    trusted_seed_gc.publish(str(self.root), target, candidate.name)
                )
            except BaseException as error:
                publisher_result.append(error)

        with (
            mock.patch.object(trusted_seed_gc.subprocess, "Popen", FakeProcess),
            mock.patch.object(
                trusted_seed_gc, "_process_group_exists", return_value=False
            ),
        ):
            runner = threading.Thread(target=run_direct)
            runner.start()
            self.assertTrue(process_entered.wait(5))
            publisher = threading.Thread(target=run_publisher)
            publisher.start()
            time.sleep(0.05)
            self.assertTrue(publisher.is_alive())
            self.assertEqual(
                json.loads((self.root / "current.json").read_bytes())["epoch"],
                current,
            )
            release_process.set()
            runner.join(5)
            publisher.join(5)
        self.assertFalse(runner.is_alive())
        self.assertFalse(publisher.is_alive())
        self.assertEqual(runner_result, [0])
        self.assertIsInstance(publisher_result[0], dict)
        self.assertEqual(
            captured_environment[0]["CODECLEW_HOME"],
            str(self.root / current / "parallel-state"),
        )

    def test_signal_during_spawn_reaps_assigned_process_group(self) -> None:
        current = self.current()

        class SignalDuringSpawn:
            def __init__(self, _command, **_kwargs):
                self.pid = 456789
                os.kill(os.getpid(), signal.SIGTERM)

            def wait(self):
                raise AssertionError("pending signal must interrupt before wait")

        with (
            mock.patch.object(
                trusted_seed_gc.subprocess, "Popen", SignalDuringSpawn
            ),
            mock.patch.object(
                trusted_seed_gc, "_terminate_process_group"
            ) as terminate,
        ):
            with self.assertRaises(trusted_seed_gc.SupervisorInterrupted) as caught:
                trusted_seed_gc.run_current_state(
                    str(self.root),
                    "parallel-state",
                    current.removeprefix("release-N-"),
                    "8" * 40,
                    ["fake-clew"],
                )
        self.assertEqual(caught.exception.signum, signal.SIGTERM)
        terminate.assert_called_once_with(456789)

    def test_nonzero_leader_with_reap_failure_is_typed_fail_closed(self) -> None:
        current = self.current()
        pinned: list[int] = []

        class FailedLeader:
            pid = 567890

            def __init__(self, _command, **_kwargs):
                pass

            def wait(self):
                return 42

        def pin(lifecycle, _process_group):
            pinned.append(os.dup(lifecycle.fileno()))

        with (
            mock.patch.object(trusted_seed_gc.subprocess, "Popen", FailedLeader),
            mock.patch.object(
                trusted_seed_gc, "_process_group_exists", return_value=True
            ),
            mock.patch.object(
                trusted_seed_gc,
                "_terminate_process_group",
                side_effect=trusted_seed_gc.AuthorityRefusal(
                    "PROCESS_GROUP_AUTHORITY"
                ),
            ),
            mock.patch.object(
                trusted_seed_gc,
                "_pin_process_group_until_gone",
                side_effect=pin,
            ),
        ):
            with self.assertRaises(
                trusted_seed_gc.ProcessGroupReapFailure
            ) as caught:
                trusted_seed_gc.run_current_state(
                    str(self.root),
                    "parallel-state",
                    current.removeprefix("release-N-"),
                    "8" * 40,
                    ["fake-clew"],
                )
        self.assertEqual(caught.exception.leader_exit_code, 42)
        self.assertEqual(len(pinned), 1)
        lifecycle_path = self.root / "locks" / "lifecycle.lock"
        with lifecycle_path.open("r+b") as lifecycle:
            with self.assertRaises(BlockingIOError):
                fcntl.flock(lifecycle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        os.close(pinned.pop())
        with lifecycle_path.open("r+b") as lifecycle:
            fcntl.flock(lifecycle, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def test_guardian_is_passive_and_ignores_reused_pid_identity(self) -> None:
        self.assertNotIn("killpg", trusted_seed_gc.PROCESS_GROUP_GUARDIAN)
        self.assertNotIn("SIGKILL", trusted_seed_gc.PROCESS_GROUP_GUARDIAN)
        with mock.patch.object(
            trusted_seed_gc.subprocess,
            "check_output",
            return_value="123 Mon Jan  1 00:00:01 2027\n",
        ):
            self.assertFalse(
                trusted_seed_gc._identities_still_live(
                    [(123, "Mon Jan  1 00:00:00 2027")]
                )
            )

    def test_process_snapshot_rejects_malformed_or_partial_ps_output(self) -> None:
        malformed = subprocess.CompletedProcess(
            [trusted_seed_gc.TRUSTED_PS], 0, "123 malformed\n", ""
        )
        duplicate = subprocess.CompletedProcess(
            [trusted_seed_gc.TRUSTED_PS],
            0,
            (
                "123 123 Mon Jan  1 00:00:00 2027\n"
                "123 123 Mon Jan  1 00:00:00 2027\n"
            ),
            "",
        )
        for completed in (malformed, duplicate):
            with self.subTest(output=completed.stdout):
                with mock.patch.object(
                    trusted_seed_gc.subprocess, "run", return_value=completed
                ):
                    self.assertIsNone(trusted_seed_gc._ps_group_snapshot(123))

    @unittest.skipUnless(hasattr(os, "fork"), "requires POSIX process groups")
    def test_guardian_tracks_descendant_forked_after_initial_snapshot(self) -> None:
        lifecycle_path = self.root / "locks" / "lifecycle.lock"
        lifecycle_path.parent.mkdir(mode=0o700)
        lifecycle_path.write_bytes(b"")
        lifecycle_path.chmod(0o600)
        trigger = self.root.parent / "guardian-trigger"
        marker = self.root.parent / "guardian-child"
        release = self.root.parent / "guardian-release"
        program = (
            "import os,pathlib,sys,time; "
            "trigger=pathlib.Path(sys.argv[1]); marker=pathlib.Path(sys.argv[2]); "
            "release=pathlib.Path(sys.argv[3]); "
            "\nwhile not trigger.exists(): time.sleep(.01)\n"
            "child=os.fork()\n"
            "if child == 0:\n"
            " marker.write_text(str(os.getpid()))\n"
            " while not release.exists(): time.sleep(.01)\n"
            " os._exit(0)\n"
            "time.sleep(1); os._exit(0)\n"
        )
        governed = subprocess.Popen(
            [sys.executable, "-I", "-S", "-c", program, str(trigger), str(marker), str(release)],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        try:
            with lifecycle_path.open("r+b") as lifecycle:
                fcntl.flock(lifecycle, fcntl.LOCK_SH | fcntl.LOCK_NB)
                trusted_seed_gc._pin_process_group_until_gone(
                    lifecycle, governed.pid
                )
            trigger.touch()
            deadline = time.monotonic() + 5
            while not marker.is_file() and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(marker.is_file())
            self.assertEqual(governed.wait(timeout=5), 0)
            with lifecycle_path.open("r+b") as contender:
                with self.assertRaises(BlockingIOError):
                    fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
                release.touch()
                deadline = time.monotonic() + 5
                while True:
                    try:
                        fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        break
                    except BlockingIOError:
                        if time.monotonic() >= deadline:
                            raise
                        time.sleep(0.02)
        finally:
            release.touch(exist_ok=True)
            try:
                os.killpg(governed.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            try:
                governed.wait(timeout=5)
            except subprocess.TimeoutExpired:
                pass

    @unittest.skipUnless(hasattr(os, "fork"), "requires POSIX process groups")
    def test_guardian_handshake_failure_keeps_parent_pin(self) -> None:
        lifecycle_path = self.root / "locks" / "lifecycle.lock"
        lifecycle_path.parent.mkdir(mode=0o700)
        lifecycle_path.write_bytes(b"")
        lifecycle_path.chmod(0o600)
        governed = subprocess.Popen(
            [sys.executable, "-I", "-S", "-c", "import time; time.sleep(60)"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        returned = threading.Event()

        def pin() -> None:
            with lifecycle_path.open("r+b") as lifecycle:
                fcntl.flock(lifecycle, fcntl.LOCK_SH | fcntl.LOCK_NB)
                trusted_seed_gc._pin_process_group_until_gone(
                    lifecycle, governed.pid
                )
            returned.set()

        failing_guardian = "import os,sys; os.close(int(sys.argv[5])); os._exit(23)"
        try:
            with mock.patch.object(
                trusted_seed_gc, "PROCESS_GROUP_GUARDIAN", failing_guardian
            ):
                thread = threading.Thread(target=pin)
                thread.start()
                time.sleep(0.25)
                self.assertFalse(returned.is_set())
                with lifecycle_path.open("r+b") as contender:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
                os.killpg(governed.pid, signal.SIGTERM)
                governed.wait(timeout=5)
                thread.join(5)
                self.assertFalse(thread.is_alive())
                self.assertTrue(returned.is_set())
        finally:
            try:
                os.killpg(governed.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            try:
                governed.wait(timeout=5)
            except subprocess.TimeoutExpired:
                pass

    @unittest.skipUnless(hasattr(os, "fork"), "requires POSIX process groups")
    def test_direct_state_runner_reaps_background_descendants_on_success(self) -> None:
        current = self.current()
        command = [
            sys.executable,
            "-I",
            "-S",
            "-c",
            (
                "import os,time; child=os.fork(); "
                "os._exit(0) if child else time.sleep(60)"
            ),
        ]
        started = time.monotonic()
        result = trusted_seed_gc.run_current_state(
            str(self.root),
            "parallel-state",
            current.removeprefix("release-N-"),
            "8" * 40,
            command,
        )
        self.assertEqual(result, trusted_seed_gc.DESCENDANT_LEAK_EXIT)
        self.assertLess(time.monotonic() - started, 8)

    def test_direct_state_runner_rejects_removed_serial_state(self) -> None:
        current = self.current()
        with self.assertRaisesRegex(
            trusted_seed_gc.AuthorityRefusal, "COMMAND_AUTHORITY"
        ):
            trusted_seed_gc.run_current_state(
                str(self.root),
                "serial-state",
                current.removeprefix("release-N-"),
                "8" * 40,
                ["ignored"],
            )

    @unittest.skipUnless(hasattr(os, "fork"), "requires POSIX process groups")
    def test_direct_state_runner_preserves_nonzero_leader_first_cause(self) -> None:
        current = self.current()
        command = [
            sys.executable,
            "-I",
            "-S",
            "-c",
            (
                "import os,time; child=os.fork(); "
                "os._exit(42) if child else time.sleep(60)"
            ),
        ]
        result = trusted_seed_gc.run_current_state(
            str(self.root),
            "parallel-state",
            current.removeprefix("release-N-"),
            "8" * 40,
            command,
        )
        self.assertEqual(result, 42)

    @unittest.skipUnless(hasattr(os, "fork"), "requires POSIX process groups")
    def test_sigterm_supervisor_reaps_child_before_releasing_lifecycle(self) -> None:
        current = self.current()
        marker = self.root.parent / "governed-processes"
        program = (
            "sh -c 'trap \"sleep 1\" TERM; sleep 60' & "
            "child=$!; printf '%s %s\\n' \"$$\" \"$child\" >\"$1\"; wait"
        )
        supervisor = subprocess.Popen(
            [
                sys.executable,
                "-I",
                "-S",
                str(Path(trusted_seed_gc.__file__)),
                "--root",
                str(self.root),
                "--run-current-state",
                "parallel-state",
                "--expected-source-revision",
                current.removeprefix("release-N-"),
                "--expected-source-tree",
                "8" * 40,
                "--",
                "/bin/sh",
                "-c",
                program,
                "codeclew-governed-test",
                str(marker),
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        deadline = time.monotonic() + 5
        while not marker.is_file() and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue(marker.is_file())
        leader = int(marker.read_text().split()[0])
        lifecycle_path = self.root / "locks" / "lifecycle.lock"
        with lifecycle_path.open("r+b") as lifecycle:
            with self.assertRaises(BlockingIOError):
                fcntl.flock(lifecycle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            os.kill(supervisor.pid, signal.SIGTERM)
            time.sleep(0.1)
            with self.assertRaises(BlockingIOError):
                fcntl.flock(lifecycle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertEqual(supervisor.wait(timeout=10), 128 + signal.SIGTERM)
            deadline = time.monotonic() + 5
            while True:
                try:
                    fcntl.flock(lifecycle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(0.02)
        self.assertFalse(trusted_seed_gc._process_group_exists(leader))

    def test_authority_digest_validates_and_binds_rollback_generation(self) -> None:
        previous = self.current()
        candidate = self.epoch(".candidate.AUTH123")
        self.add_valid_seed(candidate, "9" * 40)
        current = "release-N-" + "9" * 40
        trusted_seed_gc.publish(str(self.root), current, candidate.name)
        authority = trusted_seed_gc.authority_digest(
            str(self.root), "9" * 40, "8" * 40
        )
        self.assertRegex(authority, r"^sha256:[0-9a-f]{64}$")

        rollback_core = (
            self.root
            / previous
            / "parallel-state"
            / "v2"
            / "runtimes"
            / DIGEST[7:]
            / "bin"
            / "clew"
        )
        rollback_core.chmod(0o700)
        rollback_core.write_bytes(b"tampered rollback")
        rollback_core.chmod(0o500)
        with self.assertRaises(trusted_seed_gc.AuthorityRefusal):
            trusted_seed_gc.authority_digest(
                str(self.root), "9" * 40, "8" * 40
            )

    def test_public_output_never_contains_private_paths(self) -> None:
        epoch = self.epoch(f"release-N-{REV_A}")
        git = epoch / ".git"
        private = self.root.parent / "person-name" / "worktree"
        git.write_text(f"gitdir: {private}\n", encoding="utf-8")
        git.chmod(0o600)
        self.current()
        output = io.StringIO()
        errors = io.StringIO()
        with (
            mock.patch.object(
                sys,
                "argv",
                ["trusted_seed_gc.py", "--root", str(self.root)],
            ),
            contextlib.redirect_stdout(output),
            contextlib.redirect_stderr(errors),
        ):
            self.assertEqual(trusted_seed_gc.main(), 0)
        combined = output.getvalue() + errors.getvalue()
        self.assertNotIn(str(self.root), combined)
        self.assertNotIn("person-name", combined)

    def test_exact_name_allowlist_leaves_near_misses_inert(self) -> None:
        names = (
            "release-N-" + "a" * 39,
            "release-N-" + "A" * 40,
            "release-N-" + "a" * 40 + "-extra",
            "failed-" + "a" * 40 + "-0",
            "failed-" + "a" * 40 + "-12345678901",
            ".candidate.ABC123",
            "unmanaged",
        )
        paths = [self.epoch(name) for name in names]
        self.current()
        report = self.collect()
        self.assertEqual(report["deletedEpochs"], 0)
        self.assertTrue(all(path.exists() for path in paths))

    def test_owner_only_read_only_tree_is_collectable(self) -> None:
        epoch = self.epoch(f"release-N-{REV_A}")
        child = epoch / "sealed"
        child.mkdir(mode=0o700)
        artifact = child / "artifact"
        artifact.write_bytes(b"capsule")
        artifact.chmod(0o400)
        child.chmod(0o500)
        self.current()
        report = self.collect()
        self.assertFalse(epoch.exists())
        self.assertEqual(report["deletedEpochs"], 1)

    def test_all_tree_bounds_fail_closed(self) -> None:
        cases = (
            ("MAX_ENTRIES", 0),
            ("MAX_APPARENT_BYTES", 1),
            ("MAX_DEPTH", 0),
            ("MAX_LOCKS", 0),
            ("MAX_GITDIR_FILES", 0),
        )
        self.current()
        for index, (constant, limit) in enumerate(cases):
            with self.subTest(constant=constant):
                epoch = self.epoch("release-N-" + format(index + 10, "x") * 40)
                if constant == "MAX_DEPTH":
                    child = epoch / "nested"
                    child.mkdir(mode=0o700)
                    nested = child / "file"
                    nested.write_bytes(b"x")
                    nested.chmod(0o600)
                elif constant == "MAX_LOCKS":
                    lock = epoch / "build.lock"
                    lock.write_bytes(b"")
                    lock.chmod(0o600)
                elif constant == "MAX_GITDIR_FILES":
                    git = epoch / ".git"
                    git.write_text(
                        f"gitdir: {self.root.parent / 'missing'}\n",
                        encoding="utf-8",
                    )
                    git.chmod(0o600)
                with mock.patch.object(trusted_seed_gc, constant, limit):
                    report = self.collect()
                self.assertTrue(epoch.exists())
                if constant in {"MAX_ENTRIES", "MAX_APPARENT_BYTES", "MAX_DEPTH"}:
                    self.assertEqual(report["status"], "REFUSED_CURRENT")
                    self.assertEqual(report["deletedEpochs"], 0)
                else:
                    self.assertGreaterEqual(report["skippedUnsafe"], 1)

    def test_malformed_relative_and_live_symlink_gitdirs_are_retained(self) -> None:
        self.current()
        values = (
            "not-a-gitdir\n",
            "gitdir: ../relative\n",
        )
        for index, value in enumerate(values):
            epoch = self.epoch("release-N-" + str(index + 3) * 40)
            git = epoch / ".git"
            git.write_text(value, encoding="utf-8")
            git.chmod(0o600)
        target = self.root.parent / "dangling-target"
        target.symlink_to(self.root.parent / "absent")
        epoch = self.epoch("release-N-" + "5" * 40)
        git = epoch / ".git"
        git.write_text(f"gitdir: {target}\n", encoding="utf-8")
        git.chmod(0o600)
        report = self.collect()
        self.assertEqual(report["skippedUnsafe"], 2)
        self.assertEqual(report["skippedGitdir"], 1)

    def test_symlink_locator_refuses_without_deleting(self) -> None:
        epoch = self.epoch(f"release-N-{REV_A}")
        target = self.root / "locator-target"
        target.write_text("{}", encoding="utf-8")
        target.chmod(0o600)
        (self.root / "current.json").symlink_to(target)
        with self.assertRaises(trusted_seed_gc.AuthorityRefusal):
            self.collect()
        self.assertTrue(epoch.exists())

    def test_validate_epoch_refuses_symlink_without_touching_outside_tree(self) -> None:
        epoch_name = f"release-N-{REV_A}"
        outside = self.root.parent / "outside-epoch"
        outside.mkdir(mode=0o700)
        sentinel = outside / "sentinel"
        sentinel.write_bytes(b"unchanged")
        sentinel.chmod(0o600)
        (self.root / epoch_name).symlink_to(outside, target_is_directory=True)
        with self.assertRaises(
            (OSError, trusted_seed_gc.UnsafeEpoch, trusted_seed_gc.AuthorityRefusal)
        ):
            trusted_seed_gc.validate_epoch(str(self.root), epoch_name)
        self.assertEqual(sentinel.read_bytes(), b"unchanged")

    def test_rename_and_fsync_faults_never_partially_delete_visible_epoch(self) -> None:
        self.current()
        rename_epoch = self.epoch(f"release-N-{REV_A}")
        with mock.patch.object(os, "rename", side_effect=OSError("injected")):
            report = self.collect()
        self.assertTrue(rename_epoch.exists())
        self.assertEqual(report["skippedUnsafe"], 1)

        for child in rename_epoch.iterdir():
            child.unlink()
        rename_epoch.rmdir()
        fsync_epoch = self.epoch(f"release-N-{REV_B}")
        original_fsync = trusted_seed_gc._fsync_directory
        failed = False

        def fail_after_withdrawal(descriptor):
            nonlocal failed
            if not failed and (self.root / (".gc-" + fsync_epoch.name)).exists():
                failed = True
                raise OSError("injected")
            return original_fsync(descriptor)

        with mock.patch.object(
            trusted_seed_gc,
            "_fsync_directory",
            side_effect=fail_after_withdrawal,
        ):
            report = self.collect()
        self.assertTrue(fsync_epoch.exists())
        self.assertFalse((self.root / (".gc-" + fsync_epoch.name)).exists())
        self.assertEqual(report["skippedUnsafe"], 1)

    def test_gc_snapshot_and_concurrent_publish_never_create_dangling_current(self) -> None:
        current = self.current()
        candidate_epoch = "release-N-" + "9" * 40
        candidate = self.epoch(candidate_epoch)
        self.add_valid_seed(candidate, "9" * 40)
        snapshot = threading.Event()
        release = threading.Event()
        original_read = trusted_seed_gc._read_current
        collector_result = []
        publisher_result = []

        def pause_after_snapshot(root_fd):
            value = original_read(root_fd)
            snapshot.set()
            if not release.wait(5):
                raise AssertionError("test synchronization timed out")
            return value

        def run_collector():
            try:
                collector_result.append(self.collect())
            except BaseException as error:
                collector_result.append(error)

        def run_publisher():
            try:
                publisher_result.append(
                    trusted_seed_gc.publish(str(self.root), candidate_epoch)
                )
            except BaseException as error:
                publisher_result.append(error)

        with mock.patch.object(
            trusted_seed_gc,
            "_read_current",
            side_effect=pause_after_snapshot,
        ):
            collector = threading.Thread(target=run_collector)
            collector.start()
            self.assertTrue(snapshot.wait(5))
            publisher = threading.Thread(target=run_publisher)
            publisher.start()
            time.sleep(0.02)
            release.set()
            collector.join(5)
            publisher.join(5)
        self.assertFalse(collector.is_alive())
        self.assertFalse(publisher.is_alive())
        self.assertIsInstance(publisher_result[0], trusted_seed_gc.AuthorityRefusal)
        locator = json.loads((self.root / "current.json").read_bytes())
        self.assertEqual(locator["epoch"], current)
        self.assertTrue((self.root / current).is_dir())
        self.assertFalse((self.root / candidate_epoch).exists())
        self.assertEqual(collector_result[0]["deletedEpochs"], 1)

    def test_current_locator_seed_and_capsule_mismatch_make_gc_zero_delete(self) -> None:
        current = self.current()
        stale = self.epoch(f"release-N-{REV_A}")
        locator_path = self.root / "current.json"
        original_locator = json.loads(locator_path.read_bytes())

        for field in ("runtimeKey", "seedDigest"):
            with self.subTest(field=field):
                locator = dict(original_locator)
                locator[field] = "sha256:" + "1" * 64
                locator_path.write_text(
                    json.dumps(locator, sort_keys=True, separators=(",", ":")) + "\n",
                    encoding="utf-8",
                )
                locator_path.chmod(0o600)
                report = self.collect()
                self.assertEqual(report["status"], "REFUSED_CURRENT")
                self.assertTrue(stale.exists())

        locator_path.write_text(
            json.dumps(original_locator, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        locator_path.chmod(0o600)
        seed_path = self.root / current / "seed.json"
        seed_bytes = seed_path.read_bytes()
        seed_path.unlink()
        report = self.collect()
        self.assertEqual(report["status"], "REFUSED_CURRENT")
        self.assertTrue(stale.exists())
        seed_path.write_bytes(seed_bytes)
        seed_path.chmod(0o400)

        core = (
            self.root
            / current
            / "parallel-state"
            / "v2"
            / "runtimes"
            / DIGEST[7:]
            / "bin"
            / "clew"
        )
        core.chmod(0o700)
        core.write_bytes(b"tampered")
        core.chmod(0o500)
        report = self.collect()
        self.assertEqual(report["status"], "REFUSED_CURRENT")
        self.assertTrue(stale.exists())

    def test_publish_rejects_epoch_source_revision_mismatch(self) -> None:
        current = self.current()
        candidate = self.epoch(".candidate.GHI789")
        self.add_valid_seed(candidate, "8" * 40)
        target = "release-N-" + "9" * 40
        with self.assertRaises(trusted_seed_gc.AuthorityRefusal):
            trusted_seed_gc.publish(str(self.root), target, candidate.name)
        locator = json.loads((self.root / "current.json").read_bytes())
        self.assertEqual(locator["epoch"], current)
        self.assertTrue((self.root / current).is_dir())

    def test_invalid_rollback_authority_makes_gc_zero_delete(self) -> None:
        previous = self.current()
        candidate = self.epoch(".candidate.BAD123")
        self.add_valid_seed(candidate, "9" * 40)
        current = "release-N-" + "9" * 40
        trusted_seed_gc.publish(str(self.root), current, candidate.name)
        stale = self.epoch(f"release-N-{REV_A}")
        path = self.root / "current.json"
        locator = json.loads(path.read_bytes())
        self.assertEqual(locator["rollback"]["epoch"], previous)
        locator["rollback"]["seedDigest"] = "sha256:" + "1" * 64
        path.write_text(
            json.dumps(locator, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        path.chmod(0o600)
        report = self.collect()
        self.assertEqual(report["status"], "REFUSED_CURRENT")
        self.assertTrue(stale.exists())

    def test_generation_two_cannot_drop_embedded_rollback(self) -> None:
        self.current()
        candidate = self.epoch(".candidate.MISS123")
        self.add_valid_seed(candidate, "9" * 40)
        current = "release-N-" + "9" * 40
        trusted_seed_gc.publish(str(self.root), current, candidate.name)
        path = self.root / "current.json"
        locator = json.loads(path.read_bytes())
        locator["generation"] = 1
        locator["rollback"] = None
        path.write_text(
            json.dumps(locator, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        path.chmod(0o600)
        with self.assertRaises(trusted_seed_gc.AuthorityRefusal):
            trusted_seed_gc.authority_digest(str(self.root), "9" * 40, "8" * 40)
        report = self.collect()
        self.assertEqual(report["status"], "REFUSED_CURRENT")

    def test_missing_locator_after_generation_two_cannot_reset_history(self) -> None:
        previous = self.current()
        candidate = self.epoch(".candidate.LOST123")
        self.add_valid_seed(candidate, "9" * 40)
        current = "release-N-" + "9" * 40
        trusted_seed_gc.publish(str(self.root), current, candidate.name)
        (self.root / "current.json").unlink()
        stale = self.epoch(f"release-N-{REV_A}")
        report = self.collect()
        self.assertEqual(report["status"], "NO_CURRENT")
        self.assertTrue((self.root / previous).exists())
        self.assertTrue((self.root / current).exists())
        self.assertTrue(stale.exists())
        with self.assertRaises(trusted_seed_gc.AuthorityRefusal):
            trusted_seed_gc.authority_digest(
                str(self.root), "9" * 40, "8" * 40
            )
        with self.assertRaises(trusted_seed_gc.AuthorityRefusal) as caught:
            trusted_seed_gc.publish(str(self.root), current)
        self.assertEqual(caught.exception.reason, "HISTORY_AUTHORITY")

    def test_publication_commit_crash_recovers_generation_one_and_n(self) -> None:
        def fail_current(root_fd, name, value):
            if name == "current.json":
                raise OSError("injected after durable publication")
            return trusted_seed_gc._atomic_json_at(root_fd, name, value)

        first_candidate = self.epoch(".candidate.REC101")
        self.add_valid_seed(first_candidate, "9" * 40)
        first_epoch = "release-N-" + "9" * 40
        with mock.patch.object(
            trusted_seed_gc, "_atomic_json_at", side_effect=fail_current
        ):
            with self.assertRaises(trusted_seed_gc.AuthorityRefusal):
                trusted_seed_gc.publish(
                    str(self.root), first_epoch, first_candidate.name
                )
        self.assertFalse((self.root / "current.json").exists())
        self.assertTrue((self.root / first_epoch / "publication.json").is_file())
        trusted_seed_gc.publish(str(self.root), first_epoch)
        first_locator = json.loads((self.root / "current.json").read_bytes())
        self.assertEqual(first_locator["generation"], 1)

        next_candidate = self.epoch(".candidate.REC202")
        self.add_valid_seed(next_candidate, "7" * 40)
        next_epoch = "release-N-" + "7" * 40
        with mock.patch.object(
            trusted_seed_gc, "_atomic_json_at", side_effect=fail_current
        ):
            with self.assertRaises(trusted_seed_gc.AuthorityRefusal):
                trusted_seed_gc.publish(
                    str(self.root), next_epoch, next_candidate.name
                )
        self.assertEqual(
            json.loads((self.root / "current.json").read_bytes())["epoch"],
            first_epoch,
        )
        self.assertTrue((self.root / next_epoch / "publication.json").is_file())
        trusted_seed_gc.publish(str(self.root), next_epoch)
        next_locator = json.loads((self.root / "current.json").read_bytes())
        self.assertEqual(next_locator["generation"], 2)
        self.assertEqual(next_locator["rollback"]["epoch"], first_epoch)

    def test_publication_final_is_never_partial_across_write_boundaries(self) -> None:
        locator = trusted_seed_gc._current_locator(
            {
                "epoch": "release-N-" + "9" * 40,
                "runtimeKey": DIGEST,
                "seedDigest": DIGEST,
            },
            None,
        )
        injections = (
            ("fdopen", mock.patch.object(os, "fdopen", side_effect=OSError("write"))),
            ("fchmod", mock.patch.object(os, "fchmod", side_effect=OSError("chmod"))),
            ("fsync", mock.patch.object(os, "fsync", side_effect=OSError("fsync"))),
            ("link", mock.patch.object(os, "link", side_effect=OSError("link"))),
        )
        for index, (label, injection) in enumerate(injections):
            with self.subTest(label=label):
                candidate = self.epoch(f".candidate.FLT{index}00")
                root_fd = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
                try:
                    with injection:
                        with self.assertRaises(OSError):
                            trusted_seed_gc._create_or_validate_publication(
                                root_fd,
                                locator,
                                container_name=candidate.name,
                            )
                finally:
                    os.close(root_fd)
                self.assertFalse((candidate / "publication.json").exists())
                self.assertFalse(
                    any(
                        path.name.startswith(".publication.")
                        for path in candidate.iterdir()
                    )
                )


if __name__ == "__main__":
    unittest.main()
