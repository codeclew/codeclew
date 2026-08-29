#!/usr/bin/env python3
"""Platform-independent contract tests for macOS release version binding."""

from __future__ import annotations

import json
from pathlib import Path
import stat
import sys
import tempfile
import unittest


sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_macos_release as release  # noqa: E402


class ReleaseVersionTest(unittest.TestCase):
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

    def test_core_profile_drops_kotlin23_and_build_component_cache(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            temporary = Path(value)
            state = temporary / "source-state"
            runtime_key = "1" * 64
            capsule = state / "v2" / "runtimes" / runtime_key
            kotlin23 = capsule / "workers" / "kotlin23" / "build" / "install" / "kotlin23"
            kotlin24 = capsule / "workers" / "kotlin" / "build" / "install" / "kotlin"
            kotlin23.mkdir(parents=True)
            kotlin24.mkdir(parents=True)
            (kotlin23 / "worker.jar").write_bytes(b"kotlin23")
            (kotlin24 / "worker.jar").write_bytes(b"kotlin24")
            components = state / "v2" / "runtimes" / "components"
            component23 = "2" * 64
            component24 = "3" * 64
            (components / component23 / "files").mkdir(parents=True)
            (components / component24 / "files").mkdir(parents=True)
            (components / component23 / "files" / "worker.jar").write_bytes(
                b"kotlin23"
            )
            (components / component24 / "files" / "worker.jar").write_bytes(
                b"kotlin24"
            )
            manifest = {
                "artifacts": {"clew": {"sha256": "sha256:" + "4" * 64}},
                "components": {
                    "kotlin23": "sha256:" + component23,
                    "kotlin24": "sha256:" + component24,
                },
                "manifestDigest": "sha256:" + "5" * 64,
                "mode": "RELEASE",
                "runtimeKey": "sha256:" + runtime_key,
                "workerIds": ["kotlin23", "kotlin24"],
                "workers": {
                    "kotlin23": {
                        "distribution": "workers/kotlin23/build/install/kotlin23",
                        "treeHash": "sha256:" + "6" * 64,
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
            self.assertEqual(set(core_manifest["workers"]), {"kotlin24"})
            self.assertEqual(set(core_manifest["components"]), {"kotlin24"})
            self.assertFalse((core_capsule / "workers" / "kotlin23").exists())
            self.assertEqual(
                list((destination / "v2" / "runtimes" / "components").iterdir()),
                [],
            )


if __name__ == "__main__":
    unittest.main()
