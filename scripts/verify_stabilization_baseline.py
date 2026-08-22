#!/usr/bin/env python3
"""Verify the recoverable, privacy-scrubbed S0 baseline without exposing paths."""

from __future__ import annotations

from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent


def git(*arguments: str, check: bool = True) -> str:
    completed = subprocess.run(
        ("git", *arguments),
        cwd=ROOT,
        check=check,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    return completed.stdout.strip()


def sha256(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(block)
    return hasher.hexdigest()


def main() -> int:
    raw = os.environ.get("CODECLEW_RECOVERY_MANIFEST")
    if not raw:
        raise SystemExit("CODECLEW_RECOVERY_MANIFEST is required")
    manifest_path = Path(raw)
    if not manifest_path.is_absolute() or ".." in manifest_path.parts:
        raise SystemExit("recovery manifest path is unsafe")
    metadata = manifest_path.stat()
    if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o600:
        raise SystemExit("recovery manifest must be private")
    value = json.loads(manifest_path.read_text(encoding="utf-8"))
    if set(value) != {
        "bundle",
        "bundleSha256",
        "expiresAfter",
        "originalHead",
        "rewrittenHead",
        "schema",
        "tree",
    } or value["schema"] != "codeclew-history-recovery/1.0":
        raise SystemExit("recovery manifest schema is invalid")
    if datetime.fromisoformat(value["expiresAfter"]) <= datetime.now().astimezone():
        raise SystemExit("recovery authority expired")
    bundle = manifest_path.parent / value["bundle"]
    bundle_metadata = bundle.stat()
    if bundle_metadata.st_uid != os.geteuid() or stat.S_IMODE(bundle_metadata.st_mode) != 0o600:
        raise SystemExit("recovery bundle must be private")
    if sha256(bundle) != value["bundleSha256"]:
        raise SystemExit("recovery bundle digest mismatch")
    subprocess.run(
        ("git", "bundle", "verify", str(bundle)),
        cwd=ROOT,
        check=True,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    ancestry = subprocess.run(
        ("git", "merge-base", "--is-ancestor", value["rewrittenHead"], "HEAD"),
        cwd=ROOT,
        check=False,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if ancestry.returncode != 0:
        raise SystemExit("rewritten baseline is not an ancestor of the current source")
    if git("rev-parse", value["rewrittenHead"] + "^{tree}") != value["tree"]:
        raise SystemExit("rewritten tree differs from the recovery mapping")
    old = subprocess.run(
        ("git", "cat-file", "-e", value["originalHead"] + "^{commit}"),
        cwd=ROOT,
        check=False,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if old.returncode == 0:
        raise SystemExit("original commit remains reachable in the working repository")
    refs = git("for-each-ref", "--format=%(refname)", "refs/heads", "refs/remotes", "refs/tags", "refs/original").splitlines()
    if refs != ["refs/heads/main"]:
        raise SystemExit("working repository has unexpected refs after the rewrite")
    worktrees = [line for line in git("worktree", "list", "--porcelain").splitlines() if line.startswith("worktree ")]
    if len(worktrees) != 1:
        raise SystemExit("working repository has stale linked worktrees")
    remote = git("remote", "get-url", "origin")
    if remote.startswith("/") or not remote.endswith("/codeclew.git"):
        raise SystemExit("origin is not the canonical non-local repository")
    print(json.dumps({"schema": "codeclew-stabilization-baseline/1.0", "status": "PASS"}, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    sys.exit(main())
