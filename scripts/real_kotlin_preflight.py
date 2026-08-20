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


SCHEMA = "codeclew.real-kotlin-preflight/0.5"
ROOT = Path(__file__).resolve().parents[1]
GRADLE_CACHE_MARKER_SCHEMA = "codeclew.real-kotlin-gradle-cache/0.1"
GRADLE_CACHE_MARKER = ".codeclew-real-kotlin-preflight-cache.json"
GRADLE_CACHE_MEMBERS = ("caches", "wrapper", "jdks")
PERSISTENT_COMPILER_INDEX_STATUSES = {
    "COLD_FULL",
    "INCREMENTAL",
    "RECOVERED_FULL",
    "UNCHANGED_HIT",
}
USABLE_PROJECT_MODEL_CACHE_STATUSES = {
    "EXTRACTED_PUBLISHED",
    "PERSISTENT_HIT",
    "MEMORY_HIT",
}
PERSISTENT_COMPILER_INDEX_VERSIONS = {"2.1.21"}


class PreflightFailure(Exception):
    def __init__(self, stage: str, message: str) -> None:
        super().__init__(message)
        self.stage = stage


def canonical(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=True, separators=(",", ":"), sort_keys=True) + "\n").encode()


def last_json_object(raw: bytes) -> dict[str, object]:
    text = raw.decode("utf-8", "strict")
    decoder = json.JSONDecoder()
    cursor = 0
    last: dict[str, object] | None = None
    while cursor < len(text):
        if cursor == 0 and text.startswith("{"):
            candidate = 0
        else:
            newline = text.find("\n{", cursor)
            if newline < 0:
                break
            candidate = newline + 1
        try:
            value, end = decoder.raw_decode(text, candidate)
        except json.JSONDecodeError:
            next_line = text.find("\n", candidate)
            cursor = len(text) if next_line < 0 else next_line + 1
            continue
        if isinstance(value, dict):
            last = value
        cursor = max(end, candidate + 1)
    if last is not None:
        return last
    raise PreflightFailure("SEMANTIC_INDEX", "clew index returned no complete JSON object")


def semantic_index_summary(raw: bytes) -> dict[str, object]:
    value = last_json_object(raw)
    if value.get("schema") != "semantic-index-result/0.1":
        raise PreflightFailure("SEMANTIC_INDEX", "clew index result schema is invalid")
    for field in (
        "declarationDescriptorHash",
        "declarationRelationHash",
        "persistentIndexHash",
        "workerIndexHash",
    ):
        digest = value.get(field)
        if not isinstance(digest, str) or len(digest) != 71 or not digest.startswith("sha256:"):
            raise PreflightFailure("SEMANTIC_INDEX", f"clew index result has invalid {field}")
    compiler_index = value.get("compilerIndex")
    if compiler_index is None:
        status = "UNAVAILABLE_FOR_TOOLCHAIN"
        valid = None
        fallback_used = None
    elif isinstance(compiler_index, dict):
        status = compiler_index.get("status")
        valid = compiler_index.get("valid")
        fallback_used = compiler_index.get("fallbackUsed")
        if not isinstance(status, str) or not isinstance(valid, bool) or not isinstance(fallback_used, bool):
            raise PreflightFailure("SEMANTIC_INDEX", "clew index compiler profile is malformed")
    else:
        raise PreflightFailure("SEMANTIC_INDEX", "clew index compiler profile is malformed")
    project_model_cache = value.get("projectModelCache")
    if not isinstance(project_model_cache, dict) or not isinstance(
        project_model_cache.get("status"), str
    ):
        raise PreflightFailure("SEMANTIC_INDEX", "clew index project-model cache profile is missing")
    return {
        "compilerIndexStatus": status,
        "compilerIndexValid": valid,
        "fallbackUsed": fallback_used,
        "projectModelCacheStatus": project_model_cache["status"],
    }


def project_compiler_version(raw: bytes) -> str:
    value = last_json_object(raw)
    version = value.get("compilerVersion")
    if value.get("schema") != "semantic-project/0.1" or not isinstance(version, str) or not version:
        raise PreflightFailure("SEMANTIC_BUILD_DISCOVERY", "project compiler version is unavailable")
    return version


def supports_persistent_compiler_index(compiler_version: str) -> bool:
    return compiler_version in PERSISTENT_COMPILER_INDEX_VERSIONS


def canonical_cached_seed(requested: str | None) -> str | None:
    if requested is None:
        return None
    if requested.startswith("callable:") and "#jvm:" not in requested:
        raise PreflightFailure(
            "SEED_ENTITY",
            "warm toolchain without a persistent compiler backend requires the canonical seed from the cold receipt",
        )
    return requested


def canonical_seed_entity(raw: bytes, requested: str | None) -> str | None:
    if requested is None:
        return None
    value = last_json_object(raw)
    descriptor_graph = value.get("declarationDescriptors")
    descriptors = (
        descriptor_graph.get("descriptors")
        if isinstance(descriptor_graph, dict)
        else None
    )
    if not isinstance(descriptors, list):
        raise PreflightFailure(
            "SEED_ENTITY", "semantic index has no declaration descriptor set"
        )
    matches: set[str] = set()
    callable_key = requested.removeprefix("callable:")
    for descriptor in descriptors:
        if not isinstance(descriptor, dict) or descriptor.get("resolution") != "PROVEN":
            continue
        symbol = descriptor.get("symbolIdentity")
        if not isinstance(symbol, str) or not symbol:
            continue
        compiler_callable = descriptor.get("compilerCallableId")
        if (
            symbol == requested
            or symbol.partition("#jvm:")[0] == requested
            or compiler_callable == requested
            or (
                requested.startswith("callable:")
                and compiler_callable == callable_key
            )
        ):
            matches.add(symbol)
    if len(matches) != 1:
        raise PreflightFailure(
            "SEED_ENTITY",
            "requested seed does not resolve to exactly one proven canonical entity",
        )
    return next(iter(matches))


def require_persistent_reuse(summary: dict[str, object], run_phase: str) -> None:
    status = summary["compilerIndexStatus"]
    if (
        status not in PERSISTENT_COMPILER_INDEX_STATUSES
        or summary["compilerIndexValid"] is not True
        or summary["fallbackUsed"] is not False
    ):
        raise PreflightFailure(
            "COMPILER_INDEX",
            "persistent compiler index is invalid, unavailable, or used the legacy fallback",
        )
    if run_phase == "warm" and status != "UNCHANGED_HIT":
        raise PreflightFailure("COMPILER_INDEX", "warm run did not reuse an unchanged compiler generation")
    project_status = summary["projectModelCacheStatus"]
    if project_status not in USABLE_PROJECT_MODEL_CACHE_STATUSES:
        raise PreflightFailure("PROJECT_MODEL_CACHE", "project model was not published for persistent reuse")
    if run_phase == "warm" and project_status != "PERSISTENT_HIT":
        raise PreflightFailure("PROJECT_MODEL_CACHE", "warm run did not load the persistent project model")


def atomic_bytes(path: Path, body: bytes) -> None:
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        temporary.write_bytes(body)
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def run(
    argv: list[str],
    repo: Path,
    deadline: float | None,
    *,
    environment: dict[str, str] | None = None,
) -> tuple[subprocess.CompletedProcess[bytes], int]:
    remaining = None if deadline is None else deadline - time.monotonic()
    if remaining is not None and remaining <= 0:
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


def java_launch_authority() -> tuple[Path, Path]:
    raw_home = os.environ.get("JAVA_HOME")
    if not raw_home or not raw_home.strip():
        raise PreflightFailure("JAVA", "JAVA_HOME must be explicit for preflight and launch parity")
    home = Path(raw_home)
    if home.is_symlink() or not home.is_dir():
        raise PreflightFailure("JAVA", "JAVA_HOME must be a real JDK directory")
    home = home.resolve(strict=True)
    configured_java = exact_executable(str(home / "bin" / "java"), "JAVA")
    return home, configured_java


def validate_run_contract(
    run_phase: str, seed_entity: str | None, state_root: Path | None
) -> None:
    if seed_entity is not None and not seed_entity.strip():
        raise PreflightFailure("RUN_PHASE", "seed entity must be nonblank when provided")
    if run_phase == "warm" and seed_entity is None:
        raise PreflightFailure("RUN_PHASE", "warm projection requires --seed-entity")
    if run_phase == "warm" and state_root is None:
        raise PreflightFailure("RUN_PHASE", "warm projection requires --state-root")


def effective_budget(run_phase: str, requested: float | None) -> float | None:
    if requested is not None and requested <= 0:
        raise PreflightFailure("ARGUMENTS", "budget must be positive")
    if run_phase == "warm":
        return 60.0 if requested is None else min(60.0, requested)
    return requested


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
    budget = effective_budget(args.run_phase, args.budget_seconds)
    deadline = None if budget is None else started + budget
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
    java_home, java = java_launch_authority()
    java_probe, java_millis = run([str(java), "-version"], repo, deadline)
    require_success(java_probe, "JAVA")
    java_version = (java_probe.stderr + java_probe.stdout).decode("utf-8", "replace")
    if 'version "21.' not in java_version:
        raise PreflightFailure("JAVA", "JDK 21 is required")
    probes: list[dict[str, object]] = [
        {
            "kind": "JAVA_21",
            "durationMillis": java_millis,
            "javaHome": str(java_home),
            "executable": str(java),
            "version": next((line.strip() for line in java_version.splitlines() if "version" in line), ""),
        }
    ]
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
    compiler_version = project_compiler_version(inspect_probe.stdout)
    persistent_compiler_index = supports_persistent_compiler_index(compiler_version)
    probes.append({
        "kind": "CLEW_PROJECT_INSPECT",
        "durationMillis": inspect_millis,
        "compilerVersion": compiler_version,
        "persistentCompilerIndexSupported": persistent_compiler_index,
    })
    index_argv = [str(clew)]
    if state_root is not None:
        index_argv.extend(["--compiler-index-root", str(Path(state_root) / "compiler-index")])
    index_argv.extend(["index", "--repo", str(repo), "--compilation", args.compilation])
    if args.run_phase == "warm" and not persistent_compiler_index:
        canonical_seed = canonical_cached_seed(args.seed_entity)
        probes.append({
            "kind": "CLEW_SEMANTIC_INDEX",
            "durationMillis": 0,
            "status": "SKIPPED_WARM_NO_PERSISTENT_BACKEND",
            "persistentCompilerIndexSupported": False,
        })
    else:
        index_probe, index_millis = run(index_argv, repo, deadline)
        require_success(index_probe, "SEMANTIC_INDEX")
        index_summary = semantic_index_summary(index_probe.stdout)
        canonical_seed = canonical_seed_entity(index_probe.stdout, args.seed_entity)
        if state_root is not None and persistent_compiler_index:
            require_persistent_reuse(index_summary, args.run_phase)
        probes.append({
            "kind": "CLEW_SEMANTIC_INDEX",
            "durationMillis": index_millis,
            "persistentCompilerIndexSupported": persistent_compiler_index,
            **index_summary,
        })
    elapsed = round((time.monotonic() - started) * 1000)
    if budget is not None and elapsed > round(budget * 1000):
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
            "requestedSeedEntity": args.seed_entity,
            "canonicalSeedEntity": canonical_seed,
            "compilerVersion": compiler_version,
            "persistentCompilerIndexSupported": persistent_compiler_index,
            "requiresVerifiedCacheHit": args.run_phase == "warm",
        },
        "build": build,
        "probes": probes,
        "elapsedMillis": elapsed,
        "budgetMillis": None if budget is None else round(budget * 1000),
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
    parser.add_argument(
        "--budget-seconds",
        type=float,
        help="optional cold timeout; warm runs are always capped at 60 seconds",
    )
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
