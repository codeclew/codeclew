#!/usr/bin/env python3
"""Offline contract tests for the public macOS installer and release surface."""

from __future__ import annotations

import hashlib
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import threading
import unittest


ROOT = Path(__file__).resolve().parent.parent
SOURCE_LAUNCHER = ROOT / "clew"
INSTALLER = ROOT / "site" / "install.sh"
LAUNCHER = ROOT / "packaging" / "macos" / "clew"
UPGRADER = ROOT / "packaging" / "macos" / "upgrade"


class QuietHandler(SimpleHTTPRequestHandler):
    def log_message(self, _format: str, *_arguments: object) -> None:
        pass


class MacosDistributionTest(unittest.TestCase):
    def test_installer_is_idempotent_and_never_builds_locally(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            temporary = Path(value)
            document_root = temporary / "www"
            package = temporary / "payload" / "codeclew"
            (package / "bin").mkdir(parents=True)
            (package / "VERSION").write_text("v0.1.0\n", encoding="ascii")
            (package / "PROFILE").write_text("core\n", encoding="ascii")
            launcher = package / "bin" / "clew"
            shutil.copyfile(LAUNCHER, launcher)
            launcher.chmod(0o500)
            binary = package / "source" / "clew"
            binary.parent.mkdir()
            binary.write_text(
                "#!/bin/sh\n"
                "case \"${1:-}\" in\n"
                "  --version) version=$(sed -n '1p' \"$(dirname -- \"$0\")/../VERSION\"); "
                "printf 'clew %s\\n' \"${version#v}\" ;;\n"
                "  capabilities) printf '%s\\n' "
                "'{\"schema\":\"codeclew-capabilities/1.0\",\"status\":\"PILOT_READY\"}' ;;\n"
                "  *) exit 2 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            binary.chmod(0o500)
            bundled_upgrader = package / "source" / "packaging" / "macos" / "upgrade"
            bundled_upgrader.parent.mkdir(parents=True)
            shutil.copyfile(UPGRADER, bundled_upgrader)
            bundled_installer = package / "source" / "site" / "install.sh"
            bundled_installer.parent.mkdir(parents=True)
            shutil.copyfile(INSTALLER, bundled_installer)
            seed = package / "seed" / "release-N-test" / "seed.json"
            seed.parent.mkdir(parents=True)
            seed.write_text("{}\n", encoding="ascii")

            def publish(
                downloads: Path, version: str, profile: str = "core"
            ) -> tuple[Path, Path]:
                (package / "VERSION").write_text(f"{version}\n", encoding="ascii")
                (package / "PROFILE").write_text(f"{profile}\n", encoding="ascii")
                downloads.mkdir(parents=True, exist_ok=True)
                asset_name = (
                    "codeclew-macos-arm64.tar.gz"
                    if profile == "core"
                    else f"codeclew-{profile}-macos-arm64.tar.gz"
                )
                asset = downloads / asset_name
                with tarfile.open(asset, "w:gz") as archive:
                    archive.add(package, arcname="codeclew")
                digest = hashlib.sha256(asset.read_bytes()).hexdigest()
                checksum = downloads / f"{asset.name}.sha256"
                checksum.write_text(f"{digest}  {asset.name}\n", encoding="ascii")
                return asset, checksum

            initial_asset, initial_checksum = publish(
                document_root / "releases" / "download" / "v0.1.0", "v0.1.0"
            )
            publish(
                document_root / "releases" / "download" / "v0.1.0",
                "v0.1.0",
                "kotlin23",
            )
            publish(document_root / "releases" / "download" / "v0.1.1", "v0.1.1")
            publish(
                document_root / "releases" / "download" / "v0.1.1",
                "v0.1.1",
                "kotlin23",
            )
            release_api = document_root / "release-api.json"
            release_api.write_text('{"tag_name":"v0.1.0"}\n', encoding="ascii")

            fake_bin = temporary / "fake-bin"
            fake_bin.mkdir()
            uname = fake_bin / "uname"
            uname.write_text(
                "#!/bin/sh\n"
                "case \"${1:-}\" in\n"
                "  -s) printf '%s\\n' Darwin ;;\n"
                "  -m) printf '%s\\n' arm64 ;;\n"
                "  *) exit 2 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            uname.chmod(0o500)

            previous = Path.cwd()
            os.chdir(document_root)
            server = ThreadingHTTPServer(("127.0.0.1", 0), QuietHandler)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                environment = dict(os.environ)
                environment.update(
                    {
                        "CODECLEW_ALLOW_INSECURE_DOWNLOAD": "1",
                        "CODECLEW_BIN_DIR": str(temporary / "bin"),
                        "CODECLEW_INSTALL_ROOT": str(temporary / "install"),
                        "CODECLEW_RELEASE_BASE": (
                            f"http://127.0.0.1:{server.server_port}/releases"
                        ),
                        "CODECLEW_RELEASE_API": (
                            f"http://127.0.0.1:{server.server_port}/release-api.json"
                        ),
                        "PATH": f"{fake_bin}{os.pathsep}{environment['PATH']}",
                    }
                )
                for _attempt in range(2):
                    completed = subprocess.run(
                        ["/bin/sh", str(INSTALLER)],
                        cwd=ROOT,
                        env=environment,
                        stdin=subprocess.DEVNULL,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE,
                        check=False,
                        text=True,
                    )
                    self.assertEqual(completed.returncode, 0, completed.stderr)
                    self.assertIn("Codeclew v0.1.0 installed", completed.stdout)
                    self.assertIn("clew doctor attach --human", completed.stdout)
                    for message in [
                        "[1/7] Checking macOS and required tools",
                        "[3/7] Downloading the macOS arm64 core profile",
                        "[4/7] Checksum verified",
                        "[5/7] Extracting the sealed runtime",
                        "[6/7] Activating Codeclew v0.1.0 (core)",
                        "[7/7] Runtime verification passed",
                    ]:
                        self.assertIn(message, completed.stderr)
                installed = temporary / "bin" / "clew"
                self.assertTrue(installed.is_symlink())
                result = subprocess.run(
                    [str(installed), "capabilities"],
                    check=True,
                    stdout=subprocess.PIPE,
                    text=True,
                )
                self.assertIn("codeclew-capabilities/1.0", result.stdout)

                current = subprocess.run(
                    [str(installed), "upgrade"],
                    env=environment,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=False,
                    text=True,
                )
                self.assertEqual(current.returncode, 0, current.stderr)
                self.assertIn("v0.1.0 is already up to date", current.stdout)
                self.assertIn("Checking for updates from v0.1.0", current.stderr)

                release_api.write_text('{"tag_name":"v0.1.1"}\n', encoding="ascii")
                upgraded = subprocess.run(
                    [str(installed), "upgrade"],
                    env=environment,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=False,
                    text=True,
                )
                self.assertEqual(upgraded.returncode, 0, upgraded.stderr)
                self.assertIn("Codeclew v0.1.1 installed", upgraded.stdout)
                self.assertIn("Updating Codeclew from v0.1.0 to v0.1.1", upgraded.stderr)
                self.assertIn("v0.1.1-macos-arm64-core", str(installed.resolve()))

                current_again = subprocess.run(
                    [str(installed), "upgrade"],
                    env=environment,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=False,
                    text=True,
                )
                self.assertEqual(current_again.returncode, 0, current_again.stderr)
                self.assertIn("v0.1.1 is already up to date", current_again.stdout)

                packed = subprocess.run(
                    [str(installed), "pack", "install", "kotlin23"],
                    env=environment,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=False,
                    text=True,
                )
                self.assertEqual(packed.returncode, 0, packed.stderr)
                self.assertIn("kotlin23 profile", packed.stdout)
                self.assertIn(
                    "v0.1.1-macos-arm64-kotlin23", str(installed.resolve())
                )
                listed = subprocess.run(
                    [str(installed), "pack", "list"],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=False,
                    text=True,
                )
                self.assertEqual(listed.returncode, 0, listed.stderr)
                self.assertIn("Kotlin 2.3.0 preview", listed.stdout)
                unpacked = subprocess.run(
                    [str(installed), "pack", "remove", "kotlin23"],
                    env=environment,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=False,
                    text=True,
                )
                self.assertEqual(unpacked.returncode, 0, unpacked.stderr)
                self.assertIn("core profile", unpacked.stdout)
                self.assertIn("v0.1.1-macos-arm64-core", str(installed.resolve()))

                binary.chmod(0o700)
                binary.write_text(
                    "#!/bin/sh\n"
                    "case \"${1:-}\" in\n"
                    "  --version) printf '%s\\n' 'clew 0.1.1' ;;\n"
                    "  capabilities) printf '%s\\n' "
                    "'{\"schema\":\"codeclew-capabilities/1.0\",\"status\":\"PILOT_READY\"}' ;;\n"
                    "  *) exit 2 ;;\n"
                    "esac\n",
                    encoding="utf-8",
                )
                binary.chmod(0o500)
                publish(document_root / "releases" / "download" / "v0.1.2", "v0.1.2")
                environment["CODECLEW_VERSION"] = "v0.1.2"
                mismatched = subprocess.run(
                    ["/bin/sh", str(INSTALLER)],
                    cwd=ROOT,
                    env=environment,
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=False,
                    text=True,
                )
                self.assertNotEqual(mismatched.returncode, 0)
                self.assertIn(
                    "CLI version does not match release metadata", mismatched.stderr
                )
                self.assertIn("v0.1.1-macos-arm64-core", str(installed.resolve()))

                release_api.write_text('{"tag_name":"v0.1.0"}\n', encoding="ascii")
                initial_checksum.write_text(
                    f"{'0' * 64}  {initial_asset.name}\n", encoding="ascii"
                )
                environment["CODECLEW_VERSION"] = "latest"
                rejected = subprocess.run(
                    ["/bin/sh", str(INSTALLER)],
                    cwd=ROOT,
                    env=environment,
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=False,
                    text=True,
                )
                self.assertNotEqual(rejected.returncode, 0)
                self.assertIn("checksum mismatch", rejected.stderr)
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)
                os.chdir(previous)

        installer = INSTALLER.read_text(encoding="utf-8")
        for forbidden in ["cargo build", "rustc ", "gradle ", "./gradlew", "mvn "]:
            self.assertNotIn(forbidden, installer)
        self.assertIn("REPOSITORY=codeclew/codeclew", installer)

    def test_shell_entrypoints_are_syntactically_valid(self) -> None:
        for path in [SOURCE_LAUNCHER, INSTALLER, LAUNCHER, UPGRADER]:
            completed = subprocess.run(
                ["/bin/sh", "-n", str(path)],
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr.decode())

    def test_source_checkout_upgrade_points_to_git_without_bootstrapping(self) -> None:
        completed = subprocess.run(
            [str(SOURCE_LAUNCHER), "upgrade"],
            cwd=ROOT,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            text=True,
        )
        self.assertEqual(completed.returncode, 2)
        self.assertIn("source checkout", completed.stdout)
        self.assertIn("Git", completed.stdout)
        self.assertEqual(completed.stderr, "")


if __name__ == "__main__":
    unittest.main()
