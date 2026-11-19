#!/usr/bin/env python3
"""Verify native all-open annotation replay for the owned Kotlin 2.4.20 project."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_kapt_combined_24 import (
    Call, check_javap, check_project, git, prepare_sources, require, verify_capture, verify_witness, write_json,
)

SERVICE = "kotlin-allopen-24"
REMOTE = f"https://example.invalid/owned/{SERVICE}.git"
SOURCE = "src/main/kotlin/fixture/Label.kt"
EXPECTED_OPTION = "plugin:org.jetbrains.kotlin.allopen:annotation=fixture.OpenForExtension"
TARGET = "fixture/Label.text"
DESCRIPTOR = "()Ljava/lang/String;"
CALLS = (
    Call("fixture.Derived", "text", "fixture/Derived.text", TARGET, DESCRIPTOR, b"super.text()", "invokespecial"),
    Call("fixture.LabelKt", "display", "fixture/display", TARGET, DESCRIPTOR, b"label.text()", "invokevirtual",
         "(Lfixture/Label;)Ljava/lang/String;"),
)


def run(args):
    out = args.output.resolve()
    out.mkdir(mode=0o700)
    jdk = args.jdk21.resolve()
    env = {**os.environ, "JAVA_HOME": str(jdk), "CODECLEW_WORKER_JAVA_HOME": str(jdk),
           "CODECLEW_HOME": str(out / "state")}
    repo, native, revision = prepare_sources(out, SERVICE, REMOTE)
    command = [native / "gradlew", "--no-build-cache", "--no-configuration-cache", "--console=plain", ":classes"]
    compiled = subprocess.run(command, cwd=native, env=env, text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=300)
    (out / "native-classes.log").write_text(compiled.stdout, encoding="utf-8")
    require(compiled.returncode == 0 and re.search(r"(?m)^> Task :compileKotlin[ \t]*$", compiled.stdout),
            "The independent native build did not execute :compileKotlin")
    check_javap(out, native / "build/classes/kotlin/main", jdk, CALLS, "Label", "text")
    docs, cli, checked = check_project(out, args.clew.resolve(), env, repo, SERVICE, REMOTE, exit_code=0)
    evidence = out / "evidence"
    cli("export", "docs", "evidence", "export", "--root", docs, "--service", SERVICE,
        "--snapshot", checked["snapshot"], "--output", evidence)
    calls, witness = verify_capture(checked, evidence, repo, revision, service=SERVICE, source=SOURCE, calls=CALLS)
    verify_witness(witness, native, revision, "2.4.20", "9.6.1", 21, options=[EXPECTED_OPTION])
    # Remove the effective plugin option in the independent native checkout.
    # The sources still need the open modality to compile the subclass.
    config = native / "build.gradle.kts"
    original = git(native, "show", f"{revision}:build.gradle.kts").decode()
    config.write_text(original.replace('allOpen { annotation("fixture.OpenForExtension") }\n', ""), encoding="utf-8")
    failed = subprocess.run(command + ["--rerun-tasks"], cwd=native, env=env, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, timeout=300)
    (out / "native-option-off.log").write_text(failed.stdout, encoding="utf-8")
    require(failed.returncode != 0 and "final" in failed.stdout.lower() and "Label.kt" in failed.stdout,
            "Removing the annotation option did not demonstrate the native effect")

    unsupported = out / "unsupported"
    unsupported.mkdir()
    modified, _, _ = prepare_sources(unsupported, SERVICE, REMOTE)
    with (modified / "build.gradle.kts").open("a", encoding="utf-8") as script:
        script.write('\nkotlin.compilerOptions.freeCompilerArgs.add("-Xjsr305=strict")\n')
    git(modified, "add", "build.gradle.kts")
    git(modified, "commit", "-qm", "Owned compilation with out-of-scope replay flag")
    _, _, rejected = check_project(unsupported, args.clew.resolve(), env, modified, SERVICE, REMOTE, exit_code=3)
    failure = rejected.get("unresolved", {}).get(SERVICE, {})
    diagnostic = failure.get("compilerPluginDiagnostic", {}).get("diagnostic", {})
    require(not rejected.get("services") and failure.get("reason") == "UNSUPPORTED_COMPILER_PLUGIN_ABI"
            and diagnostic.get("abiRejection") == "ALL_OPEN_NATIVE_AUTHORITY_REQUIRED"
            and diagnostic.get("native") == {"check": "NORMAL_PLUGIN_OPTIONS", "outcome": "FAILED"},
            "Out-of-scope options did not fail closed with the typed all-open boundary")
    result = {"status": "PASS", "revision": revision, "snapshot": checked["snapshot"], "nativeOptionOff": "REFUSED",
              "retainedCalls": calls, "nativeCompilationWitness": witness, "unsupportedDiagnostic": diagnostic}
    write_json(out / "result.json", result)
    print(json.dumps(result))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clew", type=Path, required=True)
    parser.add_argument("--jdk21", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    run(parser.parse_args())
