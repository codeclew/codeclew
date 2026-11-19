#!/usr/bin/env python3
"""Verify all-open replay and native FIR evidence for the owned Kotlin 2.4.20 project."""

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
OVERLOADS = """
class Printer {
    fun show(value: Int): String = value.toString()
    fun show(value: String): String = value
}
fun render(printer: Printer): String = printer.show(42) + printer.show("x")
"""
OVERLOAD_CALLS = tuple(
    Call("fixture.LabelKt", "render", "fixture/render", "fixture/Printer.show", descriptor, expression,
         "invokevirtual", "(Lfixture/Printer;)Ljava/lang/String;")
    for descriptor, expression in (("(I)Ljava/lang/String;", b"printer.show(42)"),
                                   ("(Ljava/lang/String;)Ljava/lang/String;", b'printer.show("x")'))
)


def check_replay(out, launcher, jdk, env):
    repo, native, revision = prepare_sources(out, SERVICE, REMOTE)
    command = [native / "gradlew", "--no-build-cache", "--no-configuration-cache", "--console=plain", ":classes"]
    compiled = subprocess.run(command, cwd=native, env=env, text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=300)
    (out / "native-classes.log").write_text(compiled.stdout, encoding="utf-8")
    require(compiled.returncode == 0 and re.search(r"(?m)^> Task :compileKotlin[ \t]*$", compiled.stdout),
            "The independent native build did not execute :compileKotlin")
    check_javap(out, native / "build/classes/kotlin/main", jdk, CALLS, "Label", "text")
    docs, cli, checked = check_project(out, launcher, env, repo, SERVICE, REMOTE, exit_code=(0, 3))
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

    return {"revision": revision, "snapshot": checked["snapshot"], "nativeOptionOff": "REFUSED",
            "retainedCalls": calls, "nativeCompilationWitness": witness}


def check_native_fir(out, launcher, jdk, env):
    out.mkdir()
    repo, _, _ = prepare_sources(out, SERVICE, REMOTE)
    config = repo / "build.gradle.kts"
    with config.open("a", encoding="utf-8") as script:
        script.write('\nkotlin.compilerOptions.freeCompilerArgs.addAll("-Xjsr305=strict", "-Xno-param-assertions")\n')
        script.write("kotlin.compilerOptions.javaParameters.set(true)\n")
    with (repo / SOURCE).open("a", encoding="utf-8") as source:
        source.write(OVERLOADS)
    git(repo, "add", "build.gradle.kts", SOURCE)
    git(repo, "commit", "-qm", "Owned compilation with native compiler options and same-line overloads")
    revision = git(repo, "rev-parse", "HEAD").decode().strip()
    native = out / "native-selected"
    subprocess.run(["git", "clone", "-q", "--no-local", repo, native], check=True)
    command = [native / "gradlew", "--no-build-cache", "--no-configuration-cache", "--console=plain", ":classes"]
    compiled = subprocess.run(command, cwd=native, env=env, text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=300)
    (out / "native-classes.log").write_text(compiled.stdout, encoding="utf-8")
    require(compiled.returncode == 0 and re.search(r"(?m)^> Task :compileKotlin[ \t]*$", compiled.stdout),
            "The independent build did not execute :compileKotlin")
    check_javap(out, native / "build/classes/kotlin/main", jdk, CALLS + OVERLOAD_CALLS, "Label", "text")
    require("checkNotNullParameter" not in (out / "native-javap.log").read_text(),
            "Native bytecode did not retain -Xno-param-assertions")
    docs, cli, checked = check_project(out, launcher, env, repo, SERVICE, REMOTE, exit_code=(0, 3))
    evidence = out / "evidence"
    cli("export", "docs", "evidence", "export", "--root", docs, "--service", SERVICE, "--snapshot",
        checked["snapshot"], "--output", evidence)
    calls, witness = verify_capture(checked, evidence, repo, revision, service=SERVICE, source=SOURCE,
                                    calls=CALLS + OVERLOAD_CALLS)
    require(witness.get("schema") == "codeclew-kotlin-native-fir/1", "Expected facts from the executing project compiler")
    verify_witness(witness, native, revision, "2.4.20", "9.6.1", 21)

    removed = out / "removed-annotation"
    removed.mkdir()
    control = removed / "source"
    subprocess.run(["git", "clone", "-q", "--no-local", repo, control], check=True)
    git(control, "remote", "set-url", "origin", REMOTE)
    path = control / "build.gradle.kts"
    original = path.read_text(encoding="utf-8")
    path.write_text(original.replace('allOpen { annotation("fixture.OpenForExtension") }\n', ""), encoding="utf-8")
    require(path.read_text(encoding="utf-8") != original, "Native control must remove the annotation option")
    git(control, "add", "build.gradle.kts")
    git(control, "commit", "-qm", "Make the inherited native declaration final")
    _, _, rejected = check_project(removed, launcher, env, control, SERVICE, REMOTE, exit_code=3)
    failure = rejected.get("unresolved", {}).get(SERVICE, {})
    require(not rejected.get("services") and failure.get("reason") == "UNSUPPORTED_PROJECT_CONFIGURATION"
            and "NATIVE_KOTLIN_COMPILE_FAILED" in json.dumps(failure),
            "Removing the annotation option must fail in the selected native compiler")
    return {"revision": revision, "snapshot": checked["snapshot"], "retainedCalls": calls,
            "nativeCompilationWitness": witness, "nativeOptionOff": failure}


def run(args):
    out = args.output.resolve()
    out.mkdir(mode=0o700)
    jdk, launcher = args.jdk21.resolve(), args.clew.resolve()
    env = {**os.environ, "JAVA_HOME": str(jdk), "CODECLEW_WORKER_JAVA_HOME": str(jdk),
           "CODECLEW_HOME": str(out / "state")}
    result = {"status": "PASS"}
    if not args.native_fir_only:
        result.update(check_replay(out, launcher, jdk, env))
    result["nativeFir"] = check_native_fir(out / "native-fir", launcher, jdk, env)
    write_json(out / "result.json", result)
    print(json.dumps(result))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clew", type=Path, required=True)
    parser.add_argument("--jdk21", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--native-fir-only", action="store_true", help="Reuse an already accepted replay baseline")
    run(parser.parse_args())
