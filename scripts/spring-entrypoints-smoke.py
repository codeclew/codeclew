#!/usr/bin/env python3
"""Public CLI acceptance: Kotlin 2.4.10 + Java 17 Spring roots across repositories.

Uses real Spring 6 / Kafka 3 annotation jars already in GRADLE_USER_HOME's
standard module cache. Run after building/sealing the current maintainer runtime;
CLEW_SMOKE_LAUNCHER may select a prepared release launcher. No service is started.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent


def run(args: list[str], cwd: Path, env: dict[str, str]) -> str:
    process = subprocess.run(args, cwd=cwd, env=env, text=True, stdin=subprocess.DEVNULL,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=900)
    if process.returncode:
        raise AssertionError(f"{Path(args[0]).name} failed ({process.returncode}): "
                             f"{process.stdout[-4000:]} {process.stderr[-2000:]}")
    return process.stdout


def spring_jars() -> list[Path]:
    cache = Path(os.environ.get("GRADLE_USER_HOME", str(Path.home() / ".gradle"))) / "caches/modules-2/files-2.1"
    result = []
    for group, name, major in [
        ("org.springframework", name, "6")
        for name in ("spring-core", "spring-beans", "spring-jcl", "spring-web", "spring-context", "spring-messaging")
    ] + [("org.springframework.kafka", "spring-kafka", "3")]:
        jars = sorted((cache / group / name).glob(f"{major}.*/*/{name}-*.jar"))
        if not jars:
            raise AssertionError(f"cached {group}:{name}:{major}.x jar required")
        result.append(jars[-1])
    return result


def fixture(root: Path, language: str, jars: list[Path], env: dict[str, str]) -> Path:
    repository = root / language
    repository.mkdir()
    shutil.copy2(ROOT / "fixtures/kotlin-basic/gradlew", repository / "gradlew")
    shutil.copytree(ROOT / "fixtures/kotlin-basic/gradle", repository / "gradle")
    (repository / "gradlew").chmod(0o755)
    (repository / ".gitignore").write_text(".gradle/\nbuild/\n.kotlin/\n")
    (repository / "settings.gradle.kts").write_text(
        'pluginManagement { repositories { gradlePluginPortal(); mavenCentral() } }\n'
        'dependencyResolutionManagement { repositories { mavenCentral() } }\n'
        f'rootProject.name = "spring-entrypoints-{language}"\n')
    (repository / "gradle.properties").write_text("org.gradle.daemon=false\norg.gradle.workers.max=2\n")
    (repository / "libs").mkdir()
    for jar in jars:
        shutil.copy2(jar, repository / "libs" / jar.name)
    plugins = ('plugins { kotlin("jvm") version "2.4.10" }\nkotlin { jvmToolchain(21) }\n'
               if language == "kotlin" else
               'plugins { java }\njava { toolchain { languageVersion = JavaLanguageVersion.of(21) } }\n'
               'tasks.withType<JavaCompile>().configureEach { options.release = 17 }\n')
    (repository / "build.gradle.kts").write_text(plugins +
        'dependencies { implementation(fileTree("libs") { include("*.jar") }) }\n')
    source = repository / f"src/main/{language}/example"
    source.mkdir(parents=True)
    if language == "kotlin":
        body = r'''package example
import org.springframework.web.bind.annotation.*
import org.springframework.kafka.annotation.KafkaListener
import org.springframework.scheduling.annotation.Scheduled
@RestController
@RequestMapping("/api")
class Entrypoints {
    @GetMapping("/items") fun http(): String = "ok"
    @KafkaListener(topics = ["${topic}"]) fun consume(message: String) { message.length }
    @Scheduled(fixedDelayString = "${delay:1000}") fun tick() {}
}
'''.replace('"${', '"\\${')
    else:
        body = r'''package example;
import org.springframework.web.bind.annotation.*;
import org.springframework.kafka.annotation.KafkaListener;
import org.springframework.scheduling.annotation.Scheduled;
@RestController @RequestMapping("/api")
class Entrypoints {
    @GetMapping("/items") String http() { return "ok"; }
    @KafkaListener(topics = {"${topic}"}) void consume(String message) {}
    @Scheduled(fixedDelayString = "${delay:1000}") void tick() {}
}
'''
    (source / ("Entrypoints.kt" if language == "kotlin" else "Entrypoints.java")).write_text(body)
    for args in (("init", "-q", "-b", "main"), ("config", "user.name", "Codeclew Spring Smoke"),
                 ("config", "user.email", "spring-smoke@localhost"), ("add", "."),
                 ("commit", "-q", "-m", "Spring annotation fixture")):
        run(["git", *args], repository, env)
    return repository


def verify(entries: list[dict], sessions: set[str]) -> None:
    assert len(entries) == 3 * len(sessions), entries
    assert len({entry["id"] for entry in entries}) == len(entries)
    assert {entry["sessionId"] for entry in entries} == sessions
    assert len({entry["repositoryKey"] for entry in entries}) == len(sessions)
    for session in sessions:
        rows = [entry for entry in entries if entry["sessionId"] == session]
        assert {row["kind"] for row in rows} == {"HTTP_ENDPOINT", "KAFKA_LISTENER", "SCHEDULED_JOB"}
        for row in rows:
            assert row["runtimeActivation"] == "UNPROVEN"
            assert row["symbolIdentity"] and row["file"] and row["generation"] and row["evidence"]
            assert row["binding"]["registration"] == "RUNTIME_CONDITIONAL"
        http = next(row for row in rows if row["kind"] == "HTTP_ENDPOINT")
        assert http["trigger"]["paths"] == ["/api/items"], http
        assert http["trigger"]["methods"] == ["GET"], http
        kafka = next(row for row in rows if row["kind"] == "KAFKA_LISTENER")
        assert kafka["binding"]["attributes"]["topics"] == ["${topic}"], kafka
        scheduled = next(row for row in rows if row["kind"] == "SCHEDULED_JOB")
        assert scheduled["binding"]["attributes"]["fixedDelayString"] == "${delay:1000}", scheduled
        assert scheduled["trigger"]["disabled"] is None, scheduled
        assert all("RUNTIME_EXPRESSION" in row["boundaries"] for row in [kafka, scheduled])


def main() -> int:
    started = time.monotonic()
    launcher = Path(os.environ.get("CLEW_SMOKE_LAUNCHER", str(ROOT / "clew"))).resolve(strict=True)
    jars = spring_jars()
    with tempfile.TemporaryDirectory(prefix="codeclew-spring-smoke-") as directory:
        root = Path(directory).resolve()
        env = dict(os.environ)
        state = root / "state"
        state.mkdir(mode=0o700)
        env["CODECLEW_HOME"] = str(state)
        def clew(*args: str) -> dict:
            result = json.loads(run([str(launcher), *args], ROOT, env))
            assert str(root) not in json.dumps(result), "public output leaked fixture path"
            return result
        sessions = []
        for language in ("kotlin", "java"):
            repository = fixture(root, language, jars, env)
            profile = "kotlin-2.4.10-gradle-single" if language == "kotlin" else "java-17plus-gradle-read-only"
            opened = clew("context", "open", "--repo", str(repository), "--target-ref", "main",
                          "--language", language, "--compilation", ":/main", "--profile", profile,
                          "--operation", "analysis", "--intent", "catalogue Spring computation roots",
                          "--term", "Entrypoints", "--max-roots", "4")
            assert opened["status"] == "OPEN", opened
            session = opened["session"]["sessionId"]
            sessions.append(session)
            catalogue = clew("entrypoints", "--session", session, "--limit", "100")
            verify(catalogue["entries"], {session})
            print(json.dumps({"stage": language, "roots": catalogue["total"]}), flush=True)
        combined_args = ["entrypoints", "--session", sessions[0], "--session", sessions[1]]
        combined = clew(*combined_args, "--limit", "100")
        verify(combined["entries"], set(sessions))
        paged = []
        cursor = None
        while True:
            page = clew(*combined_args, "--limit", "1", *(["--cursor", cursor] if cursor else []))
            assert page["catalogueDigest"] == combined["catalogueDigest"]
            assert page["offset"] == len(paged)
            assert len(page["entries"]) == 1
            paged.extend(page["entries"])
            cursor = page["nextCursor"]
            if cursor is None:
                break
            assert len(paged) < 6, "pagination cursor did not terminate"
        assert paged == combined["entries"]
        print(json.dumps({"schema": "codeclew-spring-entrypoints-smoke/1.0", "status": "PASS",
                          "repositories": 2, "roots": 6, "pages": 6,
                          "elapsedSeconds": round(time.monotonic() - started, 2)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
