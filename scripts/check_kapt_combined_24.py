#!/usr/bin/env python3
"""Run the owned Kotlin 2.4.10 combined generated-Java regression."""

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SERVICE = "kapt-combined-24"
REMOTE = "https://example.invalid/owned/kapt-combined-24.git"
SOURCE = "src/main/kotlin/fixture/Service.kt"
SCOPE = ":/main"
TASKS = {":generateIndependent", ":kaptGenerateStubsKotlin", ":kaptKotlin",
         ":compileKotlin", ":compileJava", ":classes"}


@dataclass(frozen=True)
class Call:
    jvm_class: str
    method: str
    owner: str
    target: str
    descriptor: str
    expression: bytes
    opcode: str = "invokestatic"
    owner_descriptor: str | None = None


CALLS = (
    Call("fixture.LabelService", "label", "fixture/LabelService.label",
         "fixture/generated/GeneratedValue.value", "()Ljava/lang/String;", b"GeneratedValue.value()"),
    Call("fixture.ServiceKt", "independentNumber", "fixture/independentNumber",
         "fixture/generated/IndependentJava.number", "()I", b"fixture.generated.IndependentJava.number()"),
)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def read_json(path):
    return json.loads(path.read_bytes())


def write_json(path, data):
    path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def git(repo, *args, env=None):
    return subprocess.check_output(
        ["git", "-C", repo, "-c", "core.autocrlf=false", "-c", "core.hooksPath=/dev/null",
         "-c", "user.name=Owned Fixture", "-c", "user.email=fixture@example.invalid",
         "-c", "commit.gpgsign=false", *args], env=env,
    )


def prepare_sources(output, fixture="kapt-combined-24", remote=REMOTE):
    repo = output / "check-source"
    shutil.copytree(ROOT / "fixtures" / fixture, repo)
    shutil.copyfile(ROOT / "gradlew", repo / "gradlew")
    (repo / "gradlew").chmod(0o755)
    shutil.copyfile(ROOT / "gradle/wrapper/gradle-wrapper.jar", repo / "gradle/wrapper/gradle-wrapper.jar")
    git(repo, "init", "-qb", "main")
    git(repo, "remote", "add", "origin", remote)
    git(repo, "add", ".")
    dated = {**os.environ, "GIT_AUTHOR_DATE": "2026-11-19T00:00:00Z",
             "GIT_COMMITTER_DATE": "2026-11-19T00:00:00Z"}
    git(repo, "commit", "-qm", "Owned combined kapt and compileJava source producers", env=dated)
    revision = git(repo, "rev-parse", "HEAD").decode().strip()
    native = output / "native-source"
    subprocess.run(["git", "clone", "-q", "--no-local", repo, native], check=True)
    require(git(native, "rev-parse", "HEAD").decode().strip() == revision, "Native checkout revision diverged")
    return repo, native, revision


def verify_native(output, native, jdk25, environment):
    tasks = subprocess.run(
        [native / "gradlew", "--no-build-cache", "--no-configuration-cache", "--console=plain", ":classes"],
        cwd=native, env=environment, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300,
    )
    (output / "native-classes.log").write_text(tasks.stdout, encoding="utf-8")
    require(tasks.returncode == 0, "Native :classes failed; see native-classes.log")
    present = set(re.findall(r"^> Task (:\S+)[ \t]*$", tasks.stdout, flags=re.M))
    require(TASKS <= present, f"Tasks missing or reused in native :classes: {sorted(TASKS - present)}")
    kotlin_classes = native / "build/classes/kotlin/main"
    java_classes = native / "build/classes/java/main"
    for classfile in (
        kotlin_classes / "fixture/LabelService.class", kotlin_classes / "fixture/ServiceKt.class",
        java_classes / "fixture/generated/GeneratedValue.class", java_classes / "fixture/generated/IndependentJava.class",
    ):
        header = classfile.read_bytes()[:8]
        require(header.startswith(b"\xca\xfe\xba\xbe") and int.from_bytes(header[6:], "big") == 69,
                f"{classfile.name} is not a JVM 25 class")
    check_javap(output, kotlin_classes, jdk25, CALLS, "LabelService", "label")
    return sorted(TASKS)


def check_javap(output, classes, jdk, calls, open_class, open_method):
    open_name = f"fixture.{open_class}"
    disassembly = {name: subprocess.check_output(
        [jdk / "bin/javap", "-p", "-s", "-c", "-classpath", classes, name], text=True, timeout=30,
    ) for name in sorted({open_name} | {call.jvm_class for call in calls})}
    (output / "native-javap.log").write_text("\n".join(disassembly.values()), encoding="utf-8")
    require(f"public class {open_name} {{" in disassembly[open_name]
            and f"  public java.lang.String {open_method}();" in disassembly[open_name],
            "The selected all-open effect is absent from the native class or method")
    for call in calls:
        members = re.split(r"(?m)^  (?=\S)", disassembly[call.jvm_class])
        header = re.compile(rf" {re.escape(call.method)}\([^)]*\);$")
        matches = [member for member in members if header.search(member.splitlines()[0])]
        expected = rf"\b{call.opcode}\s+#\d+\s+// Method {re.escape(call.target)}:{re.escape(call.descriptor)}"
        require(len(matches) == 1 and f"descriptor: {call.owner_descriptor or call.descriptor}" in matches[0]
                and re.search(expected, matches[0]), f"Native {call.jvm_class}.{call.method} lost {call.target}")


def verify_boundary(checked, report):
    unresolved = checked.get("unresolved", {})
    require(checked.get("status") == "UNRESOLVED" and set(unresolved) == {SERVICE} and not checked.get("services"),
            "Check must report the expected unresolved compilation without captured service evidence")
    record = unresolved[SERVICE]
    require(record.get("reason") == "UNSUPPORTED_COMPILER_PLUGIN_ABI", "Check refused for an unexpected reason")
    saved = record.get("compilerPluginDiagnostic", {})
    diagnostic = saved.get("diagnostic", {})
    require(diagnostic.get("native") == {"check": "COMPILER_ROW", "outcome": "FAILED"}
            and diagnostic.get("abiRejection") == "KAPT_NATIVE_AUTHORITY_REQUIRED"
            and diagnostic.get("knownKaptRegistrar") is True
            and diagnostic.get("matches", {}).get("compiler") is False
            and diagnostic.get("requestedCompilerRow") == diagnostic.get("effectiveCompilerRow") == "2.4.10",
            f"Unexpected ABI diagnostic: {diagnostic}")
    services = report.get("services", [])
    require(report.get("status") == "COMPLETE" and report.get("snapshot") == checked["snapshot"]
            and len(services) == 1 and services[0].get("outcome") == "FAILED", "Support report did not retain the failed Check")
    failure = services[0].get("failure", {})
    require(failure.get("reason") == "UNSUPPORTED_COMPILER_PLUGIN_ABI"
            and failure.get("compilerPluginDiagnostic", {}).get("status") == "RECORDED"
            and failure["compilerPluginDiagnostic"].get("originalFailure") == saved.get("originalFailure")
            and failure["compilerPluginDiagnostic"].get("diagnostic") == diagnostic,
            "Support collect lost the original ABI diagnostic")
    return diagnostic


def verify_capture(checked, evidence, repo, revision, *, service=SERVICE, source=SOURCE, calls=CALLS):
    require(checked.get("status") == "CHECKED" and not checked.get("unresolved")
            and checked.get("services", {}).get(service, {}).get("revision") == revision,
            "Check did not capture the selected service and revision")
    manifest = read_json(evidence / "manifest.json")
    require(manifest.get("outcome") == "CAPTURED" and manifest.get("revision") == revision
            and manifest.get("service", {}).get("id") == service
            and manifest.get("retained", {}).get("snapshot") == checked["snapshot"],
            "Export does not describe this successful Check")
    tracked = git(repo, "show", f"{revision}:{source}")
    source_digest = "sha256:" + hashlib.sha256(tracked).hexdigest()
    occurrences = set()
    witnesses = []
    for part in manifest["parts"]:
        for row in read_json(evidence / part["path"])["records"]:
            fact = row.get("normalized", {})
            if row.get("kind") == "NATIVE_COMPILATION":
                witnesses.append(fact)
            span = fact.get("sourceSpan", {})
            site = fact.get("outlineSite", {})
            if (fact.get("kind") != "CALL" or fact.get("resolution") != "COMPILER_EXACT"
                    or fact.get("scope") != SCOPE or span.get("compilationScope") != SCOPE
                    or span.get("file") != source or site.get("file") != source
                    or span.get("fullCompilationSourceDigest") != source_digest):
                continue
            start, end = span["byteStart"], span["byteEnd"]
            if not (0 <= start < end <= len(tracked) and site.get("startLine") == tracked.count(b"\n", 0, start) + 1):
                continue
            for call in calls:
                owner = f"callable:{call.owner}#jvm:{call.owner_descriptor or call.descriptor}"
                target = f"callable:{call.target}#jvm:{call.descriptor}"
                if (span.get("ownerSymbolIdentity") == owner and fact.get("target") == target
                        and tracked[start:end] == call.expression):
                    occurrences.add((target, start, end))
    require(len(occurrences) == len(calls) and all(
        any(target == f"callable:{call.target}#jvm:{call.descriptor}" and tracked[start:end] == call.expression
            for target, start, end in occurrences) for call in calls),
            f"Export lacks both exact source-grounded generated calls: {sorted(occurrences)}")
    require(len(witnesses) == 1, "Export must contain one native compilation witness")
    return sorted(occurrences), witnesses[0]


def verify_witness(witness, native, revision, compiler, gradle, target, *, has_kapt=False, options=None):
    def hash_bytes(content):
        return "sha256:" + hashlib.sha256(content).hexdigest()

    def hash_json(value):
        return hash_bytes(json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode())

    def digests(root, pattern, relative):
        return {path.relative_to(relative).as_posix(): hash_bytes(path.read_bytes())
                for path in sorted(root.rglob(pattern))}

    src = digests(native / "src", "*.kt", native)
    build = native / "build"
    kapt = digests(build / "generated/source/kapt/main", "*.java", native)
    independent = digests(build / "generated/sources/independent/main", "*.java", native)
    require(has_kapt == bool(kapt) == bool(independent), "Expected separate generated inputs in the native build")
    classes = build / "classes/kotlin/main"
    expected = {
        "revision": revision, "scope": SCOPE, "compileTask": ":compileKotlin", "outcome": "EXECUTED",
        "requestedCompiler": compiler, "nativeGradleVersion": gradle, "nativeJdkMajor": target, "jvmTarget": str(target),
        "analyzerCompiler": "2.4.10",
        "comparison": "CLASS_FLAGS_INHERITANCE_METHODS_AND_JVM_CALLS_MATCH",
        "selectedSourcesDigest": hash_json({"kotlin": src, "java": dict(sorted((kapt | independent).items()))}),
        "nativeClassfilesDigest": hash_json(digests(classes, "*.class", classes)),
        "generatedJavaSelectionDigest": hash_json(kapt),
        "javaSourceProducersDigest": hash_json(
            [{"task": ":generateIndependent", "selectedJavaDigests": independent}] if independent else []),
    }
    if options is not None:
        expected["orderedOptionsDigest"] = hash_json(options)
    mismatches = [name for name, value in expected.items() if witness.get(name) != value]
    require(not mismatches, f"Native compilation witness differs from independent :classes: {mismatches}")


def check_project(out, launcher, env, repo, service, remote, *, exit_code):
    def cli(label, *parts, exit_code=0):
        with (out / f"{label}.stderr").open("wb") as stderr:
            done = subprocess.run([launcher, *parts], env=env, stdout=subprocess.PIPE, stderr=stderr, timeout=300)
        (out / f"{label}.json").write_bytes(done.stdout)
        allowed = exit_code if isinstance(exit_code, tuple) else (exit_code,)
        require(done.returncode in allowed, f"{label} exited {done.returncode}; see its saved output")
        return json.loads(done.stdout)

    require(not (repo / "build").exists() and not git(repo, "status", "--porcelain"),
            "Check checkout must start clean and without native outputs")
    docs = out / "docs"
    spec = out / "service.json"
    write_json(spec, {"schema": "codeclew-documentation-service/1.0", "id": service, "title": service,
                      "repositoryId": service, "repository": remote, "language": "kotlin",
                      "profile": "kotlin-jvm-gradle-analysis", "compilations": [SCOPE], "targetRef": "main"})
    initial = cli("init", "docs", "init", "--root", docs)
    cli("add", "docs", "service", "add", "--root", docs, "--input", spec, "--expected-input-digest", initial["inputDigest"])
    cli("bind", "docs", "bind", "--root", docs, "--service", service, "--repo", repo)
    return docs, cli, cli("check", "docs", "check", "--root", docs, exit_code=exit_code)


def run(args):
    out = args.output.resolve()
    out.mkdir(mode=0o700)
    jdk21, jdk25 = args.jdk21.resolve(), args.jdk25.resolve()
    env = {**os.environ, "JAVA_HOME": str(jdk25), "JDK21_HOME": str(jdk21), "JDK25_HOME": str(jdk25),
           "CODECLEW_WORKER_JAVA_HOME": str(jdk21), "CODECLEW_HOME": str(out / "state")}
    repo, native, revision = prepare_sources(out)
    native_tasks = verify_native(out, native, jdk25, env)
    boundary = args.expect == "abi-boundary"
    # Partial semantic coverage can set exit 3 even when every service was
    # captured. Verify CHECKED and the required exact evidence independently.
    docs, cli, checked = check_project(out, args.clew.resolve(), env, repo, SERVICE, REMOTE,
                                       exit_code=3 if boundary else (0, 3))
    snapshot = checked["snapshot"]
    result = {"status": "PASS", "expected": args.expect, "revision": revision, "snapshot": snapshot,
              "nativeTasks": native_tasks, "nativeCalls": [call.target + ":" + call.descriptor for call in CALLS]}
    if boundary:
        report = out / "support"
        cli("collect", "support", "collect", "--root", docs, "--service", SERVICE, "--snapshot", snapshot, "--output", report)
        result["boundary"] = verify_boundary(checked, read_json(report / "report.json"))
    else:
        evidence = out / "evidence"
        cli("export", "docs", "evidence", "export", "--root", docs, "--service", SERVICE, "--snapshot", snapshot,
            "--output", evidence)
        calls, witness = verify_capture(checked, evidence, repo, revision)
        verify_witness(witness, native, revision, "2.4.10", "9.4.1", 25, has_kapt=True,
                       options=["plugin:org.jetbrains.kotlin.allopen:preset=spring"])
        result["retainedCalls"] = calls
        result["nativeCompilationWitness"] = witness
    write_json(out / "result.json", result)
    print(json.dumps(result))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clew", type=Path, required=True, help="Path to a built package launcher")
    parser.add_argument("--jdk21", type=Path, required=True, help="JDK 21 home for Gradle, worker and processor")
    parser.add_argument("--jdk25", type=Path, required=True, help="JDK 25 home for compile toolchain and javap")
    parser.add_argument("--output", type=Path, required=True, help="New directory for logs, check and evidence")
    parser.add_argument("--expect", choices=("abi-boundary", "captured"), default="captured")
    run(parser.parse_args())
