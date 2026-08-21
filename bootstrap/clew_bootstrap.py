#!/usr/bin/env python3
"""Build or reuse one immutable Codeclew runtime capsule, then execute it."""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import uuid


SCHEMA = "codeclew-runtime-capsule/2.0"
DOMAIN = b"codeclew-runtime/v2\0"
MAX_MANIFEST_BYTES = 1024 * 1024
WORKERS = {
    "kotlin21": ("2.1.21", "workers/kotlin21/build/install/kotlin21", "workers/manifests/kotlin21.json"),
    "kotlin23": ("2.3.0", "workers/kotlin23/build/install/kotlin23", "workers/manifests/kotlin23.json"),
    "kotlin24": ("2.4.10", "workers/kotlin/build/install/kotlin", "workers/manifests/kotlin24.json"),
}
ROOT_FILES = {
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "build.gradle.kts",
    "settings.gradle.kts", "gradlew", "gradlew.bat", "clew",
}
INJECTION_ENV = {
    "RUSTC", "RUSTC_WRAPPER", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS",
    "JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "_JAVA_OPTIONS", "GRADLE_OPTS",
    "PYTHONPATH", "PYTHONHOME", "PYTHONSTARTUP", "PYTHONINSPECT",
    "PYTHONWARNINGS", "PYTHONSAFEPATH",
}


class BootstrapError(RuntimeError):
    pass


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def digest_bytes(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def digest_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return "sha256:" + digest.hexdigest()


def run(arguments: list[str], cwd: Path, environment: dict[str, str] | None = None) -> bytes:
    completed = subprocess.run(
        arguments,
        cwd=cwd,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=None,
        check=False,
    )
    if completed.returncode != 0:
        raise BootstrapError(f"bootstrap command failed ({completed.returncode}): {arguments[0]}")
    return completed.stdout


def progress(event: str, stage: str, **values: object) -> None:
    print(canonical({
        "schema": "codeclew-capsule-progress/2.0",
        "event": event,
        "stage": stage,
        **values,
    }).decode(), file=sys.stderr, flush=True)


def run_build_stage(
    arguments: list[str],
    cwd: Path,
    environment: dict[str, str],
    stage: str,
) -> None:
    progress("STAGE_STARTED", stage)
    started = time.monotonic()
    process = subprocess.Popen(
        arguments,
        cwd=cwd,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=None,
        start_new_session=True,
    )
    next_heartbeat = started + 5
    while True:
        return_code = process.poll()
        if return_code is not None:
            break
        now = time.monotonic()
        if now >= next_heartbeat:
            progress(
                "HEARTBEAT",
                stage,
                durationMillis=int((now - started) * 1000),
            )
            next_heartbeat = now + 5
        time.sleep(0.25)
    duration = int((time.monotonic() - started) * 1000)
    if return_code != 0:
        progress("STAGE_FAILED", stage, durationMillis=duration, exitCode=return_code)
        raise BootstrapError(f"capsule build stage failed ({return_code}): {stage}")
    progress("STAGE_COMPLETED", stage, durationMillis=duration)


def selected_source(relative: str) -> bool:
    if relative in ROOT_FILES:
        return True
    if relative.startswith("bootstrap/"):
        return relative.endswith(".py")
    if relative.startswith("schemas/"):
        return True
    if relative.startswith("gradle/wrapper/"):
        return True
    if relative.startswith(".cargo/"):
        return True
    if relative.startswith("crates/"):
        parts = relative.split("/")
        if "tests" in parts or "examples" in parts or "target" in parts:
            return False
        return parts[-1] in {"Cargo.toml", "build.rs"} or "/src/" in relative
    if relative.startswith("workers/manifests/"):
        return relative.endswith(".json")
    if relative.startswith("workers/"):
        parts = relative.split("/")
        if len(parts) == 3 and parts[-1] == "build.gradle.kts":
            return True
        return "/src/main/" in relative
    return False


def source_manifest(source: Path) -> tuple[list[dict[str, object]], bool]:
    tracked = run(["git", "ls-files", "-z"], source).split(b"\0")
    untracked = run(["git", "ls-files", "--others", "--exclude-standard", "-z"], source).split(b"\0")
    paths = sorted({row.decode() for row in [*tracked, *untracked] if row and selected_source(row.decode())})
    if not paths:
        raise BootstrapError("runtime input closure is empty")
    rows: list[dict[str, object]] = []
    for relative in paths:
        path = source / relative
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
            raise BootstrapError(f"runtime input is not a regular file: {relative}")
        rows.append({
            "path": relative,
            "size": metadata.st_size,
            "mode": metadata.st_mode & 0o111,
            "sha256": digest_file(path),
        })
    dirty_output = run(["git", "status", "--porcelain=v1", "--untracked-files=all"], source).decode()
    dirty_paths = set()
    for line in dirty_output.splitlines():
        value = line[3:]
        if " -> " in value:
            value = value.split(" -> ", 1)[1]
        dirty_paths.add(value.strip('"'))
    development = any(selected_source(path) for path in dirty_paths)
    return rows, development


def verify_source_manifest(source: Path, rows: list[dict[str, object]]) -> None:
    for row in rows:
        path = source / str(row["path"])
        metadata = path.lstat()
        if (
            stat.S_ISLNK(metadata.st_mode)
            or not stat.S_ISREG(metadata.st_mode)
            or metadata.st_size != row["size"]
            or metadata.st_mode & 0o111 != row["mode"]
            or digest_file(path) != row["sha256"]
        ):
            raise BootstrapError(f"runtime input changed during bootstrap: {row['path']}")


def state_root() -> Path:
    explicit = os.environ.get("CODECLEW_HOME")
    if explicit:
        root = Path(explicit)
    elif os.environ.get("XDG_CACHE_HOME"):
        root = Path(os.environ["XDG_CACHE_HOME"]) / "codeclew"
    else:
        root = Path.home() / ".cache" / "codeclew"
    if not root.is_absolute() or ".." in root.parts:
        raise BootstrapError("Codeclew state root must be normalized and absolute")
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    metadata = root.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise BootstrapError("Codeclew state root is unsafe")
    if metadata.st_uid != os.geteuid():
        raise BootstrapError("Codeclew state root has a different owner")
    os.chmod(root, 0o700)
    root = root / "v2"
    root.mkdir(mode=0o700, exist_ok=True)
    os.chmod(root, 0o700)
    for child in [
        "runtimes", "repos", "sessions", "runs", "locks", "tmp", "quarantine",
        "objects", "objects/sha256", "objects/packs", "generations", "attempts", "gc",
        "build-cache", "build-cache/cargo",
    ]:
        path = root / child
        path.mkdir(mode=0o700, exist_ok=True)
        child_metadata = path.lstat()
        if stat.S_ISLNK(child_metadata.st_mode) or not stat.S_ISDIR(child_metadata.st_mode):
            raise BootstrapError(f"Codeclew state child is unsafe: {child}")
        os.chmod(path, 0o700)
    return root.resolve(strict=True)


def toolchain_authority(source: Path) -> dict[str, object]:
    python_executable = Path(sys.executable).resolve(strict=True)
    rustc = run(["rustc", "-Vv"], source).decode().strip()
    cargo = run(["cargo", "-V"], source).decode().strip()
    java_home = Path(os.environ.get("JAVA_HOME", ""))
    if not java_home.is_absolute():
        java_binary = shutil.which("java")
        if not java_binary:
            raise BootstrapError("JDK 21 is unavailable")
        java_home = Path(java_binary).resolve(strict=True).parent.parent
    java_files = [java_home / "release", java_home / "bin/java", java_home / "lib/modules"]
    if not all(path.is_file() and not path.is_symlink() for path in java_files[:2]):
        raise BootstrapError("JDK authority files are unavailable")
    java_release = (java_home / "release").read_text(errors="strict")
    return {
        "python": {
            "implementation": platform.python_implementation(),
            "version": platform.python_version(),
            "executableSha256": digest_file(python_executable),
        },
        "rust": {"rustcVv": digest_bytes(rustc.encode()), "cargoVersion": cargo},
        "jdk": {
            "releaseSha256": digest_bytes(java_release.encode()),
            "javaSha256": digest_file(java_home / "bin/java"),
            "modulesSha256": digest_file(java_home / "lib/modules") if (java_home / "lib/modules").is_file() else None,
        },
        "platform": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "libc": platform.libc_ver(),
        },
    }


def fast_toolchain_locator_authority() -> dict[str, object]:
    python_executable = Path(sys.executable).resolve(strict=True)
    resolved = {}
    for name in ["rustc", "cargo", "java"]:
        executable = shutil.which(name)
        if not executable:
            raise BootstrapError(f"{name} is unavailable")
        path = Path(executable).resolve(strict=True)
        resolved[name] = {
            "path": str(path),
            "size": path.stat().st_size,
            "sha256": digest_file(path),
        }
    java_home = Path(os.environ.get("JAVA_HOME", ""))
    if not java_home.is_absolute():
        java_home = Path(resolved["java"]["path"]).parent.parent
    release = java_home / "release"
    if not release.is_file() or release.is_symlink():
        raise BootstrapError("JDK release authority is unavailable")
    return {
        "python": {
            "implementation": platform.python_implementation(),
            "version": platform.python_version(),
            "path": str(python_executable),
            "sha256": digest_file(python_executable),
        },
        "executables": resolved,
        "jdkReleaseSha256": digest_file(release),
        "platform": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "libc": platform.libc_ver(),
        },
    }


def locator_key(mode: str, inputs: list[dict[str, object]], fast_tools: dict[str, object]) -> str:
    return digest_bytes(DOMAIN + b"locator\0" + mode.encode() + b"\0" + canonical({
        "inputs": inputs,
        "toolchains": fast_tools,
    }))


def locator_path(root: Path, locator: str) -> Path:
    directory = root / "runtimes" / "locators"
    directory.mkdir(mode=0o700, exist_ok=True)
    os.chmod(directory, 0o700)
    return directory / (locator.removeprefix("sha256:") + ".json")


def read_locator(path: Path, expected: str) -> str | None:
    if not path.exists():
        return None
    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode) or metadata.st_size > 4096:
        raise BootstrapError("runtime locator is unsafe")
    value = json.loads(path.read_bytes())
    if (
        value.get("schema") != "codeclew-runtime-locator/2.0"
        or value.get("locatorKey") != expected
        or not isinstance(value.get("runtimeKey"), str)
        or not value["runtimeKey"].startswith("sha256:")
    ):
        raise BootstrapError("runtime locator authority mismatch")
    return value["runtimeKey"]


def write_locator(path: Path, locator: str, runtime: str) -> None:
    temporary = path.parent / f".locator-{uuid.uuid4().hex}"
    descriptor = os.open(temporary, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    try:
        with os.fdopen(descriptor, "wb", closefd=False) as stream:
            stream.write(canonical({
                "schema": "codeclew-runtime-locator/2.0",
                "locatorKey": locator,
                "runtimeKey": runtime,
            }) + b"\n")
            stream.flush()
            os.fsync(stream.fileno())
    finally:
        os.close(descriptor)
    os.replace(temporary, path)


def runtime_key(mode: str, inputs: list[dict[str, object]], tools: dict[str, object]) -> str:
    digest = hashlib.sha256()
    digest.update(DOMAIN)
    digest.update(mode.encode())
    digest.update(b"\0")
    digest.update(canonical({"inputs": inputs, "toolchains": tools}))
    return "sha256:" + digest.hexdigest()


def stage_inputs(source: Path, destination: Path, rows: list[dict[str, object]]) -> None:
    destination.mkdir(mode=0o700)

    def copy(row: dict[str, object]) -> None:
        relative = Path(str(row["path"]))
        target = destination / relative
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        shutil.copyfile(source / relative, target, follow_symlinks=False)
        os.chmod(target, 0o755 if row["mode"] else 0o600)

    with ThreadPoolExecutor(max_workers=min(8, max(1, os.cpu_count() or 1))) as executor:
        list(executor.map(copy, rows))
    verify_source_manifest(destination, rows)


def build_environment(stage: Path, root: Path) -> dict[str, str]:
    environment = {name: value for name, value in os.environ.items() if name not in INJECTION_ENV and not name.startswith("CODECLEW_")}
    cargo_cache = root / "build-cache/cargo"
    cargo_cache.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(cargo_cache, 0o700)
    environment["CARGO_TARGET_DIR"] = str(cargo_cache)
    environment["CARGO_INCREMENTAL"] = "1"
    environment["GIT_TERMINAL_PROMPT"] = "0"
    gradle_home = Path(environment.get("GRADLE_USER_HOME", str(Path.home() / ".gradle")))
    for relative in ["init.gradle", "init.gradle.kts", "init.d"]:
        if (gradle_home / relative).exists():
            raise BootstrapError("Gradle init injection is unsupported for trusted capsule builds")
    return environment


def file_rows(root: Path) -> list[dict[str, object]]:
    rows = []
    for path in sorted(path for path in root.rglob("*") if path.is_file()):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
            raise BootstrapError("capsule output contains an unsafe file")
        rows.append({
            "path": path.relative_to(root).as_posix(),
            "size": metadata.st_size,
            "sha256": digest_file(path),
        })
    return rows


def tree_hash(rows: list[dict[str, object]]) -> str:
    digest = hashlib.sha256()
    for row in rows:
        digest.update(str(row["path"]).encode())
        digest.update(b"\0")
        digest.update(str(row["size"]).encode())
        digest.update(b"\0")
        digest.update(str(row["sha256"]).encode())
        digest.update(b"\0")
    return "sha256:" + digest.hexdigest()


def bootstrap_self_test() -> None:
    rows = [
        {"path": "bin/a", "size": 3, "sha256": "sha256:" + "0" * 64},
        {"path": "lib/b", "size": 5, "sha256": "sha256:" + "1" * 64},
    ]
    assert tree_hash(rows) == "sha256:17991e194c0c77b4a7ff59263df0339e2a26c7e8bc5556e11a3afeb2510c6177"
    first = locator_key("RELEASE", rows, {"tool": "a"})
    assert first == locator_key("RELEASE", rows, {"tool": "a"})
    assert first != locator_key("DEVELOPMENT", rows, {"tool": "a"})
    assert first != locator_key("RELEASE", rows, {"tool": "b"})
    assert runtime_build_plan(8, 32 * 1024**3)["parallel"] is True
    assert runtime_build_plan(8, 4 * 1024**3)["parallel"] is False
    assert warm_audit_payload(False, False)["status"] == "PASSED"
    assert warm_audit_payload(True, False)["status"] == "COLD_MISS"
    assert warm_audit_payload(False, True)["status"] == "COLD_MISS"
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        staged = root / "staged"
        destination = root / "published"
        (staged / "bin").mkdir(parents=True)
        executable = staged / "bin" / "clew"
        executable.write_bytes(b"binary")
        os.chmod(executable, 0o700)
        os.replace(staged, destination)
        seal_capsule(destination)
        verify_sealed_capsule(destination)
        (root / "quarantine").mkdir(mode=0o700)
        published_executable = destination / "bin" / "clew"
        os.chmod(published_executable, 0o700)
        with published_executable.open("ab") as stream:
            stream.write(b"corrupt")
        quarantine(root, destination, "SelfTestCorruption")
        assert not destination.exists()
        quarantined = [path for path in (root / "quarantine").iterdir() if path.is_dir()]
        assert len(quarantined) == 1
        metadata = quarantined[0].with_suffix(".json")
        assert metadata.is_file() and stat.S_IMODE(metadata.stat().st_mode) == 0o600


def warm_audit_payload(toolchain_invoked: bool, capsule_build_invoked: bool) -> dict[str, object]:
    return {
        "schema": "codeclew-bootstrap-warm-audit/2.0",
        "status": "PASSED" if not toolchain_invoked and not capsule_build_invoked else "COLD_MISS",
        "coldToolchainInvoked": toolchain_invoked,
        "capsuleBuildInvoked": capsule_build_invoked,
        "forbiddenWarmProcesses": ["cargo", "rustc", "gradle", "maven"],
    }


def verify_release_worker(stage: Path, manifest_relative: str, rows: list[dict[str, object]]) -> None:
    manifest = json.loads((stage / manifest_relative).read_text())
    expected = manifest.get("files")
    if expected != rows or manifest.get("treeHash") != tree_hash(rows):
        raise BootstrapError(
            "RELEASE worker differs from its committed manifest; regenerate and verify all "
            f"affected worker variants: {manifest_relative}"
        )


def runtime_build_plan(cpu_count: int, total_memory_bytes: int) -> dict[str, object]:
    cpu_count = max(1, cpu_count)
    reserved = max(1024**3, total_memory_bytes * 15 // 100)
    memory_budget = max(0, total_memory_bytes - reserved) * 70 // 100
    cargo_memory = 2 * 1024**3
    gradle_memory = 3 * 1024**3
    parallel = cpu_count >= 2 and memory_budget >= cargo_memory + gradle_memory
    if parallel:
        cargo_workers = max(1, cpu_count // 2)
        gradle_workers = max(1, cpu_count - cargo_workers)
    else:
        cargo_workers = cpu_count
        gradle_workers = cpu_count
    return {
        "parallel": parallel,
        "cargoWorkers": cargo_workers,
        "gradleWorkers": gradle_workers,
        "memoryBudgetBytes": memory_budget,
    }


def host_memory_bytes() -> int:
    try:
        pages = os.sysconf("SC_PHYS_PAGES")
        page_size = os.sysconf("SC_PAGE_SIZE")
        if pages > 0 and page_size > 0:
            return int(pages * page_size)
    except (AttributeError, OSError, ValueError):
        pass
    raise BootstrapError("host memory authority is unavailable for capsule admission")


def build_toolchains(stage: Path, environment: dict[str, str]) -> dict[str, object]:
    plan = runtime_build_plan(os.cpu_count() or 1, host_memory_bytes())
    gradle = [
        str(stage / "gradlew"),
        ":workers:kotlin21:installDist",
        ":workers:kotlin23:installDist",
        ":workers:kotlin:installDist",
        "--no-daemon",
        "--parallel",
        "--build-cache",
        f"--max-workers={plan['gradleWorkers']}",
        "--quiet",
    ]
    cargo = [
        "cargo", "build", "--locked", "--release", "-p", "clew",
        "--bin", "clew", "--bin", "semanticd", "--jobs", str(plan["cargoWorkers"]),
    ]
    if plan["parallel"]:
        with ThreadPoolExecutor(max_workers=2) as executor:
            futures = [
                executor.submit(run_build_stage, gradle, stage, environment, "GRADLE_WORKERS"),
                executor.submit(run_build_stage, cargo, stage, environment, "CARGO_BINARIES"),
            ]
            for future in futures:
                future.result()
    else:
        run_build_stage(gradle, stage, environment, "GRADLE_WORKERS")
        run_build_stage(cargo, stage, environment, "CARGO_BINARIES")
    return plan


def package_worker(
    stage: Path,
    capsule: Path,
    mode: str,
    item: tuple[str, tuple[str, str, str]],
) -> tuple[str, dict[str, object]]:
    name, (compiler, distribution, manifest) = item
    source_distribution = stage / distribution
    destination = capsule / distribution
    shutil.copytree(source_distribution, destination, symlinks=False)
    rows = file_rows(destination)
    if mode == "RELEASE":
        verify_release_worker(stage, manifest, rows)
    return name, {
        "compilerVersion": compiler,
        "distribution": distribution,
        "treeHash": tree_hash(rows),
        "files": rows,
    }


def seal_capsule(capsule: Path) -> None:
    for path in sorted(capsule.rglob("*"), key=lambda value: len(value.parts), reverse=True):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise BootstrapError("capsule output contains a symlink")
        if stat.S_ISREG(metadata.st_mode):
            os.chmod(path, 0o500 if metadata.st_mode & 0o111 else 0o400)
        elif stat.S_ISDIR(metadata.st_mode):
            os.chmod(path, 0o500)
        else:
            raise BootstrapError("capsule output contains an unsupported filesystem entry")
    os.chmod(capsule, 0o500)


def verify_sealed_capsule(capsule: Path) -> None:
    for path in [capsule, *capsule.rglob("*")]:
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise BootstrapError("sealed capsule contains a symlink")
        if stat.S_ISDIR(metadata.st_mode):
            if metadata.st_mode & 0o277:
                raise BootstrapError("capsule directory is not sealed read-only")
        elif stat.S_ISREG(metadata.st_mode):
            if metadata.st_mode & 0o277:
                raise BootstrapError("capsule file is not sealed read-only")
        else:
            raise BootstrapError("sealed capsule contains an unsupported entry")


def build_capsule(source: Path, root: Path, key: str, mode: str, inputs: list[dict[str, object]], tools: dict[str, object]) -> Path:
    temporary = Path(tempfile.mkdtemp(prefix="capsule-build-", dir=root / "tmp"))
    stage = temporary / "source"
    capsule = temporary / "capsule"
    try:
        stage_inputs(source, stage, inputs)
        environment = build_environment(stage, root)
        build_cache_lock = root / "locks/build-cache.lock"
        with build_cache_lock.open("a+b") as lock:
            os.chmod(build_cache_lock, 0o600)
            fcntl.flock(lock, fcntl.LOCK_EX)
            build_toolchains(stage, environment)
        verify_source_manifest(stage, inputs)
        (capsule / "bin").mkdir(mode=0o700, parents=True)
        cargo_target = Path(environment["CARGO_TARGET_DIR"]) / "release"
        for name in ["clew", "semanticd"]:
            shutil.copy2(cargo_target / name, capsule / "bin" / name)
            os.chmod(capsule / "bin" / name, 0o500)
        progress("STAGE_STARTED", "PACKAGE_AND_HASH_WORKERS")
        with ThreadPoolExecutor(max_workers=len(WORKERS)) as executor:
            packaged = executor.map(
                lambda item: package_worker(stage, capsule, mode, item),
                WORKERS.items(),
            )
            workers = dict(packaged)
        progress("STAGE_COMPLETED", "PACKAGE_AND_HASH_WORKERS")
        artifacts = {}
        for name in ["clew", "semanticd"]:
            path = capsule / "bin" / name
            artifacts[name] = {"path": f"bin/{name}", "size": path.stat().st_size, "sha256": digest_file(path)}
        manifest = {
            "schema": SCHEMA,
            "runtimeKey": key,
            "mode": mode,
            "manifestDigest": "",
            "artifacts": artifacts,
            "workers": workers,
        }
        manifest["manifestDigest"] = digest_bytes(canonical(manifest))
        (capsule / "runtime.json").write_bytes(canonical(manifest) + b"\n")
        (capsule / "READY").write_text(key + "\n")
        verify_source_manifest(source, inputs)
        verify_capsule(capsule, key, require_sealed=False)
        destination = root / "runtimes" / key.removeprefix("sha256:")
        if destination.exists():
            return destination
        os.replace(capsule, destination)
        try:
            seal_capsule(destination)
            verify_sealed_capsule(destination)
        except Exception as error:
            quarantine(root, destination, type(error).__name__)
            raise BootstrapError("published capsule could not be sealed") from error
        return destination
    finally:
        shutil.rmtree(temporary, ignore_errors=True)


def verify_capsule(path: Path, key: str, *, require_sealed: bool = True) -> dict[str, object]:
    manifest_path = path / "runtime.json"
    if not manifest_path.is_file() or manifest_path.stat().st_size > MAX_MANIFEST_BYTES:
        raise BootstrapError("runtime manifest is unavailable")
    manifest = json.loads(manifest_path.read_bytes())
    expected = manifest.get("manifestDigest")
    manifest["manifestDigest"] = ""
    if (
        manifest.get("schema") != SCHEMA
        or expected != digest_bytes(canonical(manifest))
        or manifest.get("runtimeKey") != key
    ):
        raise BootstrapError("runtime manifest authority mismatch")
    manifest["manifestDigest"] = expected
    for artifact in manifest.get("artifacts", {}).values():
        target = path / artifact["path"]
        if (
            not target.is_file()
            or target.is_symlink()
            or target.stat().st_size != artifact["size"]
            or digest_file(target) != artifact["sha256"]
        ):
            raise BootstrapError("runtime executable authority mismatch")
    for worker in manifest.get("workers", {}).values():
        distribution = path / worker["distribution"]
        rows = file_rows(distribution)
        if rows != worker["files"] or tree_hash(rows) != worker["treeHash"]:
            raise BootstrapError("runtime worker authority mismatch")
    if require_sealed:
        verify_sealed_capsule(path)
    return manifest


def quarantine(root: Path, capsule: Path, reason: str) -> None:
    try:
        metadata = capsule.lstat()
    except FileNotFoundError:
        return
    reason_token = reason if reason.isascii() and reason.replace("_", "").isalnum() else "VerificationFailure"
    reason_token = reason_token[:64]
    directory = root / "quarantine"
    destination = directory / f"{capsule.name}-{uuid.uuid4().hex}"
    try:
        if stat.S_ISDIR(metadata.st_mode) and not stat.S_ISLNK(metadata.st_mode):
            # macOS refuses to rename a sealed directory whose own mode is 0500.
            # Only the capsule root is relaxed; quarantined contents stay sealed.
            os.chmod(capsule, 0o700)
        os.replace(capsule, destination)
        metadata_path = directory / f"{destination.name}.json"
        temporary = directory / f".quarantine-{uuid.uuid4().hex}"
        descriptor = os.open(temporary, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        try:
            with os.fdopen(descriptor, "wb", closefd=False) as stream:
                stream.write(canonical({
                    "schema": "codeclew-runtime-quarantine/2.0",
                    "reason": reason_token,
                }) + b"\n")
                stream.flush()
                os.fsync(stream.fileno())
        finally:
            os.close(descriptor)
        os.replace(temporary, metadata_path)
    except OSError as error:
        raise BootstrapError("unsafe runtime capsule could not be quarantined") from error


def main() -> int:
    if sys.version_info < (3, 11):
        raise BootstrapError("Codeclew requires Python 3.11 or newer")
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--source-root", type=Path, required=True)
    known, command = parser.parse_known_args()
    if command == ["--bootstrap-self-test"]:
        bootstrap_self_test()
        print(canonical({"schema": "codeclew-bootstrap-self-test/1.0", "status": "PASSED"}).decode())
        return 0
    warm_audit = command == ["--bootstrap-warm-audit"]
    source = known.source_root.resolve(strict=True)
    root = state_root()
    inputs, development = source_manifest(source)
    mode = "DEVELOPMENT" if development else "RELEASE"
    fast_tools = fast_toolchain_locator_authority()
    locator = locator_key(mode, inputs, fast_tools)
    path_to_locator = locator_path(root, locator)
    key = read_locator(path_to_locator, locator)
    tools = None
    if key is None:
        tools = toolchain_authority(source)
        key = runtime_key(mode, inputs, tools)
    cold_toolchain_invoked = tools is not None
    capsule_build_invoked = False
    capsule = root / "runtimes" / key.removeprefix("sha256:")
    lock_path = root / "locks" / f"runtime-{key.removeprefix('sha256:')}.lock"
    with lock_path.open("a+b") as lock:
        os.chmod(lock_path, 0o600)
        fcntl.flock(lock, fcntl.LOCK_EX)
        if capsule.exists():
            try:
                verify_capsule(capsule, key)
            except Exception as error:
                quarantine(root, capsule, type(error).__name__)
        if not capsule.exists():
            if tools is None:
                tools = toolchain_authority(source)
                cold_toolchain_invoked = True
                rebuilt_key = runtime_key(mode, inputs, tools)
                if rebuilt_key != key:
                    raise BootstrapError("runtime locator disagrees with cold toolchain authority")
            capsule_build_invoked = True
            capsule = build_capsule(source, root, key, mode, inputs, tools)
        verify_capsule(capsule, key)
        write_locator(path_to_locator, locator, key)
    if warm_audit:
        print(canonical(warm_audit_payload(cold_toolchain_invoked, capsule_build_invoked)).decode())
        return 0
    lease_path = root / "locks" / f"runtime-{key.removeprefix('sha256:')}.lease"
    lease = lease_path.open("a+b")
    os.chmod(lease_path, 0o600)
    fcntl.flock(lease, fcntl.LOCK_SH)
    os.set_inheritable(lease.fileno(), True)
    environment = {name: value for name, value in os.environ.items() if not name.startswith("CODECLEW_")}
    environment["CODECLEW_HOME"] = str(root.parent)
    environment["CODECLEW_RUNTIME_ROOT"] = str(capsule)
    environment["CODECLEW_RUNTIME_LEASE_FD"] = str(lease.fileno())
    os.execve(capsule / "bin/clew", [str(capsule / "bin/clew"), *command], environment)
    return 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except BootstrapError as error:
        print(canonical({"schema": "codeclew-bootstrap-error/1.0", "error": str(error)}).decode(), file=sys.stderr)
        raise SystemExit(7)
    except Exception as error:
        print(canonical({
            "schema": "codeclew-bootstrap-error/2.0",
            "error": "unexpected bootstrap failure",
            "errorType": type(error).__name__,
        }).decode(), file=sys.stderr)
        raise SystemExit(7)
