#!/usr/bin/env python3
"""Exercise CI dispatch and failure propagation without running toolchains."""

from collections import Counter
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
SUITES = ("checks", "retained-docs", "native-pages", "managed-docs", "java", "kotlin", "usability")
STUB = r'''
import json, os
from pathlib import Path
import sys
name, *args = sys.argv[1:]
with open(os.environ["CLEW_TEST_COMMAND_LOG"], "a") as stream:
    stream.write(json.dumps([name, *args]) + "\n")
failure = os.environ.get("CLEW_TEST_FAIL_COMMAND")
if failure and " ".join([name, *args]).startswith(failure):
    sys.exit(23)
if name == "python3" and "-c" in args:
    print(os.environ["CLEW_TEST_CI_TMP_BASE"])
elif name == "dotnet" and args == ["--version"]:
    print(os.environ.get("CLEW_TEST_DOTNET_VERSION", "10.0.401"))
elif name == "dotnet" and args[0] == "publish":
    artifact = Path("workers/dotnet/publish/Codeclew.CSharp.Analyzer.dll")
    artifact.parent.mkdir(parents=True, exist_ok=True)
    artifact.touch()
elif name == "cargo" and "--list" in args:
    print(args[args.index("--lib") + 1] + ": test")
'''


class CiDispatchTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="clew-ci-dispatch-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "scripts").mkdir()
        (self.root / "workers/dotnet").mkdir(parents=True)
        self.script = self.root / "scripts/ci-verify.sh"
        shutil.copyfile(ROOT / "scripts/ci-verify.sh", self.script)
        helper = self.root / "stub.py"
        helper.write_text(STUB)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ("python3", "dotnet", "cargo", "node", "gradlew"):
            path = self.root / "gradlew" if name == "gradlew" else self.bin / name
            path.write_text(
                "#!/bin/sh\nexec " + shlex.quote(sys.executable) + " -I -S "
                + shlex.quote(str(helper)) + " " + name + ' "$@"\n'
            )
            path.chmod(0o700)
        self.jdk = self.root / "jdk25"
        (self.jdk / "bin").mkdir(parents=True)
        java = self.jdk / "bin/java"
        java.write_text("#!/bin/sh\nexit 0\n")
        java.chmod(0o700)
        self.counter = 0

    def run_ci(self, *args, failure="", jdk25=True, dotnet_version="10.0.401"):
        self.counter += 1
        log = self.root / f"commands-{self.counter}.jsonl"
        environment = dict(os.environ)
        environment.update(
            PATH=str(self.bin) + os.pathsep + os.defpath,
            CLEW_TEST_COMMAND_LOG=str(log),
            CLEW_TEST_CI_TMP_BASE=str(self.root),
            CLEW_TEST_FAIL_COMMAND=failure,
            CLEW_TEST_DOTNET_VERSION=dotnet_version,
        )
        environment.pop("CODECLEW_TEST_JDK25_HOME", None)
        if jdk25:
            environment["CODECLEW_TEST_JDK25_HOME"] = str(self.jdk)
        result = subprocess.run(
            ["/bin/sh", str(self.script), *args], env=environment,
            text=True, capture_output=True, timeout=30,
        )
        commands = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
        return result, commands

    def test_suite_union_matches_complete_execution(self):
        full, full_commands = self.run_ci()
        self.assertEqual(full.returncode, 0, full.stderr)
        self.assertNotIn("scope", json.loads(full.stdout.splitlines()[-1]))
        selected_commands = []
        for suite in SUITES:
            result, commands = self.run_ci("--suite", suite)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout.splitlines()[-1])["scope"], suite)
            self.assertIn("[ci] COMPLETE " + suite, result.stdout)
            selected_commands.extend(commands)

        def tasks(commands):
            return Counter(tuple(c) for c in commands if not (c[0] == "python3" and "-c" in c))

        self.assertEqual(tasks(full_commands), tasks(selected_commands))

    def test_native_suite_does_not_require_dotnet_or_jdk25(self):
        result, commands = self.run_ci("--suite", "native-pages", failure="dotnet", jdk25=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(c[0] == "dotnet" for c in commands))

    def test_usability_runs_alone_without_project_toolchain_qualification(self):
        result, commands = self.run_ci("--suite", "usability", jdk25=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        tasks = [c for c in commands if not (c[0] == "python3" and "-c" in c)]
        self.assertEqual(tasks, [["python3", "-I", "-S", "scripts/usability-smoke.py"]])

    def test_usability_failure_fails_verification(self):
        result, _ = self.run_ci(
            "--suite", "usability", failure="python3 -I -S scripts/usability-smoke.py", jdk25=False,
        )
        self.assertEqual(result.returncode, 23)
        self.assertNotIn('"status":"PASSED"', result.stdout)

    def test_tool_failure_stops_later_commands(self):
        result, commands = self.run_ci("--suite", "checks", failure="cargo fmt")
        self.assertEqual(result.returncode, 23)
        self.assertNotIn('"status":"PASSED"', result.stdout)
        self.assertFalse(any(c[:2] == ["cargo", "clippy"] for c in commands))

    def test_csharp_function_failure_cannot_be_masked(self):
        result, commands = self.run_ci("--suite", "checks", failure="dotnet publish")
        self.assertEqual(result.returncode, 23)
        self.assertNotIn('"status":"PASSED"', result.stdout)
        self.assertFalse(any(c[:2] == ["cargo", "test"] for c in commands))

    def test_legacy_csharp_only_keeps_its_scope_without_jdk25(self):
        result, _ = self.run_ci("--csharp-only", jdk25=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout.splitlines()[-1])["scope"], "CSHARP")

    def test_checks_reject_unsupported_dotnet_before_tasks(self):
        result, commands = self.run_ci("--suite", "checks", dotnet_version="9.0.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(commands, [["dotnet", "--version"]])

    def test_kotlin_requires_its_project_toolchain(self):
        result, commands = self.run_ci("--suite", "kotlin", jdk25=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('"status":"PASSED"', result.stdout)
        self.assertFalse(any(c[0] in ("cargo", "gradlew") for c in commands))

    def test_invalid_arguments_fail_before_tool_execution(self):
        for args in (("--suite",), ("--suite", "unknown"), ("--csharp-only", "extra")):
            with self.subTest(args=args):
                result, commands = self.run_ci(*args)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(commands, [])


if __name__ == "__main__":
    unittest.main()
