#!/usr/bin/env python3
"""Fast deterministic tests for the S4K private runner and closed broker."""

from __future__ import annotations

import json
import io
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest import mock

sys.path.insert(0, os.fspath(Path(__file__).resolve().parent))

import build_thread_kotlin_shape_oracle as shape_builder
import run_thread_kotlin_pilot as pilot
import run_thread_kotlin_descriptor_gate as descriptor_gate
import thread_kotlin_pilot_broker as broker
import verify_thread_kotlin_pilot as public_verifier


class _ShapeBuilderProgressHelpers(unittest.TestCase):
    @staticmethod
    def stream(*rows: dict[str, object]) -> bytes:
        return b"".join(shape_builder.canonical_bytes(row) + bytes([10]) for row in rows)

    @staticmethod
    def progress(event: str, stage_id: str | None, **values: int) -> dict[str, object]:
        return {
            "admittedCpu": values.get("admittedCpu", 0),
            "admittedRssBytes": values.get("admittedRssBytes", 0),
            "done": values.get("done", 0),
            "event": event,
            "queued": values.get("queued", 0),
            "running": values.get("running", 0),
            "schema": "codeclew-cold-start-progress/2.0",
            "stageId": stage_id,
            "unixMillis": values.get("unixMillis", 1),
        }

    @staticmethod
    def capsule(event: str, stage: str, **values: int) -> dict[str, object]:
        return {
            "schema": "codeclew-capsule-progress/2.0",
            "event": event,
            "stage": stage,
            **values,
        }

    @staticmethod
    def emitted_stream(*rows: dict[str, object]) -> bytes:
        order = (
            "schema", "event", "stageId", "queued", "running", "done",
            "admittedCpu", "admittedRssBytes", "unixMillis",
        )
        return b"".join(
            json.dumps(
                {key: row[key] for key in order},
                ensure_ascii=False,
                separators=(",", ":"),
            ).encode("utf-8") + bytes([10])
            for row in rows
        )


class MavenDistributionAuthorityTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def _distribution(
        self, name: str = "maven"
    ) -> tuple[Path, Path, Path, Path]:
        install = self.root / name
        distribution = install / "libexec"
        for relative in ("bin", "boot", "conf", "lib"):
            (distribution / relative).mkdir(parents=True, exist_ok=True)
        wrapper = install / "bin" / "mvn"
        wrapper.parent.mkdir(parents=True)
        wrapper.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        wrapper.chmod(0o700)
        launcher = distribution / "bin" / "mvn"
        launcher.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        launcher.chmod(0o700)
        (distribution / "boot" / "classworlds.jar").write_bytes(b"boot-v1")
        configuration = distribution / "conf" / "settings.xml"
        configuration.write_bytes(b"<settings/>")
        library = distribution / "lib" / "maven-core.jar"
        library.write_bytes(b"core-v1")
        return wrapper, launcher, configuration, library

    def test_distribution_mutation_invalidates_review_with_wrapper_unchanged(self) -> None:
        wrapper, _launcher, _configuration, library = self._distribution()
        environment = {
            "HOME": os.fspath(self.root),
            "PATH": os.fspath(wrapper.parent),
        }
        with mock.patch.dict(os.environ, environment, clear=True):
            executable, first_digest = pilot.maven_executable()
        wrapper_digest = pilot.file_digest(wrapper)
        self.assertEqual(
            executable,
            (wrapper.parent.parent / "libexec" / "bin" / "mvn").resolve(strict=True),
        )

        fixture = shape_builder.SealedFixture(
            "0" * 40, f"sha256:{'1' * 64}", tuple()
        )
        ingredients = {
            "builderDigest": f"sha256:{'2' * 64}",
            "pilotRunnerDigest": f"sha256:{'3' * 64}",
            "g1kEvidenceDigest": f"sha256:{'4' * 64}",
            "publicFixtureTreeOid": fixture.tree_oid,
            "publicFixtureContentDigest": fixture.content_digest,
            "testDigest": f"sha256:{'5' * 64}",
            "localModuleManifest": {"authorityDigest": f"sha256:{'6' * 64}"},
            "gitDigest": f"sha256:{'7' * 64}",
            "gitEnvironmentDigest": f"sha256:{'8' * 64}",
            "mavenDigest": first_digest,
        }
        unsigned = {
            "schema": shape_builder.REVIEW_SCHEMA,
            **ingredients,
            "verdict": "PASS",
            "findings": [],
        }
        review = {
            **unsigned,
            "authorityDigest": shape_builder.authority_digest(unsigned),
        }
        shape_builder.validate_review_manifest_value(
            review,
            builder_digest=ingredients["builderDigest"],
            pilot_runner_digest=ingredients["pilotRunnerDigest"],
            g1k_digest=ingredients["g1kEvidenceDigest"],
            fixture=fixture,
            test_digest=ingredients["testDigest"],
            module_manifest=ingredients["localModuleManifest"],
            git_digest=ingredients["gitDigest"],
            git_environment_digest=ingredients["gitEnvironmentDigest"],
            maven_digest=first_digest,
        )

        library.write_bytes(b"core-v2")
        with mock.patch.dict(os.environ, environment, clear=True):
            _, second_digest = pilot.maven_executable()
            _, builder_digest = shape_builder.maven_executable(environment)
        self.assertEqual(pilot.file_digest(wrapper), wrapper_digest)
        self.assertNotEqual(second_digest, first_digest)
        self.assertEqual(builder_digest, second_digest)
        with self.assertRaisesRegex(
            shape_builder.BuilderError, "INVALID_INDEPENDENT_REVIEW"
        ):
            shape_builder.validate_review_manifest_value(
                review,
                builder_digest=ingredients["builderDigest"],
                pilot_runner_digest=ingredients["pilotRunnerDigest"],
                g1k_digest=ingredients["g1kEvidenceDigest"],
                fixture=fixture,
                test_digest=ingredients["testDigest"],
                module_manifest=ingredients["localModuleManifest"],
                git_digest=ingredients["gitDigest"],
                git_environment_digest=ingredients["gitEnvironmentDigest"],
                maven_digest=second_digest,
            )

    def test_actual_launcher_config_and_jar_are_each_digest_bound(self) -> None:
        for index, selected in enumerate((1, 2, 3)):
            with self.subTest(selected=selected):
                wrapper, launcher, configuration, library = self._distribution(
                    f"maven-{index}"
                )
                environment = {"PATH": os.fspath(wrapper.parent)}
                with mock.patch.dict(os.environ, environment, clear=True):
                    executable, before = pilot.maven_executable()
                self.assertEqual(executable, launcher.resolve(strict=True))
                target = (launcher, configuration, library)[selected - 1]
                target.write_bytes(target.read_bytes() + b"-changed")
                if target == launcher:
                    target.chmod(0o700)
                with mock.patch.dict(os.environ, environment, clear=True):
                    _, after = pilot.maven_executable()
                self.assertNotEqual(before, after)

    def test_symlinked_authority_directory_is_rejected(self) -> None:
        wrapper, _launcher, configuration, _library = self._distribution()
        authority_directory = configuration.parent
        shutil.rmtree(authority_directory)
        external = self.root / "external-conf"
        external.mkdir()
        (external / "settings.xml").write_bytes(b"<settings/>")
        authority_directory.symlink_to(external, target_is_directory=True)
        with mock.patch.dict(
            os.environ, {"PATH": os.fspath(wrapper.parent)}, clear=True
        ):
            with self.assertRaisesRegex(
                pilot.PilotError, "INVALID_MAVEN_EXECUTABLE"
            ):
                pilot.maven_executable()

    def test_semantic_phase_rechecks_distribution_after_execution(self) -> None:
        wrapper, _launcher, _configuration, library = self._distribution()
        rustc = shape_builder._environment_executable(
            "rustc", dict(os.environ), "INVALID_RUSTC_EXECUTABLE"
        )
        cargo = shape_builder._environment_executable(
            "cargo", dict(os.environ), "INVALID_CARGO_EXECUTABLE"
        )
        environment = {
            "HOME": os.fspath(self.root),
            "PATH": ":".join(
                dict.fromkeys(
                    [
                        os.fspath(wrapper.parent),
                        os.fspath(rustc.parent),
                        os.fspath(cargo.parent),
                    ]
                )
            ),
        }
        with mock.patch.dict(os.environ, environment, clear=True):
            _, expected = shape_builder.maven_executable(environment)

            def mutate(*_args: object, **_kwargs: object) -> tuple[int, bytes, bytes]:
                library.write_bytes(b"mutated-during-semantic-phase")
                return 0, b"{}", b""

            with mock.patch.object(
                shape_builder, "_run_bounded_process", side_effect=mutate
            ):
                with self.assertRaisesRegex(
                    shape_builder.BuilderError, "MAVEN_AUTHORITY_CHANGED"
                ):
                    shape_builder._run_json(
                        Path("clew"), ["thread", "impact"], 1, expected
                    )

    def test_distribution_verifier_round_trips_non_file_digest(self) -> None:
        wrapper, launcher, _configuration, _library = self._distribution()
        environment = {"PATH": os.fspath(wrapper.parent)}
        with mock.patch.dict(os.environ, environment, clear=True):
            executable, distribution_digest = pilot.maven_executable()
            verified = pilot._require_maven_authority(distribution_digest)
        with mock.patch.dict(
            os.environ, {"PATH": os.fspath(launcher.parent)}, clear=True
        ):
            pinned_executable, pinned_digest = pilot.maven_executable()
        self.assertEqual(executable, launcher.resolve(strict=True))
        self.assertEqual(verified, executable)
        self.assertEqual(pinned_executable, executable)
        self.assertEqual(pinned_digest, distribution_digest)
        self.assertNotEqual(pilot.file_digest(executable), distribution_digest)

    def test_file_swap_growth_and_node_overflow_fail_closed(self) -> None:
        module = pilot.maven_distribution_authority

        wrapper, _launcher, _configuration, library = self._distribution(
            "maven-swap"
        )
        replacement = self.root / "replacement.jar"
        replacement.write_bytes(b"replacement")
        original_open = os.open
        parent_descriptor = original_open(
            library.parent,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )

        def swap_after_open(
            path: object, flags: int, *args: object, **kwargs: object
        ) -> int:
            descriptor = original_open(path, flags, *args, **kwargs)
            if path == library.name:
                library.unlink()
                library.symlink_to(replacement)
            return descriptor

        try:
            with mock.patch.object(module.os, "open", side_effect=swap_after_open):
                with self.assertRaisesRegex(
                    module.MavenAuthorityError, "MAVEN_AUTHORITY_CHANGED"
                ):
                    module._file_row_at(
                        parent_descriptor,
                        library.name,
                        "lib/maven-core.jar",
                        {"nodes": 0, "files": 0, "bytes": 0},
                    )
        finally:
            os.close(parent_descriptor)

        _wrapper, _launcher, _configuration, growing = self._distribution(
            "maven-growth"
        )
        original_read = os.read
        grew = False

        def grow_after_read(descriptor: int, size: int) -> bytes:
            nonlocal grew
            chunk = original_read(descriptor, size)
            if chunk and not grew:
                grew = True
                with growing.open("ab") as stream:
                    stream.write(b"-growth")
            return chunk

        growth_parent = os.open(
            growing.parent,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        try:
            with mock.patch.object(module.os, "read", side_effect=grow_after_read):
                with self.assertRaisesRegex(
                    module.MavenAuthorityError, "MAVEN_AUTHORITY_CHANGED"
                ):
                    module._file_row_at(
                        growth_parent,
                        growing.name,
                        "lib/maven-core.jar",
                        {"nodes": 0, "files": 0, "bytes": 0},
                    )
        finally:
            os.close(growth_parent)

        overflow_wrapper, *_ = self._distribution("maven-overflow")
        with (
            mock.patch.object(module, "MAX_NODES", 2),
            mock.patch.dict(
                os.environ,
                {"PATH": os.fspath(overflow_wrapper.parent)},
                clear=True,
            ),
        ):
            with self.assertRaisesRegex(
                pilot.PilotError, "INVALID_MAVEN_EXECUTABLE"
            ):
                pilot.maven_executable()

    def test_top_level_and_nested_directory_swaps_fail_closed(self) -> None:
        module = pilot.maven_distribution_authority
        original_scandir = os.scandir
        for index, nested in enumerate((False, True)):
            with self.subTest(nested=nested):
                wrapper, _launcher, configuration, library = self._distribution(
                    f"maven-directory-swap-{index}"
                )
                target = configuration.parent
                if nested:
                    target = library.parent / "extensions"
                    target.mkdir()
                    (target / "extension.jar").write_bytes(b"extension")
                target_identity = target.stat().st_ino
                swapped = False

                def swap_directory(descriptor: int) -> object:
                    nonlocal swapped
                    if not swapped and os.fstat(descriptor).st_ino == target_identity:
                        swapped = True
                        retained = target.with_name(f"{target.name}-retained")
                        target.rename(retained)
                        external = target.with_name(f"{target.name}-external")
                        external.mkdir()
                        (external / "substitute.jar").write_bytes(b"substitute")
                        target.symlink_to(external, target_is_directory=True)
                    return original_scandir(descriptor)

                with (
                    mock.patch.object(module.os, "scandir", side_effect=swap_directory),
                    mock.patch.dict(
                        os.environ,
                        {"PATH": os.fspath(wrapper.parent)},
                        clear=True,
                    ),
                ):
                    with self.assertRaisesRegex(
                        pilot.PilotError, "INVALID_MAVEN_EXECUTABLE"
                    ):
                        pilot.maven_executable()
                self.assertTrue(swapped)

    def test_total_byte_budget_stops_before_reading_next_file(self) -> None:
        module = pilot.maven_distribution_authority
        wrapper, launcher, _configuration, _library = self._distribution(
            "maven-total-budget"
        )
        original_read = os.read
        with (
            mock.patch.object(
                module, "MAX_TOTAL_BYTES", launcher.stat().st_size
            ),
            mock.patch.object(module.os, "read", wraps=original_read) as read_call,
            mock.patch.dict(
                os.environ,
                {"PATH": os.fspath(wrapper.parent)},
                clear=True,
            ),
        ):
            with self.assertRaisesRegex(
                pilot.PilotError, "INVALID_MAVEN_EXECUTABLE"
            ):
                pilot.maven_executable()
        # bin/mvn is the first and only file within the reduced budget: one
        # data read plus EOF. The boot file that exceeds the total is rejected
        # from fstat metadata before any read from its descriptor.
        self.assertEqual(read_call.call_count, 2)


class ShapeBuilderProgressTest(_ShapeBuilderProgressHelpers):

    def test_accepts_only_closed_success_progress(self) -> None:
        completed = {"durationMs": 7, "event": "request_completed", "success": True}
        shape_builder._validate_clew_progress(self.stream(completed))
        valid = self.emitted_stream(
            self.progress("DAG_STARTED", None, queued=1),
            self.progress("STAGE_STARTED", "adapter-analysis", running=1),
            self.progress("HEARTBEAT", None, running=1),
            self.progress("STAGE_COMPLETED", "adapter-analysis", done=1),
            self.progress("DAG_READY", None, done=1),
        ) + self.stream(completed)
        shape_builder._validate_clew_progress(valid)
        cold_capsule = self.stream(
            self.capsule("STAGE_STARTED", "GRADLE_WORKERS"),
            self.capsule("STAGE_STARTED", "CARGO_BINARIES"),
            self.capsule("HEARTBEAT", "CARGO_BINARIES", durationMillis=5),
            self.capsule(
                "STAGE_COMPLETED", "GRADLE_WORKERS", durationMillis=7
            ),
            self.capsule(
                "STAGE_COMPLETED", "CARGO_BINARIES", durationMillis=11
            ),
            self.capsule("STAGE_STARTED", "ASSEMBLE_VERIFIED_COMPONENTS"),
            self.capsule("STAGE_COMPLETED", "ASSEMBLE_VERIFIED_COMPONENTS"),
        ) + valid
        shape_builder._validate_clew_progress(cold_capsule)
        invalid = [
            b"",
            self.stream({"durationMs": 7, "event": "request_completed", "success": False}),
            self.emitted_stream(self.progress("DAG_STARTED", None)) + self.stream(completed),
            self.emitted_stream(self.progress("STAGE_FAILED", "adapter-analysis")) + self.stream(completed),
            self.emitted_stream(
                self.progress("DAG_STARTED", None),
                self.progress("STAGE_COMPLETED", "adapter-analysis"),
                self.progress("DAG_READY", None),
            ) + self.stream(completed),
            self.stream(self.progress("DAG_STARTED", None)) + self.stream(completed),
            self.stream({"event": "unknown"}, completed),
            self.stream(completed, completed),
            self.stream(
                self.capsule("STAGE_COMPLETED", "CARGO_BINARIES"),
                completed,
            ),
            self.stream(
                self.capsule("STAGE_STARTED", "CARGO_BINARIES"),
                self.capsule("STAGE_STARTED", "CARGO_BINARIES"),
                completed,
            ),
            self.stream(
                self.capsule("STAGE_STARTED", "CARGO_BINARIES"),
                completed,
            ),
            self.emitted_stream(
                self.progress("DAG_STARTED", None),
                self.progress("DAG_READY", None),
            )
            + self.stream(
                self.capsule("STAGE_STARTED", "CARGO_BINARIES"),
                self.capsule("STAGE_COMPLETED", "CARGO_BINARIES"),
                completed,
            ),
            self.stream(
                self.capsule(
                    "STAGE_FAILED",
                    "CARGO_BINARIES",
                    durationMillis=1,
                    exitCode=1,
                ),
                completed,
            ),
            b'{"event":"request_completed", "durationMs":7,"success":true}\n',
            self.stream(completed).replace(b"\n", b"\r\n"),
        ]
        for stream in invalid:
            with self.subTest(stream=stream):
                with self.assertRaisesRegex(
                    shape_builder.BuilderError, "COMPILER_EVIDENCE_FAILED"
                ):
                    shape_builder._validate_clew_progress(stream)


class ShapeBuilderImpactDiscoveryTest(unittest.TestCase):
    def test_token_discovery_is_member_scoped_before_exact_confirmation(self) -> None:
        candidates = {
            member: shape_builder.CompilerDeclaration(
                member_alias=member,
                side=member.upper(),
                symbol_identity=f"function:example/{member}.Needle():kotlin/Unit",
                compiler_name="Needle",
                declaration_kind="FUNCTION",
                descriptor_class="FUNCTION",
                projected_shape={"ownerIdentity": f"class:example/{member}"},
                shape_digest=shape_builder.authority_digest({"member": member}),
                source={"path": f"src/{member}.kt", "start": 1, "end": 2},
            )
            for member in ("provider", "consumer")
        }

        def declaration(
            _finding: object, *, member_alias: str, exact: bool
        ) -> shape_builder.CompilerDeclaration:
            self.assertIsInstance(exact, bool)
            return candidates[member_alias]

        with (
            mock.patch.object(
                shape_builder,
                "run_impact",
                return_value={"findings": [{}]},
            ) as run_impact,
            mock.patch.object(
                shape_builder,
                "declaration_from_finding",
                side_effect=declaration,
            ),
        ):
            confirmed = shape_builder._confirmed_declarations(
                Path("/private/clew"),
                "thread:test",
                "thread-callables:test",
                "pair-test",
                ["Needle"],
                10,
                f"sha256:{'9' * 64}",
            )

        token_calls = [
            call
            for call in run_impact.call_args_list
            if call.args[4] == "token"
        ]
        self.assertEqual(
            [call.kwargs["member"] for call in token_calls],
            ["provider", "consumer"],
        )
        self.assertTrue(
            all(call.kwargs["declarations_only"] is True for call in token_calls)
        )
        self.assertTrue(
            all(call.kwargs["declaration_name_only"] is True for call in token_calls)
        )
        self.assertEqual(confirmed, {
            "provider": [candidates["provider"]],
            "consumer": [candidates["consumer"]],
        })


class PilotHarnessTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name).resolve(strict=True)
        self.git, _ = pilot.git_executable()
        self.repositories: dict[str, tuple[Path, str, str]] = {}
        for index, alias in enumerate(["service-01", "service-02"], 1):
            repository = self.root / alias
            repository.mkdir()
            subprocess.run([self.git, "-C", repository, "init", "-q", "-b", "pilot"], check=True)
            relative = "src/main/kotlin/com/acme/Sample.kt"
            source = repository / relative
            source.parent.mkdir(parents=True)
            source.write_text(
                f"package com.acme\npublic fun sample{index}(value: String): String = value\n",
                encoding="utf-8",
            )
            subprocess.run([self.git, "-C", repository, "add", relative], check=True)
            environment = {
                **os.environ,
                "GIT_AUTHOR_NAME": "Codeclew Pilot",
                "GIT_AUTHOR_EMAIL": "pilot" + "@" + "example.invalid",
                "GIT_COMMITTER_NAME": "Codeclew Pilot",
                "GIT_COMMITTER_EMAIL": "pilot" + "@" + "example.invalid",
                "GIT_AUTHOR_DATE": "2000-01-01T00:00:00Z",
                "GIT_COMMITTER_DATE": "2000-01-01T00:00:00Z",
            }
            subprocess.run(
                [self.git, "-C", repository, "-c", "commit.gpgsign=false", "commit", "-q", "-m", "fixture"],
                check=True,
                env=environment,
            )
            revision = subprocess.check_output([self.git, "-C", repository, "rev-parse", "HEAD"], text=True).strip()
            blob = subprocess.check_output([self.git, "-C", repository, "rev-parse", f"HEAD:{relative}"], text=True).strip()
            self.repositories[alias] = (repository, revision, blob)
        self.authority = self._authority()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_r2_authorities_are_consistent_across_pilot_modules(self) -> None:
        self.assertEqual(pilot.FROZEN_AT, descriptor_gate.FROZEN_AT)
        self.assertEqual(
            public_verifier.EXPECTED_PRIVATE_CORPUS_DIGEST,
            descriptor_gate.EXPECTED_CORPUS_DIGEST,
        )
        self.assertEqual(
            public_verifier.EXPECTED_BENCHMARK_DIGEST,
            descriptor_gate.EXPECTED_BENCHMARK_DIGEST,
        )
        self.assertEqual(public_verifier.EXPECTED_PAIRS, ["pair-01"] * 10)

    def _authority(self) -> dict[str, object]:
        repositories = [
            {"serviceAlias": alias, "path": os.fspath(row[0]), "revision": row[1]}
            for alias, row in sorted(self.repositories.items())
        ]
        return {
            "budgets": pilot.BUDGETS,
            "semanticEnvironment": {"PATH": "/usr/bin:/bin"},
            "executables": {
                "clew": "/bin/true",
                "git": os.fspath(self.git),
            },
            "repositories": repositories,
            "sessions": [
                {
                    "serviceAlias": alias,
                    "sessionId": f"session:{index:064x}",
                    "sessionAuthorityDigest": f"sha256:{index:064x}",
                    "runtimeKey": f"sha256:{9:064x}",
                    "runtimeMode": "RELEASE",
                }
                for index, alias in enumerate(sorted(self.repositories), 1)
            ],
            "tasks": [
                {
                    "taskId": "task-01",
                    "pairId": "pair-01",
                    "provider": "service-01",
                    "consumer": "service-02",
                    "manualVerification": [
                        {"category": "ROUTE", "requiredCheck": "VERIFY_ROUTE"}
                    ],
                    "thread": {
                        "threadId": f"thread:sha256:{7:064x}",
                        "threadAuthorityDigest": f"sha256:{8:064x}",
                        "providerMember": "provider",
                        "consumerMember": "consumer",
                    },
                }
            ],
        }

    def _prepare_publication_fixture(
        self,
    ) -> tuple[
        Path,
        Path,
        Path,
        Path,
        dict[str, str],
        str,
        dict[str, str],
        list[dict[str, str]],
        dict[str, object],
        dict[str, object],
    ]:
        true_executable = shutil.which("true")
        self.assertIsNotNone(true_executable)
        clew = Path(true_executable).resolve(strict=True)
        authority_path = self.root / "pilot-authority.json"
        oracle_path = self.root / "pilot-oracle.json"
        pending = pilot._pending_path(authority_path)
        environment = {
            "HOME": os.fspath(self.root), "PATH": "/usr/bin:/bin",
            "LANG": "C", "LC_ALL": "C",
        }
        runtime_key = f"sha256:{'7' * 64}"
        threads = {"task-01": "thread:publication"}
        sessions = [
            {
                "serviceAlias": "service-01",
                "sessionId": "session:publication",
                "sessionAuthorityDigest": f"sha256:{'8' * 64}",
                "runtimeKey": runtime_key,
                "runtimeMode": "RELEASE",
            }
        ]
        pilot._write_pending(
            pending,
            clew,
            environment,
            runtime_key,
            threads,
            sessions,
            "PREPARING",
        )
        authority: dict[str, object] = {
            "schema": "fixture-authority/1.0",
            "value": "authority",
        }
        oracle: dict[str, object] = {
            "schema": "fixture-oracle/1.0",
            "value": "oracle",
        }
        return (
            clew,
            authority_path,
            oracle_path,
            pending,
            environment,
            runtime_key,
            threads,
            sessions,
            authority,
            oracle,
        )

    def test_private_input_requires_canonical_0600_regular_file(self) -> None:
        path = (self.root / "private.json").resolve()
        path.write_bytes(pilot.canonical_bytes({"value": 1}) + b"\n")
        os.chmod(path, 0o600)
        _, value, _ = pilot.private_json(path, "TEST")
        self.assertEqual(value, {"value": 1})
        os.chmod(path, 0o644)
        with self.assertRaises(pilot.PilotError):
            pilot.private_json(path, "TEST")

    def test_experiment_root_is_fresh_owned_and_private_paths_are_direct(self) -> None:
        experiment = self.root / "experiment"
        state = experiment / "codeclew-state"
        temporary = experiment / "tmp"
        experiment.mkdir(mode=0o700)
        state.mkdir(mode=0o700)
        temporary.mkdir(mode=0o700)
        with mock.patch.dict(
            os.environ,
            {"CODECLEW_HOME": os.fspath(state), "TMPDIR": os.fspath(temporary)},
            clear=False,
        ):
            root, authority = pilot._experiment_root(experiment)
        self.assertEqual(root, experiment)
        self.assertEqual(authority["inode"], experiment.stat().st_ino)
        pilot._require_experiment_paths(
            root, [experiment / "corpus.json", experiment / "run.json"]
        )
        nested = experiment / "nested"
        nested.mkdir(mode=0o700)
        with self.assertRaisesRegex(
            pilot.PilotError, "EXPERIMENT_PATH_AUTHORITY_INVALID"
        ):
            pilot._require_experiment_paths(root, [nested / "escape.json"])

    def test_answer_size_is_rejected_before_reading_sparse_body(self) -> None:
        path = self.root / "answer.json"
        with path.open("wb") as stream:
            stream.truncate(pilot.BUDGETS["answerBytes"] + 1)
        os.chmod(path, 0o600)
        with self.assertRaisesRegex(pilot.PilotError, "ANSWER_SIZE_INVALID"):
            pilot._bounded_private_answer(path)

    def test_phase_outputs_are_create_once_and_reviews_are_digest_bound(self) -> None:
        output = self.root / "new-output.json"
        self.assertEqual(pilot.fresh_output_target(output), output)
        output.write_text("occupied", encoding="utf-8")
        with self.assertRaisesRegex(pilot.PilotError, "OUTPUT_ALREADY_EXISTS"):
            pilot.fresh_output_target(output)
        with self.assertRaisesRegex(pilot.PilotError, "OUTPUT_PATH_COLLISION"):
            pilot.require_distinct_paths([output], [output])

        digest_keys = {
            "runnerDigest", "brokerDigest", "publicVerifierDigest",
            "answerSchemaDigest", "warmAuditAdapterDigest",
            "shapeOracleBuilderDigest",
        }
        module_manifest = pilot.local_module_manifest()
        authority = {
            "authorityDigest": f"sha256:{'a' * 64}",
            "protocolDigest": f"sha256:{'b' * 64}",
            "inputs": {
                **{key: f"sha256:{index:064x}" for index, key in enumerate(sorted(digest_keys), 1)},
                "localModuleManifest": module_manifest,
                "localModuleManifestDigest": module_manifest["authorityDigest"],
                "benchmarkDigest": f"sha256:{'c' * 64}",
            },
        }
        implementation_unsigned = {
            "schema": pilot.IMPLEMENTATION_REVIEW_SCHEMA,
            "protocolDigest": authority["protocolDigest"],
            **{key: authority["inputs"][key] for key in digest_keys},
            "localModuleManifest": module_manifest,
            "localModuleManifestDigest": module_manifest["authorityDigest"],
            "verdict": "PASS",
            "findings": [],
        }
        implementation = {
            **implementation_unsigned,
            "authorityDigest": pilot.authority_digest(implementation_unsigned),
        }
        implementation_path = self.root / "implementation-review.json"
        implementation_path.write_bytes(pilot.canonical_bytes(implementation) + b"\n")
        _, implementation_digest = pilot._implementation_review(
            implementation_path, authority
        )
        self.assertEqual(implementation_digest, implementation["authorityDigest"])

        value_unsigned = {
            "schema": pilot.VALUE_REVIEW_SCHEMA,
            "pilotAuthorityDigest": authority["authorityDigest"],
            "runDigest": f"sha256:{'d' * 64}",
            "warmAttestationDigest": f"sha256:{'e' * 64}",
            "draftMetricsDigest": f"sha256:{'f' * 64}",
            "benchmarkDigest": authority["inputs"]["benchmarkDigest"],
            "verdict": "PASS",
            "findings": [],
        }
        value = {
            **value_unsigned,
            "authorityDigest": pilot.authority_digest(value_unsigned),
        }
        value_path = self.root / "value-review.json"
        value_path.write_bytes(pilot.canonical_bytes(value) + b"\n")
        _, value_digest = pilot._value_review(
            value_path,
            authority,
            value_unsigned["runDigest"],
            value_unsigned["warmAttestationDigest"],
            value_unsigned["draftMetricsDigest"],
        )
        self.assertEqual(value_digest, value["authorityDigest"])
        value["runDigest"] = f"sha256:{'1' * 64}"
        value_path.write_bytes(pilot.canonical_bytes(value) + b"\n")
        with self.assertRaisesRegex(pilot.PilotError, "VALUE_REVIEW_INVALID"):
            pilot._value_review(
                value_path,
                authority,
                value_unsigned["runDigest"],
                value_unsigned["warmAttestationDigest"],
                value_unsigned["draftMetricsDigest"],
            )

    def test_local_module_manifest_and_run_creation_are_exact_and_single_winner(self) -> None:
        manifest = pilot.local_module_manifest()
        self.assertEqual(pilot._verify_local_module_authority(manifest), manifest)
        substituted = json.loads(json.dumps(manifest))
        substituted["modules"][0]["digest"] = f"sha256:{'f' * 64}"
        unsigned = dict(substituted)
        unsigned.pop("authorityDigest")
        substituted["authorityDigest"] = pilot.authority_digest(unsigned)
        with self.assertRaisesRegex(
            pilot.PilotError, "LOCAL_MODULE_AUTHORITY_CHANGED"
        ):
            pilot._verify_local_module_authority(substituted)

        output = self.root / "single-winner-run.json"
        barrier = threading.Barrier(4)
        winners: list[str] = []
        failures: list[str] = []

        def contender(index: int) -> None:
            token = f"{index:064x}"
            value = {
                "schema": pilot.PRIVATE_RUN_SCHEMA,
                "ownerPid": os.getpid(),
                "ownerToken": token,
            }
            barrier.wait()
            try:
                pilot._create_private_once(
                    output, value, "PRIVATE_RUN_CREATE_FAILED"
                )
                winners.append(token)
            except pilot.PilotError as error:
                failures.append(error.code)

        threads = [threading.Thread(target=contender, args=(index,)) for index in range(1, 5)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=5)
            self.assertFalse(thread.is_alive())
        self.assertEqual(len(winners), 1)
        self.assertEqual(failures, ["PRIVATE_RUN_CREATE_FAILED"] * 3)
        _, stored, _ = pilot.private_json(output, "PRIVATE_RUN")
        self.assertEqual(stored["ownerToken"], winners[0])

        experiment_root = self.root / "experiment"
        experiment_root.mkdir(mode=0o700)
        root_metadata = experiment_root.stat()
        authority = {
            "authorityDigest": f"sha256:{'a' * 64}",
            "protocolDigest": f"sha256:{'b' * 64}",
            "experimentRoot": {
                "path": os.fspath(experiment_root),
                "device": root_metadata.st_dev,
                "inode": root_metadata.st_ino,
            },
        }
        owner = {"ownerPid": os.getpid(), "ownerToken": "c" * 64}
        first_run = experiment_root / "pilot-run.json"
        marker, admitted = pilot._admit_execute(
            experiment_root, authority, first_run, owner
        )
        self.assertTrue(marker.exists())
        self.assertEqual(
            pilot._verify_execute_admission(experiment_root, authority, first_run),
            admitted,
        )
        alternate = experiment_root / "favorable-alternate-run.json"
        with self.assertRaisesRegex(
            pilot.PilotError, "EXECUTE_ALREADY_ADMITTED"
        ):
            pilot._admit_execute(experiment_root, authority, alternate, owner)
        with self.assertRaisesRegex(
            pilot.PilotError, "EXECUTE_ADMISSION_INVALID"
        ):
            pilot._verify_execute_admission(experiment_root, authority, alternate)

    def test_private_oracle_nested_shape_fails_with_typed_locator_free_error(self) -> None:
        valid_manual = [
            {"category": "ALPHA", "requiredCheck": "VERIFY_ALPHA"},
            {"category": "BETA", "requiredCheck": "VERIFY_BETA"},
        ]
        self.assertEqual(
            pilot._validate_manual_verification(valid_manual, 2, "INVALID_PILOT_ORACLE"),
            valid_manual,
        )
        malformed_manual = [
            ["ALPHA", {"category": "BETA", "requiredCheck": "VERIFY_BETA"}],
            [
                {"category": "ALPHA", "requiredCheck": "VERIFY_WRONG"},
                {"category": "BETA", "requiredCheck": "VERIFY_BETA"},
            ],
            list(reversed(valid_manual)),
        ]
        for rows in malformed_manual:
            with self.assertRaisesRegex(
                pilot.PilotError, "INVALID_PILOT_ORACLE"
            ):
                pilot._validate_manual_verification(
                    rows, 2, "INVALID_PILOT_ORACLE"
                )

        unsigned = {
            "schema": pilot.PRIVATE_ORACLE_SCHEMA,
            "protocolDigest": f"sha256:{'1' * 64}",
            "shapeOracleDigest": f"sha256:{'2' * 64}",
            "fixture": [None] * 5,
            "tasks": [None] * 10,
        }
        oracle = {
            **unsigned,
            "authorityDigest": pilot.authority_digest(unsigned),
        }
        authority = {
            "protocolDigest": unsigned["protocolDigest"],
            "inputs": {
                "pilotOracleDigest": oracle["authorityDigest"],
                "shapeOracleDigest": unsigned["shapeOracleDigest"],
            },
            "tasks": [{} for _ in range(10)],
        }
        with self.assertRaises(pilot.PilotError) as raised:
            pilot.verify_oracle(oracle, authority)
        self.assertEqual(raised.exception.code, "INVALID_DECLARATION")
        self.assertNotIn(os.fspath(self.root), str(raised.exception))

    def test_project_draft_is_closed_create_once_and_cli_ordered(self) -> None:
        unsigned = {
            "schema": pilot.PRIVATE_DRAFT_SCHEMA,
            "status": "DRAFT",
            "pilotAuthorityDigest": f"sha256:{'1' * 64}",
            "protocolDigest": f"sha256:{'2' * 64}",
            "runDigest": f"sha256:{'3' * 64}",
            "warmAttestationDigest": f"sha256:{'4' * 64}",
            "implementationReviewManifestDigest": f"sha256:{'7' * 64}",
            "draftMetricsDigest": f"sha256:{'5' * 64}",
            "fixture": {"result": "FAIL"},
            "comparison": {"taskResults": []},
            "warmAudit": {"runCount": 30},
        }
        draft = {**unsigned, "draftDigest": pilot.authority_digest(unsigned)}
        self.assertEqual(pilot._verify_private_draft(draft, draft), draft)
        substituted = dict(draft)
        substituted["runDigest"] = f"sha256:{'6' * 64}"
        substituted_unsigned = dict(substituted)
        substituted_unsigned.pop("draftDigest")
        substituted["draftDigest"] = pilot.authority_digest(substituted_unsigned)
        with self.assertRaisesRegex(pilot.PilotError, "PRIVATE_DRAFT_MISMATCH"):
            pilot._verify_private_draft(substituted, draft)

        draft_path = self.root / "pilot-draft.json"
        pilot._create_private_once(draft_path, draft, "DRAFT_TEST_FAILED")
        with self.assertRaisesRegex(pilot.PilotError, "DRAFT_TEST_FAILED"):
            pilot._create_private_once(draft_path, draft, "DRAFT_TEST_FAILED")

        common = [
            "--experiment-root", "root",
            "--private-authority", "authority.json",
            "--private-oracle", "oracle.json",
            "--private-run", "run.json",
            "--private-warm", "warm.json",
        ]
        parsed_draft = pilot.parser().parse_args(
            [
                "project", "draft", *common,
                "--implementation-review-manifest", "implementation.json",
                "--private-draft-output", "draft.json",
            ]
        )
        self.assertEqual(parsed_draft.project_action, "draft")
        parsed_publish = pilot.parser().parse_args(
            [
                "project", "publish", *common, "--private-draft", "draft.json",
                "--implementation-review-manifest", "implementation.json",
                "--value-review-manifest", "value.json",
                "--checked-output", "evidence.json",
            ]
        )
        self.assertEqual(parsed_publish.project_action, "publish")
        parsed_execute = pilot.parser().parse_args(
            [
                "execute", "--private-authority", "authority.json",
                "--experiment-root", "root",
                "--private-oracle", "oracle.json",
                "--implementation-review-manifest", "implementation.json",
                "--private-output", "run.json",
            ]
        )
        self.assertEqual(
            parsed_execute.implementation_review_manifest,
            Path("implementation.json"),
        )

    def test_prepare_pair_publication_activates_only_after_both_outputs_are_durable(
        self,
    ) -> None:
        (
            clew,
            authority_path,
            oracle_path,
            pending,
            environment,
            runtime_key,
            threads,
            sessions,
            authority,
            oracle,
        ) = self._prepare_publication_fixture()
        original_write_pending = pilot._write_pending
        original_link = os.link
        statuses: list[str] = []
        final_links: list[Path] = []

        def observed_write_pending(*args: object, **kwargs: object) -> None:
            status = args[6]
            self.assertIsInstance(status, str)
            statuses.append(status)
            if status == "ACTIVE":
                for path, expected in (
                    (authority_path, authority),
                    (oracle_path, oracle),
                ):
                    self.assertEqual(
                        path.read_bytes(), pilot.canonical_bytes(expected) + b"\n"
                    )
                    self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            original_write_pending(*args, **kwargs)

        def observed_link(
            source: os.PathLike[str] | str,
            target: os.PathLike[str] | str,
            *args: object,
            **kwargs: object,
        ) -> None:
            target_path = Path(target)
            if target_path in {authority_path, oracle_path}:
                publication = pilot._prepare_publication_path(authority_path)
                self.assertTrue(publication.exists())
                self.assertEqual(stat.S_IMODE(publication.stat().st_mode), 0o600)
                ledger = pilot._read_prepare_publication(
                    publication, authority_path, oracle_path
                )
                self.assertTrue(
                    all(
                        pilot._publication_file_exact(
                            Path(row["stagePath"]), row
                        )
                        for row in ledger["outputs"]
                    )
                )
                final_links.append(target_path)
            original_link(source, target, *args, **kwargs)

        with mock.patch.object(
            pilot, "_write_pending", side_effect=observed_write_pending
        ), mock.patch.object(pilot.os, "link", side_effect=observed_link):
            pilot._publish_prepare_pair(
                authority_path,
                authority,
                oracle_path,
                oracle,
                pending,
                clew,
            )

        self.assertEqual(statuses, ["ACTIVE"])
        self.assertEqual(final_links, [oracle_path, authority_path])
        _, resource, _ = pilot.private_json(pending, "SEMANTIC_PENDING")
        self.assertEqual(resource["status"], "ACTIVE")
        self.assertFalse(pilot._prepare_publication_path(authority_path).exists())
        self.assertEqual(list(self.root.glob("*.stage")), [])

    def test_dead_prepare_owner_finalizes_from_exact_stages(self) -> None:
        (
            clew,
            authority_path,
            oracle_path,
            pending,
            _environment,
            _runtime_key,
            _threads,
            _sessions,
            authority,
            oracle,
        ) = self._prepare_publication_fixture()
        publication, ledger = pilot._begin_prepare_publication(
            authority_path,
            authority,
            oracle_path,
            oracle,
            pending,
            clew,
        )
        first = ledger["outputs"][0]
        self.assertIsInstance(first, dict)
        os.link(Path(first["stagePath"]), Path(first["finalPath"]))
        with mock.patch.object(pilot, "_publication_owner_alive", return_value=False):
            status = pilot._recover_prepare_publication(
                clew, authority_path, oracle_path
            )
        self.assertEqual(status, "FINALIZED")
        self.assertEqual(
            authority_path.read_bytes(), pilot.canonical_bytes(authority) + b"\n"
        )
        self.assertEqual(
            oracle_path.read_bytes(), pilot.canonical_bytes(oracle) + b"\n"
        )
        _, resource, _ = pilot.private_json(pending, "SEMANTIC_PENDING")
        self.assertEqual(resource["status"], "ACTIVE")
        self.assertFalse(publication.exists())

    def test_dead_prepare_owner_recovers_complete_creating_ledger(self) -> None:
        (
            clew,
            authority_path,
            oracle_path,
            pending,
            _environment,
            _runtime_key,
            _threads,
            _sessions,
            authority,
            oracle,
        ) = self._prepare_publication_fixture()
        publication, _ledger = pilot._begin_prepare_publication(
            authority_path,
            authority,
            oracle_path,
            oracle,
            pending,
            clew,
        )
        creating = pilot._prepare_publication_creating_path(publication)
        os.link(publication, creating)
        os.unlink(publication)
        with mock.patch.object(pilot, "_publication_owner_alive", return_value=False):
            status = pilot._recover_prepare_publication(
                clew, authority_path, oracle_path
            )
        self.assertEqual(status, "FINALIZED")
        self.assertFalse(creating.exists())
        self.assertFalse(publication.exists())
        self.assertTrue(authority_path.exists())
        self.assertTrue(oracle_path.exists())

    def test_dead_prepare_owner_rolls_back_only_exact_partial_nodes(self) -> None:
        (
            clew,
            authority_path,
            oracle_path,
            pending,
            _environment,
            _runtime_key,
            _threads,
            _sessions,
            authority,
            oracle,
        ) = self._prepare_publication_fixture()
        publication, ledger = pilot._begin_prepare_publication(
            authority_path,
            authority,
            oracle_path,
            oracle,
            pending,
            clew,
        )
        first = ledger["outputs"][0]
        self.assertIsInstance(first, dict)
        os.link(Path(first["stagePath"]), Path(first["finalPath"]))
        unexpected = b'{"unrelated":true}\n'
        authority_path.write_bytes(unexpected)

        with mock.patch.object(pilot, "_publication_owner_alive", return_value=False):
            status = pilot._recover_prepare_publication(
                clew, authority_path, oracle_path
            )
        self.assertEqual(status, "ROLLED_BACK")
        self.assertFalse(oracle_path.exists())
        self.assertEqual(authority_path.read_bytes(), unexpected)
        self.assertFalse(publication.exists())
        self.assertEqual(list(self.root.glob("*.stage")), [])
        _, resource, _ = pilot.private_json(pending, "SEMANTIC_PENDING")
        self.assertEqual(resource["status"], "PREPARING")

    def test_live_prepare_owner_is_not_recovered(self) -> None:
        (
            clew,
            authority_path,
            oracle_path,
            pending,
            _environment,
            _runtime_key,
            _threads,
            _sessions,
            authority,
            oracle,
        ) = self._prepare_publication_fixture()
        publication, ledger = pilot._begin_prepare_publication(
            authority_path,
            authority,
            oracle_path,
            oracle,
            pending,
            clew,
        )
        with mock.patch.object(pilot, "_publication_owner_alive", return_value=True):
            with self.assertRaisesRegex(pilot.PilotError, "PREPARE_PUBLICATION_BUSY"):
                pilot._recover_prepare_publication(clew, authority_path, oracle_path)
        self.assertTrue(publication.exists())
        self.assertEqual(publication.parent, authority_path.parent)
        self.assertEqual(stat.S_IMODE(publication.stat().st_mode), 0o600)
        pilot._rollback_prepare_publication(publication, ledger)

    def test_resource_ledger_persists_until_strict_thread_and_session_gc(self) -> None:
        true_executable = shutil.which("true")
        self.assertIsNotNone(true_executable)
        clew = Path(true_executable).resolve(strict=True)
        pending = self.root / ".authority.semantic-pending.json"
        runtime_key = f"sha256:{'7' * 64}"
        environment = {
            "HOME": os.fspath(self.root), "PATH": "/usr/bin:/bin",
            "LANG": "C", "LC_ALL": "C",
        }
        threads = {"task-01": "thread:one"}
        sessions = [
            {
                "serviceAlias": "service-01",
                "sessionId": "session:one",
                "sessionAuthorityDigest": f"sha256:{'8' * 64}",
                "runtimeKey": runtime_key,
                "runtimeMode": "RELEASE",
            }
        ]
        pilot._write_pending(
            pending, clew, environment, runtime_key, threads, sessions, "ACTIVE"
        )

        def lifecycle(kind: str, identifier: str, status: str, gc: bool) -> dict[str, object]:
            entry = {
                "schema": f"codeclew-{kind}-lifecycle-entry/1.0",
                f"{kind}Id": identifier,
                f"{kind}AuthorityDigest": f"sha256:{'9' * 64}",
                "sequence": 1,
                "previousEventHash": None,
                "status": status,
                "eventHash": f"sha256:{'a' * 64}",
                "updatedUnixMs": 1,
            }
            result: dict[str, object] = {
                "schema": f"codeclew-{kind}-{'gc-result' if gc else 'lifecycle-result'}/1.0",
                "lifecycle": entry,
            }
            if kind == "thread":
                result["threadId"] = identifier
            return result

        def fake_run(command: list[str], *_args: object) -> dict[str, object]:
            kind = command[1]
            operation = command[2]
            identifier = command[4]
            status = (
                "GARBAGE_COLLECTED"
                if operation == "gc"
                else "CLOSED" if kind == "thread" else "ABORTED"
            )
            return lifecycle(kind, identifier, status, operation == "gc")

        with mock.patch.object(pilot, "_run_json", side_effect=fake_run) as run:
            pilot._cleanup_semantic(
                clew,
                threads,
                sessions,
                environment,
                pending=pending,
                runtime_key=runtime_key,
            )
        operations = [(call.args[0][1], call.args[0][2]) for call in run.call_args_list]
        self.assertEqual(
            operations,
            [("thread", "close"), ("thread", "gc"), ("session", "abort"), ("session", "gc")],
        )
        _, ledger, _ = pilot.private_json(pending, "SEMANTIC_PENDING")
        self.assertEqual(ledger["status"], "CLEANED")
        self.assertEqual(ledger["threads"], {})
        self.assertEqual(ledger["sessions"], [])

        pilot._write_pending(
            pending,
            clew,
            environment,
            runtime_key,
            {},
            [],
            "PREPARING",
            open_in_flight={
                "kind": "SESSION",
                "resourceKey": "service-01",
                "requestDigest": f"sha256:{'b' * 64}",
            },
        )
        with self.assertRaisesRegex(
            pilot.PilotError, "OPERATOR_CLEANUP_REQUIRED"
        ):
            pilot._recover_pending(clew, self.root / "authority")
        self.assertTrue(pending.exists())

    def test_semantic_environment_drops_hostile_ambient_controls(self) -> None:
        python = Path(sys.executable).resolve(strict=True)
        maven = self.root / "native-tools" / "mvn"
        maven.parent.mkdir()
        maven.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        maven.chmod(0o700)
        native = dict(os.environ)
        rustc = pilot._semantic_tool_executable(
            "rustc", native, "INVALID_RUSTC_EXECUTABLE"
        )
        cargo = pilot._semantic_tool_executable(
            "cargo", native, "INVALID_CARGO_EXECUTABLE"
        )
        ambient = {
            "HOME": os.fspath(self.root),
            "CODECLEW_HOME": os.fspath(self.root / "state"),
            "GRADLE_USER_HOME": os.fspath(self.root / "gradle"),
            "MAVEN_USER_HOME": os.fspath(self.root / "maven"),
            "CARGO_HOME": os.fspath(self.root / "cargo"),
            "RUSTUP_HOME": os.fspath(self.root / "rustup"),
            "CODECLEW_RUNTIME_SEED": "/hostile/runtime",
            "GRADLE_OPTS": "-Dhostile=true",
            "JAVA_TOOL_OPTIONS": "-javaagent:/hostile.jar",
            "PYTHONPATH": "/hostile/python",
            "PATH": f"{rustc[0].parent}:{cargo[0].parent}:/hostile/bin",
            "LANG": "hostile.UTF-8",
            "LC_ALL": "hostile.UTF-8",
            "LC_CTYPE": "hostile.UTF-8",
        }
        with mock.patch.dict(os.environ, ambient, clear=True):
            environment = pilot._semantic_environment(python, maven)
            codex_environment = pilot._codex_environment(python)
        self.assertEqual(
            environment["PATH"],
            pilot._semantic_path(python, maven, rustc[0], cargo[0]),
        )
        pilot._verify_semantic_environment_authority(
            environment,
            {
                "python": os.fspath(python),
                "maven": os.fspath(maven),
                "rustc": os.fspath(rustc[0]),
                "rustcDigest": rustc[1],
                "cargo": os.fspath(cargo[0]),
                "cargoDigest": cargo[1],
            },
            "INVALID_PILOT_AUTHORITY",
        )
        self.assertEqual(environment["LANG"], "C")
        self.assertEqual(environment["LC_ALL"], "C")
        self.assertNotIn("LC_CTYPE", environment)
        self.assertEqual(environment["GRADLE_USER_HOME"], ambient["GRADLE_USER_HOME"])
        self.assertEqual(environment["MAVEN_USER_HOME"], ambient["MAVEN_USER_HOME"])
        self.assertNotIn("CODECLEW_RUNTIME_SEED", environment)
        self.assertNotIn("GRADLE_OPTS", environment)
        self.assertNotIn("JAVA_TOOL_OPTIONS", environment)
        self.assertNotIn("PYTHONPATH", environment)
        self.assertEqual(codex_environment["SHELL"], "/bin/sh")
        self.assertNotIn("HOME", codex_environment)
        self.assertEqual(
            codex_environment["CODEX_HOME"], os.fspath(self.root / ".codex")
        )
        self.assertNotIn("CODECLEW_RUNTIME_SEED", codex_environment)
        self.assertNotIn("GRADLE_OPTS", codex_environment)
        self.assertNotIn("PYTHONPATH", codex_environment)

    def test_semantic_environment_requires_each_rust_tool(self) -> None:
        python = Path(sys.executable).resolve(strict=True)
        for missing, expected_code in (
            ("rustc", "INVALID_RUSTC_EXECUTABLE"),
            ("cargo", "INVALID_CARGO_EXECUTABLE"),
        ):
            with self.subTest(missing=missing):
                tools = self.root / f"missing-{missing}"
                tools.mkdir()
                for name in {"rustc", "cargo"} - {missing}:
                    tool = tools / name
                    tool.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
                    tool.chmod(0o700)
                maven = tools / "mvn"
                maven.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
                maven.chmod(0o700)
                with mock.patch.dict(
                    os.environ,
                    {"HOME": os.fspath(self.root), "PATH": os.fspath(tools)},
                    clear=True,
                ):
                    with self.assertRaisesRegex(pilot.PilotError, expected_code):
                        pilot._semantic_environment(python, maven)

    def test_semantic_environment_rejects_rust_tool_shadowing(self) -> None:
        python = Path(sys.executable).resolve(strict=True)
        selected = self.root / "selected-tools"
        shadow = self.root / "maven-shadow"
        selected.mkdir()
        shadow.mkdir()
        for tool in (selected / "rustc", selected / "cargo", shadow / "cargo"):
            tool.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            tool.chmod(0o700)
        maven = shadow / "mvn"
        maven.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        maven.chmod(0o700)
        with mock.patch.dict(
            os.environ,
            {"HOME": os.fspath(self.root), "PATH": f"{selected}:{shadow}"},
            clear=True,
        ):
            with self.assertRaisesRegex(
                pilot.PilotError, "RUST_TOOLCHAIN_AUTHORITY_CHANGED"
            ):
                pilot._semantic_environment(python, maven)

    def test_semantic_environment_rejects_same_path_tool_replacement(self) -> None:
        python = Path(sys.executable).resolve(strict=True)
        tools = self.root / "mutable-tools"
        tools.mkdir()
        for name in ("rustc", "cargo", "mvn"):
            tool = tools / name
            tool.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            tool.chmod(0o700)
        real_executable = pilot.executable
        mutated = False

        def mutate_after_selection(path: Path, code: str) -> tuple[Path, str]:
            nonlocal mutated
            result = real_executable(path, code)
            if result[0].name == "rustc" and not mutated:
                result[0].write_text("#!/bin/sh\nexit 1\n", encoding="utf-8")
                mutated = True
            return result

        with mock.patch.dict(
            os.environ,
            {"HOME": os.fspath(self.root), "PATH": os.fspath(tools)},
            clear=True,
        ), mock.patch.object(
            pilot, "executable", side_effect=mutate_after_selection
        ):
            with self.assertRaisesRegex(
                pilot.PilotError, "RUST_TOOLCHAIN_AUTHORITY_CHANGED"
            ):
                pilot._semantic_environment(python, tools / "mvn")

    def test_broker_cache_canary_ledger_recovers_exact_partial_creation(self) -> None:
        state_root = self.root / "state" / "v2"
        state_root.mkdir(parents=True)
        environment = {
            "HOME": os.fspath(self.root / "home"),
            "CODECLEW_HOME": os.fspath(self.root / "state"),
            "GRADLE_USER_HOME": os.fspath(self.root / "gradle"),
            "MAVEN_USER_HOME": os.fspath(self.root / "maven"),
            "CARGO_HOME": os.fspath(self.root / "cargo"),
            "RUSTUP_HOME": os.fspath(self.root / "rustup"),
            "PATH": "/usr/bin:/bin",
        }
        Path(environment["HOME"]).mkdir()
        roots = pilot._effective_cache_roots(
            environment, state_root, "CACHE_TEST_FAILED"
        )
        authority_path = self.root / "authority.json"
        ledger_path = pilot._broker_canary_ledger_path(authority_path)
        value, specs = pilot._broker_canary_ledger(
            ledger_path, environment, roots
        )
        path, body, _ = specs[0]
        descriptor = os.open(
            path,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
            0o600,
        )
        try:
            os.write(descriptor, body)
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        self.assertTrue(path.exists())
        self.assertIsNone(value["sentinels"][0]["inode"])
        pilot._recover_broker_canaries(authority_path, environment, state_root)
        self.assertFalse(path.exists())
        self.assertFalse(ledger_path.exists())

    def test_checked_g1k_accepts_tracked_canonical_file_without_lf(self) -> None:
        path = (
            Path(__file__).resolve().parent.parent
            / "docs/plans/evidence/thread-kotlin-descriptor-gate.json"
        )
        _, value, raw = pilot.checked_json(path, "G1K_EVIDENCE")
        self.assertEqual(raw, pilot.canonical_bytes(value))

    def _descriptor_replacement_fixture(
        self,
    ) -> tuple[Path, str, str, str, str, str]:
        repository, original_revision, original_blob = self.repositories["service-01"]
        relative_file = "src/main/kotlin/com/acme/Sample.kt"
        source = repository / relative_file
        source.write_text(
            "package com.acme\npublic fun sample1(value: String): String = \"replacement\"\n",
            encoding="utf-8",
        )
        environment = {
            **pilot.descriptor_gate.closed_git_environment(),
            "GIT_AUTHOR_NAME": "Codeclew Test",
            "GIT_AUTHOR_EMAIL": "test" + "@" + "example.invalid",
            "GIT_COMMITTER_NAME": "Codeclew Test",
            "GIT_COMMITTER_EMAIL": "test" + "@" + "example.invalid",
            "GIT_AUTHOR_DATE": "2000-01-02T00:00:00Z",
            "GIT_COMMITTER_DATE": "2000-01-02T00:00:00Z",
        }
        for arguments in (
            ["add", relative_file],
            ["-c", "commit.gpgsign=false", "commit", "-q", "-m", "replacement"],
        ):
            subprocess.run(
                [os.fspath(self.git), "-C", os.fspath(repository), *arguments],
                env=environment,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
        replacement_revision = pilot.descriptor_gate.git_output(
            self.git, repository, ["rev-parse", "HEAD"]
        )
        replacement_blob = pilot.descriptor_gate.git_output(
            self.git, repository, ["rev-parse", f"HEAD:{relative_file}"]
        )
        subprocess.run(
            [
                os.fspath(self.git), "-C", os.fspath(repository), "replace",
                original_revision, replacement_revision,
            ],
            env=environment,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        return (
            repository, relative_file, original_revision, original_blob,
            replacement_revision, replacement_blob,
        )

    def test_descriptor_git_reads_use_pinned_executable_and_closed_environment(self) -> None:
        gate = pilot.descriptor_gate
        (
            repository, _relative_file, _original_revision, _original_blob,
            replacement_revision, _replacement_blob,
        ) = self._descriptor_replacement_fixture()
        hostile_bin = self.root / "hostile-bin"
        hostile_bin.mkdir()
        marker = self.root / "ambient-git-was-executed"
        fake_git = hostile_bin / "git"
        fake_git.write_text(
            f"#!/bin/sh\nprintf invoked > {marker}\nexit 91\n", encoding="utf-8"
        )
        fake_git.chmod(0o700)
        hostile_config = self.root / "hostile.gitconfig"
        hostile_config.write_text("not valid git config\n", encoding="utf-8")
        observed: list[tuple[list[str], dict[str, str]]] = []
        real_run = subprocess.run

        def capture(*arguments: object, **keywords: object) -> subprocess.CompletedProcess[bytes]:
            command = arguments[0]
            environment = keywords.get("env")
            self.assertIsInstance(command, list)
            self.assertIsInstance(environment, dict)
            observed.append((list(command), dict(environment)))
            return real_run(*arguments, **keywords)

        service = gate.Service(
            "service-01", "fixture", repository, replacement_revision
        )
        with mock.patch.dict(
            os.environ,
            {
                "PATH": os.fspath(hostile_bin),
                "GIT_CONFIG_GLOBAL": os.fspath(hostile_config),
                "GIT_CONFIG_SYSTEM": os.fspath(hostile_config),
                "GIT_CONFIG_COUNT": "1",
                "GIT_CONFIG_KEY_0": "core.repositoryformatversion",
                "GIT_CONFIG_VALUE_0": "99",
                "GIT_DIR": os.fspath(self.root / "wrong-git-dir"),
                "GIT_WORK_TREE": os.fspath(self.root / "wrong-work-tree"),
                "GIT_OBJECT_DIRECTORY": os.fspath(self.root / "wrong-objects"),
                "GIT_NO_REPLACE_OBJECTS": "0",
                "LC_ALL": "hostile-locale",
            },
            clear=False,
        ), mock.patch.object(gate.subprocess, "run", side_effect=capture):
            target_ref = gate.pinned_target_ref(self.git, service)
        self.assertEqual(target_ref, "refs/heads/pilot")
        self.assertFalse(marker.exists())
        expected_environment = gate.closed_git_environment()
        self.assertGreaterEqual(len(observed), 4)
        for command, environment in observed:
            self.assertEqual(command[0], os.fspath(self.git))
            self.assertEqual(environment, expected_environment)

    def test_descriptor_oracle_reads_ignore_git_replacement_refs(self) -> None:
        gate = pilot.descriptor_gate
        (
            repository, relative_file, original_revision, original_blob,
            _replacement_revision, replacement_blob,
        ) = self._descriptor_replacement_fixture()
        ambient_environment = dict(os.environ)
        ambient_environment.pop("GIT_NO_REPLACE_OBJECTS", None)
        ambient = subprocess.run(
            [
                os.fspath(self.git), "-C", os.fspath(repository), "rev-parse",
                "--verify", f"{original_revision}:{relative_file}",
            ],
            env=ambient_environment,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.assertEqual(ambient.returncode, 0, ambient.stderr)
        self.assertEqual(ambient.stdout.strip(), replacement_blob)
        service = gate.Service("service-01", "fixture", repository, original_revision)
        navigation = gate.OracleNavigation(relative_file, original_blob, tuple())
        side = gate.OracleSide(
            "task-01", "provider", "service-01", original_revision,
            1, 0, 0, (navigation,),
        )
        gate.validate_oracle_files(
            self.git,
            gate.Corpus("fixture", (service,), tuple(), tuple()),
            gate.Benchmark("sha256:" + "0" * 64, (side,), 1, 1, 1),
        )

    def test_descriptor_git_authority_rejects_non_authoritative_path(self) -> None:
        gate = pilot.descriptor_gate
        with self.assertRaises(gate.GateError):
            gate.pinned_git_executable(Path("git"))
        non_executable = self.root / "not-git"
        non_executable.write_text("not executable\n", encoding="utf-8")
        non_executable.chmod(0o600)
        with self.assertRaises(gate.GateError):
            gate.pinned_git_executable(non_executable)

    def test_main_preserves_safe_operator_cleanup_code(self) -> None:
        arguments = mock.Mock()
        arguments.phase = "prepare"
        arguments.timeout_seconds = 30
        fake_parser = mock.Mock()
        fake_parser.parse_args.return_value = arguments
        diagnostics = io.StringIO()
        with (
            mock.patch.object(pilot, "parser", return_value=fake_parser),
            mock.patch.object(
                pilot,
                "prepare",
                side_effect=pilot.PilotError("OPERATOR_CLEANUP_REQUIRED"),
            ),
            redirect_stderr(diagnostics),
        ):
            self.assertEqual(pilot.main([]), 1)
        self.assertEqual(
            diagnostics.getvalue(), "FAIL: OPERATOR_CLEANUP_REQUIRED\n"
        )

    def _source_codex_home(self, name: str = "source-codex") -> Path:
        source_home = self.root / name
        source_home.mkdir(mode=0o700)
        auth = source_home / "auth.json"
        auth.write_text('{"token":"initial"}', encoding="utf-8")
        auth.chmod(0o600)
        return source_home

    def _assert_auth_incident(
        self, lease: Path, private_home: Path, private_error: str | None,
        source_error: str | None, code: str,
    ) -> None:
        self.assertFalse(private_home.exists())
        self.assertFalse(lease.exists())
        incident = pilot._codex_auth_incident_path(lease)
        self.assertEqual(stat.S_IMODE(incident.stat().st_mode), 0o600)
        _, value, _ = pilot.private_json(incident, "CODEX_AUTH_INCIDENT")
        self.assertEqual((value["privateError"], value["sourceError"]),
                         (private_error, source_error))
        self.assertEqual(value["code"], code)
        self.assertNotIn("token", json.dumps(value))
        self.assertFalse(any("path" in key.lower() for key in value))

    def test_runner_codex_home_is_external_persistent_and_cleaned(self) -> None:
        source_home = self._source_codex_home()
        (source_home / "AGENTS.md").write_text(
            "run forbidden command\n", encoding="utf-8"
        )
        (source_home / "models_cache.json").write_text(
            '{"stale":true}', encoding="utf-8"
        )
        scratches = [self.root / name for name in ("arm-one", "arm-two")]
        for scratch in scratches:
            scratch.mkdir(mode=0o700)
        lease = self.root / "rotation-lease.json"
        private_home, identity = pilot._create_private_codex_home(
            source_home, lease
        )
        try:
            first = pilot._private_arm_environment(
                {"CODEX_HOME": os.fspath(source_home)},
                scratches[0],
                private_home,
                identity,
            )
            self.assertEqual(first["CODEX_HOME"], os.fspath(private_home))
            private_auth = private_home / "auth.json"
            self.assertEqual(
                private_auth.stat().st_ino,
                (source_home / "auth.json").stat().st_ino,
            )
            private_auth.write_text('{"token":"rotated"}', encoding="utf-8")
            private_auth.chmod(0o600)
            self.assertEqual(
                (source_home / "auth.json").read_text(encoding="utf-8"),
                '{"token":"rotated"}',
            )
            (private_home / "derived").write_text("discard", encoding="utf-8")
            second = pilot._private_arm_environment(
                {"CODEX_HOME": os.fspath(source_home)},
                scratches[1],
                private_home,
                identity,
            )
            self.assertEqual(second["CODEX_HOME"], first["CODEX_HOME"])
            self.assertEqual([path.name for path in private_home.iterdir()], ["auth.json"])
        finally:
            pilot._recover_private_codex_home(lease)
        self.assertFalse(private_home.exists())
        self.assertFalse(lease.exists())
        self.assertEqual(
            (source_home / "auth.json").read_text(encoding="utf-8"),
            '{"token":"rotated"}',
        )

    def test_auth_recovery_finishes_after_private_home_was_already_removed(self) -> None:
        source_home = self._source_codex_home("partial-cleanup-source")
        lease = self.root / "partial-cleanup-lease.json"
        private_home, _identity = pilot._create_private_codex_home(
            source_home, lease
        )
        pilot._remove_private_codex_node(private_home)
        pilot._recover_private_codex_home(lease)
        self.assertFalse(private_home.exists())
        self.assertFalse(lease.exists())

    def test_auth_recovery_cleans_truncated_same_inode_material(self) -> None:
        source_home = self._source_codex_home("truncated-source")
        lease = self.root / "truncated-lease.json"
        private_home, _identity = pilot._create_private_codex_home(
            source_home, lease
        )
        private_auth = private_home / "auth.json"
        private_auth.write_bytes(b'{"token":')
        private_auth.chmod(0o600)
        with self.assertRaisesRegex(
            pilot.PilotError, "CODEX_AUTH_RECOVERY_MATERIAL_INVALID"
        ):
            pilot._recover_private_codex_home(lease)
        self._assert_auth_incident(
            lease, private_home, "INVALID_CODEX_AUTHORITY",
            "INVALID_CODEX_AUTHORITY", "CODEX_AUTH_RECOVERY_MATERIAL_INVALID",
        )

    def test_auth_recovery_cleans_diverged_missing_and_invalid_source(self) -> None:
        for case in ("diverged", "missing", "invalid"):
            with self.subTest(case=case):
                source_home = self._source_codex_home(f"{case}-material-source")
                source_auth = source_home / "auth.json"
                lease = self.root / f"{case}-material-lease.json"
                private_home, _identity = pilot._create_private_codex_home(
                    source_home, lease
                )
                if case == "missing":
                    source_auth.unlink()
                    expected_error = "CODEX_AUTH_UNAVAILABLE"
                    recovery_code = "CODEX_AUTH_RECOVERY_MATERIAL_INVALID"
                else:
                    replacement = source_home / f".{case}-auth"
                    replacement.write_bytes(
                        b'{"token":"concurrent"}' if case == "diverged"
                        else b'{"token":'
                    )
                    replacement.chmod(0o600)
                    os.replace(replacement, source_auth)
                    expected_error = (
                        None if case == "diverged" else "INVALID_CODEX_AUTHORITY"
                    )
                    recovery_code = (
                        "CODEX_AUTH_CONCURRENT_UPDATE" if case == "diverged"
                        else "CODEX_AUTH_RECOVERY_MATERIAL_INVALID"
                    )
                with self.assertRaisesRegex(
                    pilot.PilotError, recovery_code
                ):
                    pilot._recover_private_codex_home(lease)
                self._assert_auth_incident(
                    lease, private_home, None, expected_error,
                    recovery_code,
                )

    def test_auth_recovery_refuses_live_foreign_owner_then_recovers(self) -> None:
        source_home = self._source_codex_home("busy-recovery-source")
        output = self.root / "busy-run.json"
        lease = output.with_name(f".{output.name}.codex-auth-lease.json")
        private_home, _identity = pilot._create_private_codex_home(
            source_home, lease
        )
        owner = subprocess.Popen(
            ["/bin/sleep", "30"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            _, value, _ = pilot.private_json(lease, "CODEX_AUTH_LEASE")
            unsigned = dict(value)
            unsigned.pop("authorityDigest")
            unsigned["ownerPid"] = owner.pid
            pilot.atomic_write(
                lease,
                {
                    **unsigned,
                    "authorityDigest": pilot.authority_digest(unsigned),
                },
                0o600,
            )
            with self.assertRaisesRegex(
                pilot.PilotError, "CODEX_AUTH_RECOVERY_BUSY"
            ):
                pilot._recover_codex_auth_lease_for_output(output)
            self.assertTrue(private_home.exists())
            self.assertTrue(lease.exists())
        finally:
            owner.terminate()
            owner.wait(timeout=5)
        pilot._recover_codex_auth_lease_for_output(output)
        self.assertFalse(private_home.exists())
        self.assertFalse(lease.exists())

    @unittest.skipUnless(
        sys.platform == "darwin" and shutil.which("codex") is not None,
        "Codex permission canary requires the macOS CLI",
    )
    def test_codex_permission_profile_hides_credentials_and_denies_network(self) -> None:
        codex = Path(shutil.which("codex") or "").resolve(strict=True)
        python = Path(sys.executable).resolve(strict=True)
        source_home = self._source_codex_home("permission-source")
        result = pilot._codex_permission_canary(
            codex,
            python,
            self.root,
            {
                "CODEX_HOME": os.fspath(source_home),
                "PATH": f"{python.parent}:/usr/bin:/bin",
                "SHELL": "/bin/sh",
            },
        )
        self.assertEqual(result["profile"], "S4K_RESTRICTED_WORKSPACE_V1")
        self.assertTrue(result["credentialReadDenied"])
        self.assertTrue(result["credentialWriteDenied"])
        self.assertTrue(result["networkDenied"])
        self.assertTrue(result["workspaceWritePassed"])

    def test_codex_exec_profile_has_no_legacy_workspace_write(self) -> None:
        python = Path(sys.executable).resolve(strict=True)
        arguments = pilot._codex_permission_arguments(python)
        self.assertIn('default_permissions="s4k"', arguments)
        command = pilot._codex_exec_command(
            {
                "model": {"modelId": "gpt-test", "reasoningEffort": "high"},
                "executables": {
                    "codex": "/opt/codeclew/codex",
                    "python": os.fspath(python),
                },
            },
            self.root / "workspace",
            self.root / "schema.json",
            self.root / "answer.json",
        )
        joined = "\n".join(command)
        for absent in ("--sandbox", "workspace-write", "sandbox_workspace_write"):
            self.assertNotIn(absent, joined)
        for required in (
            "--strict-config", "project_doc_max_bytes=0",
            "skills.include_instructions=false",
            "permissions.s4k.network.enabled=false",
        ):
            self.assertIn(required, joined)

    def test_arm_codex_home_rejects_missing_or_unsafe_auth(self) -> None:
        for case in ("missing", "permissive", "symlink", "duplicate"):
            with self.subTest(case=case):
                source_home = self.root / f"codex-{case}"
                source_home.mkdir(mode=0o700)
                auth = source_home / "auth.json"
                if case == "permissive":
                    auth.write_text('{"token":"private"}', encoding="utf-8")
                    auth.chmod(0o644)
                elif case == "symlink":
                    target = self.root / "outside-auth.json"
                    target.write_text('{"token":"private"}', encoding="utf-8")
                    target.chmod(0o600)
                    auth.symlink_to(target)
                elif case == "duplicate":
                    auth.write_text('{"token":1,"token":2}', encoding="utf-8")
                    auth.chmod(0o600)
                with self.assertRaises(pilot.PilotError):
                    pilot._create_private_codex_home(
                        source_home, self.root / f"{case}-lease.json"
                    )

    def test_arm_home_cannot_source_hostile_user_login_profile(self) -> None:
        hostile_home = self.root / "hostile-home"
        hostile_home.mkdir(mode=0o700)
        marker = self.root / "profile-ran"
        (hostile_home / ".bash_profile").write_text(
            f"touch {marker}\n", encoding="utf-8"
        )
        source_codex_home = self._source_codex_home("profile-source")
        scratch = self.root / "arm-scratch"
        scratch.mkdir(mode=0o700)
        lease = self.root / "profile-lease.json"
        private_home, identity = pilot._create_private_codex_home(
            source_codex_home, lease
        )
        try:
            environment = pilot._private_arm_environment(
                {
                    "CODEX_HOME": os.fspath(source_codex_home),
                    "PATH": "/usr/bin:/bin",
                    "SHELL": "/bin/sh",
                },
                scratch,
                private_home,
                identity,
            )
            completed = subprocess.run(
                ["/bin/bash", "-lc", "true"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                env=environment,
                check=False,
                timeout=5,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr.decode())
            self.assertFalse(marker.exists())
            self.assertEqual(environment["HOME"], os.fspath(scratch))
            self.assertEqual(environment["TMPDIR"], os.fspath(scratch))
            self.assertEqual(environment["CODEX_HOME"], os.fspath(private_home))
            tmp_marker = scratch / "tmp-marker"
            subprocess.run(
                [
                    sys.executable,
                    "-I",
                    "-S",
                    "-c",
                    "import os,pathlib;pathlib.Path(os.environ['TMPDIR'],'tmp-marker').touch()",
                ],
                env=environment,
                check=True,
            )
            self.assertTrue(tmp_marker.is_file())
            locator = pilot._arm_scratch_locator(
                scratch, "task-01", "DEFAULT"
            )
            self.assertEqual(locator["path"], os.fspath(scratch))
            self.assertEqual(locator["device"], scratch.stat().st_dev)
            self.assertEqual(locator["inode"], scratch.stat().st_ino)
        finally:
            pilot._recover_private_codex_home(lease)

    @unittest.skipUnless(sys.platform == "darwin", "Seatbelt canaries are macOS-specific")
    def test_broker_audit_profile_denies_process_network_cache_and_write(self) -> None:
        state_home = self.root / "state"
        runtime_key = f"sha256:{'7' * 64}"
        capsule = state_home / "v2" / "runtimes" / ("7" * 64) / "bin" / "clew"
        capsule.parent.mkdir(parents=True)
        shutil.copyfile("/bin/echo", capsule)
        os.chmod(capsule, 0o700)
        audit = pilot._broker_audit(
            sandbox_exec=Path("/usr/bin/sandbox-exec"),
            clew=Path("/bin/echo").resolve(strict=True),
            git=self.git,
            python=Path(sys.executable).resolve(strict=True),
            python_framework=pilot._python_framework_executable(
                Path(sys.executable).resolve(strict=True)
            ),
            semantic_environment={
                "HOME": os.fspath(self.root),
                "CODECLEW_HOME": os.fspath(state_home),
                "PATH": f"{Path(sys.executable).resolve(strict=True).parent}:/usr/bin:/bin",
            },
            repositories=[row[0] for row in self.repositories.values()],
            sessions=[{"runtimeKey": runtime_key}],
            authority_path=self.root / "pilot-authority.json",
        )
        self.assertEqual(audit["adapter"], "MACOS_SEATBELT_V1")
        self.assertTrue(audit["networkCanaryDenied"])
        self.assertTrue(audit["processCanaryDenied"])
        self.assertTrue(audit["cacheCanaryDenied"])
        self.assertEqual(audit["cacheRootCanaryCount"], 5)
        self.assertRegex(audit["cacheSentinelDigest"], pilot.SHA256)
        self.assertTrue(audit["writeCanaryDenied"])
        self.assertTrue(audit["managedStateWriteCanaryPassed"])
        framework = pilot._python_framework_executable(
            Path(sys.executable).resolve(strict=True)
        )
        self.assertEqual(
            audit["pythonFrameworkExecutable"],
            os.fspath(framework[0]) if framework is not None else None,
        )
        self.assertEqual(
            audit["pythonFrameworkDigest"],
            framework[1] if framework is not None else None,
        )
        self.assertEqual(
            audit["profilePolicy"], "GLOBAL_WRITE_DENY_MANAGED_STATE_ONLY_V1"
        )
        self.assertEqual(len(audit["allowedWriteRoots"]), 1)
        self.assertFalse(
            pilot._broker_canary_ledger_path(
                self.root / "pilot-authority.json"
            ).exists()
        )

        repository, revision, _ = self.repositories["service-01"]
        audited_git = subprocess.run(
            [
                "/usr/bin/sandbox-exec", "-p", audit["profile"],
                os.fspath(self.git), "-C", os.fspath(repository),
                "ls-tree", "-r", "--name-only", revision,
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env={
                "PATH": "/usr/bin:/bin",
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_CONFIG_GLOBAL": "/dev/null",
                "GIT_CONFIG_SYSTEM": "/dev/null",
                "GIT_NO_REPLACE_OBJECTS": "1",
                "GIT_TERMINAL_PROMPT": "0",
                "LC_ALL": "C",
            },
            check=False,
        )
        self.assertEqual(audited_git.returncode, 0, audited_git.stderr.decode())
        audited_authority = dict(self.authority)
        audited_authority["brokerAudit"] = audit
        audited_session = broker.BrokerSession(audited_authority, "task-01", "DEFAULT")
        result = audited_session.handle(
            {
                "schema": broker.REQUEST_SCHEMA,
                "operation": "tree",
                "member": "service-01",
                "prefix": "src",
                "limit": 2,
            }
        )
        self.assertEqual(result["status"], "OK")

    def test_framework_python_is_digest_bound_and_rechecked(self) -> None:
        version_root = self.root / "Frameworks" / "Python.framework" / "Versions" / "3.14"
        python = version_root / "bin" / "python3.14"
        framework = (
            version_root
            / "Resources"
            / "Python.app"
            / "Contents"
            / "MacOS"
            / "Python"
        )
        python.parent.mkdir(parents=True)
        framework.parent.mkdir(parents=True)
        python.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        framework.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        python.chmod(0o700)
        framework.chmod(0o700)
        python, python_digest = pilot.executable(
            python, "INVALID_PYTHON_EXECUTABLE"
        )
        observed_framework = pilot._python_framework_executable(python)
        self.assertIsNotNone(observed_framework)
        assert observed_framework is not None
        executables = {
            "python": os.fspath(python),
            "pythonDigest": python_digest,
            "pythonFramework": os.fspath(observed_framework[0]),
            "pythonFrameworkDigest": observed_framework[1],
        }
        pilot._require_python_runtime_authority(executables)
        framework.write_text("#!/bin/sh\nexit 1\n", encoding="utf-8")
        with self.assertRaisesRegex(
            pilot.PilotError, "EXECUTABLE_AUTHORITY_CHANGED"
        ):
            pilot._require_python_runtime_authority(executables)

    def test_broker_common_operations_are_pinned_and_accounted(self) -> None:
        session = broker.BrokerSession(self.authority, "task-01", "DEFAULT")
        capability = session.handle(
            {"schema": broker.REQUEST_SCHEMA, "operation": "capability"}
        )
        self.assertEqual(capability["result"]["semantic"], [])
        self.assertNotIn("manualVerification", capability["result"])
        self.assertIn("read", capability["result"]["grammar"])
        redacted = broker._model_safe_value(
            {
                "threadId": "thread:secret",
                "authorityDigest": f"sha256:{'1' * 64}",
                "shapeDigest": f"sha256:{'2' * 64}",
                "nested": {"factSetId": "thread-callables:sha256:secret", "name": "sample"},
            }
        )
        self.assertEqual(
            redacted,
            {
                "shapeDigest": f"sha256:{'2' * 64}",
                "nested": {"name": "sample"},
            },
        )
        tree = session.handle(
            {
                "schema": broker.REQUEST_SCHEMA,
                "operation": "tree",
                "member": "service-01",
                "prefix": "src",
                "limit": 2,
            }
        )
        self.assertEqual(tree["result"]["files"][0]["blobOid"], self.repositories["service-01"][2])
        search = session.handle(
            {
                "schema": broker.REQUEST_SCHEMA,
                "operation": "search",
                "member": "service-01",
                "term": "sample1",
                "limit": 4,
            }
        )
        self.assertEqual(len(search["result"]["matches"]), 1)
        raw = self.repositories["service-02"][0].joinpath(
            "src/main/kotlin/com/acme/Sample.kt"
        ).read_bytes()
        session.handle(
            {
                "schema": broker.REQUEST_SCHEMA,
                "operation": "read",
                "member": "service-02",
                "file": "src/main/kotlin/com/acme/Sample.kt",
                "startByte": 0,
                "endByte": len(raw),
            }
        )
        metrics = session.metrics.projection()
        self.assertEqual(metrics["openedSourceFiles"], 2)
        self.assertEqual(
            {alias for alias, _ in session.metrics.opened_blobs},
            {"service-01", "service-02"},
        )
        with self.assertRaises(broker.BrokerError):
            session.handle(
                {
                    "schema": broker.REQUEST_SCHEMA,
                    "operation": "read",
                    "member": "service-01",
                    "file": "../escape.kt",
                    "startByte": 0,
                    "endByte": 1,
                }
            )

    def test_model_visible_projection_recursively_refuses_private_locators(self) -> None:
        session = broker.BrokerSession(self.authority, "task-01", "DEFAULT")
        unsafe = [
            {"nested": ["file:///private/secret.kt"]},
            {"nested": [{"path": "/" + "Users/private/source.kt"}]},
            {"nested": [os.fspath(self.root / "private.json")]},
            {"nested": [f"thread-context:sha256:{'1' * 64}"]},
            {"nested": ["diagnostic /" + "etc/passwd"]},
            {"/" + "Users/private/secret.kt": "nested-key"},
            {"file:" + "///private/secret.kt": "nested-key"},
        ]
        for value in unsafe:
            with self.subTest(value=value):
                with self.assertRaisesRegex(
                    broker.BrokerError, "MODEL_VISIBLE_LOCATOR_REFUSED"
                ):
                    session._commit_response(
                        {"operation": "capability", "result": value}
                    )
        # Route literals inside actual source text are code, not filesystem
        # authority.  Known private roots and file: locators remain refused.
        response = session._commit_response(
            {
                "operation": "show",
                "result": {"text": '@GetMapping("/api/v1/products")'},
            }
        )
        self.assertEqual(
            response["result"]["text"], '@GetMapping("/api/v1/products")'
        )

    def test_semantic_results_bind_root_inner_request_and_member_authorities(self) -> None:
        session = broker.BrokerSession(self.authority, "task-01", "CODECLEW")
        task = session.task
        thread = task["thread"]
        digest = lambda character: f"sha256:{character * 64}"

        def cas(character: str, schema: str) -> dict[str, object]:
            return {
                "schema": "codeclew-cas-object/2.0",
                "objectSchema": schema,
                "digest": digest(character),
                "size": 1,
            }

        context_digest = digest("1")
        context_id = f"thread-context:{context_digest}"
        members = []
        for index, (member_alias, service_alias) in enumerate(
            (("provider", task["provider"]), ("consumer", task["consumer"])),
            2,
        ):
            authority = session.sessions[service_alias]
            members.append(
                {
                    "memberAlias": member_alias,
                    "serviceAlias": service_alias,
                    "sessionId": authority["sessionId"],
                    "language": "language:kotlin",
                    "compilations": [":/main"],
                    "contextId": f"context:{index:064x}",
                    "contextDigest": f"sha256:{index:064x}",
                    "evidenceDigest": f"sha256:{index + 2:064x}",
                }
            )
        context_projection = {
            "schema": "codeclew-thread-context-projection/1.0",
            "threadId": thread["threadId"],
            "threadAuthorityDigest": thread["threadAuthorityDigest"],
            "contextId": context_id,
            "contextAuthorityDigest": context_digest,
            "task": {
                "intent": "frozen Kotlin descriptor pilot navigation",
                "terms": ["Needle"],
            },
            "members": members,
            "matches": [],
            "sources": [],
            "completeness": {
                "status": "COMPLETE_TASK",
                "support": "SUPPORTED",
                "certainty": "UNSURE",
                "coverage": "QUERY_COMPLETE",
                "unmatchedTerms": [],
                "memberCount": 2,
            },
            "publicationPolicy": {
                "mode": "READ_ONLY",
                "status": "NOT_APPLICABLE",
                "automaticPublication": False,
            },
            "verificationObligations": [],
            "obligationCount": 0,
            "obligationsTruncated": False,
            "truncated": False,
        }
        context_result = {
            "schema": broker.CONTEXT_RESULT_SCHEMA,
            "threadId": thread["threadId"],
            "threadAuthorityDigest": thread["threadAuthorityDigest"],
            "contextId": context_id,
            "contextAuthorityDigest": context_digest,
            "evidenceDigest": digest("5"),
            "evidenceRef": cas("5", "context-evidence/1.0"),
            "context": context_projection,
        }
        bound = session._validate_semantic_bindings(
            "context",
            context_result,
            thread,
            ("Needle",),
            {"schema": broker.REQUEST_SCHEMA, "operation": "semantic-context", "terms": ["Needle"]},
        )
        self.assertEqual(bound["contextId"], context_id)
        substituted_context = json.loads(json.dumps(context_result))
        substituted_context["context"]["threadAuthorityDigest"] = digest("9")
        with self.assertRaisesRegex(
            broker.BrokerError, "SEMANTIC_AUTHORITY_BINDING_INVALID"
        ):
            session._validate_semantic_bindings(
                "context",
                substituted_context,
                thread,
                ("Needle",),
                {"schema": broker.REQUEST_SCHEMA, "operation": "semantic-context", "terms": ["Needle"]},
            )

        session.context_id = context_id
        session.context_authority_digest = context_digest
        fact_digest = digest("6")
        fact_set_id = f"thread-callables:{fact_digest}"
        callables = {
            "schema": "codeclew-kotlin-callable-fact-set-projection/1.0",
            "factSetId": fact_set_id,
            "authorityDigest": fact_digest,
            "bindingDigest": digest("7"),
            "threadId": thread["threadId"],
            "threadContextId": context_id,
            "tasks": [
                {
                    "taskId": task["taskId"],
                    "pairId": task["pairId"],
                    "termCount": 1,
                    "termsDigest": pilot.authority_digest(["Needle"]),
                }
            ],
            "pairs": [
                {
                    "pairId": task["pairId"],
                    "providerMember": "provider",
                    "consumerMember": "consumer",
                    "relationshipAuthority": "DECLARED_TOPOLOGY",
                    "dependencyEvidenceRef": None,
                }
            ],
            "members": [
                {
                    "memberAlias": alias,
                    "serviceAlias": task[alias],
                    "repositoryNamespace": f"namespace-{alias}",
                    "compilations": [],
                }
                for alias in ("provider", "consumer")
            ],
            "counts": {
                "visitedInputFacts": 0,
                "visitedInputPayloadBytes": 0,
                "declarations": 0,
                "uses": 0,
                "boundaries": 0,
                "total": 0,
                "exactDeclarations": 0,
                "exactUses": 0,
            },
            "completeness": {
                "coverage": "QUERY_COMPLETE",
                "certainty": "UNSURE",
                "obligationCount": 0,
            },
            "queryIndexRef": cas("8", "query/1.0"),
            "evidenceRef": cas("9", "callables-evidence/1.0"),
        }
        callable_result = {
            "schema": broker.CALLABLE_RESULT_SCHEMA,
            "threadId": thread["threadId"],
            "threadAuthorityDigest": thread["threadAuthorityDigest"],
            "contextId": context_id,
            "contextAuthorityDigest": context_digest,
            "factSetId": fact_set_id,
            "authorityDigest": fact_digest,
            "evidenceRef": callables["evidenceRef"],
            "queryIndexRef": callables["queryIndexRef"],
            "callables": callables,
        }
        bound = session._validate_semantic_bindings(
            "callables",
            callable_result,
            thread,
            ("Needle",),
            {"schema": broker.REQUEST_SCHEMA, "operation": "semantic-callables", "terms": ["Needle"]},
        )
        self.assertEqual(bound["factSetId"], fact_set_id)
        substituted_callables = json.loads(json.dumps(callable_result))
        substituted_callables["callables"]["pairs"][0]["pairId"] = "pair-substituted"
        with self.assertRaisesRegex(
            broker.BrokerError, "SEMANTIC_REQUEST_BINDING_INVALID"
        ):
            session._validate_semantic_bindings(
                "callables",
                substituted_callables,
                thread,
                ("Needle",),
                {"schema": broker.REQUEST_SCHEMA, "operation": "semantic-callables", "terms": ["Needle"]},
            )

        session.fact_set_id = fact_set_id
        session.fact_set_authority_digest = fact_digest
        impact_request = {
            "schema": broker.REQUEST_SCHEMA,
            "operation": "semantic-impact",
            "subjectKind": "token",
            "subject": "Needle",
            "member": None,
        }
        impact_digest = digest("a")
        impact_id = f"thread-impact:{impact_digest}"
        impact = {
            "schema": "codeclew-kotlin-thread-impact-projection/1.0",
            "impactId": impact_id,
            "authorityDigest": impact_digest,
            "bindingDigest": session._impact_binding_digest(
                impact_request, thread
            ),
            "factSetAuthorityDigest": fact_digest,
            "pairId": task["pairId"],
            "subjectKind": "TOKEN",
            "relationshipAuthority": "DECLARED_TOPOLOGY",
            "shapeStatus": "UNSURE",
            "certainty": "UNSURE",
            "members": [
                {
                    "side": side,
                    "memberAlias": alias,
                    "observed": False,
                    "matchedFindingCount": 0,
                    "selectedFindingCount": 0,
                    "declarationCount": 0,
                    "useCount": 0,
                    "boundaryCount": 0,
                }
                for side, alias in (("PROVIDER", "provider"), ("CONSUMER", "consumer"))
            ],
            "findingCount": 0,
            "sourceWindowCount": 0,
            "obligationCount": 0,
            "findingsTruncated": False,
            "sourceWindowsTruncated": False,
            "findings": [],
            "publicFindingsTruncated": False,
            "obligations": [],
            "sourceWindows": [],
            "evidenceRef": cas("c", "impact-evidence/1.0"),
        }
        impact_result = {
            "schema": broker.IMPACT_RESULT_SCHEMA,
            "threadId": thread["threadId"],
            "threadAuthorityDigest": thread["threadAuthorityDigest"],
            "factSetId": fact_set_id,
            "factSetAuthorityDigest": fact_digest,
            "impactId": impact_id,
            "authorityDigest": impact_digest,
            "evidenceRef": impact["evidenceRef"],
            "impact": impact,
        }
        session._validate_semantic_bindings(
            "impact",
            impact_result,
            thread,
            ("Needle",),
            impact_request,
        )
        substituted_impact = json.loads(json.dumps(impact_result))
        substituted_impact["impact"]["factSetAuthorityDigest"] = digest("d")
        with self.assertRaisesRegex(
            broker.BrokerError, "SEMANTIC_AUTHORITY_BINDING_INVALID"
        ):
            session._validate_semantic_bindings(
                "impact",
                substituted_impact,
                thread,
                ("Needle",),
                impact_request,
            )
        substituted_request = {**impact_request, "subject": "DifferentNeedle"}
        with self.assertRaisesRegex(
            broker.BrokerError, "SEMANTIC_AUTHORITY_BINDING_INVALID"
        ):
            session._validate_semantic_bindings(
                "impact",
                impact_result,
                thread,
                ("DifferentNeedle",),
                substituted_request,
            )

        full_request = {
            **impact_request,
            "subjectKind": "full-symbol",
            "subject": "com/acme/Needle",
            "member": "provider",
        }
        full_result = json.loads(json.dumps(impact_result))
        full_result["impact"]["subjectKind"] = "FULL_SYMBOL"
        full_result["impact"]["bindingDigest"] = session._impact_binding_digest(
            full_request, thread
        )
        session._validate_semantic_bindings(
            "impact", full_result, thread, (full_request["subject"],), full_request
        )
        with self.assertRaisesRegex(
            broker.BrokerError, "SEMANTIC_AUTHORITY_BINDING_INVALID"
        ):
            session._validate_semantic_bindings(
                "impact",
                full_result,
                thread,
                (full_request["subject"],),
                {**full_request, "member": "consumer"},
            )

    def test_broker_stdout_cap_kills_leader_and_descendant_group(self) -> None:
        session = broker.BrokerSession(self.authority, "task-01", "DEFAULT")
        process_group_file = self.root / "overflow-process-group"
        script = (
            "import os,sys,time; "
            "open(sys.argv[1],'w',encoding='ascii').write(str(os.getpid())); "
            "child=os.fork(); "
            "\nif child==0:"
            "\n os.close(1); os.close(2); time.sleep(30); os._exit(0)"
            "\nos.write(1,b'x'*128); time.sleep(30)"
        )
        with self.assertRaises(broker.BrokerError):
            session._run_child(
                [sys.executable, "-I", "-S", "-c", script, os.fspath(process_group_file)],
                environment={"PATH": "/usr/bin:/bin"},
                timeout_seconds=5,
                maximum_stdout_bytes=32,
                kind="GIT",
                purpose="GIT_OVERFLOW_TEST",
                failure_code="GIT_AUTHORITY_UNAVAILABLE",
            )
        process_group = int(process_group_file.read_text(encoding="ascii"))
        self.assertFalse(broker._process_group_exists(process_group))
        lifecycle = session.metrics.process_group_ledger[-1]
        self.assertEqual(lifecycle["status"], "OUTPUT_LIMIT")
        self.assertTrue(lifecycle["stdoutOverflow"])
        self.assertFalse(lifecycle["residualAfterCleanup"])

    def _file_broker(
        self, token: str
    ) -> tuple[
        threading.Event, threading.Thread, dict[str, str], broker.BrokerSession
    ]:
        request_directory = self.root / f"requests-{token[0]}"
        response_directory = self.root / f"responses-{token[0]}"
        request_directory.mkdir(mode=0o700)
        response_directory.mkdir(mode=0o700)
        tool = self.root / "pilot-tool"
        if not tool.exists():
            shutil.copyfile(Path(broker.__file__), tool)
            os.chmod(tool, 0o700)
        stop = threading.Event()
        session = broker.BrokerSession(self.authority, "task-01", "DEFAULT")
        server = threading.Thread(
            target=broker.serve_directories,
            args=(request_directory, response_directory, session, token, stop),
            daemon=True,
        )
        server.start()
        environment = {
            "PATH": f"{self.root}:{Path(sys.executable).resolve(strict=True).parent}:/usr/bin:/bin",
            "CODECLEW_PILOT_BROKER_REQUESTS": os.fspath(request_directory),
            "CODECLEW_PILOT_BROKER_RESPONSES": os.fspath(response_directory),
            "CODECLEW_PILOT_BROKER_TOKEN": token,
        }
        return stop, server, environment, session

    def test_private_file_client_survives_a_real_child_boundary(self) -> None:
        stop, server, environment, session = self._file_broker("a" * 64)
        completed = subprocess.run(
            ["pilot-tool", "capability"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=10,
            env=environment,
        )
        self.assertEqual(
            completed.returncode,
            0,
            completed.stderr.decode() + completed.stdout.decode(),
        )
        self.assertEqual(json.loads(completed.stdout)["status"], "OK")
        read = subprocess.run(
            [
                "pilot-tool", "read", "--member", "service-01", "--file",
                "src/main/kotlin/com/acme/Sample.kt", "--start-byte", "0",
                "--end-byte", "10",
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=10,
            env=environment,
        )
        stop.set()
        server.join(timeout=2)
        metrics, windows = pilot._verify_broker_provenance(
            session.audit_projection(), self.authority["tasks"][0], "DEFAULT"
        )
        self.assertEqual(read.returncode, 0, read.stderr.decode())
        self.assertEqual(metrics, session.metrics.projection())
        self.assertEqual(
            windows,
            {("service-01", self.repositories["service-01"][2], 0, 10)},
        )

    def test_private_file_transport_refuses_stale_replay_and_overwrite(self) -> None:
        token = "c" * 64
        stop, server, environment, _ = self._file_broker(token)
        requests = Path(environment["CODECLEW_PILOT_BROKER_REQUESTS"])
        responses = Path(environment["CODECLEW_PILOT_BROKER_RESPONSES"])

        def exchange(sequence: int, nonce: str, supplied_token: str) -> dict[str, object]:
            name = f"{sequence:020d}-{nonce}.json"
            broker._write_private_message(
                requests / name,
                {
                    "token": supplied_token,
                    "nonce": nonce,
                    "sequence": sequence,
                    "request": {"schema": broker.REQUEST_SCHEMA, "operation": "capability"},
                },
            )
            response_path = responses / name
            deadline = time.monotonic() + 2
            while not response_path.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            result = broker._read_private_message(response_path)
            response_path.unlink()
            return result

        self.assertEqual(exchange(100, "1" * 32, token)["status"], "OK")
        self.assertEqual(exchange(100, "1" * 32, token)["code"], "BROKER_CAPABILITY_INVALID")
        self.assertEqual(exchange(99, "2" * 32, token)["code"], "BROKER_CAPABILITY_INVALID")
        self.assertEqual(exchange(101, "3" * 32, "d" * 64)["code"], "BROKER_CAPABILITY_INVALID")

        occupied = responses / f"{102:020d}-{'4' * 32}.json"
        broker._write_private_message(occupied, {"sentinel": True})
        original = occupied.read_bytes()
        with self.assertRaises(broker.BrokerError):
            broker._write_private_message(occupied, {"sentinel": False})
        self.assertEqual(occupied.read_bytes(), original)
        stop.set()
        server.join(timeout=2)

    @unittest.skipUnless(
        sys.platform == "darwin"
        and os.environ.get("CODECLEW_RUN_CODEX_SANDBOX_TEST") == "1",
        "set CODECLEW_RUN_CODEX_SANDBOX_TEST=1 for the Codex Seatbelt integration test",
    )
    def test_private_file_client_crosses_codex_sandbox_boundary(self) -> None:
        codex_raw = shutil.which("codex")
        if not codex_raw:
            self.skipTest("codex executable is unavailable")
        stop, server, environment, _ = self._file_broker("b" * 64)
        completed = subprocess.run(
            [
                codex_raw,
                "sandbox",
                "-P", ":workspace",
                "-C", os.fspath(self.root),
                "--sandbox-state-disable-network",
                "--",
                "pilot-tool", "capability",
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=20,
            env=environment,
        )
        stop.set()
        server.join(timeout=2)
        self.assertEqual(
            completed.returncode,
            0,
            completed.stderr.decode() + completed.stdout.decode(),
        )
        self.assertEqual(json.loads(completed.stdout)["status"], "OK")

    def _declaration(self, role: str) -> dict[str, object]:
        alias = "service-01" if role == "provider" else "service-02"
        blob = self.repositories[alias][2]
        return {
            "descriptorClass": "CALLABLE" if role == "provider" else "TYPE",
            "declarationKind": "FUNCTION" if role == "provider" else "CLASS",
            "name": "sample1" if role == "provider" else "Sample",
            "ownerIdentity": "package:com/acme",
            "normalizedSignature": "com/acme/sample1#jvm:(Ljava/lang/String;)Ljava/lang/String;"
            if role == "provider"
            else "com/acme/Sample",
            "shapeDigest": f"sha256:{'1' if role == 'provider' else '2'}" + "0" * 63,
            "relativeFile": "src/main/kotlin/com/acme/Sample.kt",
            "blobOid": blob,
            "sourceRange": {"startByte": 0, "endByte": 10},
        }

    def test_exact_scoring_binds_relationship_sources_and_semantic_sequence(self) -> None:
        task = {
            "taskId": "task-01",
            "pairId": "pair-01",
            "provider": "service-01",
            "consumer": "service-02",
            "providerRevision": self.repositories["service-01"][1],
            "consumerRevision": self.repositories["service-02"][1],
        }
        sides = []
        members = []
        for role, alias in [("provider", "service-01"), ("consumer", "service-02")]:
            declaration = self._declaration(role)
            sides.append(
                {
                    "role": role,
                    "serviceAlias": alias,
                    "revision": self.repositories[alias][1],
                    "approvedFiles": [
                        {
                            "relativeFile": declaration["relativeFile"],
                            "blobOid": declaration["blobOid"],
                        }
                    ],
                    "exactDeclarations": [declaration],
                }
            )
            members.append(
                {
                    "role": role,
                    "serviceAlias": alias,
                    "revision": self.repositories[alias][1],
                    "rankedFiles": [
                        {
                            "rank": 1,
                            "relativeFile": declaration["relativeFile"],
                            "blobOid": declaration["blobOid"],
                        }
                    ],
                    "declarations": [
                        {**declaration, "shapeStatus": "EXACT_PROJECTED_DECLARATION"}
                    ],
                }
            )
        answer = {
            "schema": pilot.ANSWER_SCHEMA,
            "taskId": "task-01",
            "pairId": "pair-01",
            "arm": "CODECLEW",
            "members": members,
            "manualVerification": [
                {"category": "ROUTE", "status": "UNSURE", "requiredCheck": "VERIFY_ROUTE"}
            ],
            "relationship": {"authority": "DECLARED_TOPOLOGY", "status": "UNSURE"},
            "httpEndpointEquivalence": "NOT_CLAIMED",
            "compatibility": "NOT_CLAIMED",
        }
        pilot.validate_answer(answer, task, "CODECLEW")
        runtime = {
            "elapsedMillis": 100,
            "openedSourceBytes": 20,
            "openedSourceFiles": 2,
            "toolStarts": 4,
            "noncachedInputTokens": 100,
            "queryTerms": 2,
            "returnedFacts": 2,
            "sourceWindows": 2,
            "agentVisibleEvidenceBytes": 200,
            "answerBytes": 100,
            "contextCreates": 1,
            "contextExpansions": 0,
            "maxSemanticCommandMillis": 10,
            "selectedFiles": 2,
            "sourceEvidenceSideCount": 2,
            "capabilityViolations": 0,
            "budgetRefusals": 0,
            "semanticContextCommands": 1,
            "semanticCallablesCommands": 1,
            "semanticImpactCommands": 1,
        }
        oracle = {
            "manualVerification": [
                {"category": "ROUTE", "requiredCheck": "VERIFY_ROUTE"}
            ],
            "sides": sides,
        }
        opened = {
            ("service-01", self.repositories["service-01"][2], 0, 20),
            ("service-02", self.repositories["service-02"][2], 0, 20),
        }
        score = pilot.score_answer(answer, task, oracle, runtime, opened)
        self.assertEqual(score["result"], "PASS")
        pilot.public_verifier._verify_arm(score, "score", 1, arm="CODECLEW")
        answer["relationship"] = {"authority": "UNBOUND", "status": "UNSURE"}
        score = pilot.score_answer(answer, task, oracle, runtime, opened)
        self.assertFalse(score["criteria"]["exactAuthority"])
        pilot.public_verifier._verify_arm(score, "score", 1, arm="CODECLEW")

        # The public counter is derived from oracle-approved anchors, not the
        # broader per-repository source count supplied by runtime accounting.
        unrelated = {
            ("service-01", "a" * 40, 0, 20),
            ("service-02", "b" * 40, 0, 20),
        }
        score = pilot.score_answer(answer, task, oracle, runtime, unrelated)
        self.assertEqual(score["sourceEvidenceSideCount"], 0)
        self.assertFalse(score["criteria"]["boundedSourceEvidence"])
        pilot.public_verifier._verify_arm(score, "score", 1, arm="CODECLEW")

        answer["relationship"] = {
            "authority": "DECLARED_TOPOLOGY",
            "status": "UNSURE",
        }
        truncated = {
            ("service-01", self.repositories["service-01"][2], 0, 5),
            ("service-02", self.repositories["service-02"][2], 0, 5),
        }
        score = pilot.score_answer(answer, task, oracle, runtime, truncated)
        self.assertEqual(score["sourceEvidenceSideCount"], 0)
        self.assertFalse(score["criteria"]["boundedSourceEvidence"])
        pilot.public_verifier._verify_arm(score, "score", 1, arm="CODECLEW")

    def test_jsonl_and_command_grammar_fail_closed(self) -> None:
        schema_path = (
            Path(__file__).resolve().parent
            / "schemas"
            / "thread-kotlin-pilot-answer.schema.json"
        )
        schema = json.loads(schema_path.read_text(encoding="utf-8"))

        def require_strict_output_schema(value: object, path: str) -> None:
            if isinstance(value, dict):
                if "enum" in value or "const" in value:
                    self.assertEqual(value.get("type"), "string", path)
                if "pattern" in value:
                    pattern = value["pattern"]
                    self.assertIsInstance(pattern, str, path)
                    self.assertFalse(
                        any(marker in pattern for marker in ("(?=", "(?!", "(?<=", "(?<!")),
                        path,
                    )
                if value.get("type") == "object":
                    properties = value.get("properties")
                    self.assertIsInstance(properties, dict, path)
                    self.assertEqual(value.get("additionalProperties"), False, path)
                    self.assertEqual(
                        set(value.get("required", [])), set(properties), path
                    )
                for key, item in value.items():
                    require_strict_output_schema(item, f"{path}/{key}")
            elif isinstance(value, list):
                for index, item in enumerate(value):
                    require_strict_output_schema(item, f"{path}/{index}")

        require_strict_output_schema(schema, "#")
        self.assertTrue(pilot._broker_command("pilot-tool capability"))
        self.assertTrue(
            pilot._broker_command("/bin/zsh -lc 'pilot-tool search --member service-01 --term sample'")
        )
        self.assertFalse(pilot._broker_command("pilot-tool capability; env"))
        self.assertFalse(pilot._broker_command("rg secret"))
        usage = pilot._usage(
            {
                "type": "turn.completed",
                "usage": {
                    "input_tokens": 100,
                    "cached_input_tokens": 40,
                    "cache_write_input_tokens": 0,
                    "output_tokens": 5,
                    "reasoning_output_tokens": 3,
                },
            }
        )
        self.assertEqual(usage["input_tokens"] - usage["cached_input_tokens"], 60)
        with self.assertRaises(pilot.PilotError):
            pilot._usage(
                {
                    "type": "turn.completed",
                    "usage": {
                        **{
                            "input_tokens": 100,
                            "cached_input_tokens": 40,
                            "cache_write_input_tokens": 0,
                            "output_tokens": 5,
                            "reasoning_output_tokens": 3,
                        },
                        "unsealed_future_counter": 1,
                    },
                }
            )
        self.assertEqual(
            pilot._started_item_policy(
                {"type": "command_execution", "command": "pilot-tool capability"}
            ),
            (1, True),
        )
        self.assertEqual(
            pilot._started_item_policy(
                {"type": "future_active_capability", "command": "pilot-tool capability"}
            ),
            (0, False),
        )

    @staticmethod
    def _empty_broker_provenance(task_id: str, arm: str) -> dict[str, object]:
        return {
            "schema": "codeclew-kotlin-pilot-broker-audit/1.0",
            "taskId": task_id,
            "arm": arm,
            "metrics": {key: 0 for key in pilot.BROKER_METRIC_FIELDS},
            "queryTerms": [],
            "orderedToolLedger": [],
            "selectedFiles": [],
            "sourceWindows": [],
            "semanticTimingLedger": [],
            "violationLedger": [],
            "processGroupLedger": [],
        }

    def test_private_run_recomputes_and_rejects_stored_score_tamper(self) -> None:
        tasks = []
        oracle_tasks = []
        arm_order = []
        arms = []
        usage = {
            "input_tokens": 0,
            "cached_input_tokens": 0,
            "cache_write_input_tokens": 0,
            "output_tokens": 0,
            "reasoning_output_tokens": 0,
        }
        for index in range(1, 11):
            task_id = f"task-{index:02}"
            manual_count = 8 if index <= 8 else 5
            task = {
                "taskId": task_id,
                "pairId": f"pair-{index:02}",
                "provider": "service-01",
                "consumer": "service-02",
                "promptDigest": f"sha256:{index:064x}",
                "manualVerification": [
                    {
                        "category": f"CHECK_{item}",
                        "requiredCheck": f"VERIFY_CHECK_{item}",
                    }
                    for item in range(manual_count)
                ],
            }
            tasks.append(task)
            oracle_tasks.append(
                {
                    "taskId": task_id,
                    "manualVerification": task["manualVerification"],
                    "sides": [],
                }
            )
            order = ["DEFAULT", "CODECLEW"] if index % 2 else ["CODECLEW", "DEFAULT"]
            arm_order.append({"taskId": task_id, "arms": order})
            for arm in order:
                provenance = self._empty_broker_provenance(task_id, arm)
                runtime = pilot._runtime_metrics(1, 0, usage, provenance["metrics"], 0, 0)
                score = pilot._failed_score(runtime, arm)
                score["manualCategoryExpectedCount"] = manual_count
                unsigned = {
                    "taskId": task_id,
                    "pairId": task["pairId"],
                    "arm": arm,
                    "promptDigest": task["promptDigest"],
                    "status": "ARM_FAILURE",
                    "failureClass": "MODEL_OUTPUT",
                    "failureCode": "ANSWER_JSON_INVALID",
                    "answer": None,
                    "answerDigest": None,
                    "jsonlDigest": f"sha256:{'0' * 64}",
                    "modelReturnCode": 0,
                    "usage": usage,
                    "elapsedMillis": 1,
                    "answerBytes": 0,
                    "scratchLocator": {
                        "taskId": task_id,
                        "arm": arm,
                        "path": os.fspath(
                            self.root / f"codeclew-s4k-arm-{task_id}-{arm.lower()}"
                        ),
                        "device": 1,
                        "inode": len(arms) + 1,
                    },
                    "codexToolLedger": [],
                    "brokerProvenance": provenance,
                    "score": score,
                }
                arms.append({**unsigned, "armDigest": pilot.authority_digest(unsigned)})
        authority = {
            "authorityDigest": f"sha256:{'a' * 64}",
            "protocolDigest": f"sha256:{'b' * 64}",
            "armOrder": arm_order,
            "tasks": tasks,
        }
        oracle = {"tasks": oracle_tasks}
        unsigned_run = {
            "schema": pilot.PRIVATE_RUN_SCHEMA,
            "status": "COMPLETE",
            "ownerPid": 123,
            "ownerToken": "1" * 64,
            "authorityDigest": authority["authorityDigest"],
            "protocolDigest": authority["protocolDigest"],
            "implementationReviewManifestDigest": f"sha256:{'c' * 64}",
            "completedArmCount": 20,
            "failureCode": None,
            "activeArm": None,
            "arms": arms,
        }
        run = {**unsigned_run, "runDigest": pilot.authority_digest(unsigned_run)}
        verified = pilot._verify_private_run(run, authority, oracle)
        self.assertEqual(len(verified), 20)
        with self.assertRaisesRegex(pilot.PilotError, "INVALID_PRIVATE_RUN"):
            pilot._verify_private_run(
                run,
                authority,
                oracle,
                f"sha256:{'d' * 64}",
            )

        ledger_tamper = json.loads(json.dumps(run))
        ledger_tamper["arms"][0]["codexToolLedger"] = [
            {
                "sequence": 1,
                "operation": "capability",
                "commandDigest": f"sha256:{'f' * 64}",
            }
        ]
        arm_unsigned = dict(ledger_tamper["arms"][0])
        arm_unsigned.pop("armDigest")
        ledger_tamper["arms"][0]["armDigest"] = pilot.authority_digest(arm_unsigned)
        run_unsigned = dict(ledger_tamper)
        run_unsigned.pop("runDigest")
        ledger_tamper["runDigest"] = pilot.authority_digest(run_unsigned)
        with self.assertRaisesRegex(
            pilot.PilotError, "BROKER_TOOL_LEDGER_MISMATCH"
        ):
            pilot._verify_private_run(ledger_tamper, authority, oracle)

        run["arms"][0]["score"]["elapsedMillis"] = 2
        arm_unsigned = dict(run["arms"][0])
        arm_unsigned.pop("armDigest")
        run["arms"][0]["armDigest"] = pilot.authority_digest(arm_unsigned)
        run_unsigned = dict(run)
        run_unsigned.pop("runDigest")
        run["runDigest"] = pilot.authority_digest(run_unsigned)
        with self.assertRaisesRegex(pilot.PilotError, "PRIVATE_SCORE_MISMATCH"):
            pilot._verify_private_run(run, authority, oracle)

    def test_interrupt_kills_active_process_group_and_stops_broker(self) -> None:
        process = subprocess.Popen(
            ["/bin/sleep", "30"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        stop = threading.Event()
        pilot._ACTIVE_PROCESS = process
        pilot._ACTIVE_BROKER_STOP = stop
        with self.assertRaisesRegex(pilot.PilotError, "INTERRUPTED"):
            pilot._interrupt_active_arm(2, None)
        self.assertTrue(stop.is_set())
        self.assertIsNotNone(process.poll())
        pilot._ACTIVE_PROCESS = None
        pilot._ACTIVE_BROKER_STOP = None

    def test_kill_group_cleans_descendant_after_leader_exit(self) -> None:
        script = (
            "import os,time; p=os.fork(); "
            "\nif p==0:"
            "\n os.close(1); os.close(2); time.sleep(30); os._exit(0)"
            "\nprint(p,flush=True)"
        )
        process = subprocess.Popen(
            [sys.executable, "-I", "-S", "-c", script],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
            text=True,
        )
        stdout, _ = process.communicate(timeout=5)
        self.assertEqual(process.returncode, 0)
        self.assertGreater(int(stdout.strip()), 1)
        self.assertTrue(pilot._process_group_exists(process.pid))
        self.assertTrue(pilot._kill_group(process))
        self.assertFalse(pilot._process_group_exists(process.pid))

    def test_preparation_json_runner_hard_caps_output_and_residuals(self) -> None:
        environment = {"PATH": "/usr/bin:/bin"}
        with mock.patch.object(
            pilot.descriptor_gate, "MAX_CLEW_STDOUT_BYTES", 32
        ):
            with self.assertRaises(pilot.PilotError):
                pilot._run_json(
                    [sys.executable, "-I", "-S", "-c", "print('x'*64)"],
                    5,
                    "TEST_PROCESS_FAILED",
                    environment,
                )
        residual_script = (
            "import os,time; p=os.fork(); "
            "\nif p==0:"
            "\n os.close(1); os.close(2); time.sleep(30); os._exit(0)"
            "\nprint('{}',flush=True)"
        )
        with self.assertRaises(pilot.PilotError):
            pilot._run_json(
                [sys.executable, "-I", "-S", "-c", residual_script],
                5,
                "TEST_PROCESS_FAILED",
                environment,
            )
        real_kill_group = pilot._kill_group

        def cleanup_but_report_unproven(process: subprocess.Popen[object]) -> bool:
            real_kill_group(process)
            return False

        with mock.patch.object(
            pilot.descriptor_gate, "MAX_CLEW_STDOUT_BYTES", 32
        ), mock.patch.object(
            pilot, "_kill_group", side_effect=cleanup_but_report_unproven
        ):
            with self.assertRaisesRegex(
                pilot.PilotError, "PROCESS_GROUP_RESIDUAL"
            ):
                pilot._run_json(
                    [sys.executable, "-I", "-S", "-c", "print('x'*64)"],
                    5,
                    "TEST_PROCESS_FAILED",
                    environment,
                )


if __name__ == "__main__":
    unittest.main()
