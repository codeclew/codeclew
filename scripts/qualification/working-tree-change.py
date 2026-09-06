#!/usr/bin/env python3
"""Qualify retained HEAD-to-saved Kotlin change evidence through the public CLI."""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    transcript = []
    reports = []

    def run(*command):
        started = time.monotonic()
        process = subprocess.run([str(ROOT / "clew"), *command], cwd=ROOT, capture_output=True, text=True)
        try:
            value = json.loads(process.stdout)
        except json.JSONDecodeError:
            value = {"stdout": process.stdout, "stderr": process.stderr}
        transcript.append({"command": list(command), "seconds": round(time.monotonic() - started, 3),
                           "exitCode": process.returncode, "result": value})
        (args.output / "transcript.json").write_text(json.dumps(transcript, indent=2) + "\n")
        assert process.returncode == 0, value
        return value

    with tempfile.TemporaryDirectory(prefix="codeclew-change-") as directory:
        repo = Path(directory) / "repo"
        shutil.copytree(ROOT / "fixtures/kotlin-basic", repo,
                        ignore=shutil.ignore_patterns(".git", ".gradle", "build", ".semantic-thread"))
        shutil.rmtree(repo / "src")
        source = repo / "src/main/kotlin/Price.kt"
        source.parent.mkdir(parents=True)
        base = """package fixture
object Pricing {
    fun price(): Int = 1
    fun label(x: Int): String = x.toString()
}
fun consumer(): Int = Pricing.price() + 1
fun labelConsumer(): String = Pricing.label(1)
fun stopCalling(): Int = Pricing.price()
fun main() { Pricing.price() }
fun stable(): Int = 9
fun documented(): Int { /* old comment */ return 7 }
"""
        source.write_text(base)
        test = repo / "src/test/kotlin/PriceTest.kt"
        test.parent.mkdir(parents=True)
        test.write_text("package fixture\nimport kotlin.test.Test\nimport kotlin.test.assertEquals\nclass PriceTest { @Test fun priceCheck() { assertEquals(1, Pricing.price()) } }\n")
        deleted = source.parent / "Deleted.kt"
        deleted.write_text("package fixture\nfun removed(): Int = 4\n")
        renamed = source.parent / "Original.kt"
        renamed.write_text("package fixture\nfun moved(): Int = 6\n")
        def git(*command):
            return subprocess.check_output(["git", *command], cwd=repo, stderr=subprocess.PIPE)
        git("init", "-q", "-b", "main")
        git("add", ".")
        git("-c", "user.name=Qualification", "-c", "user.email=test@codeclew.invalid", "commit", "-qm", "base")
        source.write_text(base.replace("= 1\n", "= 2\n"))
        git("add", ".")
        saved = base.replace("= 1\n", "= 3\n").replace("x: Int", "x: Long").replace("label(1)", "label(1L)").replace("old comment", "new comment").replace("stopCalling(): Int = Pricing.price()", "stopCalling(): Int = 0")
        source.write_text(saved)
        deleted.unlink()
        renamed.rename(source.parent / "Renamed.kt")
        (source.parent / "Added.kt").write_text("package fixture\nfun added(): Int = 5\n")
        index = (repo / ".git/index").read_bytes()
        refs = git("show-ref")
        command = ("change", "inspect", "--repo", str(repo), "--target-ref", "main", "--language", "kotlin",
                   "--profile", "kotlin-jvm-gradle-analysis", "--compilation", ":/main", "--working-tree")
        try:
            result = run(*command)
            reports.append(result["comparisonId"])
            assert result["beforeAnalysis"]["status"] == "AVAILABLE", result
            assert result["afterAnalysis"]["status"] == "AVAILABLE", result
            changes = result["declarations"]
            def named(name):
                return next(row for row in changes if name in json.dumps(row.get("after") or row.get("before")))
            price = named("fixture/Pricing.price")
            assert price["sourceTextChanged"] and not price["changedShapeFields"], price
            assert "3" in json.dumps(result["files"]), result
            label = named("fixture/Pricing.label#")
            assert label["changedShapeFields"], label
            comment = named("fixture/documented")
            assert comment["sourceTextChanged"] and not comment["changedShapeFields"], comment
            assert comment["behavioralEquivalence"] == "NOT_PROVEN_BY_THIS_COMPARISON"
            assert "REMOVED" in named("fixture/removed")["changes"]
            assert "ADDED" in named("fixture/added")["changes"]
            kinds = {row["file"].split("/")[-1]: row["kind"] for row in result["files"]}
            assert kinds["Original.kt"] == "DELETED" and kinds["Renamed.kt"] == "ADDED", kinds
            assert result["testsExecuted"] is False
            assert all(row["status"] == "COLLECTED" for row in result["cleanup"]), result
            graph = run("change", "graph", "--comparison", result["comparisonId"])
            assert graph["testScope"] == "TEST_COMPILATION_NOT_ANALYZED", graph
            assert graph["edges"] and graph["candidates"], graph
            assert any(edge["presence"] == "BEFORE_ONLY" for edge in graph["edges"]), graph
            assert any(node["entrypoints"] for node in graph["nodes"]), graph
            assert all(c["authority"] == "STATIC_DERIVED_AFFECTED_CANDIDATE" for c in graph["candidates"]), graph
            assert any("consumer" in json.dumps(node) for node in graph["nodes"]), graph
            with_tests = run(*command, "--compilation", ":/test")
            reports.append(with_tests["comparisonId"])
            test_graph = run("change", "graph", "--comparison", with_tests["comparisonId"])
            assert test_graph["testScope"] == "EXPLICIT_GRADLE_TEST_COMPILATION_SELECTED_RELATION_EVIDENCE_ONLY", test_graph
            assert any(c["selectedTestCompilation"] and c["source"]["file"].endswith("PriceTest.kt") for c in test_graph["candidates"]), test_graph
            assert test_graph["testsExecuted"] is False
            rendered = args.output / "report.html"
            run("change", "render", "--comparison", result["comparisonId"], "--output", str(rendered.resolve()))
            initial_html = rendered.read_bytes()
            assert run("change", "check-freshness", "--comparison", result["comparisonId"])["liveStatus"] == "FRESH"
            retained = run("change", "show", "--comparison", result["comparisonId"])
            source.write_text(saved + "\nfun broken( = missing\n")
            assert run("change", "show", "--comparison", result["comparisonId"]) == retained
            freshness = run("change", "check-freshness", "--comparison", result["comparisonId"])
            assert freshness["liveStatus"] == "LIVE_CHANGED" and freshness["retainedEvidenceValid"]
            repeated = args.output / "report-repeated.html"
            run("change", "render", "--comparison", result["comparisonId"], "--output", str(repeated.resolve()))
            assert repeated.read_bytes() == initial_html
            exact = run("change", "source", "--comparison", result["comparisonId"], "--file", "src/main/kotlin/Price.kt", "--side", "after")
            assert exact["text"] == saved and exact["nextOffset"] is None
            broken = run(*command)
            reports.append(broken["comparisonId"])
            assert broken["status"] == "INCOMPLETE" or "DECLARATION_COVERAGE_IS_PARTIAL" in broken["obligations"], broken
            assert broken["files"] and broken["beforeAnalysis"]["status"] == "AVAILABLE", broken
            source.write_text(saved)
            build = repo / "build.gradle.kts"
            build.write_text(build.read_text() + "\n// changed build input\n")
            changed_build = run(*command)
            reports.append(changed_build["comparisonId"])
            assert "BUILD_INPUTS_CHANGED" in changed_build["comparability"], changed_build
            assert (repo / ".git/index").read_bytes() == index
            assert git("show-ref") == refs
        finally:
            for comparison in reports:
                run("change", "forget", "--comparison", comparison)
            assert (repo / ".git/index").read_bytes() == index
            assert git("show-ref") == refs
    summary = {"schema": "codeclew-working-tree-change-qualification/1.0", "status": "PASS",
               "checks": ["saved-not-staged", "body-change", "signature-change", "comment-only-no-behavior-claim",
                          "added-deleted-renamed", "retained-after-edit", "broken-after-preserves-text",
                          "changed-build-model-binding", "index-and-refs-preserved", "session-cleanup",
                          "direct-consumers", "removed-before-call", "jvm-main-evidence", "selected-test-relations-without-test-run", "retained-offline-render", "live-freshness-separate-from-validity", "exact-source-after-edit"],
               "commandSeconds": [row["seconds"] for row in transcript]}
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary))


if __name__ == "__main__":
    main()
