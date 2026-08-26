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
INSTALLER = ROOT / "site" / "install.sh"
LAUNCHER = ROOT / "packaging" / "macos" / "clew"


class QuietHandler(SimpleHTTPRequestHandler):
    def log_message(self, _format: str, *_arguments: object) -> None:
        pass


class MacosDistributionTest(unittest.TestCase):
    def test_installer_is_idempotent_and_never_builds_locally(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            temporary = Path(value)
            document_root = temporary / "www"
            downloads = document_root / "releases" / "latest" / "download"
            package = temporary / "payload" / "codeclew"
            (package / "bin").mkdir(parents=True)
            (package / "VERSION").write_text("v0.1.0\n", encoding="ascii")
            launcher = package / "bin" / "clew"
            shutil.copyfile(LAUNCHER, launcher)
            launcher.chmod(0o500)
            binary = package / "source" / "clew"
            binary.parent.mkdir()
            binary.write_text(
                "#!/bin/sh\n"
                "[ \"${1:-}\" = capabilities ] || exit 2\n"
                "printf '%s\\n' '{\"schema\":\"codeclew-capabilities/1.0\",\"status\":\"PILOT_READY\"}'\n",
                encoding="utf-8",
            )
            binary.chmod(0o500)
            seed = package / "seed" / "release-N-test" / "seed.json"
            seed.parent.mkdir(parents=True)
            seed.write_text("{}\n", encoding="ascii")
            downloads.mkdir(parents=True)
            asset = downloads / "codeclew-macos-arm64.tar.gz"
            with tarfile.open(asset, "w:gz") as archive:
                archive.add(package, arcname="codeclew")
            digest = hashlib.sha256(asset.read_bytes()).hexdigest()
            checksum = downloads / f"{asset.name}.sha256"
            checksum.write_text(f"{digest}  {asset.name}\n", encoding="ascii")

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
                    for message in [
                        "[1/6] Checking macOS and required tools",
                        "[2/6] Downloading the macOS arm64 release",
                        "[3/6] Checksum verified",
                        "[4/6] Extracting the sealed runtime",
                        "[5/6] Activating Codeclew v0.1.0",
                        "[6/6] Runtime verification passed",
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

                checksum.write_text(f"{'0' * 64}  {asset.name}\n", encoding="ascii")
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
        for path in [INSTALLER, LAUNCHER]:
            completed = subprocess.run(
                ["/bin/sh", "-n", str(path)],
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr.decode())


if __name__ == "__main__":
    unittest.main()
