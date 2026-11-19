#!/usr/bin/env python3
"""Capture and collect native kapt diagnostics on three owned synthetic projects."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess

GRADLE = """plugins {
    kotlin("jvm") version "2.2.21"
    kotlin("kapt") version "2.2.21"
}
repositories { mavenCentral() }
kotlin {
    jvmToolchain(21)
    compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_21) }
}
dependencies {
    compileOnly(project(":processor"))
    kapt(project(":processor"))
}
tasks.named("compileKotlin") {
    doFirst { throw GradleException("The selected Kotlin task must not run during Check") }
}
"""
PROCESSOR = """package sample;
import java.io.IOException;
import java.io.Writer;
import java.util.Set;
import javax.annotation.processing.*;
import javax.lang.model.SourceVersion;
import javax.lang.model.element.TypeElement;

@SupportedAnnotationTypes("sample.Generate")
@SupportedOptions("skip")
public final class ProcessorImpl extends AbstractProcessor {
    @Override public SourceVersion getSupportedSourceVersion() { return SourceVersion.RELEASE_21; }
    @Override public boolean process(Set<? extends TypeElement> annotations, RoundEnvironment round) {
        if (annotations.isEmpty() || processingEnv.getOptions().containsKey("skip")) return false;
        try (Writer out = processingEnv.getFiler().createSourceFile("sample.Generated").openWriter()) {
            out.write("package sample; public final class Generated { public static int value() { return 42; } }");
        } catch (IOException error) { throw new IllegalStateException(error); }
        return true;
    }
}
"""


def prepare_project(repo, mode):
    def write(relative, text):
        path = repo / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    root = Path(__file__).resolve().parents[1]
    options = {
        "positive": "",
        "options": 'kotlin.compilerOptions.freeCompilerArgs.add("-Xplugin=" + file("empty-plugin").absolutePath)\n',
        "outputs": 'kapt { arguments { arg("skip", "fixture") } }\n',
    }
    write("build.gradle.kts", GRADLE + options[mode])
    if mode == "options":
        write("empty-plugin/.keep", "")
    write("settings.gradle.kts", 'rootProject.name = "diagnostic-sample"\ninclude("processor")\n')
    write("processor/build.gradle.kts", 'plugins { `java-library` }\njava { toolchain { languageVersion.set(JavaLanguageVersion.of(21)) } }\n')
    write("processor/src/main/java/sample/Generate.java", "package sample; public @interface Generate {}\n")
    write("processor/src/main/java/sample/ProcessorImpl.java", PROCESSOR)
    write("processor/src/main/resources/META-INF/services/javax.annotation.processing.Processor", "sample.ProcessorImpl\n")
    expression = "Generated.value()" if mode != "outputs" else "42"
    write("src/main/kotlin/sample/Entry.kt", f"package sample\n@Generate class Entry\nfun value() = {expression}\n// DIAGNOSTIC_PRIVATE_SENTINEL\n")
    write("gradle/wrapper/gradle-wrapper.properties",
          "distributionUrl=https\\://services.gradle.org/distributions/gradle-8.14.1-bin.zip\n")
    shutil.copyfile(root / "gradle/wrapper/gradle-wrapper.jar", repo / "gradle/wrapper/gradle-wrapper.jar")
    shutil.copyfile(root / "gradlew", repo / "gradlew")
    (repo / "gradlew").chmod(0o700)
    url = f"https://example.invalid/owned/kapt-{mode}.git"
    git = ["git", "-C", str(repo), "-c", "core.hooksPath=/dev/null",
           "-c", "user.name=Owned Fixture", "-c", "user.email=fixture@example.invalid"]
    for args in (["init", "-qb", "main"], ["remote", "add", "origin", url], ["add", "."],
                 ["-c", "commit.gpgsign=false", "commit", "-qm", "Synthetic kapt diagnostic"]):
        subprocess.run(git + args, check=True)
    return url


def run(args):
    output = args.output.absolute()
    output.mkdir(mode=0o700)
    environment = {**os.environ, "CODECLEW_HOME": str(output / "state")}
    version = subprocess.check_output([args.clew, "--version"], env=environment, text=True).split()[1]

    def check(label, command, exits=(0,), env=None):
        with (output / (label + ".stderr")).open("wb") as stderr:
            result = subprocess.run(command, env=env or environment, stdout=subprocess.PIPE,
                                    stderr=stderr, check=False, timeout=300)
        (output / (label + ".json")).write_bytes(result.stdout)
        if result.returncode not in exits:
            raise RuntimeError(f"{label} exited {result.returncode}. See its saved stdout and stderr.")
        return json.loads(result.stdout)

    repos = {}
    for label in ("positive", "options", "outputs"):
        repo = output / ("repo-" + label)
        repos[label] = (repo, prepare_project(repo, label))

    def capture(label, project_mode, launcher, *, syntax=False):
        repo, url = repos[project_mode]
        docs = output / ("docs-" + label)
        command = [launcher, "docs"]
        check(label + "-init", [*command, "init", "--root", str(docs)])
        digest = check(label + "-list", [*command, "service", "list", "--root", str(docs)])["inputDigest"]
        service = {"schema": "codeclew-documentation-service/1.0", "id": "sample", "title": "Sample",
                   "repositoryId": "synthetic-kapt", "repository": url, "language": "kotlin", "targetRef": "main"}
        if syntax:
            service.update(profile="source-syntax", source={"roots": ["src/main/kotlin"], "dialect": "declared"})
        else:
            service.update(profile="kotlin-jvm-gradle-analysis", compilations=[":/main"])
        spec = output / (label + ".input.json")
        spec.write_text(json.dumps(service), encoding="utf-8")
        check(label + "-add", [*command, "service", "add", "--root", str(docs), "--input", str(spec),
                              "--expected-input-digest", digest])
        check(label + "-bind", [*command, "bind", "--root", str(docs), "--service", "sample", "--repo", str(repo)])
        return docs, check(label + "-check", [*command, "check", "--root", str(docs)], (0, 3))

    def report_for(label, docs, snapshot, collector, env):
        bundle = output / ("support-" + label)
        check(label + "-collect", [collector, "support", "collect", "--root", str(docs), "--snapshot",
                                  snapshot, "--output", str(bundle)], (0, 3), env)
        public = (bundle / "report.json").read_text()
        assert "DIAGNOSTIC_PRIVATE_SENTINEL" not in public and str(output) not in public
        assert not list(bundle.glob("PRIVATE*"))
        return json.loads(public)

    collector = args.collector or args.clew
    for label, expected in (("options", "FORBIDDEN_FREE_ARGS"), ("outputs", "GENERATED_JAVA_PRESENT"),
                            ("positive", None), ("syntax", None), ("legacy", "UNAVAILABLE")):
        if label == "legacy" and not args.legacy:
            continue
        launcher = args.legacy if label == "legacy" else args.clew
        mode = "options" if label in {"syntax", "legacy"} else label
        docs, captured = capture(label, mode, launcher, syntax=label == "syntax")
        report = report_for(label, docs, captured["snapshot"], collector, environment)
        row, = report["services"]
        if expected is None:
            assert row["outcome"] == "CAPTURED", report
            assert "failure" not in row
            if label == "syntax":
                assert row["producer"] == "codeclew-documentation-source/1.0", row
        else:
            failure = row["failure"]
            assert failure["reason"] == "UNSUPPORTED_COMPILER_PLUGIN_ABI", failure
            diagnostic = failure["compilerPluginDiagnostic"]
            if label == "legacy":
                assert diagnostic == {"status": "UNAVAILABLE"}
            else:
                assert diagnostic["status"] == "RECORDED", diagnostic
                assert diagnostic["originalFailure"]["cliVersion"] == version
                facts = diagnostic["diagnostic"]
                assert facts["native"]["check"] == expected and facts["native"]["outcome"] == "FAILED", facts
                assert facts["abiRejection"] == "COMPILER_VERSION_MISMATCH", facts
                assert facts["requestedCompilerRow"] == "2.2.21" and facts["effectiveCompilerRow"] == "2.4.10"
                assert "artifactCandidates" not in facts and "knownKaptRegistrar" not in facts
                assert facts["matches"]["pluginOptionsEmpty"] is True
                assert "generatedJavaSelected" not in facts["matches"]
                if label == "options":
                    assert "generatedJavaPresent" not in facts["matches"]
                else:
                    assert facts["matches"]["generatedJavaPresent"] is False
                assert facts["native"].get("freeArgClassifier") == ("XPLUGIN" if label == "options" else None)
        print(label, "PASS", captured["snapshot"], flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clew", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--legacy", help="Optional launcher for a real pre-diagnostic snapshot")
    parser.add_argument("--collector", help="Optional independent current launcher for support collection")
    run(parser.parse_args())
