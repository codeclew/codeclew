#!/usr/bin/env python3
"""Machine-enforced stabilization-first development controller."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import hmac
import json
import os
from pathlib import Path
import platform
import re
import secrets
import shlex
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import xml.etree.ElementTree as ElementTree


ROOT = Path(__file__).resolve().parent.parent
PLAN_PATH = ROOT / "docs" / "stabilization-plan.json"
VERIFIER = ROOT / "scripts" / "stabilization_verifier.py"
PLAN_SCHEMA = "codeclew-stabilization-plan/2.0"
_DYNAMIC_AUTHORITY_CACHE: dict[str, object] = {}


class ControlError(RuntimeError):
    pass


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")


def digest_bytes(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def has_valid_embedded_digest(value: object, field: str) -> bool:
    if not isinstance(value, dict) or field not in value:
        return False
    expected = value[field]
    payload = dict(value)
    del payload[field]
    return expected == digest_bytes(canonical(payload))


def load_json(path: Path) -> object:
    with path.open("r", encoding="utf-8") as stream:
        return json.load(stream)


def atomic_private_write(path: Path, value: bytes, mode: int = 0o400) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(path.parent, 0o700)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.{secrets.token_hex(8)}")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(value)
            stream.flush()
            os.fsync(stream.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


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


def validate_relative(value: str) -> None:
    path = Path(value)
    if not value or path.is_absolute() or ".." in path.parts or "\x00" in value:
        raise ControlError("plan contains an unsafe repository-relative path")
    if ".semantic-thread" in path.parts:
        raise ControlError("legacy state cannot be a stabilization input")


def validate_plan(plan: object) -> dict[str, object]:
    if not isinstance(plan, dict) or plan.get("schema") != PLAN_SCHEMA:
        raise ControlError("unsupported stabilization plan schema")
    if set(plan) != {"checks", "planId", "schema", "steps", "tiers"}:
        raise ControlError("stabilization plan fields differ from the closed schema")
    if not isinstance(plan["planId"], str) or not plan["planId"]:
        raise ControlError("planId is required")

    tiers: dict[str, dict[str, object]] = {}
    for tier in plan["tiers"]:
        if not isinstance(tier, dict) or set(tier) != {
            "budgetSeconds",
            "cleanRequired",
            "id",
            "minimumMemoryBytes",
            "minimumPhysicalCores",
        }:
            raise ControlError("invalid tier")
        tier_id = tier["id"]
        if tier_id in tiers:
            raise ControlError("duplicate tier")
        if not isinstance(tier["budgetSeconds"], int) or tier["budgetSeconds"] <= 0:
            raise ControlError("tier budget must be positive")
        if not isinstance(tier["cleanRequired"], bool):
            raise ControlError("tier cleanRequired must be boolean")
        for field in ("minimumMemoryBytes", "minimumPhysicalCores"):
            if not isinstance(tier[field], int) or tier[field] < 0:
                raise ControlError("invalid host qualification")
        tiers[tier_id] = tier
    if set(tiers) != {f"L{index}" for index in range(8)}:
        raise ControlError("tiers must define exactly L0 through L7")

    steps: dict[str, dict[str, object]] = {}
    order: list[str] = []
    for step in plan["steps"]:
        if not isinstance(step, dict) or set(step) != {"dependencies", "id", "requiredChecks"}:
            raise ControlError("invalid step")
        step_id = step["id"]
        if not isinstance(step_id, str) or not step_id or step_id in steps:
            raise ControlError("invalid or duplicate step")
        if not isinstance(step["dependencies"], list) or not isinstance(step["requiredChecks"], list) or not step["requiredChecks"]:
            raise ControlError("step dependencies/checks must be non-empty lists where required")
        steps[step_id] = step
        order.append(step_id)
    visited: set[str] = set()
    active: set[str] = set()

    def visit(step_id: str) -> None:
        if step_id in active:
            raise ControlError("step dependency cycle")
        if step_id in visited:
            return
        active.add(step_id)
        for dependency in steps[step_id]["dependencies"]:
            if dependency not in steps:
                raise ControlError("unknown step dependency")
            visit(dependency)
        active.remove(step_id)
        visited.add(step_id)

    for step_id in order:
        visit(step_id)

    checks: dict[str, dict[str, object]] = {}

    def valid_dynamic_authorities(values: object, environment_keys: list[str]) -> bool:
        return isinstance(values, list) and all(
            value == "git-worktrees"
            or value == "trusted-seed"
            or value == "native-gradle-environment"
            or value == "native-maven-environment"
            or value == "native-provider-toolchain"
            or (
                isinstance(value, str)
                and value.startswith("environment-file:")
                and value.removeprefix("environment-file:") in environment_keys
            )
            for value in values
        )

    for check in plan["checks"]:
        required_check_keys = {
            "command",
            "environmentKeys",
            "gate",
            "id",
            "inputRoots",
            "step",
            "tier",
        }
        allowed_check_keys = required_check_keys | {"dynamicAuthorities", "prepare"}
        if (
            not isinstance(check, dict)
            or not required_check_keys.issubset(check)
            or not set(check).issubset(allowed_check_keys)
        ):
            raise ControlError("invalid check")
        check_id = check["id"]
        if not isinstance(check_id, str) or not check_id or check_id in checks:
            raise ControlError("invalid or duplicate check")
        if check["step"] not in steps or check["tier"] not in tiers:
            raise ControlError("check references an unknown step or tier")
        if not isinstance(check["command"], list) or not check["command"] or not all(isinstance(value, str) and value for value in check["command"]):
            raise ControlError("check command must be a non-empty argv")
        if not isinstance(check["inputRoots"], list) or not check["inputRoots"]:
            raise ControlError("check input roots are required")
        for root in check["inputRoots"]:
            if not isinstance(root, str):
                raise ControlError("input root must be a string")
            validate_relative(root)
        if not isinstance(check["environmentKeys"], list) or not all(isinstance(value, str) and value for value in check["environmentKeys"]):
            raise ControlError("invalid environment key list")
        dynamic_authorities = check.get("dynamicAuthorities", [])
        if not valid_dynamic_authorities(dynamic_authorities, check["environmentKeys"]):
            raise ControlError("invalid dynamic authority list")
        if check["gate"] is not None and (not isinstance(check["gate"], str) or not check["gate"]):
            raise ControlError("invalid gate identifier")
        prepare = check.get("prepare")
        if prepare is not None:
            required_prepare_keys = {
                "command", "environmentKeys", "gate", "inputRoots"
            }
            if (
                not isinstance(prepare, dict)
                or not required_prepare_keys.issubset(prepare)
                or not set(prepare).issubset(required_prepare_keys | {"dynamicAuthorities"})
                or not isinstance(prepare["command"], list)
                or not prepare["command"]
                or not all(isinstance(value, str) and value for value in prepare["command"])
                or not isinstance(prepare["inputRoots"], list)
                or not prepare["inputRoots"]
                or not isinstance(prepare["environmentKeys"], list)
                or not all(isinstance(value, str) and value for value in prepare["environmentKeys"])
                or not valid_dynamic_authorities(
                    prepare.get("dynamicAuthorities", []), prepare["environmentKeys"]
                )
                or prepare["gate"] is None
                or not isinstance(prepare["gate"], str)
                or not prepare["gate"]
            ):
                raise ControlError("invalid check preparation")
            for root in prepare["inputRoots"]:
                if not isinstance(root, str):
                    raise ControlError("preparation input root must be a string")
                validate_relative(root)
        checks[check_id] = check
    for step_id, step in steps.items():
        for check_id in step["requiredChecks"]:
            if check_id not in checks or checks[check_id]["step"] != step_id:
                raise ControlError("step requiredChecks authority mismatch")
    if set(checks) != {check for step in steps.values() for check in step["requiredChecks"]}:
        raise ControlError("every check must be required by exactly one step")
    return {"checks": checks, "order": order, "steps": steps, "tiers": tiers}


def state_root() -> Path:
    configured = os.environ.get("CODECLEW_CONTROL_HOME")
    root = Path(configured) if configured else Path.home() / ".cache" / "codeclew-control"
    if not root.is_absolute() or ".." in root.parts:
        raise ControlError("control home must be normalized and absolute")
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    metadata = root.stat()
    if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) & 0o077:
        raise ControlError("control home must be private and owner-controlled")
    os.chmod(root, 0o700)
    return root


def authorities(plan: dict[str, object]) -> dict[str, str]:
    python_runtime_digest = digest_bytes(canonical(native_python_runtime_authority()))
    return {
        "controllerDigest": digest_bytes(canonical({
            "pythonRuntimeDigest": python_runtime_digest,
            "sourceDigest": digest_bytes(Path(__file__).read_bytes()),
        })),
        "planDigest": digest_bytes(canonical(plan)),
        "verifierDigest": digest_bytes(canonical({
            "pythonRuntimeDigest": python_runtime_digest,
            "sourceDigest": digest_bytes(VERIFIER.read_bytes()),
        })),
    }


def selected_files(roots: list[str]) -> list[str]:
    arguments = ["ls-files", "--cached", "--others", "--exclude-standard", "-z", "--"]
    arguments.extend(roots)
    raw = subprocess.check_output(("git", *arguments), cwd=ROOT)
    paths = []
    for item in raw.split(b"\0"):
        if not item:
            continue
        value = item.decode("utf-8")
        if ".semantic-thread" not in Path(value).parts:
            paths.append(value)
    return sorted(set(paths))


def input_digest(check: dict[str, object]) -> str:
    hasher = hashlib.sha256()
    roots = list(check["inputRoots"])
    for argument in check["command"]:
        candidate = Path(argument)
        if (
            not argument.startswith("-")
            and not candidate.is_absolute()
            and "/" in argument
            and argument not in roots
        ):
            validate_relative(argument)
            roots.append(argument)
    for root in roots:
        path = ROOT / root
        if root != "." and not path.exists() and not path.is_symlink():
            hasher.update(b"missing\0" + root.encode("utf-8") + b"\0")
    for relative in selected_files(roots):
        path = ROOT / relative
        if not path.exists() and not path.is_symlink():
            hasher.update(relative.encode("utf-8") + b"\0deleted\0")
            continue
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            mode = b"symlink"
            data = os.readlink(path).encode("utf-8")
        elif stat.S_ISREG(metadata.st_mode):
            mode = b"executable" if metadata.st_mode & 0o111 else b"regular"
            data = path.read_bytes()
        else:
            raise ControlError("stabilization input is not a regular file or symlink")
        hasher.update(relative.encode("utf-8") + b"\0" + mode + b"\0")
        hasher.update(len(data).to_bytes(8, "big") + data)
    return "sha256:" + hasher.hexdigest()


def environment_digest(check: dict[str, object]) -> str:
    values = {key: os.environ.get(key) for key in check["environmentKeys"]}
    values["platform"] = {
        "machine": platform.machine(),
        "python": platform.python_version(),
        "system": platform.system(),
    }
    return digest_bytes(canonical(values))


def dynamic_authority_digest(
    check: dict[str, object], *, refresh: bool = False
) -> str:
    values: dict[str, object] = {}
    for authority in check.get("dynamicAuthorities", []):
        if not refresh and authority in _DYNAMIC_AUTHORITY_CACHE:
            values[authority] = _DYNAMIC_AUTHORITY_CACHE[authority]
            continue
        if authority == "git-worktrees":
            raw = subprocess.check_output(
                ("git", "worktree", "list", "--porcelain"), cwd=ROOT
            )
            values[authority] = digest_bytes(raw)
            _DYNAMIC_AUTHORITY_CACHE[authority] = values[authority]
            continue
        if authority == "trusted-seed":
            values[authority] = trusted_seed_authority_digest()
            _DYNAMIC_AUTHORITY_CACHE[authority] = values[authority]
            continue
        if authority == "native-gradle-environment":
            values[authority] = native_gradle_environment_digest(check)
            _DYNAMIC_AUTHORITY_CACHE[authority] = values[authority]
            continue
        if authority == "native-maven-environment":
            values[authority] = native_maven_environment_digest(check)
            _DYNAMIC_AUTHORITY_CACHE[authority] = values[authority]
            continue
        if authority == "native-provider-toolchain":
            values[authority] = native_provider_toolchain_digest(check)
            _DYNAMIC_AUTHORITY_CACHE[authority] = values[authority]
            continue
        key = authority.removeprefix("environment-file:")
        configured = os.environ.get(key)
        if configured is None:
            values[authority] = None
            _DYNAMIC_AUTHORITY_CACHE[authority] = values[authority]
            continue
        path = Path(configured)
        if not path.is_absolute() or ".." in path.parts:
            raise ControlError("dynamic authority file must be normalized and absolute")
        try:
            metadata = path.lstat()
            if not stat.S_ISREG(metadata.st_mode):
                raise ControlError("dynamic authority file must be regular")
            values[authority] = digest_bytes(path.read_bytes())
        except FileNotFoundError:
            values[authority] = "MISSING"
        _DYNAMIC_AUTHORITY_CACHE[authority] = values[authority]
    return digest_bytes(canonical(values))


def read_owned_file(
    path: Path, limit: int, label: str, expected_mode: int | None = None
) -> bytes:
    try:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    except OSError as error:
        raise ControlError(f"{label} is unavailable") from error
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or metadata.st_size > limit
            or (
                expected_mode is not None
                and stat.S_IMODE(metadata.st_mode) != expected_mode
            )
        ):
            raise ControlError(f"{label} is unsafe")
        with os.fdopen(descriptor, "rb", closefd=False) as stream:
            value = stream.read(limit + 1)
        if len(value) > limit:
            raise ControlError(f"{label} is oversized")
        return value
    finally:
        os.close(descriptor)


def digest_file_path(
    path: Path,
    limit: int,
    label: str,
    *,
    require_owner: bool = True,
    require_sealed: bool = True,
) -> tuple[int, str]:
    try:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    except OSError as error:
        raise ControlError(f"{label} is unavailable") from error
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or (require_owner and metadata.st_uid != os.geteuid())
            or metadata.st_size > limit
            or (require_sealed and stat.S_IMODE(metadata.st_mode) & 0o277)
        ):
            raise ControlError(f"{label} is unsafe")
        hasher = hashlib.sha256()
        with os.fdopen(descriptor, "rb", closefd=False) as stream:
            while block := stream.read(1024 * 1024):
                hasher.update(block)
        after = os.fstat(descriptor)
        if (
            (metadata.st_dev, metadata.st_ino, metadata.st_size, metadata.st_mtime_ns,
             metadata.st_ctime_ns, stat.S_IMODE(metadata.st_mode))
            != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns,
                after.st_ctime_ns, stat.S_IMODE(after.st_mode))
        ):
            raise ControlError(f"{label} changed while it was read")
        return metadata.st_size, "sha256:" + hasher.hexdigest()
    finally:
        os.close(descriptor)


def capsule_relative_path(value: object, label: str) -> str:
    if not isinstance(value, str) or not value or "\\" in value:
        raise ControlError(f"trusted seed {label} is invalid")
    path = Path(value)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in value.split("/")):
        raise ControlError(f"trusted seed {label} is invalid")
    return path.as_posix()


def runtime_tree_hash(rows: list[dict[str, object]]) -> str:
    hasher = hashlib.sha256()
    for row in rows:
        hasher.update(str(row["path"]).encode())
        hasher.update(b"\0")
        hasher.update(str(row["mode"]).encode())
        hasher.update(b"\0")
        hasher.update(str(row["size"]).encode())
        hasher.update(b"\0")
        hasher.update(str(row["sha256"]).encode())
        hasher.update(b"\0")
    return "sha256:" + hasher.hexdigest()


def trusted_capsule_authority(
    capsule: Path, runtime_key: str, seed: dict[str, object]
) -> tuple[bytes, str]:
    try:
        capsule_metadata = capsule.lstat()
    except OSError as error:
        raise ControlError("trusted seed capsule is unavailable") from error
    if (
        not stat.S_ISDIR(capsule_metadata.st_mode)
        or stat.S_ISLNK(capsule_metadata.st_mode)
        or capsule_metadata.st_uid != os.geteuid()
        or stat.S_IMODE(capsule_metadata.st_mode) != 0o500
        or capsule.resolve(strict=True) != capsule
    ):
        raise ControlError("trusted seed capsule root is unsafe")
    manifest_bytes = read_owned_file(
        capsule / "runtime.json",
        16 * 1024 * 1024,
        "trusted seed capsule manifest",
    )
    try:
        manifest = json.loads(manifest_bytes)
    except (ValueError, TypeError) as error:
        raise ControlError("trusted seed capsule manifest is invalid") from error
    unsigned = dict(manifest) if isinstance(manifest, dict) else {}
    expected_manifest_digest = unsigned.get("manifestDigest")
    unsigned["manifestDigest"] = ""
    if (
        not isinstance(manifest, dict)
        or manifest_bytes != canonical(manifest) + b"\n"
        or manifest.get("schema") != "codeclew-runtime-capsule/4.0"
        or manifest.get("runtimeKey") != runtime_key
        or manifest.get("mode") != "RELEASE"
        or expected_manifest_digest != digest_bytes(canonical(unsigned))
        or expected_manifest_digest != seed.get("manifestDigest")
    ):
        raise ControlError("trusted seed capsule manifest authority is invalid")
    artifacts = manifest.get("artifacts")
    workers = manifest.get("workers")
    if (
        not isinstance(artifacts, dict)
        or not artifacts
        or len(artifacts) > 64
        or not isinstance(workers, dict)
        or not workers
        or len(workers) > 64
    ):
        raise ControlError("trusted seed capsule manifest closure is invalid")
    expected_files = {"READY", "runtime.json"}
    for artifact in artifacts.values():
        if not isinstance(artifact, dict):
            raise ControlError("trusted seed capsule artifact row is invalid")
        expected_files.add(
            capsule_relative_path(artifact.get("path"), "artifact path")
        )
    for worker in workers.values():
        if not isinstance(worker, dict) or not isinstance(worker.get("files"), list):
            raise ControlError("trusted seed capsule worker row is invalid")
        if len(worker["files"]) > 10_000:
            raise ControlError("trusted seed capsule worker closure is oversized")
        distribution = capsule_relative_path(
            worker.get("distribution"), "worker distribution"
        )
        for row in worker["files"]:
            if not isinstance(row, dict):
                raise ControlError("trusted seed capsule worker file row is invalid")
            child = capsule_relative_path(row.get("path"), "worker file")
            expected_files.add((Path(distribution) / child).as_posix())
    if len(expected_files) > 200_000:
        raise ControlError("trusted seed capsule file closure is oversized")
    expected_directories = {"."}
    for relative in expected_files:
        parent = Path(relative).parent
        while parent != Path("."):
            expected_directories.add(parent.as_posix())
            parent = parent.parent
    if len(expected_directories) > 200_000:
        raise ControlError("trusted seed capsule directory closure is oversized")
    rows: dict[str, dict[str, object]] = {}
    observed_directories = {"."}
    total_size = 0
    for current, directories, files in os.walk(
        capsule, topdown=True, followlinks=False
    ):
        directories.sort()
        files.sort()
        current_path = Path(current)
        for name in directories:
            path = current_path / name
            relative = path.relative_to(capsule).as_posix()
            if relative not in expected_directories:
                raise ControlError("trusted seed capsule has an undeclared directory")
            metadata = path.lstat()
            if (
                stat.S_ISLNK(metadata.st_mode)
                or not stat.S_ISDIR(metadata.st_mode)
                or metadata.st_uid != os.geteuid()
                or stat.S_IMODE(metadata.st_mode) != 0o500
            ):
                raise ControlError("trusted seed capsule contains an unsafe directory")
            observed_directories.add(relative)
        for name in files:
            path = current_path / name
            relative = path.relative_to(capsule).as_posix()
            if relative not in expected_files:
                raise ControlError("trusted seed capsule has an undeclared file")
            metadata = path.lstat()
            if (
                stat.S_ISLNK(metadata.st_mode)
                or not stat.S_ISREG(metadata.st_mode)
                or metadata.st_uid != os.geteuid()
                or stat.S_IMODE(metadata.st_mode) not in {0o400, 0o500}
            ):
                raise ControlError("trusted seed capsule contains an unsafe file")
            size, sha256 = digest_file_path(
                path, 4 * 1024 * 1024 * 1024, "trusted seed capsule file"
            )
            total_size += size
            if len(rows) >= 200_000 or total_size > 16 * 1024 * 1024 * 1024:
                raise ControlError("trusted seed capsule closure is oversized")
            rows[relative] = {
                "mode": 0o111 if stat.S_IMODE(metadata.st_mode) == 0o500 else 0,
                "path": relative,
                "sha256": sha256,
                "size": size,
            }
    manifest_row = rows.get("runtime.json")
    if (
        manifest_row is None
        or manifest_row.get("sha256") != digest_bytes(manifest_bytes)
        or manifest_row.get("size") != len(manifest_bytes)
    ):
        raise ControlError("trusted seed capsule manifest changed during verification")
    artifact_hashes: dict[str, str] = {}
    artifact_paths = set()
    for name, artifact in sorted(artifacts.items()):
        if not isinstance(name, str) or not isinstance(artifact, dict):
            raise ControlError("trusted seed capsule artifact row is invalid")
        relative = capsule_relative_path(artifact.get("path"), "artifact path")
        if relative in artifact_paths:
            raise ControlError("trusted seed capsule artifact path is duplicated")
        artifact_paths.add(relative)
        row = rows.get(relative)
        if (
            row is None
            or artifact.get("mode") != row["mode"]
            or artifact.get("size") != row["size"]
            or artifact.get("sha256") != row["sha256"]
        ):
            raise ControlError("trusted seed capsule artifact authority mismatch")
        expected_files.add(relative)
        artifact_hashes[name] = str(row["sha256"])
    worker_hashes: dict[str, str] = {}
    for name, worker in sorted(workers.items()):
        if not isinstance(name, str) or not isinstance(worker, dict):
            raise ControlError("trusted seed capsule worker row is invalid")
        distribution = capsule_relative_path(
            worker.get("distribution"), "worker distribution"
        )
        declared = worker.get("files")
        if not isinstance(declared, list):
            raise ControlError("trusted seed capsule worker files are invalid")
        actual = []
        previous_child = None
        for declared_row in declared:
            if not isinstance(declared_row, dict):
                raise ControlError("trusted seed capsule worker file row is invalid")
            child = capsule_relative_path(declared_row.get("path"), "worker file")
            if previous_child is not None and child <= previous_child:
                raise ControlError(
                    "trusted seed capsule worker files are not unique and sorted"
                )
            previous_child = child
            relative = (Path(distribution) / child).as_posix()
            row = rows.get(relative)
            expected = {
                "mode": row["mode"] if row else None,
                "path": child,
                "sha256": row["sha256"] if row else None,
                "size": row["size"] if row else None,
            }
            if row is None or declared_row != expected:
                raise ControlError("trusted seed capsule worker authority mismatch")
            expected_files.add(relative)
            actual.append(expected)
        if worker.get("treeHash") != runtime_tree_hash(actual):
            raise ControlError("trusted seed capsule worker tree authority mismatch")
        worker_hashes[name] = str(worker["treeHash"])
    if set(rows) != expected_files:
        raise ControlError("trusted seed capsule file closure mismatch")
    if observed_directories != expected_directories:
        raise ControlError("trusted seed capsule directory closure mismatch")
    ready = read_owned_file(capsule / "READY", 256, "trusted seed capsule READY")
    if (
        ready != (runtime_key + "\n").encode()
        or rows.get("READY") is None
        or rows["READY"].get("sha256") != digest_bytes(ready)
        or rows["READY"].get("size") != len(ready)
    ):
        raise ControlError("trusted seed capsule READY authority mismatch")
    if (
        artifact_hashes != seed.get("artifactHashes")
        or worker_hashes != seed.get("workerTreeHashes")
    ):
        raise ControlError("trusted seed capsule differs from seed authority")
    closure = [rows[name] for name in sorted(rows)]
    return manifest_bytes, digest_bytes(canonical({"files": closure}))


def native_environment(prefixes: tuple[str, ...], fixed: set[str]) -> dict[str, str | None]:
    keys = sorted(fixed | {key for key in os.environ if key.startswith(prefixes)})
    return {
        key: digest_bytes(os.environ[key].encode()) if key in os.environ else None
        for key in keys
    }


def native_tool_authority(name: str, configured: Path | None = None) -> tuple[Path, dict[str, object]]:
    selected = str(configured) if configured is not None else shutil.which(name)
    if selected is None:
        raise ControlError(f"native build tool is unavailable: {name}")
    path = Path(selected).resolve(strict=True)
    size, sha256 = digest_file_path(
        path,
        1024 * 1024 * 1024,
        f"native build tool {name}",
        require_owner=False,
        require_sealed=False,
    )
    return path, {
        "executableMode": stat.S_IMODE(path.stat().st_mode) & 0o111,
        "sha256": sha256,
        "size": size,
    }


def native_tree_authority(
    root: Path,
    relative_roots: list[str],
    label: str,
    *,
    max_files: int = 100_000,
    max_bytes: int = 16 * 1024 * 1024 * 1024,
    require_owner: bool = True,
    excluded_directory_names: set[str] | None = None,
    excluded_suffixes: tuple[str, ...] = (),
) -> list[dict[str, object]]:
    if not root.is_absolute() or ".." in root.parts:
        raise ControlError(f"{label} root is unsafe")
    if root.exists():
        metadata = root.lstat()
        if (
            not stat.S_ISDIR(metadata.st_mode)
            or stat.S_ISLNK(metadata.st_mode)
            or root.resolve(strict=True) != root
        ):
            raise ControlError(f"{label} root is unsafe")
    rows: list[dict[str, object]] = []
    observed: set[str] = set()
    total_size = 0
    for relative_root in relative_roots:
        target = root / relative_root
        if not target.exists():
            rows.append({"path": relative_root, "status": "MISSING"})
            continue
        if target.is_symlink():
            raise ControlError(f"{label} contains a symlinked root")
        if target.is_file():
            candidates = iter((target,))
        else:
            def tree_files() -> object:
                for current, directories, files in os.walk(
                    target, topdown=True, followlinks=False
                ):
                    directories.sort()
                    files.sort()
                    if excluded_directory_names:
                        directories[:] = [
                            name for name in directories
                            if name not in excluded_directory_names
                        ]
                    current_path = Path(current)
                    for directory in directories:
                        metadata = (current_path / directory).lstat()
                        if stat.S_ISLNK(metadata.st_mode):
                            raise ControlError(f"{label} contains a symlinked directory")
                    for file_name in files:
                        if excluded_suffixes and file_name.endswith(excluded_suffixes):
                            continue
                        yield current_path / file_name

            candidates = tree_files()
        for path in candidates:
            metadata = path.lstat()
            if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
                raise ControlError(f"{label} contains an unsafe entry")
            relative = path.relative_to(root).as_posix()
            if relative in observed:
                continue
            observed.add(relative)
            size, sha256 = digest_file_path(
                path,
                4 * 1024 * 1024 * 1024,
                f"{label} file",
                require_owner=require_owner,
                require_sealed=False,
            )
            total_size += size
            if len(observed) > max_files or total_size > max_bytes:
                raise ControlError(f"{label} authority is oversized")
            rows.append(
                {
                    "executableMode": stat.S_IMODE(metadata.st_mode) & 0o111,
                    "path": relative,
                    "sha256": sha256,
                    "size": size,
                }
            )
    return rows


def native_java_authority() -> tuple[Path, Path, dict[str, object]]:
    configured_home = os.environ.get("JAVA_HOME")
    configured_java = Path(configured_home) / "bin" / "java" if configured_home else None
    java, executable = native_tool_authority("java", configured_java)
    try:
        completed = subprocess.run(
            (str(java), "-XshowSettings:properties", "-version"),
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=10,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise ControlError("native Java authority observation failed") from error
    observation = completed.stdout + completed.stderr
    if completed.returncode != 0 or len(observation) > 1024 * 1024:
        raise ControlError("native Java authority observation failed")
    properties: dict[str, str] = {}
    stable_keys = {
        "java.home",
        "java.runtime.version",
        "java.vendor",
        "java.version",
        "java.vm.name",
        "java.vm.version",
        "os.arch",
        "user.home",
    }
    for raw_line in observation.decode("utf-8", errors="replace").splitlines():
        if " = " not in raw_line:
            continue
        key, value = (part.strip() for part in raw_line.split(" = ", 1))
        if key in stable_keys:
            properties[key] = (
                digest_bytes(value.encode()) if key in {"java.home", "user.home"} else value
            )
    observed_home = None
    observed_user_home = None
    for raw_line in observation.decode("utf-8", errors="replace").splitlines():
        if raw_line.strip().startswith("java.home = "):
            observed_home = raw_line.split(" = ", 1)[1].strip()
        elif raw_line.strip().startswith("user.home = "):
            observed_user_home = raw_line.split(" = ", 1)[1].strip()
    if observed_home is None:
        raise ControlError("native Java did not report its actual runtime home")
    observed_home_path = Path(observed_home)
    if not observed_home_path.is_absolute() or ".." in observed_home_path.parts:
        raise ControlError("native Java reported an unsafe runtime home")
    home = observed_home_path.resolve(strict=True)
    user_home = Path(observed_user_home or os.environ.get("HOME", str(Path.home())))
    if not user_home.is_absolute() or ".." in user_home.parts:
        raise ControlError("native Java user home is unsafe")
    closure = native_tree_authority(
        home,
        ["release", "bin", "conf", "lib"],
        "native JDK",
        max_files=100_000,
        max_bytes=8 * 1024 * 1024 * 1024,
        require_owner=False,
    )
    return java, user_home, {
        "closure": closure,
        "executable": executable,
        "selection": "JAVA_HOME" if configured_home else "PATH",
        "stableProperties": properties,
    }


def qualification_tool_authority(*, include_maven: bool) -> dict[str, object]:
    tools: dict[str, object] = {}
    names = [
        "cat", "chmod", "cp", "dirname", "git", "mkdir", "mktemp", "mv",
        "python3", "rm", "rmdir", "sysctl", "tar", "tr",
    ]
    if include_maven:
        names.append("mvn")
    for name in names:
        if name == "sysctl" and shutil.which(name) is None:
            tools[name] = "MISSING"
            continue
        _path, tools[name] = native_tool_authority(name)
    _path, tools["sh"] = native_tool_authority("sh", Path("/bin/sh"))
    return tools


def native_python_runtime_authority() -> dict[str, object]:
    python, executable = native_tool_authority(
        "controller Python", Path(sys.executable)
    )
    selected_from_path = shutil.which("python3")
    if (
        selected_from_path is None
        or Path(selected_from_path).resolve(strict=True) != python
    ):
        raise ControlError("controller Python differs from the python3 command authority")
    program = (
        "import json,sys,sysconfig;"
        "print(json.dumps({'basePrefix':sys.base_prefix,'executable':sys.executable,"
        "'enableShared':sysconfig.get_config_var('Py_ENABLE_SHARED'),"
        "'framework':sysconfig.get_config_var('PYTHONFRAMEWORK'),"
        "'frameworkPrefix':sysconfig.get_config_var('PYTHONFRAMEWORKPREFIX'),"
        "'ldlibrary':sysconfig.get_config_var('LDLIBRARY'),"
        "'libdir':sysconfig.get_config_var('LIBDIR'),'prefix':sys.prefix,"
        "'stdlib':sysconfig.get_path('stdlib'),'platstdlib':sysconfig.get_path('platstdlib'),"
        "'extensions':sysconfig.get_config_var('DESTSHARED'),'version':list(sys.version_info[:3])},"
        "sort_keys=True,separators=(',',':')))"
    )
    try:
        completed = subprocess.run(
            (str(python), "-I", "-S", "-c", program),
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=15,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise ControlError("native Python runtime observation failed") from error
    if (
        completed.returncode != 0
        or len(completed.stdout) > 64 * 1024
        or len(completed.stderr) > 64 * 1024
    ):
        raise ControlError("native Python runtime observation failed")
    try:
        observation = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise ControlError("native Python runtime observation is invalid") from error
    observed_executable = observation.get("executable") if isinstance(observation, dict) else None
    if (
        not isinstance(observed_executable, str)
        or Path(observed_executable).resolve(strict=True) != python
    ):
        raise ControlError("native Python observation used another executable")
    roots = []
    for key in ("stdlib", "platstdlib", "extensions"):
        value = observation.get(key) if isinstance(observation, dict) else None
        if not isinstance(value, str):
            raise ControlError("native Python runtime closure is incomplete")
        path = Path(value)
        if not path.is_absolute() or ".." in path.parts:
            raise ControlError("native Python runtime path is unsafe")
        resolved = path.resolve(strict=True)
        if resolved not in roots:
            roots.append(resolved)
    closures = [
        native_tree_authority(
            root,
            ["."],
            "native Python runtime",
            max_files=200_000,
            max_bytes=8 * 1024 * 1024 * 1024,
            require_owner=False,
            excluded_directory_names={"site-packages"},
        )
        for root in roots
    ]
    library_candidates: list[tuple[str, Path]] = []
    libdir = observation.get("libdir")
    ldlibrary = observation.get("ldlibrary")
    if isinstance(libdir, str) and isinstance(ldlibrary, str) and ldlibrary:
        candidate = Path(libdir) / ldlibrary
        if candidate.exists():
            library_candidates.append(("SHARED_LIBRARY", candidate))
    framework = observation.get("framework")
    framework_prefix = observation.get("frameworkPrefix")
    version = observation.get("version")
    if (
        isinstance(framework, str)
        and framework
        and isinstance(framework_prefix, str)
        and isinstance(version, list)
        and len(version) >= 2
    ):
        candidate = (
            Path(framework_prefix)
            / f"{framework}.framework"
            / "Versions"
            / f"{version[0]}.{version[1]}"
            / framework
        )
        if candidate.exists():
            library_candidates.append(("FRAMEWORK", candidate))
    library_authorities = []
    seen_libraries = set()
    for kind, candidate in library_candidates:
        resolved = candidate.resolve(strict=True)
        if str(resolved) in seen_libraries:
            continue
        seen_libraries.add(str(resolved))
        _path, library = native_tool_authority(
            "Python native runtime library", resolved
        )
        library_authorities.append({"authority": library, "kind": kind})
    if observation.get("enableShared") and not library_authorities:
        raise ControlError("native Python shared runtime library is unavailable")
    return {
        "closures": closures,
        "executable": executable,
        "nativeLibraries": library_authorities,
        "observation": digest_bytes(completed.stdout),
        "schema": "codeclew-native-python-runtime-authority/1.0",
    }


def gradle_daemon_policy(check: dict[str, object] | None) -> str:
    if check is None:
        return "UNIT_TEST_CALLER"
    command = check.get("command")
    if not isinstance(command, list) or not command or not isinstance(command[0], str):
        raise ControlError("native Gradle check command is invalid")
    script = ROOT / command[0]
    try:
        source = script.read_text(encoding="utf-8")
    except OSError as error:
        raise ControlError("native Gradle gate script is unavailable") from error
    marker = '-Dorg.gradle.daemon=false'
    if marker not in source:
        raise ControlError("native Gradle gate does not force daemon reuse off")
    return f"CHECK_SCRIPT_FORCES_ORG_GRADLE_DAEMON_FALSE:{check.get('id')}"


def gradle_effective_homes(java_user_home: Path) -> list[Path]:
    configured = os.environ.get("GRADLE_USER_HOME")
    homes = [Path(configured) if configured else java_user_home / ".gradle"]
    for key in (
        "GRADLE_OPTS",
        "JAVA_OPTS",
        "JAVA_TOOL_OPTIONS",
        "JDK_JAVA_OPTIONS",
        "_JAVA_OPTIONS",
    ):
        raw = os.environ.get(key)
        if not raw:
            continue
        try:
            tokens = shlex.split(raw, comments=False, posix=True)
        except ValueError as error:
            raise ControlError("native Gradle JVM option authority is invalid") from error
        for token in tokens:
            if token.startswith("-Dgradle.user.home="):
                homes.append(Path(token.split("=", 1)[1]))
            elif token.startswith("-Duser.home="):
                homes.append(Path(token.split("=", 1)[1]) / ".gradle")
    result = []
    for home in homes:
        if not home.is_absolute() or ".." in home.parts:
            raise ControlError("native Gradle effective home is unsafe")
        if str(home) not in {str(value) for value in result}:
            result.append(home)
    return result


def native_gradle_environment_digest(check: dict[str, object] | None = None) -> str:
    common = {
        "CLASSPATH", "GRADLE_OPTS", "GRADLE_USER_HOME", "HOME", "HTTP_PROXY",
        "HTTPS_PROXY", "JAVA_HOME", "JAVA_OPTS", "JAVA_TOOL_OPTIONS", "JDK_HOME",
        "JDK_JAVA_OPTIONS", "KOTLIN_DAEMON_JVM_OPTIONS", "LANG", "LC_ALL",
        "NO_PROXY", "PATH", "TMPDIR", "_JAVA_OPTIONS", "http_proxy",
        "https_proxy", "no_proxy",
    }
    environment = native_environment(
        ("GIT_", "GRADLE_", "JAVA_", "JDK_", "KOTLIN_", "ORG_GRADLE_PROJECT_"),
        common,
    )
    _java, java_user_home, java_authority = native_java_authority()
    tools = qualification_tool_authority(include_maven=False)
    authorities = [
        native_tree_authority(
            gradle_home,
            [
                "gradle.properties", "init.gradle", "init.gradle.kts", "init.d",
                "wrapper/dists", "caches/modules-2",
            ],
            "native Gradle authority",
        )
        for gradle_home in gradle_effective_homes(java_user_home)
    ]
    return digest_bytes(canonical({
        "cachePrecondition": "GRADLE_PROPERTIES_WRAPPER_METADATA_AND_ARTIFACT_BYTES_EXACT",
        "daemonPolicy": gradle_daemon_policy(check),
        "environment": environment,
        "gradleAuthorities": authorities,
        "javaAuthority": java_authority,
        "provider": "GRADLE_PROJECT_NATIVE",
        "pythonRuntime": native_python_runtime_authority(),
        "schema": "codeclew-native-gradle-environment-authority/1.0",
        "tools": tools,
    }))


def native_provider_toolchain_digest(check: dict[str, object] | None = None) -> str:
    environment = native_environment(
        ("GIT_", "GRADLE_", "JAVA_", "JDK_", "KOTLIN_", "M2_", "MAVEN_"),
        {
            "CLASSPATH", "GRADLE_OPTS", "GRADLE_USER_HOME", "HOME", "JAVA_HOME",
            "JAVA_OPTS", "JAVA_TOOL_OPTIONS", "JDK_HOME", "JDK_JAVA_OPTIONS",
            "KOTLIN_DAEMON_JVM_OPTIONS", "M2_HOME", "MAVEN_ARGS", "MAVEN_CONFIG",
            "MAVEN_HOME", "MAVEN_OPTS", "MAVEN_USER_HOME", "PATH", "TMPDIR",
            "_JAVA_OPTIONS",
        },
    )
    _java, _user_home, java_authority = native_java_authority()
    tools = qualification_tool_authority(include_maven=True)
    mvn, _mvn_tool = native_tool_authority("mvn")
    try:
        completed = subprocess.run(
            (str(mvn), "-version"),
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=15,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise ControlError("native Maven toolchain observation failed") from error
    observation = completed.stdout + completed.stderr
    if completed.returncode != 0 or len(observation) > 1024 * 1024:
        raise ControlError("native Maven toolchain observation failed")
    maven_home = os.environ.get("MAVEN_HOME") or os.environ.get("M2_HOME")
    if not maven_home:
        for line in observation.decode("utf-8", errors="replace").splitlines():
            if line.startswith("Maven home: "):
                maven_home = line.removeprefix("Maven home: ").strip()
                break
    distribution = []
    if maven_home:
        distribution = native_tree_authority(
            Path(maven_home),
            ["bin", "boot", "conf", "lib"],
            "native Maven prime distribution",
            max_files=20_000,
            max_bytes=4 * 1024 * 1024 * 1024,
            require_owner=False,
        )
    return digest_bytes(canonical({
        "daemonPolicy": gradle_daemon_policy(check),
        "environment": environment,
        "javaAuthority": java_authority,
        "mavenDistribution": distribution,
        "mavenObservation": digest_bytes(observation),
        "pythonRuntime": native_python_runtime_authority(),
        "schema": "codeclew-native-provider-prime-toolchain/1.0",
        "tools": tools,
    }))


def resolve_maven_path(value: str, base: Path | None, user_home: Path) -> Path:
    expanded = value.strip()
    expanded = expanded.replace("${user.home}", str(user_home))
    for match in set(re.findall(r"\$\{env\.([A-Za-z_][A-Za-z0-9_]*)\}", expanded)):
        if match not in os.environ:
            raise ControlError("native Maven path contains an unresolved environment value")
        expanded = expanded.replace(f"${{env.{match}}}", os.environ[match])
    if "${" in expanded or expanded.startswith("~"):
        raise ControlError("native Maven path contains an unresolved placeholder")
    path = Path(expanded)
    if not path.is_absolute():
        if base is None:
            raise ControlError("native Maven environment path must be absolute")
        path = base / path
    if ".." in path.parts:
        raise ControlError("native Maven path is unsafe")
    return path


def maven_configuration_sources(check: dict[str, object] | None) -> list[tuple[str, Path | None]]:
    sources: list[tuple[str, Path | None]] = []
    for key in ("MAVEN_ARGS", "MAVEN_OPTS"):
        if value := os.environ.get(key):
            sources.append((value, None))
    configured = os.environ.get("MAVEN_CONFIG")
    if configured:
        configured_path = Path(configured)
        if configured.startswith("-"):
            sources.append((configured, None))
        elif configured_path.is_absolute() and ".." not in configured_path.parts:
            candidates = (
                [configured_path]
                if configured_path.is_file()
                else [configured_path / "maven.config", configured_path / "jvm.config"]
            )
            for candidate in candidates:
                if candidate.is_file() and not candidate.is_symlink():
                    sources.append((candidate.read_text(encoding="utf-8"), candidate.parent))
        else:
            raise ControlError("MAVEN_CONFIG must be an absolute path or option string")
    if check is not None:
        for relative in selected_files(check["inputRoots"]):
            if not (
                relative in {".mvn/maven.config", ".mvn/jvm.config"}
                or relative.endswith(("/.mvn/maven.config", "/.mvn/jvm.config"))
            ):
                continue
            path = ROOT / relative
            sources.append((path.read_text(encoding="utf-8"), path.parent.parent))
    return sources


def maven_external_configuration(
    check: dict[str, object] | None, user_home: Path
) -> tuple[list[Path], list[Path], list[Path]]:
    repositories: list[Path] = []
    settings: list[Path] = []
    user_homes: list[Path] = []
    for raw, base in maven_configuration_sources(check):
        try:
            tokens = shlex.split(raw, comments=True, posix=True)
        except ValueError as error:
            raise ControlError("native Maven option authority is invalid") from error
        index = 0
        while index < len(tokens):
            token = tokens[index]
            if token.startswith("-Dmaven.repo.local="):
                repositories.append(
                    resolve_maven_path(token.split("=", 1)[1], base, user_home)
                )
            elif token.startswith("-Duser.home="):
                user_homes.append(
                    resolve_maven_path(token.split("=", 1)[1], base, user_home)
                )
            elif token in {"-s", "--settings", "-gs", "--global-settings"}:
                index += 1
                if index >= len(tokens):
                    raise ControlError("native Maven settings option has no path")
                settings.append(resolve_maven_path(tokens[index], base, user_home))
            elif token.startswith(("--settings=", "--global-settings=")):
                settings.append(resolve_maven_path(token.split("=", 1)[1], base, user_home))
            elif token.startswith("-s") and len(token) > 2:
                settings.append(resolve_maven_path(token[2:], base, user_home))
            elif token.startswith("-gs") and len(token) > 3:
                settings.append(resolve_maven_path(token[3:], base, user_home))
            index += 1
    return repositories, settings, user_homes


def maven_settings_repository(path: Path, user_home: Path) -> Path | None:
    if not path.exists():
        return None
    size, _sha256 = digest_file_path(
        path,
        16 * 1024 * 1024,
        "native Maven settings",
        require_owner=False,
        require_sealed=False,
    )
    if size == 0:
        return None
    try:
        root = ElementTree.fromstring(path.read_bytes())
    except (OSError, ElementTree.ParseError) as error:
        raise ControlError("native Maven settings XML is invalid") from error
    for element in root.iter():
        if element.tag.rsplit("}", 1)[-1] == "localRepository" and element.text:
            return resolve_maven_path(element.text, path.parent, user_home)
    return None


def native_maven_environment_digest(check: dict[str, object] | None = None) -> str:
    common = {
        "CLASSPATH", "HOME", "HTTP_PROXY", "HTTPS_PROXY", "JAVA_HOME", "JAVA_OPTS",
        "JAVA_TOOL_OPTIONS", "JDK_HOME", "JDK_JAVA_OPTIONS", "LANG", "LC_ALL",
        "M2_HOME", "MAVEN_ARGS", "MAVEN_CONFIG", "MAVEN_HOME", "MAVEN_OPTS",
        "MAVEN_USER_HOME", "NO_PROXY", "PATH", "TMPDIR", "_JAVA_OPTIONS",
        "http_proxy", "https_proxy", "no_proxy",
    }
    environment = native_environment(
        ("GIT_", "JAVA_", "JDK_", "M2_", "MAVEN_"), common
    )
    _java, java_user_home, java_authority = native_java_authority()
    tools = qualification_tool_authority(include_maven=True)
    resolved = {"mvn": native_tool_authority("mvn")[0]}
    try:
        completed = subprocess.run(
            (str(resolved["mvn"]), "-version"),
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=15,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise ControlError("native Maven authority observation failed") from error
    observation = completed.stdout + completed.stderr
    if completed.returncode != 0 or len(observation) > 1024 * 1024:
        raise ControlError("native Maven authority observation failed")
    user_home = Path(os.environ.get("MAVEN_USER_HOME") or java_user_home)
    maven_user = user_home if os.environ.get("MAVEN_USER_HOME") else user_home / ".m2"
    user_rows = native_tree_authority(
        maven_user,
        ["settings.xml", "settings-security.xml", "toolchains.xml", "extensions.xml", "wrapper/dists"],
        "native Maven user authority",
    )
    configured_distribution = os.environ.get("MAVEN_HOME") or os.environ.get("M2_HOME")
    if not configured_distribution:
        for line in observation.decode("utf-8", errors="replace").splitlines():
            if line.startswith("Maven home: "):
                configured_distribution = line.removeprefix("Maven home: ").strip()
                break
    distribution_rows: list[dict[str, object]] = []
    if configured_distribution:
        distribution_rows = native_tree_authority(
            Path(configured_distribution), ["bin", "boot", "conf", "lib"],
            "native Maven distribution", max_files=20_000,
            max_bytes=4 * 1024 * 1024 * 1024,
            require_owner=False,
        )
    configured_repositories, configured_settings, configured_user_homes = maven_external_configuration(
        check, java_user_home
    )
    effective_user_homes: list[Path] = []
    for candidate in [java_user_home, *configured_user_homes]:
        if str(candidate) not in {str(value) for value in effective_user_homes}:
            effective_user_homes.append(candidate)
    settings_contexts = [
        (path, effective_home)
        for path in [maven_user / "settings.xml", *configured_settings]
        for effective_home in effective_user_homes
    ]
    repositories = [maven_user / "repository", *configured_repositories]
    additional_user_authorities = []
    for configured_user_home in configured_user_homes:
        configured_m2 = configured_user_home / ".m2"
        settings_contexts.append((configured_m2 / "settings.xml", configured_user_home))
        repositories.append(configured_m2 / "repository")
        additional_user_authorities.append(
            native_tree_authority(
                configured_m2,
                ["settings.xml", "settings-security.xml", "toolchains.xml", "extensions.xml", "wrapper/dists"],
                "native Maven overridden user authority",
            )
        )
    if configured_distribution:
        settings_contexts.extend(
            (Path(configured_distribution) / "conf" / "settings.xml", effective_home)
            for effective_home in effective_user_homes
        )
    settings_authorities = []
    seen_settings = set()
    seen_settings_contexts = set()
    for path, effective_home in settings_contexts:
        lexical = str(path)
        if lexical in seen_settings:
            pass
        else:
            seen_settings.add(lexical)
            settings_authorities.append(
                native_tree_authority(
                    path.parent,
                    [path.name],
                    "native Maven effective settings",
                    require_owner=False,
                )
            )
        context_key = (lexical, str(effective_home))
        if context_key in seen_settings_contexts:
            continue
        seen_settings_contexts.add(context_key)
        if repository := maven_settings_repository(path, effective_home):
            repositories.append(repository)
    repository_authorities = []
    seen_repositories = set()
    for repository in repositories:
        lexical = str(repository)
        if lexical in seen_repositories:
            continue
        seen_repositories.add(lexical)
        repository_authorities.append(
            native_tree_authority(repository, ["."], "native Maven effective repository")
        )
    return digest_bytes(canonical({
        "distributionAuthority": distribution_rows,
        "environment": environment,
        "javaAuthority": java_authority,
        "mavenObservation": digest_bytes(observation),
        "provider": "MAVEN_PROJECT_NATIVE",
        "pythonRuntime": native_python_runtime_authority(),
        "repositoryPrecondition": "MAVEN_SETTINGS_TOOLCHAINS_AND_REPOSITORY_BYTES_EXACT",
        "repositoryAuthorities": repository_authorities,
        "schema": "codeclew-native-maven-environment-authority/1.0",
        "tools": tools,
        "effectiveSettingsAuthorities": settings_authorities,
        "userAuthority": user_rows,
        "overriddenUserAuthorities": additional_user_authorities,
    }))


def trusted_seed_authority_digest() -> str:
    configured = os.environ.get("CODECLEW_SEED_HOME")
    root = Path(configured) if configured else Path.home() / ".cache" / "codeclew-seeds"
    if not root.is_absolute() or ".." in root.parts:
        raise ControlError("trusted seed home must be normalized and absolute")
    locator_path = root / "current.json"
    try:
        locator_bytes = read_owned_file(
            locator_path, 4096, "trusted seed locator", expected_mode=0o600
        )
        locator = json.loads(locator_bytes)
        epoch = locator.get("epoch") if isinstance(locator, dict) else None
        if (
            not isinstance(locator, dict)
            or locator.get("schema") != "codeclew-trusted-seed-locator/1.0"
            or not isinstance(epoch, str)
            or len(epoch) != len("release-N-") + 40
            or not epoch.startswith("release-N-")
            or not all(character in "0123456789abcdef" for character in epoch[10:])
            or Path(epoch).name != epoch
        ):
            raise ControlError("trusted seed locator authority is invalid")
        epoch_root = root / epoch
        parallel_root = epoch_root / "parallel-state"
        state_root = parallel_root / "v2"
        runtimes_root = state_root / "runtimes"
        for path in (epoch_root, parallel_root, state_root, runtimes_root):
            metadata = path.lstat()
            if (
                not stat.S_ISDIR(metadata.st_mode)
                or stat.S_ISLNK(metadata.st_mode)
                or metadata.st_uid != os.geteuid()
                or stat.S_IMODE(metadata.st_mode) != 0o700
                or path.resolve(strict=True) != path
            ):
                raise ControlError("trusted seed state authority is unsafe")
        seed_path = epoch_root / "seed.json"
        seed_bytes = read_owned_file(
            seed_path, 1024 * 1024, "trusted seed file", expected_mode=0o400
        )
        seed = json.loads(seed_bytes)
        runtime_key = seed.get("runtimeKey") if isinstance(seed, dict) else None
        unsigned_seed = dict(seed) if isinstance(seed, dict) else {}
        expected_seed_digest = unsigned_seed.pop("seedDigest", None)
        expected_seed_fields = {
            "artifactHashes",
            "buildEvidenceDigests",
            "manifestDigest",
            "mode",
            "runtimeKey",
            "schema",
            "seedDigest",
            "sourceRevision",
            "sourceTree",
            "stateEpoch",
            "workerTreeHashes",
        }
        if (
            not isinstance(seed, dict)
            or set(seed) != expected_seed_fields
            or seed.get("schema") != "codeclew-trusted-release-seed/1.0"
            or seed.get("mode") != "RELEASE"
            or expected_seed_digest != digest_bytes(canonical(unsigned_seed))
            or locator.get("runtimeKey") != runtime_key
            or locator.get("seedDigest") != seed.get("seedDigest")
            or not isinstance(runtime_key, str)
            or len(runtime_key) != 71
            or not runtime_key.startswith("sha256:")
            or not all(character in "0123456789abcdef" for character in runtime_key[7:])
            or seed.get("sourceRevision") != git("rev-parse", "HEAD")
            or seed.get("sourceTree") != git("rev-parse", "HEAD^{tree}")
        ):
            raise ControlError("trusted seed content authority is invalid")
        capsule = runtimes_root / runtime_key[7:]
        manifest_bytes, capsule_digest = trusted_capsule_authority(
            capsule, runtime_key, seed
        )
    except FileNotFoundError as error:
        raise ControlError("trusted seed authority is missing") from error
    return digest_bytes(
        canonical(
            {
                "locator": digest_bytes(locator_bytes),
                "manifest": digest_bytes(manifest_bytes),
                "capsule": capsule_digest,
                "seed": digest_bytes(seed_bytes),
            }
        )
    )


def evidence_digest(
    check: dict[str, object],
    authority: dict[str, str],
    dynamic_authority: str | None = None,
) -> str:
    dynamic = dynamic_authority or dynamic_authority_digest(check)
    value = {
        "checkAuthorityDigest": check_authority_digest(
            check, authority, dynamic_authority=dynamic
        ),
        "clean": is_clean(),
        "dynamicAuthorityDigest": dynamic,
        "environmentDigest": environment_digest(check),
        "memoryBytes": memory_bytes(),
        "physicalCores": physical_cores(),
        "sourceRevision": git("rev-parse", "HEAD"),
    }
    return digest_bytes(canonical(value))


def check_authority_digest(
    check: dict[str, object],
    authority: dict[str, str],
    dynamic_authority: str | None = None,
) -> str:
    """Return the current reusable authority for one check.

    The source revision belongs to the historical receipt, but not to this
    digest: an unrelated descendant commit or an undeclared transient execution
    value must not invalidate a completed step. Relevant input bytes, declared
    dynamic authorities, command bytes, or any controller/plan/verifier change
    must invalidate it.
    """
    value = {
        **authority,
        "commandDigest": digest_bytes(canonical(check["command"])),
        "dynamicAuthorityDigest": dynamic_authority
        or dynamic_authority_digest(check),
        "sourceInputDigest": input_digest(check),
    }
    return digest_bytes(canonical(value))


def physical_cores() -> int:
    if sys.platform == "darwin":
        try:
            physical = int(subprocess.check_output(("sysctl", "-n", "hw.physicalcpu"), text=True).strip())
            try:
                logical = len(os.sched_getaffinity(0))
            except AttributeError:
                logical = os.cpu_count() or physical
            return min(physical, logical)
        except (OSError, subprocess.SubprocessError, ValueError):
            return 0
    try:
        allowed = os.sched_getaffinity(0)
        pairs: set[tuple[str, str]] = set()
        processor = None
        physical = core = None
        for line in Path("/proc/cpuinfo").read_text(encoding="ascii").splitlines() + [""]:
            if not line:
                if processor in allowed and physical is not None and core is not None:
                    pairs.add((physical, core))
                processor = None
                physical = core = None
            elif line.startswith("processor"):
                processor = int(line.split(":", 1)[1].strip())
            elif line.startswith("physical id"):
                physical = line.split(":", 1)[1].strip()
            elif line.startswith("core id"):
                core = line.split(":", 1)[1].strip()
        return len(pairs)
    except (AttributeError, OSError, ValueError):
        return 0


def memory_bytes() -> int:
    if sys.platform == "darwin":
        try:
            return int(subprocess.check_output(("sysctl", "-n", "hw.memsize"), text=True).strip())
        except (OSError, subprocess.SubprocessError, ValueError):
            return 0
    try:
        for line in Path("/proc/meminfo").read_text(encoding="ascii").splitlines():
            if line.startswith("MemTotal:"):
                return int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        pass
    return 0


def is_clean() -> bool:
    return not bool(git("status", "--porcelain=v1", "--untracked-files=all"))


def plan_state(authority: dict[str, str]) -> Path:
    return state_root() / "plans" / authority["planDigest"].split(":", 1)[1]


def completion_path(authority: dict[str, str], step: str) -> Path:
    return plan_state(authority) / "completions" / f"{step}.json"


def valid_completion(
    model: dict[str, object], authority: dict[str, str], step: str
) -> bool:
    path = completion_path(authority, step)
    try:
        value = load_json(path)
    except (FileNotFoundError, json.JSONDecodeError, OSError):
        return False
    try:
        required = {
            check_id: check_authority_digest(model["checks"][check_id], authority)
            for check_id in model["steps"][step]["requiredChecks"]
        }
    except ControlError:
        return False
    return (
        isinstance(value, dict)
        and value.get("status") == "COMPLETE"
        and value.get("stepId") == step
        and value.get("checkAuthorities") == required
        and has_valid_embedded_digest(value, "completionDigest")
        and all(value.get(key) == authority[key] for key in authority)
    )


def require_dependencies(model: dict[str, object], authority: dict[str, str], step: str) -> None:
    completed = completed_steps(model, authority)
    missing = [
        dependency
        for dependency in model["steps"][step]["dependencies"]
        if dependency not in completed
    ]
    if missing:
        raise ControlError("step prerequisites are incomplete: " + ",".join(missing))


def receipt_path(authority: dict[str, str], check: str, input_authority: str) -> Path:
    return plan_state(authority) / "checks" / check / f"{input_authority.split(':', 1)[1]}.json"


def exclusive_lock(authority: dict[str, str], name: str):
    path = plan_state(authority) / "locks" / f"{name}.lock"
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(path.parent, 0o700)
    stream = path.open("a+b")
    os.chmod(path, 0o600)
    fcntl.flock(stream, fcntl.LOCK_EX)
    return stream


def global_exclusive_lock(name: str):
    path = state_root() / "locks" / f"{name}.lock"
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(path.parent, 0o700)
    stream = path.open("a+b")
    os.chmod(path, 0o600)
    fcntl.flock(stream, fcntl.LOCK_EX)
    return stream


def preparation_failure_path(
    authority: dict[str, str], check_id: str, evidence: str
) -> Path:
    return (
        plan_state(authority)
        / "preparations"
        / check_id
        / f"{evidence.split(':', 1)[1]}.failed.json"
    )


def private_secret() -> bytes:
    path = state_root() / "capability.key"
    if not path.exists():
        atomic_private_write(path, secrets.token_bytes(32), mode=0o600)
    metadata = path.stat()
    if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o600:
        raise ControlError("capability key authority is invalid")
    value = path.read_bytes()
    if len(value) != 32:
        raise ControlError("capability key is corrupt")
    return value


def issue_capability(authority: dict[str, str], gate: str, budget_seconds: int) -> Path:
    payload = {
        "controllerDigest": authority["controllerDigest"],
        "expiresUnixMillis": int(time.time() * 1000) + (budget_seconds + 60) * 1000,
        "gate": gate,
        "nonce": secrets.token_hex(32),
        "planDigest": authority["planDigest"],
        "schema": "codeclew-stabilization-capability/1.0",
    }
    value = dict(payload)
    value["signature"] = hmac.new(private_secret(), canonical(payload), hashlib.sha256).hexdigest()
    path = state_root() / "capabilities" / f"{payload['nonce']}.json"
    atomic_private_write(path, canonical(value) + b"\n", mode=0o600)
    return path


def consume_capability(path: Path, gate: str, authority: dict[str, str]) -> None:
    if not path.is_absolute() or ".." in path.parts:
        raise ControlError("gate capability path is unsafe")
    metadata = path.stat()
    if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o600:
        raise ControlError("gate capability permissions are invalid")
    value = load_json(path)
    if not isinstance(value, dict) or set(value) != {
        "controllerDigest",
        "expiresUnixMillis",
        "gate",
        "nonce",
        "planDigest",
        "schema",
        "signature",
    }:
        raise ControlError("gate capability schema is invalid")
    signature = value.pop("signature")
    expected = hmac.new(private_secret(), canonical(value), hashlib.sha256).hexdigest()
    if not isinstance(signature, str) or not hmac.compare_digest(signature, expected):
        raise ControlError("gate capability signature is invalid")
    if value["gate"] != gate or value["planDigest"] != authority["planDigest"] or value["controllerDigest"] != authority["controllerDigest"]:
        raise ControlError("gate capability authority mismatch")
    if not isinstance(value["expiresUnixMillis"], int) or value["expiresUnixMillis"] < int(time.time() * 1000):
        raise ControlError("gate capability expired")
    used = path.with_suffix(".used")
    os.replace(path, used)


def file_digest(stream) -> str:
    stream.seek(0)
    hasher = hashlib.sha256()
    while True:
        block = stream.read(1024 * 1024)
        if not block:
            break
        hasher.update(block)
    return "sha256:" + hasher.hexdigest()


def invoke(check: dict[str, object], tier: dict[str, object], authority: dict[str, str]) -> tuple[int, int, str, str]:
    environment = dict(os.environ)
    capability: Path | None = None
    if check["gate"] is not None:
        capability = issue_capability(authority, check["gate"], tier["budgetSeconds"])
        environment["CODECLEW_PLAN_CAPABILITY"] = str(capability)
    started = time.monotonic_ns()
    try:
        with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
            try:
                process = subprocess.Popen(
                    check["command"],
                    cwd=ROOT,
                    env=environment,
                    stdin=subprocess.DEVNULL,
                    stdout=stdout,
                    stderr=stderr,
                    start_new_session=True,
                )
            except FileNotFoundError:
                exit_code = 127
            else:
                try:
                    exit_code = process.wait(timeout=tier["budgetSeconds"])
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait()
                    exit_code = 124
                except BaseException:
                    try:
                        os.killpg(process.pid, signal.SIGTERM)
                        process.wait(timeout=5)
                    except (ProcessLookupError, subprocess.TimeoutExpired):
                        try:
                            os.killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        process.wait()
                    raise
            duration = (time.monotonic_ns() - started) // 1_000_000
            stdout_digest = file_digest(stdout)
            stderr_digest = file_digest(stderr)
        return exit_code, duration, stdout_digest, stderr_digest
    finally:
        if capability is not None:
            for candidate in (capability, capability.with_suffix(".used")):
                try:
                    candidate.unlink()
                except FileNotFoundError:
                    pass


def verified_receipt(
    plan: dict[str, object],
    authority: dict[str, str],
    check: dict[str, object],
    tier: dict[str, object],
    input_authority: str,
    dynamic_authority_before: str,
) -> dict[str, object]:
    def require_unchanged_authority() -> None:
        try:
            current_plan = load_json(PLAN_PATH)
        except (OSError, json.JSONDecodeError) as error:
            raise ControlError("stabilization plan changed during execution") from error
        current_authority = authorities(current_plan)
        if current_authority != authority:
            raise ControlError("controller, plan, or verifier changed during execution")
        current_dynamic = dynamic_authority_digest(check, refresh=True)
        if current_dynamic != dynamic_authority_before:
            raise ControlError("dynamic check authority changed during execution")
        if (
            evidence_digest(
                check, current_authority, dynamic_authority=current_dynamic
            )
            != input_authority
        ):
            raise ControlError("check input or repository authority changed during execution")

    physical = physical_cores()
    memory = memory_bytes()
    clean = is_clean()
    qualified = physical >= tier["minimumPhysicalCores"] and memory >= tier["minimumMemoryBytes"]
    if not qualified or (tier["cleanRequired"] and not clean):
        exit_code, duration, stdout_digest, stderr_digest = 0, 0, digest_bytes(b""), digest_bytes(b"")
    else:
        exit_code, duration, stdout_digest, stderr_digest = invoke(check, tier, authority)
    require_unchanged_authority()
    request = {
        "checkId": check["id"],
        "clean": clean,
        "command": check["command"],
        "commandDigest": digest_bytes(canonical(check["command"])),
        "controllerDigest": authority["controllerDigest"],
        "durationMillis": duration,
        "environmentDigest": environment_digest(check),
        "exitCode": exit_code,
        "inputDigest": input_authority,
        "memoryBytes": memory,
        "physicalCores": physical,
        "planDigest": authority["planDigest"],
        "sourceRevision": git("rev-parse", "HEAD"),
        "stderrDigest": stderr_digest,
        "stdoutDigest": stdout_digest,
        "stepId": check["step"],
        "tier": check["tier"],
        "verifierDigest": authority["verifierDigest"],
    }
    with tempfile.NamedTemporaryFile(dir=state_root(), mode="wb", delete=False) as stream:
        request_path = Path(stream.name)
        stream.write(canonical(request) + b"\n")
    os.chmod(request_path, 0o600)
    try:
        completed = subprocess.run(
            (sys.executable, "-I", "-S", str(VERIFIER), "--request", str(request_path)),
            cwd=ROOT,
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        if completed.returncode != 0:
            raise ControlError("independent verifier rejected the check evidence")
        value = json.loads(completed.stdout)
        if not isinstance(value, dict):
            raise ControlError("verifier returned an invalid receipt")
        require_unchanged_authority()
        return value
    finally:
        request_path.unlink(missing_ok=True)


def run_check(
    plan: dict[str, object],
    model: dict[str, object],
    authority: dict[str, str],
    step: str,
    check_id: str,
) -> dict[str, object]:
    if step not in model["steps"] or check_id not in model["checks"]:
        raise ControlError("unknown step or check")
    check = model["checks"][check_id]
    if check["step"] != step:
        raise ControlError("check does not belong to the requested step")
    require_dependencies(model, authority, step)
    tier = model["tiers"][check["tier"]]
    # Provider preparation and the evidence-bearing check share writable native
    # caches. The process-independent lock spans their complete lifecycle and is
    # deliberately outside the plan-digest namespace so another controller
    # version cannot prime those caches concurrently.
    with global_exclusive_lock(f"check-lifecycle-{check_id}"):
        return run_check_lifecycle(
            plan, model, authority, step, check_id, check, tier
        )


def run_check_lifecycle(
    plan: dict[str, object],
    model: dict[str, object],
    authority: dict[str, str],
    step: str,
    check_id: str,
    check: dict[str, object],
    tier: dict[str, object],
) -> dict[str, object]:
    qualified = (
        physical_cores() >= tier["minimumPhysicalCores"]
        and memory_bytes() >= tier["minimumMemoryBytes"]
        and (not tier["cleanRequired"] or is_clean())
    )
    preparation = check.get("prepare")
    if preparation is not None and qualified:
        prepared_check = {
            **preparation,
            "id": f"{check_id}-prepare",
            "step": step,
            "tier": check["tier"],
        }
        preparation_dynamic = dynamic_authority_digest(
            prepared_check, refresh=True
        )
        preparation_evidence = evidence_digest(
            prepared_check,
            authority,
            dynamic_authority=preparation_dynamic,
        )
        failed_path = preparation_failure_path(
            authority, check_id, preparation_evidence
        )
        if failed_path.exists():
            failed = load_json(failed_path)
            if (
                not isinstance(failed, dict)
                or failed.get("status") != "FAIL"
                or failed.get("inputDigest") != preparation_evidence
                or failed.get("checkId") != check_id
                or not has_valid_embedded_digest(failed, "attemptDigest")
                or not all(failed.get(key) == authority[key] for key in authority)
            ):
                raise ControlError("preparation failure marker is corrupt")
            raise ControlError(
                "blind retry refused for the same failed preparation evidence key"
            )
        exit_code: int | None = None
        try:
            exit_code, _duration, _stdout, _stderr = invoke(
                prepared_check, tier, authority
            )
            if exit_code != 0:
                raise ControlError("check preparation failed")
            try:
                current_plan = load_json(PLAN_PATH)
            except (OSError, json.JSONDecodeError) as error:
                raise ControlError("stabilization plan changed during preparation") from error
            if authorities(current_plan) != authority:
                raise ControlError("controller, plan, or verifier changed during preparation")
            current_dynamic = dynamic_authority_digest(
                prepared_check, refresh=True
            )
            if current_dynamic != preparation_dynamic:
                raise ControlError("preparation authority changed during execution")
            if evidence_digest(
                prepared_check,
                authority,
                dynamic_authority=current_dynamic,
            ) != preparation_evidence:
                raise ControlError("preparation input or repository authority changed during execution")
        except BaseException:
            failed = {
                **authority,
                "checkId": check_id,
                "exitCode": exit_code,
                "inputDigest": preparation_evidence,
                "schema": "codeclew-stabilization-preparation-attempt/1.0",
                "status": "FAIL",
            }
            failed["attemptDigest"] = digest_bytes(canonical(failed))
            atomic_private_write(failed_path, canonical(failed) + b"\n")
            raise
    dynamic_authority = dynamic_authority_digest(check, refresh=True)
    input_authority = evidence_digest(
        check, authority, dynamic_authority=dynamic_authority
    )
    path = receipt_path(authority, check_id, input_authority)
    with exclusive_lock(authority, f"check-{check_id}-{input_authority.split(':', 1)[1]}"):
        if path.exists():
            existing = load_json(path)
            if (
                isinstance(existing, dict)
                and existing.get("status") == "PASS"
                and has_valid_embedded_digest(existing, "receiptDigest")
                and all(existing.get(key) == authority[key] for key in authority)
            ):
                if dynamic_authority_digest(check, refresh=True) != dynamic_authority:
                    raise ControlError("dynamic check authority changed during receipt reuse")
                return {"checkId": check_id, "reused": True, "status": "PASS"}
            raise ControlError("blind retry refused for the same failed evidence key")
        receipt = verified_receipt(
            plan,
            authority,
            check,
            tier,
            input_authority,
            dynamic_authority,
        )
        atomic_private_write(path, canonical(receipt) + b"\n")
        return {"checkId": check_id, "reused": False, "status": receipt["status"]}


def seal_step(model: dict[str, object], authority: dict[str, str], step: str) -> dict[str, object]:
    if step not in model["steps"]:
        raise ControlError("unknown step")
    require_dependencies(model, authority, step)
    receipt_digests = []
    check_authorities = {}
    dynamic_authorities = {}
    for check_id in model["steps"][step]["requiredChecks"]:
        check = model["checks"][check_id]
        dynamic = dynamic_authority_digest(check, refresh=True)
        dynamic_authorities[check_id] = dynamic
        evidence = evidence_digest(check, authority, dynamic_authority=dynamic)
        check_authorities[check_id] = check_authority_digest(
            check, authority, dynamic_authority=dynamic
        )
        path = receipt_path(authority, check_id, evidence)
        if not path.exists():
            raise ControlError("required check has no receipt: " + check_id)
        receipt = load_json(path)
        if not isinstance(receipt, dict) or receipt.get("status") != "PASS":
            raise ControlError("required check did not pass: " + check_id)
        if not has_valid_embedded_digest(receipt, "receiptDigest"):
            raise ControlError("required check receipt integrity failed")
        if any(receipt.get(key) != authority[key] for key in authority):
            raise ControlError("required check authority is stale")
        if receipt.get("inputDigest") != evidence:
            raise ControlError("required check input authority is stale")
        receipt_digests.append(receipt["receiptDigest"])
    for check_id, expected in dynamic_authorities.items():
        if dynamic_authority_digest(
            model["checks"][check_id], refresh=True
        ) != expected:
            raise ControlError("dynamic check authority changed while sealing step")
    with exclusive_lock(authority, f"completion-{step}"):
        for check_id, expected in dynamic_authorities.items():
            if dynamic_authority_digest(
                model["checks"][check_id], refresh=True
            ) != expected:
                raise ControlError("dynamic check authority changed before completion write")
        completion: dict[str, object] = {
            **authority,
            "checkAuthorities": check_authorities,
            "receiptDigests": sorted(receipt_digests),
            "schema": "codeclew-stabilization-step-completion/1.0",
            "sourceRevision": git("rev-parse", "HEAD"),
            "status": "COMPLETE",
            "stepId": step,
        }
        completion["completionDigest"] = digest_bytes(canonical(completion))
        atomic_private_write(completion_path(authority, step), canonical(completion) + b"\n")
    return {"status": "COMPLETE", "stepId": step}


def completed_steps(
    model: dict[str, object], authority: dict[str, str]
) -> list[str]:
    completed: list[str] = []
    for step in model["order"]:
        if all(
            dependency in completed
            for dependency in model["steps"][step]["dependencies"]
        ) and valid_completion(model, authority, step):
            completed.append(step)
    return completed


def status(model: dict[str, object], authority: dict[str, str]) -> dict[str, object]:
    completed = completed_steps(model, authority)
    next_step = next((step for step in model["order"] if step not in completed and all(dependency in completed for dependency in model["steps"][step]["dependencies"])), None)
    return {
        "completed": completed,
        "nextStep": next_step,
        "planDigest": authority["planDigest"],
        "schema": "codeclew-stabilization-status/1.0",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("validate")
    subparsers.add_parser("status")
    run_parser = subparsers.add_parser("run")
    run_parser.add_argument("--step", required=True)
    run_parser.add_argument("--check", required=True)
    seal_parser = subparsers.add_parser("seal")
    seal_parser.add_argument("--step", required=True)
    guard_parser = subparsers.add_parser("guard")
    guard_parser.add_argument("--gate", required=True)
    arguments = parser.parse_args()
    try:
        plan = load_json(PLAN_PATH)
        assert isinstance(plan, dict)
        model = validate_plan(plan)
        authority = authorities(plan)
        if arguments.command == "validate":
            result = {"planDigest": authority["planDigest"], "schema": "codeclew-stabilization-plan-validation/1.0", "status": "PASS"}
        elif arguments.command == "status":
            result = status(model, authority)
        elif arguments.command == "run":
            result = run_check(plan, model, authority, arguments.step, arguments.check)
        elif arguments.command == "seal":
            result = seal_step(model, authority, arguments.step)
        else:
            capability = os.environ.get("CODECLEW_PLAN_CAPABILITY")
            if not capability:
                raise ControlError("direct expensive gate execution is forbidden")
            consume_capability(Path(capability), arguments.gate, authority)
            result = {"gate": arguments.gate, "schema": "codeclew-stabilization-gate-admission/1.0", "status": "ADMITTED"}
    except (AssertionError, ControlError, json.JSONDecodeError, OSError, subprocess.SubprocessError, ValueError) as error:
        print(canonical({"error": str(error), "schema": "codeclew-stabilization-control-error/1.0"}).decode("utf-8"))
        return 2
    print(canonical(result).decode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
