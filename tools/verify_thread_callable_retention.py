#!/usr/bin/env python3
"""Verify a retained thread-callable root without invoking Codeclew runtime tools.

The verifier accepts private paths as command-line inputs but emits only
content-derived counts and digests. It understands both loose and packed v3
CAS objects so the same receipt can be compared before and after lifecycle GC.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from collections import deque
from pathlib import Path
from typing import Any


CAS_SCHEMA = "codeclew-cas-object/2.0"
CAS_DOMAIN = b"codeclew-cas/v2\0"
MAX_ROOT_BYTES = 65 * 1024 * 1024


class VerificationError(RuntimeError):
    pass


def canonical(value: Any) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")


def load_canonical(path: Path, maximum: int) -> tuple[Any, bytes]:
    metadata = path.lstat()
    if path.is_symlink() or not path.is_file() or metadata.st_size > maximum:
        raise VerificationError("input is missing, unsafe, or exceeds its budget")
    data = path.read_bytes()
    try:
        value = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise VerificationError("input is not canonical JSON") from error
    if canonical(value) != data:
        raise VerificationError("input JSON is not canonical")
    return value, data


def cas_references(value: Any) -> list[dict[str, Any]]:
    found: list[dict[str, Any]] = []
    pending = [value]
    while pending:
        current = pending.pop()
        if isinstance(current, dict):
            if current.get("schema") == CAS_SCHEMA:
                if set(current) != {"schema", "objectSchema", "digest", "size"}:
                    raise VerificationError("CAS reference has an open or malformed shape")
                found.append(current)
            else:
                pending.extend(current.values())
        elif isinstance(current, list):
            pending.extend(current)
    return found


def validate_reference(reference: dict[str, Any]) -> None:
    digest = reference.get("digest")
    object_schema = reference.get("objectSchema")
    size = reference.get("size")
    if (
        not isinstance(digest, str)
        or len(digest) != 71
        or not digest.startswith("sha256:")
        or any(character not in "0123456789abcdef" for character in digest[7:])
        or not isinstance(object_schema, str)
        or not object_schema
        or not isinstance(size, int)
        or isinstance(size, bool)
        or size < 0
        or size > MAX_ROOT_BYTES
    ):
        raise VerificationError("CAS reference identity is invalid")


def pack_catalog(state_root: Path) -> dict[str, tuple[dict[str, Any], Path, int]]:
    catalog: dict[str, tuple[dict[str, Any], Path, int]] = {}
    packs = state_root / "objects" / "packs-v3"
    if not packs.is_dir() or packs.is_symlink():
        return catalog
    for index_path in sorted(packs.glob("*.json")):
        manifest, _ = load_canonical(index_path, 64 * 1024 * 1024)
        entries = manifest.get("objects") if isinstance(manifest, dict) else None
        if not isinstance(entries, list):
            raise VerificationError("CAS pack index is malformed")
        offset = 0
        pack_path = index_path.with_suffix(".pack")
        for entry in entries:
            if not isinstance(entry, dict) or set(entry) != {"object", "offset"}:
                raise VerificationError("CAS pack entry is malformed")
            reference = entry["object"]
            validate_reference(reference)
            if entry["offset"] != offset:
                raise VerificationError("CAS pack offsets are not contiguous")
            digest = reference["digest"]
            location = (reference, pack_path, offset)
            if digest in catalog and catalog[digest] != location:
                raise VerificationError("CAS digest has conflicting pack locations")
            catalog[digest] = location
            offset += reference["size"]
        if not pack_path.is_file() or pack_path.stat().st_size != offset:
            raise VerificationError("CAS pack data size is inconsistent")
    return catalog


def object_bytes(
    state_root: Path,
    catalog: dict[str, tuple[dict[str, Any], Path, int]],
    reference: dict[str, Any],
) -> bytes:
    validate_reference(reference)
    component = reference["digest"][7:]
    loose = state_root / "objects" / "sha256" / component[:2] / component[2:]
    if loose.is_file() and not loose.is_symlink():
        data = loose.read_bytes()
    else:
        location = catalog.get(reference["digest"])
        if location is None or location[0] != reference:
            raise VerificationError("retained CAS object is unavailable")
        with location[1].open("rb") as pack:
            pack.seek(location[2])
            data = pack.read(reference["size"])
    if len(data) != reference["size"]:
        raise VerificationError("retained CAS object size changed")
    digest = hashlib.sha256(
        CAS_DOMAIN + reference["objectSchema"].encode("utf-8") + b"\0" + data
    ).hexdigest()
    if reference["digest"] != f"sha256:{digest}":
        raise VerificationError("retained CAS object digest changed")
    return data


def verify(
    state_root: Path,
    root_path: Path,
    expected_root_schema: str = "codeclew-thread-callable-root/1.0",
) -> dict[str, Any]:
    root, root_bytes = load_canonical(root_path, MAX_ROOT_BYTES)
    if not isinstance(root, dict) or root.get("schema") != expected_root_schema:
        raise VerificationError("retained root schema is invalid")
    authority = root.get("authority")
    declared = authority.get("directCasClosure") if isinstance(authority, dict) else None
    if not isinstance(declared, list) or not declared:
        raise VerificationError("thread callable root omits its retained closure")
    declared_by_digest: dict[str, dict[str, Any]] = {}
    for reference in declared:
        validate_reference(reference)
        digest = reference["digest"]
        if digest in declared_by_digest and declared_by_digest[digest] != reference:
            raise VerificationError("retained closure has conflicting references")
        declared_by_digest[digest] = reference
    if list(declared_by_digest.values()) != sorted(
        declared_by_digest.values(), key=lambda row: (row["objectSchema"], row["digest"])
    ):
        raise VerificationError("retained closure is not canonical")

    catalog = pack_catalog(state_root)
    queue = deque(cas_references(root))
    observed: dict[str, tuple[dict[str, Any], bytes]] = {}
    while queue:
        reference = queue.popleft()
        digest = reference["digest"]
        if digest in observed:
            if observed[digest][0] != reference:
                raise VerificationError("reachable CAS reference was substituted")
            continue
        data = object_bytes(state_root, catalog, reference)
        observed[digest] = (reference, data)
        try:
            value = json.loads(data)
        except (UnicodeDecodeError, json.JSONDecodeError):
            continue
        queue.extend(cas_references(value))

    missing = set(declared_by_digest) - set(observed)
    if missing:
        raise VerificationError("declared retained closure is not reachable")
    closure_identity = hashlib.sha256()
    for digest, (reference, data) in sorted(observed.items()):
        closure_identity.update(reference["objectSchema"].encode("utf-8"))
        closure_identity.update(b"\0")
        closure_identity.update(digest.encode("ascii"))
        closure_identity.update(b"\0")
        closure_identity.update(hashlib.sha256(data).digest())
    return {
        "schema": "codeclew-thread-callable-retention-verification/1.0",
        "status": "PASS",
        "rootDigest": "sha256:" + hashlib.sha256(root_bytes).hexdigest(),
        "declaredObjectCount": len(declared_by_digest),
        "reachableObjectCount": len(observed),
        "reachableBytes": sum(len(data) for _reference, data in observed.values()),
        "closureDigest": "sha256:" + closure_identity.hexdigest(),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--state-root", required=True, type=Path)
    parser.add_argument("--root", required=True, type=Path)
    arguments = parser.parse_args()
    try:
        result = verify(arguments.state_root, arguments.root)
    except (OSError, VerificationError) as error:
        print(json.dumps({"schema": "codeclew-thread-callable-retention-verification/1.0", "status": "FAIL", "reason": str(error)}, separators=(",", ":"), sort_keys=True))
        return 1
    print(json.dumps(result, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
