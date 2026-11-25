#!/usr/bin/env python3
"""Require the packaged projector and Work transport suites with real toolchains."""

from pathlib import Path
import os
import runpy
import shutil
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest


ROOT = Path(__file__).resolve().parent.parent
PACKAGE = ROOT / "skills/codeclew/scripts/prepared_facts"
SUITES = ("test_prepared_java.py", "test_prepared_managed_languages.py", "test_work_transport.py")


def main():
    with tempfile.TemporaryDirectory(prefix="clew-projector-deps-") as directory:
        dependencies = Path(directory)
        for name in ("package.json", "package-lock.json"):
            shutil.copyfile(PACKAGE / name, dependencies / name)
        subprocess.run(
            ["npm", "ci", "--ignore-scripts", "--no-audit", "--no-fund", "--prefix", str(dependencies)],
            check=True,
        )
        os.environ["NODE_PATH"] = str(dependencies / "node_modules")
        suite = unittest.TestSuite()
        for name in SUITES:
            module = SimpleNamespace(**runpy.run_path(str(ROOT / "scripts" / name)))
            tests = unittest.defaultTestLoader.loadTestsFromModule(module)
            if not tests.countTestCases():
                sys.exit(f"Required test suite is empty: {name}")
            suite.addTests(tests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        if result.skipped:
            print("Required projector qualification must run every test.", file=sys.stderr)
        return 0 if result.wasSuccessful() and not result.skipped else 1


if __name__ == "__main__":
    raise SystemExit(main())
