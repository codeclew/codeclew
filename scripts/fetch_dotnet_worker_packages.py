#!/usr/bin/env python3
"""Populate a local NuGet folder feed for the C# Roslyn worker.

Some hosts cannot use the .NET TLS stack for NuGet (for example a sandbox that
denies keychain access) while Python can reach nuget.org with an explicit CA
bundle. This tool resolves the worker's package closure from its lock file when
present, otherwise from nuspec dependencies, verifies each package's SHA-512
against the lock file, and writes `<id>.<version>.nupkg` files to the feed
directory. `dotnet restore --source <feed> --locked-mode` then runs offline.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import io
import json
import os
import re
import ssl
import sys
import urllib.request
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

FLAT = "https://api.nuget.org/v3-flatcontainer"
TARGET = "net10.0"
FRAMEWORK_ORDER = [
    "net10.0", "net9.0", "net8.0", "net7.0", "net6.0", "net5.0",
    "netcoreapp3.1", "netstandard2.1", "netstandard2.0", "netstandard1.6",
    "netstandard1.3", "netstandard1.0", "",
]


def context() -> ssl.SSLContext:
    bundle = os.environ.get("CODECLEW_NUGET_CA_BUNDLE") or os.environ.get("NODE_EXTRA_CA_CERTS")
    created = ssl.create_default_context()
    if bundle:
        created.load_verify_locations(cafile=bundle)
        # Inspection CAs commonly lack extensions that strict RFC 5280 checks require.
        created.verify_flags &= ~ssl.VERIFY_X509_STRICT
    return created


def fetch(url: str, tls: ssl.SSLContext) -> bytes:
    with urllib.request.urlopen(url, context=tls, timeout=120) as response:
        return response.read()


def normalize_framework(value: str) -> str:
    value = value.lower().lstrip(".")
    aliases = {".netstandard2.0": "netstandard2.0", "netstandard20": "netstandard2.0"}
    return aliases.get(value, value)


def lowest_version(range_text: str) -> str:
    text = range_text.strip()
    if text and text[0] in "[(":
        text = text[1:-1].split(",")[0].strip()
    return text


def nuspec_dependencies(package: bytes) -> list[tuple[str, str]]:
    with zipfile.ZipFile(io.BytesIO(package)) as archive:
        name = next(n for n in archive.namelist() if n.endswith(".nuspec") and "/" not in n)
        root = ET.fromstring(archive.read(name))
    namespace = re.match(r"\{.*\}", root.tag)
    ns = namespace.group(0) if namespace else ""
    groups: dict[str, list[tuple[str, str]]] = {}
    dependencies = root.find(f"{ns}metadata/{ns}dependencies")
    if dependencies is None:
        return []
    for group in dependencies.findall(f"{ns}group"):
        framework = normalize_framework(group.get("targetFramework", ""))
        groups[framework] = [
            (dep.get("id"), lowest_version(dep.get("version", "")))
            for dep in group.findall(f"{ns}dependency")
        ]
    flat = [(dep.get("id"), lowest_version(dep.get("version", ""))) for dep in dependencies.findall(f"{ns}dependency")]
    if not groups:
        return flat
    for framework in FRAMEWORK_ORDER:
        if framework in groups:
            return groups[framework]
    return []


def from_lock(lock: Path) -> dict[str, tuple[str, str | None]]:
    data = json.loads(lock.read_text(encoding="utf-8"))
    result: dict[str, tuple[str, str | None]] = {}
    for framework in data.get("dependencies", {}).values():
        for name, entry in framework.items():
            if entry.get("type") == "Project":
                continue
            result[name.lower()] = (entry["resolved"], entry.get("contentHash"))
    return result


def from_props(props: Path) -> list[tuple[str, str]]:
    root = ET.parse(props).getroot()
    return [(item.get("Include"), item.get("Version")) for item in root.iter("PackageVersion")]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", default=str(Path(__file__).resolve().parents[1] / "workers/dotnet"))
    parser.add_argument("--feed", required=True)
    args = parser.parse_args()
    worker = Path(args.worker)
    feed = Path(args.feed)
    feed.mkdir(parents=True, exist_ok=True)
    tls = context()
    lock = worker / "src/packages.lock.json"
    pending: list[tuple[str, str, str | None]]
    locked = from_lock(lock) if lock.is_file() else {}
    if locked:
        pending = [(name, version, digest) for name, (version, digest) in sorted(locked.items())]
        follow = False
    else:
        pending = [(name, version, None) for name, version in from_props(worker / "Directory.Packages.props")]
        follow = True
    seen: set[str] = set()
    while pending:
        name, version, expected = pending.pop()
        key = f"{name.lower()}/{version.lower()}"
        if key in seen:
            continue
        seen.add(key)
        target = feed / f"{name.lower()}.{version.lower()}.nupkg"
        if target.is_file():
            package = target.read_bytes()
        else:
            package = fetch(f"{FLAT}/{name.lower()}/{version.lower()}/{name.lower()}.{version.lower()}.nupkg", tls)
        if expected is not None:
            actual = base64.b64encode(hashlib.sha512(package).digest()).decode("ascii")
            if actual != expected:
                print(f"content hash mismatch for {name} {version}", file=sys.stderr)
                return 1
        if not target.is_file():
            target.write_bytes(package)
        if follow:
            for dependency, dependency_version in nuspec_dependencies(package):
                pending.append((dependency, dependency_version, None))
    print(json.dumps({"feed": str(feed), "packages": len(seen)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
