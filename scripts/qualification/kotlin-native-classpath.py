#!/usr/bin/env python3
"""Exercise the native Kotlin classpath seal through saved-source Checks."""
from __future__ import annotations

import argparse
from hashlib import sha256
import json
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
SOURCE = "src/main/kotlin/demo/Reading.kt"
SCOPE = ":/main"
REMOTE = "https://example.invalid/neutral/classpath.git"
KONVERT_BUILD = """plugins {
    id 'org.jetbrains.kotlin.jvm' version '2.4.20'
    id 'com.google.devtools.ksp' version '2.3.12'
}
repositories { mavenCentral() }
dependencies {
    implementation 'io.mcarle:konvert-api:4.5.1'
    ksp 'io.mcarle:konvert:4.5.1'
}
kotlin { jvmToolchain(21) }
"""
KONVERT_SOURCE = """package demo
import io.mcarle.konvert.api.Konverter

data class Reading(val amount: Int)
data class Card(val amount: Int)
@Konverter
interface ReadingMapper { fun toCard(reading: Reading): Card }

fun selectedCard(reading: Reading): Card = ReadingMapperImpl.toCard(reading)
"""
OTHER_BUILD = """plugins { id 'org.jetbrains.kotlin.jvm' version '2.4.20' }
repositories { mavenCentral() }
kotlin { jvmToolchain(21) }

def sourceRoot = layout.buildDirectory.dir('generated/survey/kotlin')
def directory = layout.buildDirectory.dir('survey-output/classes')
def archive = layout.buildDirectory.file('survey-output/required.jar')
def stage = tasks.register('prepareGaugeSources') {
    outputs.dir(sourceRoot)
    OUTPUT_DECLARATION
    doLast {
        def output = sourceRoot.get().asFile
        output.mkdirs()
        new File(output, 'GeneratedGauge.kt').text = 'package demo; object GeneratedGauge { fun read(): Int = 7 }'
        EXTRA_OUTPUT
    }
}
tasks.named('compileKotlin') {
    dependsOn(stage)
    source(sourceRoot)
    libraries.from(CLASSPATH_INPUT)
    AFTER_COMPILE
}
OTHER_VARIANT
"""
EXISTING_OUTPUT = """def binary = directory.get().asFile
        binary.mkdirs()
        def javaSource = new File(sourceRoot.get().asFile.parentFile, 'BinaryFlag.java')
        javaSource.text = 'package demo; public class BinaryFlag { public static int number() { return 9; } }'
        def javac = new File(System.getProperty('java.home'), 'bin/javac')
        if (new ProcessBuilder(javac.path, '-d', binary.path, javaSource.path).inheritIO().start().waitFor() != 0) {
            throw new GradleException('Fixture javac failed')
        }
"""
POSITIVES = {"konvert", "independent", "existing"}
NEGATIVES = {"missing-jar", "other-variant", "state-changed"}


ABSENT_OUTPUT = """project.delete(directory.get().asFile)
        assert !directory.get().asFile.exists()
"""


def git(repo, *args):
    return subprocess.check_output(["git", *args], cwd=repo)


def fixture(repo, case):
    repo.mkdir()
    shutil.copy2(ROOT / "gradlew", repo / "gradlew")
    shutil.copytree(ROOT / "gradle/wrapper", repo / "gradle/wrapper")
    source = repo / SOURCE
    source.parent.mkdir(parents=True)
    (repo / "settings.gradle").write_text(
        "pluginManagement { repositories { mavenCentral(); gradlePluginPortal() } }\n"
        "rootProject.name = 'measurement'\n")
    (repo / ".gitignore").write_text(".gradle/\n.kotlin/\nbuild/\n")
    if case == "konvert":
        build, content = KONVERT_BUILD, KONVERT_SOURCE
        expectation = ("demo.ReadingMapperImpl.toCard", b"ReadingMapperImpl.toCard(reading)")
    else:
        declaration = ("outputs.file(archive)" if case == "missing-jar" else ""
                       if case == "other-variant" else "outputs.dir(directory)")
        variant = ("def preview = tasks.register('preparePreview') { outputs.dir(directory); doLast {} }\n"
                   "tasks.named('compileTestKotlin') { dependsOn(preview) }" if case == "other-variant" else "")
        build = (OTHER_BUILD.replace("OUTPUT_DECLARATION", declaration)
                 .replace("CLASSPATH_INPUT", "archive" if case == "missing-jar" else "directory")
                 .replace("EXTRA_OUTPUT", EXISTING_OUTPUT if case == "existing" else ABSENT_OUTPUT)
                 .replace("AFTER_COMPILE", """doLast {
        assert !directory.get().asFile.exists()
        assert directory.get().asFile.mkdirs()
    }""" if case == "state-changed" else "")
                 .replace("OTHER_VARIANT", variant))
        content = "package demo\nfun selectedCard(): Int = GeneratedGauge.read()\n"
        expectation = ("demo.GeneratedGauge.read", b"GeneratedGauge.read()")
        if case == "existing":
            content += "fun selectedBinary(): Int = BinaryFlag.number()\n"
    (repo / "build.gradle").write_text(build)
    source.write_text(content)
    git(repo, "init", "-qb", "main")
    git(repo, "remote", "add", "origin", REMOTE)
    git(repo, "add", ".")
    git(repo, "-c", "user.name=Neutral Fixture", "-c", "user.email=fixture@example.invalid",
        "commit", "-qm", "Commit neutral classpath fixture")
    source.write_text(content + "\n// Saved local selection.\n")
    return expectation


def check(base, clew, case, expect_baseline=False):
    base.mkdir()
    repo, docs = base / "repo", base / "docs"
    expected = fixture(repo, case)
    head, index = git(repo, "rev-parse", "HEAD").decode().strip(), (repo / ".git/index").read_bytes()
    saved = (repo / SOURCE).read_bytes()
    started = time.monotonic()

    def cli(label, *command, codes=(0,)):
        with (base / (label + ".stderr")).open("wb") as error:
            process = subprocess.run([clew, *command], stderr=error, stdout=subprocess.PIPE, timeout=300)
        (base / (label + ".json")).write_bytes(process.stdout)
        assert process.returncode in codes, (label, process.returncode, base)
        return json.loads(process.stdout)

    specification = base / "service.json"
    specification.write_text(json.dumps({
        "schema": "codeclew-documentation-service/1.0", "id": "measurement", "title": "Measurement",
        "repositoryId": "measurement", "repository": REMOTE, "language": "kotlin",
        "profile": "kotlin-jvm-gradle-analysis", "compilations": [SCOPE], "targetRef": "main",
    }))
    initial = cli("init", "docs", "init", "--root", docs)
    cli("add", "docs", "service", "add", "--root", docs, "--input", specification,
        "--expected-input-digest", initial["inputDigest"])
    cli("bind", "docs", "bind", "--root", docs, "--service", "measurement", "--repo", repo)
    checked = cli("check", "docs", "check", "--root", docs, "--service", "measurement", "--working-tree", codes=(0, 3))
    assert saved == (repo / SOURCE).read_bytes() and index == (repo / ".git/index").read_bytes()
    assert head == git(repo, "rev-parse", "HEAD").decode().strip()
    assert not (repo / "build").exists()
    evidence = {"case": case, "snapshot": checked["snapshot"], "baseRevision": head}
    if case in NEGATIVES or expect_baseline:
        failure = checked["unresolved"]["measurement"]
        assert not checked.get("services") and checked["status"] == "UNRESOLVED", checked
        assert "NATIVE_KOTLIN_RECEIPT_FAILED" in failure["nextAction"], failure
        return {**evidence, "outcome": "REFUSED", "reason": "NATIVE_KOTLIN_RECEIPT_FAILED",
                "seconds": round(time.monotonic() - started, 3)}

    assert checked["status"] == "CHECKED" and not checked["unresolved"], checked
    exported = base / "evidence"
    cli("export", "docs", "evidence", "export", "--root", docs, "--service", "measurement",
        "--snapshot", checked["snapshot"], "--output", exported)
    manifest = json.loads((exported / "manifest.json").read_bytes())
    rows = [row for part in manifest["parts"] for row in json.loads((exported / part["path"]).read_bytes())["records"]]
    receipt, = [row["normalized"] for row in rows if row.get("kind") == "NATIVE_COMPILATION"]
    selection, = [row["normalized"] for row in rows if row.get("kind") == "WORKING_TREE_SOURCE"]
    assert selection["kind"] == "WORKING_TREE" and selection["baseRevision"] == head, selection
    assert receipt["sourceSelection"] == selection and receipt["revision"] == selection["snapshotDigest"].removeprefix("sha256:"), (receipt, selection)
    assert receipt["schema"] == "codeclew-kotlin-native-fir/1" and receipt["scope"] == SCOPE, receipt
    assert (receipt["requestedCompiler"], receipt["nativeGradleVersion"], receipt["jvmTarget"]) == ("2.4.20", "9.6.1", "21")
    digest = "sha256:" + sha256(saved).hexdigest()
    calls = []
    for row in rows:
        fact = row.get("normalized") or {}
        span = fact.get("sourceSpan", {})
        if (fact.get("kind"), fact.get("resolution"), fact.get("scope")) != ("CALL", "COMPILER_EXACT", SCOPE):
            continue
        if (span.get("file"), span.get("compilationScope"), span.get("fullCompilationSourceDigest")) != (SOURCE, SCOPE, digest):
            continue
        start, end = span["byteStart"], span["byteEnd"]
        calls.append((fact["target"], saved[start:end]))
    expectations = [expected]
    if case == "existing":
        expectations.append(("demo.BinaryFlag.number", b"BinaryFlag.number()"))
    links = []
    for member, expression in expectations:
        match, = [target for target, snippet in calls if snippet == expression and target.startswith("callable:demo/" + member.removeprefix("demo.") + "#")]
        links.append({"member": match, "expression": expression.decode(), "sourceDigest": digest})
    return {**evidence, "outcome": "CAPTURED", "nativeReceipt": receipt, "calls": links,
            "seconds": round(time.monotonic() - started, 3)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clew", type=Path, default=ROOT / "clew")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", action="append", choices=sorted(POSITIVES | NEGATIVES))
    parser.add_argument("--expect-baseline", action="store_true")
    options = parser.parse_args()
    options.output.mkdir(mode=0o700)
    cases = options.case or (["konvert"] if options.expect_baseline else sorted(POSITIVES | NEGATIVES))
    results = [check(options.output / case, options.clew.resolve(), case, options.expect_baseline) for case in cases]
    result = {"status": "PASS", "cases": results}
    (options.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))
