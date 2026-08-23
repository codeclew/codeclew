#!/usr/bin/env python3
"""Fail closed when tracked Git data contains private repository information."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from collections.abc import Iterable


GENERIC_NAME = "Codeclew Maintainers"
GENERIC_EMAIL = "maintainers@codeclew.invalid"
FORBIDDEN_TOKEN_SHA256 = {
    "0b3e1b057983454547b3bcbf2fac99b7309dbb5737a2d387f9f4a9bddf895147",
    "f5a38e4245e1c0094edfe353a8d77577f61031999ec900c57de05640cb5459d9",
    "b717fd72f2de6146e6ac170bc7913e8783cd0da553e8a9b3a9ba3d506ff628f7",
}
FORBIDDEN_PATH_PREFIXES = (
    "benchmarks/reports/",
    "docs/experiments/evidence/",
    "docs/pilot/results/",
    "evidence/graphs/",
)
FORBIDDEN_EXACT_PATHS = {
    "benchmarks/kotlin-real-repository/k1/corpus.json",
    "docs/research/codeclew/source-manifest.json",
}
PILOT_CASE_TEMPLATE = "docs/pilot/case-template.json"
PILOT_CASE_TEMPLATE_SHA256 = "c5a004f7fd6c7c544fd6346188944f3aec2df7d509d7aa185b1a83c0df20b0e1"
PILOT_EVIDENCE_SCHEMAS = {
    "codeclew-pilot-attestation-key/1.0",
    "codeclew-pilot-case/1.0",
    "codeclew-pilot-case-set/1.0",
    "codeclew-pilot-release-decision/1.0",
    "codeclew-pilot-source-snapshot/1.0",
}
TOKEN_RE = re.compile(br"[A-Za-z0-9]+")
HOME_PATH_RE = re.compile(br"/(?:Users|home)/[A-Za-z0-9._-]+")
EMAIL_RE = re.compile(br"[A-Z0-9._%+-]+@([A-Z0-9.-]+\.[A-Z]{2,})", re.IGNORECASE)
SECRET_PATTERNS = (
    ("private-key", re.compile(br"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----")),
    ("aws-key", re.compile(br"(?:AKIA|ASIA)[0-9A-Z]{16}")),
    ("github-token", re.compile(br"gh[pousr]_[A-Za-z0-9_]{20,}")),
    ("slack-token", re.compile(br"xox[baprs]-[A-Za-z0-9-]{10,}")),
    ("openai-token", re.compile(br"(?:sk-proj-[A-Za-z0-9_-]{20,}|sk-[A-Za-z0-9]{32,})")),
    ("credential-url", re.compile(br"https?://[^\s/:@]+:[^\s@/]+@")),
)


def git(*args: str, input_bytes: bytes | None = None) -> bytes:
    return subprocess.check_output(("git", *args), input=input_bytes)


def path_rules(path: str) -> list[str]:
    findings = []
    if path in FORBIDDEN_EXACT_PATHS:
        findings.append("private-generated-path")
    if any(path.startswith(prefix) for prefix in FORBIDDEN_PATH_PREFIXES):
        findings.append("private-generated-path")
    return findings


def blob_rules(data: bytes, path: str | None = None) -> list[str]:
    findings: set[str] = set()
    try:
        parsed = json.loads(data)
    except (json.JSONDecodeError, UnicodeDecodeError):
        parsed = None
    pilot_schema = parsed.get("schema") if isinstance(parsed, dict) else None
    exact_template = (
        pilot_schema == "codeclew-pilot-case/1.0"
        and path == PILOT_CASE_TEMPLATE
        and hashlib.sha256(data).hexdigest() == PILOT_CASE_TEMPLATE_SHA256
    )
    if pilot_schema in PILOT_EVIDENCE_SCHEMAS and not exact_template:
        findings.add("filled-pilot-case")
    if HOME_PATH_RE.search(data):
        findings.add("personal-home-path")
    for token in TOKEN_RE.findall(data):
        digest = hashlib.sha256(token.lower()).hexdigest()
        if digest in FORBIDDEN_TOKEN_SHA256:
            findings.add("forbidden-identity")
    for match in EMAIL_RE.finditer(data):
        if not match.group(1).lower().endswith(b".invalid"):
            findings.add("non-placeholder-email")
    for label, pattern in SECRET_PATTERNS:
        if pattern.search(data):
            findings.add(label)
    return sorted(findings)


def read_blobs(entries: Iterable[tuple[str, str]]) -> Iterable[tuple[str, bytes]]:
    process = subprocess.Popen(
        ("git", "cat-file", "--batch"),
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
    )
    assert process.stdin is not None and process.stdout is not None
    cache: dict[str, bytes] = {}
    try:
        for oid, path in entries:
            data = cache.get(oid)
            if data is None:
                process.stdin.write(oid.encode("ascii") + b"\n")
                process.stdin.flush()
                header = process.stdout.readline().split()
                if len(header) != 3 or header[1] != b"blob":
                    raise RuntimeError(f"invalid Git blob response for {path}")
                size = int(header[2])
                data = process.stdout.read(size)
                if process.stdout.read(1) != b"\n":
                    raise RuntimeError(f"truncated Git blob response for {path}")
                cache[oid] = data
            yield path, data
    finally:
        process.stdin.close()
        process.stdout.close()
        if process.wait() != 0:
            raise RuntimeError("git cat-file failed")


def index_entries() -> list[tuple[str, str]]:
    entries = []
    for raw in git("ls-files", "--stage", "-z").split(b"\0"):
        if not raw:
            continue
        metadata, path_bytes = raw.split(b"\t", 1)
        mode, oid, stage = metadata.split()
        path = path_bytes.decode("utf-8")
        if stage != b"0":
            raise RuntimeError(f"unmerged index entry: {path}")
        if mode == b"160000":
            continue
        entries.append((oid.decode("ascii"), path))
    return entries


def history_entries() -> list[tuple[str, str]]:
    objects = git("rev-list", "--objects", "HEAD")
    checked = git(
        "cat-file",
        "--batch-check=%(objectname) %(objecttype) %(rest)",
        input_bytes=objects,
    )
    entries = []
    for raw in checked.splitlines():
        fields = raw.decode("utf-8").split(" ", 2)
        if len(fields) == 3 and fields[1] == "blob":
            entries.append((fields[0], fields[2]))
    return entries


def check_entries(entries: Iterable[tuple[str, str]]) -> list[tuple[str, str]]:
    findings = []
    materialized = list(entries)
    for _oid, path in materialized:
        findings.extend((path, rule) for rule in path_rules(path))
    for path, data in read_blobs(materialized):
        findings.extend((path, rule) for rule in blob_rules(data, path))
    return sorted(set(findings))


def check_history_metadata() -> list[tuple[str, str]]:
    findings = []
    rows = git("log", "HEAD", "--format=%H%x00%an%x00%ae%x00%cn%x00%ce").splitlines()
    for row in rows:
        commit, author, email, committer, committer_email = row.decode("utf-8").split("\0")
        if (author, email) != (GENERIC_NAME, GENERIC_EMAIL):
            findings.append((commit, "non-generic-author"))
        if (committer, committer_email) != (GENERIC_NAME, GENERIC_EMAIL):
            findings.append((commit, "non-generic-committer"))
    return findings


def check_local_identity() -> list[tuple[str, str]]:
    def config_value(key: str) -> str:
        result = subprocess.run(
            ("git", "config", "--get", key),
            check=False,
            stdout=subprocess.PIPE,
        )
        return result.stdout.decode().strip() if result.returncode == 0 else ""

    name = config_value("user.name")
    email = config_value("user.email")
    return [] if (name, email) == (GENERIC_NAME, GENERIC_EMAIL) else [("git-config", "non-generic-identity")]


def self_test() -> None:
    assert not blob_rules(b"path=/workspace/user email=dev@example.invalid")
    assert "personal-home-path" in blob_rules(b"/Users/" + b"private-user/project")
    assert "personal-home-path" in blob_rules(b"/home/" + b"private-user/project")
    assert "forbidden-identity" in blob_rules(b"lada" + b"digit")
    assert "non-placeholder-email" in blob_rules(b"dev@" + b"example.com")
    assert path_rules("evidence/graphs/private.json")
    assert path_rules("docs/pilot/results/forced.json")
    pilot_case = b'{"schema": "codeclew-pilot-case/1.0"}\n'
    assert "filled-pilot-case" in blob_rules(pilot_case, "private-case.json")
    assert "filled-pilot-case" in blob_rules(pilot_case, PILOT_CASE_TEMPLATE)
    assert "filled-pilot-case" in blob_rules(
        b'{"schema":"codeclew-pilot-case-set/1.0"}\n', "private-set.json"
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--history", action="store_true")
    parser.add_argument("--pre-commit", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    entries = history_entries() if args.history else index_entries()
    findings = check_entries(entries)
    if args.history:
        findings.extend(check_history_metadata())
    if args.pre_commit:
        findings.extend(check_local_identity())
    for path, rule in sorted(set(findings)):
        print(f"privacy check failed: {path}: {rule}", file=sys.stderr)
    return 1 if findings else 0


if __name__ == "__main__":
    raise SystemExit(main())
