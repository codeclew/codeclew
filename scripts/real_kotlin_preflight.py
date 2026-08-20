#!/usr/bin/env python3
"""Fail-fast launch preflight for a real Kotlin repository."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import time


SCHEMA = "codeclew.real-kotlin-preflight/0.1"


class PreflightFailure(Exception):
    def __init__(self, stage: str, message: str) -> None:
        super().__init__(message)
        self.stage = stage


def canonical(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=True, separators=(",", ":"), sort_keys=True) + "\n").encode()


def run(argv: list[str], repo: Path, deadline: float) -> tuple[subprocess.CompletedProcess[bytes], int]:
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise PreflightFailure("BUDGET", "real-project preparation exceeded its budget")
    started = time.monotonic()
    try:
        completed = subprocess.run(
            argv,
            cwd=repo,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=remaining,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise PreflightFailure("BUDGET", "real-project preparation exceeded its budget") from error
    return completed, round((time.monotonic() - started) * 1000)


def require_success(completed: subprocess.CompletedProcess[bytes], stage: str) -> None:
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", "replace")[-1024:].strip()
        raise PreflightFailure(stage, detail or f"command exited {completed.returncode}")


def git_stdout(repo: Path, *arguments: str) -> str:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=repo,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    require_success(completed, "GIT")
    return completed.stdout.decode("utf-8", "strict").strip()


def gradle_configuration_task(compilation: str) -> str:
    if not compilation.startswith(":") or not compilation.endswith("/main"):
        raise PreflightFailure("COMPILATION", "Gradle compilation must be canonical :<project>/main")
    project = compilation[:-5]
    return f"{project}:properties" if project != ":" else ":properties"


def exact_executable(candidate: str | None, stage: str) -> Path:
    if not candidate:
        raise PreflightFailure(stage, f"required executable is unavailable on PATH: {stage.lower()}")
    executable = Path(candidate)
    if executable.is_symlink():
        executable = executable.resolve(strict=True)
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise PreflightFailure(stage, "resolved executable is not a regular executable file")
    return executable.resolve(strict=True)


def verify_private_state_root(value: Path | None, repo: Path) -> str | None:
    if value is None:
        return None
    if not value.is_absolute() or value.is_symlink():
        raise PreflightFailure("STATE_ROOT", "state root must be absolute and nonsymlinked")
    value.mkdir(mode=0o700, parents=True, exist_ok=True)
    canonical_root = value.resolve(strict=True)
    mode = stat.S_IMODE(canonical_root.stat().st_mode)
    if mode & 0o077 or canonical_root == repo or canonical_root.is_relative_to(repo):
        raise PreflightFailure("STATE_ROOT", "state root must be private and external to the repository")
    return str(canonical_root)


def execute(args: argparse.Namespace) -> dict[str, object]:
    started = time.monotonic()
    budget = min(60.0, args.budget_seconds)
    if budget <= 0:
        raise PreflightFailure("ARGUMENTS", "budget must be positive")
    deadline = started + budget
    repo = args.repo.resolve(strict=True)
    if repo.is_symlink() or not repo.is_dir():
        raise PreflightFailure("REPOSITORY", "repository must be a real directory")
    if Path(git_stdout(repo, "rev-parse", "--show-toplevel")).resolve() != repo:
        raise PreflightFailure("REPOSITORY", "repository must be the exact Git root")
    tracked = git_stdout(repo, "status", "--porcelain=v1", "--untracked-files=no")
    if tracked:
        raise PreflightFailure("GIT", "tracked worktree is dirty")
    revision = git_stdout(repo, "rev-parse", "HEAD")
    java = exact_executable(shutil.which("java"), "JAVA")
    java_probe, java_millis = run([str(java), "-version"], repo, deadline)
    require_success(java_probe, "JAVA")
    java_version = (java_probe.stderr + java_probe.stdout).decode("utf-8", "replace")
    if 'version "21.' not in java_version:
        raise PreflightFailure("JAVA", "JDK 21 is required")
    probes: list[dict[str, object]] = [{"kind": "JAVA_21", "durationMillis": java_millis}]
    if (repo / "pom.xml").is_file():
        maven = exact_executable(shutil.which("mvn"), "MAVEN")
        model_probe, model_millis = run([str(maven), "--offline", "--version"], repo, deadline)
        require_success(model_probe, "MAVEN")
        build = {"system": "MAVEN", "launcher": str(maven), "compilation": args.compilation}
        probes.append({"kind": "MAVEN_OFFLINE", "durationMillis": model_millis})
    else:
        wrapper = repo / "gradlew"
        if wrapper.is_symlink() or not wrapper.is_file() or not os.access(wrapper, os.X_OK):
            raise PreflightFailure("GRADLE", "trusted executable Gradle wrapper is unavailable")
        task = gradle_configuration_task(args.compilation)
        model_probe, model_millis = run(
            [str(wrapper), "--offline", "--no-daemon", "--quiet", task], repo, deadline
        )
        require_success(model_probe, "GRADLE")
        build = {"system": "GRADLE", "launcher": "./gradlew", "compilation": args.compilation, "configurationTask": task}
        probes.append({"kind": "GRADLE_OFFLINE_ROUTE", "durationMillis": model_millis})
    state_root = verify_private_state_root(args.state_root, repo)
    elapsed = round((time.monotonic() - started) * 1000)
    if elapsed > round(budget * 1000):
        raise PreflightFailure("BUDGET", "real-project preparation exceeded its budget")
    return {
        "schema": SCHEMA,
        "status": "READY",
        "repository": str(repo),
        "revision": revision,
        "trackedClean": True,
        "stateRoot": state_root,
        "build": build,
        "probes": probes,
        "elapsedMillis": elapsed,
        "budgetMillis": round(budget * 1000),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--compilation", default=":/main")
    parser.add_argument("--state-root", type=Path)
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--budget-seconds", type=float, default=60.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    started = time.monotonic()
    try:
        receipt = execute(args)
        exit_code = 0
    except (PreflightFailure, OSError) as error:
        receipt = {
            "schema": SCHEMA,
            "status": "FAILED",
            "stage": error.stage if isinstance(error, PreflightFailure) else "IO",
            "message": str(error),
            "elapsedMillis": round((time.monotonic() - started) * 1000),
        }
        exit_code = 1
    body = canonical(receipt)
    if args.receipt is not None:
        args.receipt.parent.mkdir(parents=True, exist_ok=True)
        args.receipt.write_bytes(body)
    sys.stdout.buffer.write(body)
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
