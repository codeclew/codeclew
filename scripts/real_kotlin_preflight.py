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


SCHEMA = "codeclew.real-kotlin-preflight/0.4"
ROOT = Path(__file__).resolve().parents[1]
GRADLE_CACHE_MARKER_SCHEMA = "codeclew.real-kotlin-gradle-cache/0.1"
GRADLE_CACHE_MARKER = ".codeclew-real-kotlin-preflight-cache.json"
GRADLE_CACHE_MEMBERS = ("caches", "wrapper", "jdks")


class PreflightFailure(Exception):
    def __init__(self, stage: str, message: str) -> None:
        super().__init__(message)
        self.stage = stage


def canonical(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=True, separators=(",", ":"), sort_keys=True) + "\n").encode()


def atomic_bytes(path: Path, body: bytes) -> None:
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        temporary.write_bytes(body)
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def run(
    argv: list[str], repo: Path, deadline: float, *, environment: dict[str, str] | None = None
) -> tuple[subprocess.CompletedProcess[bytes], int]:
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
            env=environment,
        )
    except subprocess.TimeoutExpired as error:
        raise PreflightFailure("BUDGET", "real-project preparation exceeded its budget") from error
    return completed, round((time.monotonic() - started) * 1000)


def require_success(completed: subprocess.CompletedProcess[bytes], stage: str) -> None:
    if completed.returncode != 0:
        detail = (completed.stderr + b"\n" + completed.stdout).decode("utf-8", "replace")[-4096:].strip()
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


def gradle_cache_marker_matches(target: Path, source: Path, members: list[str]) -> bool:
    marker = target / GRADLE_CACHE_MARKER
    if marker.is_symlink() or not marker.is_file():
        return False
    try:
        value = json.loads(marker.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return False
    return value == {
        "schema": GRADLE_CACHE_MARKER_SCHEMA,
        "source": str(source),
        "members": members,
    } and all((target / member).is_dir() and not (target / member).is_symlink() for member in members)


def hydrate_gradle_cache(source: Path, target: Path, repo: Path, deadline: float) -> tuple[int, bool]:
    source = source.resolve(strict=True)
    if source.is_symlink() or not source.is_dir() or source == repo or source.is_relative_to(repo):
        raise PreflightFailure("GRADLE_CACHE", "Gradle cache seed must be a real external directory")
    members = [member for member in GRADLE_CACHE_MEMBERS if (source / member).is_dir()]
    if "caches" not in members or "wrapper" not in members:
        raise PreflightFailure("GRADLE_CACHE", "Gradle cache seed must contain caches and wrapper")
    if gradle_cache_marker_matches(target, source, members):
        return 0, True
    if target.is_symlink() or (target.exists() and not target.is_dir()):
        raise PreflightFailure("GRADLE_CACHE", "repo-local Gradle cache must be a real directory")
    target.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    for member in members:
        destination = target / member
        destination.mkdir(parents=True, exist_ok=True)
        if sys.platform == "darwin":
            argv = ["cp", "-cR", f"{source / member}/.", str(destination)]
        else:
            argv = ["cp", "-a", "--reflink=auto", f"{source / member}/.", str(destination)]
        copied, _ = run(argv, repo, deadline)
        require_success(copied, "GRADLE_CACHE")
    atomic_bytes(
        target / GRADLE_CACHE_MARKER,
        canonical({"schema": GRADLE_CACHE_MARKER_SCHEMA, "source": str(source), "members": members}),
    )
    return round((time.monotonic() - started) * 1000), False


def isolated_gradle_environment(gradle_user_home: Path) -> dict[str, str]:
    environment = os.environ.copy()
    for key in (
        "GRADLE_USER_HOME",
        "CODECLEW_K1_BUILD_STATE_ROOT",
        "CODECLEW_K2_INDEX_ROOT",
    ):
        environment.pop(key, None)
    environment["GRADLE_USER_HOME"] = str(gradle_user_home)
    return environment


def gradle_daemon_stop_argv(wrapper: Path, gradle_user_home: Path) -> list[str]:
    return [str(wrapper), "--gradle-user-home", str(gradle_user_home), "--stop"]


def exact_executable(candidate: str | None, stage: str) -> Path:
    if not candidate:
        raise PreflightFailure(stage, f"required executable is unavailable on PATH: {stage.lower()}")
    executable = Path(candidate)
    if executable.is_symlink():
        executable = executable.resolve(strict=True)
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise PreflightFailure(stage, "resolved executable is not a regular executable file")
    return executable.resolve(strict=True)


def validate_run_contract(
    run_phase: str, seed_entity: str | None, state_root: Path | None
) -> None:
    if seed_entity is not None and not seed_entity.strip():
        raise PreflightFailure("RUN_PHASE", "seed entity must be nonblank when provided")
    if run_phase == "warm" and seed_entity is None:
        raise PreflightFailure("RUN_PHASE", "warm projection requires --seed-entity")
    if run_phase == "warm" and state_root is None:
        raise PreflightFailure("RUN_PHASE", "warm projection requires --state-root")


def verify_private_state_root(
    value: Path | None, repo: Path, *, require_existing: bool
) -> str | None:
    if value is None:
        return None
    if not value.is_absolute() or value.is_symlink():
        raise PreflightFailure("STATE_ROOT", "state root must be absolute and nonsymlinked")
    if require_existing and not value.exists():
        raise PreflightFailure("STATE_ROOT", "warm state root must already exist")
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
    validate_run_contract(args.run_phase, args.seed_entity, args.state_root)
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
    state_root = verify_private_state_root(
        args.state_root, repo, require_existing=args.run_phase == "warm"
    )
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
        hydration_millis, hydration_hit = hydrate_gradle_cache(
            args.gradle_cache_seed, repo / ".gradle", repo, deadline
        )
        probes.append(
            {
                "kind": "GRADLE_CACHE_HYDRATION",
                "durationMillis": hydration_millis,
                "hit": hydration_hit,
            }
        )
        task = gradle_configuration_task(args.compilation)
        gradle_environment = isolated_gradle_environment(repo / ".gradle")
        daemon_stop, daemon_stop_millis = run(
            gradle_daemon_stop_argv(wrapper, repo / ".gradle"),
            repo,
            deadline,
            environment=gradle_environment,
        )
        require_success(daemon_stop, "GRADLE_DAEMON_RESET")
        probes.append({"kind": "GRADLE_DAEMON_RESET", "durationMillis": daemon_stop_millis})
        model_probe, model_millis = run(
            [str(wrapper), "--offline", "--no-daemon", "--quiet", task],
            repo,
            deadline,
            environment=gradle_environment,
        )
        require_success(model_probe, "GRADLE")
        build = {"system": "GRADLE", "launcher": "./gradlew", "compilation": args.compilation, "configurationTask": task}
        probes.append({"kind": "GRADLE_OFFLINE_ROUTE", "durationMillis": model_millis})
    clew = exact_executable(str(args.clew), "CLEW")
    inspect_argv = [str(clew)]
    if state_root is not None:
        compiler_index = Path(state_root) / "compiler-index"
        compiler_index.mkdir(mode=0o700, parents=True, exist_ok=True)
        inspect_argv.extend(["--compiler-index-root", str(compiler_index)])
    inspect_argv.extend(
        ["project", "inspect", "--repo", str(repo), "--compilation", args.compilation]
    )
    inspect_probe, inspect_millis = run(inspect_argv, repo, deadline)
    require_success(inspect_probe, "SEMANTIC_BUILD_DISCOVERY")
    probes.append({"kind": "CLEW_PROJECT_INSPECT", "durationMillis": inspect_millis})
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
        "run": {
            "phase": args.run_phase.upper(),
            "seedEntity": args.seed_entity,
            "requiresVerifiedCacheHit": args.run_phase == "warm",
        },
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
    parser.add_argument("--run-phase", choices=("cold", "warm"), default="cold")
    parser.add_argument("--seed-entity")
    parser.add_argument("--clew", type=Path, default=ROOT / "target/release/clew")
    parser.add_argument("--gradle-cache-seed", type=Path, default=Path.home() / ".gradle")
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
