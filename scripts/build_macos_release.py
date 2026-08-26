#!/usr/bin/env python3
"""Build one self-contained, no-local-compilation macOS Codeclew release bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import subprocess
import tarfile
import tempfile


class ReleaseError(RuntimeError):
    pass


def canonical(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()


def sha256(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while block := stream.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def run(arguments: list[str], cwd: Path, *, environment: dict[str, str] | None = None) -> bytes:
    completed = subprocess.run(
        arguments,
        cwd=cwd,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise ReleaseError(f"release command failed: {arguments[0]}")
    return completed.stdout


def git_text(root: Path, *arguments: str) -> str:
    return run(["git", *arguments], root).decode().strip()


def make_removable(path: Path) -> None:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return
    if stat.S_ISLNK(metadata.st_mode):
        return
    if stat.S_ISDIR(metadata.st_mode):
        path.chmod(0o700)
        for child in path.iterdir():
            make_removable(child)
    elif stat.S_ISREG(metadata.st_mode):
        path.chmod(0o600)


def validate_tree(root: Path) -> None:
    for current, directories, files in os.walk(root, followlinks=False):
        current_path = Path(current)
        for name in [*directories, *files]:
            path = current_path / name
            if path.is_symlink():
                raise ReleaseError(f"release tree contains a symlink: {path.relative_to(root)}")
            metadata = path.lstat()
            if not (stat.S_ISDIR(metadata.st_mode) or stat.S_ISREG(metadata.st_mode)):
                raise ReleaseError("release tree contains an unsupported entry")


def build_seed(root: Path, work: Path, revision: str, source_tree: str) -> Path:
    runtime_home = work / "runtime-state"
    environment = dict(os.environ)
    environment["CODECLEW_HOME"] = str(runtime_home)
    completed = subprocess.run(
        [str(root / "clew"), "capabilities"],
        cwd=root,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0 or len(completed.stdout) > 1024 * 1024:
        raise ReleaseError("release runtime build failed")
    try:
        capabilities = json.loads(completed.stdout)
    except (TypeError, ValueError) as error:
        raise ReleaseError("release capabilities are invalid") from error
    workers = capabilities.get("packagedWorkers")
    if (
        capabilities.get("schema") != "codeclew-capabilities/1.0"
        or capabilities.get("status") != "PILOT_READY"
        or capabilities.get("runtimeMode") != "RELEASE"
        or not isinstance(workers, list)
        or {row.get("compilerVersion") for row in workers if isinstance(row, dict)}
        != {"2.3.0", "2.4.10"}
    ):
        raise ReleaseError("release runtime does not match the supported macOS profile")

    runtime_parent = runtime_home / "v2" / "runtimes"
    candidates = sorted(
        path
        for path in runtime_parent.iterdir()
        if path.is_dir() and re.fullmatch(r"[0-9a-f]{64}", path.name)
    )
    if len(candidates) != 1:
        raise ReleaseError("release build produced an ambiguous runtime capsule")
    manifest_path = candidates[0] / "runtime.json"
    try:
        manifest = json.loads(manifest_path.read_bytes())
    except (OSError, ValueError, TypeError) as error:
        raise ReleaseError("release runtime manifest is invalid") from error
    runtime_key = manifest.get("runtimeKey")
    if runtime_key != f"sha256:{candidates[0].name}" or manifest.get("mode") != "RELEASE":
        raise ReleaseError("release runtime identity is invalid")

    seed_root = work / "package" / "codeclew" / "seed"
    epoch = seed_root / f"release-N-{revision}"
    locks = seed_root / "locks"
    locks.mkdir(parents=True, mode=0o700)
    lifecycle = locks / "lifecycle.lock"
    lifecycle.write_bytes(b"")
    lifecycle.chmod(0o600)
    epoch.mkdir(mode=0o700)
    destination_state = epoch / "parallel-state"
    runtime_home.rename(destination_state)

    artifact_hashes = {
        name: value["sha256"]
        for name, value in sorted(manifest.get("artifacts", {}).items())
    }
    worker_hashes = {
        name: value["treeHash"]
        for name, value in sorted(manifest.get("workers", {}).items())
    }
    seed = {
        "artifactHashes": artifact_hashes,
        "buildEvidenceDigests": [sha256(completed.stdout)],
        "manifestDigest": manifest.get("manifestDigest"),
        "mode": "RELEASE",
        "runtimeKey": runtime_key,
        "schema": "codeclew-trusted-release-seed/1.0",
        "sourceRevision": revision,
        "sourceTree": source_tree,
        "stateEpoch": sha256(f"{runtime_key}\0{revision}".encode()),
        "workerTreeHashes": worker_hashes,
    }
    seed["seedDigest"] = sha256(canonical(seed))
    seed_path = epoch / "seed.json"
    seed_path.write_bytes(canonical(seed) + b"\n")
    seed_path.chmod(0o400)
    return seed_path


def assemble_source(root: Path, package: Path, version: str, revision: str, source_tree: str) -> None:
    source = package / "source"
    run(
        [
            "git",
            "clone",
            "--quiet",
            "--depth",
            "1",
            "--branch",
            version,
            "--no-hardlinks",
            "--no-local",
            str(root),
            str(source),
        ],
        root,
    )
    run(["git", "remote", "remove", "origin"], source)
    if git_text(source, "rev-parse", "HEAD") != revision:
        raise ReleaseError("packaged source revision mismatch")
    if git_text(source, "rev-parse", "HEAD^{tree}") != source_tree:
        raise ReleaseError("packaged source tree mismatch")
    if git_text(source, "status", "--porcelain=v1", "--untracked-files=all"):
        raise ReleaseError("packaged source checkout is dirty")


def write_archive(package: Path, output: Path, architecture: str) -> tuple[Path, Path]:
    asset = output / f"codeclew-macos-{architecture}.tar.gz"
    with tarfile.open(asset, mode="w:gz", format=tarfile.PAX_FORMAT) as archive:
        archive.add(package, arcname="codeclew", recursive=True)
    digest = file_sha256(asset)
    checksum = output / f"{asset.name}.sha256"
    checksum.write_text(f"{digest}  {asset.name}\n", encoding="ascii")
    return asset, checksum


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", arguments.version):
        raise ReleaseError("release version must be vMAJOR.MINOR.PATCH")
    if platform.system() != "Darwin" or platform.machine() not in {"arm64", "x86_64"}:
        raise ReleaseError("macOS release build requires arm64 or x86_64 macOS")

    root = Path(__file__).resolve().parent.parent
    output = arguments.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=True)
    revision = git_text(root, "rev-parse", "HEAD")
    source_tree = git_text(root, "rev-parse", "HEAD^{tree}")
    if arguments.version not in git_text(root, "tag", "--points-at", "HEAD").splitlines():
        raise ReleaseError("release version tag does not point at HEAD")
    if git_text(root, "status", "--porcelain=v1", "--untracked-files=all"):
        raise ReleaseError("release source checkout must be clean")

    temporary = Path(tempfile.mkdtemp(prefix="codeclew-release-", dir=output.parent))
    try:
        package = temporary / "package" / "codeclew"
        (package / "bin").mkdir(parents=True, mode=0o700)
        seed_path = build_seed(root, temporary, revision, source_tree)
        if not seed_path.is_file():
            raise ReleaseError("release seed was not created")
        assemble_source(root, package, arguments.version, revision, source_tree)
        launcher = package / "bin" / "clew"
        shutil.copyfile(root / "packaging" / "macos" / "clew", launcher)
        launcher.chmod(0o500)
        (package / "VERSION").write_text(arguments.version + "\n", encoding="ascii")
        version_check_environment = dict(os.environ)
        version_check_environment["CODECLEW_HOME"] = str(work / "version-check-state")
        version_check = subprocess.run(
            [str(launcher), "--version"],
            cwd=package,
            env=version_check_environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            text=True,
        )
        expected_cli_version = f"clew {arguments.version.removeprefix('v')}\n"
        if version_check.returncode != 0 or version_check.stdout != expected_cli_version:
            raise ReleaseError("release CLI version does not match the semantic version tag")
        metadata = {
            "architecture": platform.machine(),
            "operatingSystem": "macos",
            "schema": "codeclew-release/1.0",
            "sourceRevision": revision,
            "sourceTree": source_tree,
            "status": "PILOT_READY",
            "version": arguments.version,
        }
        (package / "release.json").write_bytes(canonical(metadata) + b"\n")
        validate_tree(package)
        asset, checksum = write_archive(package, output, platform.machine())
        print(
            json.dumps(
                {
                    "asset": str(asset),
                    "checksum": str(checksum),
                    "schema": "codeclew-release-build/1.0",
                    "status": "PASS",
                },
                separators=(",", ":"),
                sort_keys=True,
            )
        )
        return 0
    finally:
        make_removable(temporary)
        shutil.rmtree(temporary, ignore_errors=True)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ReleaseError as error:
        print(
            json.dumps(
                {
                    "error": str(error),
                    "schema": "codeclew-release-build-error/1.0",
                },
                separators=(",", ":"),
                sort_keys=True,
            ),
            file=os.sys.stderr,
        )
        raise SystemExit(1)
