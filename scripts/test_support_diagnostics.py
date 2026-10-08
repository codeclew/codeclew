#!/usr/bin/env python3
"""Bounded streaming and real shell-entrypoint lifecycle tests, offline."""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parent.parent
WRAPPER = ROOT / "bootstrap" / "diagnostics.py"
LIMIT = 1024 * 1024


class DiagnosticsTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.launcher = self.root / "command"

    def command(self, program: str) -> None:
        self.launcher.write_text("#!" + sys.executable + "\n" + program, encoding="utf-8")
        self.launcher.chmod(0o700)

    def invoke(self, output: Path, private: bool = False, args: list[str] | None = None) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run([sys.executable, "-I", "-S", "-B", str(WRAPPER), "--launcher", str(self.launcher),
                               "--diagnostics", str(output), *(["--include-private-logs"] if private else []),
                               *(args or ["sample"])], capture_output=True, check=False)

    def manifest(self, output: Path) -> dict:
        value = json.loads((output / "manifest.json").read_bytes())
        self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o700)
        total = 0
        for artifact in value["artifacts"]:
            path = output / artifact["path"]
            self.assertFalse(path.is_symlink())
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            self.assertLessEqual(path.stat().st_size, LIMIT)
            total += path.stat().st_size
        self.assertLessEqual(total + (output / "manifest.json").stat().st_size, value["limits"]["totalBytes"])
        return value

    def test_success_and_failure_preserve_streams_exit_and_one_invocation(self) -> None:
        for code in [0, 2, 3, 7]:
            with self.subTest(code=code):
                count = self.root / ("count" + str(code))
                self.command("import os,sys\n" + f"with open({str(count)!r},'ab') as f: f.write(b'1')\n" +
                             "os.write(1,b'original human or JSON output\\x00\\xff\\n')\n"
                             "os.write(2,b'original stderr PRIVATE_SECRET\\n')\n" + f"sys.exit({code})\n")
                normal = subprocess.run([str(self.launcher), "sample"], capture_output=True, check=False)
                count.unlink()
                output = self.root / ("bundle" + str(code))
                wrapped = self.invoke(output)
                self.assertEqual(wrapped.returncode, normal.returncode)
                self.assertEqual(wrapped.stdout, normal.stdout)
                self.assertTrue(wrapped.stderr.startswith(normal.stderr))
                self.assertNotIn(b"PRIVATE_SECRET", wrapped.stderr[len(normal.stderr):])
                self.assertEqual(count.read_bytes(), b"1")
                manifest = self.manifest(output)
                self.assertEqual(manifest["invocation"]["executionCount"], 1)
                self.assertEqual(manifest["sharing"], "ALLOWLISTED_METADATA")
                for path in output.iterdir():
                    self.assertNotIn(b"PRIVATE_SECRET", path.read_bytes())

    def test_oversized_output_is_streamed_in_full_with_bounded_private_tails(self) -> None:
        self.command("import os\nfor _ in range(48):\n os.write(1,b'x'*65536)\n os.write(2,b'y'*65536)\n"
                     "os.write(1,b'PRIVATE_FINAL\\n')\nos.write(2,b'PRIVATE_ERROR_FINAL\\n')\n")
        output = self.root / "oversized"
        result = self.invoke(output, private=True)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"x" * (48 * 65536) + b"PRIVATE_FINAL\n")
        self.assertTrue(result.stderr.startswith(b"y" * (48 * 65536) + b"PRIVATE_ERROR_FINAL\n"))
        manifest = self.manifest(output)
        self.assertEqual(manifest["sharing"], "PRIVATE_REVIEW_REQUIRED")
        for name in ["stdout", "stderr"]:
            metadata = manifest["invocation"][name]
            self.assertEqual(metadata["retainedBytes"], LIMIT)
            self.assertGreater(metadata["droppedBytes"], 2 * LIMIT)
            self.assertEqual((output / ("PRIVATE_" + name + ".tail")).stat().st_size, LIMIT)
        self.assertNotIn(b"PRIVATE_FINAL", (output / "report.json").read_bytes())

    def test_existing_symlink_and_inside_docs_outputs_are_refused_without_hiding_failure(self) -> None:
        self.command("import os,sys\nos.write(1,b'original failure\\n')\nsys.exit(3)\n")
        existing = self.root / "existing"
        existing.mkdir()
        (existing / "sentinel").write_text("preserved")
        link = self.root / "link"
        link.symlink_to(existing)
        for output in [existing, link, existing / "inside"]:
            args = ["docs", "check", "--root", str(existing)] if output.name == "inside" else None
            result = self.invoke(output, args=args)
            self.assertEqual(result.returncode, 3)
            self.assertEqual(result.stdout, b"original failure\n")
            self.assertIn(b"collection unavailable", result.stderr)
        self.assertEqual((existing / "sentinel").read_text(), "preserved")
        self.assertEqual(sorted(p.name for p in existing.iterdir()), ["sentinel"])

    def test_source_launcher_catches_bootstrap_failure_before_core(self) -> None:
        source = self.root / "source"
        (source / "bootstrap").mkdir(parents=True)
        shutil.copyfile(ROOT / "clew", source / "clew")
        (source / "clew").chmod(0o700)
        shutil.copyfile(WRAPPER, source / "bootstrap" / "diagnostics.py")
        (source / "bootstrap" / "clew_bootstrap.py").write_text(
            "import sys\nprint('{\"schema\":\"codeclew-bootstrap-error/2.0\",\"error\":\"PRIVATE_BOOTSTRAP_SECRET\"}',file=sys.stderr)\nsys.exit(7)\n")
        normal = subprocess.run([str(source / "clew"), "capabilities"], capture_output=True, check=False)
        output = self.root / "source-bundle"
        wrapped = subprocess.run([str(source / "clew"), "--diagnostics", str(output), "capabilities"], capture_output=True, check=False)
        self.assertEqual(wrapped.returncode, normal.returncode)
        self.assertEqual(wrapped.stdout, normal.stdout)
        self.assertTrue(wrapped.stderr.startswith(normal.stderr))
        self.assertEqual(json.loads((output / "report.json").read_bytes())["sourceStage"], "BOOTSTRAP")
        self.manifest(output)
        for path in output.iterdir():
            self.assertNotIn(b"PRIVATE_BOOTSTRAP_SECRET", path.read_bytes())

    def test_installed_launcher_catches_profile_failure_and_forwards_success(self) -> None:
        release = self.root / "releases" / "v-test"
        (release / "bin").mkdir(parents=True)
        (release / "source" / "bootstrap").mkdir(parents=True)
        shutil.copyfile(ROOT / "packaging" / "macos" / "clew", release / "bin" / "clew")
        (release / "bin" / "clew").chmod(0o700)
        shutil.copyfile(WRAPPER, release / "source" / "bootstrap" / "diagnostics.py")
        entrypoint = self.root / "installed-clew"
        entrypoint.symlink_to(release / "bin" / "clew")
        normal = subprocess.run([str(entrypoint), "capabilities"], capture_output=True, check=False)
        output = self.root / "installed-failure"
        wrapped = subprocess.run([str(entrypoint), "--diagnostics", str(output), "capabilities"], capture_output=True, check=False)
        self.assertEqual(wrapped.returncode, 7)
        self.assertEqual(wrapped.stdout, normal.stdout)
        self.assertTrue(wrapped.stderr.startswith(normal.stderr))
        self.assertEqual(json.loads((output / "report.json").read_bytes())["sourceStage"], "INSTALLATION")
        (release / "PROFILE").write_text("core\n")
        seed = release / "seed" / "release-N-test" / "seed.json"
        seed.parent.mkdir(parents=True)
        seed.write_text("{}")
        (release / "source" / "clew").write_text("#!/bin/sh\nprintf '%s\\n' 'original installed human output'\n")
        (release / "source" / "clew").chmod(0o700)
        output = self.root / "installed-success"
        wrapped = subprocess.run([str(entrypoint), "--diagnostics", str(output), "doctor"], capture_output=True, check=False)
        self.assertEqual(wrapped.returncode, 0)
        self.assertEqual(wrapped.stdout, b"original installed human output\n")
        self.manifest(output)

    def test_private_docs_check_reuses_debug_output_within_same_run(self) -> None:
        args_file = self.root / "args"
        self.command("import json,os,sys\n" + f"open({str(args_file)!r},'w').write(json.dumps(sys.argv[1:]))\n"
                     "assert os.environ['CLEW_DIAGNOSTIC_DEBUG_DIR'] == sys.argv[-1]\n")
        output = self.root / "debug-output"
        result = self.invoke(output, private=True, args=["docs", "check", "--root", str(self.root / "docs")])
        self.assertEqual(result.returncode, 0)
        args = json.loads(args_file.read_text())
        self.assertEqual(args[-2:], ["--debug-output", str(output.resolve() / "private")])
        self.assertEqual(stat.S_IMODE((output / "private").stat().st_mode), 0o700)

    def test_interrupt_terminates_child_group_and_finalizes_partial_bundle(self) -> None:
        pid_file = self.root / "pid"
        self.command("import os,signal,time\n" + f"open({str(pid_file)!r},'w').write(str(os.getpid()))\n"
                     "signal.signal(signal.SIGINT,signal.SIG_IGN)\nwhile True: time.sleep(0.1)\n")
        output = self.root / "interrupted"
        process = subprocess.Popen([sys.executable, "-I", "-S", "-B", str(WRAPPER), "--launcher", str(self.launcher),
                                    "--diagnostics", str(output), "sample"], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        deadline = time.monotonic() + 5
        while not pid_file.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        self.assertTrue(pid_file.exists())
        child_pid = int(pid_file.read_text())
        process.send_signal(signal.SIGINT)
        _, stderr = process.communicate(timeout=8)
        self.assertEqual(process.returncode, 143)
        self.assertIn(b"bundle finalized", stderr)
        with self.assertRaises(ProcessLookupError):
            os.kill(child_pid, 0)
        self.manifest(output)

    def test_interrupt_still_escalates_after_both_pipes_close(self) -> None:
        pid_file = self.root / "closed-pid"
        self.command("import os,signal,time\n" + f"open({str(pid_file)!r},'w').write(str(os.getpid()))\n"
                     "signal.signal(signal.SIGINT,signal.SIG_IGN)\nos.close(1)\nos.close(2)\nwhile True: time.sleep(0.1)\n")
        output = self.root / "closed-interrupted"
        process = subprocess.Popen([sys.executable, "-I", "-S", "-B", str(WRAPPER), "--launcher", str(self.launcher),
                                    "--diagnostics", str(output), "sample"], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        deadline = time.monotonic() + 5
        while not pid_file.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        self.assertTrue(pid_file.exists())
        time.sleep(0.1)
        process.send_signal(signal.SIGINT)
        process.communicate(timeout=8)
        self.assertEqual(process.returncode, 143)
        self.manifest(output)


if __name__ == "__main__":
    unittest.main()
