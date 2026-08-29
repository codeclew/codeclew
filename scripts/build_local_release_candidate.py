#!/usr/bin/env python3
"""Build one clean-source local RELEASE candidate without weakening release tags."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import re
import shutil
import sys
import tempfile
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_macos_release as release


SCHEMA = "codeclew-local-release-candidate/1.0"


def workspace_version(root: Path) -> str:
    try:
        document = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
        version = document["workspace"]["package"]["version"]
    except (OSError, KeyError, TypeError, tomllib.TOMLDecodeError) as error:
        raise release.ReleaseError("workspace version is unavailable") from error
    if not isinstance(version, str) or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version) is None:
        raise release.ReleaseError("workspace version is invalid")
    return "v" + version


def clean_source_authority(root: Path) -> tuple[str, str]:
    if release.git_text(root, "status", "--porcelain=v1", "--untracked-files=all"):
        raise release.ReleaseError("local release candidate requires a clean source checkout")
    return (
        release.git_text(root, "rev-parse", "HEAD"),
        release.git_text(root, "rev-parse", "HEAD^{tree}"),
    )


def candidate_metadata(
    version: str,
    revision: str,
    source_tree: str,
    profile: str,
    runtime_key: str,
    runtime_manifest_digest: str,
    source_payload_digest: str,
) -> dict[str, object]:
    value: dict[str, object] = {
        "architecture": platform.machine(),
        "candidateDigest": "",
        "operatingSystem": "macos",
        "profile": profile,
        "runtimeKey": runtime_key,
        "runtimeManifestDigest": runtime_manifest_digest,
        "schema": SCHEMA,
        "sourcePayloadDigest": source_payload_digest,
        "sourceRevision": revision,
        "sourceTree": source_tree,
        "status": "LOCAL_ONLY",
        "version": version,
    }
    value["candidateDigest"] = release.sha256(release.canonical(value))
    return value


def validate_output(output: Path) -> Path:
    parent = output.expanduser().absolute().parent.resolve(strict=True)
    destination = parent / output.name
    if not output.name or output.name in {".", ".."} or destination.exists() or destination.is_symlink():
        raise release.ReleaseError("candidate output must not already exist")
    return destination


def build_candidate(root: Path, output: Path, profile: str) -> dict[str, object]:
    if platform.system() != "Darwin" or platform.machine() not in {"arm64", "x86_64"}:
        raise release.ReleaseError("local release candidates require supported macOS")
    if profile not in release.RELEASE_PROFILES:
        raise release.ReleaseError("candidate profile is invalid")
    destination = validate_output(output)
    version = workspace_version(root)
    revision, source_tree = clean_source_authority(root)
    temporary = Path(tempfile.mkdtemp(prefix=".codeclew-candidate-", dir=destination.parent))
    try:
        runtime_state, evidence = release.build_runtime_state(root, temporary, version)
        package = temporary / "package" / "codeclew"
        (package / "bin").mkdir(parents=True, mode=0o700)
        package.chmod(0o700)
        source_payload_digest = release.assemble_source(
            root, package, revision, source_tree
        )
        profiled_state = temporary / "profile-state"
        capsule = release.prepare_profile_state(runtime_state, profiled_state, profile)
        runtime_manifest = json.loads((capsule / "runtime.json").read_bytes())
        seed_path = release.write_seed(
            package,
            profiled_state,
            evidence,
            revision,
            source_tree,
            source_payload_digest,
        )
        if not seed_path.is_file():
            raise release.ReleaseError("candidate seed was not created")
        launcher = package / "bin" / "clew"
        shutil.copyfile(root / "packaging" / "macos" / "clew", launcher)
        launcher.chmod(0o500)
        (package / "VERSION").write_text(version + "\n", encoding="ascii")
        (package / "PROFILE").write_text(profile + "\n", encoding="ascii")
        metadata = candidate_metadata(
            version,
            revision,
            source_tree,
            profile,
            str(runtime_manifest.get("runtimeKey")),
            str(runtime_manifest.get("manifestDigest")),
            source_payload_digest,
        )
        (package / "release.json").write_bytes(release.canonical(metadata) + b"\n")
        release.validate_tree(package)

        environment = dict(os.environ)
        environment["CODECLEW_HOME"] = str(temporary / "verification-home")
        observed_version = release.run(
            [str(launcher), "--version"], package, environment=environment
        )
        if observed_version != f"clew {version.removeprefix('v')}\n".encode():
            raise release.ReleaseError("candidate CLI version is invalid")
        capabilities = json.loads(
            release.run([str(launcher), "capabilities"], package, environment=environment)
        )
        if (
            capabilities.get("runtimeMode") != "RELEASE"
            or capabilities.get("runtimeKey") != metadata["runtimeKey"]
            or capabilities.get("runtimeManifestDigest")
            != metadata["runtimeManifestDigest"]
        ):
            raise release.ReleaseError("candidate runtime identity is invalid")

        os.replace(package, destination)
        destination.chmod(0o700)
        return {
            "candidateDigest": metadata["candidateDigest"],
            "launcher": "bin/clew",
            "profile": profile,
            "runtimeKey": metadata["runtimeKey"],
            "runtimeManifestDigest": metadata["runtimeManifestDigest"],
            "schema": SCHEMA,
            "sourceRevision": revision,
            "status": "PASS",
            "version": version,
        }
    finally:
        release.make_removable(temporary)
        shutil.rmtree(temporary, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--profile", choices=sorted(release.RELEASE_PROFILES), default="core")
    arguments = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    print(
        json.dumps(
            build_candidate(root, arguments.output, arguments.profile),
            separators=(",", ":"),
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except release.ReleaseError as error:
        print(
            json.dumps(
                {"error": str(error), "schema": SCHEMA, "status": "FAILED"},
                separators=(",", ":"),
                sort_keys=True,
            ),
            file=os.sys.stderr,
        )
        raise SystemExit(1)
