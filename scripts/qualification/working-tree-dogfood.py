#!/usr/bin/env python3
"""Exercise saved-edit comparison on Codeclew's real Kotlin and Rust sources."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
# Qualification guardrails from macOS development observations (426s Kotlin,
# 75s Rust including a runtime build, <=14s retained reads); not release SLOs.
INSPECT_SECONDS = {"kotlin": 900, "rust": 180}
RETAINED_READ_SECONDS = 60


def git(repo, *args):
    return subprocess.check_output(["git", *args], cwd=repo, stderr=subprocess.PIPE).decode().strip()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--language", choices=["kotlin", "rust", "all"], default="all")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    transcript, rows = [], []
    base = git(ROOT, "rev-parse", "HEAD")

    def run(*command):
        started = time.monotonic()
        process = subprocess.run([str(ROOT / "clew"), *command], cwd=ROOT, capture_output=True, text=True)
        try:
            value = json.loads(process.stdout)
        except json.JSONDecodeError:
            value = {"stdout": process.stdout, "stderr": process.stderr}
        transcript.append({"command": list(command), "seconds": round(time.monotonic()-started, 3),
                           "exitCode": process.returncode, "result": value})
        (args.output / "transcript.json").write_text(json.dumps(transcript, indent=2)+"\n")
        assert process.returncode == 0, value
        budget = INSPECT_SECONDS[language] if command[:2] == ("change", "inspect") else RETAINED_READ_SECONDS
        assert transcript[-1]["seconds"] <= budget, {"budgetSeconds": budget, "command": command[:2], "seconds": transcript[-1]["seconds"]}
        return value

    languages = ["kotlin", "rust"] if args.language == "all" else [args.language]
    for language in languages:
        branch = "qualification/working-tree-" + uuid.uuid4().hex
        report_id = None
        with tempfile.TemporaryDirectory(prefix="codeclew-dogfood-") as directory:
            repo = Path(directory) / "repo"
            git(ROOT, "worktree", "add", "-b", branch, str(repo), base)
            try:
                if language == "kotlin":
                    file = "workers/kotlin/src/main/kotlin/dev/semanticthread/worker/Worker.kt"
                    old, new = 'error("unsupported request kind $kind")', 'error("unsupported worker request kind $kind")'
                    profile, compilation, symbol = "kotlin-jvm-gradle-analysis", ":workers:kotlin/main", "Worker.handle"
                else:
                    file = "crates/clew/src/main.rs"
                    old = "change inspect requires --working-tree, --base HEAD and non-cacheable model authority"
                    new = "saved-change inspection requires --working-tree, --base HEAD and non-cacheable model authority"
                    profile, compilation, symbol = "rust-syntax", "cargo:crates/clew/Cargo.toml#clew#bin#clew", "change_inspect"
                source = repo / file
                text = source.read_text()
                assert text.count(old) == 1
                saved = text.replace(old, new)
                source.write_text(saved)
                index_path = Path(git(repo, "rev-parse", "--git-path", "index"))
                index = index_path.read_bytes()
                refs = git(repo, "show-ref")
                result = run("change", "inspect", "--repo", str(repo.resolve()), "--target-ref", branch,
                             "--language", language, "--profile", profile, "--compilation", compilation, "--working-tree")
                report_id = result["comparisonId"]
                assert result["beforeAnalysis"]["status"] == "AVAILABLE", result
                assert result["afterAnalysis"]["status"] == "AVAILABLE", result
                assert any(symbol in json.dumps(d) for d in result["declarations"]), result
                assert all(c["status"] == "COLLECTED" for c in result["cleanup"])
                graph = run("change", "graph", "--comparison", report_id)
                if language == "kotlin":
                    assert any("Main.kt" in json.dumps(c) for c in graph["candidates"]), graph
                else:
                    assert result["afterAnalysis"]["authority"] == "RUST_SYNTAX_ONLY"
                    assert not graph["edges"], graph
                output = args.output / (language + ".html")
                rendered = run("change", "render", "--comparison", report_id, "--output", str(output.resolve()))
                original = output.read_bytes()
                source.write_text(saved + "\n// later saved edit after evidence capture\n")
                fresh = run("change", "check-freshness", "--comparison", report_id)
                assert fresh["liveStatus"] == "LIVE_CHANGED" and fresh["retainedEvidenceValid"]
                repeated = args.output / (language + "-repeated.html")
                run("change", "render", "--comparison", report_id, "--output", str(repeated.resolve()))
                assert repeated.read_bytes() == original
                assert index_path.read_bytes() == index and git(repo, "show-ref") == refs
                rows.append({"language": language, "status": "PASS", "baseRevision": base,
                             "compilation": compilation, "source": file, "changedSymbol": symbol,
                             "counts": result["counts"], "authority": result["afterAnalysis"]["authority"],
                             "coverage": result["coverage"], "graphStatus": graph["status"],
                             "returnedEdges": len(graph["edges"]), "returnedCandidates": len(graph["candidates"]),
                             "unresolvedRelations": graph["unresolvedRelations"],
                             "retainedReportBytes": result["report"]["size"], "htmlBytes": rendered["bytes"],
                             "htmlSha256": hashlib.sha256(original).hexdigest(), "testsExecuted": False,
                             "indexAndRefsPreserved": True, "temporarySessionsCollected": True})
            finally:
                try:
                    if report_id:
                        run("change", "forget", "--comparison", report_id)
                finally:
                    git(ROOT, "worktree", "remove", "--force", str(repo))
                    git(ROOT, "branch", "-D", branch)
    summary = {"schema": "codeclew-working-tree-dogfood/1.0", "status": "PASS", "rows": rows,
               "qualificationBudgetsSeconds": {"inspect": INSPECT_SECONDS, "retainedRead": RETAINED_READ_SECONDS},
               "commandSeconds": [{"command": " ".join(r["command"][:2]), "seconds": r["seconds"]} for r in transcript]}
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2)+"\n")
    print(json.dumps(summary))


if __name__ == "__main__":
    main()
