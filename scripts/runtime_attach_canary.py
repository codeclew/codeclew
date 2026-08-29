#!/usr/bin/env python3
"""Prove that all matching RELEASE selectors attach without build tools."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


SCHEMA = "codeclew-runtime-attach-canary/1.0"
MAX_OUTPUT_BYTES = 1024 * 1024
FORBIDDEN_WARM_PROCESSES = ("cargo", "rustc", "gradle", "mvn", "maven")


class CanaryError(RuntimeError):
    pass


def canonical(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()


def digest(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def load_json(path: Path) -> dict[str, object]:
    if not path.is_file() or path.is_symlink() or path.stat().st_size > MAX_OUTPUT_BYTES:
        raise CanaryError("candidate metadata is unavailable")
    try:
        value = json.loads(path.read_bytes())
    except (OSError, ValueError, TypeError) as error:
        raise CanaryError("candidate metadata is invalid") from error
    if not isinstance(value, dict):
        raise CanaryError("candidate metadata is invalid")
    return value


def candidate_authority(root: Path) -> tuple[dict[str, object], Path, Path, Path]:
    root = root.resolve(strict=True)
    metadata = load_json(root / "release.json")
    unsigned = dict(metadata)
    expected_candidate_digest = unsigned.get("candidateDigest")
    unsigned["candidateDigest"] = ""
    if (
        metadata.get("schema") != "codeclew-local-release-candidate/1.0"
        or metadata.get("status") != "LOCAL_ONLY"
        or expected_candidate_digest != digest(canonical(unsigned))
    ):
        raise CanaryError("local candidate authority is invalid")
    seeds = sorted(root.glob("seed/release-N-*/seed.json"))
    if len(seeds) != 1:
        raise CanaryError("local candidate seed is ambiguous")
    seed = load_json(seeds[0])
    runtime_key = metadata.get("runtimeKey")
    if seed.get("runtimeKey") != runtime_key or not isinstance(runtime_key, str):
        raise CanaryError("local candidate runtime binding is invalid")
    state = seeds[0].parent / "parallel-state" / "v2"
    runtime = state / "runtimes" / runtime_key.removeprefix("sha256:")
    lease = state / "locks" / f"runtime-{runtime_key.removeprefix('sha256:')}.lease"
    if not runtime.is_dir() or not lease.is_file():
        raise CanaryError("local candidate runtime is unavailable")
    return metadata, seeds[0], runtime, lease


def parse_capabilities(completed: subprocess.CompletedProcess[bytes]) -> dict[str, object]:
    if completed.returncode != 0 or len(completed.stdout) > MAX_OUTPUT_BYTES:
        raise CanaryError("selector did not return bounded capabilities")
    try:
        value = json.loads(completed.stdout)
    except (ValueError, TypeError) as error:
        raise CanaryError("selector capabilities are invalid") from error
    if not isinstance(value, dict) or value.get("schema") != "codeclew-capabilities/1.0":
        raise CanaryError("selector capabilities schema is invalid")
    return value


def selector_result(
    name: str,
    capabilities: dict[str, object],
    elapsed_millis: int,
    expected: dict[str, object],
) -> dict[str, object]:
    if (
        capabilities.get("runtimeMode") != "RELEASE"
        or capabilities.get("runtimeKey") != expected.get("runtimeKey")
        or capabilities.get("runtimeManifestDigest")
        != expected.get("runtimeManifestDigest")
        or capabilities.get("productVersion")
        != str(expected.get("version", "")).removeprefix("v")
    ):
        raise CanaryError(f"{name} selector resolved a mismatched runtime")
    return {
        "durationMillis": elapsed_millis,
        "productVersion": capabilities["productVersion"],
        "runtimeKey": capabilities["runtimeKey"],
        "runtimeManifestDigest": capabilities["runtimeManifestDigest"],
        "selector": name,
        "status": "ATTACHED",
    }


def run_command(
    arguments: list[str], cwd: Path, environment: dict[str, str], pass_fds: tuple[int, ...] = ()
) -> tuple[dict[str, object], int]:
    started = time.monotonic_ns()
    completed = subprocess.run(
        arguments,
        cwd=cwd,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        pass_fds=pass_fds,
        timeout=30,
        check=False,
    )
    elapsed = (time.monotonic_ns() - started) // 1_000_000
    return parse_capabilities(completed), int(elapsed)


def direct_capabilities(
    candidate: Path,
    runtime: Path,
    lease: Path,
    environment: dict[str, str],
) -> tuple[dict[str, object], int]:
    state = runtime.parent.parent
    state_fd = os.open(state, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    runtime_fd = os.open(runtime, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    lease_fd = os.open(lease, os.O_RDWR)
    try:
        fcntl.flock(lease_fd, fcntl.LOCK_SH)
        direct_environment = {
            name: value for name, value in environment.items() if not name.startswith("CODECLEW_")
        }
        direct_environment.update(
            {
                "CODECLEW_RUNTIME_LEASE_FD": str(lease_fd),
                "CODECLEW_RUNTIME_ROOT_FD": str(runtime_fd),
                "CODECLEW_STATE_ROOT_FD": str(state_fd),
            }
        )
        return run_command(
            [str(runtime / "bin" / "clew"), "capabilities"],
            candidate,
            direct_environment,
            (state_fd, runtime_fd, lease_fd),
        )
    finally:
        os.close(lease_fd)
        os.close(runtime_fd)
        os.close(state_fd)


def stale_result(capabilities: dict[str, object], expected: dict[str, object]) -> dict[str, object]:
    matches = (
        capabilities.get("runtimeKey") == expected.get("runtimeKey")
        and capabilities.get("runtimeManifestDigest")
        == expected.get("runtimeManifestDigest")
        and capabilities.get("productVersion")
        == str(expected.get("version", "")).removeprefix("v")
    )
    return {
        "code": "UNEXPECTED_MATCH" if matches else "RUNTIME_ATTACH_MISMATCH",
        "selector": "stale-path",
        "status": "FAILED" if matches else "EXPECTED_MISMATCH",
    }


def write_audit_wrappers(root: Path, log: Path) -> Path:
    directory = root / "audit-bin"
    directory.mkdir(mode=0o700)
    for name in FORBIDDEN_WARM_PROCESSES:
        path = directory / name
        path.write_text(
            "#!/bin/sh\nprintf '%s\\n' \"${0##*/}\" >>\"$Q1_ATTACH_AUDIT_LOG\"\nexit 97\n",
            encoding="ascii",
        )
        path.chmod(0o700)
    log.write_bytes(b"")
    log.chmod(0o600)
    return directory


def run_canary(candidate: Path, stale_launcher: Path | None) -> dict[str, object]:
    candidate = candidate.resolve(strict=True)
    metadata, seed, runtime, lease = candidate_authority(candidate)
    with tempfile.TemporaryDirectory(prefix=".runtime-attach-canary-", dir=candidate.parent) as value:
        temporary = Path(value)
        audit_log = temporary / "process-audit.log"
        audit_bin = write_audit_wrappers(temporary, audit_log)
        environment = dict(os.environ)
        environment["PATH"] = str(audit_bin) + os.pathsep + environment.get("PATH", "")
        environment["Q1_ATTACH_AUDIT_LOG"] = str(audit_log)
        environment["CODECLEW_HOME"] = str(temporary / "state")

        installed, installed_ms = run_command(
            [str(candidate / "bin" / "clew"), "capabilities"], candidate, environment
        )
        source_environment = dict(environment)
        source_environment["CODECLEW_RUNTIME_SEED"] = str(seed)
        source, source_ms = run_command(
            [str(candidate / "source" / "clew"), "capabilities"],
            candidate,
            source_environment,
        )
        direct, direct_ms = direct_capabilities(
            candidate, runtime, lease, environment
        )
        selectors = [
            selector_result("direct-capsule", direct, direct_ms, metadata),
            selector_result("installed-locator", installed, installed_ms, metadata),
            selector_result("pinned-source-locator", source, source_ms, metadata),
        ]
        negative = None
        if stale_launcher is not None:
            stale, _ = run_command(
                [str(stale_launcher.resolve(strict=True)), "capabilities"],
                candidate,
                environment,
            )
            negative = stale_result(stale, metadata)
            if negative["status"] != "EXPECTED_MISMATCH":
                raise CanaryError("stale launcher unexpectedly matched the candidate")
        process_starts = [row for row in audit_log.read_text(encoding="ascii").splitlines() if row]
        if process_starts:
            raise CanaryError("warm attach invoked a forbidden build process")
        return {
            "candidateDigest": metadata["candidateDigest"],
            "forbiddenWarmProcessStarts": process_starts,
            "negativeControl": negative,
            "privacyAssertions": {
                "containsAbsolutePaths": False,
                "containsRepositoryIdentity": False,
                "containsSource": False,
            },
            "runtimeKey": metadata["runtimeKey"],
            "runtimeManifestDigest": metadata["runtimeManifestDigest"],
            "schema": SCHEMA,
            "selectors": selectors,
            "status": "PASS",
        }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--stale-launcher", type=Path)
    arguments = parser.parse_args()
    print(json.dumps(run_canary(arguments.candidate, arguments.stale_launcher), separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (CanaryError, OSError, subprocess.SubprocessError) as error:
        print(
            json.dumps(
                {"error": str(error), "schema": SCHEMA, "status": "FAILED"},
                separators=(",", ":"),
                sort_keys=True,
            ),
            file=os.sys.stderr,
        )
        raise SystemExit(1)
