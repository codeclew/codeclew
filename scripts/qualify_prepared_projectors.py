#!/usr/bin/env python3
"""Require the source projector suites or the installed Java admission test."""

import argparse
import json
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
SOURCE_SKILL = ROOT / "skills/codeclew"
SUITES = (("test_prepared_java.py", "PackagedJava"),
          ("test_prepared_managed_languages.py", None), ("test_work_transport.py", None))


def install_dependencies(directory):
    subprocess.run(
        ["npm", "ci", "--ignore-scripts", "--no-audit", "--no-fund", "--prefix", str(directory)],
        check=True,
    )


def source_tests():
    suite = unittest.TestSuite()
    for name, case in SUITES:
        module = runpy.run_path(str(ROOT / "scripts" / name))
        loader = unittest.defaultTestLoader
        tests = (loader.loadTestsFromTestCase(module[case]) if case else
                 loader.loadTestsFromModule(SimpleNamespace(**module)))
        if not tests.countTestCases():
            sys.exit(f"Required test suite is empty: {name}")
        suite.addTests(tests)
    return suite


def installed_skill(launcher):
    package = launcher.resolve(strict=True).parent.parent
    metadata = json.loads((package / "release.json").read_text())
    if not isinstance(metadata, dict) or metadata.get("schema") not in (
        "codeclew-release/2.0", "codeclew-local-release-candidate/1.0",
    ):
        raise ValueError("Java admission requires an installed release launcher.")
    return package / "source/skills/codeclew"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--installed-launcher", type=Path,
                        help="run the Java admission test using an offline installation")
    args = parser.parse_args()
    launcher = args.installed_launcher.absolute() if args.installed_launcher else None
    try:
        skill = installed_skill(launcher) if launcher else SOURCE_SKILL
    except (OSError, ValueError) as error:
        parser.error(str(error))

    with tempfile.TemporaryDirectory(prefix="clew-projector-deps-") as directory:
        dependencies = Path(directory)
        package = skill / "scripts/prepared_facts"
        for name in ("package.json", "package-lock.json"):
            shutil.copyfile(package / name, dependencies / name)
        install_dependencies(dependencies)
        os.environ["NODE_PATH"] = str(dependencies / "node_modules")
        if launcher:
            renderer = dependencies / "markdown-renderer"
            shutil.copytree(skill / "examples/markdown-renderer", renderer)
            install_dependencies(renderer)
            case = runpy.run_path(str(ROOT / "scripts/test_prepared_java.py"))["InstalledJava"]
            suite = unittest.TestSuite([case(launcher, skill, renderer)])
        else:
            suite = source_tests()
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        if result.skipped:
            print("Required projector qualification must run every test.", file=sys.stderr)
        return 0 if result.wasSuccessful() and not result.skipped else 1


if __name__ == "__main__":
    raise SystemExit(main())
