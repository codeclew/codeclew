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
        diagnostic = completed.stderr.decode("utf-8", errors="replace").strip()
        if len(diagnostic) > 4096:
            diagnostic = diagnostic[-4096:]
        suffix = f": {diagnostic}" if diagnostic else ""
        raise ReleaseError(
            f"release command failed ({completed.returncode}): {arguments[0]}{suffix}"
        )
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


RELEASE_PROFILES = {
    "core": {"2.4.10"},
    "kotlin23": {"2.3.0", "2.4.10"},
}

MINIMAL_SOURCE_FILES = (
    "bootstrap/clew_bootstrap.py",
    "bootstrap/host_resources.py",
    "clew",
    "packaging/macos/upgrade",
    "scripts/install_agent_skill.py",
    "site/install.sh",
    "skills/codeclew/SKILL.md",
    "skills/codeclew/agents/openai.yaml",
)


def build_runtime_state(
    root: Path, work: Path, release_version: str
) -> tuple[Path, bytes]:
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
    verify_cli_version(root / "clew", release_version, runtime_home, root)

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

    return runtime_home, completed.stdout


def source_file_rows(source: Path) -> list[dict[str, object]]:
    rows = []
    for path in sorted(source.rglob("*"), key=str):
        if path.name == "release-source.json":
            continue
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise ReleaseError("release source contains a symlink")
        if stat.S_ISDIR(metadata.st_mode):
            continue
        if not stat.S_ISREG(metadata.st_mode):
            raise ReleaseError("release source contains an unsupported entry")
        rows.append({
            "mode": 0o111 if metadata.st_mode & 0o111 else 0,
            "path": path.relative_to(source).as_posix(),
            "sha256": "sha256:" + file_sha256(path),
            "size": metadata.st_size,
        })
    return rows


def assemble_source(root: Path, package: Path, revision: str, source_tree: str) -> str:
    source = package / "source"
    source.mkdir(mode=0o700)
    for relative in MINIMAL_SOURCE_FILES:
        origin = root / relative
        destination = source / relative
        destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        shutil.copyfile(origin, destination)
        destination.chmod(0o500 if origin.stat().st_mode & 0o111 else 0o400)
    rows = source_file_rows(source)
    manifest = {
        "files": rows,
        "manifestDigest": "",
        "schema": "codeclew-release-source/1.0",
        "sourceRevision": revision,
        "sourceTree": source_tree,
    }
    manifest["manifestDigest"] = sha256(canonical(manifest))
    manifest_path = source / "release-source.json"
    manifest_path.write_bytes(canonical(manifest) + b"\n")
    manifest_path.chmod(0o400)
    return str(manifest["manifestDigest"])


def make_editable(path: Path) -> None:
    metadata = path.lstat()
    if stat.S_ISDIR(metadata.st_mode):
        path.chmod(0o700)
        for child in path.iterdir():
            make_editable(child)
    elif stat.S_ISREG(metadata.st_mode):
        path.chmod(0o700 if metadata.st_mode & 0o111 else 0o600)


def seal_capsule_tree(path: Path) -> None:
    for child in path.rglob("*"):
        metadata = child.lstat()
        if stat.S_ISDIR(metadata.st_mode):
            continue
        if not stat.S_ISREG(metadata.st_mode):
            raise ReleaseError("profiled runtime contains an unsupported entry")
        child.chmod(0o500 if metadata.st_mode & 0o111 else 0o400)
    for directory in sorted(
        (value for value in path.rglob("*") if value.is_dir()),
        key=lambda value: len(value.parts),
        reverse=True,
    ):
        directory.chmod(0o500)
    path.chmod(0o500)


def runtime_capsule(state: Path) -> Path:
    runtime_parent = state / "v2" / "runtimes"
    candidates = sorted(
        path
        for path in runtime_parent.iterdir()
        if path.is_dir() and re.fullmatch(r"[0-9a-f]{64}", path.name)
    )
    if len(candidates) != 1:
        raise ReleaseError("release profile has an ambiguous runtime capsule")
    return candidates[0]


def prepare_profile_state(source: Path, destination: Path, profile: str) -> Path:
    if profile not in RELEASE_PROFILES:
        raise ReleaseError("release profile is invalid")
    shutil.copytree(source, destination)
    capsule = runtime_capsule(destination)
    manifest_path = capsule / "runtime.json"
    manifest = json.loads(manifest_path.read_bytes())
    if profile == "core":
        make_editable(capsule)
        worker = manifest.get("workers", {}).pop("kotlin23", None)
        component_key = manifest.get("components", {}).pop("kotlin23", None)
        if not isinstance(worker, dict) or not isinstance(component_key, str):
            raise ReleaseError("full runtime does not contain the Kotlin 2.3 profile")
        distribution = capsule / str(worker.get("distribution"))
        if not distribution.is_dir() or not distribution.is_relative_to(capsule):
            raise ReleaseError("Kotlin 2.3 distribution path is invalid")
        make_removable(distribution)
        shutil.rmtree(distribution)
        parent = distribution.parent
        while parent != capsule:
            try:
                parent.rmdir()
            except OSError:
                break
            parent = parent.parent
        manifest["workerIds"] = sorted(manifest["workers"])
        old_key = str(manifest.get("runtimeKey"))
        new_key = sha256(canonical({
            "baseRuntimeKey": old_key,
            "profile": profile,
            "workerIds": manifest["workerIds"],
        }))
        manifest["runtimeKey"] = new_key
        manifest["manifestDigest"] = ""
        manifest["manifestDigest"] = sha256(canonical(manifest))
        manifest_path.write_bytes(canonical(manifest) + b"\n")
        (capsule / "READY").write_text(new_key + "\n", encoding="ascii")
        target = capsule.parent / new_key.removeprefix("sha256:")
        capsule.rename(target)
        capsule = target
        seal_capsule_tree(capsule)
    components = destination / "v2" / "runtimes" / "components"
    if components.exists():
        make_removable(components)
        shutil.rmtree(components)
    components.mkdir(mode=0o700)
    return capsule


def write_seed(
    package: Path,
    state: Path,
    evidence: bytes,
    revision: str,
    source_tree: str,
    source_payload_digest: str,
) -> Path:
    capsule = runtime_capsule(state)
    manifest = json.loads((capsule / "runtime.json").read_bytes())
    runtime_key = manifest.get("runtimeKey")
    if runtime_key != f"sha256:{capsule.name}" or manifest.get("mode") != "RELEASE":
        raise ReleaseError("profiled runtime identity is invalid")
    seed_root = package / "seed"
    seed_root.mkdir(mode=0o700)
    epoch = seed_root / f"release-N-{revision}"
    locks = seed_root / "locks"
    locks.mkdir(mode=0o700)
    lifecycle = locks / "lifecycle.lock"
    lifecycle.write_bytes(b"")
    lifecycle.chmod(0o600)
    epoch.mkdir(mode=0o700)
    destination_state = epoch / "parallel-state"
    shutil.copytree(state, destination_state)
    runtime_locks = destination_state / "v2" / "locks"
    runtime_locks.mkdir(mode=0o700, parents=True, exist_ok=True)
    runtime_lease = (
        runtime_locks
        / f"runtime-{str(runtime_key).removeprefix('sha256:')}.lease"
    )
    try:
        lease_metadata = runtime_lease.lstat()
    except FileNotFoundError:
        runtime_lease.write_bytes(b"")
    else:
        if (
            not stat.S_ISREG(lease_metadata.st_mode)
            or lease_metadata.st_uid != os.geteuid()
            or lease_metadata.st_size != 0
        ):
            raise ReleaseError("profiled runtime lease is unsafe")
    runtime_lease.chmod(0o600)

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
        "buildEvidenceDigests": [sha256(evidence)],
        "manifestDigest": manifest.get("manifestDigest"),
        "mode": "RELEASE",
        "runtimeKey": runtime_key,
        "schema": "codeclew-trusted-release-seed/2.0",
        "sourcePayloadDigest": source_payload_digest,
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


def write_archive(
    package: Path, output: Path, architecture: str, profile: str
) -> tuple[Path, Path]:
    name = (
        f"codeclew-macos-{architecture}.tar.gz"
        if profile == "core"
        else f"codeclew-{profile}-macos-{architecture}.tar.gz"
    )
    asset = output / name
    with tarfile.open(asset, mode="w:gz", format=tarfile.PAX_FORMAT) as archive:
        archive.add(package, arcname="codeclew", recursive=True)
    digest = file_sha256(asset)
    checksum = output / f"{asset.name}.sha256"
    checksum.write_text(f"{digest}  {asset.name}\n", encoding="ascii")
    return asset, checksum


def verify_cli_version(
    launcher: Path, release_version: str, state_root: Path, working_directory: Path
) -> None:
    environment = dict(os.environ)
    environment["CODECLEW_HOME"] = str(state_root)
    completed = subprocess.run(
        [str(launcher), "--version"],
        cwd=working_directory,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        text=True,
    )
    expected = f"clew {release_version.removeprefix('v')}\n"
    if completed.returncode != 0 or completed.stdout != expected:
        raise ReleaseError("release CLI version does not match the semantic version tag")


def verify_annotated_tag_navigation(
    root: Path,
    launcher: Path,
    environment: dict[str, str],
    work: Path,
) -> None:
    """Exercise the packaged launcher against branch and annotated-tag authority."""
    source = work / "source"
    shutil.copytree(root / "fixtures" / "kotlin-basic", source)
    run(["git", "init", "-q", "-b", "main"], source)
    run(["git", "add", "."], source)
    identity = [
        "-c",
        "user.name=Codeclew Release Smoke",
        "-c",
        "user.email=codeclew-release-smoke@localhost",
    ]
    run(["git", *identity, "commit", "-q", "-m", "fixture"], source)
    run(
        ["git", *identity, "tag", "-a", "v-release-smoke", "-m", "annotated fixture"],
        source,
    )
    revision = git_text(source, "rev-parse", "HEAD")
    if git_text(source, "rev-parse", "refs/tags/v-release-smoke") == revision:
        raise ReleaseError("navigation smoke tag is not annotated")

    linked = work / "linked-worktree"
    clone = work / "standalone-clone"
    run(
        ["git", "worktree", "add", "-q", "--detach", str(linked), "v-release-smoke"],
        source,
    )
    run(
        [
            "git",
            "clone",
            "-q",
            "--no-local",
            "--depth",
            "1",
            "--branch",
            "v-release-smoke",
            str(source),
            str(clone),
        ],
        work,
    )

    for repository, target_ref, expected_ref in [
        (source, "main", "refs/heads/main"),
        (linked, "v-release-smoke", "refs/tags/v-release-smoke"),
        (clone, "v-release-smoke", "refs/tags/v-release-smoke"),
    ]:
        before_head = git_text(repository, "rev-parse", "--verify", "HEAD^{commit}")
        before_status = git_text(
            repository,
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--ignore-submodules=none",
        )
        if before_head != revision or before_status:
            raise ReleaseError("navigation smoke repository authority is invalid")
        output = run(
            [
                str(launcher),
                "nav",
                "query",
                "--repo",
                str(repository),
                "--target-ref",
                target_ref,
                "--language",
                "kotlin",
                "--profile",
                "kotlin-2.4.10-gradle-single",
                "--compilation",
                ":/main",
                "--term",
                "IntegerSource",
                "--term",
                "NumericSource",
                "--term",
                "callSource",
                "--source",
            ],
            repository,
            environment=environment,
        )
        try:
            result = json.loads(output)
            session = result["session"]
            admission = result["admission"]
            agent_contract = admission["agentContract"]
            navigation = result["navigation"]
            candidates = navigation["candidates"]
            decision_source = navigation["decisionSource"]["source"]
            session_id = session["sessionId"]
        except (KeyError, TypeError, ValueError) as error:
            raise ReleaseError("packaged navigation smoke output is invalid") from error
        if (
            result.get("schema") != "codeclew-nav-query/2.0"
            or result.get("status") != "OPEN"
            or not isinstance(admission, dict)
            or admission.get("status") != "PASS"
            or admission.get("runtimeMode") != "RELEASE"
            or not isinstance(agent_contract, dict)
            or agent_contract.get("launcherAuthority") != "INSTALLED_RELEASE"
            or agent_contract.get("sourceFallbackAllowed") is not False
            or not isinstance(session, dict)
            or session.get("targetRef") != expected_ref
            or session.get("baseRevision") != revision
            or not isinstance(candidates, list)
            or not candidates
            or not isinstance(decision_source, dict)
            or decision_source.get("status") != "SUPPORTED"
            or not decision_source.get("windows")
        ):
            raise ReleaseError("packaged navigation smoke authority is invalid")
        run(
            [str(launcher), "session", "close", "--session", str(session_id)],
            repository,
            environment=environment,
        )
        run(
            [str(launcher), "session", "gc", "--session", str(session_id)],
            repository,
            environment=environment,
        )
        if (
            git_text(repository, "rev-parse", "--verify", "HEAD^{commit}") != before_head
            or git_text(
                repository,
                "status",
                "--porcelain=v1",
                "--untracked-files=all",
                "--ignore-submodules=none",
            )
            != before_status
        ):
            raise ReleaseError("packaged navigation smoke changed the repository")


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
        runtime_state, evidence = build_runtime_state(
            root, temporary, arguments.version
        )
        assets = []
        for profile, expected_workers in RELEASE_PROFILES.items():
            package = temporary / "packages" / profile / "codeclew"
            (package / "bin").mkdir(parents=True, mode=0o700)
            source_payload_digest = assemble_source(
                root, package, revision, source_tree
            )
            profiled_state = temporary / "states" / profile
            prepare_profile_state(runtime_state, profiled_state, profile)
            seed_path = write_seed(
                package,
                profiled_state,
                evidence,
                revision,
                source_tree,
                source_payload_digest,
            )
            if not seed_path.is_file():
                raise ReleaseError("release seed was not created")
            launcher = package / "bin" / "clew"
            shutil.copyfile(root / "packaging" / "macos" / "clew", launcher)
            launcher.chmod(0o500)
            (package / "VERSION").write_text(
                arguments.version + "\n", encoding="ascii"
            )
            (package / "PROFILE").write_text(profile + "\n", encoding="ascii")
            metadata = {
                "architecture": platform.machine(),
                "operatingSystem": "macos",
                "profile": profile,
                "schema": "codeclew-release/2.0",
                "sourceRevision": revision,
                "sourceTree": source_tree,
                "status": "PILOT_READY",
                "version": arguments.version,
            }
            (package / "release.json").write_bytes(canonical(metadata) + b"\n")
            validate_tree(package)
            verification_home = temporary / "verification" / profile
            environment = dict(os.environ)
            environment["CODECLEW_HOME"] = str(verification_home)
            version_output = run([str(launcher), "--version"], package, environment=environment)
            if version_output != f"clew {arguments.version.removeprefix('v')}\n".encode():
                raise ReleaseError("packaged profile CLI version is invalid")
            capabilities = json.loads(
                run([str(launcher), "capabilities"], package, environment=environment)
            )
            observed_workers = {
                row.get("compilerVersion")
                for row in capabilities.get("packagedWorkers", [])
                if isinstance(row, dict)
            }
            if observed_workers != expected_workers:
                raise ReleaseError("packaged profile worker set is invalid")
            if profile == "core":
                verify_annotated_tag_navigation(
                    root,
                    launcher,
                    environment,
                    temporary / "navigation-smoke",
                )
            asset, checksum = write_archive(
                package, output, platform.machine(), profile
            )
            assets.append({
                "asset": str(asset),
                "checksum": str(checksum),
                "profile": profile,
            })
        print(
            json.dumps(
                {
                    "assets": assets,
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
