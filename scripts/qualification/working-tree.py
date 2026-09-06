#!/usr/bin/env python3
"""Qualify immutable saved Kotlin inputs through the public source launcher."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]


def git(repo: Path, *args: str) -> bytes:
    return subprocess.check_output(["git", *args], cwd=repo, stderr=subprocess.PIPE)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    transcript: list[dict] = []
    sessions: list[str] = []

    def run(*command: str) -> dict:
        start = time.monotonic()
        result = subprocess.run([str(ROOT / "clew"), *command], cwd=ROOT,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            value = json.loads(result.stdout)
        except json.JSONDecodeError:
            value = {"stdout": result.stdout, "stderr": result.stderr}
        transcript.append({"command": list(command), "seconds": round(time.monotonic() - start, 3),
                           "exitCode": result.returncode, "result": value})
        (args.output / "transcript.json").write_text(json.dumps(transcript, indent=2) + "\n")
        if result.returncode:
            raise RuntimeError(json.dumps(value))
        return value

    with tempfile.TemporaryDirectory(prefix="codeclew-working-tree-") as directory:
        repo = Path(directory) / "repo"
        shutil.copytree(ROOT / "fixtures/kotlin-basic", repo,
                        ignore=shutil.ignore_patterns(".git", ".gradle", "build", ".semantic-thread"))
        shutil.rmtree(repo / "src")
        source = repo / "src/main/kotlin/Price.kt"
        source.parent.mkdir(parents=True)
        source.write_text("package fixture\nfun basePrice(): Int = 100\n")
        git(repo, "init", "-q", "-b", "main")
        git(repo, "add", ".")
        git(repo, "-c", "user.name=Codeclew Qualification", "-c", "user.email=test@codeclew.invalid",
            "commit", "-qm", "base")
        source.write_text("package fixture\nfun stagedPrice(): Int = 200\n")
        git(repo, "add", ".")
        source.write_text("package fixture\nfun savedPrice(): Int = 300\n")
        (source.parent / "Extra.kt").write_text("package fixture\nfun extraPrice(): Int = 400\n")
        (repo / "build").mkdir()
        (repo / "build/ignored.kt").write_text("fun ignoredGenerated() = 0\n")
        index_before = (repo / ".git/index").read_bytes()
        refs_before = git(repo, "show-ref")
        head_before = git(repo, "rev-parse", "HEAD")
        saved_bytes = source.read_bytes()
        try:
            discovery = run("doctor", "repository", "--repo", str(repo), "--working-tree")
            contours = [c for c in discovery["contours"]
                        if c["profileId"] == "kotlin-jvm-gradle-analysis"
                        and c["status"] == "READY_FOR_TASK_DOCTOR"]
            assert contours, discovery
            contour = contours[0]
            compilations = contour["compilations"]
            assert ":/main" in compilations, compilations
            opened = run("nav", "query", "--repo", str(repo), "--target-ref", "main",
                         "--language", "kotlin", "--profile", contour["profileId"],
                         "--compilation", ":/main", "--working-tree", "--term", "savedPrice",
                         "--decision-identifier", "savedPrice", "--source")
            session = opened.get("session", {}).get("sessionId") or opened["sessionId"]
            sessions.append(session)
            context = opened["navigation"]["contextId"]
            encoded = json.dumps(opened)
            assert "savedPrice" in encoded and "300" in encoded, opened
            assert source.read_bytes() == saved_bytes
            assert (repo / ".git/index").read_bytes() == index_before
            fresh = run("change", "check-freshness", "--session", session)
            assert fresh["status"] == "FRESH" and fresh["retainedEvidenceValid"] is True, fresh
            source.write_text("package fixture\nfun laterPrice(): Int = 500\n")
            fresh = run("change", "check-freshness", "--session", session)
            assert fresh["status"] == "LIVE_CHANGED" and fresh["retainedEvidenceValid"] is True, fresh
            expanded = run("context", "expand", "--session", session, "--from", context,
                           "--term", "extraPrice", "--term", "savedPrice")
            encoded = json.dumps(expanded)
            assert "extraPrice" in encoded and "savedPrice" in encoded, expanded
            assert "laterPrice" not in encoded, expanded
            assert (repo / ".git/index").read_bytes() == index_before
            assert git(repo, "show-ref") == refs_before and git(repo, "rev-parse", "HEAD") == head_before
        finally:
            for session in sessions:
                run("session", "close", "--session", session)
                run("session", "gc", "--session", session)
            assert (repo / ".git/index").read_bytes() == index_before
            assert git(repo, "show-ref") == refs_before
        summary = {"schema": "codeclew-working-tree-qualification/1.0", "status": "PASS",
                   "language": "KOTLIN", "compilations": [":/main"],
                   "checks": ["saved-not-staged", "untracked-source", "immutable-after-edit",
                              "live-freshness", "index-preserved", "refs-preserved", "session-gc"],
                   "baseRevision": head_before.decode().strip(),
                   "indexDigest": hashlib.sha256(index_before).hexdigest(),
                   "commandSeconds": [row["seconds"] for row in transcript]}
        (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps(summary))


if __name__ == "__main__":
    main()
