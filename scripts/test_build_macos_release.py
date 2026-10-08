#!/usr/bin/env python3
"""Platform-independent contract tests for release packaging and version binding."""

from __future__ import annotations

import io
import json
from pathlib import Path
import stat
import sys
import tarfile
import tempfile
import unittest
from unittest import mock


sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_macos_release as release  # noqa: E402


class ReleaseVersionTest(unittest.TestCase):
    def capabilities(self, profile: str) -> dict:
        return {
            "schema": "codeclew-capabilities/1.0", "status": "PILOT_READY",
            "runtimeMode": "RELEASE",
            "packagedWorkers": [
                {"runtimeName": name, "compilerVersion": version}
                for name, version in release.RELEASE_PROFILES[profile].items()
            ],
            "analysisModules": [{"id": "csharp-roslyn", "compiler": "roslyn-5.9.0"}],
        }

    def test_release_requires_dotnet_10_before_building_any_runtime(self) -> None:
        with mock.patch.object(release.shutil, "which", return_value=None), mock.patch.object(release, "run") as run:
            with self.assertRaisesRegex(release.ReleaseError, ".NET 10"):
                release.build_runtime_state(Path("/source"), Path("/work"), "v1.2.3")
            run.assert_not_called()
        for version, accepted in [(b"6.0.428\n", False), (b"9.0.100\n", False), (b"10.0.401\n", True), (b"invalid\n", False)]:
            with self.subTest(version=version), mock.patch.object(release.shutil, "which", return_value="/sdk/dotnet"), mock.patch.object(release, "run", return_value=version) as run:
                if accepted:
                    release.require_dotnet_sdk(Path("/source"), {"PATH": "/sdk"})
                else:
                    with self.assertRaisesRegex(release.ReleaseError, ".NET 10"):
                        release.require_dotnet_sdk(Path("/source"), {"PATH": "/sdk"})
                self.assertEqual(run.call_args.args, (["/sdk/dotnet", "--version"], Path("/source/workers/dotnet")))

    def test_profiles_reject_missing_substituted_or_duplicate_csharp_workers(self) -> None:
        for profile in release.RELEASE_PROFILES:
            release.verify_worker_profile(self.capabilities(profile), profile)
            for corruption in ["missing", "wrong-name", "wrong-version", "duplicate", "missing-module"]:
                with self.subTest(profile=profile, corruption=corruption):
                    value = self.capabilities(profile)
                    csharp = next(row for row in value["packagedWorkers"] if row["runtimeName"] == "csharp")
                    if corruption == "missing":
                        value["packagedWorkers"].remove(csharp)
                    elif corruption == "wrong-name":
                        csharp["runtimeName"] = "unexpected"
                    elif corruption == "wrong-version":
                        csharp["compilerVersion"] = "roslyn-5.8.0"
                    elif corruption == "duplicate":
                        value["packagedWorkers"].append(dict(csharp))
                    else:
                        value["analysisModules"] = []
                    with self.assertRaisesRegex(release.ReleaseError, "C# Roslyn is required"):
                        release.verify_worker_profile(value, profile)

    def test_csharp_smoke_uses_public_launcher_and_preserves_partial_authority(self) -> None:
        for defect, expected_error in [
            (None, None), ("call", "resolved repository Save call"),
            ("boundary", "unrestored test-project boundary"),
            ("route", "entrypoint routes"), ("coverage", "partial Roslyn authority"),
            ("write", "changed caller-owned"),
        ]:
            with self.subTest(defect=defect), tempfile.TemporaryDirectory() as value:
                work = Path(value)
                repository = work / "repository"
                calls = []

                def run(arguments, cwd, *, environment=None):
                    calls.append(arguments)
                    if arguments[0] in {"git", "dotnet"}:
                        if arguments[0] == "dotnet":
                            assets = repository / "src/Orders.Api/obj/project.assets.json"
                            assets.parent.mkdir()
                            assets.write_text("{}")
                        return b""
                    self.assertEqual(arguments[0], "/extracted/bin/clew")
                    self.assertEqual(environment, {"CODECLEW_HOME": "/isolated/state"})
                    if arguments[1:] == ["capabilities"]:
                        result = self.capabilities("core")
                    elif arguments[1:3] == ["session", "open"]:
                        result = {"status": "OPEN", "session": {"sessionId": "session:smoke"}}
                    elif arguments[1:3] == ["context", "create"]:
                        term = arguments[arguments.index("--term") + 1]
                        if term == "Save":
                            payloads = [{"kind": "DECLARATION", "name": "Save"}]
                            if defect != "call":
                                payloads.append({"kind": "RELATION", "relationKind": "CALLS", "targetIdentity": "method:class:Orders.Core.IOrderRepository#Save(LOrders/Core/Order;)V"})
                        else:
                            payloads = [] if defect == "boundary" else [{"kind": "BOUNDARY", "code": "CSHARP_PROJECT_UNRESTORED"}]
                        result = {"context": {"matches": [{"payload": row} for row in payloads]}}
                    elif arguments[1] == "entrypoints":
                        routes = [
                            ("DELETE", "/internal/audit/{id}"), ("GET", "/admin/Reports/Daily"),
                            ("GET", "/api/Orders/{id:int}"), ("GET", "/api/v{version:apiVersion}/quotes"),
                            ("GET", "/health"), ("POST", "/api/Orders"), ("PUT", "/api/Orders/{id}"),
                        ]
                        result = {
                            "entries": [{"trigger": {"methods": [method], "paths": [path]}} for method, path in routes],
                            "scopes": [{"boundaries": ["CSHARP_PROJECT_UNRESTORED"], "generationCoverage": "COMPLETE" if defect == "coverage" else "PARTIAL", "generationCertainty": "UNSURE"}],
                            "nextCursor": None,
                        }
                        if defect == "route":
                            result["entries"].pop()
                        if defect == "write":
                            (repository / "src/Orders.Core/Orders.cs").write_text("changed")
                    else:
                        result = {}
                    return json.dumps(result).encode()

                with mock.patch.object(release, "run", side_effect=run):
                    arguments = (
                        Path(__file__).resolve().parent.parent, Path("/extracted/bin/clew"),
                        {"CODECLEW_HOME": "/isolated/state"}, work, "core",
                    )
                    if expected_error:
                        with self.assertRaisesRegex(release.ReleaseError, expected_error):
                            release.verify_csharp_archive(*arguments)
                    else:
                        release.verify_csharp_archive(*arguments)
                self.assertIn(["dotnet", "restore", "src/Orders.Api/Orders.Api.csproj"], calls)
                self.assertEqual(calls[-2:], [
                    ["/extracted/bin/clew", "session", "close", "--session", "session:smoke"],
                    ["/extracted/bin/clew", "session", "gc", "--session", "session:smoke"],
                ])
                self.assertFalse((repository / "tests/Orders.Tests/obj/project.assets.json").exists())

    def test_release_platform_accepts_linux_x64_and_rejects_native_windows(self) -> None:
        for system, architecture, expected in [
            ("Darwin", "arm64", "macos"),
            ("Darwin", "x86_64", "macos"),
            ("Linux", "x86_64", "linux"),
        ]:
            self.assertEqual(release.release_platform(system, architecture), expected)
        for system, architecture in [("Linux", "aarch64"), ("Windows", "AMD64"), ("Linux", "i686")]:
            with self.subTest(system=system, architecture=architecture):
                with self.assertRaises(release.ReleaseError):
                    release.release_platform(system, architecture)

    def test_installer_smoke_selects_each_profile_and_isolates_state(self) -> None:
        for profile in ["core"]:
            with self.subTest(profile=profile), mock.patch.object(release, "run") as run, mock.patch.object(release, "verify_cli_version") as version:
                release.verify_installer(Path("/source"), Path("/assets"), Path("/smoke"), "v1.2.3", profile)
                arguments = run.call_args.args[0]
                environment = run.call_args.kwargs["environment"]
                self.assertEqual(arguments, ["/bin/sh", "/source/site/install.sh"])
                self.assertEqual(environment["CODECLEW_ASSET_DIR"], "/assets")
                self.assertEqual(environment["CODECLEW_PACKS"], "" if profile == "core" else profile)
                self.assertEqual(environment["CODECLEW_VERSION"], "v1.2.3")
                self.assertEqual(environment["CODECLEW_HOME"], "/smoke/state")
                self.assertEqual(environment["CODECLEW_BIN_DIR"], "/smoke/bin")
                self.assertEqual(environment["CODECLEW_INSTALL_ROOT"], "/smoke/install")
                version.assert_called_once_with(Path("/smoke/bin/clew"), "v1.2.3", Path("/smoke/state"), Path("/source"))

    def test_linux_archives_have_distinct_names_and_matching_checksums(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            output = Path(value)
            package = output / "package"
            package.mkdir()
            (package / "VERSION").write_text("v1.2.3\n", encoding="ascii")
            for profile, expected in [
                ("core", "codeclew-linux-x86_64.tar.gz"),
            ]:
                asset, checksum = release.write_archive(package, output, "x86_64", profile, "linux")
                self.assertEqual(asset.name, expected)
                self.assertEqual(checksum.read_text(), f"{release.file_sha256(asset)}  {expected}\n")
                extracted = release.extract_release_archive(asset, output / f"extracted-{profile}")
                self.assertEqual((extracted / "VERSION").read_text(), "v1.2.3\n")

    def test_retired_profile_cannot_create_release_assets(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            root = Path(value)
            with self.assertRaisesRegex(release.ReleaseError, "retired"):
                release.write_archive(root, root, "arm64", "kotlin23")
            self.assertEqual(list(root.iterdir()), [])

    def test_navigation_smoke_declares_the_exact_decision_identifier(self) -> None:
        arguments = release.navigation_smoke_query(
            Path("/release/bin/clew"),
            Path("/repository"),
            "v-release-smoke",
        )

        self.assertEqual(
            arguments[arguments.index("--decision-identifier") + 1],
            "Counter",
        )
        self.assertEqual(
            [arguments[index + 1] for index, value in enumerate(arguments) if value == "--term"],
            ["Counter"],
        )
        self.assertEqual(arguments[arguments.index("--language") + 1], "python")
        self.assertEqual(arguments[arguments.index("--profile") + 1], "python-syntax")
        self.assertIn("--source", arguments)

    def test_release_archive_is_reopened_and_extracted_from_produced_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            temporary = Path(value)
            package = temporary / "package"
            (package / "bin").mkdir(parents=True)
            launcher = package / "bin" / "clew"
            launcher.write_text("#!/bin/sh\n", encoding="ascii")
            launcher.chmod(0o500)
            output = temporary / "output"
            output.mkdir()
            asset, _ = release.write_archive(package, output, "arm64", "core")

            extracted = release.extract_release_archive(
                asset,
                temporary / "extracted",
            )
            self.assertEqual((extracted / "bin" / "clew").read_bytes(), launcher.read_bytes())
            self.assertEqual(stat.S_IMODE((extracted / "bin" / "clew").stat().st_mode), 0o500)

            unsafe = output / "unsafe.tar.gz"
            with tarfile.open(unsafe, mode="w:gz") as archive:
                member = tarfile.TarInfo("codeclew/link")
                member.type = tarfile.SYMTYPE
                member.linkname = "../../outside"
                archive.addfile(member, io.BytesIO())
            with self.assertRaisesRegex(release.ReleaseError, "unsafe member"):
                release.extract_release_archive(
                    unsafe,
                    temporary / "unsafe-extracted",
                )

    def test_seed_lifecycle_root_has_bootstrap_safe_permissions(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            temporary = Path(value)
            package = temporary / "package"
            package.mkdir()
            state = temporary / "state"
            runtime_key = "1" * 64
            capsule = state / "v2" / "runtimes" / runtime_key
            capsule.mkdir(parents=True)
            manifest = {
                "artifacts": {},
                "manifestDigest": "sha256:" + "2" * 64,
                "mode": "RELEASE",
                "runtimeKey": "sha256:" + runtime_key,
                "workers": {},
            }
            (capsule / "runtime.json").write_bytes(
                release.canonical(manifest) + b"\n"
            )

            seed_path = release.write_seed(
                package,
                state,
                b"evidence",
                "a" * 40,
                "b" * 40,
                "sha256:" + "3" * 64,
            )

            self.assertEqual(
                stat.S_IMODE((package / "seed").stat().st_mode),
                0o700,
            )
            lease = (
                seed_path.parent
                / "parallel-state"
                / "v2"
                / "locks"
                / f"runtime-{runtime_key}.lease"
            )
            self.assertTrue(lease.is_file())
            self.assertFalse(lease.is_symlink())
            self.assertEqual(stat.S_IMODE(lease.stat().st_mode), 0o600)
            self.assertEqual(lease.stat().st_size, 0)

    def test_cli_version_must_match_the_release_tag(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            temporary = Path(value)
            launcher = temporary / "clew"
            launcher.write_text(
                "#!/bin/sh\nprintf '%s\\n' 'clew 0.2.0'\n", encoding="ascii"
            )
            launcher.chmod(0o500)

            release.verify_cli_version(
                launcher, "v0.2.0", temporary / "state", temporary
            )
            with self.assertRaisesRegex(release.ReleaseError, "does not match"):
                release.verify_cli_version(
                    launcher, "v0.1.7", temporary / "other-state", temporary
                )

    def test_minimal_source_excludes_repository_and_is_manifest_bound(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            package = Path(value) / "codeclew"
            package.mkdir()
            digest = release.assemble_source(
                release.Path(__file__).resolve().parent.parent,
                package,
                "a" * 40,
                "b" * 40,
            )
            source = package / "source"
            observed = {
                path.relative_to(source).as_posix()
                for path in source.rglob("*")
                if path.is_file()
            }
            self.assertEqual(
                observed,
                {*release.MINIMAL_SOURCE_FILES, "release-source.json"},
            )
            manifest = json.loads((source / "release-source.json").read_bytes())
            self.assertEqual(manifest["manifestDigest"], digest)
            self.assertNotIn("Cargo.toml", observed)
            self.assertFalse((source / ".git").exists())

    def test_core_profile_preserves_runtime_identity_and_drops_build_component_cache(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            temporary = Path(value)
            state = temporary / "source-state"
            runtime_key = "1" * 64
            capsule = state / "v2" / "runtimes" / runtime_key
            kotlin24 = capsule / "workers" / "kotlin" / "build" / "install" / "kotlin"
            csharp = capsule / "workers" / "dotnet" / "publish"
            kotlin24.mkdir(parents=True)
            csharp.mkdir(parents=True)
            (kotlin24 / "worker.jar").write_bytes(b"kotlin24")
            (csharp / "Codeclew.CSharp.Analyzer.dll").write_bytes(b"roslyn")
            components = state / "v2" / "runtimes" / "components"
            component24 = "3" * 64
            (components / component24 / "files").mkdir(parents=True)
            (components / component24 / "files" / "worker.jar").write_bytes(
                b"kotlin24"
            )
            manifest = {
                "artifacts": {"clew": {"sha256": "sha256:" + "4" * 64}},
                "components": {
                    "csharp": "sha256:" + "8" * 64,
                    "kotlin24": "sha256:" + component24,
                },
                "manifestDigest": "sha256:" + "5" * 64,
                "mode": "RELEASE",
                "runtimeKey": "sha256:" + runtime_key,
                "workerIds": ["csharp", "kotlin24"],
                "workers": {
                    "csharp": {
                        "distribution": "workers/dotnet/publish",
                        "treeHash": "sha256:" + "9" * 64,
                    },
                    "kotlin24": {
                        "distribution": "workers/kotlin/build/install/kotlin",
                        "treeHash": "sha256:" + "7" * 64,
                    },
                },
            }
            (capsule / "runtime.json").write_bytes(release.canonical(manifest) + b"\n")
            (capsule / "READY").write_text("sha256:" + runtime_key + "\n")

            destination = temporary / "core-state"
            core_capsule = release.prepare_profile_state(state, destination, "core")
            core_manifest = json.loads((core_capsule / "runtime.json").read_bytes())
            self.assertEqual(stat.S_IMODE(core_capsule.stat().st_mode), 0o500)
            self.assertEqual(set(core_manifest["workers"]), {"csharp", "kotlin24"})
            self.assertEqual(set(core_manifest["components"]), {"csharp", "kotlin24"})
            self.assertEqual((core_capsule / "workers/dotnet/publish/Codeclew.CSharp.Analyzer.dll").read_bytes(), b"roslyn")
            self.assertEqual(core_manifest["runtimeKey"], "sha256:" + runtime_key)
            self.assertEqual((core_capsule / "runtime.json").read_bytes(), (capsule / "runtime.json").read_bytes())
            self.assertFalse((core_capsule / "workers" / "kotlin23").exists())
            manifest["workers"]["kotlin23"] = {"distribution": "retired"}
            (capsule / "runtime.json").write_bytes(release.canonical(manifest) + b"\n")
            with self.assertRaisesRegex(release.ReleaseError, "sole core"):
                release.prepare_profile_state(state, temporary / "retired-state", "core")
            self.assertEqual(
                list((destination / "v2" / "runtimes" / "components").iterdir()),
                [],
            )


if __name__ == "__main__":
    unittest.main()
