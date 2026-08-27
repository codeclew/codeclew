#!/usr/bin/env python3
"""Run one real Kotlin project/semantic-engine qualification row."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parent.parent
GOLDENS = ROOT / "scripts" / "qualification" / "kotlin-compatibility-goldens.json"
RESULT_PREFIX = "CODECLEW_KOTLIN_QUALIFICATION_RESULT="

ROWS = {
    "k24-exact": {
        "version": "2.4.10",
        "language": "2.4",
        "outcome": "QUALIFIED",
    },
    "k24-language-23": {
        "version": "2.4.10",
        "language": "2.3",
        "outcome": "QUALIFIED",
    },
    "k24-from-240": {
        "version": "2.4.0",
        "language": "2.4",
        "outcome": "QUALIFIED",
    },
    "k24-from-230": {
        "version": "2.3.0",
        "language": "2.3",
        "outcome": "QUALIFIED",
        "k23_oracle": True,
    },
    "k24-from-2121": {
        "version": "2.1.21",
        "language": "2.1",
        "outcome": "QUALIFIED",
    },
    "k24-from-2121-serialization": {
        "version": "2.1.21",
        "language": "2.1",
        "outcome": "QUALIFIED",
        "serialization": True,
    },
    "allopen-negative": {
        "version": "2.3.0",
        "language": "2.3",
        "outcome": "UNSUPPORTED_COMPILER_PLUGIN_ABI",
        "plugin": "allopen",
    },
    "k19-negative": {
        "version": "1.9.24",
        "language": "1.9",
        "outcome": "UNSUPPORTED_PROJECT_CONFIGURATION",
    },
}


def kotlin_version_constant(version: str) -> str:
    major, minor = version.split(".")[:2]
    return f"KOTLIN_{major}_{minor}"


def build_script(row: dict[str, object]) -> str:
    version = str(row["version"])
    plugins = [f'kotlin("jvm") version "{version}"']
    if row.get("serialization"):
        plugins.append(f'kotlin("plugin.serialization") version "{version}"')
    if row.get("plugin") == "allopen":
        plugins.append(f'kotlin("plugin.allopen") version "{version}"')
    language = kotlin_version_constant(str(row["language"]))
    return (
        "plugins {\n    "
        + "\n    ".join(plugins)
        + "\n}\n"
        + "kotlin {\n"
        + "    jvmToolchain(21)\n"
        + "    compilerOptions {\n"
        + f"        languageVersion.set(org.jetbrains.kotlin.gradle.dsl.KotlinVersion.{language})\n"
        + f"        apiVersion.set(org.jetbrains.kotlin.gradle.dsl.KotlinVersion.{language})\n"
        + "    }\n"
        + "}\n"
        + "dependencies { testImplementation(kotlin(\"test\")) }\n"
        + "tasks.test { useJUnitPlatform() }\n"
    )


def prepare_fixture(destination: Path, row: dict[str, object]) -> None:
    shutil.copytree(
        ROOT / "fixtures" / "kotlin-basic",
        destination,
        ignore=shutil.ignore_patterns(".git", ".gradle", ".semantic-thread", "build"),
    )
    (destination / "build.gradle.kts").write_text(build_script(row), encoding="utf-8")
    sources = destination / "src"
    shutil.rmtree(sources)
    main = sources / "main" / "kotlin" / "com" / "acme"
    main.mkdir(parents=True)
    (main / "QualifiedA.kt").write_text(
        "package com.acme\n\n"
        "interface Source { fun read(value: Int): Number }\n"
        "abstract class IntegerSource : Source { abstract override fun read(value: Int): Int }\n",
        encoding="utf-8",
    )
    (main / "QualifiedB.kt").write_text(
        "package com.acme\n\n"
        "class Box(val value: String)\n"
        "interface NullableReader { fun nullableLength(value: String?): Int }\n",
        encoding="utf-8",
    )
    (main / "Empty.kt").write_text("", encoding="utf-8")
    subprocess.run(["git", "init", "-q", "-b", "main"], cwd=destination, check=True)
    subprocess.run(["git", "add", "."], cwd=destination, check=True)
    subprocess.run(
        [
            "git",
            "-c",
            "user.name=Codeclew Qualification",
            "-c",
            "user.email=codeclew@localhost",
            "commit",
            "-qm",
            "qualification fixture",
        ],
        cwd=destination,
        check=True,
    )


def load_goldens() -> dict[str, str]:
    if not GOLDENS.is_file():
        return {}
    value = json.loads(GOLDENS.read_text(encoding="utf-8"))
    if value.get("schema") != "codeclew-kotlin-compatibility-goldens/1.0":
        raise SystemExit("invalid Kotlin compatibility golden schema")
    rows = value.get("rows")
    if not isinstance(rows, dict) or not all(
        isinstance(key, str) and isinstance(digest, str) for key, digest in rows.items()
    ):
        raise SystemExit("invalid Kotlin compatibility golden rows")
    return rows


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--row", choices=sorted(ROWS), required=True)
    parser.add_argument("--full-lifecycle", action="store_true")
    parser.add_argument("--discover-golden", action="store_true")
    args = parser.parse_args()
    row = ROWS[args.row]
    positive = row["outcome"] == "QUALIFIED"
    goldens = load_goldens()
    expected_golden = goldens.get(args.row)
    if positive and expected_golden is None and not args.discover_golden:
        raise SystemExit(f"missing checked-in qualification golden for {args.row}")

    with tempfile.TemporaryDirectory(prefix="codeclew-kotlin-qualification-") as directory:
        fixture = Path(directory) / "repo"
        prepare_fixture(fixture, row)
        environment = os.environ.copy()
        environment.update(
            {
                "CODECLEW_KOTLIN_QUALIFICATION_FIXTURE": str(fixture),
                "CODECLEW_KOTLIN_QUALIFICATION_PROJECT_VERSION": str(row["version"]),
                "CODECLEW_KOTLIN_QUALIFICATION_OUTCOME": str(row["outcome"]),
            }
        )
        if args.full_lifecycle and positive:
            environment["CODECLEW_KOTLIN_QUALIFICATION_FULL_LIFECYCLE"] = "1"
        if row.get("k23_oracle"):
            environment["CODECLEW_KOTLIN_QUALIFICATION_K23_ORACLE"] = "1"
        if row.get("serialization"):
            environment["CODECLEW_KOTLIN_QUALIFICATION_SERIALIZATION"] = "1"
        if expected_golden is not None:
            environment["CODECLEW_KOTLIN_QUALIFICATION_GOLDEN"] = expected_golden
        completed = subprocess.run(
            [
                "cargo",
                "test",
                "--locked",
                "-p",
                "clew",
                "--lib",
                "kotlin_adapter_v2::tests::kotlin_engine_qualification_probe",
                "--",
                "--ignored",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ],
            cwd=ROOT,
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        if completed.returncode != 0:
            sys.stderr.write(completed.stdout)
            return completed.returncode
        result = next(
            (
                json.loads(line.split(RESULT_PREFIX, 1)[1])
                for line in completed.stdout.splitlines()
                if RESULT_PREFIX in line
            ),
            None,
        )
        if positive:
            if result is None:
                raise SystemExit("qualification test did not emit a result")
            print(json.dumps({"row": args.row, **result}, sort_keys=True))
        else:
            print(json.dumps({"row": args.row, "status": "REJECTED_AS_EXPECTED"}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
