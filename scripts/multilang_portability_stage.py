#!/usr/bin/env python3
"""Capture and verify adapter-only portability stages against frozen K0."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LOCK = ROOT / "contracts/core/core-contract.lock.json"
IGNORED_COMPONENTS = {".git", "node_modules", "target", "__pycache__"}


def canonical(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()


def sha256(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def members(paths: list[Path]) -> list[dict[str, object]]:
    result: list[dict[str, object]] = []
    for source in paths:
        source = source.resolve(strict=True)
        if source.is_symlink():
            raise SystemExit(f"adapter input is a symlink: {source}")
        candidates = [source] if source.is_file() else sorted(source.rglob("*"))
        for path in candidates:
            relative_parts = path.relative_to(ROOT).parts
            if any(part in IGNORED_COMPONENTS for part in relative_parts):
                continue
            stat = path.lstat()
            if path.is_symlink() or not path.is_file():
                continue
            data = path.read_bytes()
            result.append({
                "path": "/".join(relative_parts),
                "size": stat.st_size,
                "sha256": sha256(data),
            })
    result.sort(key=lambda item: str(item["path"]))
    if not result:
        raise SystemExit("adapter stage has no files")
    if len({item["path"] for item in result}) != len(result):
        raise SystemExit("adapter stage contains duplicate files")
    return result


def stage(stage_id: str, language: str, adapter_paths: list[Path], evidence: list[Path]) -> dict[str, object]:
    lock_bytes = LOCK.read_bytes()
    lock = json.loads(lock_bytes)
    if canonical(lock) != lock_bytes:
        raise SystemExit("core lock is not canonical JSON plus newline")
    adapter_members = members(adapter_paths)
    evidence_members = members(evidence)
    adapter_digest = sha256(canonical(adapter_members))
    evidence_digest = sha256(canonical(evidence_members))
    value: dict[str, object] = {
        "schema": "codeclew.portability-stage/0.1",
        "stage": stage_id,
        "language": language,
        "coreLockSha256": sha256(lock_bytes),
        "coreContractDigests": lock["digests"],
        "adapterMembers": adapter_members,
        "adapterDigest": adapter_digest,
        "evidenceMembers": evidence_members,
        "evidenceDigest": evidence_digest,
        "sharedCoreChanged": False,
        "stageDigest": "",
    }
    value["stageDigest"] = sha256(canonical(value))
    return value


def main() -> None:
    parser = argparse.ArgumentParser()
    subcommands = parser.add_subparsers(dest="command", required=True)
    capture = subcommands.add_parser("capture")
    capture.add_argument("--stage", required=True)
    capture.add_argument("--language", required=True)
    capture.add_argument("--adapter-path", action="append", required=True, type=Path)
    capture.add_argument("--evidence", action="append", required=True, type=Path)
    capture.add_argument("--output", required=True, type=Path)
    verify = subcommands.add_parser("verify")
    verify.add_argument("manifest", type=Path)
    args = parser.parse_args()

    if args.command == "capture":
        output = args.output.resolve()
        if output.exists():
            raise SystemExit("portability stage output already exists")
        value = stage(args.stage, args.language, args.adapter_path, args.evidence)
        output.parent.mkdir(parents=True, exist_ok=True)
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
        descriptor = os.open(output, flags, 0o444)
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(canonical(value))
            handle.flush()
            os.fsync(handle.fileno())
        print(json.dumps({"status": "CAPTURED", "stageDigest": value["stageDigest"]}, sort_keys=True))
        return

    manifest_bytes = args.manifest.read_bytes()
    manifest = json.loads(manifest_bytes)
    if canonical(manifest) != manifest_bytes:
        raise SystemExit("portability manifest is not canonical JSON plus newline")
    rebuilt = stage(
        str(manifest["stage"]),
        str(manifest["language"]),
        [ROOT / item["path"] for item in manifest["adapterMembers"]],
        [ROOT / item["path"] for item in manifest["evidenceMembers"]],
    )
    if rebuilt != manifest:
        raise SystemExit("portability stage no longer matches current core/adapter/evidence bytes")
    print(json.dumps({"status": "VERIFIED", "stageDigest": manifest["stageDigest"]}, sort_keys=True))


if __name__ == "__main__":
    main()
