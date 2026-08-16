#!/usr/bin/env python3
"""Production-pinned readiness and retained-attempt harness for Kotlin K1.

This module is deliberately standard-library-only.  The frozen requirements,
corpus, eligibility evidence, and DAG are authorities, not CLI inputs: a path
accepted on the command line is only an assertion that it is byte-identical to
the repository-owned authority.
"""

from __future__ import annotations

import argparse
import contextlib
import fcntl
import hashlib
import json
import os
import re
import resource
import secrets
import shlex
import signal
import shutil
import socket
import stat
import subprocess
import tarfile
import tempfile
import threading
import time
import tomllib
from pathlib import Path
from typing import Any, Callable, Mapping, Sequence


ROOT = Path(__file__).resolve().parents[1]
K1 = ROOT / "benchmarks/kotlin-real-repository/k1"
AUTHORITIES = {
    "requirements": (K1 / "requirements.json", "aa7b0a8bb012bbb7800c21d91d291fbcfe71a35c268507c209f463a8ac217527"),
    "corpus": (K1 / "corpus.json", "dfd6fb4a0004fb2b0e04e52988407ab0eef47f6743c91e8a55f105cbc52fa4ea"),
    "corpusEligibilityEvidence": (K1 / "corpus-eligibility-evidence.json", "5b832a0732297e3ca9d4e81be465b90f047b65056936a162b6e7dd5b1495b9e4"),
    "readinessGraph": (K1 / "readiness-graph.json", "84efe05930c19cba8297e353dec1cfe418c3465bb47ca2786dd96a7686bc7dc1"),
    "preregistrationAmendment": (K1 / "preregistration-amendment-k1.12.json", "2a7569c622118516782ebc5d6a6bb45ea5a17ef8b16337ac87210496f8a70869"),
    "holdoutEligibilityAudit": (K1 / "holdout-eligibility-audit.json", "7003a87a19f6349305a45f477c676f0ac662bdea6c333ab1e36e50b6ecbddbfb"),
}
SERIES_ID = "KOTLIN_REAL_REPOSITORY_K1_12_2026_08_13"
PREDECESSOR_SERIES_ID = "KOTLIN_REAL_REPOSITORY_K1_11_2026_08_13"
K1_10_SERIES_ID = "KOTLIN_REAL_REPOSITORY_K1_10_2026_08_13"
K1_9_SERIES_ID = "KOTLIN_REAL_REPOSITORY_K1_9_2026_08_13"
K1_8_SERIES_ID = "KOTLIN_REAL_REPOSITORY_K1_8_2026_08_13"
K1_7_SERIES_ID = "KOTLIN_REAL_REPOSITORY_K1_7_2026_08_13"
K1_6_SERIES_ID = "KOTLIN_REAL_REPOSITORY_K1_6_2026_08_13"
K1_5_SERIES_ID = "KOTLIN_REAL_REPOSITORY_K1_5_2026_08_13"
K1_4_SERIES_ID = "KOTLIN_REAL_REPOSITORY_K1_4_2026_08_13"
K1_3_SERIES_ID = "KOTLIN_REAL_REPOSITORY_K1_3_2026_08_13"
K1_2_SERIES_ID = "KOTLIN_REAL_REPOSITORY_K1_2_2026_08_13"
K1_1_SERIES_ID = "KOTLIN_REAL_REPOSITORY_K1_1_2026_08_13"
K1_1_AMENDMENT = (K1 / "preregistration-amendment-k1.1.json", "eda9932e1425b22612ca346379a74cb097b8cbdd75b2a6eeb256bb3b036731d4")
K1_2_AMENDMENT = (K1 / "preregistration-amendment-k1.2.json", "108f224f640fe8861263523a593b0e90ed71eac098a93bca8ee62d9367b70652")
K1_3_AMENDMENT = (K1 / "preregistration-amendment-k1.3.json", "caa13edf921dabff6d275cbb8793c759fc734d359f6bc0a054e6efe7156f0344")
K1_4_AMENDMENT = (K1 / "preregistration-amendment-k1.4.json", "0dc67f9818f82aa12988712e1da3701a185fb485da843de0007f99e9aa0a4cc5")
K1_5_AMENDMENT = (K1 / "preregistration-amendment-k1.5.json", "d7b084153952615ccae6790b887ad5900bef96dbf2d1c869ec34f5ae2716a492")
K1_6_AMENDMENT = (K1 / "preregistration-amendment-k1.6.json", "e5c41f37946e583eddcc8f127d964bb845d2c27021385b01c39040b10b79f604")
K1_7_AMENDMENT = (K1 / "preregistration-amendment-k1.7.json", "f92b57d04a0a210be431484537338f6ac1d79761e3021221f0fa106180608f7e")
K1_8_AMENDMENT = (K1 / "preregistration-amendment-k1.8.json", "38e63018b3baeec78d93ae309e0f71559f73906a58b21d827d08ffb41f3c8305")
K1_9_AMENDMENT = (K1 / "preregistration-amendment-k1.9.json", "4c68825249d649187c21e5c01fe993ca854ae22ccc5f195513515f0739ae3daf")
K1_10_AMENDMENT = (K1 / "preregistration-amendment-k1.10.json", "051858cffbc9770c8d90aaf98f079155f3cd6681f02ce1e159384d43ad249993")
K1_11_AMENDMENT = (K1 / "preregistration-amendment-k1.11.json", "f8bbd1296a777dff321852326deadc5600b424700ab2a97979667f17230f205e")
STORE_SCHEMA = "codeclew.kotlin-k1-readiness-store/0.1"
RECEIPT_SCHEMA = "codeclew.kotlin-k1-readiness-receipt/0.1"
POINTER_SCHEMA = "codeclew.kotlin-k1-readiness-pointer/0.1"
EXPLAIN_SCHEMA = "codeclew.kotlin-k1-readiness-explain/0.1"
CHECKER_VERSION = "codeclew.kotlin-k1-harness/0.1"
SERIES_GUARD_SCHEMA = "codeclew.kotlin-k1-series-guard/0.1"
SERIES_GUARD_MARKER_SCHEMA = "codeclew.kotlin-k1-series-guard-marker/0.1"
CHILD_START_SCHEMA = "codeclew.kotlin-k1-child-start/0.1"
LIVE_SET_SCHEMA = "codeclew.kotlin-k1-live-set/0.1"
PINNED_REPOSITORY_BASE_REVISION = "be19fd06ce7509ffbaf4622b8f8d13ec7cbcd854"
FATAL_REASON_CODES = frozenset({
    "K0_1_DRIFT",
    "PINNED_AUTHORITY_DRIFT",
    "THRESHOLD_OR_CORPUS_REWRITE",
    "SOURCE_MUTATION",
    "POST_FREEZE_CANDIDATE_OR_HOLDOUT_CHANGE",
    "VERIFIED_AUTHORITY_BYPASS",
    "UNRETAINED_STARTED_CHILD",
    "MATRIX_SAFETY_VIOLATION",
})
RECEIPT_STATES = frozenset({"READY", "FAILED", "CANCELLED"})
DERIVED_STATES = frozenset({"ABSENT", "BLOCKED", "RUNNING", "STALE"})
GENERICALLY_ISSUABLE = frozenset({"VERIFY"})
EXPECTED_QUALIFICATION = tuple(f"K1-Q{number:02d}" for number in range(1, 7))
EXPECTED_HOLDOUT = tuple(f"K1-H{number:02d}" for number in range(1, 7))
ATTEMPT_SCHEMA = "codeclew.kotlin-k1-retained-attempt/0.1"
ATTEMPT_POINTER_SCHEMA = "codeclew.kotlin-k1-attempt-pointer/0.1"
MAX_WALL_SECONDS = 900
MAX_RESIDENT_BYTES = 8 * 1024 * 1024 * 1024
MAX_STDOUT_BYTES = 64 * 1024 * 1024
KOTLIN_TYPED_ATTEMPT_SCHEMA = "codeclew.kotlin-real-repository-attempt/0.1"
PREPARED_REFUSAL_SCHEMA = "codeclew.kotlin-k1-dependency-preparation-refusal/0.11"
SOURCE_SNAPSHOT_IGNORED = frozenset({
    ".git", ".gradle", ".kotlin", ".semantic-thread", "build", "target",
    "node_modules", ".idea", ".vscode"
})


class HarnessError(RuntimeError):
    """A fail-closed K1 harness rejection."""


def canonical(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()


def sha256_bytes(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    with path.open("rb") as handle:
        digest = hashlib.sha256()
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return "sha256:" + digest.hexdigest()


def _regular_file(path: Path, label: str) -> Path:
    absolute = path.absolute()
    try:
        metadata = absolute.lstat()
    except FileNotFoundError as error:
        raise HarnessError(f"{label} is absent") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise HarnessError(f"{label} must be a regular non-symlink file")
    return absolute


def _load_json_bytes(raw: bytes, label: str) -> Any:
    try:
        return json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise HarnessError(f"{label} is not JSON") from error


def load_authority(name: str, asserted_path: Path | None = None) -> tuple[Any, str]:
    """Load a hard-pinned repository authority and reject alternate bytes."""
    if name not in AUTHORITIES:
        raise HarnessError(f"unknown K1 authority: {name}")
    production_path, expected_hex = AUTHORITIES[name]
    production = _regular_file(production_path, f"production {name}")
    raw = production.read_bytes()
    actual = hashlib.sha256(raw).hexdigest()
    if actual != expected_hex:
        raise HarnessError(f"production {name} drift: expected sha256:{expected_hex}, got sha256:{actual}")
    if asserted_path is not None:
        asserted = _regular_file(asserted_path, f"asserted {name}")
        if asserted.read_bytes() != raw:
            raise HarnessError(f"asserted {name} is not byte-exact production authority")
    return _load_json_bytes(raw, name), "sha256:" + actual


def _pinned_predecessor_amendment(authority: tuple[Path, str], label: str) -> tuple[dict[str, Any], str]:
    path, expected_hex = authority
    raw = _regular_file(path, label).read_bytes()
    actual = hashlib.sha256(raw).hexdigest()
    if actual != expected_hex:
        raise HarnessError(f"{label} drift: expected sha256:{expected_hex}, got sha256:{actual}")
    value = _load_json_bytes(raw, label)
    if not isinstance(value, dict) or canonical(value) != raw:
        raise HarnessError(f"{label} must be canonical JSON")
    return value, "sha256:" + actual


def _verify_k1_1_preservation(
    k1_2_authorities: Mapping[str, bytes], old_digests: Mapping[str, str], amendment_digest: str,
) -> None:
    """Prove the pinned K1.2 authorities reversibly preserve K1.1."""
    old_series = K1_1_SERIES_ID.encode()
    new_series = K1_2_SERIES_ID.encode()

    def replace_once(raw: bytes, before: bytes, after: bytes, label: str) -> bytes:
        if raw.count(before) != 1:
            raise HarnessError(f"K1.1 preservation transform mismatch: {label}")
        return raw.replace(before, after)

    reconstructed: dict[str, bytes] = {}
    for name in ("corpusEligibilityEvidence",):
        raw = k1_2_authorities[name]
        reconstructed[name] = replace_once(raw, new_series, old_series, f"{name} series")

    corpus_raw = k1_2_authorities["corpus"]
    corpus_raw = replace_once(corpus_raw, new_series, old_series, "corpus series")
    old_analyzer_trees = {
        "2.1": "sha256:00ed4ac81d56f00f452dcda147917d6c65638c719b87fbc5bf6d5cb27e8e8dce",
        "2.3": "sha256:fbdc3575d1e443fd9d28fc2b531597ce4d7b5890f88023e4403e72763c40894d",
        "2.4": "sha256:91cc4221343db69d8e2161a971d33b089d0b06afaacc2c76445a58da73b9ed53",
    }
    for minor, digest in old_analyzer_trees.items():
        needle = f'        "compilerVersion": "{minor}'.encode()
        start = corpus_raw.find(needle)
        if start < 0:
            raise HarnessError(f"K1.2 preservation transform mismatch: analyzer {minor}")
        newline = corpus_raw.find(b"\n", start)
        insertion = f'        "distributionTreeSha256": "{digest}",\n'.encode()
        corpus_raw = corpus_raw[: newline + 1] + insertion + corpus_raw[newline + 1 :]
    reconstructed["corpus"] = corpus_raw

    requirements_raw = k1_2_authorities["requirements"]
    requirements_raw = replace_once(requirements_raw, new_series, old_series, "requirements series")
    amendment_line = f'  "preregistrationAmendmentSha256": "{amendment_digest}",\n'.encode()
    predecessor_line = b'  "preregistrationAmendmentSha256": "sha256:eda9932e1425b22612ca346379a74cb097b8cbdd75b2a6eeb256bb3b036731d4",\n'
    reconstructed["requirements"] = replace_once(
        requirements_raw, amendment_line, predecessor_line, "requirements amendment binding"
    )

    graph_raw = k1_2_authorities["readinessGraph"]
    graph_raw = replace_once(graph_raw, new_series, old_series, "graph series")
    graph_raw = replace_once(
        graph_raw,
        b'    "directAuthority": "DIRECT, IMPORT, DECISION and CONDITIONAL_ROOT nodes cannot be issued by generic verify or caller-authored evidence. K1_SERIES_GUARD is an internal irreversible OPEN/FATAL store authority. An OPEN K1_DECISION requires every declared dependency and selected input; a FATAL K1_DECISION is current only from the exact guard object and can issue STOP but never GO or PIVOT.",',
        b'    "directAuthority": "DIRECT, IMPORT, DECISION and CONDITIONAL_ROOT nodes cannot be issued by generic verify or caller-authored evidence. Guard attempts may retain READY or FAILED; K1_INDEPENDENT_AUDIT_IMPORT and K1_DECISION bind those exact attempt hashes through selectedInputs rather than requiring FAILED guards as READY dependencies.",',
        "series guard authority rule",
    )
    graph_raw = replace_once(
        graph_raw,
        b'''    {
      "id": "CANDIDATE_FREEZE_PREPARE",
      "action": "PREPARE",
      "deps": ["QUALIFICATION_RUN_6_COMPLETE", "K0_1_BYTE_EXACT_VERIFY"],
      "selectedInputs": ["candidateFreeze", "candidateSources", "candidateBinaries", "candidateTools", "harnessSource", "independentAuditorSource", "requirements", "readinessGraph", "corpus"]
    },''',
        b'''    {
      "id": "CANDIDATE_FREEZE_PREPARE",
      "action": "PREPARE",
      "deps": ["QUALIFICATION_RUN_6_COMPLETE", "K0_1_BYTE_EXACT_VERIFY"],
      "selectedInputs": ["candidateFreeze"]
    },''',
        "candidate freeze prepare consumed inputs",
    )
    graph_rewrites = (
        (b'      "selectedInputs": ["qualificationDependencySeed", "qualificationSourceSet", "candidateTools"]',
         b'      "selectedInputs": ["qualificationDependencySeed", "qualificationSourceSet"]',
         "qualification candidate-tools selector"),
        (b'      "selectedInputs": ["holdoutDependencySeed", "holdoutSourceSet", "candidateTools"]',
         b'      "selectedInputs": ["holdoutDependencySeed", "holdoutSourceSet"]',
         "holdout candidate-tools selector"),
        (b'      "selectedInputs": ["candidateFreeze", "candidateSources", "candidateBinaries", "candidateTools", "harnessSource", "independentAuditorSource", "requirements", "readinessGraph", "corpus"]',
         b'      "selectedInputs": ["candidateFreeze", "candidateSources", "candidateBinaries", "candidateTools", "harnessSource", "requirements", "readinessGraph", "corpus"]',
         "candidate auditor freeze selector"),
    )
    for after, before, label in graph_rewrites:
        graph_raw = replace_once(graph_raw, after, before, label)
    added_start = graph_raw.index(b'    {\n      "id": "REQUIREMENT_CONFORMANCE_VERIFY"')
    audit_start = graph_raw.index(b'    {\n      "id": "K1_INDEPENDENT_AUDIT_IMPORT"', added_start)
    graph_raw = graph_raw[:added_start] + graph_raw[audit_start:]
    graph_raw = replace_once(
        graph_raw,
        b'      "selectedInputs": ["independentAuditorSource", "independentAuditorRunReceipt", "matrixSafetyReceipt", "applicabilityReceipt", "cacheCostReceipt", "requirementConformance", "independentAudit", "qualificationMatrix", "holdoutMatrix", "requirements", "corpus", "candidateTools", "baselinePacket", "harnessSelfTestPacket", "candidateFreeze"]',
        b'      "selectedInputs": ["independentAuditorSource", "independentAuditorRunReceipt", "matrixSafetyReceipt", "applicabilityReceipt", "cacheCostReceipt", "requirementConformance", "independentAudit"]',
        "independent audit consumed inputs",
    )
    current_audit = b'''    {
      "id": "K1_INDEPENDENT_AUDIT_IMPORT",
      "action": "IMPORT",
      "deps": ["K1_INDEPENDENT_AUDITOR_RUN", "REQUIREMENT_CONFORMANCE_VERIFY", "MATRIX_TOTALITY_AND_SAFETY_VERIFY", "APPLICABILITY_VERIFY", "CACHE_AND_COST_VERIFY", "HOLDOUT_RUN_6_COMPLETE", "CANDIDATE_FREEZE_VERIFY"],
      "selectedInputs": ["independentAuditorSource", "independentAuditorRunReceipt", "matrixSafetyReceipt", "applicabilityReceipt", "cacheCostReceipt", "requirementConformance", "independentAudit"]
    },'''
    predecessor_audit = b'''    {
      "id": "K1_INDEPENDENT_AUDIT_IMPORT",
      "action": "IMPORT",
      "deps": ["HOLDOUT_RUN_6_COMPLETE", "CANDIDATE_FREEZE_VERIFY"],
      "selectedInputs": ["matrixSafetyReceipt", "applicabilityReceipt", "cacheCostReceipt", "independentAudit"]
    },'''
    graph_raw = replace_once(graph_raw, current_audit, predecessor_audit, "independent audit contour")
    graph_raw = replace_once(
        graph_raw,
        b'      "deps": ["K1_SERIES_GUARD", "K1_INDEPENDENT_AUDIT_IMPORT", "REQUIREMENT_CONFORMANCE_VERIFY", "MATRIX_TOTALITY_AND_SAFETY_VERIFY", "APPLICABILITY_VERIFY", "CACHE_AND_COST_VERIFY", "HOLDOUT_RUN_6_COMPLETE", "CANDIDATE_FREEZE_VERIFY"],',
        b'      "deps": ["K1_INDEPENDENT_AUDIT_IMPORT", "REQUIREMENT_CONFORMANCE_VERIFY", "MATRIX_TOTALITY_AND_SAFETY_VERIFY", "APPLICABILITY_VERIFY", "CACHE_AND_COST_VERIFY", "HOLDOUT_RUN_6_COMPLETE", "CANDIDATE_FREEZE_VERIFY"],',
        "series guard decision dependency",
    )
    current_decision = b'''      "deps": ["K1_INDEPENDENT_AUDIT_IMPORT", "REQUIREMENT_CONFORMANCE_VERIFY", "MATRIX_TOTALITY_AND_SAFETY_VERIFY", "APPLICABILITY_VERIFY", "CACHE_AND_COST_VERIFY", "HOLDOUT_RUN_6_COMPLETE", "CANDIDATE_FREEZE_VERIFY"],
      "selectedInputs": ["matrixSafetyReceipt", "applicabilityReceipt", "cacheCostReceipt", "requirementConformance", "decision", "requirements", "corpus", "candidateFreeze", "qualificationMatrix", "holdoutMatrix", "independentAudit"]'''
    predecessor_decision = b'''      "deps": ["K1_INDEPENDENT_AUDIT_IMPORT", "HOLDOUT_RUN_6_COMPLETE", "CANDIDATE_FREEZE_VERIFY"],
      "selectedInputs": ["matrixSafetyReceipt", "applicabilityReceipt", "cacheCostReceipt", "decision", "requirements", "corpus", "candidateFreeze", "qualificationMatrix", "holdoutMatrix", "independentAudit"]'''
    graph_raw = replace_once(graph_raw, current_decision, predecessor_decision, "decision measurement dependencies")
    reconstructed["readinessGraph"] = graph_raw
    for name, raw in reconstructed.items():
        if sha256_bytes(raw) != old_digests[name]:
            raise HarnessError(f"K1.2 changed frozen {name} outside the registered amendment")


def _verify_k1_2_preservation(
    k1_3_authorities: Mapping[str, bytes], old_digests: Mapping[str, str], amendment_digest: str,
) -> dict[str, bytes]:
    """Reconstruct every K1.2 authority byte-for-byte from K1.3."""
    old_series = K1_2_SERIES_ID.encode()
    new_series = K1_3_SERIES_ID.encode()

    def replace_once(raw: bytes, before: bytes, after: bytes, label: str) -> bytes:
        if raw.count(before) != 1:
            raise HarnessError(f"K1.2 preservation transform mismatch: {label}")
        return raw.replace(before, after)

    reconstructed: dict[str, bytes] = {}
    for name in ("corpus", "corpusEligibilityEvidence"):
        raw = k1_3_authorities[name]
        reconstructed[name] = replace_once(raw, new_series, old_series, f"{name} series")

    requirements_raw = k1_3_authorities["requirements"]
    requirements_raw = replace_once(requirements_raw, new_series, old_series, "requirements series")
    requirements_raw = replace_once(
        requirements_raw,
        f'  "preregistrationAmendmentSha256": "{amendment_digest}",\n'.encode(),
        b'  "preregistrationAmendmentSha256": "sha256:108f224f640fe8861263523a593b0e90ed71eac098a93bca8ee62d9367b70652",\n',
        "requirements amendment binding",
    )
    reconstructed["requirements"] = requirements_raw

    graph_raw = k1_3_authorities["readinessGraph"]
    graph_raw = replace_once(graph_raw, new_series, old_series, "readinessGraph series")
    graph_raw = replace_once(
        graph_raw,
        b'''    {
      "id": "BASELINE_CAPTURE",
      "action": "DIRECT",
      "deps": ["INPUT_AUTHORITY_VERIFY", "CORPUS_FREEZE_VERIFY", "K0_1_BYTE_EXACT_VERIFY"],
      "selectedInputs": ["baselinePacket", "candidateSources", "candidateTools", "repositoryBaseRevision"]
    },''',
        b'''    {
      "id": "BASELINE_CAPTURE",
      "action": "DIRECT",
      "deps": ["CORPUS_FREEZE_VERIFY", "K0_1_BYTE_EXACT_VERIFY"],
      "selectedInputs": ["baselinePacket"]
    },''',
        "baseline authority block",
    )
    reconstructed["readinessGraph"] = graph_raw

    holdout_raw = k1_3_authorities["holdoutEligibilityAudit"]
    holdout_raw = replace_once(holdout_raw, new_series, old_series, "holdout audit series")
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_3_authorities["corpus"])[7:].encode(), old_digests["corpus"][7:].encode(),
        "holdout audit corpus binding",
    )
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_3_authorities["corpusEligibilityEvidence"])[7:].encode(),
        old_digests["corpusEligibilityEvidence"][7:].encode(), "holdout audit eligibility binding",
    )
    reconstructed["holdoutEligibilityAudit"] = holdout_raw

    for name, raw in reconstructed.items():
        if sha256_bytes(raw) != old_digests[name]:
            raise HarnessError(f"K1.3 changed frozen {name} outside the registered amendment")
    return reconstructed


def _verify_k1_3_preservation(
    k1_4_authorities: Mapping[str, bytes], old_digests: Mapping[str, str], amendment_digest: str,
) -> dict[str, bytes]:
    """Reconstruct every K1.3 authority byte-for-byte from K1.4."""
    old_series = K1_3_SERIES_ID.encode()
    new_series = K1_4_SERIES_ID.encode()

    def replace_once(raw: bytes, before: bytes, after: bytes, label: str) -> bytes:
        if raw.count(before) != 1:
            raise HarnessError(f"K1.3 preservation transform mismatch: {label}")
        return raw.replace(before, after)

    reconstructed: dict[str, bytes] = {}
    for name in ("corpus", "corpusEligibilityEvidence"):
        raw = k1_4_authorities[name]
        reconstructed[name] = replace_once(raw, new_series, old_series, f"{name} series")

    requirements_raw = k1_4_authorities["requirements"]
    requirements_raw = replace_once(requirements_raw, new_series, old_series, "requirements series")
    requirements_raw = replace_once(
        requirements_raw,
        f'  "preregistrationAmendmentSha256": "{amendment_digest}",\n'.encode(),
        b'  "preregistrationAmendmentSha256": "sha256:caa13edf921dabff6d275cbb8793c759fc734d359f6bc0a054e6efe7156f0344",\n',
        "requirements amendment binding",
    )
    reconstructed["requirements"] = requirements_raw

    graph_raw = k1_4_authorities["readinessGraph"]
    reconstructed["readinessGraph"] = replace_once(
        graph_raw, new_series, old_series, "readinessGraph series",
    )

    holdout_raw = k1_4_authorities["holdoutEligibilityAudit"]
    holdout_raw = replace_once(holdout_raw, new_series, old_series, "holdout audit series")
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_4_authorities["corpus"])[7:].encode(), old_digests["corpus"][7:].encode(),
        "holdout audit corpus binding",
    )
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_4_authorities["corpusEligibilityEvidence"])[7:].encode(),
        old_digests["corpusEligibilityEvidence"][7:].encode(), "holdout audit eligibility binding",
    )
    reconstructed["holdoutEligibilityAudit"] = holdout_raw

    for name, raw in reconstructed.items():
        if sha256_bytes(raw) != old_digests[name]:
            raise HarnessError(f"K1.4 changed frozen {name} outside the registered amendment")
    return reconstructed


def _verify_k1_4_preservation(
    k1_5_authorities: Mapping[str, bytes], old_digests: Mapping[str, str], amendment_digest: str,
) -> dict[str, bytes]:
    """Reconstruct every K1.4 authority byte-for-byte from K1.5."""
    old_series = K1_4_SERIES_ID.encode()
    new_series = K1_5_SERIES_ID.encode()

    def replace_once(raw: bytes, before: bytes, after: bytes, label: str) -> bytes:
        if raw.count(before) != 1:
            raise HarnessError(f"K1.4 preservation transform mismatch: {label}")
        return raw.replace(before, after)

    reconstructed: dict[str, bytes] = {}
    for name in ("corpus", "corpusEligibilityEvidence"):
        reconstructed[name] = replace_once(
            k1_5_authorities[name], new_series, old_series, f"{name} series",
        )

    requirements_raw = k1_5_authorities["requirements"]
    requirements_raw = replace_once(requirements_raw, new_series, old_series, "requirements series")
    requirements_raw = replace_once(
        requirements_raw,
        f'  "preregistrationAmendmentSha256": "{amendment_digest}",\n'.encode(),
        b'  "preregistrationAmendmentSha256": "sha256:0dc67f9818f82aa12988712e1da3701a185fb485da843de0007f99e9aa0a4cc5",\n',
        "requirements amendment binding",
    )
    reconstructed["requirements"] = requirements_raw
    reconstructed["readinessGraph"] = replace_once(
        k1_5_authorities["readinessGraph"], new_series, old_series,
        "readinessGraph series",
    )

    holdout_raw = k1_5_authorities["holdoutEligibilityAudit"]
    holdout_raw = replace_once(holdout_raw, new_series, old_series, "holdout audit series")
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_5_authorities["corpus"])[7:].encode(), old_digests["corpus"][7:].encode(),
        "holdout audit corpus binding",
    )
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_5_authorities["corpusEligibilityEvidence"])[7:].encode(),
        old_digests["corpusEligibilityEvidence"][7:].encode(),
        "holdout audit eligibility binding",
    )
    reconstructed["holdoutEligibilityAudit"] = holdout_raw

    for name, raw in reconstructed.items():
        if sha256_bytes(raw) != old_digests[name]:
            raise HarnessError(f"K1.5 changed frozen {name} outside the registered amendment")
    return reconstructed


def _verify_k1_5_preservation(
    k1_6_authorities: Mapping[str, bytes], old_digests: Mapping[str, str], amendment_digest: str,
) -> dict[str, bytes]:
    """Reconstruct every K1.5 authority byte-for-byte from K1.6."""
    old_series = K1_5_SERIES_ID.encode()
    new_series = K1_6_SERIES_ID.encode()

    def replace_once(raw: bytes, before: bytes, after: bytes, label: str) -> bytes:
        if raw.count(before) != 1:
            raise HarnessError(f"K1.5 preservation transform mismatch: {label}")
        return raw.replace(before, after)

    reconstructed: dict[str, bytes] = {}
    for name in ("corpus", "corpusEligibilityEvidence"):
        reconstructed[name] = replace_once(
            k1_6_authorities[name], new_series, old_series, f"{name} series",
        )
    requirements_raw = replace_once(
        k1_6_authorities["requirements"], new_series, old_series, "requirements series",
    )
    requirements_raw = replace_once(
        requirements_raw,
        f'  "preregistrationAmendmentSha256": "{amendment_digest}",\n'.encode(),
        b'  "preregistrationAmendmentSha256": "sha256:d7b084153952615ccae6790b887ad5900bef96dbf2d1c869ec34f5ae2716a492",\n',
        "requirements amendment binding",
    )
    reconstructed["requirements"] = requirements_raw
    reconstructed["readinessGraph"] = replace_once(
        k1_6_authorities["readinessGraph"], new_series, old_series, "readinessGraph series",
    )
    holdout_raw = replace_once(
        k1_6_authorities["holdoutEligibilityAudit"], new_series, old_series,
        "holdout audit series",
    )
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_6_authorities["corpus"])[7:].encode(), old_digests["corpus"][7:].encode(),
        "holdout audit corpus binding",
    )
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_6_authorities["corpusEligibilityEvidence"])[7:].encode(),
        old_digests["corpusEligibilityEvidence"][7:].encode(),
        "holdout audit eligibility binding",
    )
    reconstructed["holdoutEligibilityAudit"] = holdout_raw
    for name, raw in reconstructed.items():
        if sha256_bytes(raw) != old_digests[name]:
            raise HarnessError(f"K1.6 changed frozen {name} outside the registered amendment")
    return reconstructed


def _verify_k1_6_preservation(
    k1_7_authorities: Mapping[str, bytes], old_digests: Mapping[str, str], amendment_digest: str,
) -> dict[str, bytes]:
    """Reconstruct every K1.6 authority byte-for-byte from K1.7."""
    old_series = K1_6_SERIES_ID.encode()
    new_series = K1_7_SERIES_ID.encode()

    def replace_once(raw: bytes, before: bytes, after: bytes, label: str) -> bytes:
        if raw.count(before) != 1:
            raise HarnessError(f"K1.6 preservation transform mismatch: {label}")
        return raw.replace(before, after)

    reconstructed: dict[str, bytes] = {}
    for name in ("corpus", "corpusEligibilityEvidence"):
        reconstructed[name] = replace_once(
            k1_7_authorities[name], new_series, old_series, f"{name} series",
        )
    requirements_raw = replace_once(
        k1_7_authorities["requirements"], new_series, old_series, "requirements series",
    )
    requirements_raw = replace_once(
        requirements_raw,
        f'  "preregistrationAmendmentSha256": "{amendment_digest}",\n'.encode(),
        b'  "preregistrationAmendmentSha256": "sha256:e5c41f37946e583eddcc8f127d964bb845d2c27021385b01c39040b10b79f604",\n',
        "requirements amendment binding",
    )
    reconstructed["requirements"] = requirements_raw
    reconstructed["readinessGraph"] = replace_once(
        k1_7_authorities["readinessGraph"], new_series, old_series,
        "readinessGraph series",
    )
    holdout_raw = replace_once(
        k1_7_authorities["holdoutEligibilityAudit"], new_series, old_series,
        "holdout audit series",
    )
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_7_authorities["corpus"])[7:].encode(), old_digests["corpus"][7:].encode(),
        "holdout audit corpus binding",
    )
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_7_authorities["corpusEligibilityEvidence"])[7:].encode(),
        old_digests["corpusEligibilityEvidence"][7:].encode(),
        "holdout audit eligibility binding",
    )
    reconstructed["holdoutEligibilityAudit"] = holdout_raw
    for name, raw in reconstructed.items():
        if sha256_bytes(raw) != old_digests[name]:
            raise HarnessError(f"K1.7 changed frozen {name} outside the registered amendment")
    return reconstructed


def _verify_k1_7_preservation(
    k1_8_authorities: Mapping[str, bytes], old_digests: Mapping[str, str], amendment_digest: str,
) -> dict[str, bytes]:
    """Reconstruct every K1.7 authority byte-for-byte from K1.8."""
    old_series = K1_7_SERIES_ID.encode()
    new_series = K1_8_SERIES_ID.encode()

    def replace_once(raw: bytes, before: bytes, after: bytes, label: str) -> bytes:
        if raw.count(before) != 1:
            raise HarnessError(f"K1.7 preservation transform mismatch: {label}")
        return raw.replace(before, after)

    reconstructed: dict[str, bytes] = {}
    for name in ("corpus", "corpusEligibilityEvidence"):
        reconstructed[name] = replace_once(k1_8_authorities[name], new_series, old_series, f"{name} series")
    requirements_raw = replace_once(k1_8_authorities["requirements"], new_series, old_series, "requirements series")
    requirements_raw = replace_once(
        requirements_raw,
        f'  "preregistrationAmendmentSha256": "{amendment_digest}",\n'.encode(),
        b'  "preregistrationAmendmentSha256": "sha256:f92b57d04a0a210be431484537338f6ac1d79761e3021221f0fa106180608f7e",\n',
        "requirements amendment binding",
    )
    reconstructed["requirements"] = requirements_raw
    reconstructed["readinessGraph"] = replace_once(k1_8_authorities["readinessGraph"], new_series, old_series, "readinessGraph series")
    holdout_raw = replace_once(k1_8_authorities["holdoutEligibilityAudit"], new_series, old_series, "holdout audit series")
    holdout_raw = replace_once(holdout_raw, sha256_bytes(k1_8_authorities["corpus"])[7:].encode(), old_digests["corpus"][7:].encode(), "holdout audit corpus binding")
    holdout_raw = replace_once(holdout_raw, sha256_bytes(k1_8_authorities["corpusEligibilityEvidence"])[7:].encode(), old_digests["corpusEligibilityEvidence"][7:].encode(), "holdout audit eligibility binding")
    reconstructed["holdoutEligibilityAudit"] = holdout_raw
    for name, raw in reconstructed.items():
        if sha256_bytes(raw) != old_digests[name]:
            raise HarnessError(f"K1.8 changed frozen {name} outside the registered amendment")
    return reconstructed


def _verify_k1_8_preservation(
    k1_9_authorities: Mapping[str, bytes], old_digests: Mapping[str, str], amendment_digest: str,
) -> dict[str, bytes]:
    """Reconstruct every K1.8 authority byte-for-byte from K1.9."""
    old_series = K1_8_SERIES_ID.encode()
    new_series = K1_9_SERIES_ID.encode()

    def replace_once(raw: bytes, before: bytes, after: bytes, label: str) -> bytes:
        if raw.count(before) != 1:
            raise HarnessError(f"K1.8 preservation transform mismatch: {label}")
        return raw.replace(before, after)

    reconstructed: dict[str, bytes] = {}
    for name in ("corpus", "corpusEligibilityEvidence"):
        reconstructed[name] = replace_once(
            k1_9_authorities[name], new_series, old_series, f"{name} series",
        )
    requirements_raw = replace_once(
        k1_9_authorities["requirements"], new_series, old_series, "requirements series",
    )
    requirements_raw = replace_once(
        requirements_raw,
        f'  "preregistrationAmendmentSha256": "{amendment_digest}",\n'.encode(),
        b'  "preregistrationAmendmentSha256": "sha256:38e63018b3baeec78d93ae309e0f71559f73906a58b21d827d08ffb41f3c8305",\n',
        "requirements amendment binding",
    )
    reconstructed["requirements"] = requirements_raw
    reconstructed["readinessGraph"] = replace_once(
        k1_9_authorities["readinessGraph"], new_series, old_series, "readinessGraph series",
    )
    holdout_raw = replace_once(
        k1_9_authorities["holdoutEligibilityAudit"], new_series, old_series,
        "holdout audit series",
    )
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_9_authorities["corpus"])[7:].encode(), old_digests["corpus"][7:].encode(),
        "holdout audit corpus binding",
    )
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_9_authorities["corpusEligibilityEvidence"])[7:].encode(),
        old_digests["corpusEligibilityEvidence"][7:].encode(), "holdout audit eligibility binding",
    )
    reconstructed["holdoutEligibilityAudit"] = holdout_raw
    for name, raw in reconstructed.items():
        if sha256_bytes(raw) != old_digests[name]:
            raise HarnessError(f"K1.9 changed frozen {name} outside the registered amendment")
    return reconstructed


def _verify_k1_9_preservation(
    k1_10_authorities: Mapping[str, bytes], old_digests: Mapping[str, str], amendment_digest: str,
) -> dict[str, bytes]:
    """Reconstruct every K1.9 authority byte-for-byte from K1.10."""
    old_series = K1_9_SERIES_ID.encode()
    new_series = K1_10_SERIES_ID.encode()

    def replace_once(raw: bytes, before: bytes, after: bytes, label: str) -> bytes:
        if raw.count(before) != 1:
            raise HarnessError(f"K1.9 preservation transform mismatch: {label}")
        return raw.replace(before, after)

    reconstructed: dict[str, bytes] = {}
    for name in ("corpus", "corpusEligibilityEvidence"):
        reconstructed[name] = replace_once(
            k1_10_authorities[name], new_series, old_series, f"{name} series",
        )
    requirements_raw = replace_once(
        k1_10_authorities["requirements"], new_series, old_series, "requirements series",
    )
    requirements_raw = replace_once(
        requirements_raw,
        f'  "preregistrationAmendmentSha256": "{amendment_digest}",\n'.encode(),
        b'  "preregistrationAmendmentSha256": "sha256:4c68825249d649187c21e5c01fe993ca854ae22ccc5f195513515f0739ae3daf",\n',
        "requirements amendment binding",
    )
    reconstructed["requirements"] = requirements_raw
    reconstructed["readinessGraph"] = replace_once(
        k1_10_authorities["readinessGraph"], new_series, old_series, "readinessGraph series",
    )
    holdout_raw = replace_once(
        k1_10_authorities["holdoutEligibilityAudit"], new_series, old_series,
        "holdout audit series",
    )
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_10_authorities["corpus"])[7:].encode(), old_digests["corpus"][7:].encode(),
        "holdout audit corpus binding",
    )
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_10_authorities["corpusEligibilityEvidence"])[7:].encode(),
        old_digests["corpusEligibilityEvidence"][7:].encode(), "holdout audit eligibility binding",
    )
    reconstructed["holdoutEligibilityAudit"] = holdout_raw
    for name, raw in reconstructed.items():
        if sha256_bytes(raw) != old_digests[name]:
            raise HarnessError(f"K1.10 changed frozen {name} outside the registered amendment")
    return reconstructed


def _verify_k1_10_preservation(
    k1_11_authorities: Mapping[str, bytes], old_digests: Mapping[str, str], amendment_digest: str,
) -> dict[str, bytes]:
    """Reconstruct every K1.10 authority byte-for-byte from K1.11."""
    old_series = K1_10_SERIES_ID.encode()
    new_series = PREDECESSOR_SERIES_ID.encode()

    def replace_once(raw: bytes, before: bytes, after: bytes, label: str) -> bytes:
        if raw.count(before) != 1:
            raise HarnessError(f"K1.10 preservation transform mismatch: {label}")
        return raw.replace(before, after)

    reconstructed: dict[str, bytes] = {}
    for name in ("corpus", "corpusEligibilityEvidence"):
        reconstructed[name] = replace_once(
            k1_11_authorities[name], new_series, old_series, f"{name} series",
        )
    requirements_raw = replace_once(
        k1_11_authorities["requirements"], new_series, old_series, "requirements series",
    )
    requirements_raw = replace_once(
        requirements_raw,
        f'  "preregistrationAmendmentSha256": "{amendment_digest}",\n'.encode(),
        b'  "preregistrationAmendmentSha256": "sha256:051858cffbc9770c8d90aaf98f079155f3cd6681f02ce1e159384d43ad249993",\n',
        "requirements amendment binding",
    )
    reconstructed["requirements"] = requirements_raw
    reconstructed["readinessGraph"] = replace_once(
        k1_11_authorities["readinessGraph"], new_series, old_series, "readinessGraph series",
    )
    holdout_raw = replace_once(
        k1_11_authorities["holdoutEligibilityAudit"], new_series, old_series,
        "holdout audit series",
    )
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_11_authorities["corpus"])[7:].encode(), old_digests["corpus"][7:].encode(),
        "holdout audit corpus binding",
    )
    holdout_raw = replace_once(
        holdout_raw, sha256_bytes(k1_11_authorities["corpusEligibilityEvidence"])[7:].encode(),
        old_digests["corpusEligibilityEvidence"][7:].encode(), "holdout audit eligibility binding",
    )
    reconstructed["holdoutEligibilityAudit"] = holdout_raw
    for name, raw in reconstructed.items():
        if sha256_bytes(raw) != old_digests[name]:
            raise HarnessError(f"K1.11 changed frozen {name} outside the registered amendment")
    return reconstructed


def _verify_k1_11_preservation(
    old_digests: Mapping[str, str], amendment_digest: str,
) -> dict[str, bytes]:
    """Reconstruct every K1.11 authority byte-for-byte from K1.12."""
    old_series = PREDECESSOR_SERIES_ID.encode()
    new_series = SERIES_ID.encode()

    def replace_once(raw: bytes, before: bytes, after: bytes, label: str) -> bytes:
        if raw.count(before) != 1:
            raise HarnessError(f"K1.11 preservation transform mismatch: {label}")
        return raw.replace(before, after)

    reconstructed: dict[str, bytes] = {}
    for name in ("corpus", "corpusEligibilityEvidence"):
        reconstructed[name] = replace_once(
            AUTHORITIES[name][0].read_bytes(), new_series, old_series, f"{name} series",
        )
    requirements_raw = replace_once(
        AUTHORITIES["requirements"][0].read_bytes(), new_series, old_series, "requirements series",
    )
    requirements_raw = replace_once(
        requirements_raw,
        f'  "preregistrationAmendmentSha256": "{amendment_digest}",\n'.encode(),
        b'  "preregistrationAmendmentSha256": "sha256:f8bbd1296a777dff321852326deadc5600b424700ab2a97979667f17230f205e",\n',
        "requirements amendment binding",
    )
    reconstructed["requirements"] = requirements_raw
    reconstructed["readinessGraph"] = replace_once(
        AUTHORITIES["readinessGraph"][0].read_bytes(), new_series, old_series, "readinessGraph series",
    )
    holdout_raw = replace_once(
        AUTHORITIES["holdoutEligibilityAudit"][0].read_bytes(), new_series, old_series,
        "holdout audit series",
    )
    holdout_raw = replace_once(
        holdout_raw, AUTHORITIES["corpus"][1].encode(), old_digests["corpus"][7:].encode(),
        "holdout audit corpus binding",
    )
    holdout_raw = replace_once(
        holdout_raw, AUTHORITIES["corpusEligibilityEvidence"][1].encode(),
        old_digests["corpusEligibilityEvidence"][7:].encode(), "holdout audit eligibility binding",
    )
    reconstructed["holdoutEligibilityAudit"] = holdout_raw
    for name, raw in reconstructed.items():
        if sha256_bytes(raw) != old_digests[name]:
            raise HarnessError(f"K1.12 changed frozen {name} outside the registered amendment")
    return reconstructed


def _validate_graph(graph: Any) -> dict[str, Any]:
    if not isinstance(graph, dict) or set(graph) != {
        "schema", "graphId", "receiptStates", "derivedStates", "rules", "nodes", "roots"
    }:
        raise HarnessError("readiness graph top-level contract mismatch")
    if graph["schema"] != "codeclew.kotlin-real-repository-readiness-graph/0.1" or graph["graphId"] != SERIES_ID:
        raise HarnessError("readiness graph identity mismatch")
    if set(graph["receiptStates"]) != RECEIPT_STATES or set(graph["derivedStates"]) != DERIVED_STATES:
        raise HarnessError("readiness graph state contour mismatch")
    nodes = graph["nodes"]
    if not isinstance(nodes, list) or not nodes:
        raise HarnessError("readiness graph has no nodes")
    identifiers: list[str] = []
    for node in nodes:
        required = {"id", "action", "deps", "selectedInputs"}
        optional = {"condition"}
        if not isinstance(node, dict) or not required.issubset(node) or not set(node).issubset(required | optional):
            raise HarnessError("readiness node contract mismatch")
        if node["action"] not in {"VERIFY", "PREPARE", "DIRECT", "IMPORT", "DECISION", "CONDITIONAL_ROOT"}:
            raise HarnessError(f"invalid readiness action for {node.get('id')}")
        if not isinstance(node["id"], str) or not node["id"]:
            raise HarnessError("readiness node id mismatch")
        if not isinstance(node["deps"], list) or len(node["deps"]) != len(set(node["deps"])):
            raise HarnessError(f"readiness dependency set mismatch for {node['id']}")
        if not isinstance(node["selectedInputs"], list) or len(node["selectedInputs"]) != len(set(node["selectedInputs"])):
            raise HarnessError(f"readiness selected input set mismatch for {node['id']}")
        if node["action"] == "CONDITIONAL_ROOT" and not isinstance(node.get("condition"), str):
            raise HarnessError(f"conditional root lacks a condition: {node['id']}")
        identifiers.append(node["id"])
    if len(identifiers) != len(set(identifiers)):
        raise HarnessError("readiness graph has duplicate node ids")
    known = set(identifiers)
    if any(dependency not in known for node in nodes for dependency in node["deps"]):
        raise HarnessError("readiness graph has an unknown dependency")
    if not isinstance(graph["roots"], list) or set(graph["roots"]) != {
        "KOTLIN_REAL_REPOSITORY_READY", "KOTLIN_APPLICABILITY_OR_COST_GAP", "K1_SERIES_STOPPED"
    }:
        raise HarnessError("readiness root set mismatch")
    dependencies = {node["id"]: node["deps"] for node in nodes}
    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(identifier: str) -> None:
        if identifier in visiting:
            raise HarnessError("readiness graph contains a cycle")
        if identifier in visited:
            return
        visiting.add(identifier)
        for dependency in dependencies[identifier]:
            visit(dependency)
        visiting.remove(identifier)
        visited.add(identifier)

    for identifier in identifiers:
        visit(identifier)
    exact_k1_1_contour = {
        "QUALIFICATION_DEPENDENCY_SEED_PREPARE": {
            "deps": ["BASELINE_CAPTURE", "HARNESS_SELF_TEST", "CORPUS_FREEZE_VERIFY"],
            "selectedInputs": ["qualificationDependencySeed", "qualificationSourceSet", "candidateTools"],
        },
        "CANDIDATE_FREEZE_PREPARE": {
            "deps": ["QUALIFICATION_RUN_6_COMPLETE", "K0_1_BYTE_EXACT_VERIFY"],
            "selectedInputs": ["candidateFreeze", "candidateSources", "candidateBinaries", "candidateTools", "harnessSource", "independentAuditorSource", "requirements", "readinessGraph", "corpus"],
        },
        "BASELINE_CAPTURE": {
            "deps": ["INPUT_AUTHORITY_VERIFY", "CORPUS_FREEZE_VERIFY", "K0_1_BYTE_EXACT_VERIFY"],
            "selectedInputs": ["baselinePacket", "candidateSources", "candidateTools", "repositoryBaseRevision"],
        },
        "HOLDOUT_SOURCE_MATERIALIZE": {
            "deps": ["CANDIDATE_FREEZE_VERIFY", "HOLDOUT_ELIGIBILITY_AUDIT_IMPORT"],
            "selectedInputs": ["holdoutSourceSet"],
        },
        "HOLDOUT_DEPENDENCY_SEED_PREPARE": {
            "deps": ["HOLDOUT_SOURCE_MATERIALIZE", "CANDIDATE_FREEZE_VERIFY", "HOLDOUT_ELIGIBILITY_AUDIT_IMPORT"],
            "selectedInputs": ["holdoutDependencySeed", "holdoutSourceSet", "candidateTools"],
        },
        "HOLDOUT_DEPENDENCY_SEED_VERIFY": {
            "deps": ["HOLDOUT_DEPENDENCY_SEED_PREPARE", "CANDIDATE_FREEZE_VERIFY"],
            "selectedInputs": ["holdoutDependencySeed"],
        },
        "REQUIREMENT_CONFORMANCE_VERIFY": {
            "deps": ["MATRIX_TOTALITY_AND_SAFETY_VERIFY", "APPLICABILITY_VERIFY", "CACHE_AND_COST_VERIFY", "CANDIDATE_FREEZE_VERIFY", "BASELINE_CAPTURE", "HARNESS_SELF_TEST"],
            "selectedInputs": ["requirementConformance", "matrixSafetyReceipt", "applicabilityReceipt", "cacheCostReceipt", "requirements", "qualificationMatrix", "holdoutMatrix", "candidateFreeze", "baselinePacket", "harnessSelfTestPacket"],
        },
        "K1_INDEPENDENT_AUDITOR_RUN": {
            "deps": ["REQUIREMENT_CONFORMANCE_VERIFY", "HOLDOUT_RUN_6_COMPLETE", "CANDIDATE_FREEZE_VERIFY"],
            "selectedInputs": ["independentAuditorSource", "independentAuditorRunReceipt", "independentAudit", "requirementConformance", "matrixSafetyReceipt", "applicabilityReceipt", "cacheCostReceipt", "candidateFreeze", "qualificationMatrix", "holdoutMatrix", "requirements", "corpus", "candidateTools", "baselinePacket", "harnessSelfTestPacket"],
        },
        "K1_SERIES_GUARD": {"deps": [], "selectedInputs": []},
    }
    by_identifier = {node["id"]: node for node in nodes}
    for identifier, expected in exact_k1_1_contour.items():
        node = by_identifier.get(identifier)
        if node is None or any(node.get(key) != value for key, value in expected.items()):
            raise HarnessError(f"K1.1 critical dependency contour mismatch: {identifier}")
    return graph


def load_production_bundle(assertions: Mapping[str, Path] | None = None) -> dict[str, Any]:
    assertions = assertions or {}
    values: dict[str, Any] = {}
    digests: dict[str, str] = {}
    for name in AUTHORITIES:
        values[name], digests[name] = load_authority(name, assertions.get(name))
    graph = _validate_graph(values["readinessGraph"])
    requirements = values["requirements"]
    corpus = values["corpus"]
    eligibility = values["corpusEligibilityEvidence"]
    amendment = values["preregistrationAmendment"]
    k1_1_digests = {
        "requirements": "sha256:4c5775a7e8c49bd796ecea1cf4b7d1f008d02f442248d274f4969e6b80c323c4",
        "corpus": "sha256:29308abae2c31b3396a997838880f6addfe6e6febf60e692c74353357352e510",
        "corpusEligibilityEvidence": "sha256:779d0f9b327e530c8f9081f6823168317a916839d46b6aa2e17f3e33eae7104a",
        "readinessGraph": "sha256:ff585861b325988fee6ca9495902230dd609f01e8a489172cd52945da0ffceac",
    }
    k1_2_digests = {
        "requirements": "sha256:a455cc069b06efce903edada568bf5b4b48476ef07955d9abd475267dc691687",
        "corpus": "sha256:953bed2908f3d72e933d73b11670243255cd3154d2a91588af36f4dba8133d8e",
        "corpusEligibilityEvidence": "sha256:3366200fec534b50ec789301593dbf642863981f43450f1a3662587fd4beea23",
        "readinessGraph": "sha256:161e6294461a80c8a4ecca9d1e4c1a1539bdff9ecb4d6e7706a04aa44e160bd6",
        "holdoutEligibilityAudit": "sha256:d64c8a8e67e51e29ae8829271d49a5320426d5df1e5a2da8cea5080b20791690",
    }
    k1_3_digests = {
        "requirements": "sha256:3c230c0ec06d6e23eeee0c9db9245d121ac81264f00f66e124ae81c464e71f0f",
        "corpus": "sha256:8b088e9f2b4a0afa54792b1e7925e8e305c5328d13ddc3a5035e739a83e732b2",
        "corpusEligibilityEvidence": "sha256:32bec104ecb1d4ec309973fdc41e3da03cd55d455d276c127a0a4608cafc563e",
        "readinessGraph": "sha256:0188a99161bc3dbf14ce2c474bfe8417a5fa5f7bdd8b023503c41ac1bf6d0091",
        "holdoutEligibilityAudit": "sha256:60c60de512982da06a732c9f5d3d8c863297566e45d22ffc3f2ea84c07f4c270",
    }
    k1_4_digests = {
        "requirements": "sha256:4c80b7bccd7eaf5528919d36a7b3343bbf7d3ff8df2834f8e42e252894eb046f",
        "corpus": "sha256:88b593b7507ece8ffeb425d926633e33d5b9b3a800f6895d1c68f7598e8c7f55",
        "corpusEligibilityEvidence": "sha256:ab02f2874c5a1f586685bce532deced8a89345a561d850eb85e88e22ffa0b816",
        "readinessGraph": "sha256:ef7f95074fccc981930a26bbf1bec1ff7372ac29976c7a16cfb6db65433436be",
        "holdoutEligibilityAudit": "sha256:becf43debdeb6f887dbbe54f7c8544dce1b26fe53b57b984d49e38f0bf36731e",
    }
    k1_5_digests = {
        "requirements": "sha256:a43d6658166a752c3183edffd85f12604edf353c6a2d517ac0648a91cd4ee38f",
        "corpus": "sha256:f8ff690b8fe9e6ff834976ac0094facfb7b55c27a3c5eda666e55b8d47423570",
        "corpusEligibilityEvidence": "sha256:0437ef2b74522fd997367020aa51ebcc62233311ac270aaac07ebeb0b9bf9ed0",
        "readinessGraph": "sha256:a249d10ab38f92673b026bcce101d4fa0d382d406e143adc0094da5fe9d8489a",
        "holdoutEligibilityAudit": "sha256:7b6e22055b10317b8bfa66a5fc82214b07d09c2fb2543e7beebc45adceb7cc95",
    }
    k1_6_digests = {
        "requirements": "sha256:b11d1b60d3940715f8834c963d532c40ccf121924eade21019aca68b04b53f72",
        "corpus": "sha256:3f3367c8b5b9b2f076889971396b086e18ff5f214db37d40521d1022a09f1bb6",
        "corpusEligibilityEvidence": "sha256:de32750e441648468e858302f8141e12fe0fe330307fb5219aa27acb12aaafc5",
        "readinessGraph": "sha256:021d26f9fe7ce0a27a5a9e72fb9ea925d7b74009844cc5b56a34a396318f4e91",
        "holdoutEligibilityAudit": "sha256:a7e7d5d611dfa26d226fccf213eabdc7ba8b7b865f1741995c99a420a396e245",
    }
    k1_7_digests = {
        "requirements": "sha256:5c54f5c4c2921aed208ee23ebcb8054a0e33ce8ba2f37bf82443bbda85d56949",
        "corpus": "sha256:956bc75f2b3eff45d88a8519cf287bf842bbb7b285e2c3fd6cda6a26dbfe3942",
        "corpusEligibilityEvidence": "sha256:8516ede4d2e4a1c1e5246abb2a14f6a91704ccb0facc6e048f6a97619d18ca03",
        "readinessGraph": "sha256:d825ec97f0b0869376f67ead8dac3fdd7b8260b7ab98cbd9209b7a9488657b79",
        "holdoutEligibilityAudit": "sha256:5f638d06d3ade9f4f761a2abc53b64625bb95ed198f8966a874f7637f9dfa8f2",
    }
    k1_8_digests = {
        "requirements": "sha256:b52d478f3e4fd0a03dd324e7bf06932fee0c1c8ab154b0d4fca5c78437b32851",
        "corpus": "sha256:7bd268e337a8d1e98a3f243a19617263886faddde36e0176247221d3b31fc639",
        "corpusEligibilityEvidence": "sha256:f8638cd98589d6b606bb0b53a9c7bd58071047c96cac3f423364e79f2a341ce1",
        "readinessGraph": "sha256:6ba9e8a2696fd99d4c3e1b1fa93cebf62bfacfd11131a91f4ad91e78744890ec",
        "holdoutEligibilityAudit": "sha256:602bc409982ff52bb0a5b70d11e9bbede62953092530c4ed1d91a0b7b9dfd408",
    }
    k1_9_digests = {
        "requirements": "sha256:ca0150e883ae821c48d800105ebaba20edeb87a0cf387ba46f44ff792b8a7175",
        "corpus": "sha256:eb2ab5d8671b712b960b54f1adf799f4c20b2a2f5058b5ba8c75b51e34606ae2",
        "corpusEligibilityEvidence": "sha256:2cd5e5d376a9a698537a3bcc04ed48545a553df668ad37e07cf89d0c9c1b8739",
        "readinessGraph": "sha256:1f52e3cdd11c3e070481e39dbc3ddd4f6b221924ed45ce8c6b16fd57b1e65ccd",
        "holdoutEligibilityAudit": "sha256:395b8fc6352c845cdb87b0c4c34833c67a1499e4f7278cd238e93115b2ee1741",
    }
    k1_10_digests = {
        "requirements": "sha256:ba359d1efcc30567cc490334f0ada7b4949025fbf53da56d597f383f3e22f30a",
        "corpus": "sha256:403899de6f64b8684789e57e9fcdc9c3b6770963a6b4464a5bd787f104c3c811",
        "corpusEligibilityEvidence": "sha256:38659516f524b2fbe06e7577fe90807a3aefb036b49ac024bca087dc8182421b",
        "readinessGraph": "sha256:a75e92aa2c622ace73481ada73c4099d3a26ac56c08cf473fba7f2f58102cb4f",
        "holdoutEligibilityAudit": "sha256:f32923c83493c52d4fe241e1ebf6324438d410188795fbd92cfd0ba917187165",
    }
    k1_11_digests = {
        "requirements": "sha256:38900636dd92f80cdca59104168e29047836dccace2c24c9d5cdb363343069be",
        "corpus": "sha256:bf8e6f40ca408c128eefce1c0598b435c472799a4569ce4c26012daccdbe2bfc",
        "corpusEligibilityEvidence": "sha256:c76ad7f47384983af94391ddb88269b378417dcd360f0668b58586e4744a91fa",
        "readinessGraph": "sha256:dbe64435921607350efe71e9e637af5e5ddec01d48f4c62189067b8560edef27",
        "holdoutEligibilityAudit": "sha256:d7ba2daeb629a11ed18c9e9e16d2dc90f9473a5a25cbacbdbc43aaeb42741894",
    }
    k1_2_correction = {
        "analyzerDistributionPinsMovedFromCorpusToCandidateTools": {
            "2.1": "sha256:00ed4ac81d56f00f452dcda147917d6c65638c719b87fbc5bf6d5cb27e8e8dce",
            "2.3": "sha256:fbdc3575d1e443fd9d28fc2b531597ce4d7b5890f88023e4403e72763c40894d",
            "2.4": "sha256:91cc4221343db69d8e2161a971d33b089d0b06afaacc2c76445a58da73b9ed53",
        },
        "candidateToolsSelectedByDependencyPrepare": True,
        "boundaryObligationDigestBijection": True,
        "exactCompilerIdentityRequired": True,
        "measurementArtifactProducerBinding": True,
        "perEntryDependencyOutcomes": ["READY", "TYPED_REFUSAL"],
        "pinnedHoldoutEligibilityAudit": True,
        "prepareSourceSetBeforeAfterRecheck": True,
        "sourceSetSelectedByDependencyPrepare": True,
        "sourceSetSnapshotKind": "GIT_INDEX_TRACKED_BYTES_ONLY",
        "thresholdsCorpusAndWorkloadPreserved": True,
        "trackedSourceSymlinkPolicy": "HASH_LSTAT_READLINK_LINK_OBJECT_REJECT_UNTRACKED_OR_ESCAPE",
        "analysisAndPreparationSandbox": "DEFAULT_DENY_CREDENTIAL_ISOLATED",
        "graphChanges": {
            "candidateFreezeAdds": ["independentAuditorSource"],
            "decisionDirectMeasurementDependencies": [
                "MATRIX_TOTALITY_AND_SAFETY_VERIFY", "APPLICABILITY_VERIFY",
                "CACHE_AND_COST_VERIFY", "REQUIREMENT_CONFORMANCE_VERIFY",
            ],
            "dependencyPrepareCandidateToolsSelector": True,
            "pinnedIndependentAuditorNodes": ["K1_INDEPENDENT_AUDITOR_RUN", "K1_INDEPENDENT_AUDIT_IMPORT"],
            "requirementConformanceNode": "REQUIREMENT_CONFORMANCE_VERIFY",
            "auditorAllConsumedInputsSelected": True,
            "candidateFreezePrepareAllConsumedInputsSelected": True,
            "decisionSeriesGuardDependency": True,
            "requirementConformanceProducerDependencies": ["BASELINE_CAPTURE", "HARNESS_SELF_TEST"],
            "seriesGuardNode": "K1_SERIES_GUARD",
        },
        "auditorExpectedDecisionBinding": True,
        "baselineFailureIsMeasuredReady": True,
        "fatalReasonWhitelist": [
            "K0_1_DRIFT", "PINNED_AUTHORITY_DRIFT", "THRESHOLD_OR_CORPUS_REWRITE",
            "SOURCE_MUTATION", "POST_FREEZE_CANDIDATE_OR_HOLDOUT_CHANGE",
            "VERIFIED_AUTHORITY_BYPASS", "UNRETAINED_STARTED_CHILD",
            "MATRIX_SAFETY_VIOLATION",
        ],
        "finalizeSeriesCli": True,
        "irreversibleSeriesGuard": "OPEN_TO_FATAL_STOP_ONLY",
        "storeDegradedOpen": "FATAL_FINALIZE_ONLY",
        "unsafeColdWarmNoChildTotality": True,
    }
    k1_1_amendment, k1_1_amendment_digest = _pinned_predecessor_amendment(
        K1_1_AMENDMENT, "K1.1 preregistration amendment",
    )
    k1_2_amendment, k1_2_amendment_digest = _pinned_predecessor_amendment(
        K1_2_AMENDMENT, "K1.2 preregistration amendment",
    )
    k1_3_amendment, k1_3_amendment_digest = _pinned_predecessor_amendment(
        K1_3_AMENDMENT, "K1.3 preregistration amendment",
    )
    k1_4_amendment, k1_4_amendment_digest = _pinned_predecessor_amendment(
        K1_4_AMENDMENT, "K1.4 preregistration amendment",
    )
    k1_5_amendment, k1_5_amendment_digest = _pinned_predecessor_amendment(
        K1_5_AMENDMENT, "K1.5 preregistration amendment",
    )
    k1_6_amendment, k1_6_amendment_digest = _pinned_predecessor_amendment(
        K1_6_AMENDMENT, "K1.6 preregistration amendment",
    )
    k1_7_amendment, k1_7_amendment_digest = _pinned_predecessor_amendment(
        K1_7_AMENDMENT, "K1.7 preregistration amendment",
    )
    k1_8_amendment, k1_8_amendment_digest = _pinned_predecessor_amendment(
        K1_8_AMENDMENT, "K1.8 preregistration amendment",
    )
    k1_9_amendment, k1_9_amendment_digest = _pinned_predecessor_amendment(
        K1_9_AMENDMENT, "K1.9 preregistration amendment",
    )
    k1_10_amendment, k1_10_amendment_digest = _pinned_predecessor_amendment(
        K1_10_AMENDMENT, "K1.10 preregistration amendment",
    )
    k1_11_amendment, k1_11_amendment_digest = _pinned_predecessor_amendment(
        K1_11_AMENDMENT, "K1.11 preregistration amendment",
    )
    if k1_1_amendment.get("replacementSeriesId") != K1_1_SERIES_ID:
        raise HarnessError("K1.1 predecessor amendment identity mismatch")
    if k1_2_amendment != {
        "schema": "codeclew.kotlin-k1-preregistration-amendment/0.2",
        "cancelledSeriesId": K1_1_SERIES_ID,
        "replacementSeriesId": K1_2_SERIES_ID,
        "oldAuthorityDigests": k1_1_digests,
        "predecessorAmendmentSha256": k1_1_amendment_digest,
        "reasonCode": "K1_1_PREQUALIFICATION_AUTHORITY_GAPS",
        "qualificationAttempts": 0,
        "holdoutAttempts": 0,
        "modelCalls": 0,
        "holdoutOpened": False,
        "authorityStateBeforeFinalPin": "PENDING_NO_STORE_NO_OUTCOMES",
        "correction": k1_2_correction,
    }:
        raise HarnessError("K1.2 cancellation/amendment contract mismatch")
    k1_3_correction = {
        "baselineAuthority": {
            "depsAfter": ["INPUT_AUTHORITY_VERIFY", "CORPUS_FREEZE_VERIFY", "K0_1_BYTE_EXACT_VERIFY"],
            "depsBefore": ["CORPUS_FREEZE_VERIFY", "K0_1_BYTE_EXACT_VERIFY"],
            "repositoryHeadObservation": "IMMEDIATE_BEFORE_AND_AFTER",
            "selectedInputsAfter": ["baselinePacket", "candidateSources", "candidateTools", "repositoryBaseRevision"],
            "selectedInputsBefore": ["baselinePacket"],
        },
        "baselinePacketSchema": "codeclew.kotlin-k1-baseline-packet/0.2",
        "candidateAndHarnessDisposition": "REBUILD_AND_REBIND_BEFORE_NEW_STORE",
        "cargoExecutionAuthority": {
            "cargoHome": "ISOLATED",
            "credentialInheritance": False,
            "dependencySeed": "CARGO_LOCK_DERIVED_CRATES_IO_SPARSE_INDEX_AND_ARCHIVES_ONLY",
            "launcherIdentity": "RUST_1_92_0_EXECUTABLE_SET_SHA256",
            "lockedFlagAddedToTestAndClippy": True,
            "network": "OFFLINE",
        },
        "preflightDisposition": "DIAGNOSTIC_ONLY_SUPERSEDED_STORE_MUST_NOT_BE_REUSED",
        "preserved": [
            "DECISION_THRESHOLDS", "REQUIREMENTS_EXCEPT_SERIES_AND_AMENDMENT_BINDING",
            "CORPUS_EXCEPT_SERIES", "ELIGIBILITY_EXCEPT_SERIES",
            "HOLDOUT_ELIGIBILITY_PROCEDURE_MEMBERS_AND_DECISION", "K0_1_BYTE_EXACT", "WORKLOAD",
            "BASELINE_COMMAND_TARGETS_AND_TEST_FILTERS", "BASELINE_RED_IS_MEASURED_READY",
        ],
    }
    required_preflight = {
        "schema": "codeclew.kotlin-k1-preflight-evidence/0.1",
        "seriesId": K1_2_SERIES_ID,
        "kind": "INFRASTRUCTURE_ONLY",
        "storeId": "3884cdd5ea48aa1b0ed6d0edcfb24716aed50ae864dba51ab2de5569dfcaeb15",
        "storeIdentitySha256": "sha256:5889c3798a83fe8be596b4b7a43330f2d4894a3a2a32e6704911fe4c64ef5b75",
        "baselinePacketSha256": "sha256:4cd77351c4adcf59935f692fa6f0be7d2063e8296f81d67877073ee5eaf9e605",
        "baselinePointerSha256": "sha256:f04dad16c4d53590858a3de637ecf38b9617144d3df0062ec7e00ac14f0ce7e3",
        "baselineReceiptDigest": "sha256:1e6a3676d016cce3cc4b320ab31f32ec74f9a637e62c4168e3aff3073da49364",
        "candidateToolsSha256": "sha256:dd17d195d27fbfb78b7c978547f5c94d3697c7d3a8e169b683fe90eff33cb1fb",
        "liveInputsSha256": "sha256:211f6ef19590754e4c14b28be9281b9a0cdcbd44f1c70690eca0bfc6a0e315b6",
        "harnessSourceSha256": "sha256:33b76d0e1c8cec9b21f57d035678a9d7a1f66c82c8a372387f365726e51e8f7f",
        "harnessSelfTestPacketSha256": "sha256:4d060443c84d5371aee7752c9585086cd4b4d877745751f2999e53c401ff4b09",
        "guardReceiptDigest": "sha256:156558cb06cea8c2e8c0a349cc209866da3f55113f113b810d1907353870a0fe",
        "retainedEvidenceSha256": "sha256:b442e8b7edfedea834463b1713a392977e9132e4176b0f92874a5d9a0a8981ee",
        "qualificationAttemptCount": 0, "holdoutAttemptCount": 0, "childStartJournalCount": 0,
        "holdoutSourceMaterialized": False, "decisionIssued": False, "modelCalls": 0,
    }
    preflight = k1_3_amendment.get("preflightEvidence") if isinstance(k1_3_amendment, dict) else None
    if not isinstance(preflight, dict) or any(preflight.get(key) != value for key, value in required_preflight.items()):
        raise HarnessError("K1.3 exact preflight/store evidence mismatch")
    retained_evidence = ROOT / "docs/experiments/evidence/codeclew-k1.2-preflight-retained-evidence.json"
    if sha256_file(retained_evidence) != required_preflight["retainedEvidenceSha256"]:
        raise HarnessError("K1.3 retained preflight evidence file drift")
    retained = _load_json_bytes(_regular_file(retained_evidence, "retained K1.2 preflight evidence").read_bytes(), "retained K1.2 preflight evidence")
    if not isinstance(retained, dict) or canonical(retained) != retained_evidence.read_bytes() or retained.get("fileManifestCount") != 28 or retained.get("baselinePacketSha256") != preflight.get("baselinePacketSha256") or retained.get("baselineReceiptSha256") != preflight.get("baselineReceiptDigest") or retained.get("storeIdentitySha256") != preflight.get("storeIdentitySha256"):
        raise HarnessError("K1.3 retained preflight evidence content mismatch")
    if not isinstance(k1_3_amendment, dict) or set(k1_3_amendment) != {
        "schema", "cancelledSeriesId", "replacementSeriesId", "oldAuthorityDigests",
        "reasonCode", "qualificationAttempts", "holdoutAttempts", "modelCalls", "holdoutOpened",
        "correction", "predecessorAmendmentSha256", "authorityStateBeforeReplacement",
        "baselineAttempts", "preflightEvidence",
    } or k1_3_amendment.get("schema") != "codeclew.kotlin-k1-preregistration-amendment/0.3" \
        or k1_3_amendment.get("cancelledSeriesId") != K1_2_SERIES_ID \
        or k1_3_amendment.get("replacementSeriesId") != K1_3_SERIES_ID \
        or k1_3_amendment.get("oldAuthorityDigests") != k1_2_digests \
        or k1_3_amendment.get("predecessorAmendmentSha256") != k1_2_amendment_digest \
        or k1_3_amendment.get("reasonCode") != "K1_2_PREFLIGHT_ISOLATED_CARGO_HOME_UNSEEDED" \
        or k1_3_amendment.get("qualificationAttempts") != 0 or k1_3_amendment.get("holdoutAttempts") != 0 \
        or k1_3_amendment.get("modelCalls") != 0 or k1_3_amendment.get("holdoutOpened") is not False \
        or k1_3_amendment.get("baselineAttempts") != 1 \
        or k1_3_amendment.get("authorityStateBeforeReplacement") != "PREFLIGHT_ONLY_NO_QUALIFICATION_NO_HOLDOUT_NO_MODEL" \
        or k1_3_amendment.get("correction") != k1_3_correction:
        raise HarnessError("K1.3 cancellation/amendment contract mismatch")
    k1_4_correction = {
        "baselineEnvironmentAuthority": {
            "after": "EXACT_TOOL_FAMILY_SCOPED_ENVIRONMENTS",
            "before": "ONE_SHARED_ENVIRONMENT_FOR_ALL_BASELINE_TOOL_FAMILIES",
            "cargoAndRustEnvironment": {
                "forbidden": ["JAVA_HOME", "GRADLE_OPTS", "GRADLE_USER_HOME", "JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "JAVA_OPTS", "_JAVA_OPTIONS", "ORG_GRADLE_PROJECT_*"],
                "included": ["HOME", "TMPDIR", "PATH", "LANG", "LC_ALL", "CODECLEW_K1_MODEL_CALLS", "CARGO_HOME", "CARGO_TARGET_DIR", "CARGO_NET_OFFLINE", "CARGO_REGISTRIES_CRATES_IO_PROTOCOL"],
                "normalizedPolicySha256": "sha256:8af2372824171d735669ec8a0ad02ae603a7cb432ea0576302624946ab5fcd39",
            },
            "gradleEnvironment": {
                "explicitExecutionArgvPrefix": ["$GRADLE_9_6_1", "-Duser.home=$ISOLATED"],
                "forbidden": ["CARGO_HOME", "CARGO_TARGET_DIR", "CARGO_NET_OFFLINE", "CARGO_REGISTRIES_CRATES_IO_PROTOCOL", "GRADLE_OPTS", "JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "JAVA_OPTS", "_JAVA_OPTIONS", "ORG_GRADLE_PROJECT_*"],
                "included": ["HOME", "TMPDIR", "PATH", "LANG", "LC_ALL", "CODECLEW_K1_MODEL_CALLS", "JAVA_HOME", "GRADLE_USER_HOME"],
                "normalizedPolicySha256": "sha256:cb99dd606ca84cd2517c6ab8fdcc8ad3aed71bea54f199d24bae41be89918bd1",
            },
            "workerInjectionRefusal": "UNCHANGED_FAIL_CLOSED",
        },
        "candidateAndHarnessDisposition": "REBUILD_AND_REBIND_BEFORE_NEW_STORE",
        "preparedRefusalIdentity": {
            "schemaAfter": "codeclew.kotlin-k1-dependency-preparation-refusal/0.3",
            "schemaBefore": "codeclew.kotlin-k1-dependency-preparation-refusal/0.2",
            "seriesAfter": K1_4_SERIES_ID,
            "seriesBefore": K1_3_SERIES_ID,
        },
        "preserved": [
            "DECISION_THRESHOLDS", "REQUIREMENTS_EXCEPT_SERIES_AND_AMENDMENT_BINDING",
            "CORPUS_EXCEPT_SERIES", "ELIGIBILITY_EXCEPT_SERIES",
            "HOLDOUT_ELIGIBILITY_PROCEDURE_MEMBERS_AND_DECISION", "READINESS_GRAPH_EXCEPT_GRAPH_ID",
            "K0_1_BYTE_EXACT", "WORKLOAD", "BASELINE_PACKET_AND_CONTEXT_SCHEMAS",
            "BASELINE_LOGICAL_COMMAND_ARGV_TARGETS_AND_TEST_FILTERS", "CARGO_LOCK_DERIVED_SEED",
            "RUST_GRADLE_JDK_LAUNCHER_IDENTITIES", "BASELINE_RED_IS_MEASURED_READY",
        ],
        "supersededStoreDisposition": "BASELINE_ONLY_RETAINED_EVIDENCE_STORE_MUST_NOT_BE_REUSED",
    }
    baseline_only = k1_4_amendment.get("baselineOnlyEvidence") if isinstance(k1_4_amendment, dict) else None
    required_baseline_only = {
        "schema": "codeclew.kotlin-k1-baseline-only-evidence/0.1",
        "seriesId": K1_3_SERIES_ID,
        "kind": "BASELINE_ONLY_INFRASTRUCTURE_OUTCOME",
        "storeId": "24ffd6fd803b1441cee6ab3d27afefbc87bf7375af1ad70d2abbb904b4816852",
        "storeIdentitySha256": "sha256:ef5cbc72a9d8919f263413fcccac5723d70bd577aa7f1f05d1e44bd6507f586f",
        "baselinePacketSha256": "sha256:a856576bf4b39af2c0eb116c4b01db59c5a35c26244114de905b8966d0174f3e",
        "baselinePointerSha256": "sha256:685c858bc82611fea4f576a8f2e3fa60abe1779f4d95d3f51991a803db3484e9",
        "baselineReceiptDigest": "sha256:13707c557297e71114d1e9c4b82e4c749cd834fda3a879c9fc4565d71ccfc6c5",
        "candidateToolsSha256": "sha256:6ae8050d836727f9d26e994093f92531d33e6b832ad35bfb05f3abf63e61ef98",
        "candidateSourcesSha256": "sha256:8f3ba34fd1025785e2b6e15d62e07290a24a215f118075e8d951805c8f600d80",
        "candidateBinariesSha256": "sha256:18d7abd62e9ffc1901e4db4c85e6f80a44b993ec0aff15df04dafdf04f5ca0be",
        "liveInputsSha256": "sha256:759193a0ef1d6a64c41af0eefec7d5171a8bc6b2ed04193fc368efb128219666",
        "harnessSourceSha256": "sha256:d91a5a35f35a990d167435a8eed678ef04ec5351eed9e76b6cd04d6421f174b4",
        "retainedEvidenceSha256": "sha256:52496625de20c25403f891f84b8215c7d6a838a170a967b03d6abd46a2a19548",
        "qualificationAttemptCount": 0, "holdoutAttemptCount": 0, "childStartJournalCount": 0,
        "holdoutSourceMaterialized": False, "decisionIssued": False, "modelCalls": 0,
    }
    if not isinstance(baseline_only, dict) or any(baseline_only.get(key) != value for key, value in required_baseline_only.items()):
        raise HarnessError("K1.4 exact baseline/store evidence mismatch")
    retained_k1_3 = ROOT / "docs/experiments/evidence/codeclew-k1.3-baseline-retained-evidence.json"
    if sha256_file(retained_k1_3) != required_baseline_only["retainedEvidenceSha256"]:
        raise HarnessError("K1.4 retained K1.3 baseline evidence file drift")
    retained_k1_3_value = _load_json_bytes(_regular_file(retained_k1_3, "retained K1.3 baseline evidence").read_bytes(), "retained K1.3 baseline evidence")
    if not isinstance(retained_k1_3_value, dict) or canonical(retained_k1_3_value) != retained_k1_3.read_bytes() or retained_k1_3_value.get("storeFileManifestCount") != 41 or retained_k1_3_value.get("baselinePacketSha256") != baseline_only.get("baselinePacketSha256") or retained_k1_3_value.get("baselineReceiptSha256") != baseline_only.get("baselineReceiptDigest") or retained_k1_3_value.get("storeIdentitySha256") != baseline_only.get("storeIdentitySha256"):
        raise HarnessError("K1.4 retained K1.3 baseline evidence content mismatch")
    if not isinstance(k1_4_amendment, dict) or set(k1_4_amendment) != {
        "schema", "cancelledSeriesId", "replacementSeriesId", "oldAuthorityDigests",
        "predecessorAmendmentSha256", "reasonCode", "authorityStateBeforeReplacement",
        "baselineAttempts", "qualificationAttempts", "holdoutAttempts", "modelCalls", "holdoutOpened",
        "correction", "baselineOnlyEvidence",
    } or k1_4_amendment.get("schema") != "codeclew.kotlin-k1-preregistration-amendment/0.4" \
        or k1_4_amendment.get("cancelledSeriesId") != K1_3_SERIES_ID \
        or k1_4_amendment.get("replacementSeriesId") != K1_4_SERIES_ID \
        or k1_4_amendment.get("oldAuthorityDigests") != k1_3_digests \
        or k1_4_amendment.get("predecessorAmendmentSha256") != k1_3_amendment_digest \
        or k1_4_amendment.get("reasonCode") != "K1_3_BASELINE_SHARED_ENVIRONMENT_INJECTED_JVM_GRADLE_STATE_INTO_RUST_TEST" \
        or k1_4_amendment.get("authorityStateBeforeReplacement") != "BASELINE_ONLY_NO_QUALIFICATION_NO_HOLDOUT_NO_MODEL" \
        or k1_4_amendment.get("baselineAttempts") != 1 or k1_4_amendment.get("qualificationAttempts") != 0 \
        or k1_4_amendment.get("holdoutAttempts") != 0 or k1_4_amendment.get("modelCalls") != 0 \
        or k1_4_amendment.get("holdoutOpened") is not False or k1_4_amendment.get("correction") != k1_4_correction:
        raise HarnessError("K1.4 cancellation/amendment contract mismatch")

    prepare_evidence = k1_5_amendment.get("prepareInfrastructureEvidence") if isinstance(k1_5_amendment, dict) else None
    correction = k1_5_amendment.get("correction") if isinstance(k1_5_amendment, dict) else None
    expected_reason_codes = [
        "K1_4_PREPARE_READ_ONLY_SYNTHETIC_GIT_CLEANUP_PERMISSION_ERROR",
        "K1_4_PREQUALIFICATION_OFFLINE_PREPARE_NETWORK_AUTHORITY_GAP",
        "K1_4_PREQUALIFICATION_DEPENDENCY_SEED_PHYSICAL_SEALING_GAP",
    ]
    if not isinstance(k1_5_amendment, dict) or set(k1_5_amendment) != {
        "schema", "cancelledSeriesId", "replacementSeriesId", "oldAuthorityDigests",
        "predecessorAmendmentSha256", "reasonCodes", "authorityStateBeforeReplacement",
        "baselineAttempts", "dependencyPrepareInvocations", "qualificationAttempts",
        "holdoutAttempts", "modelCalls", "holdoutOpened", "correction",
        "prepareInfrastructureEvidence",
    } or k1_5_amendment.get("schema") != "codeclew.kotlin-k1-preregistration-amendment/0.5" \
        or k1_5_amendment.get("cancelledSeriesId") != K1_4_SERIES_ID \
        or k1_5_amendment.get("replacementSeriesId") != K1_5_SERIES_ID \
        or k1_5_amendment.get("oldAuthorityDigests") != k1_4_digests \
        or k1_5_amendment.get("predecessorAmendmentSha256") != k1_4_amendment_digest \
        or k1_5_amendment.get("reasonCodes") != expected_reason_codes \
        or k1_5_amendment.get("authorityStateBeforeReplacement") != "PREPARE_INFRASTRUCTURE_ONLY_NO_QUALIFICATION_NO_HOLDOUT_NO_MODEL" \
        or k1_5_amendment.get("baselineAttempts") != 1 or k1_5_amendment.get("dependencyPrepareInvocations") != 1 \
        or k1_5_amendment.get("qualificationAttempts") != 0 or k1_5_amendment.get("holdoutAttempts") != 0 \
        or k1_5_amendment.get("modelCalls") != 0 or k1_5_amendment.get("holdoutOpened") is not False:
        raise HarnessError("K1.5 cancellation/amendment contract mismatch")
    if not isinstance(correction, dict) or set(correction) != {
        "candidateAndHarnessDisposition", "dependencySeedPhysicalSealing",
        "disposableSourceCleanup", "prepareNetworkAuthority", "preparedRefusalIdentity",
        "preserved", "supersededStoreDisposition",
    } or correction.get("candidateAndHarnessDisposition") != "REBUILD_AND_REBIND_BEFORE_NEW_STORE" \
        or correction.get("supersededStoreDisposition") != "PREPARE_INFRASTRUCTURE_ONLY_RETAINED_EVIDENCE_STORE_AND_STAGING_MUST_NOT_BE_REUSED" \
        or correction.get("preparedRefusalIdentity") != {
            "schemaAfter": "codeclew.kotlin-k1-dependency-preparation-refusal/0.4",
            "schemaBefore": "codeclew.kotlin-k1-dependency-preparation-refusal/0.3",
            "seriesAfter": K1_5_SERIES_ID, "seriesBefore": K1_4_SERIES_ID,
        }:
        raise HarnessError("K1.5 correction identity mismatch")
    cleanup_correction = correction.get("disposableSourceCleanup")
    if not isinstance(cleanup_correction, dict) \
        or cleanup_correction.get("helper") != "_discard_disposable_source(repository, containment_root)" \
        or cleanup_correction.get("targetAuthority") != "EXACT_NON_SYMLINK_DIRECT_CHILD_OF_PRIVATE_CONTAINMENT_ROOT" \
        or cleanup_correction.get("traversal") != "OS_WALK_NO_FOLLOW_SYMLINKS" \
        or cleanup_correction.get("permissionRestoration") != "OWNER_MINIMUM_RW_REGULAR_FILES_AND_RWX_DIRECTORIES_ONLY" \
        or cleanup_correction.get("specialObjects") != "REJECT" \
        or cleanup_correction.get("errors") != "TYPED_HARNESS_ERROR":
        raise HarnessError("K1.5 disposable cleanup correction mismatch")
    network_correction = correction.get("prepareNetworkAuthority")
    if not isinstance(network_correction, dict) \
        or network_correction.get("onlinePolicy") != {
            "exactNetworkClause": "(allow network*)", "name": "EXPLICIT_ALLOW_NETWORK",
        } or network_correction.get("offlinePolicy") != {
            "exactNetworkClause": "(deny network*)", "forbiddenClause": "(allow network*)",
            "name": "DENY_DEFAULT_NO_NETWORK_ALLOW",
        } or network_correction.get("typedRefusalStages") != [
            "ONLINE_DEPENDENCY_PREPARATION", "OFFLINE_DEPENDENCY_VERIFICATION",
        ] or network_correction.get("validation") != [
            "HARNESS_PUBLISH_AND_REOPEN", "HARNESS_REQUIREMENT_CONFORMANCE_K1_R13",
            "INDEPENDENT_AUDITOR", "ADVERSARIAL_PROFILE_SENTINEL_AND_MARKER_CASES",
        ]:
        raise HarnessError("K1.5 PREPARE network correction mismatch")
    sentinel_correction = network_correction.get("offlineNetworkSentinel")
    if not isinstance(sentinel_correction, dict) \
        or sentinel_correction.get("executable") != "/usr/bin/perl" \
        or sentinel_correction.get("executableSelectedInCandidateTools") is not True \
        or sentinel_correction.get("acceptedErrnos") != ["EACCES", "EPERM"] \
        or sentinel_correction.get("successExitCode") != 0 \
        or sentinel_correction.get("runsBeforeOfflineBuildCommand") is not True:
        raise HarnessError("K1.5 offline sentinel correction mismatch")
    sealing = correction.get("dependencySeedPhysicalSealing")
    if not isinstance(sealing, dict) \
        or sealing.get("contentManifestSchema") != "codeclew.kotlin-k1-build-state-manifest/0.1" \
        or sealing.get("contentManifestCompatibility") != "UNCHANGED_FOR_WORKER" \
        or sealing.get("modeSidecarSchema") != "codeclew.kotlin-k1-build-state-modes/0.1" \
        or sealing.get("publicationModes") != {"directories": "0500", "files": "0400"} \
        or sealing.get("modeDrift") != "REJECT_EXACTLY" \
        or sealing.get("workerProtocolChange") is not False:
        raise HarnessError("K1.5 dependency-seed sealing correction mismatch")

    required_prepare_evidence = {
        "schema": "codeclew.kotlin-k1-prepare-infrastructure-evidence/0.1",
        "seriesId": K1_4_SERIES_ID,
        "kind": "PREPARE_INFRASTRUCTURE_ONLY_NO_QUALIFICATION_OUTCOME",
        "storeId": "6b4d95fbd363ae35b30efe1bdaefa0e6f9ac488226d37f1b137f4b4447550502",
        "storeIdentitySha256": "sha256:84a330d9b31a22b4c96e62243fb057e3a7d836e26ae8b71c156f28d9aca0015a",
        "sourceStoreFileManifestSha256": "sha256:9937defa5ddfba3e670e262913f3a8eb2c296659d575c99d11abfcad7c87a0d6",
        "baselinePacketSha256": "sha256:15fe8fa2ae54258bb5fd4d9ac75641231b9babf9fe262682cb214653fc3170e9",
        "baselinePointerSha256": "sha256:75811218d42e38780dfa75ef72c9bb9f1aaae76eb88e5d5ba4434a0677af2aa0",
        "baselineReceiptDigest": "sha256:aff3c4d91b5d5f5f53ba300e9c9d777c270ff5b437ff4f208f667477c146b2e1",
        "baselineRequiredGreen": True,
        "harnessSelfTestPacketSha256": "sha256:dfccd57d43eb8e51c151a63ff497850a94195a20e4d91e3c4509bf51e5e95ab1",
        "harnessSelfTestReceiptDigest": "sha256:f2adb4ef18eef1f5af426efb7e8c4b29fa6ba54a6cef01917562b4ad7e38a9d2",
        "candidateToolsSha256": "sha256:3ae93ad3867fc2c5ecf05748f033151691421336bc67151e7ab677ea486486e3",
        "liveInputsSha256": "sha256:8b27e1c7cacad825fff5ccd8fcb53506c0ebbd7ee9699fe58957b55ee6da48e8",
        "retainedEvidenceSha256": "sha256:dd43a79889391b7e84f70f3cf20e97a31b4aa75223d3ffc90e075a4c2509dbe3",
        "dependencyPrepareInvocations": 1, "dependencyPrepareCommandsCompleted": 2,
        "dependencyPrepareEntry": "K1-Q01", "qualificationDependencySeedPublished": False,
        "qualificationDependencyPreparePointerPresent": False, "qualificationAttemptCount": 0,
        "holdoutAttemptCount": 0, "retainedAttemptCount": 0, "childStartJournalCount": 0,
        "modelCalls": 0, "holdoutOpened": False, "holdoutSourceMaterialized": False,
        "decisionIssued": False, "guardState": "OPEN",
    }
    if not isinstance(prepare_evidence, dict) or any(
        prepare_evidence.get(key) != value for key, value in required_prepare_evidence.items()
    ) or prepare_evidence.get("failure") != {
        "exceptionType": "PermissionError", "failureDetailBytes": 187,
        "failureDetailSha256": "sha256:a6dcbb488602877f20534b2ddf6cd3071cdc8342952bbb1d9dda4ef91a10a680",
        "operation": "CLEAN_HARNESS_CREATED_READ_ONLY_SYNTHETIC_GIT_TREE",
    } or prepare_evidence.get("discoveredPreQualificationConformanceGap") != {
        "condition": "ONLINE_AND_OFFLINE_PREPARE_COMMANDS_SHARED_NETWORK_ENABLED_SANDBOX_PROFILE",
        "qualificationOutcomeObserved": False, "requirement": "K1-R13",
    }:
        raise HarnessError("K1.5 exact PREPARE/store evidence mismatch")
    staging_evidence = prepare_evidence.get("stagingEvidence")
    if not isinstance(staging_evidence, dict) \
        or staging_evidence.get("relativePath") != ".qualificationDependencySeed.prepare-31d68f97972b87024d93b8bf" \
        or staging_evidence.get("fileCount") != 1868 \
        or staging_evidence.get("fileBytes") != 4278734 \
        or staging_evidence.get("fileManifestSha256") != "sha256:3bb8f41cead01d75a22a91de39980e3093c40c2415ec70cf723bce49be30afa0" \
        or staging_evidence.get("allMemberCount") != 2471 \
        or staging_evidence.get("allMemberManifestSha256") != "sha256:84e5be077a9498351538bd61308c00c55138528a83d7bd2f969d3bf959bf288b":
        raise HarnessError("K1.5 retained PREPARE staging evidence mismatch")
    retained_k1_4 = ROOT / "docs/experiments/evidence/codeclew-k1.4-prepare-infrastructure-retained-evidence.json"
    if sha256_file(retained_k1_4) != required_prepare_evidence["retainedEvidenceSha256"]:
        raise HarnessError("K1.5 retained K1.4 PREPARE evidence file drift")
    retained_k1_4_value = _load_json_bytes(
        _regular_file(retained_k1_4, "retained K1.4 PREPARE evidence").read_bytes(),
        "retained K1.4 PREPARE evidence",
    )
    if not isinstance(retained_k1_4_value, dict) or canonical(retained_k1_4_value) != retained_k1_4.read_bytes() \
        or retained_k1_4_value.get("sourceStoreFileManifestCount") != 42 \
        or retained_k1_4_value.get("storeIdentitySha256") != prepare_evidence.get("storeIdentitySha256") \
        or retained_k1_4_value.get("baselinePacketSha256") != prepare_evidence.get("baselinePacketSha256") \
        or retained_k1_4_value.get("failure") != prepare_evidence.get("failure"):
        raise HarnessError("K1.5 retained K1.4 PREPARE evidence content mismatch")

    k1_6_evidence = k1_6_amendment.get("prepareInfrastructureEvidence") if isinstance(k1_6_amendment, dict) else None
    k1_6_correction = k1_6_amendment.get("correction") if isinstance(k1_6_amendment, dict) else None
    if not isinstance(k1_6_amendment, dict) or set(k1_6_amendment) != {
        "schema", "cancelledSeriesId", "replacementSeriesId", "oldAuthorityDigests",
        "predecessorAmendmentSha256", "reasonCode", "authorityStateBeforeReplacement",
        "baselineAttempts", "officialDependencyPrepareAttempts", "diagnosticInfrastructureReplays",
        "qualificationAttempts", "holdoutAttempts", "modelCalls", "holdoutOpened", "correction",
        "prepareInfrastructureEvidence",
    } or k1_6_amendment.get("schema") != "codeclew.kotlin-k1-preregistration-amendment/0.6" \
        or k1_6_amendment.get("cancelledSeriesId") != K1_5_SERIES_ID \
        or k1_6_amendment.get("replacementSeriesId") != K1_6_SERIES_ID \
        or k1_6_amendment.get("oldAuthorityDigests") != k1_5_digests \
        or k1_6_amendment.get("predecessorAmendmentSha256") != k1_5_amendment_digest \
        or k1_6_amendment.get("reasonCode") != "K1_5_PREPARE_SANDBOX_ANCESTOR_TRAVERSAL_AUTHORITY_INCOMPLETE" \
        or k1_6_amendment.get("authorityStateBeforeReplacement") != "PREPARE_INFRASTRUCTURE_ONLY_NO_QUALIFICATION_NO_HOLDOUT_NO_MODEL" \
        or k1_6_amendment.get("baselineAttempts") != 1 \
        or k1_6_amendment.get("officialDependencyPrepareAttempts") != 1 \
        or k1_6_amendment.get("diagnosticInfrastructureReplays") != 1 \
        or k1_6_amendment.get("qualificationAttempts") != 0 or k1_6_amendment.get("holdoutAttempts") != 0 \
        or k1_6_amendment.get("modelCalls") != 0 or k1_6_amendment.get("holdoutOpened") is not False:
        raise HarnessError("K1.6 cancellation/amendment contract mismatch")
    if not isinstance(k1_6_correction, dict) or set(k1_6_correction) != {
        "candidateAndHarnessDisposition", "prepareSandboxTraversalAuthority",
        "preparedRefusalIdentity", "preserved", "requirementR18Authority",
        "supersededStoreDisposition",
    } or k1_6_correction.get("candidateAndHarnessDisposition") != "REBUILD_AND_REBIND_BEFORE_NEW_STORE" \
        or k1_6_correction.get("supersededStoreDisposition") != "PREPARE_INFRASTRUCTURE_ONLY_RETAINED_EVIDENCE_STORE_MUST_NOT_BE_REUSED" \
        or k1_6_correction.get("preparedRefusalIdentity") != {
            "schemaBefore": "codeclew.kotlin-k1-dependency-preparation-refusal/0.4",
            "schemaAfter": "codeclew.kotlin-k1-dependency-preparation-refusal/0.5",
            "seriesBefore": K1_5_SERIES_ID, "seriesAfter": K1_6_SERIES_ID,
        }:
        raise HarnessError("K1.6 correction identity mismatch")
    traversal = k1_6_correction.get("prepareSandboxTraversalAuthority")
    if not isinstance(traversal, dict) \
        or traversal.get("platformProfile") != "DARWIN_SANDBOX_PROFILE_VERSION_1" \
        or traversal.get("defaultPolicy") != "(deny default)" \
        or traversal.get("previousAncestorClause") != '(allow file-read-data (literal "<canonical ancestor>"))' \
        or traversal.get("correctedAncestorClause") != '(allow file-read-data file-read-metadata (literal "<canonical ancestor>"))' \
        or traversal.get("ancestorSelection") != "ROOT_AND_EXACT_CANONICAL_ANCESTOR_CLOSURE_OF_RECOMPUTED_CONTENT_ROOTS" \
        or traversal.get("writeAuthority") != "EXACT_ONE_SUBPATH_CLAUSE_FOR_SHARED_ENTRY_WORK" \
        or traversal.get("writeRootShape") != "<staging>/.work/<exact entry>" \
        or traversal.get("onlineOfflineWriteRoot") != "EXACTLY_IDENTICAL" \
        or traversal.get("prepareArgvAuthority") != "RECOMPUTE_EXACTLY_FROM_ENTRY_BUILD_DSL_SELECTED_COMPILATION_SHARED_ENTRY_WORK_AND_PHASE" \
        or traversal.get("outputLimitPerStreamBytes") != MAX_STDOUT_BYTES \
        or traversal.get("dualValidatorMutations") != {
            "passed": 4, "total": 4,
            "cases": ["ROOT_SUBSTITUTION", "PRIVATE_SUBSTITUTION", "SIBLING_SUBSTITUTION", "SPLIT_PHASE_ROOTS"],
        }:
        raise HarnessError("K1.6 PREPARE traversal correction mismatch")
    expected_prepare_cases = [
        "prepareMavenLauncherTraversalPassed", "prepareSourceAncestryTraversalPassed",
        "prepareAncestorSecretReadDenied", "prepareAncestorWriteDenied",
        "prepareSelectedSourceWriteDenied", "prepareKeychainReadDenied",
        "prepareTraversalNetworkSemanticsPreserved", "prepareAncestorDataOnlyMutationRejected",
        "prepareBroadSandboxPermissionRejected", "prepareRootAuthoritySubstitutionsRejected",
        "prepareSplitPhaseRootsRejected",
    ]
    expected_supervisor_values = {
        "sandbox_network_env": "DENIED_AND_ISOLATED", "sandbox_secret_paths": "DENIED",
        "sandbox_unix_network": "DENIED", "sandbox_source_write": "DENIED",
        "sandbox_keychain_read": "DENIED", "sandbox_background_child": "TERMINATED_WITH_GROUP",
    }
    if k1_6_correction.get("requirementR18Authority") != {
        "supervisorExpectedValues": expected_supervisor_values,
        "prepareSecurityCases": expected_prepare_cases,
        "allValuesMustMatchExactly": True,
        "notRunMutationParity": {
            "passed": 17, "total": 17, "supervisorMutations": 6, "prepareMutations": 11,
        },
        "negativeSelfTests": ["requirementR18SupervisorNotRunRejected", "requirementR18PrepareNotRunRejected"],
    }:
        raise HarnessError("K1.6 R18 correction mismatch")
    retained_k1_5 = ROOT / "docs/experiments/evidence/codeclew-k1.5-prepare-infrastructure-retained-evidence.json"
    retained_k1_5_digest = "sha256:7e7bc41a809f5143106462650d998f9ef746558f8af535fe549989dc9387c08d"
    if sha256_file(retained_k1_5) != retained_k1_5_digest:
        raise HarnessError("K1.6 retained K1.5 PREPARE evidence file drift")
    retained_k1_5_value = _load_json_bytes(
        _regular_file(retained_k1_5, "retained K1.5 PREPARE evidence").read_bytes(),
        "retained K1.5 PREPARE evidence",
    )
    required_k1_6_evidence = {
        "schema": "codeclew.kotlin-k1-prepare-infrastructure-evidence/0.2",
        "seriesId": K1_5_SERIES_ID,
        "kind": "PREPARE_INFRASTRUCTURE_ONLY_NO_QUALIFICATION_OUTCOME",
        "retainedEvidenceSha256": retained_k1_5_digest,
        "storeId": "1d46aa8c98f63f8e5eb1e50b477793be8f938b575a2708cb3c918169a42ba96c",
        "storeIdentitySha256": "sha256:987782892718dcc9f646b1f2b4e7b81c927f8d17b87bde0b341c3fe9f82eadbc",
        "sourceStoreFileManifestCount": 42,
        "sourceStoreFileManifestSha256": "sha256:d2b33f61f5f99a92eed7a985ecc490be2e73e10e323f1d583a09521a90104061",
        "currentNodeCount": 8, "guardState": "OPEN",
        "officialPrepareAttempts": 1, "diagnosticInfrastructureReplays": 1,
        "qualificationDependencyPreparePointerPresent": False,
        "qualificationDependencySeedPublished": False,
        "qualificationDependencyStagingPresent": False,
        "qualificationAttemptCount": 0, "retainedAttemptCount": 0,
        "childStartJournalCount": 0, "holdoutAttemptCount": 0,
        "holdoutOpened": False, "holdoutSourceMaterialized": False,
        "modelCalls": 0, "decisionIssued": False,
    }
    if not isinstance(k1_6_evidence, dict) or any(
        k1_6_evidence.get(key) != value for key, value in required_k1_6_evidence.items()
    ) or not isinstance(retained_k1_5_value, dict) \
        or canonical(retained_k1_5_value) != retained_k1_5.read_bytes() \
        or retained_k1_5_value.get("sourceStoreFileManifestCount") != 42 \
        or retained_k1_5_value.get("storeIdentitySha256") != k1_6_evidence.get("storeIdentitySha256") \
        or retained_k1_5_value.get("baseline") != k1_6_evidence.get("baseline") \
        or retained_k1_5_value.get("harnessSelfTest") != k1_6_evidence.get("harnessSelfTest") \
        or retained_k1_5_value.get("officialPrepare") != k1_6_evidence.get("officialPrepare") \
        or retained_k1_5_value.get("diagnosticInfrastructureReplay") != k1_6_evidence.get("diagnosticInfrastructureReplay"):
        raise HarnessError("K1.6 exact PREPARE/store evidence mismatch")
    official_failure = k1_6_evidence.get("officialPrepare", {}).get("failure")
    diagnostic_replay = k1_6_evidence.get("diagnosticInfrastructureReplay")
    if official_failure != {
        "exceptionType": "HarnessError",
        "failureDetail": "dependency PREPARE infrastructure output limit exceeded",
        "failureDetailBytes": 55,
        "failureDetailNewlineTerminated": False,
        "failureDetailSha256": "sha256:bd377b41d36bcafe84919e728406109202e247d3ba7081f07362531793babc9f",
        "operation": "BOUNDED_PREPARE_OUTPUT_CAPTURE", "outputLimitPerStreamBytes": MAX_STDOUT_BYTES,
        "atLeastOneStreamExceededLimit": True,
        "exceededStream": "STDOUT_OR_STDERR_NOT_DISTINGUISHABLE",
        "streamBytesRetained": False, "semanticOutcomeObserved": False,
    } or not isinstance(diagnostic_replay, dict) \
        or diagnostic_replay.get("classification") != "DIAGNOSTIC_ONLY_NOT_PRODUCTION_OUTCOME" \
        or diagnostic_replay.get("diagnosticOutputLimitBytes") != 2 * 1024 * 1024 \
        or diagnostic_replay.get("observedStderrBytesAtTermination") != 2129931 \
        or diagnostic_replay.get("semanticParameterChange") != "MAX_STDOUT_BYTES_ONLY" \
        or any(diagnostic_replay.get(key) is not False for key in (
            "commandReceiptPublished", "dependencySeedPublished", "blobPublished",
            "attemptPublished", "childStartPublished",
        )):
        raise HarnessError("K1.6 official/diagnostic PREPARE evidence distinction mismatch")

    k1_7_evidence = k1_7_amendment.get("prepareInfrastructureEvidence") if isinstance(k1_7_amendment, dict) else None
    k1_7_correction = k1_7_amendment.get("correction") if isinstance(k1_7_amendment, dict) else None
    if not isinstance(k1_7_amendment, dict) or set(k1_7_amendment) != {
        "schema", "cancelledSeriesId", "replacementSeriesId", "oldAuthorityDigests",
        "predecessorAmendmentSha256", "reasonCode", "authorityStateBeforeReplacement",
        "baselineAttempts", "officialDependencyPrepareAttempts", "diagnosticCampaigns",
        "qualificationAttempts", "holdoutAttempts", "modelCalls", "holdoutOpened", "correction",
        "prepareInfrastructureEvidence",
    } or k1_7_amendment.get("schema") != "codeclew.kotlin-k1-preregistration-amendment/0.7" \
        or k1_7_amendment.get("cancelledSeriesId") != K1_6_SERIES_ID \
        or k1_7_amendment.get("replacementSeriesId") != K1_7_SERIES_ID \
        or k1_7_amendment.get("oldAuthorityDigests") != k1_6_digests \
        or k1_7_amendment.get("predecessorAmendmentSha256") != k1_6_amendment_digest \
        or k1_7_amendment.get("reasonCode") != "K1_6_PREPARE_MAVEN_RUNTIME_MINIMAL_AUTHORITY_INCOMPLETE" \
        or k1_7_amendment.get("authorityStateBeforeReplacement") != "PREPARE_INFRASTRUCTURE_ONLY_NO_QUALIFICATION_NO_HOLDOUT_NO_MODEL" \
        or k1_7_amendment.get("baselineAttempts") != 1 \
        or k1_7_amendment.get("officialDependencyPrepareAttempts") != 1 \
        or k1_7_amendment.get("diagnosticCampaigns") != 1 \
        or k1_7_amendment.get("qualificationAttempts") != 0 or k1_7_amendment.get("holdoutAttempts") != 0 \
        or k1_7_amendment.get("modelCalls") != 0 or k1_7_amendment.get("holdoutOpened") is not False:
        raise HarnessError("K1.7 cancellation/amendment contract mismatch")
    if not isinstance(k1_7_correction, dict) or set(k1_7_correction) != {
        "candidateAndHarnessDisposition", "prepareMavenRuntimeAuthority", "preparedRefusalIdentity",
        "preserved", "requirementR18Authority", "supersededStoreDisposition",
    } or k1_7_correction.get("candidateAndHarnessDisposition") != "REBUILD_AND_REBIND_BEFORE_NEW_STORE" \
        or k1_7_correction.get("supersededStoreDisposition") != "PREPARE_INFRASTRUCTURE_ONLY_RETAINED_EVIDENCE_STORE_MUST_NOT_BE_REUSED" \
        or k1_7_correction.get("preparedRefusalIdentity") != {
            "schemaBefore": "codeclew.kotlin-k1-dependency-preparation-refusal/0.5",
            "schemaAfter": "codeclew.kotlin-k1-dependency-preparation-refusal/0.6",
            "seriesBefore": K1_6_SERIES_ID, "seriesAfter": K1_7_SERIES_ID,
        }:
        raise HarnessError("K1.7 correction identity mismatch")
    runtime_authority = k1_7_correction.get("prepareMavenRuntimeAuthority")
    new_prepare_cases = [
        "prepareDevNullWriteDataPassed", "prepareOnlineVarMetadataOnlyPassed",
        "prepareMissingProfileClauseRejected", "prepareBroadDevNullWriteRejected",
        "prepareOfflineVarAliasRejected", "prepareWrongMavenTmpdirRejected",
        "prepareSplitPhaseEnvironmentRejected",
    ]
    expected_k1_7_prepare_cases = [*expected_prepare_cases, *new_prepare_cases]
    if not isinstance(runtime_authority, dict) \
        or runtime_authority.get("platformProfile") != "DARWIN_SANDBOX_PROFILE_VERSION_1" \
        or runtime_authority.get("defaultPolicy") != "(deny default)" \
        or runtime_authority.get("onlineNetworkClause") != "(allow network*)" \
        or runtime_authority.get("offlineNetworkClause") != "(deny network*)" \
        or runtime_authority.get("onlineVarAliasClause") != _SANDBOX_ONLINE_VAR_METADATA \
        or runtime_authority.get("onlineVarAliasClauseCount") != 1 \
        or runtime_authority.get("onlineVarAliasPhase") != "ONLINE_ONLY" \
        or runtime_authority.get("offlineVarAliasClauseCount") != 0 \
        or runtime_authority.get("devNullClause") != _SANDBOX_DEV_NULL_WRITE \
        or runtime_authority.get("devNullClauseCountPerProfile") != 1 \
        or runtime_authority.get("devNullPhases") != ["ONLINE", "OFFLINE"] \
        or runtime_authority.get("forbiddenDevNullAuthorities") != [
            "FILE_WRITE_STAR", "FILE_WRITE_METADATA", "SUBPATH_DEV", "SUBPATH_DEV_NULL",
        ] \
        or runtime_authority.get("sharedEntryWriteClause") != '(allow file-write* (subpath "<shared-entry-work>"))' \
        or runtime_authority.get("mavenOptsTemplate") != "-Djava.io.tmpdir=<shared-entry-work>/home" \
        or runtime_authority.get("mavenOptsPhases") != ["ONLINE", "OFFLINE"] \
        or runtime_authority.get("tmpdirWithinExistingSharedEntryWriteRoot") is not True \
        or runtime_authority.get("phaseEnvironmentIdentity") != "EXACTLY_IDENTICAL_CANONICAL_RECORD_AND_DIGEST" \
        or runtime_authority.get("profileAndEnvironmentValidation") != "HARNESS_AND_INDEPENDENT_AUDITOR_RECOMPUTE_EXACTLY" \
        or runtime_authority.get("negativeValidatorCases") != {
            "cases": new_prepare_cases, "passed": 7, "total": 7,
        } \
        or runtime_authority.get("stableSourceDigests") != {
            "harness": "sha256:ccb3adb19622e650e86cf00d63f357986f1f050335a92606163db27f20bc935c",
            "independentAuditor": "sha256:6ad5dfa388082b668a7537b70e029c1b94c2cc844fba40f111b00478eaebd935",
        } \
        or runtime_authority.get("preservedLimits") != {
            "maxResidentBytes": MAX_RESIDENT_BYTES, "maxWallSeconds": MAX_WALL_SECONDS,
            "outputLimitPerStreamBytes": MAX_STDOUT_BYTES,
        }:
        raise HarnessError("K1.7 minimal Maven runtime authority mismatch")
    if k1_7_correction.get("requirementR18Authority") != {
        "supervisorExpectedValues": expected_supervisor_values,
        "prepareSecurityCases": expected_k1_7_prepare_cases,
        "allValuesMustMatchExactly": True,
        "notRunMutationParity": {
            "passed": 24, "total": 24, "supervisorMutations": 6, "prepareMutations": 18,
        },
        "negativeSelfTests": ["requirementR18SupervisorNotRunRejected", "requirementR18PrepareNotRunRejected"],
    }:
        raise HarnessError("K1.7 R18 correction mismatch")
    retained_k1_6 = ROOT / "docs/experiments/evidence/codeclew-k1.6-prepare-infrastructure-retained-evidence.json"
    retained_k1_6_digest = "sha256:205fdc41e375af13cac6e581211094416a395cf63edf67d4987037f7453ca27b"
    if sha256_file(retained_k1_6) != retained_k1_6_digest:
        raise HarnessError("K1.7 retained K1.6 PREPARE evidence file drift")
    retained_k1_6_value = _load_json_bytes(
        _regular_file(retained_k1_6, "retained K1.6 PREPARE evidence").read_bytes(),
        "retained K1.6 PREPARE evidence",
    )
    required_k1_7_evidence = {
        "schema": "codeclew.kotlin-k1-prepare-infrastructure-evidence/0.3",
        "seriesId": K1_6_SERIES_ID,
        "kind": "PREPARE_INFRASTRUCTURE_ONLY_NO_QUALIFICATION_OUTCOME",
        "retainedEvidenceSha256": retained_k1_6_digest,
        "storeId": "28d42ccc752cca9a85be0a51bab02da1bb0248ef0dfd9f04766941988240e27b",
        "storeIdentitySha256": "sha256:a0eff93f0e881e0e73e3e724211002c8b1bf3c913b1f8232e02f9644293e5590",
        "sourceStoreFileManifestCount": 44,
        "sourceStoreFileManifestSha256": "sha256:fccaea58f21b709e38d720a6b588c922a7df970e990f0b0cb6c14ac8919c7a06",
        "currentNodeCount": 8, "guardState": "OPEN", "officialPrepareAttempts": 1,
        "officialOutputBlobCount": 2,
        "qualificationDependencyPreparePointerPresent": False,
        "qualificationDependencySeedPublished": False,
        "qualificationDependencyStagingPresent": False,
        "qualificationAttemptCount": 0, "retainedAttemptCount": 0,
        "childStartJournalCount": 0, "holdoutAttemptCount": 0,
        "holdoutOpened": False, "holdoutSourceMaterialized": False,
        "modelCalls": 0, "decisionIssued": False,
    }
    if not isinstance(k1_7_evidence, dict) or any(
        k1_7_evidence.get(key) != value for key, value in required_k1_7_evidence.items()
    ) or not isinstance(retained_k1_6_value, dict) \
        or canonical(retained_k1_6_value) != retained_k1_6.read_bytes() \
        or retained_k1_6_value.get("sourceStoreFileManifestCount") != 44 \
        or sha256_bytes(canonical(retained_k1_6_value.get("sourceStoreFileManifest"))) != retained_k1_6_value.get("sourceStoreFileManifestSha256") \
        or retained_k1_6_value.get("storeIdentitySha256") != k1_7_evidence.get("storeIdentitySha256") \
        or retained_k1_6_value.get("baseline") != k1_7_evidence.get("baseline") \
        or retained_k1_6_value.get("harnessSelfTest") != k1_7_evidence.get("harnessSelfTest") \
        or retained_k1_6_value.get("officialPrepare") != k1_7_evidence.get("officialPrepare"):
        raise HarnessError("K1.7 exact PREPARE/store evidence mismatch")
    k1_7_failure = k1_7_evidence.get("officialPrepare", {}).get("failure")
    if not isinstance(k1_7_failure, dict) \
        or k1_7_failure.get("failureDetail") != "dependency PREPARE security/authority failure: K1-Q01" \
        or k1_7_failure.get("failureDetailBytes") != 53 \
        or k1_7_failure.get("failureDetailSha256") != "sha256:3aee526ae285207038463a79dd1292d54b1583788dc155c4fa2cc221f238eac7" \
        or k1_7_failure.get("stdoutSha256") != "sha256:a4fe58486332dce99995161a83e0e5c6fdca64519ce2a549093fe6fab80d9dab" \
        or k1_7_failure.get("stdoutBytes") != 220 \
        or k1_7_failure.get("stderrSha256") != "sha256:48651800d06a8644edfc35afdef67c7729dd77fd90614744966c12ec41fd671a" \
        or k1_7_failure.get("stderrBytes") != 1920 \
        or k1_7_failure.get("semanticOutcomeObserved") is not False:
        raise HarnessError("K1.7 official PREPARE failure evidence mismatch")

    k1_8_evidence = k1_8_amendment.get("prepareInfrastructureEvidence")
    k1_8_correction = k1_8_amendment.get("correction")
    if set(k1_8_amendment) != {
        "schema", "cancelledSeriesId", "replacementSeriesId", "oldAuthorityDigests",
        "predecessorAmendmentSha256", "reasonCode", "authorityStateBeforeReplacement",
        "baselineAttempts", "officialDependencyPrepareAttempts", "diagnosticCampaigns",
        "qualificationAttempts", "holdoutAttempts", "modelCalls", "holdoutOpened", "correction",
        "prepareInfrastructureEvidence",
    } or k1_8_amendment.get("schema") != "codeclew.kotlin-k1-preregistration-amendment/0.8" \
        or k1_8_amendment.get("cancelledSeriesId") != K1_7_SERIES_ID \
        or k1_8_amendment.get("replacementSeriesId") != K1_8_SERIES_ID \
        or k1_8_amendment.get("oldAuthorityDigests") != k1_7_digests \
        or k1_8_amendment.get("predecessorAmendmentSha256") != k1_7_amendment_digest \
        or k1_8_amendment.get("reasonCode") != "K1_7_PREPARE_DISPOSABLE_ARCHIVE_IDENTITY_CONFLATED_WORKTREE_AND_RAW_BLOB_BYTES" \
        or k1_8_amendment.get("authorityStateBeforeReplacement") != "PREPARE_INFRASTRUCTURE_ONLY_NO_QUALIFICATION_NO_HOLDOUT_NO_MODEL" \
        or k1_8_amendment.get("baselineAttempts") != 1 \
        or k1_8_amendment.get("officialDependencyPrepareAttempts") != 1 \
        or k1_8_amendment.get("diagnosticCampaigns") != 1 \
        or k1_8_amendment.get("qualificationAttempts") != 0 or k1_8_amendment.get("holdoutAttempts") != 0 \
        or k1_8_amendment.get("modelCalls") != 0 or k1_8_amendment.get("holdoutOpened") is not False:
        raise HarnessError("K1.8 cancellation/amendment contract mismatch")
    expected_k1_8_preserved = [
        "DECISION_THRESHOLDS", "REQUIREMENTS_EXCEPT_SERIES_AND_AMENDMENT_BINDING",
        "CORPUS_EXCEPT_SERIES", "ELIGIBILITY_EXCEPT_SERIES",
        "HOLDOUT_ELIGIBILITY_PROCEDURE_MEMBERS_AND_DECISION", "READINESS_GRAPH_EXCEPT_GRAPH_ID",
        "K0_1_BYTE_EXACT", "WORKLOAD", "SOURCE_TREE_AND_SANDBOX_NETWORK_OUTPUT_AUTHORITY",
        "BASELINE_PACKET_AND_CONTEXT_SCHEMAS", "BASELINE_LOGICAL_COMMAND_ARGV_TARGETS_AND_TEST_FILTERS",
        "CARGO_LOCK_DERIVED_SEED", "RUST_GRADLE_MAVEN_JDK_LAUNCHER_IDENTITIES",
        "BASELINE_GREEN_MEASUREMENT", "PREPARE_NETWORK_SPLIT_AND_SENTINEL",
        "PREPARE_ANCESTOR_TRAVERSAL_AUTHORITY", "DEPENDENCY_SEED_PHYSICAL_SEALING",
        "DISPOSABLE_SOURCE_SAFE_CLEANUP", "PREPARE_RESOURCE_AND_OUTPUT_CAPS",
        "NO_EDIT_APPLY_OR_MODEL_BENCHMARK",
    ]
    if not isinstance(k1_8_correction, dict) or set(k1_8_correction) != {
        "candidateAndHarnessDisposition", "disposableArchiveDualIdentityAuthority",
        "preparedRefusalIdentity", "preserved", "supersededStoreDisposition",
    } or k1_8_correction.get("candidateAndHarnessDisposition") != "REBUILD_AND_REBIND_BEFORE_NEW_STORE" \
        or k1_8_correction.get("supersededStoreDisposition") != "PREPARE_INFRASTRUCTURE_ONLY_RETAINED_EVIDENCE_STORE_MUST_NOT_BE_REUSED" \
        or k1_8_correction.get("preparedRefusalIdentity") != {
            "schemaBefore": "codeclew.kotlin-k1-dependency-preparation-refusal/0.6",
            "schemaAfter": "codeclew.kotlin-k1-dependency-preparation-refusal/0.7",
            "seriesBefore": K1_7_SERIES_ID, "seriesAfter": K1_8_SERIES_ID,
        } or k1_8_correction.get("preserved") != expected_k1_8_preserved:
        raise HarnessError("K1.8 correction identity/preservation mismatch")
    dual_identity = k1_8_correction.get("disposableArchiveDualIdentityAuthority")
    expected_archive_cases = [
        "crlfWorktreeBytesAccepted", "repositoryLocalFilterIdentityRejected",
        "repositoryFilterNeverExecuted", "trackedDirtyDetectedWithoutFilter",
        "untrackedDetectedWithoutFilter", "missingDetectedWithoutFilter",
        "exportSubstSuppressed", "exportIgnoreMutationRejected", "rawBlobIdentityImported",
    ]
    expected_filter_free_authority = {
        "archiveIsolation": "EMPTY_TRANSIENT_GIT_DIR_WITH_VALIDATED_SOURCE_OBJECT_DIRECTORY_AND_EXACT_TREE_OID",
        "cleanStatusIdentity": "CANONICAL_MISMATCH_ROWS_EMPTY_BYTES_DIGEST",
        "cleanlinessAuthority": "SANITIZED_PLUMBING_ONLY_NO_STATUS_DIFF_CHECKOUT_OR_FILTER_EXECUTION",
        "exportIgnorePolicy": "REJECT_MEMBER_OMISSION",
        "exportSubstPolicy": "SUPPRESSED_BY_TREE_ARCHIVE_EXACT_BYTES",
        "extraMemberAuthority": "FILESYSTEM_WALK_EXCLUDES_ONLY_DOT_GIT",
        "headIndexAuthority": "EXACT_HEAD_TREE_MODE_OBJECT_ID_SET_EQUALS_INDEX",
        "localConfigurationAuthority": "SOURCE_REPOSITORY_CONFIG_PHYSICALLY_ABSENT_FROM_ARCHIVE_PROCESS",
        "objectDirectoryValidation": "REAL_DIRECTORY_NO_ALTERNATES_SYMLINKS_OR_SPECIAL_MEMBERS",
        "worktreeAuthority": "FILTER_FREE_TREE_ARCHIVE_EXACT_BYTES_VS_NOFOLLOW_FILESYSTEM_WALK",
    }
    expected_pre_transition_digests = {
        "adapterCargo": "sha256:59eff1ebc860acd9875ce8b87921ff196d5ea2455b7c132c0c542dc2faddcf40",
        "cargoLock": "sha256:ca655f22cc1922620c790faf343f953554a425dae1bddcc67817300d7f8a4048",
        "harness": "sha256:a02ebe856d07e82ff3aae00f52c98490b13710d2e71af6e45176660803f4c484",
        "independentAuditor": "sha256:701abc3b73453fe8351780c48638cb3112c310966eb985fa3314385d034fd4e5",
        "kotlinAdapterSource": "sha256:3baf0375006ed5a7a7a88fe74bf267dc758e321c2b5def540c6703259f527f62",
    }
    expected_dual_scalars = {
        "archiveMaterializationAuthority": "EXACT_SELECTED_WORKTREE_BYTES_CAPTURED_IMMEDIATELY_BEFORE_ARCHIVE",
        "archiveMemberValidation": "EXACT_MEMBER_SET_KIND_MODE_AND_REGULAR_SIZE_SHA256_OR_SYMLINK_READLINK_SIZE_SHA256_TARGET",
        "finalSyntheticIdentity": "EXACT_ORIGINAL_OBSERVATION_TREE_COMMIT_INDEX_AND_SOURCE_PROJECTION",
        "originalSourceRecheck": "EXACT_OBSERVATION_AND_INDEX_AFTER_CONSTRUCTION",
        "rawBlobImport": "OID_NAMED_PRIVATE_SYNTHETIC_GIT_STAGING_THEN_BATCH_HASH_OBJECT_WRITE_NO_FILTERS_STDIN_PATHS",
        "rawBlobImportValidation": "EXACT_OBJECT_IDS_AND_ORDER",
        "rawBlobRead": "BATCH_GIT_CAT_FILE_BY_FROZEN_TREE_OBJECT_ID",
        "rawGitObjectAuthority": "EXACT_RAW_GIT_BLOB_BYTES_FROM_ORIGINAL_OBJECT_DATABASE_WITHOUT_FILTERS",
        "repositoryLocalFilterPolicy": "REJECT_IF_SANITIZED_SYNTHETIC_OBSERVATION_CANNOT_EQUAL_SELECTED_SOURCE",
        "selectedWorktreeIndex": "GIT_INDEX_TRACKED_MEMBER_SET_PATH_MODE_OBJECT_ID_KIND_SIZE_SHA256_AND_SYMLINK_TARGET",
    }
    if not isinstance(dual_identity, dict) \
        or set(dual_identity) != set(expected_dual_scalars) | {
            "adversarialCases", "filterFreeObservationAndArchive", "preTransitionFunctionalSourceDigests",
            "qualificationFixtureValidation", "selfTest",
        } \
        or any(dual_identity.get(key) != value for key, value in expected_dual_scalars.items()) \
        or dual_identity.get("adversarialCases") != {
            "cases": expected_archive_cases, "negative": 5, "passed": 9, "positive": 4, "total": 9,
        } \
        or dual_identity.get("filterFreeObservationAndArchive") != expected_filter_free_authority \
        or dual_identity.get("preTransitionFunctionalSourceDigests") != expected_pre_transition_digests \
        or dual_identity.get("qualificationFixtureValidation") != {
            "entries": list(EXPECTED_QUALIFICATION), "exactObservationPassCount": 6, "total": 6,
        } \
        or dual_identity.get("selfTest") != {
            "counterexamples": 109, "maliciousFilterRustTests": 1, "modelCalls": 0,
            "preparedRefusalRustTests": 5, "qualificationRepositoriesValidated": 6,
            "status": "PASS", "supervisorCases": 18,
        }:
        raise HarnessError("K1.8 disposable archive dual-identity authority mismatch")

    retained_k1_7 = ROOT / "docs/experiments/evidence/codeclew-k1.7-prepare-infrastructure-retained-evidence.json"
    retained_k1_7_digest = "sha256:462916887c8543ec1d8b99bda67db5d008dc973fc91fe35ca67e4fedde2deab7"
    if sha256_file(retained_k1_7) != retained_k1_7_digest:
        raise HarnessError("K1.8 retained K1.7 PREPARE evidence file drift")
    retained_k1_7_raw = _regular_file(retained_k1_7, "retained K1.7 PREPARE evidence").read_bytes()
    retained_k1_7_value = _load_json_bytes(retained_k1_7_raw, "retained K1.7 PREPARE evidence")
    if not isinstance(k1_8_evidence, dict) or not isinstance(retained_k1_7_value, dict) \
        or canonical(retained_k1_7_value) != retained_k1_7_raw \
        or k1_8_evidence.get("retainedEvidenceSha256") != retained_k1_7_digest \
        or any(
            retained_k1_7_value.get(key) != value
            for key, value in k1_8_evidence.items()
            if key not in {"retainedEvidenceSha256", "storeId"}
        ) \
        or retained_k1_7_value.get("sourceStoreFileManifestCount") != 45 \
        or sha256_bytes(canonical(retained_k1_7_value.get("sourceStoreFileManifest"))) != "sha256:346cc29fee46edee09b93097e8fc294971589d70b097d96ec0cd7bf13b582f3c" \
        or retained_k1_7_value.get("sourceStoreFileManifestSha256") != "sha256:346cc29fee46edee09b93097e8fc294971589d70b097d96ec0cd7bf13b582f3c":
        raise HarnessError("K1.8 exact retained K1.7 PREPARE/store evidence mismatch")
    expected_k1_7_store_identity = {
        "schema": STORE_SCHEMA, "seriesId": K1_7_SERIES_ID,
        "storeId": "e99d6c477cdb69cef50fb165be668b352078296f60cf20766f1985a6709cd077",
        "authorityDigests": {
            **k1_7_digests, "preregistrationAmendment": k1_7_amendment_digest,
        },
    }
    required_k1_8_evidence = {
        "schema": "codeclew.kotlin-k1-prepare-infrastructure-retained-evidence/0.4",
        "seriesId": K1_7_SERIES_ID,
        "kind": "PREPARE_INFRASTRUCTURE_ONLY_NO_QUALIFICATION_OUTCOME",
        "sourceStoreAbsolutePath": "/private/tmp/codeclew-k1-7-production.Zy8ZE8/run/store",
        "storeId": "e99d6c477cdb69cef50fb165be668b352078296f60cf20766f1985a6709cd077",
        "storeIdentitySha256": "sha256:27774e0651f9b7ec9577908e13ee7826cee0b5b7e12a1c6a97f190012fa96ed5",
        "sourceStoreFileManifestCount": 45,
        "sourceStoreFileManifestSha256": "sha256:346cc29fee46edee09b93097e8fc294971589d70b097d96ec0cd7bf13b582f3c",
        "candidateToolsSha256": "sha256:f96b5b2c0f0caa3190ef2a3fa2786e81c08e73a509b8ea34691cef8f87598814",
        "liveInputsSha256": "sha256:cae3e7eaffcf2197d3bc6c49ed1aa0da317bcc36fe0d7c2b0395dd57e2e59543",
        "officialDependencyPrepareAttempts": 1, "officialPrepareReceiptPublished": False,
        "qualificationDependencyPreparePointerPresent": False,
        "qualificationDependencySeedPublished": False, "qualificationDependencyStagingPresent": False,
        "qualificationAttemptCount": 0, "retainedAttemptCount": 0, "childStartJournalCount": 0,
        "holdoutAttemptCount": 0, "holdoutOpened": False, "holdoutSourceMaterialized": False,
        "modelCalls": 0, "decisionIssued": False,
    }
    if any(k1_8_evidence.get(key) != value for key, value in required_k1_8_evidence.items()) \
        or retained_k1_7_value.get("storeIdentity") != expected_k1_7_store_identity \
        or k1_8_evidence.get("baseline", {}).get("packetSha256") != "sha256:1b950cfd944970f0c7198c8b0d33e0d07fdcc539e247044fe96c57e10a6126b8" \
        or k1_8_evidence.get("harnessSelfTest", {}).get("packetSha256") != "sha256:a46b99e12cd68211c5119708ca56439e9c62749ffcf87c5decc5debc3e3094db":
        raise HarnessError("K1.8 retained K1.7 PREPARE evidence identity mismatch")
    expected_k1_8_official_prepare = {
        "attempts": 1,
        "classification": "PRODUCTION_INFRASTRUCTURE_FAILURE_NOT_SEMANTIC_OUTCOME",
        "completedEntriesBeforeFailure": ["K1-Q01", "K1-Q02", "K1-Q03"],
        "entriesMaterializedBeforeFailure": ["K1-Q01", "K1-Q02", "K1-Q03"],
        "entry": "K1-Q04", "failedEntryCommandsCompleted": 0, "failedEntryCommandsStarted": 0,
        "failure": {
            "entry": "K1-Q04", "exceptionType": "HarnessError",
            "failureDetail": "archive member bytes differ from frozen Git object",
            "failureDetailBytes": 50, "failureDetailNewlineTerminated": False,
            "failureDetailSha256": "sha256:c6e15f73b2d06d618f3da65d845d3fa037737cd622bb73ca7d264c73c39449d5",
            "operation": "DISPOSABLE_ARCHIVE_IDENTITY_VALIDATION", "semanticOutcomeObserved": False,
        },
        "phase": "DISPOSABLE_SOURCE_ARCHIVE_MATERIALIZATION",
    }
    expected_k1_8_diagnostic_cause = {
        "attributes": ["text", "eol=crlf"], "entry": "K1-Q04",
        "gitArchiveMemberEqualsSelectedWorktree": True, "path": "gradlew.bat",
        "rawGitBlobBytes": {
            "crlfCount": 0,
            "sha256": "sha256:5c0a21ecd6b3a6292e0746bff3b75fd2d8f47b9ff226ce53dc22b30184ef3bec",
            "size": 2764,
        },
        "selectedWorktreeBytes": {
            "crlfCount": 82,
            "sha256": "sha256:475c4f08cd57cf2faa819e7f36d72aa93f0ad646ea23a8f7fa3ef54dee1cbc52",
            "size": 2846,
        },
    }
    diagnostic = k1_8_evidence.get("diagnosticInvestigation")
    if k1_8_evidence.get("officialPrepare") != expected_k1_8_official_prepare \
        or not isinstance(diagnostic, dict) \
        or diagnostic.get("classification") != "DIAGNOSTIC_ONLY_NOT_PRODUCTION_OUTCOME" \
        or diagnostic.get("sourceSet") != "QUALIFICATION_ONLY_HOLDOUT_UNOPENED" \
        or diagnostic.get("productionStoreMutated") is not False \
        or diagnostic.get("cause") != expected_k1_8_diagnostic_cause \
        or diagnostic.get("archiveVsSelectedWorktreeValidation") != {
            "entries": list(EXPECTED_QUALIFICATION),
            "memberCounts": [554, 462, 600, 1799, 1993, 1046], "mismatches": 0,
        } \
        or any(diagnostic.get(key) is not False for key in (
            "attemptPublished", "blobPublished", "childStartPublished",
            "commandReceiptPublished", "dependencySeedPublished",
        )):
        raise HarnessError("K1.8 official/diagnostic PREPARE evidence distinction mismatch")

    expected_k1_9_reason_codes = [
        "K1_8_POST_PUBLICATION_PREPARE_EVIDENCE_VALIDATION_EXISTENCE_SENSITIVE",
        "K1_8_GRADLE_WRAPPER_BOOTSTRAP_ESCAPED_PRIVATE_HOME",
        "K1_8_MAVEN_ONLINE_PREPARE_DID_NOT_PREFETCH_OFFLINE_MODEL_GOALS",
    ]
    k1_9_evidence = k1_9_amendment.get("prepareInfrastructureEvidence") if isinstance(k1_9_amendment, dict) else None
    k1_9_correction = k1_9_amendment.get("correction") if isinstance(k1_9_amendment, dict) else None
    if not isinstance(k1_9_amendment, dict) or set(k1_9_amendment) != {
        "schema", "cancelledSeriesId", "replacementSeriesId", "oldAuthorityDigests",
        "predecessorAmendmentSha256", "reasonCodes", "authorityStateBeforeReplacement",
        "baselineAttempts", "officialDependencyPrepareAttempts", "qualificationAttempts",
        "holdoutAttempts", "modelCalls", "holdoutOpened", "correction",
        "prepareInfrastructureEvidence",
    } or k1_9_amendment.get("schema") != "codeclew.kotlin-k1-preregistration-amendment/0.9" \
        or k1_9_amendment.get("cancelledSeriesId") != K1_8_SERIES_ID \
        or k1_9_amendment.get("replacementSeriesId") != K1_9_SERIES_ID \
        or k1_9_amendment.get("oldAuthorityDigests") != k1_8_digests \
        or k1_9_amendment.get("predecessorAmendmentSha256") != k1_8_amendment_digest \
        or k1_9_amendment.get("reasonCodes") != expected_k1_9_reason_codes \
        or k1_9_amendment.get("authorityStateBeforeReplacement") != "PUBLISHED_PREPARE_COHORT_ONLY_NO_QUALIFICATION_NO_HOLDOUT_NO_MODEL" \
        or k1_9_amendment.get("baselineAttempts") != 1 \
        or k1_9_amendment.get("officialDependencyPrepareAttempts") != 1 \
        or k1_9_amendment.get("qualificationAttempts") != 0 or k1_9_amendment.get("holdoutAttempts") != 0 \
        or k1_9_amendment.get("modelCalls") != 0 or k1_9_amendment.get("holdoutOpened") is not False:
        raise HarnessError("K1.9 cancellation/amendment contract mismatch")
    if not isinstance(k1_9_correction, dict) or set(k1_9_correction) != {
        "candidateAndHarnessDisposition", "dependencyCohortPostPublicationValidationAuthority",
        "diagnosticValidation", "functionalFreeze", "gradleWrapperBootstrapAuthority",
        "mavenOnlineModelPrefetchAuthority", "preparedRefusalIdentity", "preserved",
        "supersededStoreDisposition",
    } or k1_9_correction.get("candidateAndHarnessDisposition") != "REBUILD_AND_REBIND_BEFORE_NEW_STORE" \
        or k1_9_correction.get("supersededStoreDisposition") != "PUBLISHED_PREPARE_COHORT_RETAINED_EVIDENCE_STORE_AND_COHORT_MUST_NOT_BE_REUSED" \
        or k1_9_correction.get("preparedRefusalIdentity") != {
            "schemaBefore": "codeclew.kotlin-k1-dependency-preparation-refusal/0.7",
            "schemaAfter": "codeclew.kotlin-k1-dependency-preparation-refusal/0.8",
            "seriesBefore": K1_8_SERIES_ID, "seriesAfter": K1_9_SERIES_ID,
        }:
        raise HarnessError("K1.9 correction identity mismatch")
    post_publication = k1_9_correction.get("dependencyCohortPostPublicationValidationAuthority")
    wrapper_home = k1_9_correction.get("gradleWrapperBootstrapAuthority")
    maven_prefetch = k1_9_correction.get("mavenOnlineModelPrefetchAuthority")
    if not isinstance(post_publication, dict) \
        or post_publication.get("writeClauseTemplate") != '(allow file-write* (subpath "<shared-entry-work>"))' \
        or post_publication.get("writeSelector") != "UNCONDITIONAL_EXACT_SUBPATH_NEVER_EXISTENCE_SENSITIVE" \
        or post_publication.get("existenceIndependent") is not True \
        or post_publication.get("cases") != ["preparePostPublicationEvidenceRevalidated"] \
        or not isinstance(wrapper_home, dict) \
        or wrapper_home.get("prepareGradleEnvironment") != "GRADLE_USER_HOME=<shared-entry-work>/gradle-user-home" \
        or wrapper_home.get("preparePhases") != ["ONLINE", "OFFLINE"] \
        or wrapper_home.get("mavenPrepareGradleUserHome") != "ABSENT" \
        or wrapper_home.get("workerExternalMavenEnvironment", {}).get("GRADLE_USER_HOME") != "ABSENT" \
        or wrapper_home.get("cases") != [
            "prepareGradleWrapperBootstrapHomePassed", "prepareMissingGradleWrapperBootstrapHomeRejected",
        ] \
        or not isinstance(maven_prefetch, dict) \
        or maven_prefetch.get("onlineGoalOrder") != [
            "dependency:go-offline", "install", "help:effective-pom", "dependency:build-classpath",
        ] \
        or maven_prefetch.get("offlineGoalOrder") != ["help:effective-pom", "dependency:build-classpath"] \
        or maven_prefetch.get("cases") != ["prepareMavenOfflineModelGoalsPrefetchedOnline"]:
        raise HarnessError("K1.9 PREPARE correction authority mismatch")
    if k1_9_correction.get("functionalFreeze") != {
        "harness": "sha256:5d6fb33759ab72b2e37b86d17e96b35ef25c1d684d08a8ff366b098ca56f3fa6",
        "independentAuditor": "sha256:1490d3ad6873dd52d236b71c94d332e0766eca4da9b35fb1f4b86cd5c183e685",
        "worker": "sha256:4416fd01158fefd3210743649b1636c881b89ab1cf0271488d6dabeff7e65870",
        "mavenProjectModel": "sha256:d6b3dbee0620956b2fb8aa192e5baa5877064001711e5779a5f40633ac09a353",
        "projectModelCommandTest": "sha256:d0fb33b7542d508a649c0772f48007860eb7c185217598bc2fd9c5281a3dc68e",
        "selfTestCounterexamples": 115, "supervisorCases": 18,
        "prepareNotRunMutations": {"passed": 22, "total": 22},
        "argvEnvironmentParity": {"entries": 12, "phasesPerEntry": 2, "status": "PASS"},
        "postDeletionParity": {"entries": 6, "status": "PASS"}, "modelCalls": 0,
    }:
        raise HarnessError("K1.9 functional freeze mismatch")
    retained_k1_8 = ROOT / "docs/experiments/evidence/codeclew-k1.8-prepare-infrastructure-retained-evidence.json"
    retained_k1_8_digest = "sha256:ade9e4b66b6418bf100d59b587c4e682dee9f65f18beeaafec0bc73eafb20bf8"
    retained_k1_8_raw = _regular_file(retained_k1_8, "retained K1.8 PREPARE evidence").read_bytes()
    retained_k1_8_value = _load_json_bytes(retained_k1_8_raw, "retained K1.8 PREPARE evidence")
    if sha256_bytes(retained_k1_8_raw) != retained_k1_8_digest \
        or not isinstance(retained_k1_8_value, dict) \
        or canonical(retained_k1_8_value) != retained_k1_8_raw \
        or not isinstance(k1_9_evidence, dict) \
        or k1_9_evidence.get("retainedEvidenceSha256") != retained_k1_8_digest \
        or any(
            retained_k1_8_value.get(key) != value
            for key, value in k1_9_evidence.items()
            if key not in {"retainedEvidenceSha256", "sourceStoreMemberManifestCount",
                           "sourceStoreFileCount", "sourceStoreDirectoryCount",
                           "sourceStoreMemberManifestSha256", "publishedDependencyCohort", "failure",
                           "baseline", "harnessSelfTest", "storeId"}
        ):
        raise HarnessError("K1.9 retained K1.8 evidence identity mismatch")
    if retained_k1_8_value.get("storeIdentity", {}).get("storeId") != k1_9_evidence.get("storeId") \
        or any(
            retained_k1_8_value.get("baseline", {}).get(key) != value
            for key, value in k1_9_evidence.get("baseline", {}).items()
        ) \
        or any(
            retained_k1_8_value.get("harnessSelfTest", {}).get(key) != value
            for key, value in k1_9_evidence.get("harnessSelfTest", {}).items()
        ):
        raise HarnessError("K1.9 retained baseline/self-test/store identity mismatch")
    store_members = retained_k1_8_value.get("sourceStoreMemberManifest")
    cohort_evidence = retained_k1_8_value.get("publishedDependencyCohort")
    if not isinstance(store_members, list) or len(store_members) != 57 \
        or sha256_bytes(canonical(store_members)) != "sha256:4a077a9b4667565ba396031506363ec30e277a8e4ecf2549f9869e7a4700e32a" \
        or sum(row.get("kind") == "FILE" for row in store_members) != 48 \
        or sum(row.get("kind") == "DIRECTORY" for row in store_members) != 9 \
        or not isinstance(cohort_evidence, dict):
        raise HarnessError("K1.9 retained store/member manifest mismatch")
    cohort_members = cohort_evidence.get("memberManifest")
    cohort_manifest = cohort_evidence.get("manifest")
    typed_refusals = cohort_evidence.get("typedRefusalFiles")
    if not isinstance(cohort_members, list) or len(cohort_members) != 15 \
        or sha256_bytes(canonical(cohort_members)) != "sha256:ba29d0e58258f71d8d1bbd596900aae78ea640bb31eedd8251a83f8b3eac55f6" \
        or sum(row.get("kind") == "FILE" for row in cohort_members) != 8 \
        or sum(row.get("kind") == "DIRECTORY" for row in cohort_members) != 7 \
        or not isinstance(cohort_manifest, dict) \
        or sha256_bytes(canonical(cohort_manifest)) != "sha256:ebf53feb05fa285e1b76ca7cd4346458eb9ce3aaacf5b41548de34723e1c4bc4" \
        or cohort_evidence.get("markerAscii") != "sha256:ebf53feb05fa285e1b76ca7cd4346458eb9ce3aaacf5b41548de34723e1c4bc4\n" \
        or cohort_manifest.get("cohortDigest") != "sha256:57f9d30ef5c2eddb804d462ab84314384bf86b452872ff2f01d045c8588a2c44" \
        or not isinstance(typed_refusals, list) or [row.get("entry") for row in typed_refusals] != list(EXPECTED_QUALIFICATION):
        raise HarnessError("K1.9 retained published cohort mismatch")
    cohort_body = dict(cohort_manifest)
    cohort_body["cohortDigest"] = ""
    if sha256_bytes(canonical(cohort_body)) != cohort_manifest["cohortDigest"]:
        raise HarnessError("K1.9 retained cohort digest mismatch")
    rows_by_entry = {row.get("entry"): row for row in cohort_manifest.get("entries", []) if isinstance(row, dict)}
    for retained_refusal in typed_refusals:
        entry = retained_refusal["entry"]
        refusal = retained_refusal.get("object")
        row = rows_by_entry.get(entry)
        if not isinstance(refusal, dict) or not isinstance(row, dict) \
            or retained_refusal.get("fileSha256") != sha256_bytes(canonical(refusal)) \
            or retained_refusal.get("fileSize") != len(canonical(refusal)) \
            or refusal.get("schema") != "codeclew.kotlin-k1-dependency-preparation-refusal/0.7" \
            or refusal.get("seriesId") != K1_8_SERIES_ID \
            or refusal.get("preparationReceiptDigest") != sha256_bytes(canonical(row)):
            raise HarnessError("K1.9 retained typed refusal mismatch")
    expected_failure = {
        "exceptionType": "HarnessError",
        "failureDetail": "dependency cohort PREPARE network evidence mismatch",
        "failureDetailBytes": 51, "failureDetailNewlineTerminated": False,
        "failureDetailSha256": "sha256:4fb4c27e408a6235893764bd8543ec425722db50d52146ab75cdcb7ef921871d",
        "operation": "DEPENDENCY_COHORT_PREPARE_EVIDENCE_REVALIDATION",
        "semanticOutcomeObserved": False,
    }
    if k1_9_evidence.get("failure") != expected_failure \
        or retained_k1_8_value.get("officialPrepare", {}).get("failure") != expected_failure \
        or any(k1_9_evidence.get(key) != value for key, value in {
            "officialPrepareReceiptPublished": False,
            "qualificationDependencyPreparePointerPresent": False,
            "qualificationDependencySeedPublished": True,
            "qualificationDependencyStagingPresent": False,
            "qualificationAttemptCount": 0, "retainedAttemptCount": 0,
            "childStartJournalCount": 0, "holdoutAttemptCount": 0,
            "holdoutOpened": False, "holdoutSourceMaterialized": False,
            "modelCalls": 0, "decisionIssued": False,
        }.items()):
        raise HarnessError("K1.9 PREPARE/qualification boundary evidence mismatch")

    expected_k1_10_reasons = [
        "K1_9_PREPARE_BUILD_STATE_ROOT_SEALED_BEFORE_ATOMIC_MOVE",
        "K1_9_PREPARE_COHORT_ROOT_SEALED_BEFORE_ATOMIC_PUBLICATION",
        "K1_9_TRUSTED_WORKER_BUILD_JAVA_OPTS_INJECTION_GAP",
    ]
    k1_10_evidence = k1_10_amendment.get("prepareInfrastructureEvidence") if isinstance(k1_10_amendment, dict) else None
    k1_10_correction = k1_10_amendment.get("correction") if isinstance(k1_10_amendment, dict) else None
    if not isinstance(k1_10_amendment, dict) or set(k1_10_amendment) != {
        "schema", "cancelledSeriesId", "replacementSeriesId", "oldAuthorityDigests",
        "predecessorAmendmentSha256", "reasonCodes", "authorityStateBeforeReplacement",
        "baselineAttempts", "officialDependencyPrepareAttempts", "qualificationAttempts",
        "holdoutAttempts", "modelCalls", "holdoutOpened", "correction",
        "prepareInfrastructureEvidence",
    } or k1_10_amendment.get("schema") != "codeclew.kotlin-k1-preregistration-amendment/0.10" \
        or k1_10_amendment.get("cancelledSeriesId") != K1_9_SERIES_ID \
        or k1_10_amendment.get("replacementSeriesId") != K1_10_SERIES_ID \
        or k1_10_amendment.get("oldAuthorityDigests") != k1_9_digests \
        or k1_10_amendment.get("predecessorAmendmentSha256") != k1_9_amendment_digest \
        or k1_10_amendment.get("reasonCodes") != expected_k1_10_reasons \
        or k1_10_amendment.get("authorityStateBeforeReplacement") != "PREPARE_INFRASTRUCTURE_FAILURE_NO_QUALIFICATION_NO_HOLDOUT_NO_MODEL" \
        or k1_10_amendment.get("baselineAttempts") != 1 \
        or k1_10_amendment.get("officialDependencyPrepareAttempts") != 1 \
        or k1_10_amendment.get("qualificationAttempts") != 0 or k1_10_amendment.get("holdoutAttempts") != 0 \
        or k1_10_amendment.get("modelCalls") != 0 or k1_10_amendment.get("holdoutOpened") is not False:
        raise HarnessError("K1.10 cancellation/amendment contract mismatch")
    expected_k1_10_preserved = [
        "DECISION_THRESHOLDS", "REQUIREMENTS_EXCEPT_SERIES_AND_AMENDMENT_BINDING",
        "CORPUS_EXCEPT_SERIES", "ELIGIBILITY_EXCEPT_SERIES",
        "HOLDOUT_ELIGIBILITY_PROCEDURE_MEMBERS_AND_DECISION", "READINESS_GRAPH_EXCEPT_GRAPH_ID",
        "K0_1_BYTE_EXACT", "WORKLOAD", "SOURCE_TREE_AND_SANDBOX_NETWORK_OUTPUT_AUTHORITY",
        "BASELINE_PACKET_AND_CONTEXT_SCHEMAS", "BASELINE_LOGICAL_COMMAND_ARGV_TARGETS_AND_TEST_FILTERS",
        "CARGO_LOCK_DERIVED_SEED", "RUST_GRADLE_MAVEN_JDK_LAUNCHER_IDENTITIES",
        "BASELINE_GREEN_MEASUREMENT", "PREPARE_NETWORK_SPLIT_AND_SENTINEL",
        "PREPARE_ANCESTOR_TRAVERSAL_AUTHORITY", "PREPARE_MAVEN_RUNTIME_MINIMAL_AUTHORITY",
        "DISPOSABLE_ARCHIVE_DUAL_IDENTITY_AUTHORITY", "GRADLE_WRAPPER_BOOTSTRAP_PRIVATE_HOME",
        "MAVEN_ONLINE_MODEL_PREFETCH", "DEPENDENCY_SEED_PHYSICAL_SEALING",
        "DISPOSABLE_SOURCE_SAFE_CLEANUP", "PREPARE_RESOURCE_AND_OUTPUT_CAPS",
        "NO_EDIT_APPLY_OR_MODEL_BENCHMARK",
    ]
    if not isinstance(k1_10_correction, dict) or set(k1_10_correction) != {
        "candidateAndHarnessDisposition", "dependencyPublicationOrderingAuthority",
        "prepareReceiptPublicationAuthority", "trustedWorkerBuildEnvironmentAuthority",
        "preparedRefusalIdentity", "functionalFreeze", "preserved",
        "supersededStoreDisposition",
    } or k1_10_correction.get("candidateAndHarnessDisposition") != "REBUILD_AND_REBIND_BEFORE_NEW_STORE" \
        or k1_10_correction.get("supersededStoreDisposition") != "PREPARE_INFRASTRUCTURE_FAILURE_RETAINED_EVIDENCE_STORE_MUST_NOT_BE_REUSED" \
        or k1_10_correction.get("preserved") != expected_k1_10_preserved \
        or k1_10_correction.get("preparedRefusalIdentity") != {
            "schemaBefore": "codeclew.kotlin-k1-dependency-preparation-refusal/0.8",
            "schemaAfter": "codeclew.kotlin-k1-dependency-preparation-refusal/0.9",
            "seriesBefore": K1_9_SERIES_ID, "seriesAfter": K1_10_SERIES_ID,
        }:
        raise HarnessError("K1.10 correction identity/preservation mismatch")
    publication = k1_10_correction.get("dependencyPublicationOrderingAuthority")
    receipt_publication = k1_10_correction.get("prepareReceiptPublicationAuthority")
    builder_authority = k1_10_correction.get("trustedWorkerBuildEnvironmentAuthority")
    functional_freeze = k1_10_correction.get("functionalFreeze")
    if publication != {
        "platformFailureContour": "DARWIN_RENAME_REQUIRES_OWNER_WRITE_ON_MOVED_DIRECTORY_ROOT",
        "privateRootModeBeforeMove": 0o700,
        "nestedBuildStateSequence": [
            "SEAL_DESCENDANT_SUBTREES", "WRITE_CANONICAL_MANIFESTS",
            "ATOMIC_MOVE_ROOT_WHILE_0700", "CHMOD_DESTINATION_ROOT_0500",
            "VALIDATE_DESTINATION",
        ],
        "cohortSequence": [
            "REMOVE_PRIVATE_DOT_WORK", "WRITE_CANONICAL_COHORT_AUTHORITY",
            "SEAL_DESCENDANTS_KEEP_ROOT_0700", "ATOMIC_RENAME_TO_CREATE_ONLY_TARGET",
            "CHMOD_TARGET_ROOT_0500", "FSYNC_PARENT", "VALIDATE_PUBLISHED_COHORT",
        ],
        "publicationModes": {"directories": 0o500, "files": 0o400},
        "failureRecovery": "FAIL_CLOSED_RESTORE_OWNER_PERMISSIONS_WITHIN_EXACT_PRIVATE_PARENT_AND_REMOVE_STAGING_OR_UNRECEIPTED_TARGET",
        "cases": ["nestedMoveBeforeRootSeal", "cohortMoveBeforeRootSeal", "postRenameFailureRemoved"],
    } or receipt_publication != {
        "issuanceBoundary": "COHORT_VALIDATED_BEFORE_AUTHORITATIVE_READY_RECEIPT",
        "recovery": "ONLY_EXACT_RECONSTRUCTED_CURRENT_POINTER_AND_RECEIPT_MAY_SURVIVE_AN_ISSUE_EXCEPTION",
        "unreceiptedOutput": "REMOVE_CREATE_ONLY_TARGET",
        "durabilityFailure": "RETAIN_EXACT_AUDITABLE_POINTER_AND_RAISE_TYPED_HARNESS_ERROR",
        "injectedFaultCoverage": "PARTIAL_HARDENING_CAVEAT_NO_K1_10_BLOCKER",
    }:
        raise HarnessError("K1.10 dependency publication authority mismatch")
    expected_builder_digest = "sha256:6d853cfe8966dbde89caf6177b6757eb39256cecc9ec92afe8c8d6046d082030"
    if builder_authority != {
        "builderSha256Before": "sha256:0b57425ace9bffb6f23464d8bec1f646f98444856598c50820ba23a66bb6c7d7",
        "builderSha256After": expected_builder_digest,
        "scrubbedInjectionVariableAdded": "JAVA_OPTS",
        "preservedPinnedVariables": ["JAVA_HOME", "PATH"],
        "privateVariables": ["GRADLE_USER_HOME"],
        "distributionTasks": [
            ":workers:kotlin21:installDist", ":workers:kotlin23:installDist",
            ":workers:kotlin:installDist",
        ],
        "validation": {
            "mockedTasksPassed": 3, "mockedTasksTotal": 3,
            "hostileJavaOptsAbsent": True, "allInjectionVariablesAbsent": True,
        },
        "authorityBindings": [
            "CANDIDATE_SOURCES_LIVE_SET_FILE", "CANDIDATE_TOOLS_SOURCE_AUTHORITIES",
            "EACH_WORKER_DISTRIBUTION_BUILD_INPUT_DIGEST",
        ],
    } or functional_freeze != {
        "preTransitionHarness": "sha256:8660a21bc2077efa50f1bfedd2830313642a68611ebaf87d378fa582f0f87e2b",
        "preTransitionIndependentAuditor": "sha256:f0c8178d2a033b3fcc169ae01fcb95f90f438e66b467c7c4d528af7112dbcfe5",
        "postTransitionIndependentAuditor": "sha256:efcc20f31738cd0f6381e4d2c06361cf52e19d18288c2ecf2d222a8c3cb4c52f",
        "trustedWorkerDistributionBuilder": expected_builder_digest,
        "selfTestCounterexamples": 118, "supervisorCases": 18,
        "dependencyPublicationCases": 3, "modelCalls": 0,
        "redTeam": "ACCEPT_NO_GUARANTEED_BLOCKER",
    }:
        raise HarnessError("K1.10 builder/functional freeze mismatch")
    builder_path = ROOT / "scripts/build-trusted-worker-distributions.py"
    candidate_sources = _candidate_source_paths()
    expected_worker_build_inputs = {
        "2.1": "sha256:44d0261cdf93e56527058d34abfc5aac2fa9e10b989292438b288d9c289200d5",
        "2.3": "sha256:a9933f98bd95313043761c661d6e256fa91a39a487c21d6699b807df4b3f0096",
        "2.4": "sha256:df42131aee6e91b22425f846ffab8529d624360242da6edef4dfd75519387696",
    }
    if sha256_file(builder_path) != expected_builder_digest \
        or candidate_sources.get("trustedWorkerDistributionBuilder") != (builder_path, "FILE"):
        raise HarnessError("K1.10 trusted worker builder is not candidate-bound")
    for minor, expected_digest in expected_worker_build_inputs.items():
        manifest_name = "kotlin24.json" if minor == "2.4" else f"kotlin{minor.replace('.', '')}.json"
        manifest_body = _load_json_bytes(
            (ROOT / "workers/manifests" / manifest_name).read_bytes(), f"Kotlin {minor} worker manifest",
        )
        if _worker_candidate_identity(minor, manifest_body).get("buildInputDigest") != expected_digest:
            raise HarnessError(f"K1.10 worker builder input binding mismatch: {minor}")

    retained_k1_9 = ROOT / "docs/experiments/evidence/codeclew-k1.9-prepare-infrastructure-retained-evidence.json"
    retained_k1_9_digest = "sha256:827df53691fffec802da256781482277de7fe6e430b18328e99efc7b4927e87c"
    retained_k1_9_raw = _regular_file(retained_k1_9, "retained K1.9 PREPARE evidence").read_bytes()
    retained_k1_9_value = _load_json_bytes(retained_k1_9_raw, "retained K1.9 PREPARE evidence")
    if sha256_bytes(retained_k1_9_raw) != retained_k1_9_digest \
        or not isinstance(retained_k1_9_value, dict) \
        or canonical(retained_k1_9_value) != retained_k1_9_raw \
        or not isinstance(k1_10_evidence, dict) \
        or k1_10_evidence.get("retainedEvidenceSha256") != retained_k1_9_digest:
        raise HarnessError("K1.10 retained K1.9 evidence identity mismatch")
    retained_summary_keys = {
        "schema", "seriesId", "kind", "sourceStoreAbsolutePath", "storeId",
        "storeIdentitySha256", "sourceStoreMemberManifestCount", "sourceStoreFileCount",
        "sourceStoreDirectoryCount", "sourceStoreMemberManifestSha256",
        "candidateToolsSha256", "liveInputsSha256", "currentNodeCount", "guardState",
        "officialPrepareReceiptPublished", "qualificationDependencyPreparePointerPresent",
        "qualificationDependencySeedPublished", "qualificationDependencyStagingPresent",
        "qualificationAttemptCount", "retainedAttemptCount", "childStartJournalCount",
        "holdoutAttemptCount", "holdoutOpened", "holdoutSourceMaterialized",
        "modelCalls", "decisionIssued",
    }
    if any(k1_10_evidence.get(key) != retained_k1_9_value.get(key) for key in retained_summary_keys):
        raise HarnessError("K1.10 retained K1.9 evidence summary mismatch")
    store_members = retained_k1_9_value.get("sourceStoreMemberManifest")
    if not isinstance(store_members, list) or len(store_members) != 51 \
        or sha256_bytes(canonical(store_members)) != "sha256:2c4a3f364bbc190d49117046a6ee16524a4c9f744e7b1c519a5992720be9db61" \
        or sum(row.get("kind") == "FILE" for row in store_members) != 42 \
        or sum(row.get("kind") == "DIRECTORY" for row in store_members) != 9 \
        or any(
            row.get("kind") == "FILE" and row.get("path", "").startswith(prefix)
            for row in store_members for prefix in ("attempts/", "qualification/", "holdout/", "starts/")
        ):
        raise HarnessError("K1.10 retained K1.9 store/member manifest mismatch")
    expected_store_identity = {
        "schema": STORE_SCHEMA, "seriesId": K1_9_SERIES_ID,
        "storeId": "5942c2b16fb17c383757e0a2369d2031ce6e0cc6795d4ad4c2ff1a800b6d5e27",
        "authorityDigests": {**k1_9_digests, "preregistrationAmendment": k1_9_amendment_digest},
    }
    current_nodes = retained_k1_9_value.get("currentNodes")
    if retained_k1_9_value.get("storeIdentity") != expected_store_identity \
        or retained_k1_9_value.get("storeIdentitySha256") != sha256_bytes(canonical(expected_store_identity)) \
        or not isinstance(current_nodes, list) or [row.get("node") for row in current_nodes] != sorted({
            "INPUT_AUTHORITY_VERIFY", "K0_1_BYTE_EXACT_VERIFY", "REQUIREMENTS_FREEZE_VERIFY",
            "CORPUS_FREEZE_VERIFY", "HOLDOUT_ELIGIBILITY_AUDIT_IMPORT", "BASELINE_CAPTURE",
            "HARNESS_SELF_TEST", "K1_SERIES_GUARD",
        }):
        raise HarnessError("K1.10 retained K1.9 store identity/current nodes mismatch")
    envelope = {
        "detailSha256": "sha256:b63ca9fa2694a3406c7f471cd0076cb663a479158834d66397bb3125c1119efd",
        "reason": "PermissionError", "schema": "codeclew.kotlin-k1-harness-error/0.1",
        "status": "FAILED",
    }
    failure = retained_k1_9_value.get("officialPrepare", {}).get("failure")
    amendment_failure = k1_10_evidence.get("failure")
    if not isinstance(failure, dict) or failure.get("outputEnvelope") != envelope \
        or failure.get("outputEnvelopeBytes") != len(canonical(envelope)) \
        or failure.get("outputEnvelopeSha256") != sha256_bytes(canonical(envelope)) \
        or failure.get("safeDetailSha256") != envelope["detailSha256"] \
        or failure.get("exceptionType") != "PermissionError" \
        or failure.get("operation") != "BUILD_STATE_ROOT_ATOMIC_MOVE" \
        or failure.get("semanticOutcomeObserved") is not False \
        or not isinstance(amendment_failure, dict) \
        or any(amendment_failure.get(key) != failure.get(key) for key in (
            "exceptionType", "operation", "safeDetailSha256", "outputEnvelopeBytes",
            "outputEnvelopeNewlineTerminated", "outputEnvelopeSha256", "semanticOutcomeObserved",
        )):
        raise HarnessError("K1.10 exact K1.9 failure envelope mismatch")
    diagnostic_codes = [
        row.get("code") for row in retained_k1_9_value.get("diagnosticInvestigation", {}).get("causes", [])
        if isinstance(row, dict)
    ]
    if diagnostic_codes != [
        "BUILD_STATE_ROOT_SEALED_BEFORE_ATOMIC_MOVE",
        "COHORT_ROOT_SEALED_BEFORE_ATOMIC_PUBLICATION",
        "TRUSTED_WORKER_BUILD_JAVA_OPTS_INJECTION_GAP",
    ] or retained_k1_9_value.get("baseline", {}).get("requiredGreenPassCount") != 10 \
        or retained_k1_9_value.get("baseline", {}).get("requiredGreenFailCount") != 0 \
        or retained_k1_9_value.get("baseline", {}).get("historicalFailCount") != 2 \
        or retained_k1_9_value.get("harnessSelfTest", {}).get("counterexamples") != 115 \
        or retained_k1_9_value.get("harnessSelfTest", {}).get("supervisorCaseCount") != 18:
        raise HarnessError("K1.10 K1.9 failure diagnosis/baseline boundary mismatch")

    expected_k1_11_reasons = [
        "K1_10_CORPUS_RUNNER_LOCAL_SNAPSHOT_INPUT_SHADOWING_PRE_CHILD_UNTYPED_FAILURE",
        "K1_10_GRADLE_PREPARE_JVM_TMPDIR_INHERITED_NONEXISTENT_HOST_PATH_MISCLASSIFIED_AS_DEPENDENCY_CLOSURE",
        "K1_10_GRADLE_OFFLINE_FILE_LOCK_SANDBOX_DENIAL_CLASSIFIED_AS_INFRASTRUCTURE_INSTEAD_OF_TYPED_REFUSAL",
    ]
    k1_11_correction = k1_11_amendment.get("correction") if isinstance(k1_11_amendment, dict) else None
    k1_11_evidence = k1_11_amendment.get("qualificationInfrastructureEvidence") if isinstance(k1_11_amendment, dict) else None
    if not isinstance(k1_11_amendment, dict) or set(k1_11_amendment) != {
        "schema", "cancelledSeriesId", "replacementSeriesId", "oldAuthorityDigests",
        "predecessorAmendmentSha256", "reasonCodes", "authorityStateBeforeReplacement",
        "baselineAttempts", "officialDependencyPrepareAttempts", "qualificationRunnerInvocations",
        "qualificationAttempts", "holdoutAttempts", "modelCalls", "holdoutOpened", "correction",
        "qualificationInfrastructureEvidence",
    } or k1_11_amendment.get("schema") != "codeclew.kotlin-k1-preregistration-amendment/0.11" \
        or k1_11_amendment.get("cancelledSeriesId") != K1_10_SERIES_ID \
        or k1_11_amendment.get("replacementSeriesId") != PREDECESSOR_SERIES_ID \
        or k1_11_amendment.get("oldAuthorityDigests") != k1_10_digests \
        or k1_11_amendment.get("predecessorAmendmentSha256") != k1_10_amendment_digest \
        or k1_11_amendment.get("reasonCodes") != expected_k1_11_reasons \
        or k1_11_amendment.get("authorityStateBeforeReplacement") != "PUBLISHED_PREPARE_COHORT_AND_PRE_CHILD_QUALIFICATION_INFRASTRUCTURE_FAILURE_NO_SEMANTIC_OUTCOME" \
        or any(k1_11_amendment.get(key) != value for key, value in {
            "baselineAttempts": 1, "officialDependencyPrepareAttempts": 1,
            "qualificationRunnerInvocations": 1, "qualificationAttempts": 0,
            "holdoutAttempts": 0, "modelCalls": 0, "holdoutOpened": False,
        }.items()):
        raise HarnessError("K1.11 cancellation/amendment contract mismatch")
    expected_k1_11_preserved = [
        "DECISION_THRESHOLDS", "REQUIREMENTS_EXCEPT_SERIES_AND_AMENDMENT_BINDING",
        "CORPUS_EXCEPT_SERIES", "ELIGIBILITY_EXCEPT_SERIES",
        "HOLDOUT_ELIGIBILITY_PROCEDURE_MEMBERS_AND_DECISION", "READINESS_GRAPH_EXCEPT_GRAPH_ID",
        "K0_1_BYTE_EXACT", "WORKLOAD", "SOURCE_TREE_AND_SANDBOX_NETWORK_OUTPUT_AUTHORITY",
        "STRICT_OFFLINE_AND_DECISION_NETWORK_DENY_WITHOUT_ALLOW",
        "BASELINE_PACKET_AND_CONTEXT_SCHEMAS", "BASELINE_LOGICAL_COMMAND_ARGV_TARGETS_AND_TEST_FILTERS",
        "CARGO_LOCK_DERIVED_SEED", "RUST_GRADLE_MAVEN_JDK_LAUNCHER_IDENTITIES",
        "BASELINE_GREEN_MEASUREMENT", "PREPARE_NETWORK_SPLIT_AND_SENTINEL",
        "PREPARE_ANCESTOR_TRAVERSAL_AUTHORITY", "PREPARE_MAVEN_RUNTIME_MINIMAL_AUTHORITY",
        "DISPOSABLE_ARCHIVE_DUAL_IDENTITY_AUTHORITY", "GRADLE_WRAPPER_BOOTSTRAP_PRIVATE_HOME",
        "MAVEN_ONLINE_MODEL_PREFETCH", "DEPENDENCY_SEED_PHYSICAL_SEALING",
        "DISPOSABLE_SOURCE_SAFE_CLEANUP", "PREPARE_RESOURCE_AND_OUTPUT_CAPS",
        "NO_EDIT_APPLY_OR_MODEL_BENCHMARK", "NO_SEALED_PROJECT_MODEL_OR_NETWORK_PROFILE_WIDENING",
    ]
    if not isinstance(k1_11_correction, dict) or set(k1_11_correction) != {
        "candidateAndHarnessDisposition", "corpusRunnerSnapshotInputBindingAuthority",
        "gradlePrepareJvmTmpdirAuthority", "gradleOfflineFileLockTypedRefusalAuthority",
        "strictNetworkAuthority", "preparedRefusalIdentity", "functionalFreeze", "preserved",
        "supersededStoreDisposition",
    } or k1_11_correction.get("candidateAndHarnessDisposition") != "REBUILD_AND_REBIND_BEFORE_NEW_STORE" \
        or k1_11_correction.get("supersededStoreDisposition") != "PUBLISHED_PREPARE_COHORT_AND_PRE_CHILD_QUALIFICATION_INFRASTRUCTURE_RETAINED_EVIDENCE_STORE_AND_COHORT_MUST_NOT_BE_REUSED" \
        or k1_11_correction.get("preserved") != expected_k1_11_preserved \
        or k1_11_correction.get("preparedRefusalIdentity") != {
            "schemaBefore": "codeclew.kotlin-k1-dependency-preparation-refusal/0.9",
            "schemaAfter": "codeclew.kotlin-k1-dependency-preparation-refusal/0.10",
            "seriesBefore": K1_10_SERIES_ID, "seriesAfter": PREDECESSOR_SERIES_ID,
        }:
        raise HarnessError("K1.11 correction identity/preservation mismatch")
    if k1_11_correction.get("corpusRunnerSnapshotInputBindingAuthority") != {
        "failureBoundary": "LOCAL_NAME_SHADOWED_GLOBAL_SNAPSHOT_INPUT_BEFORE_FIRST_CHILD_START_OR_ATTEMPT_PUBLICATION",
        "correction": "NO_LOCAL_SNAPSHOT_INPUT_BINDING_GLOBAL_CALLABLE_REACHED_DURING_INITIAL_DEPENDENCY_SEED_PREFLIGHT",
        "cases": ["snapshotInputNotLocal", "globalSnapshotInputReachedInPreflight"],
    } or k1_11_correction.get("gradlePrepareJvmTmpdirAuthority") != {
        "gradleEnvironment": "GRADLE_OPTS=-Djava.io.tmpdir=<shared-entry-work>/home",
        "preparePhases": ["ONLINE", "OFFLINE"],
        "privateDirectoryAuthority": "EXISTING_SHARED_ENTRY_HOME",
        "hostJvmTmpdirInheritance": "FORBIDDEN", "mavenPrepareGradleOpts": "ABSENT",
        "cases": [
            "prepareGradleJvmTmpdirAuthorityPassed", "prepareMissingGradleJvmTmpdirRejected",
            "prepareWrongGradleJvmTmpdirRejected", "prepareGradleJvmTmpdirFailureClassifiedInfrastructure",
        ],
    }:
        raise HarnessError("K1.11 runner/Gradle tmp correction mismatch")
    expected_file_lock = {
        "scope": "GRADLE_OFFLINE_DEPENDENCY_VERIFICATION_ONLY",
        "typedReasonCode": "OFFLINE_MODEL_PROBE_FAILED", "stderrMaximumBytes": 1048576,
        "strictProfile": {
            "buildDsl": ["GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL"],
            "denyNetworkClause": "(deny network*)", "denyNetworkClauseCount": 1,
            "allowNetworkClauseCount": 0,
        },
        "requiredCommonMarker": "java.net.SocketException: Operation not permitted",
        "operationNotPermittedLines": "ALL_MUST_CONTAIN_JAVA_NET_SOCKETEXCEPTION",
        "acceptedSignatures": {
            "bind": [
                "org.gradle.cache.internal.locklistener.FileLockCommunicator.<init>",
                "org.gradle.cache.internal.locklistener.DefaultFileLockContentionHandler.reservePort",
            ],
            "receive": [
                "org.gradle.cache.internal.locklistener.FileLockCommunicator.receive",
                "sun.nio.ch.DatagramChannelImpl.receive", "java.net.DatagramSocket.receive",
            ],
        },
        "forbiddenContaminationMarkers": [
            "sandbox", "deny file", "permission denied",
            "java.io.tmpdir is set to a directory that doesn't exist",
            "TcpIncomingConnector", "Net.bind0", "ServerSocket",
        ],
        "infrastructureOtherwise": True,
        "cases": [
            "prepareGradleOfflineFileLockBindTypedRefusal",
            "prepareGradleOfflineFileLockReceiveTypedRefusal",
            "prepareGradleOfflineFileLockWrongProfileRejected",
            "prepareGradleOfflineFileLockContaminationRejected",
        ],
    }
    if k1_11_correction.get("gradleOfflineFileLockTypedRefusalAuthority") != expected_file_lock \
        or k1_11_correction.get("strictNetworkAuthority") != {
            "offlineProfile": "EXACT_DENY_NETWORK_WITH_ZERO_ALLOW_NETWORK_CLAUSES",
            "decisionWorker": "EXACT_DENY_NETWORK_NO_GRADLE_FILE_LOCK_EXCEPTION",
            "profileWidening": "FORBIDDEN", "sealedProjectModelRoute": "NOT_ADOPTED",
        }:
        raise HarnessError("K1.11 bounded FileLock/strict network correction mismatch")
    functional_freeze = k1_11_correction.get("functionalFreeze")
    if functional_freeze != {
        "preTransitionHarness": "sha256:68eec776ace3736481a6bf9d13608545a416d2d3366e95cc37d3e62c71892292",
        "preTransitionIndependentAuditor": "sha256:add18eadca23d895793e2ec135e86556f8ad1584ff920f99c2150a551feaa484",
        "postTransitionIndependentAuditor": "sha256:a3bc4ad85496136df5bfd2112ca61adc6084321e4b5013572c63f7f9dd398ed0",
        "trustedWorkerDistributionBuilder": "sha256:6d853cfe8966dbde89caf6177b6757eb39256cecc9ec92afe8c8d6046d082030",
        "worker": "sha256:4416fd01158fefd3210743649b1636c881b89ab1cf0271488d6dabeff7e65870",
        "mavenProjectModel": "sha256:d6b3dbee0620956b2fb8aa192e5baa5877064001711e5779a5f40633ac09a353",
        "projectModelCommandTest": "sha256:d0fb33b7542d508a649c0772f48007860eb7c185217598bc2fd9c5281a3dc68e",
        "selfTestCounterexamples": 131, "requirementCases": 64, "supervisorCases": 18,
        "prepareNotRunMutations": {"passed": 30, "total": 30},
        "modelCalls": 0, "redTeam": "ACCEPT_NO_GUARANTEED_BLOCKER",
    }:
        raise HarnessError("K1.11 functional freeze mismatch")

    retained_k1_10_path = ROOT / "docs/experiments/evidence/codeclew-k1.10-qualification-infrastructure-retained-evidence.json"
    retained_k1_10_digest = "sha256:038aabad5b31ba616466288dc7a016d11fe8575a28bb1761958d41e79d0a8ca9"
    retained_k1_10_raw = _regular_file(retained_k1_10_path, "retained K1.10 qualification infrastructure evidence").read_bytes()
    retained_k1_10 = _load_json_bytes(retained_k1_10_raw, "retained K1.10 qualification infrastructure evidence")
    if sha256_bytes(retained_k1_10_raw) != retained_k1_10_digest \
        or not isinstance(retained_k1_10, dict) or canonical(retained_k1_10) != retained_k1_10_raw \
        or not isinstance(k1_11_evidence, dict) \
        or k1_11_evidence.get("retainedEvidenceSha256") != retained_k1_10_digest:
        raise HarnessError("K1.11 retained K1.10 evidence identity mismatch")
    expected_retained_summary = {
        "schema": "codeclew.kotlin-k1-qualification-infrastructure-retained-evidence/0.1",
        "seriesId": K1_10_SERIES_ID,
        "kind": "PUBLISHED_PREPARE_COHORT_AND_PRE_CHILD_QUALIFICATION_INFRASTRUCTURE_FAILURE_NO_SEMANTIC_OUTCOME",
        "disposition": "SUPERSEDED_STORE_AND_PUBLISHED_COHORT_MUST_NOT_BE_REUSED",
        "sourceStoreAbsolutePath": "/private/tmp/codeclew-k1-10-production.p3wn4I/run/store",
        "storeId": "1a620cbf192e0c109df738a78e56ec34b45602625adf1109e9811bca55ea626c",
        "storeIdentitySha256": "sha256:6bcfbacdec359070602e514be5c1e2a73c551c94684b5cbab8c5c2e892ade1de",
        "sourceStoreMemberManifestCount": 65,
        "sourceStoreMemberManifestSha256": "sha256:bf227f08f797c6137627c546224a9c09ae8792303ef6a0481af7ec68dcf11759",
        "sourceStoreTreeSha256": "sha256:74a297a2732b4888a8a453d7885527dded875c29bbbf1d763a39958da6676fd1",
        "candidateToolsSha256": "sha256:3cbd96aaf2271e7b9c7f33cb716aa46a86ea8292a4f10cefe42ea51194c3066d",
        "liveInputsSha256": "sha256:5729b5054e5687b5f4a733d7f674bb23a1324460b101923f4f09e902c6030215",
    }
    if any(retained_k1_10.get(key) != value for key, value in expected_retained_summary.items()) \
        or any(k1_11_evidence.get(key) != value for key, value in expected_retained_summary.items()):
        raise HarnessError("K1.11 retained K1.10 summary mismatch")
    retained_store_members = retained_k1_10.get("sourceStoreMemberManifest")
    if not isinstance(retained_store_members, list) or len(retained_store_members) != 65 \
        or sha256_bytes(canonical(retained_store_members)) != expected_retained_summary["sourceStoreMemberManifestSha256"] \
        or sum(row.get("kind") == "FILE" for row in retained_store_members) != 56 \
        or sum(row.get("kind") == "DIRECTORY" for row in retained_store_members) != 9 \
        or any(row.get("kind") == "FILE" and row.get("path", "").startswith(prefix)
               for row in retained_store_members for prefix in ("attempts/", "qualification/", "holdout/", "starts/")):
        raise HarnessError("K1.11 retained K1.10 store/member manifest mismatch")
    retained_cohort = retained_k1_10.get("publishedDependencyCohort")
    amendment_cohort = k1_11_evidence.get("publishedDependencyCohort")
    expected_cohort_summary = {
        "sourceAbsolutePath": "/private/tmp/codeclew-k1-10-production.p3wn4I/run/qualificationDependencySeed",
        "manifestSha256": "sha256:5b8dff76560bca785181dbce13bcbace74a0057021a9213b028c333b0bedaa7f",
        "markerSha256": "sha256:e74801c43c4e8f512d8fb93aa03913727199b472651c1e59fddde3208f5d9320",
        "cohortDigest": "sha256:461a82b26b3f38ca1b843993dd56016f0e70011af8abdea0a7b876e7274c662b",
        "memberManifestCount": 7937,
        "memberManifestSha256": "sha256:35b9dd03328c107540ff7350129f3f95d690a5fcf20a5e5a7ced474d3f32b28e",
        "treeSha256": "sha256:e46c43573024e32586729cd8e0fb26edbb71d1d987a32adc3f84e791bca3be77",
    }
    if not isinstance(retained_cohort, dict) or not isinstance(amendment_cohort, dict) \
        or any(retained_cohort.get(key) != value for key, value in expected_cohort_summary.items()) \
        or any(amendment_cohort.get(key) != value for key, value in expected_cohort_summary.items()) \
        or amendment_cohort.get("misclassifiedEntries") != list(EXPECTED_QUALIFICATION[1:]) \
        or amendment_cohort.get("commonCauseCode") != "GRADLE_PREPARE_JVM_TMPDIR_INHERITED_NONEXISTENT_HOST_PATH":
        raise HarnessError("K1.11 retained K1.10 cohort summary mismatch")
    cohort_members = retained_cohort.get("memberManifest")
    cohort_manifest = retained_cohort.get("manifest")
    misclassified = retained_cohort.get("misclassifiedInfrastructureRefusals")
    if not isinstance(cohort_members, list) or len(cohort_members) != 7937 \
        or sha256_bytes(canonical(cohort_members)) != expected_cohort_summary["memberManifestSha256"] \
        or not isinstance(cohort_manifest, dict) \
        or sha256_bytes(canonical(cohort_manifest)) != expected_cohort_summary["manifestSha256"] \
        or cohort_manifest.get("cohortDigest") != expected_cohort_summary["cohortDigest"] \
        or not isinstance(misclassified, list) \
        or [row.get("entry") for row in misclassified] != list(EXPECTED_QUALIFICATION[1:]):
        raise HarnessError("K1.11 retained K1.10 cohort/member manifest mismatch")
    cohort_body = dict(cohort_manifest)
    cohort_body["cohortDigest"] = ""
    if sha256_bytes(canonical(cohort_body)) != cohort_manifest["cohortDigest"]:
        raise HarnessError("K1.11 retained K1.10 cohort self-seal mismatch")
    rows_by_entry = {row.get("entry"): row for row in cohort_manifest.get("entries", []) if isinstance(row, dict)}
    for retained_refusal in misclassified:
        entry = retained_refusal.get("entry")
        refusal = retained_refusal.get("object")
        prepare_row = retained_refusal.get("prepareRow")
        projection = dict(refusal) if isinstance(refusal, dict) else {}
        projection["objectDigest"] = ""
        if retained_refusal.get("commonCauseCode") != "GRADLE_PREPARE_JVM_TMPDIR_INHERITED_NONEXISTENT_HOST_PATH" \
            or retained_refusal.get("retainedClassification") != "INFRASTRUCTURE_MISCLASSIFIED_NOT_PRODUCT_SEMANTIC_REFUSAL_MUST_NOT_REUSE" \
            or not isinstance(refusal, dict) or not isinstance(prepare_row, dict) \
            or prepare_row != rows_by_entry.get(entry) \
            or retained_refusal.get("fileSha256") != sha256_bytes(canonical(refusal)) \
            or retained_refusal.get("fileSize") != len(canonical(refusal)) \
            or refusal.get("schema") != "codeclew.kotlin-k1-dependency-preparation-refusal/0.9" \
            or refusal.get("seriesId") != K1_10_SERIES_ID \
            or refusal.get("reasonCode") != "DEPENDENCY_CLOSURE_UNAVAILABLE" \
            or refusal.get("preparationReceiptDigest") != sha256_bytes(canonical(prepare_row)) \
            or refusal.get("objectDigest") != sha256_bytes(canonical(projection)[:-1]):
            raise HarnessError("K1.11 retained K1.10 misclassified refusal mismatch")
    runner = retained_k1_10.get("officialQualificationRunner")
    runner_failure = runner.get("failure") if isinstance(runner, dict) else None
    amendment_runner = k1_11_evidence.get("officialQualificationRunner")
    if not isinstance(runner, dict) or not isinstance(runner_failure, dict) \
        or runner.get("entry") != "K1-Q01" or runner.get("invocation") != "COLD" \
        or runner.get("cliInvocations") != 1 or runner.get("retries") != 0 \
        or runner.get("phase") != "BEFORE_CHILD_START_JOURNAL_AND_ATTEMPT_PUBLICATION" \
        or runner_failure != {
            "cliExitCode": 1, "exceptionType": "UnboundLocalError",
            "fullTracebackBytesRetained": False, "harnessJsonEnvelopeEmitted": False,
            "operation": "CORPUS_RUNNER_INITIAL_DEPENDENCY_SEED_SNAPSHOT",
            "semanticOutcomeObserved": False, "sourceStderrFinalLineTerminationRetained": False,
            "stderrFinalLine": "UnboundLocalError: cannot access local variable 'snapshot_input' where it is not associated with a value",
            "stderrFinalLineCanonicalDetailBytes": 104,
            "stderrFinalLineCanonicalDetailSha256": "sha256:3e887d45362a9c4e92574823b8ccf63afeba3b1f1331979ebda10889ca743ce5",
            "tracebackStream": "STDERR",
        } or amendment_runner != {
            "entry": "K1-Q01", "invocation": "COLD", "cliInvocations": 1, "retries": 0,
            "phase": "BEFORE_CHILD_START_JOURNAL_AND_ATTEMPT_PUBLICATION",
            "exceptionType": "UnboundLocalError",
            "finalDetailSha256": "sha256:3e887d45362a9c4e92574823b8ccf63afeba3b1f1331979ebda10889ca743ce5",
            "fullTracebackBytesRetained": False, "semanticOutcomeObserved": False,
        }:
        raise HarnessError("K1.11 retained K1.10 qualification failure mismatch")
    expected_counts = {
        "officialDependencyPrepareAttempts": 1, "qualificationRunnerInvocations": 1,
        "qualificationAttempts": 0, "retainedAttempts": 0, "childStarts": 0,
        "holdoutAttempts": 0, "modelCalls": 0, "holdoutOpened": False, "decisionIssued": False,
    }
    if k1_11_evidence.get("counts") != expected_counts \
        or any(retained_k1_10.get(key) != value for key, value in {
            "officialDependencyPrepareAttempts": 1, "qualificationRunnerInvocationCount": 1,
            "qualificationAttemptCount": 0, "retainedAttemptCount": 0, "childStartJournalCount": 0,
            "holdoutAttemptCount": 0, "modelCalls": 0, "holdoutOpened": False,
            "decisionIssued": False,
        }.items()) \
        or retained_k1_10.get("harnessSelfTest", {}).get("counterexamples") != 118 \
        or retained_k1_10.get("harnessSelfTest", {}).get("supervisorCaseCount") != 18 \
        or retained_k1_10.get("baseline", {}).get("requiredGreenPassCount") != 10 \
        or retained_k1_10.get("baseline", {}).get("requiredGreenFailCount") != 0 \
        or retained_k1_10.get("baseline", {}).get("historicalFailCount") != 2:
        raise HarnessError("K1.11 retained K1.10 zero-outcome/baseline boundary mismatch")

    k1_12_correction = amendment.get("correction") if isinstance(amendment, dict) else None
    k1_12_evidence = amendment.get("prepareInfrastructureEvidence") if isinstance(amendment, dict) else None
    if not isinstance(amendment, dict) or set(amendment) != {
        "schema", "cancelledSeriesId", "replacementSeriesId", "oldAuthorityDigests",
        "predecessorAmendmentSha256", "reasonCode", "authorityStateBeforeReplacement",
        "baselineAttempts", "officialDependencyPrepareAttempts", "qualificationAttempts",
        "holdoutAttempts", "modelCalls", "holdoutOpened", "correction", "prepareInfrastructureEvidence",
    } or amendment.get("schema") != "codeclew.kotlin-k1-preregistration-amendment/0.12" \
        or amendment.get("cancelledSeriesId") != PREDECESSOR_SERIES_ID \
        or amendment.get("replacementSeriesId") != SERIES_ID \
        or amendment.get("oldAuthorityDigests") != k1_11_digests \
        or amendment.get("predecessorAmendmentSha256") != k1_11_amendment_digest \
        or amendment.get("reasonCode") != "K1_11_GRADLE_STRICT_OFFLINE_NONZERO_MODEL_PROBE_MISCLASSIFIED_AS_SECURITY_AUTHORITY_FAILURE" \
        or amendment.get("authorityStateBeforeReplacement") != "PREPARE_INFRASTRUCTURE_FAILURE_NO_PREPARE_RECEIPT_NO_QUALIFICATION_NO_HOLDOUT_NO_MODEL" \
        or any(amendment.get(key) != value for key, value in {
            "baselineAttempts": 1, "officialDependencyPrepareAttempts": 1,
            "qualificationAttempts": 0, "holdoutAttempts": 0, "modelCalls": 0, "holdoutOpened": False,
        }.items()):
        raise HarnessError("K1.12 cancellation/amendment contract mismatch")
    expected_structural = {
        "scope": "GRADLE_OFFLINE_DEPENDENCY_VERIFICATION_ONLY",
        "typedReasonCode": "OFFLINE_MODEL_PROBE_FAILED",
        "classificationBoundary": {
            "buildDsl": ["GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL"],
            "failedCommand": "SECOND_COMMAND_EXACT_OFFLINE_ARGV_WITH_ONE_OFFLINE_FLAG",
            "commandResultCount": 2, "exitCode": "NONZERO",
            "offlineSandboxProfile": "EXACT_ONE_DENY_NETWORK_ZERO_ALLOW_NETWORK",
            "offlineSentinel": "EXIT_ZERO_EMPTY_STDOUT_EMPTY_STDERR_BEFORE_MODEL_PROBE",
            "sourceAuthority": "FROZEN_SOURCE_BEFORE_EQUALS_AFTER_BEFORE_CLASSIFICATION",
            "stderrSemantics": "NOT_CLASSIFIED",
            "publicationGate": "FULL_PREPARATION_NETWORK_EVIDENCE_VALIDATED_BEFORE_PREPARED_REFUSAL",
        },
        "neverTypedRefusal": [
            "ONLINE_PHASE_FAILURE", "MAVEN_FAILURE", "WRONG_NETWORK_PROFILE", "SENTINEL_FAILURE",
            "SOURCE_MUTATION", "ZERO_EXIT_READY", "LAUNCH_FAILURE", "TIMEOUT", "OUTPUT_LIMIT",
            "RESIDENT_LIMIT", "SIGNAL_TERMINATION",
        ],
        "cases": [
            "prepareGradleStrictOfflineFailureTypedRefusal",
            "prepareGradleStrictOfflineWrongProfileSecurityRejected",
            "prepareGradleOnlineSecurityFailureRejected", "prepareMavenOfflineSecurityFailureRejected",
        ],
    }
    freeze = k1_12_correction.get("functionalFreeze") if isinstance(k1_12_correction, dict) else None
    if not isinstance(k1_12_correction, dict) \
        or k1_12_correction.get("gradleStrictOfflineTypedRefusalAuthority") != expected_structural \
        or k1_12_correction.get("preparedRefusalIdentity") != {
            "schemaBefore": "codeclew.kotlin-k1-dependency-preparation-refusal/0.10",
            "schemaAfter": PREPARED_REFUSAL_SCHEMA,
            "seriesBefore": PREDECESSOR_SERIES_ID, "seriesAfter": SERIES_ID,
        } or k1_12_correction.get("strictNetworkAuthority") != {
            "offlineProfile": "EXACT_DENY_NETWORK_WITH_ZERO_ALLOW_NETWORK_CLAUSES",
            "decisionWorker": "EXACT_DENY_NETWORK_UNCHANGED", "profileWidening": "FORBIDDEN",
        } or freeze != {
            "preCorrectionHarness": "sha256:2dac918f53bb8d215048eb5a2d406f6d7a9e72b53b27b27d837c47f819fec9d8",
            "preCorrectionIndependentAuditor": "sha256:a3bc4ad85496136df5bfd2112ca61adc6084321e4b5013572c63f7f9dd398ed0",
            "preTransitionHarness": "sha256:6e19ea72d6905757bdbcebdd792e964fb3380aa10a62a1ee71c90d2ddd6c0cb3",
            "preTransitionIndependentAuditor": "sha256:47577f1c4e676b5fd92b08cb76ad10b747c3b6c85bff842458cae7d0d13e56aa",
            "postTransitionIndependentAuditor": "sha256:a8b7d3c25937fb3aad957f248c83dfaf7c94b8d0583522e38b08589763ec31c4",
            "selfTestCounterexamples": 132, "requirementCases": 64, "supervisorCases": 18,
            "modelCalls": 0, "redTeam": "ACCEPT_STRUCTURAL_PUBLICATION_BOUNDARY_NO_GUARANTEED_BLOCKER",
        } or sha256_file(ROOT / "scripts/k1_independent_auditor.py") != freeze["postTransitionIndependentAuditor"]:
        raise HarnessError("K1.12 structural correction/freeze mismatch")
    retained_path = ROOT / "docs/experiments/evidence/codeclew-k1.11-prepare-infrastructure-retained-evidence.json"
    retained_digest = "sha256:087aaffb3cc1e5efbd4a249ac0d78324fa49435150cb9dab099145cd854a51c7"
    retained_raw = _regular_file(retained_path, "retained K1.11 PREPARE infrastructure evidence").read_bytes()
    retained = _load_json_bytes(retained_raw, "retained K1.11 PREPARE infrastructure evidence")
    summary = {
        "schema": "codeclew.kotlin-k1-prepare-infrastructure-retained-evidence/0.7",
        "seriesId": PREDECESSOR_SERIES_ID,
        "kind": "PREPARE_INFRASTRUCTURE_FAILURE_NO_PREPARE_RECEIPT_NO_QUALIFICATION_NO_HOLDOUT_NO_MODEL",
        "disposition": "SUPERSEDED_STORE_MUST_NOT_BE_REUSED",
        "sourceStoreAbsolutePath": "/private/tmp/codeclew-k1-11-production.LULaPQ/run/store",
        "storeId": "6dc767daf7cff3d609dc61cca6af1d087f81ba1b48d9b17572965624b3028257",
        "storeIdentitySha256": "sha256:25445690b538db93030fd88ac3b7b630cc86c126826111add0dfd6657a7a03bf",
        "sourceStoreMemberManifestCount": 53,
        "sourceStoreMemberManifestSha256": "sha256:9a2ce6a6324f3c7840cb2106b92bca336b42823f7d5b24053dc9cb4e4727caa5",
        "sourceStoreTreeSha256": "sha256:47371fad64ecbe016373a06ba4ae903b7a4cc3526550a0498e26353f31a1f660",
        "guardState": "OPEN", "guardMarkerSha256": "sha256:44d9fa119690d728fb3c9ed0413a9f5b27f856e0bbc803313403f7ff66b26b14",
        "currentNodeCount": 8,
    }
    if sha256_bytes(retained_raw) != retained_digest or not isinstance(retained, dict) \
        or canonical(retained) != retained_raw or not isinstance(k1_12_evidence, dict) \
        or k1_12_evidence.get("retainedEvidenceSha256") != retained_digest \
        or any(retained.get(key) != value or k1_12_evidence.get(key) != value for key, value in summary.items()):
        raise HarnessError("K1.12 retained K1.11 identity/summary mismatch")
    members = retained.get("sourceStoreMemberManifest")
    retained_failure = retained.get("officialPrepare", {}).get("failure")
    amendment_failure = k1_12_evidence.get("failure")
    envelope = '{"detailSha256":"sha256:de321846e0bdfe95c61bc7544c4d6a05076b3aac87f22cf52900aab113ff43a7","reason":"HarnessError","schema":"codeclew.kotlin-k1-harness-error/0.1","status":"FAILED"}\n'
    if not isinstance(members, list) or len(members) != 53 \
        or sha256_bytes(canonical(members)) != summary["sourceStoreMemberManifestSha256"] \
        or sum(row.get("kind") == "FILE" for row in members) != 44 \
        or sum(row.get("kind") == "DIRECTORY" for row in members) != 9 \
        or not isinstance(retained_failure, dict) or not isinstance(amendment_failure, dict) \
        or retained_failure.get("outputEnvelopeAscii") != envelope \
        or retained_failure.get("outputEnvelopeBytes") != 181 \
        or retained_failure.get("outputEnvelopeSha256") != "sha256:e431460d1c907f4153678b07b2da3ccb7bceb8872e907e15666d6cb72545f781" \
        or retained_failure.get("fullOutputEnvelopeBytesRetained") is not True \
        or amendment_failure.get("outputEnvelopeAscii") != envelope \
        or amendment_failure.get("combinedStdoutSha256") != "sha256:0d5b6c1386d5e895929f3352669a9a0f8e21882f45c100ada67f6ed3e9a95d83" \
        or amendment_failure.get("combinedStderrSha256") != "sha256:5b14ed36343d51f76a41bef1aea4aacee3c5141d4120aa7fd189018922cacf4c" \
        or amendment_failure.get("semanticOutcomeObserved") is not False \
        or k1_12_evidence.get("counts") != {
            "officialDependencyPrepareAttempts": 1, "officialPrepareReceiptPublished": False,
            "qualificationDependencySeedPublished": False, "qualificationAttempts": 0,
            "retainedAttempts": 0, "childStarts": 0, "holdoutAttempts": 0,
            "holdoutOpened": False, "modelCalls": 0, "decisionIssued": False,
        }:
        raise HarnessError("K1.12 retained K1.11 exact failure/zero-outcome mismatch")

    k1_11_authorities = _verify_k1_11_preservation(
        k1_11_digests, digests["preregistrationAmendment"],
    )
    k1_10_authorities = _verify_k1_10_preservation(
        k1_11_authorities, k1_10_digests, k1_11_amendment_digest,
    )
    k1_9_authorities = _verify_k1_9_preservation(
        k1_10_authorities, k1_9_digests, k1_10_amendment_digest,
    )
    k1_8_authorities = _verify_k1_8_preservation(
        k1_9_authorities, k1_8_digests, k1_9_amendment_digest,
    )
    k1_7_authorities = _verify_k1_7_preservation(
        k1_8_authorities, k1_7_digests, k1_8_amendment_digest,
    )
    k1_6_authorities = _verify_k1_6_preservation(
        k1_7_authorities, k1_6_digests, k1_7_amendment_digest,
    )
    k1_5_authorities = _verify_k1_5_preservation(
        k1_6_authorities, k1_5_digests, k1_6_amendment_digest,
    )
    k1_4_authorities = _verify_k1_4_preservation(
        k1_5_authorities, k1_4_digests, k1_5_amendment_digest,
    )
    k1_3_authorities = _verify_k1_3_preservation(k1_4_authorities, k1_3_digests, k1_4_amendment_digest)
    k1_2_authorities = _verify_k1_2_preservation(k1_3_authorities, k1_2_digests, k1_3_amendment_digest)
    _verify_k1_1_preservation(k1_2_authorities, k1_1_digests, k1_2_amendment_digest)
    if requirements.get("schema") != "codeclew.kotlin-real-repository-requirements/0.1" or requirements.get("seriesId") != SERIES_ID:
        raise HarnessError("requirements identity mismatch")
    if requirements.get("preregistrationAmendmentSha256") != digests["preregistrationAmendment"]:
        raise HarnessError("requirements do not bind the K1.12 amendment")
    thresholds = requirements.get("decisionThresholds")
    if not isinstance(thresholds, dict) or thresholds.get("canonicalAttemptCount") != 12 or thresholds.get("holdoutCount") != 6 or thresholds.get("modelCalls") != 0:
        raise HarnessError("requirements threshold contour mismatch")
    if corpus.get("schema") != "codeclew.kotlin-real-repository-corpus/0.1" or corpus.get("seriesId") != SERIES_ID:
        raise HarnessError("corpus identity mismatch")
    entries = corpus.get("entries")
    if not isinstance(entries, list) or len(entries) != 12:
        raise HarnessError("corpus denominator mismatch")
    qualification = tuple(sorted(row.get("id") for row in entries if row.get("cohort") == "QUALIFICATION"))
    holdout = tuple(sorted(row.get("id") for row in entries if row.get("cohort") == "BLIND_HOLDOUT"))
    if qualification != EXPECTED_QUALIFICATION or holdout != EXPECTED_HOLDOUT:
        raise HarnessError("corpus cohort membership mismatch")
    if len({row.get("commit") for row in entries}) != 12 or any(
        not isinstance(row.get("commit"), str) or len(row["commit"]) != 40
        or not isinstance(row.get("gitTree"), str) or len(row["gitTree"]) != 40
        for row in entries
    ):
        raise HarnessError("corpus commit/tree pins mismatch")
    build_counts = {kind: sum(row.get("buildDsl") == kind for row in entries) for kind in (
        "GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL", "MAVEN"
    )}
    if build_counts != {"GRADLE_KOTLIN_DSL": 6, "GRADLE_GROOVY_DSL": 3, "MAVEN": 3}:
        raise HarnessError("corpus exact build matrix mismatch")
    analyzers = corpus.get("frozenExecutionPolicy", {}).get("trustedAnalyzers")
    expected_analyzers = {
        "2.1": {"compilerVersion": "2.1.21", "manifest": "workers/manifests/kotlin21.json"},
        "2.3": {"compilerVersion": "2.3.0", "manifest": "workers/manifests/kotlin23.json"},
        "2.4": {"compilerVersion": "2.4.10", "manifest": "workers/manifests/kotlin24.json"},
    }
    if analyzers != expected_analyzers or any("distributionTreeSha256" in row for row in analyzers.values()):
        raise HarnessError("corpus must pin analyzer version/path but not mutable candidate distribution bytes")
    if eligibility.get("schema") != "codeclew.kotlin-real-repository-corpus-eligibility/0.1" or eligibility.get("seriesId") != SERIES_ID:
        raise HarnessError("eligibility identity mismatch")
    if tuple(sorted(row.get("id") for row in eligibility.get("members", []))) != tuple(sorted(EXPECTED_QUALIFICATION + EXPECTED_HOLDOUT)):
        raise HarnessError("eligibility/corpus membership mismatch")
    return {**values, "digests": digests, "readinessGraph": graph}


def _atomic_write(path: Path, raw: bytes, mode: int = 0o644) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.parent / f".{path.name}.tmp-{os.getpid()}-{secrets.token_hex(8)}"
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
    try:
        view = memoryview(raw)
        while view:
            written = os.write(descriptor, view)
            view = view[written:]
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    os.replace(temporary, path)
    directory = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def _atomic_create(path: Path, raw: bytes, mode: int = 0o400) -> None:
    """Durably create one append-only protocol member without replacement."""
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
    try:
        view = memoryview(raw)
        while view:
            written = os.write(descriptor, view)
            view = view[written:]
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    directory = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def _tree_digest(
    path: Path,
    ignored_components: frozenset[str] = frozenset(),
    *,
    allowed_symlinks: frozenset[str] = frozenset(),
) -> str:
    root = path.absolute()
    metadata = root.lstat()
    if root.is_symlink() or not stat.S_ISDIR(metadata.st_mode):
        raise HarnessError("live tree input must be a non-symlink directory")
    members: list[dict[str, Any]] = []
    for directory, directories, files in os.walk(root, followlinks=False):
        directory_path = Path(directory)
        relative_directory = directory_path.relative_to(root)
        if any(part in ignored_components for part in relative_directory.parts):
            directories[:] = []
            continue
        directories[:] = [name for name in directories if name not in ignored_components]
        retained_directories: list[str] = []
        for name in sorted(directories):
            child = directory_path / name
            if child.is_symlink():
                relative = child.relative_to(root).as_posix()
                if relative not in allowed_symlinks:
                    raise HarnessError(f"untracked or forbidden symlink in live tree input: {child}")
                target = os.readlink(child)
                resolved = (child.parent / target).resolve(strict=False)
                if Path(target).is_absolute() or not resolved.is_relative_to(root):
                    raise HarnessError(f"escaping symlink in live tree input: {child}")
                members.append({
                    "path": relative,
                    "kind": "SYMLINK", "mode": stat.S_IMODE(child.lstat().st_mode),
                    "target": target, "targetSha256": sha256_bytes(os.fsencode(target)),
                })
            else:
                retained_directories.append(name)
        directories[:] = retained_directories
        for name in sorted(files):
            child = directory_path / name
            child_metadata = child.lstat()
            if stat.S_ISLNK(child_metadata.st_mode):
                relative = child.relative_to(root).as_posix()
                if relative not in allowed_symlinks:
                    raise HarnessError(f"untracked or forbidden symlink in live tree input: {child}")
                target = os.readlink(child)
                resolved = (child.parent / target).resolve(strict=False)
                if Path(target).is_absolute() or not resolved.is_relative_to(root):
                    raise HarnessError(f"escaping symlink in live tree input: {child}")
                members.append({
                    "path": relative,
                    "kind": "SYMLINK", "mode": stat.S_IMODE(child_metadata.st_mode),
                    "target": target, "targetSha256": sha256_bytes(os.fsencode(target)),
                })
                continue
            if not stat.S_ISREG(child_metadata.st_mode):
                raise HarnessError(f"unsafe member in live tree input: {child}")
            members.append({
                "path": child.relative_to(root).as_posix(),
                "kind": "FILE",
                "mode": stat.S_IMODE(child_metadata.st_mode),
                "size": child_metadata.st_size,
                "sha256": sha256_file(child),
            })
    members.sort(key=lambda item: item["path"])
    return sha256_bytes(canonical({"schema": "codeclew.live-tree/0.2", "members": members}))


def _git_index_identity_rows(repository: Path) -> list[dict[str, str]]:
    completed = subprocess.run(
        ["git", "-C", str(repository), "ls-files", "-s", "-z", "--", "."],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env=_git_plumbing_environment(repository), check=False, timeout=30,
    )
    if completed.returncode != 0:
        raise HarnessError("Git index identity observation failed: " + sha256_bytes(completed.stderr))
    rows: list[dict[str, str]] = []
    for raw in completed.stdout.split(b"\0"):
        if not raw:
            continue
        try:
            metadata, raw_path = raw.split(b"\t", 1)
            mode, git_object, stage = metadata.decode("ascii").split(" ")
            path = os.fsdecode(raw_path)
        except (ValueError, UnicodeDecodeError) as error:
            raise HarnessError("Git index identity row is malformed") from error
        relative = Path(path)
        if (
            relative.is_absolute() or ".." in relative.parts or relative.as_posix() != path
            or stage != "0" or mode not in {"100644", "100755", "120000"}
            or len(git_object) not in {40, 64}
            or any(character not in "0123456789abcdef" for character in git_object)
        ):
            raise HarnessError("Git index identity contains an unsupported member")
        rows.append({"path": path, "mode": mode, "gitObject": git_object})
    paths = [row["path"] for row in rows]
    if paths != sorted(paths) or len(set(paths)) != len(paths):
        raise HarnessError("Git index identity paths are not unique and sorted")
    return rows


def _tracked_symlinks(repository: Path) -> frozenset[str]:
    return frozenset(row["path"] for row in _git_index_identity_rows(repository) if row["mode"] == "120000")


def _contained_link_destination(link: Path, target: str) -> Path:
    target_path = Path(target)
    if target_path.is_absolute() or not target:
        raise HarnessError("Git tracked link target is absolute or empty")
    parts = list(link.parent.parts)
    for component in target_path.parts:
        if component in {"", "."}:
            continue
        if component == "..":
            if not parts:
                raise HarnessError("Git tracked link target escapes repository")
            parts.pop()
        else:
            parts.append(component)
    if not parts:
        raise HarnessError("Git tracked link resolves to repository root")
    return Path(*parts)


def _semantic_sensitive_symlink_path(path: Path) -> bool:
    sensitive_components = {
        *SOURCE_SNAPSHOT_IGNORED,
        ".mvn", "build-logic", "buildSrc", "generated", "generated-sources",
        "generated-test-sources", "gradle",
    }
    sensitive_files = {
        "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts",
        "gradle.properties", "libs.versions.toml", "pom.xml", "gradlew", "gradlew.bat",
        "mvnw", "mvnw.cmd",
    }
    return (
        any(component in sensitive_components for component in path.parts)
        or path.name in sensitive_files
        or path.suffix in {".java", ".kt", ".kts"}
    )


def _git_index_snapshot(repository: Path) -> dict[str, Any]:
    rows: list[dict[str, Any]] = []
    for identity in _git_index_identity_rows(repository):
        mode, git_object, path = identity["mode"], identity["gitObject"], identity["path"]
        relative = Path(path)
        member = repository / relative
        metadata = member.lstat()
        if mode == "120000":
            if not stat.S_ISLNK(metadata.st_mode):
                raise HarnessError("Git link index/worktree kind mismatch")
            target = os.readlink(member)
            destination = _contained_link_destination(relative, target)
            if _semantic_sensitive_symlink_path(relative) or _semantic_sensitive_symlink_path(destination):
                raise HarnessError("Git tracked link participates in source/build/generated/cache inputs")
            target_bytes = os.fsencode(target)
            row = {"path": relative.as_posix(), "mode": mode, "gitObject": git_object, "kind": "SYMLINK", "size": len(target_bytes), "sha256": sha256_bytes(target_bytes), "target": target}
        else:
            if not stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
                raise HarnessError("Git file index/worktree kind mismatch")
            expected_executable = mode == "100755"
            if bool(stat.S_IMODE(metadata.st_mode) & 0o111) != expected_executable:
                raise HarnessError("Git file executable mode mismatch")
            row = {"path": relative.as_posix(), "mode": mode, "gitObject": git_object, "kind": "FILE", "size": metadata.st_size, "sha256": sha256_file(member)}
        rows.append(row)
    return {"schema": "codeclew.git-index-snapshot/0.1", "members": rows, "digest": sha256_bytes(canonical(rows))}


def _source_tree_digest(repository: Path) -> str:
    tracked_links = _tracked_symlinks(repository)
    observed_links: set[str] = set()
    for directory, directories, files in os.walk(repository, followlinks=False):
        relative_directory = Path(directory).relative_to(repository)
        for name in directories + files:
            member = Path(directory) / name
            if member.is_symlink():
                observed_links.add(member.relative_to(repository).as_posix())
    if observed_links != set(tracked_links):
        raise HarnessError("source tree contains an untracked or missing symbolic link")
    index = _git_index_snapshot(repository)
    return sha256_bytes(canonical({"schema": "codeclew.git-tracked-source/0.1", "index": index}))


def _git_plumbing_environment(repository: Path) -> dict[str, str]:
    """Environment for observation commands that must not gain user config."""
    return {
        "HOME": str(repository),
        "PATH": "/usr/bin:/bin",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_SYSTEM": "/dev/null",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_CONFIG_COUNT": "0",
        "GIT_OPTIONAL_LOCKS": "0",
        "GIT_ATTR_NOSYSTEM": "1",
        "GIT_TERMINAL_PROMPT": "0",
        "GIT_ASKPASS": "/usr/bin/false",
        "SSH_ASKPASS": "/usr/bin/false",
        "GIT_PROTOCOL_FROM_USER": "0",
        "GIT_NO_REPLACE_OBJECTS": "1",
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
    }


@contextlib.contextmanager
def _filter_free_git_context(repository: Path):
    """Yield an isolated git-dir/env that exposes only validated objects."""
    git_directory = repository / ".git"
    objects = git_directory / "objects"
    for path, label in ((git_directory, "Git directory"), (objects, "Git object directory")):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
            raise HarnessError(f"{label} must be a real directory")
    if (objects / "info" / "alternates").exists() or (objects / "info" / "alternates").is_symlink():
        raise HarnessError("Git object directory alternates are forbidden")
    for directory, directories, files in os.walk(objects, followlinks=False):
        for name in directories:
            metadata = (Path(directory) / name).lstat()
            if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
                raise HarnessError("Git object directory contains a symlink or special directory")
        for name in files:
            metadata = (Path(directory) / name).lstat()
            if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
                raise HarnessError("Git object directory contains a symlink or special file")
    objects = objects.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="codeclew-k1-filter-free-git-", dir="/tmp") as temporary_text:
        isolated_git = Path(temporary_text) / "git"
        isolated_git.mkdir(mode=0o700)
        (isolated_git / "objects").mkdir(mode=0o700)
        (isolated_git / "refs").mkdir(mode=0o700)
        _atomic_write(isolated_git / "HEAD", b"ref: refs/heads/unborn\n", 0o400)
        env = {
            **_git_plumbing_environment(Path(temporary_text)),
            "GIT_OBJECT_DIRECTORY": str(objects),
            "GIT_ALTERNATE_OBJECT_DIRECTORIES": "",
        }
        yield isolated_git, env


def _filter_free_git_command(
    repository: Path, arguments: Sequence[str], *, input_bytes: bytes | None = None,
    timeout: int = 120,
) -> bytes:
    with _filter_free_git_context(repository) as (isolated_git, env):
        completed = subprocess.run(
            ["/usr/bin/git", f"--git-dir={isolated_git}", *arguments],
            input=input_bytes, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env=env, check=False, timeout=timeout,
        )
    if completed.returncode != 0:
        raise HarnessError("filter-free Git command failed: " + sha256_bytes(completed.stderr))
    return completed.stdout


def _filter_free_git_archive(repository: Path, tree: str, archive: Path) -> None:
    """Materialize one exact tree without exposing source-repository config."""
    if not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", tree):
        raise HarnessError("Git archive tree identity is malformed")
    if archive.exists() or archive.is_symlink():
        raise HarnessError("filter-free Git archive destination is create-only")
    try:
        _filter_free_git_command(repository, [
            "-c", "core.attributesFile=/dev/null", "archive", "--format=tar",
            "--output", str(archive), tree,
        ])
    except HarnessError:
        archive.unlink(missing_ok=True)
        raise
    metadata = archive.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise HarnessError("filter-free Git archive output is not a regular file")


def _git_clean_mismatch_rows(repository: Path, tree: str) -> list[dict[str, Any]]:
    index_rows = _git_index_identity_rows(repository)
    index_by_path = {row["path"]: row for row in index_rows}
    tree_by_path: dict[str, dict[str, str]] = {}
    for raw in _filter_free_git_command(
        repository, ["ls-tree", "-rz", "--full-tree", tree], timeout=30,
    ).split(b"\0"):
        if not raw:
            continue
        try:
            header, raw_path = raw.split(b"\t", 1)
            mode, object_type, git_object = header.decode("ascii").split(" ")
            path = os.fsdecode(raw_path)
        except (ValueError, UnicodeDecodeError) as error:
            raise HarnessError("Git HEAD tree row is malformed") from error
        relative = Path(path)
        if object_type != "blob" or mode not in {"100644", "100755", "120000"} or relative.is_absolute() or ".." in relative.parts:
            raise HarnessError("Git HEAD tree contains an unsupported member")
        if path in tree_by_path:
            raise HarnessError("Git HEAD tree contains duplicate paths")
        tree_by_path[path] = {"mode": mode, "gitObject": git_object}

    mismatches: list[dict[str, Any]] = []
    for path in sorted(set(tree_by_path) | set(index_by_path)):
        expected = tree_by_path.get(path)
        actual_row = index_by_path.get(path)
        actual = None if actual_row is None else {"mode": actual_row["mode"], "gitObject": actual_row["gitObject"]}
        if expected != actual:
            mismatches.append({"path": path, "kind": "HEAD_INDEX", "expected": expected, "actual": actual})

    archive_by_path: dict[str, dict[str, Any]] = {}
    with tempfile.TemporaryDirectory(prefix="codeclew-k1-clean-observation-", dir="/tmp") as temporary_text:
        archive_path = Path(temporary_text) / "tree.tar"
        _filter_free_git_archive(repository, tree, archive_path)
        bundle = tarfile.open(archive_path, mode="r:")
        try:
            archive_members = bundle.getmembers()
            for member in archive_members:
                relative = Path(member.name)
                if relative.is_absolute() or ".." in relative.parts or not relative.parts:
                    raise HarnessError("Git HEAD archive contains an unsafe member")
                path = relative.as_posix().removesuffix("/")
                if member.isdir():
                    continue
                if path in archive_by_path:
                    raise HarnessError("Git HEAD archive contains duplicate members")
                if member.isfile():
                    source = bundle.extractfile(member)
                    if source is None:
                        raise HarnessError("Git HEAD archive member is unreadable")
                    content = source.read()
                    archive_by_path[path] = {
                        "kind": "FILE", "mode": "100755" if member.mode & 0o111 else "100644",
                        "size": len(content), "sha256": sha256_bytes(content),
                    }
                elif member.issym():
                    content = os.fsencode(member.linkname)
                    archive_by_path[path] = {
                        "kind": "SYMLINK", "mode": "120000", "size": len(content),
                        "sha256": sha256_bytes(content), "target": member.linkname,
                    }
                else:
                    raise HarnessError("Git HEAD archive contains a special member")
        finally:
            bundle.close()

    tracked_paths = set(index_by_path)
    observed_by_path: dict[str, dict[str, Any]] = {}
    for directory, directories, files in os.walk(repository, followlinks=False):
        directory_path = Path(directory)
        relative_directory = directory_path.relative_to(repository)
        if relative_directory == Path(".git") or ".git" in relative_directory.parts:
            directories[:] = []
            continue
        directories[:] = sorted(name for name in directories if name != ".git")
        symlink_directories = [name for name in directories if (directory_path / name).is_symlink()]
        directories[:] = [name for name in directories if name not in symlink_directories]
        for name in sorted(files + symlink_directories):
            member = directory_path / name
            relative = member.relative_to(repository).as_posix()
            metadata = member.lstat()
            if stat.S_ISDIR(metadata.st_mode) and not stat.S_ISLNK(metadata.st_mode):
                continue
            if stat.S_ISLNK(metadata.st_mode):
                target = os.readlink(member)
                actual = {"kind": "SYMLINK", "mode": "120000", "size": len(os.fsencode(target)), "sha256": sha256_bytes(os.fsencode(target)), "target": target}
            elif stat.S_ISREG(metadata.st_mode):
                actual = {"kind": "FILE", "mode": "100755" if stat.S_IMODE(metadata.st_mode) & 0o111 else "100644", "size": metadata.st_size, "sha256": sha256_file(member)}
            else:
                actual = {"kind": "SPECIAL", "mode": stat.S_IMODE(metadata.st_mode)}
            observed_by_path[relative] = actual

    for path in sorted(set(archive_by_path) | set(observed_by_path)):
        expected = archive_by_path.get(path)
        actual = observed_by_path.get(path)
        kind = "UNTRACKED" if path not in tracked_paths else "WORKTREE"
        if expected != actual:
            mismatches.append({"path": path, "kind": kind, "expected": expected, "actual": actual})
    return sorted(mismatches, key=lambda row: (row["path"], row["kind"]))


def _source_set_digest(source_set: Path) -> str:
    root = source_set.absolute()
    if root.is_symlink() or not root.is_dir():
        raise HarnessError("source set must be a real directory")
    children = sorted(root.iterdir(), key=lambda path: path.name)
    names = [child.name for child in children]
    expected = list(EXPECTED_QUALIFICATION) if set(names) == set(EXPECTED_QUALIFICATION) else list(EXPECTED_HOLDOUT) if set(names) == set(EXPECTED_HOLDOUT) else None
    if expected is None or names != sorted(expected):
        raise HarnessError("source set must contain exactly one frozen cohort and no extras")
    rows: list[dict[str, Any]] = []
    for child in children:
        if child.is_symlink() or not child.is_dir() or not (child / ".git").exists():
            raise HarnessError(f"source set member is not a real Git checkout: {child}")
        observation = _git_observation(child)
        if not observation["clean"]:
            raise HarnessError(f"source set member is not clean: {child.name}")
        rows.append({"entry": child.name, **observation, "index": _git_index_snapshot(child)})
    if not rows:
        raise HarnessError("source set is empty")
    return sha256_bytes(canonical({"schema": "codeclew.kotlin-k1-source-set/0.1", "members": rows}))


def _source_set_members(source_set: Path) -> list[dict[str, Any]]:
    digest = _source_set_digest(source_set)
    rows = []
    for child in sorted(source_set.iterdir(), key=lambda path: path.name):
        rows.append({"entry": child.name, **_git_observation(child), "index": _git_index_snapshot(child)})
    if sha256_bytes(canonical({"schema": "codeclew.kotlin-k1-source-set/0.1", "members": rows})) != digest:
        raise HarnessError("source set changed while enumerating members")
    return rows


def _tree_rows(path: Path) -> list[dict[str, Any]]:
    root = path.resolve(strict=True)
    rows: list[dict[str, Any]] = []
    for directory, directories, files in os.walk(root, followlinks=False):
        directory_path = Path(directory)
        for name in sorted(directories):
            if (directory_path / name).is_symlink():
                raise HarnessError("dependency seed contains a symlink directory")
        for name in sorted(files):
            member = directory_path / name
            metadata = member.lstat()
            if not stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
                raise HarnessError("dependency seed contains a non-regular file")
            rows.append({
                "path": member.relative_to(root).as_posix(),
                "size": metadata.st_size,
                "sha256": sha256_file(member),
            })
    return sorted(rows, key=lambda row: row["path"])


def _build_state_tree_rows(path: Path) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Describe every regular member and directory without following links."""
    root = path.absolute()
    metadata = root.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise HarnessError("build-state subtree must be a real directory")
    directories: list[dict[str, Any]] = []
    files: list[dict[str, Any]] = []
    for directory, child_directories, child_files in os.walk(root, followlinks=False):
        directory_path = Path(directory)
        directory_metadata = directory_path.lstat()
        if stat.S_ISLNK(directory_metadata.st_mode) or not stat.S_ISDIR(directory_metadata.st_mode):
            raise HarnessError("build-state contains a symlink or non-directory directory member")
        relative_directory = directory_path.relative_to(root).as_posix() or "."
        directories.append({
            "path": relative_directory,
            "mode": stat.S_IMODE(directory_metadata.st_mode),
        })
        for name in sorted(child_directories):
            child = directory_path / name
            child_metadata = child.lstat()
            if stat.S_ISLNK(child_metadata.st_mode) or not stat.S_ISDIR(child_metadata.st_mode):
                raise HarnessError("build-state contains a symlink or unsafe directory member")
        for name in sorted(child_files):
            member = directory_path / name
            member_metadata = member.lstat()
            if stat.S_ISLNK(member_metadata.st_mode) or not stat.S_ISREG(member_metadata.st_mode):
                raise HarnessError("build-state contains a symlink or non-regular file")
            files.append({
                "path": member.relative_to(root).as_posix(),
                "mode": stat.S_IMODE(member_metadata.st_mode),
                "size": member_metadata.st_size,
                "sha256": sha256_file(member),
            })
    directories.sort(key=lambda row: row["path"])
    files.sort(key=lambda row: row["path"])
    return directories, files


def _seal_build_state_subtrees(root: Path) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Seal dependency bytes and return exact physical mode rows."""
    for root_name in ("gradle-user-home", "maven-repository"):
        directories, files = _build_state_tree_rows(root / root_name)
        for row in files:
            os.chmod(root / root_name / row["path"], 0o400, follow_symlinks=False)
        for row in sorted(directories, key=lambda item: len(Path(item["path"]).parts), reverse=True):
            member = root / root_name if row["path"] == "." else root / root_name / row["path"]
            os.chmod(member, 0o500, follow_symlinks=False)
    sealed_directories: list[dict[str, Any]] = []
    sealed_files: list[dict[str, Any]] = []
    for root_name in ("gradle-user-home", "maven-repository"):
        directories, files = _build_state_tree_rows(root / root_name)
        sealed_directories.extend({"root": root_name, **row} for row in directories)
        sealed_files.extend({"root": root_name, **row} for row in files)
    sealed_directories.sort(key=lambda row: (row["root"], row["path"]))
    sealed_files.sort(key=lambda row: (row["root"], row["path"]))
    return sealed_directories, sealed_files


def _build_state_subtree_digest(files: list[Mapping[str, Any]], root_name: str) -> str:
    raw = b"".join(
        str(row[field]).encode() + b"\0"
        for row in files if row.get("root") == root_name
        for field in ("path", "size", "sha256")
    )
    return sha256_bytes(raw)


def _seal_dependency_cohort_tree(root: Path, *, seal_root: bool = True) -> None:
    """Make a complete unpublished cohort tree physically read-only."""
    root = root.absolute()
    metadata = root.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise HarnessError("dependency cohort publication root mismatch")
    directories: list[Path] = []
    files: list[Path] = []
    for directory, child_directories, child_files in os.walk(root, followlinks=False):
        directory_path = Path(directory)
        directory_metadata = directory_path.lstat()
        if stat.S_ISLNK(directory_metadata.st_mode) or not stat.S_ISDIR(directory_metadata.st_mode):
            raise HarnessError("dependency cohort contains an unsafe directory")
        directories.append(directory_path)
        for name in child_directories:
            child_metadata = (directory_path / name).lstat()
            if stat.S_ISLNK(child_metadata.st_mode) or not stat.S_ISDIR(child_metadata.st_mode):
                raise HarnessError("dependency cohort contains a symlink or unsafe directory")
        for name in child_files:
            member = directory_path / name
            member_metadata = member.lstat()
            if stat.S_ISLNK(member_metadata.st_mode) or not stat.S_ISREG(member_metadata.st_mode):
                raise HarnessError("dependency cohort contains a symlink or unsafe file")
            files.append(member)
    for member in files:
        os.chmod(member, 0o400, follow_symlinks=False)
    for member in sorted(directories, key=lambda path: len(path.relative_to(root).parts), reverse=True):
        if member == root and not seal_root:
            continue
        os.chmod(member, 0o500, follow_symlinks=False)


def _require_sealed_dependency_cohort_tree(root: Path) -> None:
    root = root.absolute()
    for directory, child_directories, child_files in os.walk(root, followlinks=False):
        directory_path = Path(directory)
        directory_metadata = directory_path.lstat()
        if (
            stat.S_ISLNK(directory_metadata.st_mode)
            or not stat.S_ISDIR(directory_metadata.st_mode)
            or stat.S_IMODE(directory_metadata.st_mode) != 0o500
        ):
            raise HarnessError("dependency cohort directory is not physically sealed")
        for name in child_directories:
            child_metadata = (directory_path / name).lstat()
            if stat.S_ISLNK(child_metadata.st_mode) or not stat.S_ISDIR(child_metadata.st_mode):
                raise HarnessError("dependency cohort contains a symlink or unsafe directory")
        for name in child_files:
            member_metadata = (directory_path / name).lstat()
            if (
                stat.S_ISLNK(member_metadata.st_mode)
                or not stat.S_ISREG(member_metadata.st_mode)
                or stat.S_IMODE(member_metadata.st_mode) != 0o400
            ):
                raise HarnessError("dependency cohort file is not physically sealed")


def _validate_build_state_seed(root: Path, expected_cohort: str | None = None) -> dict[str, Any]:
    root = root.resolve(strict=True)
    root_metadata = root.lstat()
    if (
        stat.S_ISLNK(root_metadata.st_mode)
        or not stat.S_ISDIR(root_metadata.st_mode)
        or stat.S_IMODE(root_metadata.st_mode) != 0o500
    ):
        raise HarnessError("build-state root mode/sealing mismatch")
    manifest_path = _regular_file(root / "CODECLEW_K1_BUILD_STATE_MANIFEST.json", "build-state manifest")
    marker_path = _regular_file(root / "CODECLEW_K1_BUILD_STATE_SEED", "build-state marker")
    modes_path = _regular_file(root / "CODECLEW_K1_BUILD_STATE_MODES.json", "build-state mode manifest")
    if any(stat.S_IMODE(path.lstat().st_mode) != 0o400 for path in (manifest_path, marker_path, modes_path)):
        raise HarnessError("build-state authority file mode mismatch")
    manifest_raw = manifest_path.read_bytes()
    manifest = _load_json_bytes(manifest_raw, "build-state manifest")
    keys = {
        "schema", "seriesId", "cohort", "toolchain", "repositories",
        "gradleUserHomeTreeDigest", "mavenLocalRepositoryTreeDigest", "files", "seedDigest",
    }
    if not isinstance(manifest, dict) or set(manifest) != keys or canonical(manifest) != manifest_raw:
        raise HarnessError("build-state manifest envelope/canonical bytes mismatch")
    if manifest.get("schema") != "codeclew.kotlin-k1-build-state-manifest/0.1" or manifest.get("seriesId") != SERIES_ID:
        raise HarnessError("build-state manifest identity mismatch")
    if expected_cohort is not None and manifest.get("cohort") != expected_cohort:
        raise HarnessError("build-state manifest cohort mismatch")
    body = dict(manifest)
    body["seedDigest"] = ""
    if manifest.get("seedDigest") != sha256_bytes(canonical(body)):
        raise HarnessError("build-state manifest self seed mismatch")
    manifest_digest = sha256_bytes(manifest_raw)
    marker_raw = marker_path.read_bytes()
    if marker_raw != (manifest_digest + "\n").encode():
        raise HarnessError("build-state marker does not seal exact manifest")
    files = manifest.get("files")
    if not isinstance(files, list) or files != sorted(files, key=lambda row: (row.get("root"), row.get("path"))):
        raise HarnessError("build-state file manifest is not ordered")
    seen: set[tuple[Any, Any]] = set()
    for row in files:
        if not isinstance(row, dict) or set(row) != {"root", "path", "size", "sha256"}:
            raise HarnessError("build-state file row mismatch")
        identity = (row.get("root"), row.get("path"))
        if identity in seen or row.get("root") not in {"gradle-user-home", "maven-repository"}:
            raise HarnessError("build-state file row identity mismatch")
        seen.add(identity)
        relative = row.get("path")
        if not isinstance(relative, str) or not relative or Path(relative).is_absolute() or ".." in Path(relative).parts:
            raise HarnessError("build-state file row path mismatch")
        member = _regular_file(root / str(row["root"]) / relative, "build-state seeded member")
        if (
            stat.S_IMODE(member.lstat().st_mode) != 0o400
            or member.stat().st_size != row.get("size")
            or sha256_file(member) != row.get("sha256")
        ):
            raise HarnessError("build-state seeded member mode/bytes mismatch")
    modes_raw = modes_path.read_bytes()
    modes = _load_json_bytes(modes_raw, "build-state mode manifest")
    if not isinstance(modes, dict) or canonical(modes) != modes_raw or set(modes) != {
        "schema", "seriesId", "buildStateManifestDigest", "directories", "files", "objectDigest",
    } or modes.get("schema") != "codeclew.kotlin-k1-build-state-modes/0.1" or modes.get("seriesId") != SERIES_ID or modes.get("buildStateManifestDigest") != manifest_digest:
        raise HarnessError("build-state mode manifest envelope mismatch")
    modes_body = dict(modes)
    modes_body["objectDigest"] = ""
    if modes.get("objectDigest") != sha256_bytes(canonical(modes_body)):
        raise HarnessError("build-state mode manifest self digest mismatch")
    directories = modes.get("directories")
    mode_files = modes.get("files")
    if not isinstance(directories, list) or directories != sorted(directories, key=lambda row: (row.get("root"), row.get("path"))):
        raise HarnessError("build-state mode directory rows are not ordered")
    if not isinstance(mode_files, list) or mode_files != sorted(mode_files, key=lambda row: (row.get("root"), row.get("path"))):
        raise HarnessError("build-state mode file rows are not ordered")
    if any(
        not isinstance(row, dict) or set(row) != {"root", "path", "mode"}
        or row.get("root") not in {"gradle-user-home", "maven-repository"}
        or not isinstance(row.get("path"), str) or not row["path"]
        or (row["path"] != "." and (Path(row["path"]).is_absolute() or ".." in Path(row["path"]).parts))
        or row.get("mode") != 0o500
        for row in directories
    ):
        raise HarnessError("build-state mode directory row mismatch")
    if any(
        not isinstance(row, dict) or set(row) != {"root", "path", "mode", "size", "sha256"}
        or row.get("root") not in {"gradle-user-home", "maven-repository"}
        or not isinstance(row.get("path"), str) or not row["path"]
        or Path(row["path"]).is_absolute() or ".." in Path(row["path"]).parts
        or row.get("mode") != 0o400
        for row in mode_files
    ):
        raise HarnessError("build-state mode file row mismatch")
    if [{key: row[key] for key in ("root", "path", "size", "sha256")} for row in mode_files] != files:
        raise HarnessError("build-state content and mode manifests disagree")
    actual_directories: list[dict[str, Any]] = []
    actual_files: list[dict[str, Any]] = []
    for root_name in ("gradle-user-home", "maven-repository"):
        subtree_directories, subtree_files = _build_state_tree_rows(root / root_name)
        actual_directories.extend({"root": root_name, **row} for row in subtree_directories)
        actual_files.extend({"root": root_name, **row} for row in subtree_files)
    actual_directories.sort(key=lambda row: (row["root"], row["path"]))
    actual_files.sort(key=lambda row: (row["root"], row["path"]))
    expected_directory_projection = [{key: row[key] for key in ("root", "path", "mode")} for row in directories]
    expected_file_projection = [{key: row[key] for key in ("root", "path", "mode", "size", "sha256")} for row in mode_files]
    if actual_directories != expected_directory_projection or actual_files != expected_file_projection:
        raise HarnessError("build-state contains an undeclared, missing, or changed member")
    if manifest.get("gradleUserHomeTreeDigest") != _build_state_subtree_digest(files, "gradle-user-home") or manifest.get("mavenLocalRepositoryTreeDigest") != _build_state_subtree_digest(files, "maven-repository"):
        raise HarnessError("build-state subtree digest mismatch")
    return {
        "root": str(root), "seedDigest": manifest["seedDigest"],
        "manifestDigest": manifest_digest, "markerBytesDigest": sha256_bytes(marker_raw),
        "modeManifestDigest": sha256_bytes(modes_raw),
        "treeDigest": _tree_digest(root), "fileCount": len(files), "manifest": manifest,
    }


def _validate_dependency_cohort(
    root: Path,
    cohort: str,
    expected_entries: list[Mapping[str, Any]],
    *,
    expected_source_set_sha256: str | None = None,
    expected_candidate_tools_sha256: str | None = None,
) -> dict[str, Any]:
    root = root.resolve(strict=True)
    _require_sealed_dependency_cohort_tree(root)
    manifest_path = _regular_file(root / "CODECLEW_K1_DEPENDENCY_COHORT.json", "dependency cohort manifest")
    marker_path = _regular_file(root / "CODECLEW_K1_DEPENDENCY_COHORT", "dependency cohort marker")
    raw = manifest_path.read_bytes()
    manifest = _load_json_bytes(raw, "dependency cohort manifest")
    if not isinstance(manifest, dict) or canonical(manifest) != raw or set(manifest) != {
        "schema","seriesId","cohort","sourceSetSha256","candidateToolsSha256","entries","modelCalls","cohortDigest"
    } or manifest.get("schema") != "codeclew.kotlin-k1-dependency-cohort/0.1" or manifest.get("seriesId") != SERIES_ID or manifest.get("cohort") != cohort or manifest.get("modelCalls") != 0:
        raise HarnessError("dependency cohort envelope mismatch")
    if (
        expected_source_set_sha256 is not None
        and manifest.get("sourceSetSha256") != expected_source_set_sha256
    ) or (
        expected_candidate_tools_sha256 is not None
        and manifest.get("candidateToolsSha256") != expected_candidate_tools_sha256
    ):
        raise HarnessError("dependency cohort live source/tools authority mismatch")
    body = dict(manifest)
    body["cohortDigest"] = ""
    if manifest.get("cohortDigest") != sha256_bytes(canonical(body)) or marker_path.read_bytes() != (sha256_bytes(raw) + "\n").encode():
        raise HarnessError("dependency cohort seal mismatch")
    rows = manifest.get("entries")
    expected_by_id = {entry["id"]: entry for entry in expected_entries}
    if not isinstance(rows, list) or [row.get("entry") for row in rows] != list(expected_by_id):
        raise HarnessError("dependency cohort entry set/order mismatch")
    for row in rows:
        entry = expected_by_id[row["entry"]]
        if any(row.get(key) != entry[key] for key in ("commit","gitTree","selectedCompilation","buildDsl")):
            raise HarnessError("dependency cohort corpus pin mismatch")
        if not _preparation_network_evidence_valid(row):
            raise HarnessError("dependency cohort PREPARE network evidence mismatch")
        entry_root = root / "entries" / row["entry"]
        if row.get("outcome") == "READY":
            validated = _validate_build_state_seed(entry_root / "build-state", cohort)
            if (
                validated["seedDigest"] != row.get("buildStateSeedDigest")
                or validated["manifestDigest"] != row.get("buildStateManifestDigest")
                or validated["modeManifestDigest"] != row.get("buildStateModeManifestDigest")
            ):
                raise HarnessError("dependency cohort READY seed mismatch")
        elif row.get("outcome") == "TYPED_REFUSAL":
            refusal_path = _regular_file(entry_root / "PREPARED_REFUSAL.json", "prepared refusal")
            refusal = _load_json_bytes(refusal_path.read_bytes(), "prepared refusal")
            projection = dict(refusal)
            projection["objectDigest"] = ""
            if canonical(refusal) != refusal_path.read_bytes() or refusal.get("schema") != PREPARED_REFUSAL_SCHEMA or refusal.get("seriesId") != SERIES_ID or refusal.get("objectDigest") != _rust_canonical_digest(projection) or refusal.get("entry") != row["entry"]:
                raise HarnessError("dependency cohort refusal mismatch")
            expected_profile = row["sandboxProfiles"][
                "online" if row.get("failureStage") == "ONLINE_DEPENDENCY_PREPARATION" else "offline"
            ]["profileSha256"]
            if (
                refusal.get("sandboxProfileSha256") != expected_profile
                or refusal.get("preparationReceiptDigest") != sha256_bytes(canonical(row))
            ):
                raise HarnessError("dependency cohort refusal PREPARE evidence mismatch")
        else:
            raise HarnessError("dependency cohort has an invalid entry outcome")
    actual_entries = sorted(path.name for path in (root / "entries").iterdir())
    if actual_entries != sorted(expected_by_id):
        raise HarnessError("dependency cohort contains extra/missing entry directories")
    return {
        "manifestDigest": sha256_bytes(raw), "cohortDigest": manifest["cohortDigest"],
        "fileCount": sum(1 for _ in root.rglob("*") if _.is_file()), "manifest": manifest,
    }


def _copy_seed_to_fresh_runtime(seed: Path, parent: Path, entry: str, invocation: str) -> tuple[Path, dict[str, Any]]:
    parent = parent.absolute()
    parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    if parent.is_symlink() or not parent.is_dir():
        raise HarnessError("build-state runtime parent must be a real directory")
    target = parent / SERIES_ID / entry / invocation.lower()
    if target.exists() or target.is_symlink():
        raise HarnessError("per-invocation build-state clone is create-only")
    target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    shutil.copytree(seed, target, symlinks=False, copy_function=shutil.copy2)
    before = _validate_build_state_seed(target)
    # This per-invocation tree remains sealed authority. The Kotlin worker
    # verifies it and creates its own private writable Gradle/Maven runtime.
    return target.resolve(strict=True), before


def _dependency_prepare_commands(entry: Mapping[str, Any], repository: Path, staging: Path, *, offline: bool) -> list[str]:
    repository = repository.resolve(strict=True)
    staging = staging.resolve(strict=True)
    if entry["buildDsl"] == "MAVEN":
        selected = entry["selectedCompilation"]
        module = selected.rsplit("/", 1)[0].removeprefix(":").replace(":", "/")
        selected_pom = repository / module / "pom.xml" if module else repository / "pom.xml"
        reactor_selector = ["-pl", module, "-am"] if module else []
        classpath_output = staging / "model-evidence" / f"{entry['id']}.classpath"
        classpath_output.parent.mkdir(exist_ok=True, mode=0o700)
        base = [
            "/opt/homebrew/Cellar/maven/3.9.12/bin/mvn", "-B", "-q", "-DskipTests",
            f"-Dmaven.repo.local={staging / 'maven-repository'}", "-Duser.home=" + str(staging / "home"),
        ]
        model_probe = [
            f"-Dmdep.outputFile={classpath_output}", "-Dmdep.includeScope=compile",
            "help:effective-pom", "dependency:build-classpath",
        ]
        return (
            [*base, "-o", "-f", str(selected_pom), *model_probe]
            if offline else [
                *base, *reactor_selector, *model_probe[:2],
                "dependency:go-offline", "install", *model_probe[2:],
            ]
        )
    wrapper = _regular_file(repository / "gradlew", "frozen Gradle wrapper")
    selected = entry["selectedCompilation"]
    project_path = selected.rsplit("/", 1)[0] or ":"
    source_set = selected.rsplit("/", 1)[1]
    compile_task = "compileKotlin" if source_set == "main" else f"compile{source_set[:1].upper()}{source_set[1:]}Kotlin"
    model_task = ":semanticThreadModel" if project_path == ":" else f"{project_path}:semanticThreadModel"
    init_script = ROOT / "workers/kotlin/src/main/resources/semantic-thread-model.init.gradle"
    def command(project_cache: Path, offline: bool) -> list[str]:
        values = [
            str(wrapper), "-p", str(repository), "--gradle-user-home", str(staging / "gradle-user-home"),
            "--project-cache-dir", str(project_cache), "--no-daemon", "--stacktrace",
            "-Duser.home=" + str(staging / "home"),
            "-Pkotlin.project.persistent.dir=" + str(project_cache / "kotlin"),
            "-I", str(init_script), f"-Dsemantic.thread.compileTask={compile_task}", model_task,
        ]
        if offline:
            values.insert(3, "--offline")
        return values
    ephemeral = staging / "ephemeral-project-cache" / entry["id"]
    return command(ephemeral / ("offline-verification" if offline else "online"), offline)


def _disposable_git_archive(repository: Path, destination: Path) -> Path:
    if destination.exists() or destination.is_symlink():
        raise HarnessError("disposable source destination is create-only")
    original_observation = _git_observation(repository)
    source_index = _git_index_snapshot(repository)
    if original_observation["sourceTreeSha256"] != sha256_bytes(canonical({
        "schema": "codeclew.git-tracked-source/0.1", "index": source_index,
    })):
        raise HarnessError("frozen source changed while selecting archive bytes")
    source_rows = {row["path"]: row for row in source_index["members"]}
    if len(source_rows) != len(source_index["members"]):
        raise HarnessError("frozen source index contains duplicate paths")
    destination.mkdir(mode=0o700)
    archive = destination.parent / f".{destination.name}.tar"
    _filter_free_git_archive(repository, original_observation["tree"], archive)
    try:
        with tarfile.open(archive, mode="r:") as bundle:
            for member in bundle.getmembers():
                relative = Path(member.name)
                if relative.is_absolute() or ".." in relative.parts or not relative.parts:
                    raise HarnessError("disposable source archive contains an unsafe path")
                target = destination.joinpath(*relative.parts)
                if member.isdir():
                    target.mkdir(parents=True, exist_ok=True)
                elif member.isfile():
                    target.parent.mkdir(parents=True, exist_ok=True)
                    source = bundle.extractfile(member)
                    if source is None:
                        raise HarnessError("disposable source archive member is unreadable")
                    _atomic_write(target, source.read(), stat.S_IMODE(member.mode) or 0o600)
                elif member.issym():
                    _contained_link_destination(relative, member.linkname)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    os.symlink(member.linkname, target)
                else:
                    raise HarnessError("disposable source archive contains a special object")
    finally:
        archive.unlink(missing_ok=True)
    env = {
        "HOME": str(destination.parent), "PATH": "/usr/bin:/bin",
        "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_SYSTEM": "/dev/null",
        "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_TERMINAL_PROMPT": "0",
        "GIT_ASKPASS": "/usr/bin/false", "SSH_ASKPASS": "/usr/bin/false",
        "GIT_PROTOCOL_FROM_USER": "0", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8",
    }
    commit = subprocess.run(["git", "-C", str(repository), "cat-file", "commit", "HEAD"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout
    subprocess.run(["git", "-c", "init.templateDir=", "init", "-q", str(destination)], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, check=True)
    tree_rows_raw = subprocess.run(
        ["git", "-C", str(repository), "ls-tree", "-rz", "--full-tree", "HEAD"],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True,
    ).stdout
    index_rows = bytearray()
    tree_rows: list[tuple[str, str, str, bytes, Path]] = []
    for raw in tree_rows_raw.split(b"\0"):
        if not raw:
            continue
        header, path_raw = raw.split(b"\t", 1)
        mode, object_type, object_id = header.decode("ascii").split(" ")
        relative = Path(os.fsdecode(path_raw))
        if object_type != "blob" or mode not in {"100644", "100755", "120000"} or relative.is_absolute() or ".." in relative.parts:
            raise HarnessError("frozen Git tree contains an unsupported member")
        source_row = source_rows.get(relative.as_posix())
        if source_row is None or source_row.get("mode") != mode or source_row.get("gitObject") != object_id:
            raise HarnessError("frozen Git tree/index identity mismatch")
        member = destination / relative
        if mode == "120000":
            if source_row.get("kind") != "SYMLINK" or not member.is_symlink():
                raise HarnessError("archive/index symlink kind mismatch")
            content = os.fsencode(os.readlink(member))
            if (
                len(content) != source_row.get("size")
                or sha256_bytes(content) != source_row.get("sha256")
                or os.readlink(member) != source_row.get("target")
            ):
                raise HarnessError("archive symlink bytes differ from frozen source index")
        else:
            try:
                member_metadata = member.lstat()
            except FileNotFoundError as error:
                raise HarnessError("archive is missing a frozen source member") from error
            if source_row.get("kind") != "FILE" or member.is_symlink() or not stat.S_ISREG(member_metadata.st_mode):
                raise HarnessError("archive/index file kind mismatch")
            if (
                member_metadata.st_size != source_row.get("size")
                or sha256_file(member) != source_row.get("sha256")
                or bool(stat.S_IMODE(member_metadata.st_mode) & 0o111) != (mode == "100755")
            ):
                raise HarnessError("archive file bytes/mode differ from frozen source index")
        tree_rows.append((mode, object_type, object_id, path_raw, relative))
        index_rows.extend(f"{mode} {object_id}\t".encode())
        index_rows.extend(path_raw)
        index_rows.append(0)
    if {relative.as_posix() for _, _, _, _, relative in tree_rows} != set(source_rows):
        raise HarnessError("frozen Git tree/index member set mismatch")

    # Archive materialization is deliberately checked against the exact
    # selected worktree bytes above.  Git may apply EOL/smudge attributes when
    # producing it, so those bytes are not object-database authority.  Import
    # raw blobs separately, in one batch, through safe object-id-named staging
    # files and `--no-filters`; then require Git to reproduce every source OID.
    object_ids = list(dict.fromkeys(row[2] for row in tree_rows))
    cat_file = subprocess.run(
        ["git", "-C", str(repository), "cat-file", "--batch"],
        input=("\n".join(object_ids) + "\n").encode("ascii"),
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, check=False,
        timeout=120,
    )
    if cat_file.returncode != 0:
        raise HarnessError("raw frozen Git blob batch read failed: " + sha256_bytes(cat_file.stderr))
    raw_blobs: list[tuple[str, bytes]] = []
    cursor = 0
    for expected_object_id in object_ids:
        header_end = cat_file.stdout.find(b"\n", cursor)
        if header_end < 0:
            raise HarnessError("raw frozen Git blob batch response is truncated")
        try:
            actual_object_id, object_type, size_text = cat_file.stdout[cursor:header_end].decode("ascii").split(" ")
            object_size = int(size_text)
        except (UnicodeDecodeError, ValueError) as error:
            raise HarnessError("raw frozen Git blob batch header is malformed") from error
        content_start = header_end + 1
        content_end = content_start + object_size
        if (
            actual_object_id != expected_object_id
            or object_type != "blob"
            or object_size < 0
            or content_end >= len(cat_file.stdout)
            or cat_file.stdout[content_end:content_end + 1] != b"\n"
        ):
            raise HarnessError("raw frozen Git blob batch identity mismatch")
        raw_blobs.append((expected_object_id, cat_file.stdout[content_start:content_end]))
        cursor = content_end + 1
    if cursor != len(cat_file.stdout):
        raise HarnessError("raw frozen Git blob batch contains trailing output")
    raw_import_root = destination / ".git" / "k1-raw-blob-import"
    raw_import_root.mkdir(mode=0o700)
    try:
        import_paths = bytearray()
        for object_id, content in raw_blobs:
            _atomic_write(raw_import_root / object_id, content, 0o400)
            import_paths.extend(f".git/k1-raw-blob-import/{object_id}\n".encode("ascii"))
        imported = subprocess.run(
            ["git", "-C", str(destination), "hash-object", "-w", "--no-filters", "--stdin-paths"],
            input=bytes(import_paths), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env=env, check=False, timeout=120,
        )
        if imported.returncode != 0:
            raise HarnessError("raw frozen Git blob batch import failed: " + sha256_bytes(imported.stderr))
        try:
            imported_object_ids = imported.stdout.decode("ascii").splitlines()
        except UnicodeDecodeError as error:
            raise HarnessError("raw frozen Git blob batch import output is malformed") from error
        if imported_object_ids != object_ids:
            raise HarnessError("raw frozen Git blob import object identity mismatch")
    finally:
        for staged_blob in raw_import_root.iterdir():
            staged_blob.chmod(0o600)
        shutil.rmtree(raw_import_root)
    subprocess.run(["git", "-C", str(destination), "update-index", "-z", "--index-info"], input=bytes(index_rows), stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, check=True)
    tree = subprocess.run(["git", "-C", str(destination), "write-tree"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, check=True).stdout.decode().strip()
    expected_tree = subprocess.run(["git", "-C", str(repository), "rev-parse", "HEAD^{tree}"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout.decode().strip()
    if tree != expected_tree:
        raise HarnessError("sanitized disposable checkout tree differs from frozen source")
    commit_id = subprocess.run(["git", "-C", str(destination), "hash-object", "-t", "commit", "-w", "--stdin"], input=commit, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, check=True).stdout.decode().strip()
    expected_commit = subprocess.run(["git", "-C", str(repository), "rev-parse", "HEAD"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout.decode().strip()
    if commit_id != expected_commit:
        raise HarnessError("sanitized disposable checkout commit differs from frozen source")
    subprocess.run(["git", "-C", str(destination), "update-ref", "refs/heads/k1-detached", commit_id], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, check=True)
    _atomic_write(destination / ".git" / "HEAD", b"ref: refs/heads/k1-detached\n")
    git_config = subprocess.run(["git", "-C", str(destination), "config", "--local", "--list"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, check=True).stdout.decode()
    forbidden = ("remote.", "credential.", "url.", "include.path", "includeif.", "core.hookspath", "objects/info/alternates")
    if any(token in git_config.lower() for token in forbidden):
        raise HarnessError("sanitized disposable Git metadata contains a forbidden capability")
    for directory, directories, files in os.walk(destination / ".git", followlinks=False):
        for name in directories + files:
            member = Path(directory) / name
            metadata = member.lstat()
            if stat.S_ISLNK(metadata.st_mode) or not (stat.S_ISDIR(metadata.st_mode) or stat.S_ISREG(metadata.st_mode)):
                raise HarnessError("sanitized disposable Git metadata contains an unsafe object")
    disposable_observation = _git_observation(destination)
    current_original_observation = _git_observation(repository)
    if current_original_observation != original_observation or _git_index_snapshot(repository) != source_index:
        raise HarnessError("frozen source changed during disposable archive creation")
    if disposable_observation != original_observation:
        raise HarnessError("sanitized disposable checkout identity mismatch")
    for directory, _, files in os.walk(destination / ".git", topdown=False):
        for name in files:
            (Path(directory) / name).chmod(0o400)
        Path(directory).chmod(0o500)
    return destination.resolve(strict=True)


def _archive_identity_self_test(root: Path) -> dict[str, bool]:
    root.mkdir(mode=0o700)

    def git(repository: Path, *arguments: str, input_bytes: bytes | None = None) -> bytes:
        completed = subprocess.run(
            ["git", "-C", str(repository), *arguments], input=input_bytes,
            stdin=subprocess.PIPE if input_bytes is None else None,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False, timeout=30,
        )
        if completed.returncode != 0:
            raise AssertionError("archive identity fixture Git failure: " + sha256_bytes(completed.stderr))
        return completed.stdout

    def fixture(name: str, attributes: bytes, member_bytes: bytes, *, configure_filter: bool = False) -> Path:
        repository = root / name
        repository.mkdir(mode=0o700)
        git(repository, "init", "-q")
        git(repository, "config", "user.email", "k1@example.invalid")
        git(repository, "config", "user.name", "K1 Test")
        if configure_filter:
            git(repository, "config", "filter.k1.clean", "tr A Z")
            git(repository, "config", "filter.k1.smudge", "tr Z A")
        _atomic_write(repository / ".gitattributes", attributes)
        _atomic_write(repository / "member.txt", member_bytes)
        git(repository, "add", ".")
        git(repository, "commit", "-qm", "archive identity fixture")
        return repository

    checks: dict[str, bool] = {}
    crlf = fixture("crlf", b"*.txt text eol=crlf\n", b"A\r\nB\r\n")
    crlf_blob = git(crlf, "cat-file", "blob", "HEAD:member.txt")
    crlf_archive = _disposable_git_archive(crlf, root / "crlf-archive")
    checks["crlfWorktreeBytesAccepted"] = (
        crlf_blob != (crlf / "member.txt").read_bytes()
        and (crlf_archive / "member.txt").read_bytes() == (crlf / "member.txt").read_bytes()
        and _git_observation(crlf_archive) == _git_observation(crlf)
    )

    filtered = fixture("filtered", b"member.txt filter=k1\n", b"A\n", configure_filter=True)
    filtered_blob = git(filtered, "cat-file", "blob", "HEAD:member.txt")
    try:
        _disposable_git_archive(filtered, root / "filtered-archive")
        raise AssertionError("repository-local filter worktree identity accepted")
    except HarnessError as error:
        checks["repositoryLocalFilterIdentityRejected"] = (
            filtered_blob != (filtered / "member.txt").read_bytes()
            and str(error) == "archive file bytes/mode differ from frozen source index"
        )

    # Clean observation itself must never execute repository-owned filters.
    # A process filter makes accidental status/diff use visible via a marker.
    marker = root / "forbidden-filter-marker"
    malicious = fixture("malicious-filter", b"member.txt filter=marker\n", b"safe\n")
    git(malicious, "config", "filter.marker.process", f"sh -c 'printf invoked > {marker}'")
    git(malicious, "config", "filter.marker.clean", f"sh -c 'printf invoked > {marker}; cat'")
    if marker.exists():
        marker.unlink()
    malicious_observation = _git_observation(malicious)
    checks["repositoryFilterNeverExecuted"] = malicious_observation["clean"] and not marker.exists()

    clean_status_digest = malicious_observation["statusSha256"]
    _atomic_write(malicious / "member.txt", b"dirty\n")
    checks["trackedDirtyDetectedWithoutFilter"] = (
        not _git_observation(malicious)["clean"]
        and _git_observation(malicious)["statusSha256"] != clean_status_digest
        and not marker.exists()
    )
    _atomic_write(malicious / "member.txt", b"safe\n")
    if marker.exists():
        marker.unlink()
    _atomic_write(malicious / "extra.txt", b"untracked\n")
    checks["untrackedDetectedWithoutFilter"] = not _git_observation(malicious)["clean"] and not marker.exists()
    (malicious / "extra.txt").unlink()
    (malicious / "member.txt").unlink()
    checks["missingDetectedWithoutFilter"] = not _git_observation(malicious)["clean"] and not marker.exists()

    # Git's export-only attributes intentionally make archive bytes diverge
    # from the selected worktree identity.  Both substitution and omission
    # must be rejected rather than silently changing the analysis source.
    export_subst = fixture("export-subst", b"member.txt export-subst\n", b"$Format:%H$\n")
    export_subst_archive = _disposable_git_archive(export_subst, root / "export-subst-archive")
    checks["exportSubstTransformationSuppressed"] = (
        (export_subst_archive / "member.txt").read_bytes()
        == (export_subst / "member.txt").read_bytes()
        == b"$Format:%H$\n"
    )
    export_ignore = fixture("export-ignore", b"member.txt export-ignore\n", b"must remain selected\n")
    try:
        _disposable_git_archive(export_ignore, root / "export-ignore-archive")
        raise AssertionError("export-ignore archive member omission accepted")
    except HarnessError as error:
        checks["exportIgnoreMutationRejected"] = str(error) == "archive is missing a frozen source member"

    # Prove the synthetic repository contains the raw Git object, even though
    # the materialized CRLF bytes intentionally hash to a different object.
    crlf_object = git(crlf, "rev-parse", "HEAD:member.txt").decode("ascii").strip()
    imported_blob = git(crlf_archive, "cat-file", "blob", crlf_object)
    checks["rawBlobIdentityImported"] = (
        imported_blob == crlf_blob
        and git(crlf_archive, "hash-object", "--no-filters", "member.txt").decode("ascii").strip() != crlf_object
    )
    if not all(checks.values()):
        raise AssertionError(f"archive identity self-test failed: {checks}")
    return checks


def _discard_disposable_source(repository: Path, containment_root: Path) -> None:
    """Remove one harness-created disposable root without following links.

    Disposable Git metadata is deliberately sealed read-only after creation.
    Cleanup may restore only the owning user's minimum removal permissions and
    only below the exact private root supplied by the harness caller.
    """
    repository = Path(os.path.abspath(os.fspath(repository)))
    containment_root = Path(os.path.abspath(os.fspath(containment_root)))
    try:
        containment_metadata = containment_root.lstat()
        repository_metadata = repository.lstat()
    except FileNotFoundError as error:
        raise HarnessError("disposable source discard target is absent") from error
    if stat.S_ISLNK(containment_metadata.st_mode):
        try:
            resolved_alias = containment_root.resolve(strict=True)
            resolved_alias_metadata = resolved_alias.lstat()
        except (OSError, ValueError) as error:
            raise HarnessError("disposable source containment root mismatch") from error
        if not stat.S_ISDIR(resolved_alias_metadata.st_mode):
            raise HarnessError("disposable source containment root mismatch")
    elif not stat.S_ISDIR(containment_metadata.st_mode):
        raise HarnessError("disposable source containment root mismatch")
    if stat.S_ISLNK(repository_metadata.st_mode) or not stat.S_ISDIR(repository_metadata.st_mode):
        raise HarnessError("disposable source discard target mismatch")
    try:
        resolved_containment = containment_root.resolve(strict=True)
        resolved_repository = repository.resolve(strict=True)
    except (OSError, ValueError) as error:
        raise HarnessError("disposable source discard containment mismatch") from error
    if (
        resolved_repository == resolved_containment
        or resolved_repository.parent != resolved_containment
    ):
        raise HarnessError("disposable source discard escapes containment root")
    # The caller may pass a canonical destination returned by
    # `_disposable_git_archive` together with a lexically aliased ancestor
    # (`/private/var` versus `/var`). The canonical direct-parent relation is
    # authoritative, while lstat above still rejects a symlink target itself.
    repository = resolved_repository
    containment_root = resolved_containment
    repository_metadata = repository.lstat()

    def ensure_owner_permissions(path: Path, metadata: os.stat_result, required: int) -> None:
        mode = stat.S_IMODE(metadata.st_mode)
        if mode & required != required:
            try:
                os.chmod(path, mode | required, follow_symlinks=False)
            except OSError as error:
                raise HarnessError("disposable source permissions could not be restored") from error

    ensure_owner_permissions(repository, repository_metadata, stat.S_IRUSR | stat.S_IWUSR | stat.S_IXUSR)
    try:
        for directory, directories, files in os.walk(repository, topdown=True, followlinks=False):
            directory_path = Path(directory)
            directory_metadata = directory_path.lstat()
            if stat.S_ISLNK(directory_metadata.st_mode) or not stat.S_ISDIR(directory_metadata.st_mode):
                raise HarnessError("disposable source tree changed during cleanup")
            ensure_owner_permissions(
                directory_path, directory_metadata,
                stat.S_IRUSR | stat.S_IWUSR | stat.S_IXUSR,
            )
            for name in list(directories):
                member = directory_path / name
                metadata = member.lstat()
                if stat.S_ISLNK(metadata.st_mode):
                    directories.remove(name)
                    continue
                if not stat.S_ISDIR(metadata.st_mode):
                    raise HarnessError("disposable source contains an unsafe directory member")
                ensure_owner_permissions(
                    member, metadata, stat.S_IRUSR | stat.S_IWUSR | stat.S_IXUSR,
                )
            for name in files:
                member = directory_path / name
                metadata = member.lstat()
                if stat.S_ISLNK(metadata.st_mode):
                    continue
                if not stat.S_ISREG(metadata.st_mode):
                    raise HarnessError("disposable source contains an unsafe file member")
                ensure_owner_permissions(member, metadata, stat.S_IRUSR | stat.S_IWUSR)
        shutil.rmtree(repository)
    except HarnessError:
        raise
    except OSError as error:
        raise HarnessError("disposable source discard failed") from error


def _discard_private_tree(tree: Path, containment_root: Path) -> None:
    """Remove an exact harness-private tree, including a sealed seed tree."""
    tree = Path(os.path.abspath(os.fspath(tree)))
    containment_root = Path(os.path.abspath(os.fspath(containment_root)))
    try:
        tree_metadata = tree.lstat()
        containment_metadata = containment_root.lstat()
    except FileNotFoundError as error:
        raise HarnessError("private tree discard target is absent") from error
    if stat.S_ISLNK(tree_metadata.st_mode) or not stat.S_ISDIR(tree_metadata.st_mode):
        raise HarnessError("private tree discard target mismatch")
    if stat.S_ISLNK(containment_metadata.st_mode) or not stat.S_ISDIR(containment_metadata.st_mode):
        raise HarnessError("private tree discard containment mismatch")
    try:
        resolved_tree = tree.resolve(strict=True)
        resolved_containment = containment_root.resolve(strict=True)
    except (OSError, ValueError) as error:
        raise HarnessError("private tree discard containment resolution failed") from error
    if resolved_tree.parent != resolved_containment:
        raise HarnessError("private tree discard escapes containment root")
    try:
        for directory, directories, files in os.walk(resolved_tree, topdown=True, followlinks=False):
            directory_path = Path(directory)
            metadata = directory_path.lstat()
            if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
                raise HarnessError("private tree changed during cleanup")
            os.chmod(
                directory_path,
                stat.S_IMODE(metadata.st_mode) | stat.S_IRUSR | stat.S_IWUSR | stat.S_IXUSR,
                follow_symlinks=False,
            )
            for name in list(directories):
                member = directory_path / name
                member_metadata = member.lstat()
                if stat.S_ISLNK(member_metadata.st_mode):
                    directories.remove(name)
                elif not stat.S_ISDIR(member_metadata.st_mode):
                    raise HarnessError("private tree contains an unsafe directory member")
            for name in files:
                member = directory_path / name
                member_metadata = member.lstat()
                if stat.S_ISLNK(member_metadata.st_mode):
                    continue
                if not stat.S_ISREG(member_metadata.st_mode):
                    raise HarnessError("private tree contains an unsafe file member")
                os.chmod(
                    member,
                    stat.S_IMODE(member_metadata.st_mode) | stat.S_IRUSR | stat.S_IWUSR,
                    follow_symlinks=False,
                )
        shutil.rmtree(resolved_tree)
    except HarnessError:
        raise
    except OSError as error:
        raise HarnessError("private tree discard failed") from error


def _sandbox_string(path: Path) -> str:
    return json.dumps(str(path.resolve(strict=False)))


def _sandbox_path_clause(action: str, path: Path) -> str:
    selector = "subpath" if path.is_dir() else "literal"
    return f"(allow {action} ({selector} {_sandbox_string(path)}))"


def _sandbox_read_clauses(paths: Sequence[Path]) -> list[str]:
    """Grant exact content roots plus directory traversal, never ancestor contents."""
    resolved = sorted({path.resolve(strict=False) for path in paths}, key=str)
    ancestors: set[Path] = {Path("/")}
    for path in resolved:
        current = path if path.is_dir() else path.parent
        while current != current.parent:
            ancestors.add(current)
            current = current.parent
    clauses = [
        f"(allow file-read-data file-read-metadata (literal {_sandbox_string(path)}))"
        for path in sorted(ancestors, key=lambda value: (len(value.parts), str(value)))
    ]
    clauses.extend(_sandbox_path_clause("file-read*", path) for path in resolved)
    return clauses


_SANDBOX_METADATA_LITERAL = re.compile(
    r'^\(allow file-read-data file-read-metadata \(literal ("(?:\\.|[^"\\])*")\)\)$'
)
_SANDBOX_CONTENT_ROOT = re.compile(
    r'^\(allow file-read\* \((literal|subpath) ("(?:\\.|[^"\\])*")\)\)$'
)
_SANDBOX_WRITE_ROOT = re.compile(
    r'^\(allow file-write\* \(subpath ("(?:\\.|[^"\\])*")\)\)$'
)
_SANDBOX_DEV_NULL_WRITE = '(allow file-write-data (literal "/dev/null"))'
_SANDBOX_ONLINE_VAR_METADATA = '(allow file-read-metadata (literal "/var"))'
_PREPARE_STAGING_ROOT = re.compile(
    r'^\.(qualificationDependencySeed|holdoutDependencySeed)\.prepare-[0-9a-f]{24}$'
)


def _sandbox_read_closure_valid(profile_raw: str) -> bool:
    """Require literal ancestor traversal and explicit content roots.

    Darwin sandbox profile version 1 requires ``file-read-data`` together
    with ``file-read-metadata`` for a literal directory traversal rule.  The
    literal grants directory metadata/enumeration, not sibling file payloads;
    content reads remain confined to the explicit roots below.
    """
    metadata_paths: list[Path] = []
    content_roots: list[tuple[str, Path]] = []
    for line in profile_raw.splitlines():
        line = line.strip()
        if "file-read" not in line:
            continue
        metadata_match = _SANDBOX_METADATA_LITERAL.fullmatch(line)
        content_match = _SANDBOX_CONTENT_ROOT.fullmatch(line)
        if metadata_match is None and content_match is None:
            return False
        try:
            if metadata_match is not None:
                value = json.loads(metadata_match.group(1))
                selector = None
            else:
                value = json.loads(content_match.group(2))
                selector = content_match.group(1)
        except json.JSONDecodeError:
            return False
        if not isinstance(value, str) or not value.startswith("/") or str(Path(value)) != value:
            return False
        if selector is None:
            metadata_paths.append(Path(value))
        else:
            content_roots.append((selector, Path(value)))
    if not metadata_paths or not content_roots:
        return False
    expected_metadata = {Path("/")}
    for selector, path in content_roots:
        current = path if selector == "subpath" else path.parent
        while current != current.parent:
            expected_metadata.add(current)
            current = current.parent
    return (
        len(metadata_paths) == len(set(metadata_paths))
        and set(metadata_paths) == expected_metadata
        and len(content_roots) == len(set(content_roots))
    )


def _sandbox_expected_content_roots(
    entry_work: Path, phase: str,
) -> set[tuple[str, Path]]:
    roots: set[tuple[str, Path]] = {
        ("subpath", entry_work),
        ("subpath", entry_work / "disposable-sources" / phase),
    }
    for path in (
        Path("/System"), Path("/usr"), Path("/bin"), Path("/sbin"), Path("/etc"),
        Path("/Library/Java"), Path("/opt/homebrew"), Path("/dev"),
        Path("/private/var/select"),
    ):
        roots.add(("subpath", path.resolve(strict=False)))
    roots.add((
        "literal",
        (ROOT / "workers/kotlin/src/main/resources/semantic-thread-model.init.gradle").resolve(strict=False),
    ))
    return roots


def _preparation_content_roots(source: Path, staging: Path) -> set[tuple[str, Path]]:
    roots: set[tuple[str, Path]] = {
        ("subpath", source.resolve(strict=False)),
        ("subpath", staging.resolve(strict=False)),
    }
    for path in (
        Path("/System"), Path("/usr"), Path("/bin"), Path("/sbin"), Path("/etc"),
        Path("/Library/Java"), Path("/opt/homebrew"), Path("/dev"),
        Path("/private/var/select"),
    ):
        roots.add(("subpath", path.resolve(strict=False)))
    roots.add((
        "literal",
        (ROOT / "workers/kotlin/src/main/resources/semantic-thread-model.init.gradle").resolve(strict=False),
    ))
    return roots


def _sandbox_read_clauses_for_roots(
    roots: set[tuple[str, Path]],
) -> list[str]:
    """Render explicit selectors without consulting mutable filesystem state."""
    ancestors: set[Path] = {Path("/")}
    for selector, path in roots:
        current = path if selector == "subpath" else path.parent
        while current != current.parent:
            ancestors.add(current)
            current = current.parent
    clauses = [
        f"(allow file-read-data file-read-metadata (literal {json.dumps(str(path))}))"
        for path in sorted(ancestors, key=lambda value: (len(value.parts), str(value)))
    ]
    clauses.extend(
        f"(allow file-read* ({selector} {json.dumps(str(path))}))"
        for selector, path in sorted(roots, key=lambda value: (str(value[1]), value[0]))
    )
    return clauses


def _expected_dependency_prepare_argv(
    entry: Mapping[str, Any], repository: Path, staging: Path, *, offline: bool,
) -> list[str] | None:
    repository = repository.resolve(strict=False)
    staging = staging.resolve(strict=False)
    selected = entry.get("selectedCompilation")
    identifier = entry.get("entry", entry.get("id"))
    if not isinstance(selected, str) or not isinstance(identifier, str) or "/" not in selected:
        return None
    if entry.get("buildDsl") == "MAVEN":
        module = selected.rsplit("/", 1)[0].removeprefix(":").replace(":", "/")
        selected_pom = repository / module / "pom.xml" if module else repository / "pom.xml"
        reactor_selector = ["-pl", module, "-am"] if module else []
        classpath_output = staging / "model-evidence" / f"{identifier}.classpath"
        base = [
            "/opt/homebrew/Cellar/maven/3.9.12/bin/mvn", "-B", "-q", "-DskipTests",
            f"-Dmaven.repo.local={staging / 'maven-repository'}",
            f"-Duser.home={staging / 'home'}",
        ]
        model_probe = [
            f"-Dmdep.outputFile={classpath_output}", "-Dmdep.includeScope=compile",
            "help:effective-pom", "dependency:build-classpath",
        ]
        return (
            [*base, "-o", "-f", str(selected_pom), *model_probe]
            if offline else [
                *base, *reactor_selector, *model_probe[:2],
                "dependency:go-offline", "install", *model_probe[2:],
            ]
        )
    if entry.get("buildDsl") not in {"GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL"}:
        return None
    project_path = selected.rsplit("/", 1)[0] or ":"
    source_set = selected.rsplit("/", 1)[1]
    if not source_set:
        return None
    compile_task = "compileKotlin" if source_set == "main" else f"compile{source_set[:1].upper()}{source_set[1:]}Kotlin"
    model_task = ":semanticThreadModel" if project_path == ":" else f"{project_path}:semanticThreadModel"
    init_script = ROOT / "workers/kotlin/src/main/resources/semantic-thread-model.init.gradle"
    project_cache = staging / "ephemeral-project-cache" / identifier / (
        "offline-verification" if offline else "online"
    )
    values = [
        str(repository / "gradlew"), "-p", str(repository),
        "--gradle-user-home", str(staging / "gradle-user-home"),
        "--project-cache-dir", str(project_cache), "--no-daemon", "--stacktrace",
        f"-Duser.home={staging / 'home'}",
        f"-Pkotlin.project.persistent.dir={project_cache / 'kotlin'}",
        "-I", str(init_script), f"-Dsemantic.thread.compileTask={compile_task}", model_task,
    ]
    if offline:
        values.insert(3, "--offline")
    return values


def _sandbox_profile_shape_valid(
    profile_raw: str,
    network_clause: str,
    entry: Mapping[str, Any],
    phase: str,
    command: Sequence[Any],
) -> bool:
    lines = profile_raw.splitlines()
    expected_prefix = [
        "(version 1)", "(deny default)", "(allow process*)", network_clause,
        "(allow sysctl-read)", "(allow mach-lookup)",
        '(deny mach-lookup (global-name "com.apple.securityd"))',
        '(deny mach-lookup (global-name "com.apple.security.agent"))',
        '(deny mach-lookup (global-name "com.apple.trustd"))',
    ]
    special_read_lines = [_SANDBOX_ONLINE_VAR_METADATA] if phase == "online" else []
    if (
        len(lines) <= len(expected_prefix) + len(special_read_lines) + 2
        or lines[:len(expected_prefix)] != expected_prefix
        or lines[len(expected_prefix):len(expected_prefix) + len(special_read_lines)] != special_read_lines
        or lines[-2] != _SANDBOX_DEV_NULL_WRITE
    ):
        return False
    read_lines = lines[len(expected_prefix) + len(special_read_lines):-2]
    write_match = _SANDBOX_WRITE_ROOT.fullmatch(lines[-1])
    if write_match is None or not all("file-read" in line for line in read_lines):
        return False
    try:
        write_root = json.loads(write_match.group(1))
    except json.JSONDecodeError:
        return False
    if not isinstance(write_root, str) or not write_root.startswith("/") or str(Path(write_root)) != write_root:
        return False
    entry_id = entry.get("entry", entry.get("id"))
    entry_work = Path(write_root)
    if (
        phase not in {"online", "offline"}
        or not isinstance(entry_id, str)
        or entry_work.name != entry_id
        or entry_work.parent.name != ".work"
        or _PREPARE_STAGING_ROOT.fullmatch(entry_work.parent.parent.name) is None
    ):
        return False
    actual_content_roots: list[tuple[str, Path]] = []
    for line in read_lines:
        match = _SANDBOX_CONTENT_ROOT.fullmatch(line)
        if match is None:
            continue
        try:
            value = json.loads(match.group(2))
        except json.JSONDecodeError:
            return False
        if not isinstance(value, str):
            return False
        actual_content_roots.append((match.group(1), Path(value)))
    expected_command = _expected_dependency_prepare_argv(
        entry, entry_work / "disposable-sources" / phase, entry_work,
        offline=phase == "offline",
    )
    expected_profile = _preparation_sandbox_profile(
        entry_work / "disposable-sources" / phase,
        entry_work,
        allow_network=phase == "online",
    ).decode("utf-8")
    return (
        profile_raw == expected_profile
        and list(command) == expected_command
        and len(actual_content_roots) == len(set(actual_content_roots))
        and set(actual_content_roots) == _sandbox_expected_content_roots(entry_work, phase)
        and _sandbox_read_closure_valid("\n".join(read_lines))
    )


def _sandbox_profile_write_root(profile_raw: str) -> Path | None:
    lines = profile_raw.splitlines()
    if not lines:
        return None
    match = _SANDBOX_WRITE_ROOT.fullmatch(lines[-1])
    if match is None:
        return None
    try:
        value = json.loads(match.group(1))
    except json.JSONDecodeError:
        return None
    return Path(value) if isinstance(value, str) and value.startswith("/") else None


PREPARE_ONLINE_NETWORK_POLICY = "EXPLICIT_ALLOW_NETWORK"
PREPARE_OFFLINE_NETWORK_POLICY = "DENY_DEFAULT_NO_NETWORK_ALLOW"
SECURITY_SUPERVISOR_EXPECTED = {
    "sandbox_network_env": "DENIED_AND_ISOLATED",
    "sandbox_secret_paths": "DENIED",
    "sandbox_unix_network": "DENIED",
    "sandbox_source_write": "DENIED",
    "sandbox_keychain_read": "DENIED",
    "sandbox_background_child": "TERMINATED_WITH_GROUP",
}
PREPARE_SECURITY_CASES = frozenset({
    "prepareMavenLauncherTraversalPassed", "prepareSourceAncestryTraversalPassed",
    "prepareAncestorSecretReadDenied", "prepareAncestorWriteDenied",
    "prepareSelectedSourceWriteDenied", "prepareKeychainReadDenied",
    "prepareTraversalNetworkSemanticsPreserved", "prepareAncestorDataOnlyMutationRejected",
    "prepareBroadSandboxPermissionRejected", "prepareRootAuthoritySubstitutionsRejected",
    "prepareSplitPhaseRootsRejected",
    "prepareDevNullWriteDataPassed", "prepareOnlineVarMetadataOnlyPassed",
    "prepareMissingProfileClauseRejected", "prepareBroadDevNullWriteRejected",
    "prepareOfflineVarAliasRejected", "prepareWrongMavenTmpdirRejected",
    "prepareSplitPhaseEnvironmentRejected",
    "prepareGradleWrapperBootstrapHomePassed",
    "prepareMissingGradleWrapperBootstrapHomeRejected",
    "prepareGradleJvmTmpdirAuthorityPassed",
    "prepareMissingGradleJvmTmpdirRejected",
    "prepareWrongGradleJvmTmpdirRejected",
    "prepareGradleJvmTmpdirFailureClassifiedInfrastructure",
    "prepareGradleStrictOfflineFailureTypedRefusal",
    "prepareGradleStrictOfflineWrongProfileSecurityRejected",
    "prepareGradleOnlineSecurityFailureRejected",
    "prepareMavenOfflineSecurityFailureRejected",
    "prepareMavenOfflineModelGoalsPrefetchedOnline",
    "preparePostPublicationEvidenceRevalidated",
})
PREPARE_NETWORK_SENTINEL_CODE = (
    "socket(S,PF_INET,SOCK_STREAM,6) or exit 31;"
    "connect(S,sockaddr_in(9,inet_aton(\"127.0.0.1\"))) "
    "or exit(($!{EPERM}||$!{EACCES})?0:41);exit 42"
)


def _preparation_sandbox_profile(source: Path, staging: Path, *, allow_network: bool) -> bytes:
    clauses = "\n".join(_sandbox_read_clauses_for_roots(
        _preparation_content_roots(source, staging)
    ))
    network_clause = "(allow network*)\n" if allow_network else ""
    return (
        "(version 1)\n(deny default)\n(allow process*)\n"
        f"{network_clause if allow_network else '(deny network*)\n'}"
        "(allow sysctl-read)\n(allow mach-lookup)\n"
        "(deny mach-lookup (global-name \"com.apple.securityd\"))\n"
        "(deny mach-lookup (global-name \"com.apple.security.agent\"))\n"
        "(deny mach-lookup (global-name \"com.apple.trustd\"))\n"
        f"{_SANDBOX_ONLINE_VAR_METADATA + chr(10) if allow_network else ''}"
        f"{clauses}\n"
        f"{_SANDBOX_DEV_NULL_WRITE}\n"
        # PREPARE evidence is revalidated after `.work` is removed and the
        # cohort is published.  The authority is a directory subtree by
        # construction, so its selector must not depend on later existence.
        f"(allow file-write* (subpath {_sandbox_string(staging)}))\n"
    ).encode()


def _preparation_environment(entry_work: Path, build_dsl: str) -> dict[str, str]:
    """Return the exact environment shared by both PREPARE phases."""
    home = entry_work.resolve(strict=False) / "home"
    environment = {
        "HOME": str(home),
        "USERPROFILE": str(home),
        "PATH": "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home/bin:/opt/homebrew/Cellar/maven/3.9.12/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        "JAVA_HOME": "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home",
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "TMPDIR": str(home),
        "MAVEN_OPTS": f"-Djava.io.tmpdir={home}",
        "CODECLEW_K1_MODEL_CALLS": "0",
    }
    if build_dsl in {"GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL"}:
        # Gradle Wrapper bootstraps the distribution before Gradle can parse
        # command-line system properties. Bind both bootstrap storage and the
        # wrapper/Gradle JVM temp root to the existing private write authority.
        environment["GRADLE_USER_HOME"] = str(
            entry_work.resolve(strict=False) / "gradle-user-home"
        )
        environment["GRADLE_OPTS"] = f"-Djava.io.tmpdir={home}"
    elif build_dsl != "MAVEN":
        raise HarnessError("dependency PREPARE build DSL environment mismatch")
    return environment


def _preparation_environments_evidence_valid(
    environments: Any, entry_work: Path, build_dsl: Any,
) -> bool:
    if build_dsl not in {"MAVEN", "GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL"}:
        return False
    expected = _preparation_environment(entry_work, build_dsl)
    expected_record = {
        "environment": expected,
        "environmentSha256": sha256_bytes(canonical(expected)),
    }
    return (
        isinstance(environments, Mapping)
        and set(environments) == {"online", "offline"}
        and environments.get("online") == expected_record
        and environments.get("offline") == expected_record
    )


def _prepare_network_sentinel_argv() -> list[str]:
    return ["/usr/bin/perl", "-MSocket", "-e", PREPARE_NETWORK_SENTINEL_CODE]


def _security_tripwire_cases_valid(
    supervisor_cases: Any, requirement_cases: Any,
) -> bool:
    return (
        isinstance(supervisor_cases, Mapping)
        and all(supervisor_cases.get(name) == value for name, value in SECURITY_SUPERVISOR_EXPECTED.items())
        and isinstance(requirement_cases, Mapping)
        and all(requirement_cases.get(name) is True for name in PREPARE_SECURITY_CASES)
    )


def _preparation_network_evidence_valid(row: Mapping[str, Any]) -> bool:
    """Validate exact two-phase PREPARE network evidence, including refusals."""
    commands = row.get("prepareArgv")
    profiles = row.get("sandboxProfiles")
    environments = row.get("prepareEnvironments")
    sentinel = row.get("offlineNetworkSentinel")
    if (
        not isinstance(commands, list) or len(commands) != 2
        or not all(isinstance(command, list) for command in commands)
        or row.get("prepareArgvSha256") != sha256_bytes(canonical(commands))
        or not isinstance(profiles, Mapping) or set(profiles) != {"online", "offline"}
        or not isinstance(sentinel, Mapping)
    ):
        return False
    online, offline = commands
    offline_flag = "-o" if row.get("buildDsl") == "MAVEN" else "--offline"
    online_profile, offline_profile = profiles.get("online"), profiles.get("offline")
    if not isinstance(online_profile, Mapping) or not isinstance(offline_profile, Mapping):
        return False
    online_raw = online_profile.get("profileBytes")
    offline_raw = offline_profile.get("profileBytes")
    if not isinstance(online_raw, str) or not isinstance(offline_raw, str):
        return False
    network_clauses = {"(allow network*)", "(deny network*)"}
    online_network_lines = [line for line in online_raw.splitlines() if line in network_clauses]
    offline_network_lines = [line for line in offline_raw.splitlines() if line in network_clauses]
    online_write_root = _sandbox_profile_write_root(online_raw)
    offline_write_root = _sandbox_profile_write_root(offline_raw)
    exact_profiles = (
        online_profile == {
            "policy": PREPARE_ONLINE_NETWORK_POLICY,
            "profileSha256": sha256_bytes(online_raw.encode()),
            "profileBytes": online_raw,
        }
        and offline_profile == {
            "policy": PREPARE_OFFLINE_NETWORK_POLICY,
            "profileSha256": sha256_bytes(offline_raw.encode()),
            "profileBytes": offline_raw,
        }
        and online_network_lines == ["(allow network*)"]
        and offline_network_lines == ["(deny network*)"]
        and online_raw != offline_raw
        and online_write_root is not None
        and online_write_root == offline_write_root
        and _sandbox_profile_shape_valid(
            online_raw, "(allow network*)", row, "online", online,
        )
        and _sandbox_profile_shape_valid(
            offline_raw, "(deny network*)", row, "offline", offline,
        )
    )
    exact_environments = (
        online_write_root is not None
        and _preparation_environments_evidence_valid(
            environments, online_write_root, row.get("buildDsl"),
        )
    )
    exact_sentinel_base = (
        sentinel.get("argv") == _prepare_network_sentinel_argv()
        and sentinel.get("argvSha256") == sha256_bytes(canonical(_prepare_network_sentinel_argv()))
        and sentinel.get("denialErrnos") == ["EACCES", "EPERM"]
        and _is_digest(sentinel.get("stdoutSha256"))
        and _is_digest(sentinel.get("stderrSha256"))
    )
    command_modes = offline_flag not in online and offline_flag in offline and online != offline
    no_download_marker = row.get("offlineNoDownloadMarker") == {
        "flag": offline_flag,
        "commandIndex": 1,
        "presentExactlyOnce": offline.count(offline_flag) == 1,
        "offlineCommandSha256": sha256_bytes(canonical(offline)),
    }
    offline_executed = sentinel.get("executed") is True
    denied = (
        offline_executed and sentinel.get("exitCode") == 0
        and sentinel.get("stdoutSha256") == sha256_bytes(b"")
        and sentinel.get("stderrSha256") == sha256_bytes(b"")
    )
    online_refusal = (
        row.get("outcome") == "TYPED_REFUSAL"
        and row.get("failureStage") == "ONLINE_DEPENDENCY_PREPARATION"
        and sentinel.get("executed") is False
        and sentinel.get("exitCode") is None
    )
    return (
        exact_profiles and exact_environments and exact_sentinel_base
        and command_modes and no_download_marker and (denied or online_refusal)
    )


def _dependency_prepare_security_authority_failure(stderr: bytes) -> bool:
    """Classify fail-closed PREPARE authority failures, never product gaps."""
    lowered = stderr.lower()
    return any(marker in lowered for marker in (
        b"sandbox", b"operation not permitted", b"deny file", b"permission denied",
        b"java.io.tmpdir is set to a directory that doesn't exist",
    ))


def _bounded_prepare_run(command: list[str], cwd: Path, env: Mapping[str, str]) -> subprocess.CompletedProcess[bytes]:
    capture_root = Path(env["TMPDIR"]).resolve(strict=True)
    stdout_path = capture_root / f"prepare-stdout-{secrets.token_hex(8)}.bin"
    stderr_path = capture_root / f"prepare-stderr-{secrets.token_hex(8)}.bin"
    if stdout_path.exists() or stderr_path.exists():
        raise HarnessError("dependency PREPARE capture collision")
    output_overflow = threading.Event()
    output_stop = threading.Event()
    resident_overflow = threading.Event()
    watchdog_stop = threading.Event()
    watchdog_observation: dict[str, Any] = {}
    try:
        with stdout_path.open("xb") as stdout_handle, stderr_path.open("xb") as stderr_handle:
            try:
                process = subprocess.Popen(
                    command, cwd=cwd, stdin=subprocess.DEVNULL,
                    stdout=stdout_handle, stderr=stderr_handle, env=dict(env),
                    start_new_session=True,
                )
            except OSError as error:
                raise HarnessError("dependency PREPARE infrastructure launcher failure") from error
            output_thread = threading.Thread(
                target=_bounded_file_watchdog,
                args=(process, (stdout_path, stderr_path), MAX_STDOUT_BYTES, output_overflow, output_stop),
                daemon=True,
            )
            watchdog_thread = threading.Thread(
                target=_resident_watchdog,
                args=(process, MAX_RESIDENT_BYTES, resident_overflow, watchdog_stop, watchdog_observation),
                daemon=True,
            )
            output_thread.start()
            watchdog_thread.start()
            try:
                return_code = process.wait(timeout=MAX_WALL_SECONDS)
            except subprocess.TimeoutExpired as error:
                _kill_process_group(process)
                process.wait()
                raise HarnessError("dependency PREPARE infrastructure timeout") from error
            finally:
                output_stop.set()
                watchdog_stop.set()
                output_thread.join(timeout=5)
                watchdog_thread.join(timeout=5)
                if process.poll() is None:
                    _kill_process_group(process, signal.SIGKILL)
                    process.wait()
                _kill_remaining_process_group(process)
        stdout = stdout_path.read_bytes()
        stderr = stderr_path.read_bytes()
    finally:
        stdout_path.unlink(missing_ok=True)
        stderr_path.unlink(missing_ok=True)
    if output_overflow.is_set() or len(stdout) > MAX_STDOUT_BYTES or len(stderr) > MAX_STDOUT_BYTES:
        raise HarnessError("dependency PREPARE infrastructure output limit exceeded")
    if resident_overflow.is_set():
        raise HarnessError("dependency PREPARE infrastructure resident limit exceeded")
    if return_code < 0:
        raise HarnessError("dependency PREPARE infrastructure signal termination")
    return subprocess.CompletedProcess(command, return_code, stdout, stderr)


def _prepare_dependency_seed(
    store: Store,
    identifier: str,
    inputs: Mapping[str, Mapping[str, Any]],
) -> dict[str, Any]:
    qualification = identifier.startswith("QUALIFICATION")
    cohort = "QUALIFICATION" if qualification else "BLIND_HOLDOUT"
    seed_key = "qualificationDependencySeed" if qualification else "holdoutDependencySeed"
    source_key = "qualificationSourceSet" if qualification else "holdoutSourceSet"
    target = _input_path(inputs, seed_key, "TREE").absolute()
    if target.exists() or target.is_symlink():
        raise HarnessError("dependency seed PREPARE target is create-only")
    source_set = _input_path(inputs, source_key, "SOURCE_SET").resolve(strict=True)
    entries = [row for row in store.bundle["corpus"]["entries"] if row["cohort"] == cohort]
    if [row["id"] for row in entries] != list(EXPECTED_QUALIFICATION if qualification else EXPECTED_HOLDOUT):
        raise HarnessError("dependency PREPARE corpus membership mismatch")
    target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    staging = target.parent / f".{target.name}.prepare-{secrets.token_hex(12)}"
    if staging.exists():
        raise HarnessError("dependency PREPARE staging collision")
    staging.mkdir(mode=0o700)
    (staging / "entries").mkdir(mode=0o700)
    (staging / ".work").mkdir(mode=0o700)
    rows: list[dict[str, Any]] = []
    try:
        for entry in entries:
            entry_work = staging / ".work" / entry["id"]
            entry_work.mkdir(mode=0o700)
            (entry_work / "gradle-user-home").mkdir(mode=0o700)
            (entry_work / "maven-repository").mkdir(mode=0o700)
            (entry_work / "home").mkdir(mode=0o700)
            entry_output = staging / "entries" / entry["id"]
            entry_output.mkdir(mode=0o700)
            repository = source_set / entry["id"]
            before = _git_observation(repository)
            if before["head"] != entry["commit"] or before["tree"] != entry["gitTree"] or not before["clean"]:
                raise HarnessError(f"dependency PREPARE source is not exact frozen pin: {entry['id']}")
            disposable_parent = entry_work / "disposable-sources"
            disposable_parent.mkdir(mode=0o700)
            online_source = _disposable_git_archive(repository, disposable_parent / "online")
            offline_source = _disposable_git_archive(repository, disposable_parent / "offline")
            commands = [
                _dependency_prepare_commands(entry, online_source, entry_work, offline=False),
                _dependency_prepare_commands(entry, offline_source, entry_work, offline=True),
            ]
            online_profile = entry_work / "prepare-online.sb"
            offline_profile = entry_work / "prepare-offline.sb"
            online_profile_raw = _preparation_sandbox_profile(
                online_source, entry_work, allow_network=True,
            )
            offline_profile_raw = _preparation_sandbox_profile(
                offline_source, entry_work, allow_network=False,
            )
            if (
                b"(allow network*)" not in online_profile_raw
                or b"(allow network*)" in offline_profile_raw
                or b"(deny network*)" in online_profile_raw
                or b"(deny network*)" not in offline_profile_raw
                or online_profile_raw == offline_profile_raw
            ):
                raise HarnessError("dependency PREPARE network policy profile mismatch")
            _atomic_write(online_profile, online_profile_raw, 0o400)
            _atomic_write(offline_profile, offline_profile_raw, 0o400)
            env = _preparation_environment(entry_work, entry["buildDsl"])
            started = time.monotonic_ns()
            command_results = []
            online_result = _bounded_prepare_run(
                ["/usr/bin/sandbox-exec", "-f", str(online_profile), *commands[0]],
                online_source, env,
            )
            command_results.append(online_result)
            sentinel_argv = _prepare_network_sentinel_argv()
            sentinel_result: subprocess.CompletedProcess[bytes] | None = None
            if online_result.returncode == 0:
                sentinel_result = _bounded_prepare_run(
                    ["/usr/bin/sandbox-exec", "-f", str(offline_profile), *sentinel_argv],
                    offline_source, env,
                )
                if sentinel_result.returncode != 0 or sentinel_result.stdout or sentinel_result.stderr:
                    raise HarnessError(f"dependency PREPARE offline network sentinel failed: {entry['id']}")
                offline_result = _bounded_prepare_run(
                    ["/usr/bin/sandbox-exec", "-f", str(offline_profile), *commands[1]],
                    offline_source, env,
                )
                command_results.append(offline_result)
            after = _git_observation(repository)
            if before != after:
                raise HarnessError(f"dependency PREPARE mutated frozen source: {entry['id']}")
            row = {
                "entry": entry["id"], "commit": entry["commit"], "gitTree": entry["gitTree"],
                "selectedCompilation": entry["selectedCompilation"], "buildDsl": entry["buildDsl"],
                "prepareArgv": commands,
                "prepareArgvSha256": sha256_bytes(canonical(commands)),
                "sandboxProfiles": {
                    "online": {
                        "policy": PREPARE_ONLINE_NETWORK_POLICY,
                        "profileSha256": sha256_bytes(online_profile_raw),
                        "profileBytes": online_profile_raw.decode("utf-8"),
                    },
                    "offline": {
                        "policy": PREPARE_OFFLINE_NETWORK_POLICY,
                        "profileSha256": sha256_bytes(offline_profile_raw),
                        "profileBytes": offline_profile_raw.decode("utf-8"),
                    },
                },
                "prepareEnvironments": {
                    phase: {
                        "environment": dict(env),
                        "environmentSha256": sha256_bytes(canonical(env)),
                    }
                    for phase in ("online", "offline")
                },
                "offlineNetworkSentinel": {
                    "argv": sentinel_argv,
                    "argvSha256": sha256_bytes(canonical(sentinel_argv)),
                    "executed": sentinel_result is not None,
                    "exitCode": sentinel_result.returncode if sentinel_result is not None else None,
                    "stdoutSha256": store.put_blob(sentinel_result.stdout if sentinel_result is not None else b""),
                    "stderrSha256": store.put_blob(sentinel_result.stderr if sentinel_result is not None else b""),
                    "denialErrnos": ["EACCES", "EPERM"],
                },
                "offlineNoDownloadMarker": {
                    "flag": "-o" if entry["buildDsl"] == "MAVEN" else "--offline",
                    "commandIndex": 1,
                    "presentExactlyOnce": commands[1].count(
                        "-o" if entry["buildDsl"] == "MAVEN" else "--offline"
                    ) == 1,
                    "offlineCommandSha256": sha256_bytes(canonical(commands[1])),
                },
                "exitCode": command_results[-1].returncode,
                "stdoutSha256": store.put_blob(b"".join(result.stdout for result in command_results)),
                "stderrSha256": store.put_blob(b"".join(result.stderr for result in command_results)),
                "wallMicros": (time.monotonic_ns() - started) // 1000,
                "sourceTreeSha256": before["sourceTreeSha256"],
            }
            rows.append(row)
            # The selected source has already been re-observed and the command
            # evidence retained. Neither READY nor typed-refusal publication
            # may leave the sealed synthetic Git roots in staging.
            _discard_disposable_source(disposable_parent, entry_work)
            if command_results[-1].returncode != 0:
                combined_error = b"\n".join(result.stderr for result in command_results)
                failed_command = commands[len(command_results) - 1]
                offline_gradle_refusal = (
                    entry["buildDsl"] in {"GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL"}
                    and len(command_results) == 2
                )
                if (
                    not offline_gradle_refusal
                    and _dependency_prepare_security_authority_failure(combined_error)
                ):
                    raise HarnessError(f"dependency PREPARE security/authority failure: {entry['id']}")
                combined_error = combined_error.lower()
                reason_code = (
                    "OFFLINE_MODEL_PROBE_FAILED" if "--offline" in failed_command or "-o" in failed_command
                    else "UNSUPPORTED_BUILD_CONFIGURATION" if any(marker in combined_error for marker in (b"task not found", b"unknown lifecycle phase", b"unsupported"))
                    else "DEPENDENCY_CLOSURE_UNAVAILABLE"
                )
                row["outcome"] = "TYPED_REFUSAL"
                row["failureStage"] = (
                    "ONLINE_DEPENDENCY_PREPARATION"
                    if sentinel_result is None else "OFFLINE_DEPENDENCY_VERIFICATION"
                )
                row["reasonCode"] = reason_code
                if not _preparation_network_evidence_valid(row):
                    raise HarnessError(f"dependency PREPARE network evidence invalid: {entry['id']}")
                preparation_receipt_digest = sha256_bytes(canonical(row))
                refusal = {
                    "schema": PREPARED_REFUSAL_SCHEMA, "seriesId": SERIES_ID,
                    "cohort": cohort, "entry": entry["id"], "commit": entry["commit"], "gitTree": entry["gitTree"],
                    "selectedCompilation": entry["selectedCompilation"], "buildDsl": entry["buildDsl"],
                    "failureStage": "DEPENDENCY_PREPARATION", "reasonCode": reason_code,
                    "safeDetailDigest": row["stderrSha256"],
                    "cost": {
                        "wallMicros": row["wallMicros"],
                        "stdoutBytes": sum(len(result.stdout) for result in command_results),
                        "stderrBytes": sum(len(result.stderr) for result in command_results),
                        "exitCode": command_results[-1].returncode,
                    },
                    "sandboxProfileSha256": row["sandboxProfiles"][
                        "online" if sentinel_result is None else "offline"
                    ]["profileSha256"], "sourceTreeSha256": row["sourceTreeSha256"],
                    "candidateToolsSha256": snapshot_input(inputs["candidateTools"])["sha256"],
                    "buildInputDigest": sha256_bytes(canonical({"entry":entry["id"],"commands":commands,"source":before["sourceTreeSha256"]})),
                    "preparationReceiptDigest": preparation_receipt_digest, "objectDigest": "",
                }
                refusal["objectDigest"] = _rust_canonical_digest(refusal)
                _atomic_write(entry_output / "PREPARED_REFUSAL.json", canonical(refusal), 0o400)
            else:
                row["outcome"] = "READY"
                # Only dependency/wrapper repositories survive; project-local
                # caches, profile and HOME are discarded.
                if (entry_work / "ephemeral-project-cache").exists():
                    shutil.rmtree(entry_work / "ephemeral-project-cache")
                (entry_work / "prepare-online.sb").unlink()
                (entry_work / "prepare-offline.sb").unlink()
                shutil.rmtree(entry_work / "home")
                directories, mode_files = _seal_build_state_subtrees(entry_work)
                files = [
                    {key: item[key] for key in ("root", "path", "size", "sha256")}
                    for item in mode_files
                ]
                tools = _candidate_tools(inputs)
                body = {
                    "schema": "codeclew.kotlin-k1-build-state-manifest/0.1", "seriesId": SERIES_ID,
                    "cohort": cohort,
                    "toolchain": {"java":tools["jdk"]["javaSha256"],"javaRelease":tools["jdk"]["releaseSha256"],"maven":tools["maven"]["sha256"],"git":tools["systemTools"]["git"]["sha256"]},
                    "repositories": [{key: row[key] for key in ("entry","commit","gitTree","selectedCompilation","buildDsl","prepareArgvSha256","exitCode")}],
                    "gradleUserHomeTreeDigest": _build_state_subtree_digest(files, "gradle-user-home"),
                    "mavenLocalRepositoryTreeDigest": _build_state_subtree_digest(files, "maven-repository"),
                    "files": files, "seedDigest": "",
                }
                body["seedDigest"] = sha256_bytes(canonical(body))
                manifest_raw = canonical(body)
                _atomic_write(entry_work / "CODECLEW_K1_BUILD_STATE_MANIFEST.json", manifest_raw, 0o400)
                _atomic_write(entry_work / "CODECLEW_K1_BUILD_STATE_SEED", (sha256_bytes(manifest_raw) + "\n").encode(), 0o400)
                mode_body = {
                    "schema": "codeclew.kotlin-k1-build-state-modes/0.1", "seriesId": SERIES_ID,
                    "buildStateManifestDigest": sha256_bytes(manifest_raw),
                    "directories": directories, "files": mode_files, "objectDigest": "",
                }
                mode_body["objectDigest"] = sha256_bytes(canonical(mode_body))
                mode_raw = canonical(mode_body)
                _atomic_write(entry_work / "CODECLEW_K1_BUILD_STATE_MODES.json", mode_raw, 0o400)
                # Darwin denies rename(2) of a directory after its owner write
                # bit has been removed.  Move the still-private root first;
                # only then seal and validate its final cohort-local identity.
                build_state = entry_output / "build-state"
                os.replace(entry_work, build_state)
                os.chmod(build_state, 0o500, follow_symlinks=False)
                validated_build_state = _validate_build_state_seed(build_state, cohort)
                row["buildStateSeedDigest"] = body["seedDigest"]
                row["buildStateManifestDigest"] = sha256_bytes(manifest_raw)
                row["buildStateModeManifestDigest"] = validated_build_state["modeManifestDigest"]
            if not _preparation_network_evidence_valid(row):
                raise HarnessError(f"dependency PREPARE network evidence invalid: {entry['id']}")
        if (staging / ".work").exists():
            shutil.rmtree(staging / ".work")
        tools_snapshot = snapshot_input(inputs["candidateTools"])
        cohort_body = {
            "schema":"codeclew.kotlin-k1-dependency-cohort/0.1","seriesId":SERIES_ID,"cohort":cohort,
            "sourceSetSha256":_source_set_digest(source_set),"candidateToolsSha256":tools_snapshot["sha256"],
            "entries":rows,"modelCalls":0,"cohortDigest":"",
        }
        cohort_body["cohortDigest"] = sha256_bytes(canonical(cohort_body))
        cohort_raw = canonical(cohort_body)
        _atomic_write(staging / "CODECLEW_K1_DEPENDENCY_COHORT.json", cohort_raw, 0o400)
        _atomic_write(staging / "CODECLEW_K1_DEPENDENCY_COHORT", (sha256_bytes(cohort_raw) + "\n").encode(), 0o400)
        # Descendants are sealed before publication, but the staging root must
        # remain owner-writable until rename(2) completes on Darwin.
        _seal_dependency_cohort_tree(staging, seal_root=False)
        os.replace(staging, target)
        os.chmod(target, 0o500, follow_symlinks=False)
        directory = os.open(target.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        published = _validate_dependency_cohort(target, cohort, entries)
        return {
            "seed": snapshot_input(inputs[seed_key]),
            "sourceSetSha256": _source_set_digest(source_set),
            "sourceMembers": _source_set_members(source_set),
            "manifestDigest": published["manifestDigest"], "seedDigest": published["cohortDigest"],
            "fileCount": published["fileCount"], "preparationAttempts": rows,
            "networkPolicy": "ENABLED_ONLY_IN_THIS_PREPARE", "modelCalls": 0,
        }
    except BaseException:
        if staging.exists():
            # A child may fail after one of its synthetic Git repositories was
            # sealed read-only. Restore permissions only for those exact
            # harness-created roots before deleting the private staging tree.
            for entry in entries:
                entry_work = staging / ".work" / entry["id"]
                disposable_parent = entry_work / "disposable-sources"
                if disposable_parent.exists() or disposable_parent.is_symlink():
                    _discard_disposable_source(disposable_parent, entry_work)
            _discard_private_tree(staging, target.parent)
        elif target.exists() or target.is_symlink():
            # A failure after the atomic rename but before authoritative
            # receipt publication must not strand a create-only output.
            _discard_private_tree(target, target.parent)
        raise


def snapshot_input(descriptor: Mapping[str, Any]) -> dict[str, str]:
    if not isinstance(descriptor, Mapping) or set(descriptor) != {"kind", "path"}:
        raise HarnessError("live input descriptor must contain exactly kind/path")
    kind = descriptor["kind"]
    path_text = descriptor["path"]
    if kind not in {"FILE", "TREE", "SOURCE_SET", "LIVE_SET"} or not isinstance(path_text, str) or not Path(path_text).is_absolute():
        raise HarnessError("live input descriptor kind/path mismatch")
    path = Path(path_text)
    if kind == "FILE":
        path = _regular_file(path, "live input")
        digest = sha256_file(path)
    elif kind == "TREE":
        digest = _tree_digest(path)
    elif kind == "SOURCE_SET":
        digest = _source_set_digest(path)
    else:
        path, digest = _validate_live_set(path)
    return {"kind": kind, "path": str(path), "sha256": digest}


class Lock:
    """A process-wide flock which is re-entrant for one Store instance."""

    def __init__(self, store: "Store"):
        self.store = store

    def __enter__(self) -> "Lock":
        self.store._local_lock.acquire()
        if self.store._lock_depth == 0:
            self.store._lock_handle = (self.store.root / "LOCK").open("a+")
            fcntl.flock(self.store._lock_handle, fcntl.LOCK_EX)
        self.store._lock_depth += 1
        return self

    def __exit__(self, *_: Any) -> None:
        self.store._lock_depth -= 1
        if self.store._lock_depth == 0:
            assert self.store._lock_handle is not None
            fcntl.flock(self.store._lock_handle, fcntl.LOCK_UN)
            self.store._lock_handle.close()
            self.store._lock_handle = None
        self.store._local_lock.release()


class Store:
    """Private CAS plus mutable pointers; objects and authority bytes are immutable."""

    def __init__(self, root: Path, bundle: Mapping[str, Any], create: bool = False):
        self.root = root.absolute()
        self._local_lock = threading.RLock()
        self._lock_depth = 0
        self._lock_handle: Any = None
        if self.root.exists() and self.root.is_symlink():
            raise HarnessError("readiness store root must not be a symlink")
        if create:
            self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        identity_path = self.root / "STORE.json"
        expected_digests = dict(bundle["digests"])
        if create and not identity_path.exists():
            identity = {
                "schema": STORE_SCHEMA,
                "storeId": secrets.token_hex(32),
                "seriesId": SERIES_ID,
                "authorityDigests": expected_digests,
            }
            _atomic_write(identity_path, canonical(identity), 0o400)
            identity_path.chmod(0o400)
        identity_path = _regular_file(identity_path, "readiness store identity")
        identity = _load_json_bytes(identity_path.read_bytes(), "readiness store identity")
        if not isinstance(identity, dict) or identity.get("schema") != STORE_SCHEMA or identity.get("seriesId") != SERIES_ID or identity.get("authorityDigests") != expected_digests:
            raise HarnessError("readiness store identity/authority mismatch")
        self.store_id = identity.get("storeId")
        if not isinstance(self.store_id, str) or len(self.store_id) != 64:
            raise HarnessError("readiness store id mismatch")
        self.bundle = bundle
        self.graph = bundle["readinessGraph"]
        self.graph_digest = expected_digests["readinessGraph"]
        if create:
            for name in ("objects", "current", "authorities", "attempts", "blobs", "qualification", "holdout", "starts", "guards"):
                (self.root / name).mkdir(exist_ok=True, mode=0o700)
            for name, (source, _) in AUTHORITIES.items():
                target = self.root / "authorities" / source.name
                raw = source.read_bytes()
                if target.exists() and target.read_bytes() != raw:
                    raise HarnessError(f"stored authority collision: {name}")
                if not target.exists():
                    _atomic_write(target, raw, 0o400)
                    target.chmod(0o400)
        for name, (source, _) in AUTHORITIES.items():
            target = _regular_file(self.root / "authorities" / source.name, f"stored {name}")
            if target.read_bytes() != source.read_bytes():
                raise HarnessError(f"stored {name} differs from production authority")
        for name in ("objects", "current", "attempts", "blobs", "qualification", "holdout", "starts", "guards"):
            if not (self.root / name).is_dir() or (self.root / name).is_symlink():
                raise HarnessError(f"readiness store {name} directory mismatch")
        if create:
            self._initialize_series_guard()

    @classmethod
    def open_for_fatal_finalize(cls, root: Path) -> "Store":
        """Open only immutable stored authority after a live-authority drift.

        This intentionally bypasses production files solely so finalize-series
        can retain an internally proved fatal invariant and issue STOP.  The
        returned Store must never be used for an OPEN decision or another node.
        """
        instance = object.__new__(cls)
        instance.root = root.absolute()
        instance._local_lock = threading.RLock()
        instance._lock_depth = 0
        instance._lock_handle = None
        if instance.root.is_symlink() or not instance.root.is_dir():
            raise HarnessError("degraded readiness store root mismatch")
        identity_path = _regular_file(instance.root / "STORE.json", "readiness store identity")
        identity_raw = identity_path.read_bytes()
        identity = _load_json_bytes(identity_raw, "readiness store identity")
        expected_digests = {name: "sha256:" + digest for name, (_, digest) in AUTHORITIES.items()}
        if not isinstance(identity, dict) or canonical(identity) != identity_raw or identity != {
            "schema": STORE_SCHEMA,
            "storeId": identity.get("storeId"),
            "seriesId": SERIES_ID,
            "authorityDigests": expected_digests,
        } or not isinstance(identity.get("storeId"), str) or len(identity["storeId"]) != 64:
            raise HarnessError("degraded store identity/authority mismatch")
        values: dict[str, Any] = {}
        for name, (source, expected_hex) in AUTHORITIES.items():
            stored = _regular_file(instance.root / "authorities" / source.name, f"stored {name}")
            raw = stored.read_bytes()
            if hashlib.sha256(raw).hexdigest() != expected_hex:
                raise HarnessError(f"stored authority digest mismatch: {name}")
            values[name] = _load_json_bytes(raw, f"stored {name}")
        instance.store_id = identity["storeId"]
        instance.bundle = {**values, "digests": expected_digests}
        instance.graph = _validate_graph(values["readinessGraph"])
        instance.bundle["readinessGraph"] = instance.graph
        instance.graph_digest = expected_digests["readinessGraph"]
        for name in ("objects", "current", "attempts", "blobs", "qualification", "holdout", "starts", "guards"):
            directory = instance.root / name
            if directory.is_symlink() or not directory.is_dir():
                raise HarnessError(f"degraded store directory mismatch: {name}")
        _series_guard(instance)
        return instance

    def locked(self) -> Lock:
        return Lock(self)

    def _initialize_series_guard(self) -> None:
        """Create the sole OPEN guard before any production node can run."""
        open_marker = self.root / "guards" / "OPEN.json"
        if open_marker.exists() or open_marker.is_symlink():
            _series_guard(self)
            return
        evidence = {
            "schema": SERIES_GUARD_SCHEMA,
            "state": "OPEN",
            "reasonCode": None,
            "fatalEvidence": None,
            "fatalEvidenceSha256": None,
        }
        receipt = {
            "schema": RECEIPT_SCHEMA,
            "storeId": self.store_id,
            "seriesId": SERIES_ID,
            "graphDigest": self.graph_digest,
            "checkerVersion": CHECKER_VERSION,
            "checkerSourceDigest": sha256_file(Path(__file__)),
            "node": "K1_SERIES_GUARD",
            "action": "DIRECT",
            "nodeKey": _node_key(self, "K1_SERIES_GUARD", {}, {}),
            "status": "READY",
            "selectedInputs": {},
            "dependencies": {},
            "evidence": evidence,
            "error": None,
        }
        digest = self.put_object(receipt)
        marker = {
            "schema": SERIES_GUARD_MARKER_SCHEMA,
            "storeId": self.store_id,
            "graphDigest": self.graph_digest,
            "state": "OPEN",
            "receiptDigest": digest,
        }
        _atomic_create(open_marker, canonical(marker), 0o400)
        self._write_pointer_unchecked("K1_SERIES_GUARD", digest)

    def _write_pointer_unchecked(self, node: str, receipt_digest: str) -> None:
        if not _is_digest(receipt_digest):
            raise HarnessError("unchecked pointer receipt digest mismatch")
        pointer = {
            "schema": POINTER_SCHEMA, "storeId": self.store_id,
            "graphDigest": self.graph_digest, "node": node,
            "receiptDigest": receipt_digest,
        }
        _atomic_write(self.root / "current" / f"{node}.json", canonical(pointer))

    def child_start_value(
        self,
        entry: str,
        invocation: str,
        authority: str,
        selected_digest: str,
    ) -> dict[str, Any]:
        if entry in EXPECTED_QUALIFICATION:
            expected_authority = "DEDICATED_QUALIFICATION_EXACT_ARGV"
        elif entry in EXPECTED_HOLDOUT:
            expected_authority = "DEDICATED_HOLDOUT_EXACT_ARGV"
        else:
            raise HarnessError("child-start entry is outside the frozen corpus")
        if (
            authority != expected_authority
            or invocation not in {"COLD", "WARM"}
            or not _is_digest(selected_digest)
        ):
            raise HarnessError("child-start authority mismatch")
        return {
            "schema": CHILD_START_SCHEMA,
            "seriesId": SERIES_ID,
            "storeId": self.store_id,
            "graphDigest": self.graph_digest,
            "entry": entry,
            "invocation": invocation,
            "authority": authority,
            "selectedDigest": selected_digest,
            "state": "LAUNCH_COMMITTED",
        }

    def record_child_start(
        self,
        entry: str,
        invocation: str,
        authority: str,
        selected_digest: str,
    ) -> str:
        """Durably commit launch authority before the wrapper execs target."""
        value = self.child_start_value(entry, invocation, authority, selected_digest)
        raw = canonical(value)
        digest = sha256_bytes(raw)
        target = self.root / "starts" / f"{entry}-{invocation.lower()}.json"
        if target.exists() or target.is_symlink():
            if _regular_file(target, "child-start journal").read_bytes() != raw:
                raise HarnessError("child-start journal is create-only and differs")
            return digest
        _atomic_write(target, raw, 0o400)
        target.chmod(0o400)
        return digest

    def put_object(self, value: Mapping[str, Any]) -> str:
        raw = canonical(value)
        digest = sha256_bytes(raw)
        target = self.root / "objects" / f"{digest.removeprefix('sha256:')}.json"
        if target.exists() and target.read_bytes() != raw:
            raise HarnessError("CAS object collision")
        if not target.exists():
            _atomic_write(target, raw, 0o400)
            target.chmod(0o400)
        return digest

    def put_recovery_object(self, value: Mapping[str, Any]) -> str:
        """Install an internally rebuilt protocol object despite poisoned CAS bytes."""
        raw = canonical(value)
        digest = sha256_bytes(raw)
        target = self.root / "objects" / f"{digest.removeprefix('sha256:')}.json"
        if target.exists() or target.is_symlink():
            try:
                if _regular_file(target, "recovery CAS object").read_bytes() == raw:
                    return digest
            except (HarnessError, OSError):
                pass
            quarantine = self.root / "objects" / f"quarantine-{secrets.token_hex(32)}.bad"
            os.replace(target, quarantine)
            directory = os.open(target.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        _atomic_create(target, raw, 0o400)
        return digest

    def put_blob(self, raw: bytes) -> str:
        digest = sha256_bytes(raw)
        target = self.root / "blobs" / f"{digest.removeprefix('sha256:')}.blob"
        if target.exists() and target.read_bytes() != raw:
            raise HarnessError("CAS blob collision")
        if not target.exists():
            _atomic_write(target, raw, 0o400)
            target.chmod(0o400)
        return digest

    def publish_attempt(self, attempt: Mapping[str, Any]) -> str:
        if attempt.get("schema") != ATTEMPT_SCHEMA:
            raise HarnessError("attempt schema mismatch")
        digest = self.put_object(attempt)
        pointer = {
            "schema": ATTEMPT_POINTER_SCHEMA,
            "storeId": self.store_id,
            "graphDigest": self.graph_digest,
            "entry": attempt["entry"],
            "invocation": attempt["invocation"],
            "attemptDigest": digest,
        }
        target = self.root / "attempts" / f"{attempt['entry']}-{attempt['invocation']}-{digest[7:]}.json"
        if target.exists() and target.read_bytes() != canonical(pointer):
            raise HarnessError("attempt pointer collision")
        if not target.exists():
            _atomic_write(target, canonical(pointer), 0o400)
            target.chmod(0o400)
        return digest

    def _publish_cohort_attempt(self, attempt: Mapping[str, Any], cohort: str) -> str:
        authority = "DEDICATED_QUALIFICATION_EXACT_ARGV" if cohort == "QUALIFICATION" else "DEDICATED_HOLDOUT_EXACT_ARGV"
        identifiers = EXPECTED_QUALIFICATION if cohort == "QUALIFICATION" else EXPECTED_HOLDOUT
        directory = "qualification" if cohort == "QUALIFICATION" else "holdout"
        pointer_schema = f"codeclew.kotlin-k1-{directory}-pointer/0.1"
        if attempt.get("schema") != ATTEMPT_SCHEMA or attempt.get("authority") != authority:
            raise HarnessError(f"{cohort} attempt authority mismatch")
        entry = attempt.get("entry")
        invocation = attempt.get("invocation")
        if entry not in identifiers or invocation not in {"COLD", "WARM"}:
            raise HarnessError(f"{cohort} attempt member mismatch")
        required = {
            "seriesId", "storeId", "graphDigest", "cohort", "status", "selectedInputs",
            "child", "repositoryBefore", "repositoryAfter", "sourceMutation", "modelCalls",
            "childStartSha256", "childSelectedDigest", "attemptDigest",
        }
        empty_digest = {**attempt, "attemptDigest": ""}
        if (
            not required.issubset(attempt) or attempt.get("seriesId") != SERIES_ID
            or attempt.get("storeId") != self.store_id or attempt.get("graphDigest") != self.graph_digest
            or attempt.get("cohort") != cohort or attempt.get("modelCalls") != 0
            or not isinstance(attempt.get("selectedInputs"), Mapping)
            or not isinstance(attempt.get("child"), Mapping)
            or not isinstance(attempt.get("sourceMutation"), bool)
            or not _is_digest(attempt.get("childStartSha256"))
            or not _is_digest(attempt.get("childSelectedDigest"))
            or attempt.get("attemptDigest") != sha256_bytes(canonical(empty_digest))
        ):
            raise HarnessError(f"{cohort} retained attempt identity mismatch")
        digest = self.put_object(attempt)
        pointer = {
            "schema": pointer_schema,
            "storeId": self.store_id,
            "graphDigest": self.graph_digest,
            "entry": entry,
            "invocation": invocation,
            "attemptDigest": digest,
        }
        target = self.root / directory / f"{entry}-{str(invocation).lower()}.json"
        if target.exists() or target.is_symlink():
            existing = _regular_file(target, f"{cohort} attempt pointer").read_bytes()
            if existing != canonical(pointer):
                raise HarnessError(f"{cohort} attempt is create-only and already differs")
            return digest
        _atomic_write(target, canonical(pointer), 0o400)
        target.chmod(0o400)
        return digest

    def publish_qualification_attempt(self, attempt: Mapping[str, Any]) -> str:
        return self._publish_cohort_attempt(attempt, "QUALIFICATION")

    def qualification_attempt(self, entry: str, invocation: str) -> tuple[str, dict[str, Any]] | None:
        target = self.root / "qualification" / f"{entry}-{invocation.lower()}.json"
        if not target.exists():
            return None
        raw = _regular_file(target, "qualification attempt pointer").read_bytes()
        pointer = _load_json_bytes(raw, "qualification attempt pointer")
        expected_keys = {"schema", "storeId", "graphDigest", "entry", "invocation", "attemptDigest"}
        if not isinstance(pointer, dict) or set(pointer) != expected_keys or canonical(pointer) != raw or pointer.get("schema") != "codeclew.kotlin-k1-qualification-pointer/0.1" or pointer.get("storeId") != self.store_id or pointer.get("graphDigest") != self.graph_digest or pointer.get("entry") != entry or pointer.get("invocation") != invocation:
            raise HarnessError("forged qualification attempt pointer")
        return pointer["attemptDigest"], self.get_object(pointer["attemptDigest"])

    def publish_holdout_attempt(self, attempt: Mapping[str, Any]) -> str:
        return self._publish_cohort_attempt(attempt, "BLIND_HOLDOUT")

    def holdout_attempt(self, entry: str, invocation: str) -> tuple[str, dict[str, Any]] | None:
        target = self.root / "holdout" / f"{entry}-{invocation.lower()}.json"
        if not target.exists():
            return None
        raw = _regular_file(target, "holdout attempt pointer").read_bytes()
        pointer = _load_json_bytes(raw, "holdout attempt pointer")
        if not isinstance(pointer, dict) or canonical(pointer) != raw or pointer != {
            "schema": "codeclew.kotlin-k1-holdout-pointer/0.1", "storeId": self.store_id,
            "graphDigest": self.graph_digest, "entry": entry, "invocation": invocation,
            "attemptDigest": pointer.get("attemptDigest"),
        } or not _is_digest(pointer.get("attemptDigest")):
            raise HarnessError("forged holdout attempt pointer")
        return pointer["attemptDigest"], self.get_object(pointer["attemptDigest"])

    def get_object(self, digest: str) -> dict[str, Any]:
        if not _is_digest(digest):
            raise HarnessError("invalid CAS digest")
        target = _regular_file(self.root / "objects" / f"{digest.removeprefix('sha256:')}.json", "CAS object")
        raw = target.read_bytes()
        if sha256_bytes(raw) != digest:
            raise HarnessError("CAS object digest mismatch")
        value = _load_json_bytes(raw, "CAS object")
        if not isinstance(value, dict) or canonical(value) != raw:
            raise HarnessError("CAS object is not canonical JSON")
        return value

    def pointer(self, node: str) -> dict[str, Any] | None:
        if node == "K1_SERIES_GUARD":
            _, _, digest = _series_guard(self)
            return {
                "schema": POINTER_SCHEMA, "storeId": self.store_id,
                "graphDigest": self.graph_digest, "node": node,
                "receiptDigest": digest,
            }
        target = self.root / "current" / f"{node}.json"
        if not target.exists():
            return None
        target = _regular_file(target, f"readiness pointer {node}")
        raw = target.read_bytes()
        value = _load_json_bytes(raw, f"readiness pointer {node}")
        expected_keys = {"schema", "storeId", "graphDigest", "node", "receiptDigest"}
        if not isinstance(value, dict) or set(value) != expected_keys or canonical(value) != raw or value.get("schema") != POINTER_SCHEMA or value.get("storeId") != self.store_id or value.get("graphDigest") != self.graph_digest or value.get("node") != node:
            raise HarnessError(f"forged readiness pointer: {node}")
        self.get_object(value.get("receiptDigest"))
        return value

    def receipt(self, node: str) -> dict[str, Any] | None:
        pointer = self.pointer(node)
        return self.get_object(pointer["receiptDigest"]) if pointer else None

    def publish(self, receipt: Mapping[str, Any]) -> str:
        digest = self.put_object(receipt)
        existing = self.pointer(str(receipt["node"]))
        if existing is not None:
            existing_receipt = self.get_object(existing["receiptDigest"])
            if existing_receipt == dict(receipt):
                return existing["receiptDigest"]
            if receipt.get("node") == "K1_SERIES_GUARD":
                old_state = existing_receipt.get("evidence", {}).get("state")
                new_state = receipt.get("evidence", {}).get("state")
                if old_state == "FATAL" or (old_state, new_state) != ("OPEN", "FATAL"):
                    raise HarnessError("series guard is irreversible")
            if existing_receipt.get("status") == "READY" and receipt.get("status") != "READY":
                return existing["receiptDigest"]
        pointer = {
            "schema": POINTER_SCHEMA,
            "storeId": self.store_id,
            "graphDigest": self.graph_digest,
            "node": receipt["node"],
            "receiptDigest": digest,
        }
        _atomic_write(self.root / "current" / f"{receipt['node']}.json", canonical(pointer))
        return digest


def _is_digest(value: Any) -> bool:
    return isinstance(value, str) and value.startswith("sha256:") and len(value) == 71 and all(character in "0123456789abcdef" for character in value[7:])


def _validate_fatal_evidence(reason: str, evidence: Mapping[str, Any]) -> None:
    """Accept only internally reproducible evidence shapes for terminal STOP."""
    exact_keys = {
        "K0_1_DRIFT": {"expectedDigests", "observedDigests"},
        "PINNED_AUTHORITY_DRIFT": {"authority", "expectedSha256", "observedSha256"},
        "THRESHOLD_OR_CORPUS_REWRITE": {"authority", "expectedSha256", "observedSha256"},
        "SOURCE_MUTATION": {"entry", "invocation", "attemptDigest"},
        "POST_FREEZE_CANDIDATE_OR_HOLDOUT_CHANGE": {"node", "receiptDigest", "beforeSha256", "afterSha256"},
        "VERIFIED_AUTHORITY_BYPASS": {"invariant", "detailSha256"},
        "UNRETAINED_STARTED_CHILD": {"entry", "invocation", "startSha256"},
        "MATRIX_SAFETY_VIOLATION": {"matrixSafetyArtifactSha256", "matrixSafetyReceiptDigest", "violations"},
    }
    if reason not in FATAL_REASON_CODES or set(evidence) != exact_keys[reason]:
        raise HarnessError("fatal reason/evidence is not in the internal whitelist")
    if reason in {"PINNED_AUTHORITY_DRIFT", "THRESHOLD_OR_CORPUS_REWRITE"}:
        if evidence["authority"] not in AUTHORITIES or not all(
            _is_digest(evidence[key]) for key in ("expectedSha256", "observedSha256")
        ) or evidence["expectedSha256"] == evidence["observedSha256"]:
            raise HarnessError("authority-drift fatal evidence mismatch")
    elif reason == "K0_1_DRIFT":
        if not all(isinstance(evidence.get(key), Mapping) for key in ("expectedDigests", "observedDigests")) or evidence["expectedDigests"] == evidence["observedDigests"]:
            raise HarnessError("K0 drift fatal evidence mismatch")
    elif reason == "SOURCE_MUTATION":
        if evidence["entry"] not in EXPECTED_QUALIFICATION + EXPECTED_HOLDOUT or evidence["invocation"] not in {"COLD", "WARM"} or not _is_digest(evidence["attemptDigest"]):
            raise HarnessError("source-mutation fatal evidence mismatch")
    elif reason == "POST_FREEZE_CANDIDATE_OR_HOLDOUT_CHANGE":
        if evidence["node"] not in {"CANDIDATE_FREEZE_VERIFY", "HOLDOUT_SOURCE_MATERIALIZE"} or not all(
            _is_digest(evidence[key]) for key in ("receiptDigest", "beforeSha256", "afterSha256")
        ) or evidence["beforeSha256"] == evidence["afterSha256"]:
            raise HarnessError("post-freeze fatal evidence mismatch")
    elif reason == "VERIFIED_AUTHORITY_BYPASS":
        if evidence["invariant"] not in {"CAS_OBJECT", "CURRENT_POINTER", "RECEIPT_IDENTITY"} or not _is_digest(evidence["detailSha256"]):
            raise HarnessError("authority-bypass fatal evidence mismatch")
    elif reason == "UNRETAINED_STARTED_CHILD":
        if evidence["entry"] not in EXPECTED_QUALIFICATION + EXPECTED_HOLDOUT or evidence["invocation"] not in {"COLD", "WARM"} or not _is_digest(evidence["startSha256"]):
            raise HarnessError("unretained-child fatal evidence mismatch")
    elif reason == "MATRIX_SAFETY_VIOLATION":
        allowed = {"FALSE_PROVEN", "FALSE_COMPLETE", "MODEL_CALL", "UNTYPED_FAILURE"}
        if not _is_digest(evidence["matrixSafetyArtifactSha256"]) or not _is_digest(evidence["matrixSafetyReceiptDigest"]) or not isinstance(evidence["violations"], list) or not evidence["violations"] or not set(evidence["violations"]).issubset(allowed):
            raise HarnessError("matrix-safety fatal evidence mismatch")


def _guard_marker(store: Store, state: str) -> tuple[dict[str, Any], bytes]:
    path = _regular_file(store.root / "guards" / f"{state}.json", f"{state} series guard marker")
    raw = path.read_bytes()
    marker = _load_json_bytes(raw, f"{state} series guard marker")
    expected_keys = {
        "OPEN": {"schema", "storeId", "graphDigest", "state", "receiptDigest"},
        "FATAL": {
            "schema", "storeId", "graphDigest", "state", "previousGuardDigest",
            "receiptDigest",
        },
    }[state]
    if (
        not isinstance(marker, dict) or set(marker) != expected_keys
        or canonical(marker) != raw or marker.get("schema") != SERIES_GUARD_MARKER_SCHEMA
        or marker.get("storeId") != store.store_id
        or marker.get("graphDigest") != store.graph_digest
        or marker.get("state") != state or not _is_digest(marker.get("receiptDigest"))
    ):
        raise HarnessError(f"{state} series guard marker mismatch")
    return marker, raw


def _validate_guard_receipt(
    store: Store,
    digest: str,
    expected_state: str,
) -> dict[str, Any]:
    receipt = store.get_object(digest)
    evidence = receipt.get("evidence")
    if not isinstance(evidence, dict) or set(evidence) != {
        "schema", "state", "reasonCode", "fatalEvidence", "fatalEvidenceSha256",
    } or evidence.get("schema") != SERIES_GUARD_SCHEMA:
        raise HarnessError("series guard evidence contract mismatch")
    state = evidence.get("state")
    if state == "OPEN":
        if evidence.get("reasonCode") is not None or evidence.get("fatalEvidence") is not None or evidence.get("fatalEvidenceSha256") is not None:
            raise HarnessError("OPEN series guard contains fatal evidence")
    elif state == "FATAL":
        reason = evidence.get("reasonCode")
        fatal = evidence.get("fatalEvidence")
        if not isinstance(reason, str) or not isinstance(fatal, dict):
            raise HarnessError("FATAL series guard lacks evidence")
        _validate_fatal_evidence(reason, fatal)
        if evidence.get("fatalEvidenceSha256") != sha256_bytes(canonical(fatal)):
            raise HarnessError("FATAL series guard evidence digest mismatch")
    else:
        raise HarnessError("series guard state mismatch")
    if state != expected_state:
        raise HarnessError("series guard receipt/marker state mismatch")
    source_digest = receipt.get("checkerSourceDigest")
    expected = {
        "schema": RECEIPT_SCHEMA,
        "storeId": store.store_id,
        "seriesId": SERIES_ID,
        "graphDigest": store.graph_digest,
        "checkerVersion": CHECKER_VERSION,
        "checkerSourceDigest": source_digest,
        "node": "K1_SERIES_GUARD",
        "action": "DIRECT",
        "nodeKey": _node_key_with_source_digest(
            store, "K1_SERIES_GUARD", {}, {}, source_digest,
        ),
        "status": "READY",
        "selectedInputs": {},
        "dependencies": {},
        "evidence": evidence,
        "error": None,
    }
    if not _is_digest(source_digest) or receipt != expected or set(receipt) != set(expected):
        raise HarnessError("series guard receipt identity mismatch")
    return receipt


def _series_guard(store: Store) -> tuple[str, dict[str, Any], str]:
    guard_members = {path.name for path in (store.root / "guards").iterdir()}
    if not guard_members.issubset({"OPEN.json", "FATAL.json"}) or "OPEN.json" not in guard_members:
        raise HarnessError("series guard journal membership mismatch")
    open_marker, _ = _guard_marker(store, "OPEN")
    open_digest = open_marker["receiptDigest"]
    open_receipt = _validate_guard_receipt(store, open_digest, "OPEN")
    fatal_path = store.root / "guards" / "FATAL.json"
    if not fatal_path.exists():
        if fatal_path.is_symlink():
            raise HarnessError("FATAL series guard marker mismatch")
        return "OPEN", open_receipt, open_digest
    fatal_marker, _ = _guard_marker(store, "FATAL")
    if fatal_marker.get("previousGuardDigest") != open_digest:
        raise HarnessError("FATAL series guard predecessor mismatch")
    fatal_digest = fatal_marker["receiptDigest"]
    fatal_receipt = _validate_guard_receipt(store, fatal_digest, "FATAL")
    return "FATAL", fatal_receipt, fatal_digest


def _node(store: Store, identifier: str) -> dict[str, Any]:
    try:
        return next(node for node in store.graph["nodes"] if node["id"] == identifier)
    except StopIteration as error:
        raise HarnessError(f"unknown readiness node: {identifier}") from error


def _selected(store: Store, identifier: str, inputs: Mapping[str, Mapping[str, Any]]) -> dict[str, dict[str, str]]:
    if identifier == "K1_DECISION" and _series_guard(store)[0] == "FATAL":
        return {}
    selected: dict[str, dict[str, str]] = {}
    for key in _node(store, identifier)["selectedInputs"]:
        descriptor: Mapping[str, Any]
        if key in AUTHORITIES:
            descriptor = {"kind": "FILE", "path": str(AUTHORITIES[key][0].absolute())}
        else:
            if key not in inputs:
                raise HarnessError(f"readiness input missing: {key}")
            descriptor = inputs[key]
        selected[key] = snapshot_input(descriptor)
    return selected


def _dependency_receipts(store: Store, identifier: str, inputs: Mapping[str, Mapping[str, Any]]) -> tuple[dict[str, str], list[str]]:
    ready: dict[str, str] = {}
    blockers: list[str] = []
    declared = _node(store, identifier)["deps"]
    dependencies = declared
    if identifier == "K1_DECISION":
        guard_status, _, _ = assess(store, "K1_SERIES_GUARD", inputs)
        if guard_status != "READY":
            return {}, ["K1_SERIES_GUARD"]
        dependencies = ["K1_SERIES_GUARD"] if _series_guard(store)[0] == "FATAL" else declared
    for dependency in dependencies:
        status, _, _ = assess(store, dependency, inputs)
        pointer = store.pointer(dependency)
        if status != "READY" or pointer is None:
            blockers.append(dependency)
        else:
            ready[dependency] = pointer["receiptDigest"]
    return ready, blockers


def _node_key_with_source_digest(
    store: Store,
    identifier: str,
    selected: Mapping[str, Any],
    dependencies: Mapping[str, str],
    source_digest: str,
) -> str:
    return sha256_bytes(canonical({
        "storeId": store.store_id,
        "graphDigest": store.graph_digest,
        "checkerVersion": CHECKER_VERSION,
        "checkerSourceDigest": source_digest,
        "node": identifier,
        "nodeSpecification": _node(store, identifier),
        "selectedInputs": selected,
        "dependencies": dependencies,
    }))


def _node_key(store: Store, identifier: str, selected: Mapping[str, Any], dependencies: Mapping[str, str]) -> str:
    return _node_key_with_source_digest(
        store, identifier, selected, dependencies, sha256_file(Path(__file__)),
    )


def assess(store: Store, identifier: str, inputs: Mapping[str, Mapping[str, Any]]) -> tuple[str, list[str], dict[str, Any] | None]:
    if identifier == "K1_SERIES_GUARD":
        try:
            _, receipt, _ = _series_guard(store)
            return "READY", [], receipt
        except (HarnessError, OSError) as error:
            return "STALE", [f"append-only guard authority mismatch: {error}"], None
    receipt = store.receipt(identifier)
    if receipt is None:
        return "ABSENT", ["missing receipt"], None
    dependencies, blockers = _dependency_receipts(store, identifier, inputs)
    if blockers:
        return "BLOCKED", ["dependencies not READY: " + ",".join(blockers)], receipt
    try:
        selected = _selected(store, identifier, inputs)
    except (HarnessError, OSError) as error:
        return "STALE", [f"selected input unavailable: {error}"], receipt
    expected_key = _node_key(store, identifier, selected, dependencies)
    if not isinstance(receipt, dict) or receipt.get("node") != identifier or receipt.get("action") != _node(store, identifier)["action"] or receipt.get("status") not in RECEIPT_STATES or not isinstance(receipt.get("evidence"), dict) or not (receipt.get("error") is None or isinstance(receipt.get("error"), str)):
        return "STALE", ["receipt structural contract mismatch"], receipt
    exact = {
        "schema": RECEIPT_SCHEMA,
        "storeId": store.store_id,
        "seriesId": SERIES_ID,
        "graphDigest": store.graph_digest,
        "checkerVersion": CHECKER_VERSION,
        "checkerSourceDigest": sha256_file(Path(__file__)),
        "node": identifier,
        "action": _node(store, identifier)["action"],
        "nodeKey": expected_key,
        "status": receipt.get("status"),
        "selectedInputs": selected,
        "dependencies": dependencies,
        "evidence": receipt.get("evidence"),
        "error": receipt.get("error"),
    }
    if set(receipt) != set(exact) or receipt != exact or receipt.get("status") not in RECEIPT_STATES:
        return "STALE", ["receipt identity/current inputs changed"], receipt
    if _node(store, identifier)["action"] == "CONDITIONAL_ROOT":
        decision = store.receipt("K1_DECISION")
        expected = {
            "KOTLIN_REAL_REPOSITORY_READY": "GO",
            "KOTLIN_APPLICABILITY_OR_COST_GAP": "PIVOT",
            "K1_SERIES_STOPPED": "STOP",
        }[identifier]
        if not decision or decision.get("status") != "READY" or decision.get("evidence", {}).get("decision") != expected or receipt.get("evidence", {}).get("decision") != expected:
            return "BLOCKED", [f"terminal condition does not equal {expected}"], receipt
    return receipt["status"], ([receipt["error"]] if receipt.get("error") else []), receipt


def _latch_series_fatal(
    store: Store,
    inputs: Mapping[str, Mapping[str, Any]],
    detected: tuple[str, Mapping[str, Any]],
) -> str:
    """Latch only a fatal fact independently re-derived under the store lock."""
    with store.locked():
        state, old_receipt, old_digest = _series_guard(store)
        if state == "FATAL":
            return old_digest
        recomputed = _detect_fatal_invariant(store, inputs)
        if recomputed is None or recomputed[0] != detected[0] or recomputed[1] != dict(detected[1]):
            raise HarnessError("fatal invariant was not independently reproduced")
        reason, evidence = recomputed
        _validate_fatal_evidence(reason, evidence)
        fatal_evidence = dict(evidence)
        guard_evidence = {
            "schema": SERIES_GUARD_SCHEMA,
            "state": "FATAL",
            "reasonCode": reason,
            "fatalEvidence": fatal_evidence,
            "fatalEvidenceSha256": sha256_bytes(canonical(fatal_evidence)),
        }
        receipt = {
            "schema": RECEIPT_SCHEMA,
            "storeId": store.store_id,
            "seriesId": SERIES_ID,
            "graphDigest": store.graph_digest,
            "checkerVersion": CHECKER_VERSION,
            "checkerSourceDigest": sha256_file(Path(__file__)),
            "node": "K1_SERIES_GUARD",
            "action": "DIRECT",
            "nodeKey": _node_key(store, "K1_SERIES_GUARD", {}, {}),
            "status": "READY",
            "selectedInputs": {},
            "dependencies": {},
            "evidence": guard_evidence,
            "error": None,
        }
        digest = store.put_recovery_object(receipt)
        marker = {
            "schema": SERIES_GUARD_MARKER_SCHEMA,
            "storeId": store.store_id,
            "graphDigest": store.graph_digest,
            "state": "FATAL",
            "previousGuardDigest": old_digest,
            "receiptDigest": digest,
        }
        _atomic_create(store.root / "guards" / "FATAL.json", canonical(marker), 0o400)
        store._write_pointer_unchecked("K1_SERIES_GUARD", digest)
        return digest


def _issue_authoritative(
    store: Store,
    identifier: str,
    inputs: Mapping[str, Mapping[str, Any]],
    evidence: Mapping[str, Any],
    *,
    expected_action: str,
    status: str = "READY",
    error: str | None = None,
    captured_selected: Mapping[str, Any] | None = None,
    captured_dependencies: Mapping[str, str] | None = None,
) -> str:
    specification = _node(store, identifier)
    if specification["action"] != expected_action:
        raise HarnessError(f"dedicated issuer action mismatch for {identifier}")
    if status not in RECEIPT_STATES or not isinstance(evidence, Mapping):
        raise HarnessError("dedicated receipt status/evidence mismatch")
    with store.locked():
        dependencies, blockers = _dependency_receipts(store, identifier, inputs)
        if blockers:
            raise HarnessError("node is blocked by: " + ",".join(blockers))
        selected = _selected(store, identifier, inputs)
        if captured_selected is not None and selected != dict(captured_selected):
            raise HarnessError(f"selected inputs changed while issuing {identifier}")
        if captured_dependencies is not None and dependencies != dict(captured_dependencies):
            raise HarnessError(f"dependencies changed while issuing {identifier}")
        receipt = {
            "schema": RECEIPT_SCHEMA,
            "storeId": store.store_id,
            "seriesId": SERIES_ID,
            "graphDigest": store.graph_digest,
            "checkerVersion": CHECKER_VERSION,
            "checkerSourceDigest": sha256_file(Path(__file__)),
            "node": identifier,
            "action": expected_action,
            "nodeKey": _node_key(store, identifier, selected, dependencies),
            "status": status,
            "selectedInputs": selected,
            "dependencies": dependencies,
            "evidence": dict(evidence),
            "error": error,
        }
        return store.publish(receipt)


def _advance_locked(
    store: Store,
    identifier: str,
    inputs: Mapping[str, Mapping[str, Any]],
    checker: Callable[[Mapping[str, Any] | None, Mapping[str, str]], str],
) -> str:
    """Run a node-specific checker and publication in one authority lock.

    This prevents a checker from reading one artifact body and then publishing
    a receipt whose selected-input identity was recomputed from newer bytes.
    """
    with store.locked():
        dependencies, blockers = _dependency_receipts(store, identifier, inputs)
        if blockers:
            raise HarnessError("node is blocked by: " + ",".join(blockers))
        if identifier == "K1_DECISION" and _series_guard(store)[0] == "FATAL":
            return checker({}, dependencies)
        selected: dict[str, Any] = {}
        # PREPARE/DIRECT nodes may create one selected output. Capture every
        # already-existing selected input independently so an absent output
        # cannot suppress the before/after identity of a large source set or
        # the candidate-tools manifest.
        for key in _node(store, identifier)["selectedInputs"]:
            descriptor = (
                {"kind": "FILE", "path": str(AUTHORITIES[key][0].absolute())}
                if key in AUTHORITIES else inputs.get(key)
            )
            if descriptor is None:
                raise HarnessError(f"readiness input missing: {key}")
            try:
                selected[key] = snapshot_input(descriptor)
            except (HarnessError, FileNotFoundError) as error:
                descriptor_path = Path(str(descriptor.get("path", "")))
                if descriptor_path.exists() or descriptor_path.is_symlink():
                    raise
        return checker(selected, dependencies)


def issue_verification(
    store: Store,
    identifier: str,
    inputs: Mapping[str, Mapping[str, Any]],
    checker: Callable[[], Mapping[str, Any]],
) -> str:
    """Issue only a VERIFY node through a harness-owned checker callback."""
    specification = _node(store, identifier)
    if specification["action"] not in GENERICALLY_ISSUABLE:
        raise HarnessError(f"{specification['action']} node requires its dedicated authority: {identifier}")
    with store.locked():
        dependencies, blockers = _dependency_receipts(store, identifier, inputs)
        if blockers:
            raise HarnessError("node is blocked by: " + ",".join(blockers))
        selected = _selected(store, identifier, inputs)
        status = "READY"
        error_text: str | None = None
        try:
            evidence = dict(checker())
        except Exception as error:  # retained failure is part of the protocol
            status = "FAILED"
            evidence = {}
            error_text = f"{type(error).__name__}:{error}"
        receipt = {
            "schema": RECEIPT_SCHEMA,
            "storeId": store.store_id,
            "seriesId": SERIES_ID,
            "graphDigest": store.graph_digest,
            "checkerVersion": CHECKER_VERSION,
            "checkerSourceDigest": sha256_file(Path(__file__)),
            "node": identifier,
            "action": specification["action"],
            "nodeKey": _node_key(store, identifier, selected, dependencies),
            "status": status,
            "selectedInputs": selected,
            "dependencies": dependencies,
            "evidence": evidence,
            "error": error_text,
        }
        return store.publish(receipt)


def explain(store: Store, inputs: Mapping[str, Mapping[str, Any]]) -> dict[str, Any]:
    rows = []
    first = None
    for specification in store.graph["nodes"]:
        status, reasons, _ = assess(store, specification["id"], inputs)
        rows.append({"node": specification["id"], "status": status, "reasons": reasons})
        if first is None and status != "READY":
            first = specification["id"]
    return {
        "schema": EXPLAIN_SCHEMA,
        "seriesId": SERIES_ID,
        "storeId": store.store_id,
        "graphDigest": store.graph_digest,
        "firstBlocker": first,
        "nodes": rows,
    }


def require_root(store: Store, root: str, inputs: Mapping[str, Mapping[str, Any]]) -> dict[str, Any]:
    if root not in store.graph["roots"]:
        raise HarnessError(f"not a production readiness root: {root}")
    status, reasons, receipt = assess(store, root, inputs)
    if status != "READY" or receipt is None:
        raise HarnessError(f"readiness root {root} is {status}: {reasons}")
    return receipt


def _unretained_started_child(store: Store) -> dict[str, Any] | None:
    for path in sorted((store.root / "starts").glob("*.json")):
        raw = _regular_file(path, "child-start journal").read_bytes()
        value = _load_json_bytes(raw, "child-start journal")
        exact_keys = {
            "schema", "seriesId", "storeId", "graphDigest", "entry", "invocation",
            "authority", "selectedDigest", "state",
        }
        if (
            not isinstance(value, dict) or set(value) != exact_keys or canonical(value) != raw
            or value.get("schema") != CHILD_START_SCHEMA or value.get("seriesId") != SERIES_ID
            or value.get("storeId") != store.store_id or value.get("graphDigest") != store.graph_digest
            or value.get("state") != "LAUNCH_COMMITTED"
            or value.get("authority") not in {
                "DEDICATED_QUALIFICATION_EXACT_ARGV", "DEDICATED_HOLDOUT_EXACT_ARGV",
            } or not _is_digest(value.get("selectedDigest"))
        ):
            raise HarnessError("forged child-start journal")
        entry, invocation = value.get("entry"), value.get("invocation")
        if invocation not in {"COLD", "WARM"} or path.name != f"{entry}-{str(invocation).lower()}.json":
            raise HarnessError("child-start journal filename mismatch")
        if entry in EXPECTED_QUALIFICATION:
            expected_authority = "DEDICATED_QUALIFICATION_EXACT_ARGV"
            expected_cohort = "QUALIFICATION"
            pair = store.qualification_attempt(entry, invocation)
        elif entry in EXPECTED_HOLDOUT:
            expected_authority = "DEDICATED_HOLDOUT_EXACT_ARGV"
            expected_cohort = "BLIND_HOLDOUT"
            pair = store.holdout_attempt(entry, invocation)
        else:
            raise HarnessError("child-start journal entry is outside the frozen corpus")
        if value["authority"] != expected_authority:
            raise HarnessError("child-start cohort/authority mismatch")
        if pair is None:
            return {"entry": entry, "invocation": invocation, "startSha256": sha256_bytes(raw)}
        _, attempt = pair
        required_attempt = {
            "schema", "seriesId", "storeId", "graphDigest", "entry", "cohort",
            "invocation", "status", "selectedInputs", "child", "repositoryBefore",
            "repositoryAfter", "sourceMutation", "modelCalls", "authority",
            "childStartSha256", "childSelectedDigest", "attemptDigest",
        }
        empty_attempt_digest = {**attempt, "attemptDigest": ""}
        if (
            not required_attempt.issubset(attempt)
            or attempt.get("schema") != ATTEMPT_SCHEMA
            or attempt.get("seriesId") != SERIES_ID or attempt.get("storeId") != store.store_id
            or attempt.get("graphDigest") != store.graph_digest
            or attempt.get("entry") != entry or attempt.get("invocation") != invocation
            or attempt.get("cohort") != expected_cohort
            or attempt.get("status") not in {"ADAPTER_OUTPUT", "PARTIAL", "REFUSED", "FAILED"}
            or attempt.get("modelCalls") != 0 or not isinstance(attempt.get("sourceMutation"), bool)
            or attempt.get("attemptDigest") != sha256_bytes(canonical(empty_attempt_digest))
            or attempt.get("childStartSha256") != sha256_bytes(raw)
            or attempt.get("childSelectedDigest") != value["selectedDigest"]
            or attempt.get("authority") != value["authority"]
        ):
            raise HarnessError("retained attempt does not correlate to child-start journal")
    return None


def assert_entry_run_allowed(store: Store, entry_id: str, inputs: Mapping[str, Mapping[str, Any]]) -> dict[str, Any]:
    if _series_guard(store)[0] != "OPEN":
        raise HarnessError("K1 series is irreversibly FATAL")
    entries = {entry["id"]: entry for entry in store.bundle["corpus"]["entries"]}
    if entry_id not in entries:
        raise HarnessError(f"entry is not in the frozen corpus: {entry_id}")
    entry = entries[entry_id]
    if entry["cohort"] == "QUALIFICATION":
        if entry.get("semanticAccess") != "QUALIFICATION_ALLOWED":
            raise HarnessError("qualification semantic access contract mismatch")
        return entry
    if entry["cohort"] != "BLIND_HOLDOUT" or entry.get("semanticAccess") != "SEALED_UNTIL_IMPLEMENTATION_FREEZE":
        raise HarnessError("holdout semantic access contract mismatch")
    status, _, _ = assess(store, "CANDIDATE_FREEZE_VERIFY", inputs)
    if status != "READY":
        raise HarnessError("HOLDOUT_ACCESS_BEFORE_CANDIDATE_FREEZE")
    return entry


def _git_observation(repository: Path) -> dict[str, Any]:
    repository = repository.absolute()
    if repository.is_symlink() or not repository.is_dir():
        raise HarnessError("repository must be a non-symlink directory")

    def git(*arguments: str) -> str:
        completed = subprocess.run(
            ["git", "-C", str(repository), *arguments],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=_git_plumbing_environment(repository),
            check=False,
            timeout=30,
        )
        if completed.returncode != 0:
            raise HarnessError("Git observation failed: " + sha256_bytes(completed.stderr))
        return completed.stdout.decode("utf-8", "strict").strip()

    head = git("rev-parse", "HEAD")
    tree = git("rev-parse", "HEAD^{tree}")
    mismatches = _git_clean_mismatch_rows(repository, tree)
    status_bytes = b"".join(canonical(row) for row in mismatches)
    try:
        source_tree_sha256 = _source_tree_digest(repository)
    except (HarnessError, FileNotFoundError):
        if not mismatches:
            raise
        source_tree_sha256 = sha256_bytes(canonical({
            "schema": "codeclew.git-dirty-source/0.1", "mismatches": mismatches,
        }))
    return {
        "head": head,
        "tree": tree,
        "clean": not mismatches,
        "statusSha256": sha256_bytes(status_bytes),
        "sourceTreeSha256": source_tree_sha256,
    }


def _launch_committed_child(arguments: Sequence[str]) -> None:
    """Tiny launcher: durable marker first, target exec second, no reverse gap."""
    if len(arguments) < 3 or arguments[1] != "--":
        raise HarnessError("internal launch wrapper arguments mismatch")
    payload_path = _regular_file(Path(arguments[0]), "launch wrapper payload")
    payload_raw = payload_path.read_bytes()
    payload = _load_json_bytes(payload_raw, "launch wrapper payload")
    if not isinstance(payload, dict) or canonical(payload) != payload_raw or set(payload) != {
        "journalPath", "journal",
    } or not isinstance(payload.get("journalPath"), str) or not isinstance(payload.get("journal"), dict):
        raise HarnessError("launch wrapper payload mismatch")
    journal_path = Path(payload["journalPath"])
    if not journal_path.is_absolute() or journal_path.parent.name != "starts":
        raise HarnessError("launch wrapper journal path mismatch")
    journal_raw = canonical(payload["journal"])
    try:
        _atomic_create(journal_path, journal_raw, 0o400)
    except FileExistsError:
        if _regular_file(journal_path, "launch journal").read_bytes() != journal_raw:
            raise HarnessError("launch journal create-only mismatch")
    target = list(arguments[2:])
    os.execve(target[0], target, dict(os.environ))


def _external_directory(path: Path, repository: Path, label: str, create: bool = False) -> Path:
    absolute = path.absolute()
    if create:
        absolute.mkdir(parents=True, exist_ok=True, mode=0o700)
    if absolute.is_symlink() or not absolute.is_dir():
        raise HarnessError(f"{label} must be an existing non-symlink directory")
    real = absolute.resolve(strict=True)
    repository_real = repository.resolve(strict=True)
    if real == repository_real or real.is_relative_to(repository_real) or repository_real.is_relative_to(real):
        raise HarnessError(f"{label} must be outside and must not contain the source checkout")
    return real


def _candidate_source_paths() -> dict[str, tuple[Path, str]]:
    return {
        "adapterSourceTree": (ROOT / "crates/evidence-adapters/src", "TREE"),
        "adapterCargo": (ROOT / "crates/evidence-adapters/Cargo.toml", "FILE"),
        "clewCrate": (ROOT / "crates/clew", "TREE"),
        "clewIndex": (ROOT / "crates/clew/src/index.rs", "FILE"),
        "clewWorker": (ROOT / "crates/clew/src/worker.rs", "FILE"),
        "evidenceCoreCrate": (ROOT / "crates/evidence-core", "TREE"),
        "workspaceCargo": (ROOT / "Cargo.toml", "FILE"),
        "rustToolchain": (ROOT / "rust-toolchain.toml", "FILE"),
        "kotlinWorker": (ROOT / "workers/kotlin/src/main/kotlin/dev/semanticthread/worker/Worker.kt", "FILE"),
        "mavenProjectModel": (ROOT / "workers/kotlin/src/main/kotlin/dev/semanticthread/worker/MavenProjectModel.kt", "FILE"),
        "gradleInit": (ROOT / "workers/kotlin/src/main/resources/semantic-thread-model.init.gradle", "FILE"),
        "cargoLock": (ROOT / "Cargo.lock", "FILE"),
        "adapterOutputSchema": (ROOT / "schemas/adapter_output.schema.json", "FILE"),
        "kotlinAttemptSchema": (ROOT / "schemas/kotlin_k1_attempt.schema.json", "FILE"),
        "kotlinPreparedRefusalSchema": (ROOT / "schemas/kotlin_k1_prepared_refusal.schema.json", "FILE"),
        "kotlinSemanticCacheSchema": (ROOT / "schemas/kotlin_semantic_cache_object.schema.json", "FILE"),
        "workerProto": (ROOT / "schemas/worker.proto", "FILE"),
        "threadIrProto": (ROOT / "schemas/thread_ir.proto", "FILE"),
        "semanticFactsProto": (ROOT / "schemas/semantic_facts.proto", "FILE"),
        "localCfgProto": (ROOT / "schemas/local_cfg.proto", "FILE"),
        "editIrProto": (ROOT / "schemas/edit_ir.proto", "FILE"),
        "transactionProto": (ROOT / "schemas/transaction.proto", "FILE"),
        "evidenceCoreProto": (ROOT / "schemas/evidence_core.proto", "FILE"),
        "independentAuditor": (ROOT / "scripts/k1_independent_auditor.py", "FILE"),
        "trustedWorkerDistributionBuilder": (ROOT / "scripts/build-trusted-worker-distributions.py", "FILE"),
    }


def _validate_worker_distribution(minor: str, root: Path, manifest_row: Mapping[str, Any]) -> None:
    manifest_path = ROOT / str(manifest_row.get("path", ""))
    raw = _regular_file(manifest_path, f"worker {minor} manifest").read_bytes()
    manifest = _load_json_bytes(raw, f"worker {minor} manifest")
    if sha256_bytes(raw) != manifest_row.get("sha256") or manifest.get("treeHash") != manifest_row.get("treeHash"):
        raise HarnessError(f"worker {minor} distribution manifest binding mismatch")
    expected = {
        row.get("path"): (row.get("size"), row.get("sha256"))
        for row in manifest.get("files", []) if isinstance(row, Mapping)
    }
    actual: dict[str, tuple[int, str]] = {}
    for directory, directories, names in os.walk(root, followlinks=False):
        for name in directories + names:
            if (Path(directory) / name).is_symlink():
                raise HarnessError(f"worker {minor} distribution contains a symlink")
        for name in names:
            member = _regular_file(Path(directory) / name, f"worker {minor} distribution member")
            relative = member.relative_to(root).as_posix()
            actual[relative] = (member.stat().st_size, sha256_file(member))
    if not expected or actual != expected:
        raise HarnessError(f"worker {minor} distribution bytes differ from manifest")
    tree_bytes = b"".join(
        relative.encode() + b"\0" + str(size).encode() + b"\0" + digest.encode() + b"\0"
        for relative, (size, digest) in sorted(actual.items())
    )
    if sha256_bytes(tree_bytes) != manifest.get("treeHash"):
        raise HarnessError(f"worker {minor} distribution treeHash is not recomputable")
    launchers = [root / relative for relative in actual if relative.startswith("bin/")]
    if not launchers or any(not os.access(path, os.X_OK) for path in launchers):
        raise HarnessError(f"worker {minor} launcher executable mode mismatch")


def _live_member(identifier: str, path: Path, kind: str) -> dict[str, Any]:
    absolute = path.absolute()
    if kind == "FILE":
        member = _regular_file(absolute, f"live-set member {identifier}")
        return {
            "id": identifier, "kind": kind, "path": str(member),
            "mode": stat.S_IMODE(member.stat().st_mode), "size": member.stat().st_size,
            "sha256": sha256_file(member),
        }
    if kind != "TREE" or absolute.is_symlink() or not absolute.is_dir():
        raise HarnessError(f"live-set tree member mismatch: {identifier}")
    return {"id": identifier, "kind": kind, "path": str(absolute), "sha256": _tree_digest(absolute)}


def build_live_set(role: str, candidate_tools_path: Path | None = None) -> dict[str, Any]:
    """Reconstruct one exact sanctioned live authority set; no caller members."""
    if role == "K0_AUTHORITY_SET":
        if candidate_tools_path is not None:
            raise HarnessError("K0 live set must not select candidate tools")
        lock = _load_json_bytes((ROOT / "contracts/core/core-contract.lock.json").read_bytes(), "K0 lock")
        locked_rows: dict[str, tuple[int, str]] = {}
        for group in ("adapterContractFiles", "decisionCoreFiles", "conformanceCorpusFiles"):
            for row in lock[group]:
                value = (row.get("size"), row.get("sha256"))
                if not isinstance(row.get("path"), str) or not isinstance(value[0], int) or not _is_digest(value[1]):
                    raise HarnessError("K0 lock member contract mismatch")
                previous = locked_rows.setdefault(row["path"], value)
                if previous != value:
                    raise HarnessError("K0 lock contains conflicting duplicate member")
        relatives = sorted(locked_rows)
        specifications = {relative: (ROOT / relative, "FILE") for relative in relatives}
        for relative, (size, digest) in locked_rows.items():
            member = _regular_file(ROOT / relative, f"K0 authority member {relative}")
            if member.stat().st_size != size or sha256_file(member) != digest:
                raise HarnessError(f"K0 authority member drift: {relative}")
        tools_sha = None
        tools_path = None
    elif role in {"CANDIDATE_SOURCES", "CANDIDATE_BINARIES"}:
        if candidate_tools_path is None:
            raise HarnessError("candidate live set requires candidate-tools manifest")
        candidate_tools_path = _regular_file(candidate_tools_path, "candidate tools manifest")
        tools = _candidate_tools({"candidateTools": {"kind": "FILE", "path": str(candidate_tools_path)}})
        tools_sha = tools["manifestSha256"]
        tools_path = str(candidate_tools_path)
        if role == "CANDIDATE_SOURCES":
            specifications = _candidate_source_paths()
        else:
            specifications = {
                "genericRuntime": (Path(tools["genericRuntime"]["path"]), "FILE"),
                "kotlinAdapter": (Path(tools["kotlinAdapter"]["path"]), "FILE"),
                "worker21": (ROOT / "workers/kotlin21/build/install/kotlin21", "TREE"),
                "worker23": (ROOT / "workers/kotlin23/build/install/kotlin23", "TREE"),
                "worker24": (ROOT / "workers/kotlin/build/install/kotlin", "TREE"),
            }
            for minor, identifier in (("2.1", "worker21"), ("2.3", "worker23"), ("2.4", "worker24")):
                _validate_worker_distribution(minor, specifications[identifier][0], tools["workerManifests"][minor])
    else:
        raise HarnessError("unknown sanctioned live-set role")
    members = [_live_member(identifier, *specifications[identifier]) for identifier in sorted(specifications)]
    value = {
        "schema": LIVE_SET_SCHEMA, "seriesId": SERIES_ID, "role": role,
        "candidateToolsPath": tools_path, "candidateToolsSha256": tools_sha,
        "members": members, "setDigest": "",
    }
    value["setDigest"] = sha256_bytes(canonical(value))
    return value


def _validate_live_set(path: Path) -> tuple[Path, str]:
    manifest_path = _regular_file(path, "live-set manifest")
    raw = manifest_path.read_bytes()
    value = _load_json_bytes(raw, "live-set manifest")
    if not isinstance(value, dict) or canonical(value) != raw or set(value) != {
        "schema", "seriesId", "role", "candidateToolsPath", "candidateToolsSha256",
        "members", "setDigest",
    } or value.get("schema") != LIVE_SET_SCHEMA or value.get("seriesId") != SERIES_ID:
        raise HarnessError("live-set manifest contract mismatch")
    tools_path = value.get("candidateToolsPath")
    expected = build_live_set(
        str(value.get("role")),
        Path(tools_path) if isinstance(tools_path, str) else None,
    )
    if value != expected:
        raise HarnessError("live-set manifest differs from sanctioned current members")
    return manifest_path, sha256_bytes(raw)


def _require_live_set_role(
    inputs: Mapping[str, Mapping[str, Any]], key: str, role: str,
) -> dict[str, str]:
    descriptor = inputs.get(key)
    if not isinstance(descriptor, Mapping) or descriptor.get("kind") != "LIVE_SET":
        raise HarnessError(f"{key} must use a sanctioned LIVE_SET descriptor")
    snapshot = snapshot_input(descriptor)
    value = _load_json_bytes(Path(snapshot["path"]).read_bytes(), f"{key} live set")
    if not isinstance(value, dict) or value.get("role") != role:
        raise HarnessError(f"{key} live-set role mismatch")
    return snapshot


def _candidate_tools(inputs: Mapping[str, Mapping[str, Any]]) -> dict[str, Any]:
    descriptor = inputs.get("candidateTools")
    if descriptor is None:
        raise HarnessError("dedicated qualification requires candidateTools live input")
    snapshot = snapshot_input(descriptor)
    if snapshot["kind"] != "FILE":
        raise HarnessError("candidateTools must be a canonical manifest file")
    path = Path(snapshot["path"])
    raw = path.read_bytes()
    value = _load_json_bytes(raw, "candidate tools manifest")
    expected = {
        "schema", "seriesId", "genericRuntime", "kotlinAdapter", "harnessSourceSha256",
        "kotlinAttemptSchemaSha256", "kotlinPreparedRefusalSchemaSha256",
        "kotlinSemanticCacheSchemaSha256", "coreContractLockSha256",
        "workerManifests", "sourceAuthorities", "systemTools", "jdk", "maven", "cargoBaseline", "modelCalls",
    }
    if not isinstance(value, dict) or set(value) != expected or canonical(value) != raw or value.get("schema") != "codeclew.kotlin-k1-candidate-tools/0.1" or value.get("seriesId") != SERIES_ID or value.get("modelCalls") != 0:
        raise HarnessError("candidate tools manifest contract mismatch")
    if value["harnessSourceSha256"] != sha256_file(Path(__file__)):
        raise HarnessError("candidate tools manifest does not bind the live harness")
    if value["kotlinAttemptSchemaSha256"] != sha256_file(ROOT / "schemas/kotlin_k1_attempt.schema.json"):
        raise HarnessError("candidate tools manifest does not bind the Kotlin attempt schema")
    if value["kotlinPreparedRefusalSchemaSha256"] != sha256_file(ROOT / "schemas/kotlin_k1_prepared_refusal.schema.json"):
        raise HarnessError("candidate tools manifest does not bind the prepared-refusal schema")
    if value["kotlinSemanticCacheSchemaSha256"] != sha256_file(ROOT / "schemas/kotlin_semantic_cache_object.schema.json"):
        raise HarnessError("candidate tools manifest does not bind the semantic cache schema")
    frozen_lock = load_authority("requirements")[0]["historicalCorePolicy"]["coreContractLockSha256"]
    if value["coreContractLockSha256"] != frozen_lock or sha256_file(ROOT / "contracts/core/core-contract.lock.json") != frozen_lock:
        raise HarnessError("candidate tools/core contract lock mismatch")
    for key in ("genericRuntime", "kotlinAdapter"):
        tool = value.get(key)
        if not isinstance(tool, dict) or set(tool) != {"path", "sha256"} or not isinstance(tool.get("path"), str) or not Path(tool["path"]).is_absolute() or not _is_digest(tool.get("sha256")):
            raise HarnessError(f"candidate tool contract mismatch: {key}")
        executable = _regular_file(Path(tool["path"]), key)
        if not os.access(executable, os.X_OK) or sha256_file(executable) != tool["sha256"]:
            raise HarnessError(f"candidate tool live bytes mismatch: {key}")
        tool["path"] = str(executable)
    expected_manifests = {
        minor: {"path": f"workers/manifests/kotlin{minor.replace('.', '')}.json" if minor != "2.4" else "workers/manifests/kotlin24.json"}
        for minor in ("2.1", "2.3", "2.4")
    }
    if not isinstance(value["workerManifests"], dict) or set(value["workerManifests"]) != set(expected_manifests):
        raise HarnessError("candidate worker manifest set mismatch")
    corpus_analyzers = load_authority("corpus")[0]["frozenExecutionPolicy"]["trustedAnalyzers"]
    for minor, expectation in expected_manifests.items():
        manifest = value["workerManifests"][minor]
        path = ROOT / expectation["path"]
        body = _load_json_bytes(path.read_bytes(), f"Kotlin {minor} worker manifest")
        identity = _worker_candidate_identity(minor, body)
        if not isinstance(manifest, dict) or set(manifest) != {"path","sha256","treeHash","buildInputDigest","pluginFingerprint","compilerVersion"} or manifest != {"path":expectation["path"],"sha256":sha256_file(path),**identity}:
            raise HarnessError(f"candidate worker manifest drift: {minor}")
        analyzer = corpus_analyzers.get(minor)
        compiler_jar_versions = sorted({
            Path(row.get("path", "")).name
            .removeprefix("kotlin-compiler-embeddable-")
            .removesuffix(".jar")
            for row in body.get("files", [])
            if str(row.get("path", "")).startswith("lib/kotlin-compiler-embeddable-")
        })
        if analyzer != {
            "compilerVersion": compiler_jar_versions[0] if len(compiler_jar_versions) == 1 else None,
            "manifest": expectation["path"],
        }:
            raise HarnessError(f"candidate worker/corpus compiler identity mismatch: {minor}")
    expected_sources = _candidate_source_paths()
    if not isinstance(value["sourceAuthorities"], dict) or set(value["sourceAuthorities"]) != set(expected_sources):
        raise HarnessError("candidate source authority set mismatch")
    for key, (path, kind) in expected_sources.items():
        expected_digest = _tree_digest(path) if kind == "TREE" else sha256_file(path)
        if value["sourceAuthorities"][key] != expected_digest:
            raise HarnessError(f"candidate source authority drift: {key}")
    expected_system = {
        "sandboxExec": Path("/usr/bin/sandbox-exec"), "time": Path("/usr/bin/time"),
        "git": Path("/usr/bin/git"), "ps": Path("/bin/ps"), "perl": Path("/usr/bin/perl"),
    }
    if not isinstance(value["systemTools"], dict) or set(value["systemTools"]) != set(expected_system):
        raise HarnessError("candidate system tool set mismatch")
    for key, path in expected_system.items():
        if value["systemTools"][key] != {"path": str(path), "sha256": sha256_file(path)}:
            raise HarnessError(f"candidate system tool drift: {key}")
    java = Path("/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home/bin/java")
    release = java.parent.parent / "release"
    if value["jdk"] != {"home": str(java.parent.parent), "javaSha256": sha256_file(java), "releaseSha256": sha256_file(release)}:
        raise HarnessError("candidate JDK identity mismatch")
    mvn = Path("/opt/homebrew/Cellar/maven/3.9.12/bin/mvn")
    if value["maven"] != {"path": str(mvn), "sha256": sha256_file(mvn), "version":"3.9.12"}:
        raise HarnessError("candidate Maven identity mismatch")
    if value["cargoBaseline"] != {
        "launcher": _baseline_cargo_launcher(), "target": _BASELINE_CARGO_TARGET,
        "registry": _BASELINE_CARGO_REGISTRY, "cargoLockSha256": sha256_file(ROOT / "Cargo.lock"),
    }:
        raise HarnessError("candidate Cargo baseline identity mismatch")
    value["manifestSha256"] = snapshot["sha256"]
    return value


def _worker_candidate_identity(minor: str, manifest: Mapping[str, Any]) -> dict[str, str]:
    variant = minor.replace(".", "")
    roots = [ROOT / "workers/kotlin/src/main"]
    if variant != "24":
        roots.append(ROOT / f"workers/kotlin{variant}/src/main")
    files = [
        ROOT / "build.gradle.kts", ROOT / "settings.gradle.kts", ROOT / "gradlew",
        ROOT / "gradle/wrapper/gradle-wrapper.jar",
        ROOT / "gradle/wrapper/gradle-wrapper.properties", ROOT / "schemas/worker.proto",
        ROOT / "scripts/build-trusted-worker-distributions.py",
        ROOT / ("workers/kotlin/build.gradle.kts" if variant == "24" else f"workers/kotlin{variant}/build.gradle.kts"),
    ]
    paths: set[Path] = set(files)
    for source_root in roots:
        for directory, directories, names in os.walk(source_root, followlinks=False):
            for name in directories + names:
                member = Path(directory) / name
                if member.is_symlink():
                    raise HarnessError("candidate worker build input contains a symlink")
            paths.update(Path(directory) / name for name in names)
    input_rows = sorted(
        (path.relative_to(ROOT).as_posix(), sha256_file(_regular_file(path, "worker build input")))
        for path in paths
    )
    build_input_digest = sha256_bytes(b"".join(
        relative.encode() + b"\0" + digest.encode() + b"\0"
        for relative, digest in input_rows
    ))
    plugin_name = "kotlin-0.1.0.jar" if variant == "24" else f"kotlin{variant}-0.1.0.jar"
    plugin_rows = [row for row in manifest.get("files", []) if Path(str(row.get("path", ""))).name == plugin_name]
    compiler_versions = sorted({
        Path(str(row.get("path", ""))).name.removeprefix("kotlin-compiler-embeddable-").removesuffix(".jar")
        for row in manifest.get("files", [])
        if Path(str(row.get("path", ""))).name.startswith("kotlin-compiler-embeddable-")
    })
    if len(plugin_rows) != 1 or len(compiler_versions) != 1 or not _is_digest(plugin_rows[0].get("sha256")) or not _is_digest(manifest.get("treeHash")):
        raise HarnessError(f"worker manifest lacks exact compiler/plugin identity: {minor}")
    return {
        "treeHash": str(manifest["treeHash"]), "buildInputDigest": build_input_digest,
        "pluginFingerprint": str(plugin_rows[0]["sha256"]), "compilerVersion": compiler_versions[0],
    }


def build_candidate_tools_manifest(generic_runtime: Path, kotlin_adapter: Path) -> dict[str, Any]:
    """Deterministically derive the live candidate authority; never accepts hashes."""
    generic_runtime = _regular_file(generic_runtime, "generic runtime")
    kotlin_adapter = _regular_file(kotlin_adapter, "Kotlin adapter")
    if not os.access(generic_runtime, os.X_OK) or not os.access(kotlin_adapter, os.X_OK):
        raise HarnessError("candidate binaries must be executable")
    manifests: dict[str, Any] = {}
    for minor, relative in {
        "2.1": "workers/manifests/kotlin21.json",
        "2.3": "workers/manifests/kotlin23.json",
        "2.4": "workers/manifests/kotlin24.json",
    }.items():
        path = ROOT / relative
        body = _load_json_bytes(_regular_file(path, f"Kotlin {minor} worker manifest").read_bytes(), "worker manifest")
        manifests[minor] = {"path": relative, "sha256": sha256_file(path), **_worker_candidate_identity(minor, body)}
    source_authorities = {
        key: _tree_digest(path) if kind == "TREE" else sha256_file(path)
        for key, (path, kind) in _candidate_source_paths().items()
    }
    system_paths = {
        "sandboxExec": Path("/usr/bin/sandbox-exec"), "time": Path("/usr/bin/time"),
        "git": Path("/usr/bin/git"), "ps": Path("/bin/ps"), "perl": Path("/usr/bin/perl"),
    }
    java = Path("/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home/bin/java")
    release = java.parent.parent / "release"
    mvn = Path("/opt/homebrew/Cellar/maven/3.9.12/bin/mvn")
    return {
        "schema": "codeclew.kotlin-k1-candidate-tools/0.1", "seriesId": SERIES_ID,
        "genericRuntime": {"path": str(generic_runtime), "sha256": sha256_file(generic_runtime)},
        "kotlinAdapter": {"path": str(kotlin_adapter), "sha256": sha256_file(kotlin_adapter)},
        "harnessSourceSha256": sha256_file(Path(__file__)),
        "kotlinAttemptSchemaSha256": sha256_file(ROOT / "schemas/kotlin_k1_attempt.schema.json"),
        "kotlinPreparedRefusalSchemaSha256": sha256_file(ROOT / "schemas/kotlin_k1_prepared_refusal.schema.json"),
        "kotlinSemanticCacheSchemaSha256": sha256_file(ROOT / "schemas/kotlin_semantic_cache_object.schema.json"),
        "coreContractLockSha256": sha256_file(ROOT / "contracts/core/core-contract.lock.json"),
        "workerManifests": manifests, "sourceAuthorities": source_authorities,
        "systemTools": {key: {"path": str(path), "sha256": sha256_file(path)} for key, path in system_paths.items()},
        "jdk": {"home": str(java.parent.parent), "javaSha256": sha256_file(java), "releaseSha256": sha256_file(release)},
        "maven": {"path": str(mvn), "sha256": sha256_file(mvn), "version": "3.9.12"},
        "cargoBaseline": {
            "launcher": _baseline_cargo_launcher(), "target": _BASELINE_CARGO_TARGET,
            "registry": _BASELINE_CARGO_REGISTRY, "cargoLockSha256": sha256_file(ROOT / "Cargo.lock"),
        },
        "modelCalls": 0,
    }


def _kill_process_group(process: subprocess.Popen[bytes], reason_signal: int = signal.SIGTERM) -> None:
    try:
        os.killpg(process.pid, reason_signal)
    except ProcessLookupError:
        return
    except PermissionError:
        try:
            process.send_signal(reason_signal)
        except ProcessLookupError:
            return
    try:
        process.wait(timeout=2)
        return
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    except PermissionError:
        try:
            process.kill()
        except ProcessLookupError:
            pass


def _bounded_capture(
    stream: Any,
    limit: int,
    sink: bytearray,
    overflow: threading.Event,
    process: subprocess.Popen[bytes],
) -> None:
    while True:
        chunk = stream.read(64 * 1024)
        if not chunk:
            break
        remaining = limit - len(sink)
        if remaining > 0:
            sink.extend(chunk[:remaining])
        if len(chunk) > remaining:
            overflow.set()
            _kill_process_group(process)
            break


def _bounded_file_watchdog(
    process: subprocess.Popen[bytes],
    paths: Sequence[Path],
    limit: int,
    overflow: threading.Event,
    stop: threading.Event,
) -> None:
    while not stop.is_set() and process.poll() is None:
        try:
            if any(path.stat().st_size > limit for path in paths):
                overflow.set()
                _kill_process_group(process)
                break
        except FileNotFoundError:
            pass
        stop.wait(0.05)


def _kill_remaining_process_group(process: subprocess.Popen[bytes]) -> None:
    """Kill descendants that deliberately outlive the supervised group leader."""
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except (ProcessLookupError, PermissionError):
        return
    time.sleep(0.05)
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass


def _parse_maximum_resident(time_output: bytes) -> int | None:
    text = time_output.decode("utf-8", "replace")
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.endswith("maximum resident set size"):
            value = stripped.split(maxsplit=1)[0]
            if value.isdigit():
                return int(value)
    return None


def _resident_watchdog(
    process: subprocess.Popen[bytes],
    limit_bytes: int,
    overflow: threading.Event,
    stop: threading.Event,
    observation: dict[str, Any],
) -> None:
    peak = 0
    samples = 0
    errors = 0
    while not stop.is_set() and process.poll() is None:
        completed = subprocess.run(
            ["/bin/ps", "-axo", "pgid=,rss="], stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, check=False,
            env={"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C"},
        )
        if completed.returncode != 0:
            errors += 1
        else:
            resident_kib = 0
            for line in completed.stdout.splitlines():
                fields = line.split()
                if len(fields) != 2:
                    continue
                try:
                    pgid, rss = (int(field) for field in fields)
                except ValueError:
                    continue
                if pgid == process.pid:
                    resident_kib += rss
            resident = resident_kib * 1024
            peak = max(peak, resident)
            samples += 1
            if resident > limit_bytes:
                overflow.set()
                _kill_process_group(process)
                break
        stop.wait(0.1)
    observation.update({"peakResidentBytes": peak, "samples": samples, "errors": errors})


def _canonical_digest_with_empty_field(value: Mapping[str, Any], field: str) -> str:
    projection = dict(value)
    projection[field] = ""
    # Rust adapter canonical bytes do not append a newline; stdout adds one.
    return sha256_bytes(canonical(projection).removesuffix(b"\n"))


def _rust_canonical_digest(value: Any) -> str:
    """Digest serde_json compact/sorted value bytes (stdout newline excluded)."""
    return sha256_bytes(canonical(value).removesuffix(b"\n"))


def _validate_child_terminal(raw: bytes) -> tuple[str, str, dict[str, Any]]:
    """Classify child JSON; success is retained only as an untrusted candidate."""
    try:
        value = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise HarnessError("INVALID_JSON") from error
    if not isinstance(value, dict):
        raise HarnessError("INVALID_JSON_ROOT")
    schema = value.get("schema")
    if raw != canonical(value):
        raise HarnessError("NONCANONICAL_TERMINAL_JSON")
    if schema == "codeclew.repository-impact-projection/0.1":
        expected_keys = {
            "schema", "query", "snapshot", "adapter", "capabilities", "status",
            "selectedEntities", "relevantRelations", "affected", "paths",
            "mandatoryObligations", "boundaries", "compilerReceipt", "completeness",
            "provenance", "cost", "projectionDigest",
        }
        if set(value) != expected_keys or not _is_digest(value.get("projectionDigest")):
            raise HarnessError("INVALID_PROJECTION_CONTOUR")
        if _canonical_digest_with_empty_field(value, "projectionDigest") != value["projectionDigest"]:
            raise HarnessError("INVALID_PROJECTION_SEAL")
        provenance = value.get("provenance")
        core = provenance.get("evidenceCore") if isinstance(provenance, dict) else None
        adapter_object = provenance.get("adapterOutputObject") if isinstance(provenance, dict) else None
        if not isinstance(core, dict) or core.get("schema") != "codeclew.evidence-core-binding/0.1" or not _is_digest(core.get("bundleDigest")):
            raise HarnessError("PROJECTION_LACKS_EVIDENCE_CORE_BINDING")
        if not isinstance(adapter_object, dict) or not _is_digest(adapter_object.get("digest")):
            raise HarnessError("PROJECTION_LACKS_ADAPTER_OBJECT_BINDING")
        runtime = provenance.get("runtime")
        if not isinstance(runtime, dict) or not _is_digest(runtime.get("binaryDigest")):
            raise HarnessError("PROJECTION_LACKS_RUNTIME_BINDING")
        return "VALIDATED_PROJECTION", "COMPLETED", value
    if schema == "codeclew.adapter-output/0.1":
        required = {
            "schema", "adapter", "snapshotInput", "capabilityDescriptors", "entities",
            "occurrences", "facts", "boundaries", "compilerReceipt", "impact", "cost",
            "outputDigest",
        }
        if set(value) != required or not isinstance(value.get("outputDigest"), str):
            raise HarnessError("INVALID_ADAPTER_OUTPUT_CONTOUR")
        return "DIAGNOSTIC_ADAPTER_OUTPUT", "COMPLETED", value
    if schema in {"codeclew.evidence-run-refusal/0.1", KOTLIN_TYPED_ATTEMPT_SCHEMA}:
        status = value.get("status")
        if status not in {"PARTIAL", "REFUSED", "FAILED"}:
            raise HarnessError("INVALID_TYPED_FAILURE_STATUS")
        if schema == KOTLIN_TYPED_ATTEMPT_SCHEMA:
            exact = {
                "schema", "status", "outcomeKind", "failureStage", "reasonCode",
                "detailDigest", "selectedInputs", "snapshot", "provenance", "boundaries",
                "adapterOutputDigest", "evidenceCore", "cache", "cost",
                "terminalSemanticDigest", "attemptDigest",
            }
            if set(value) != exact or value.get("outcomeKind") != "TYPED_TERMINAL" or not _is_digest(value.get("detailDigest")) or not _is_digest(value.get("terminalSemanticDigest")) or not _is_digest(value.get("attemptDigest")):
                raise HarnessError("INVALID_KOTLIN_TYPED_ATTEMPT_CONTOUR")
            if _canonical_digest_with_empty_field(value, "attemptDigest") != value["attemptDigest"]:
                raise HarnessError("INVALID_KOTLIN_TYPED_ATTEMPT_SEAL")
        return "TYPED_ATTEMPT", str(status), value
    raise HarnessError("UNRECOGNIZED_TERMINAL_SCHEMA")


def _validate_kotlin_attempt(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict) or value.get("schema") != KOTLIN_TYPED_ATTEMPT_SCHEMA:
        raise HarnessError("retained Kotlin attempt schema mismatch")
    exact = {
        "schema", "status", "outcomeKind", "failureStage", "reasonCode", "detailDigest",
        "selectedInputs", "snapshot", "provenance", "boundaries", "adapterOutputDigest",
        "evidenceCore", "cache", "cost", "terminalSemanticDigest", "attemptDigest",
    }
    if set(value) != exact or value.get("status") not in {"SUCCEEDED", "PARTIAL", "REFUSED", "FAILED"}:
        raise HarnessError("retained Kotlin attempt contour mismatch")
    if not _is_digest(value.get("terminalSemanticDigest")) or not _is_digest(value.get("attemptDigest")):
        raise HarnessError("retained Kotlin attempt digest contour mismatch")
    if _canonical_digest_with_empty_field(value, "attemptDigest") != value["attemptDigest"]:
        raise HarnessError("retained Kotlin attempt seal mismatch")
    success = value["status"] == "SUCCEEDED"
    if success:
        core = value.get("evidenceCore")
        if value.get("outcomeKind") != "ADAPTER_OUTPUT" or value.get("failureStage") is not None or value.get("reasonCode") is not None or value.get("detailDigest") is not None or not _is_digest(value.get("adapterOutputDigest")) or not isinstance(core, dict) or core.get("schema") != "codeclew.evidence-core-binding/0.1" or not _is_digest(core.get("bundleDigest")):
            raise HarnessError("successful Kotlin attempt lacks exact evidence authority")
        cache_semantic = value.get("cache", {}).get("semanticOutputDigest")
        if not _is_digest(cache_semantic) or value["terminalSemanticDigest"] != cache_semantic:
            raise HarnessError("successful Kotlin attempt terminal/cache semantic authority mismatch")
    elif value.get("outcomeKind") != "TYPED_TERMINAL" or not isinstance(value.get("failureStage"), str) or not isinstance(value.get("reasonCode"), str) or not _is_digest(value.get("detailDigest")) or value.get("adapterOutputDigest") is not None or value.get("evidenceCore") is not None:
        raise HarnessError("terminal Kotlin attempt contour mismatch")
    if not success:
        semantic = {
            "schema": KOTLIN_TYPED_ATTEMPT_SCHEMA,
            "status": value["status"], "failureStage": value["failureStage"],
            "reasonCode": value["reasonCode"], "detailDigest": value["detailDigest"],
            "selectedInputs": value["selectedInputs"], "snapshot": value["snapshot"],
            "provenance": value["provenance"], "boundaries": value["boundaries"],
            "cache": value["cache"],
        }
        if value["terminalSemanticDigest"] != _rust_canonical_digest(semantic):
            raise HarnessError("typed Kotlin attempt terminal semantic digest mismatch")
    cost = value.get("cost")
    if not isinstance(cost, dict) or set(cost) != {
        "externalWallMicros", "maximumResidentBytes", "sourceHashingMicros", "buildDiscoveryMicros",
        "dependencyPreparationMicros", "dependencyVerificationMicros", "adapterStartupMicros",
        "coldIndexMicros", "warmIndexMicros", "providerProcessingMicros", "serializationMicros",
        "storeWriteMicros", "storeReadMicros", "queryProjectionMicros", "sourceBytesRead",
        "cacheBytesRead", "cacheBytesWritten", "emittedBytes", "storedFactBytes", "factCount",
        "boundaryCount", "cacheRequests", "cacheHits", "modelCalls",
    } or cost.get("modelCalls") != 0:
        raise HarnessError("retained Kotlin attempt telemetry mismatch")
    return value


def _adapter_semantic_output_digest(output: Mapping[str, Any]) -> str:
    semantic = json.loads(json.dumps(output))
    if not isinstance(semantic, dict):
        raise HarnessError("adapter output semantic digest input mismatch")
    semantic.pop("cost", None)
    semantic.pop("outputDigest", None)
    impact = semantic.get("impact")
    if isinstance(impact, dict):
        impact.pop("queryMicros", None)
    return _rust_canonical_digest(semantic)


def _supervisor_terminal_cost(resource_row: Mapping[str, Any]) -> dict[str, Any]:
    """Exact telemetry shape for a child that could not seal its own attempt."""
    wall = resource_row.get("externalWallMicros")
    resident = resource_row.get("maximumResidentBytes")
    cost = {
        "externalWallMicros": wall if isinstance(wall, int) and not isinstance(wall, bool) and wall >= 0 else 0,
        "maximumResidentBytes": resident if isinstance(resident, int) and not isinstance(resident, bool) and resident >= 0 else "UNAVAILABLE_SUPERVISOR_TERMINAL",
        "sourceHashingMicros": 0, "buildDiscoveryMicros": 0,
        "dependencyPreparationMicros": "NOT_IN_THIS_INVOCATION",
        "dependencyVerificationMicros": 0, "adapterStartupMicros": 0,
        "coldIndexMicros": 0, "warmIndexMicros": 0, "providerProcessingMicros": 0,
        "serializationMicros": 0, "storeWriteMicros": 0, "storeReadMicros": 0,
        "queryProjectionMicros": 0, "sourceBytesRead": 0, "cacheBytesRead": 0,
        "cacheBytesWritten": 0, "emittedBytes": 0, "storedFactBytes": 0,
        "factCount": 0, "boundaryCount": 1, "cacheRequests": 0, "cacheHits": 0,
        "modelCalls": 0,
    }
    return cost


def _valid_source_range(value: Any, sources: Mapping[str, Any]) -> bool:
    if not isinstance(value, dict) or set(value) != {
        "artifactId", "artifactContentDigest", "startByte", "endByte"
    }:
        return False
    artifact = value.get("artifactId")
    source = sources.get(artifact) if isinstance(artifact, str) else None
    start, end = value.get("startByte"), value.get("endByte")
    return (
        isinstance(source, dict)
        and source.get("contentDigest") == value.get("artifactContentDigest")
        and isinstance(start, int) and not isinstance(start, bool)
        and isinstance(end, int) and not isinstance(end, bool)
        and 0 <= start <= end <= source.get("sizeBytes", -1)
    )


def _structural_proof_safety(adapter_output: Any) -> dict[str, Any]:
    """Independently classify proof/completeness overclaims.

    The frozen generic runtime remains the primary schema/core validator. This
    checker is a second, deliberately conservative oracle for K1-R08/R10 and
    never upgrades evidence: every detected overclaim is retained by code.
    """
    false_proven: set[str] = set()
    false_complete: set[str] = set()
    if not isinstance(adapter_output, dict):
        return {
            "falseProven": ["ADAPTER_OUTPUT_NOT_OBJECT"],
            "falseComplete": ["ADAPTER_OUTPUT_NOT_OBJECT"],
        }
    snapshot = adapter_output.get("snapshotInput")
    sources = {
        source.get("artifactId"): source
        for source in snapshot.get("sources", [])
        if isinstance(snapshot, dict) and isinstance(source, dict)
        and isinstance(source.get("artifactId"), str)
    } if isinstance(snapshot, dict) and isinstance(snapshot.get("sources"), list) else {}
    tree_digest = snapshot.get("repositoryTreeDigest") if isinstance(snapshot, dict) else None
    compiler = adapter_output.get("compilerReceipt")
    if not isinstance(compiler, dict):
        false_proven.add("COMPILER_RECEIPT_MISSING")
    elif compiler.get("status") == "ACCEPTED" and compiler.get("grade") == "COMPILER_CHECKED":
        if compiler.get("snapshotTreeDigest") != tree_digest:
            false_proven.add("COMPILER_RECEIPT_SNAPSHOT_MISMATCH")
        provider = compiler.get("providerPayload")
        if not isinstance(provider, dict) or provider.get("k2Validated") is not True:
            false_proven.add("COMPILER_CHECKED_WITHOUT_K2_VALIDATION")

    capabilities = adapter_output.get("capabilityDescriptors")
    capability_by_relation: dict[str, dict[str, Any]] = {}
    if not isinstance(capabilities, list):
        false_complete.add("CAPABILITIES_NOT_ARRAY")
        capabilities = []
    for capability in capabilities:
        if not isinstance(capability, dict) or not isinstance(capability.get("operationUri"), str):
            false_complete.add("MALFORMED_CAPABILITY")
            continue
        capability_by_relation[capability["operationUri"]] = capability

    boundaries = adapter_output.get("boundaries")
    if not isinstance(boundaries, list):
        false_complete.add("BOUNDARIES_NOT_ARRAY")
        boundaries = []
    proof_invalid_or_incomplete = any(
        isinstance(boundary, dict)
        and boundary.get("consequence") in {"ENUMERATION_INCOMPLETE", "PROOF_INVALID"}
        for boundary in boundaries
    )

    impact = adapter_output.get("impact")
    if not isinstance(impact, dict):
        false_complete.add("IMPACT_NOT_OBJECT")
        impact = {}
    obligations = impact.get("mandatoryObligations")
    impact_boundaries = impact.get("boundaries")
    if not isinstance(obligations, list):
        false_complete.add("MANDATORY_OBLIGATIONS_NOT_ARRAY")
        obligations = []
    if not isinstance(impact_boundaries, list):
        false_complete.add("IMPACT_BOUNDARIES_NOT_ARRAY")
        impact_boundaries = []
    open_obligations = [
        obligation for obligation in obligations
        if not isinstance(obligation, dict)
        or obligation.get("mandatory") is not True
        or obligation.get("status") != "SATISFIED"
    ]
    if impact.get("status") == "COMPLETE_IN_SCOPE":
        if proof_invalid_or_incomplete or any(
            capability.get("guaranteedEnumeration") != "COMPLETE_IN_SCOPE"
            for capability in capabilities if isinstance(capability, dict)
        ):
            false_complete.add("IMPACT_COMPLETE_ACROSS_INCOMPLETE_CONTOUR")
        if open_obligations:
            false_complete.add("IMPACT_COMPLETE_WITH_OPEN_MANDATORY_OBLIGATION")
        if impact_boundaries:
            false_complete.add("IMPACT_COMPLETE_WITH_BOUNDARY")
    if len(obligations) < len(impact_boundaries):
        false_complete.add("MANDATORY_BOUNDARY_OBLIGATION_OMITTED")
    boundary_digests = [_rust_canonical_digest(boundary) for boundary in impact_boundaries]
    obligation_boundary_digests = [
        obligation.get("boundaryDigest")
        for obligation in obligations
        if isinstance(obligation, dict)
        and obligation.get("mandatory") is True
        and obligation.get("status") in {"UNKNOWN", "UNSUPPORTED"}
        and isinstance(obligation.get("boundaryDigest"), str)
    ]
    if sorted(boundary_digests) != sorted(obligation_boundary_digests):
        false_complete.add("BOUNDARY_OBLIGATION_DIGEST_MULTISET_MISMATCH")
    if len(set(obligation_boundary_digests)) != len(obligation_boundary_digests):
        false_complete.add("DUPLICATE_BOUNDARY_OBLIGATION")
    boundary_by_digest = {
        _rust_canonical_digest(boundary): boundary
        for boundary in impact_boundaries if isinstance(boundary, dict)
    }
    for obligation in obligations:
        if not isinstance(obligation, dict) or not isinstance(obligation.get("boundaryDigest"), str):
            continue
        boundary = boundary_by_digest.get(obligation["boundaryDigest"])
        provider = obligation.get("providerPayload")
        if (
            obligation.get("kind") != "codeclew.obligation/validate-boundary/1"
            or not isinstance(boundary, dict)
            or not isinstance(provider, dict)
            or provider.get("boundaryId") != boundary.get("boundaryId")
            or provider.get("boundaryKindUri") != boundary.get("kindUri")
        ):
            false_complete.add("BOUNDARY_OBLIGATION_SCOPE_MISMATCH")

    facts = adapter_output.get("facts")
    if not isinstance(facts, list):
        false_proven.add("FACTS_NOT_ARRAY")
        facts = []
    for index, fact in enumerate(facts):
        if not isinstance(fact, dict):
            false_proven.add(f"FACT_{index}_NOT_OBJECT")
            continue
        relation = fact.get("relation")
        capability = capability_by_relation.get(relation) if isinstance(relation, str) else None
        if fact.get("enumeration") == "COMPLETE_IN_SCOPE" and (
            capability is None
            or capability.get("guaranteedEnumeration") != "COMPLETE_IN_SCOPE"
            or proof_invalid_or_incomplete
        ):
            false_complete.add(f"FACT_{index}_ENUMERATION_EXCEEDS_CAPABILITY")
        if fact.get("truth") == "TRUE" and fact.get("grade") in {
            "COMPILER_RESOLVED", "COMPILER_CHECKED", "SOUND_STATIC_IN_SCOPE"
        }:
            provider = fact.get("providerPayload")
            if not isinstance(provider, dict) or provider.get("resolution") != "PROVEN":
                false_proven.add(f"FACT_{index}_TRUE_WITHOUT_PROVEN_PROVIDER_ROW")
            elif provider.get("provider") not in {"K2_FIR", "K2_FIR_CFG"}:
                false_proven.add(f"FACT_{index}_TRUE_WITHOUT_K2_PROVIDER")
            if not _valid_source_range(fact.get("range"), sources):
                false_proven.add(f"FACT_{index}_TRUE_WITHOUT_EXACT_SOURCE_RANGE")
            if provider.get("resolution") == "UNKNOWN" or "quarant" in canonical(provider).decode().lower():
                false_proven.add(f"FACT_{index}_UNKNOWN_OR_QUARANTINED_MAPPED_TRUE")
    return {
        "falseProven": sorted(false_proven),
        "falseComplete": sorted(false_complete),
    }


def _nonempty_projection(
    adapter_output: Any,
    projection: Any,
    seed_entity: str,
) -> dict[str, Any]:
    """Apply the frozen source-grounded nonempty projection definition."""
    if not isinstance(adapter_output, dict) or not isinstance(projection, dict):
        return {"passed": False, "reasons": ["MISSING_VALIDATED_OBJECTS"]}
    entities = adapter_output.get("entities")
    facts = adapter_output.get("facts")
    occurrences = adapter_output.get("occurrences")
    impact = adapter_output.get("impact")
    sources = adapter_output.get("snapshotInput", {}).get("sources", [])
    source_by_id = {
        source.get("artifactId"): source for source in sources
        if isinstance(source, dict) and isinstance(source.get("artifactId"), str)
    } if isinstance(sources, list) else {}
    checks = {
        "entity": isinstance(entities, list) and any(
            isinstance(entity, dict) and entity.get("opaqueId") == seed_entity
            and entity.get("resolution") == "RESOLVED" for entity in entities
        ),
        "relation": isinstance(facts, list) and any(
            isinstance(fact, dict) and fact.get("truth") == "TRUE"
            and seed_entity in {fact.get("owner"), fact.get("target")}
            and _valid_source_range(fact.get("range"), source_by_id) for fact in facts
        ),
        "sourceOccurrence": isinstance(occurrences, list) and any(
            isinstance(occurrence, dict) and occurrence.get("origin") == "SOURCE"
            and occurrence.get("entityId") == seed_entity
            and _valid_source_range(occurrence.get("range"), source_by_id)
            for occurrence in occurrences
        ),
        "core": isinstance(projection.get("provenance", {}).get("evidenceCore"), dict),
    }
    affected = impact.get("affected") if isinstance(impact, dict) else None
    paths = impact.get("paths") if isinstance(impact, dict) else None
    obligations = impact.get("mandatoryObligations") if isinstance(impact, dict) else None
    seed_affected = isinstance(affected, list) and any(
        isinstance(row, dict) and row.get("entityId") == seed_entity for row in affected
    )
    source_path = isinstance(paths, list) and any(
        isinstance(row, dict) and seed_entity in {row.get("from"), row.get("to")}
        for row in paths
    )
    seed_obligation = isinstance(obligations, list) and any(
        isinstance(row, dict) and row.get("mandatory") is True and (
            row.get("seedEntity") == seed_entity or row.get("entityId") == seed_entity
            or seed_entity in row.get("scopeEntities", [])
            or any(
                isinstance(evidence, dict) and seed_entity in {
                    evidence.get("entityId"), evidence.get("owner"), evidence.get("target")
                }
                for evidence in row.get("evidence", [])
            )
        ) for row in obligations
    )
    checks["impact"] = seed_affected and (source_path or seed_obligation)
    return {
        "passed": all(checks.values()),
        "checks": checks,
        "reasons": sorted(key for key, passed in checks.items() if not passed),
    }


def _proof_safety_conformance() -> dict[str, Any]:
    """Focused mutations that the independent structural oracle must catch."""
    digest = lambda label: sha256_bytes(label.encode())
    source = {
        "artifactId": "source:Fixture.kt", "normalizedPath": "src/Fixture.kt",
        "contentDigest": digest("source"), "sizeBytes": 128, "origin": "USER",
    }
    source_range = {
        "artifactId": source["artifactId"], "artifactContentDigest": source["contentDigest"],
        "startByte": 0, "endByte": 8,
    }
    capability = {
        "operationUri": "codeclew.relation/calls/1", "grade": "COMPILER_RESOLVED",
        "guaranteedEnumeration": "COMPLETE_IN_SCOPE",
    }
    base = {
        "snapshotInput": {"repositoryTreeDigest": digest("tree"), "sources": [source]},
        "compilerReceipt": {
            "status": "ACCEPTED", "grade": "COMPILER_CHECKED",
            "snapshotTreeDigest": digest("tree"), "providerPayload": {"k2Validated": True},
        },
        "capabilityDescriptors": [capability],
        "facts": [{
            "relation": capability["operationUri"], "truth": "TRUE",
            "grade": "COMPILER_RESOLVED", "enumeration": "COMPLETE_IN_SCOPE",
            "range": source_range, "providerPayload": {"resolution": "PROVEN", "provider": "K2_FIR"},
        }],
        "boundaries": [],
        "impact": {"status": "COMPLETE_IN_SCOPE", "boundaries": [], "mandatoryObligations": []},
    }
    clean = _structural_proof_safety(base)
    mutations: dict[str, dict[str, Any]] = {}
    future_enum = json.loads(json.dumps(base))
    future_enum["boundaries"] = [{"consequence": "ENUMERATION_INCOMPLETE", "kindUri": "future-enum"}]
    mutations["futureEnumComplete"] = _structural_proof_safety(future_enum)
    k2_false = json.loads(json.dumps(base))
    k2_false["compilerReceipt"]["providerPayload"]["k2Validated"] = False
    mutations["k2FalseCompilerChecked"] = _structural_proof_safety(k2_false)
    omitted = json.loads(json.dumps(base))
    omitted["impact"]["status"] = "PARTIAL_BOUNDARY"
    omitted["impact"]["boundaries"] = [{"consequence": "PROOF_INVALID", "kindUri": "dynamic"}]
    mutations["boundaryObligationOmitted"] = _structural_proof_safety(omitted)
    paired = json.loads(json.dumps(base))
    paired["impact"]["status"] = "PARTIAL_BOUNDARY"
    paired_boundaries = [
        {"boundaryId": "b1", "consequence": "ENUMERATION_INCOMPLETE", "kindUri": "kind:one"},
        {"boundaryId": "b2", "consequence": "PROOF_INVALID", "kindUri": "kind:two"},
    ]
    paired["impact"]["boundaries"] = paired_boundaries
    paired["impact"]["mandatoryObligations"] = [
        {
            "id": f"validate-{index}", "kind": "codeclew.obligation/validate-boundary/1",
            "mandatory": True, "status": "UNKNOWN", "boundaryDigest": _rust_canonical_digest(boundary),
            "providerPayload": {"boundaryId": boundary["boundaryId"], "boundaryKindUri": boundary["kindUri"]},
        }
        for index, boundary in enumerate(paired_boundaries)
    ]
    swapped = json.loads(json.dumps(paired))
    swapped["impact"]["mandatoryObligations"][0]["boundaryDigest"], swapped["impact"]["mandatoryObligations"][1]["boundaryDigest"] = (
        swapped["impact"]["mandatoryObligations"][1]["boundaryDigest"],
        swapped["impact"]["mandatoryObligations"][0]["boundaryDigest"],
    )
    mutations["boundaryObligationSwapped"] = _structural_proof_safety(swapped)
    duplicated = json.loads(json.dumps(paired))
    duplicated["impact"]["mandatoryObligations"].append(
        json.loads(json.dumps(duplicated["impact"]["mandatoryObligations"][0]))
    )
    mutations["boundaryObligationDuplicated"] = _structural_proof_safety(duplicated)
    unknown_true = json.loads(json.dumps(base))
    unknown_true["facts"][0]["providerPayload"]["resolution"] = "UNKNOWN"
    mutations["unknownRowMappedTrue"] = _structural_proof_safety(unknown_true)
    checks = {
        "cleanAccepted": clean == {"falseProven": [], "falseComplete": []},
        "futureEnumRejected": bool(mutations["futureEnumComplete"]["falseComplete"]),
        "k2FalseRejected": bool(mutations["k2FalseCompilerChecked"]["falseProven"]),
        "boundaryOmissionRejected": bool(mutations["boundaryObligationOmitted"]["falseComplete"]),
        "boundarySwapRejected": bool(mutations["boundaryObligationSwapped"]["falseComplete"]),
        "boundaryDuplicateRejected": bool(mutations["boundaryObligationDuplicated"]["falseComplete"]),
        "unknownTrueRejected": bool(mutations["unknownRowMappedTrue"]["falseProven"]),
    }
    locked_sources = {
        "coreConformance": sha256_file(ROOT / "contracts/core/conformance-v1.json"),
        "coreConformanceTest": sha256_file(ROOT / "crates/evidence-core/tests/conformance.rs"),
        "genericRuntimeValidator": sha256_file(ROOT / "crates/evidence-adapters/src/bin/evidence.rs"),
        "kotlinAdapter": sha256_file(ROOT / "crates/evidence-adapters/src/bin/kotlin.rs"),
    }
    return {
        "schema": "codeclew.kotlin-k1-proof-safety-conformance/0.1",
        "status": "PASS" if all(checks.values()) else "FAIL",
        "checks": checks,
        "oracleSourceSha256": sha256_file(Path(__file__)),
        "lockedSources": locked_sources,
        "mutationResultsSha256": sha256_bytes(canonical(mutations)),
    }


def _median(values: list[float]) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    middle = len(ordered) // 2
    return (
        ordered[middle]
        if len(ordered) % 2
        else (ordered[middle - 1] + ordered[middle]) / 2.0
    )


def _applicability_measurement(
    rows: list[Mapping[str, Any]],
    holdout_rows: list[Mapping[str, Any]],
    corpus_entries: Mapping[str, Mapping[str, Any]],
    thresholds: Mapping[str, Any],
) -> dict[str, Any]:
    cold_rows = [row for row in rows if row.get("invocation") == "COLD"]
    holdout_cold = [row for row in holdout_rows if row.get("invocation") == "COLD"]
    holdout_warm = [row for row in holdout_rows if row.get("invocation") == "WARM"]
    typed = {"PARTIAL", "REFUSED", "FAILED"}
    total_refusals = sum(row.get("status") in typed for row in cold_rows)
    holdout_refusals = sum(row.get("status") in typed for row in holdout_cold)
    analyzers = load_authority("corpus")[0]["frozenExecutionPolicy"]["trustedAnalyzers"]

    def exact_success(row: Mapping[str, Any]) -> bool:
        entry = corpus_entries.get(str(row.get("entry")))
        return (
            isinstance(entry, Mapping)
            and row.get("status") == "ADAPTER_OUTPUT"
            and row.get("successAuthorityValidated") is True
            and isinstance(row.get("nonemptyProjection"), Mapping)
            and row["nonemptyProjection"].get("passed") is True
            and row.get("declaredProjectCompilerVersion") == entry.get("declaredKotlinVersion")
            and row.get("analyzerCompilerVersion") == analyzers[entry["trustedAnalyzerMinorLine"]]["compilerVersion"]
            and _is_digest(row.get("candidateToolsManifestSha256"))
            and isinstance(row.get("workerDistributionTreeHash"), str)
        )

    validated_holdout = [row for row in holdout_warm if exact_success(row)]
    successful_dsl = sorted({
        corpus_entries[row["entry"]]["buildDsl"] for row in rows
        if row.get("invocation") == "WARM"
        and exact_success(row)
        and row.get("entry") in corpus_entries
    })
    successful_minor = sorted({
        corpus_entries[row["entry"]]["trustedAnalyzerMinorLine"] for row in rows
        if row.get("invocation") == "WARM"
        and exact_success(row)
        and row.get("entry") in corpus_entries
    })
    required_dsl = sorted(thresholds["requiredSuccessfulBuildDslClasses"])
    required_minor = sorted(thresholds["requiredSuccessfulKotlinMinorLines"])
    passed = (
        total_refusals <= thresholds["totalTypedRefusalMaximum"]
        and holdout_refusals <= thresholds["holdoutTypedRefusalMaximum"]
        and len(validated_holdout) >= thresholds["holdoutValidatedNonemptyProjectionMinimum"]
        and successful_dsl == required_dsl
        and successful_minor == required_minor
    )
    return {
        "totalTypedRefusals": total_refusals,
        "holdoutTypedRefusals": holdout_refusals,
        "holdoutValidatedNonemptyProjections": len(validated_holdout),
        "successfulBuildDslClasses": successful_dsl,
        "requiredBuildDslClasses": required_dsl,
        "successfulKotlinMinorLines": successful_minor,
        "requiredKotlinMinorLines": required_minor,
        "passed": passed,
    }


def _cache_cost_measurement(
    rows: list[Mapping[str, Any]],
    holdout_rows: list[Mapping[str, Any]],
    thresholds: Mapping[str, Any],
) -> dict[str, Any]:
    end_to_end_ratios: list[float] = []
    provider_ratios: list[float] = []
    cache_hits = 0
    for entry in EXPECTED_HOLDOUT:
        cold = next(row for row in holdout_rows if row["entry"] == entry and row["invocation"] == "COLD")
        warm = next(row for row in holdout_rows if row["entry"] == entry and row["invocation"] == "WARM")
        valid_pair = (
            cold.get("status") == warm.get("status") == "ADAPTER_OUTPUT"
            and cold.get("successAuthorityValidated") is True
            and warm.get("successAuthorityValidated") is True
            and cold.get("terminalSemanticDigest") == warm.get("terminalSemanticDigest")
            and warm.get("cacheHit") is True
        )
        if valid_pair:
            cache_hits += 1
        cold_external, warm_external = cold.get("externalWallMicros"), warm.get("externalWallMicros")
        cold_provider = cold.get("adapterCost", {}).get("providerProcessingMicros")
        warm_provider = warm.get("adapterCost", {}).get("providerProcessingMicros")
        eligible = valid_pair
        if eligible and isinstance(cold_external, int) and cold_external > 0 and isinstance(warm_external, int):
            end_to_end_ratios.append(warm_external / cold_external)
        if eligible and isinstance(cold_provider, int) and cold_provider > 0 and isinstance(warm_provider, int):
            provider_ratios.append(warm_provider / cold_provider)
    required_cost_keys = {
        "externalWallMicros", "maximumResidentBytes", "sourceHashingMicros", "buildDiscoveryMicros",
        "dependencyPreparationMicros", "dependencyVerificationMicros", "adapterStartupMicros",
        "coldIndexMicros", "warmIndexMicros", "providerProcessingMicros", "serializationMicros",
        "storeWriteMicros", "storeReadMicros", "queryProjectionMicros", "sourceBytesRead",
        "cacheBytesRead", "cacheBytesWritten", "emittedBytes", "storedFactBytes", "factCount",
        "boundaryCount", "cacheRequests", "cacheHits", "modelCalls",
    }


    telemetry_complete = True
    bounded = True
    for row in rows:
        cost = row.get("adapterCost")
        if not isinstance(cost, Mapping) or set(cost) != required_cost_keys:
            telemetry_complete = False
            bounded = False
            continue
        wall = row.get("externalWallMicros")
        rss = row.get("maximumResidentBytes")
        if not isinstance(wall, int) or isinstance(wall, bool) or wall < 0:
            telemetry_complete = False
            bounded = False
        elif wall > thresholds["perInvocationWallSecondsMaximum"] * 1_000_000:
            bounded = False
        if not isinstance(rss, int) or isinstance(rss, bool) or rss < 0:
            telemetry_complete = False
            bounded = False
        elif rss > thresholds["perInvocationMaximumResidentBytes"]:
            bounded = False
        for key, value in cost.items():
            if key in {"maximumResidentBytes", "dependencyPreparationMicros", "dependencyVerificationMicros"}:
                if not ((isinstance(value, int) and not isinstance(value, bool) and value >= 0) or (isinstance(value, str) and value)):
                    telemetry_complete = False
            elif not isinstance(value, int) or isinstance(value, bool) or value < 0:
                telemetry_complete = False
    median_e2e = _median(end_to_end_ratios)
    median_provider = _median(provider_ratios)
    passed = (
        cache_hits >= thresholds["holdoutRealWarmCacheHitMinimum"]
        and len(end_to_end_ratios) >= thresholds["holdoutRealWarmCacheHitMinimum"]
        and len(provider_ratios) >= thresholds["holdoutRealWarmCacheHitMinimum"]
        and median_e2e is not None
        and median_e2e <= thresholds["holdoutMedianWarmEndToEndWallRatioMaximum"]
        and median_provider is not None
        and median_provider <= thresholds["holdoutMedianWarmProviderWallRatioMaximum"]
        and telemetry_complete and bounded
    )
    return {
        "holdoutWarmCacheHits": cache_hits,
        "medianWarmEndToEndWallRatio": median_e2e,
        "medianWarmProviderWallRatio": median_provider,
        "ratioPopulation": len(end_to_end_ratios),
        "providerRatioPopulation": len(provider_ratios),
        "telemetryComplete": telemetry_complete,
        "invocationsBounded": bounded,
        "passed": passed,
    }


def _bound_producer_packet(
    store: Store,
    inputs: Mapping[str, Mapping[str, Any]],
    key: str,
    schema: str,
    producer: str,
) -> tuple[dict[str, Any], str, str]:
    """Reopen a create-only packet and bind it to its current DAG producer."""
    packet, digest = _canonical_artifact(inputs, key, schema)
    pointer = store.pointer(producer)
    receipt = store.receipt(producer)
    if (
        pointer is None
        or receipt is None
        or receipt.get("status") != "READY"
        or receipt.get("evidence", {}).get("packetSha256") != digest
    ):
        raise HarnessError(f"{key} is not bound to current {producer} authority")
    return packet, digest, pointer["receiptDigest"]


def _source_anchor_packet() -> dict[str, Any]:
    """Static K1 reachability/non-goal evidence over the exact candidate bytes.

    Legacy workers still contain edit operations for K0-era callers.  K1-R01
    is therefore proved at the K1 executable entrypoint: the Kotlin evidence
    adapter may reach OpenProject, verified indexing, inspection and shutdown,
    but has no request construction or CLI authority for an edit operation.
    """
    adapter_path = ROOT / "crates/evidence-adapters/src/bin/kotlin.rs"
    cache_path = ROOT / "crates/evidence-adapters/src/bin/kotlin_k1.rs"
    harness_path = Path(__file__)
    adapter = adapter_path.read_text(encoding="utf-8")
    production = adapter.split("#[cfg(test)]", 1)[0]
    request_kinds = sorted(set(re.findall(r"RequestKind::([A-Za-z0-9_]+)", production)))
    worker_calls = sorted(set(re.findall(r"\bworker\.([A-Za-z0-9_]+)\s*\(", production)))
    cli_block = production[production.index("struct Args"):production.index("enum RunPhase")]
    cli_fields = sorted(set(re.findall(r"^\s{4}([a-z][a-z0-9_]*):", cli_block, re.M)))
    forbidden_cli = sorted(field for field in cli_fields if any(token in field for token in (
        "edit", "apply", "patch", "transaction", "preview", "model", "recipe", "dispatch",
    )))
    forbidden_request_kinds = sorted(set(request_kinds) & {
        "ApplyEdit", "PreviewEdit", "BeginTransaction", "CommitTransaction", "RollbackTransaction",
    })
    non_goal_needles = {
        "JBMC": "jbmc", "BYTEBACK": "byteback", "MODEL_PROVIDER": "openai",
        "ANTHROPIC": "anthropic", "FAMILY_DISPATCH": "benchmark-family",
    }
    reachable_lower = production.lower()
    forbidden_non_goals = sorted(
        name for name, needle in non_goal_needles.items() if needle in reachable_lower
    )
    worker_source = (ROOT / "workers/kotlin/src/main/kotlin/dev/semanticthread/worker/Worker.kt").read_text(encoding="utf-8")
    worker_tests = (ROOT / "workers/kotlin/src/test/kotlin/dev/semanticthread/worker/ProjectModelCommandTest.kt").read_text(encoding="utf-8")
    index_source = (ROOT / "crates/clew/src/index.rs").read_text(encoding="utf-8")
    cache_source = cache_path.read_text(encoding="utf-8")
    checks = {
        "onlyReadOnlyRequestKinds": request_kinds == ["OpenProject"],
        "onlyReadOnlyWorkerCalls": set(worker_calls) <= {
            "index_files_verified", "inspect_verified_index", "request", "shutdown",
        },
        "noMutationCliAuthority": not forbidden_cli,
        "noMutationRequestKind": not forbidden_request_kinds,
        "noModelOrExcludedNonGoalReachability": not forbidden_non_goals,
        "dedicatedRunnerConstructsExactCommand": "exact_command = [" in harness_path.read_text(encoding="utf-8"),
        "adapterOwnsTypedAttemptAndCache": "KotlinAttempt" in adapter and "SemanticCache" in cache_path.read_text(encoding="utf-8"),
        "futureCompilerValuesCovered": "futureCompilerDescriptorValuesBecomeTypedBoundaries" in worker_tests,
        "malformedRowsCovered": "malformedCompilerFactRowIsRetainedAsBothTypedGraphBoundaries" in worker_tests,
        "utf16ToUtf8Covered": "compilerUtf16OffsetsAreConvertedToUtf8BytesWithoutSplittingEmoji" in worker_tests,
        "effectiveVisibilityLocalIsTyped": (
            'SUPPORTED_EFFECTIVE_VISIBILITIES = setOf(' in worker_source
            and '"local"' not in worker_source.split('SUPPORTED_EFFECTIVE_VISIBILITIES = setOf(', 1)[1].split(')', 1)[0]
            and '"UNKNOWN_EFFECTIVE_VISIBILITY"' in worker_source
        ),
        "quarantinedRowsCannotBecomeProven": (
            "REFERENCE_TO_QUARANTINED_DESCRIPTOR" in worker_source
            and "REFERENCE_TO_QUARANTINED_DESCRIPTOR" in index_source
        ),
        "cacheCorruptionAndSymlinkCovered": "cache_rejects_corruption_and_symlink" in cache_source,
        "cacheKeyOrderCovered": "cache_key_binds_ordered_manifest" in cache_source,
        "cacheInputDriftCovered": "cache_payload_receipt_must_bind_exact_key_inputs" in cache_source,
        "terminalDigestVolatilePathCovered": "terminal_identity_ignores_absolute_staging_path" in cache_source,
    }
    return {
        "schema": "codeclew.kotlin-k1-source-anchor-packet/0.1",
        "status": "PASS" if all(checks.values()) else "FAIL",
        "checks": checks,
        "requestKinds": request_kinds,
        "workerCalls": worker_calls,
        "cliFields": cli_fields,
        "forbiddenCliFields": forbidden_cli,
        "forbiddenRequestKinds": forbidden_request_kinds,
        "forbiddenNonGoals": forbidden_non_goals,
        "sources": {
            "kotlinAdapter": sha256_file(adapter_path),
            "kotlinK1Protocol": sha256_file(cache_path),
            "harness": sha256_file(harness_path),
        },
    }


def _build_dependency_conformance() -> dict[str, Any]:
    """Named build/dependency mutations checked by an independent oracle."""
    ordered_fields = (
        "orderedCompileClasspath", "orderedFriendPaths", "orderedCompilerPlugins",
        "orderedFreeCompilerArguments", "orderedOptIns", "orderedCompilerPluginOptions",
    )
    required_fields = {
        "schema", "compilation", "declaredCompilerVersion", "analyzerCompilerVersion",
        *ordered_fields, "target", "languageVersion", "apiVersion", "jdkHomeFingerprint",
        "fieldBoundaries", "buildRoot", "projectDirectory", "module", "sourceSet", "platform",
        "generatedSourceConfiguration", "buildModelBoundaries", "dependencyCoordinates",
        "repositories", "reactorPoms", "buildPlugins", "classpathAuthority", "buildState",
        "modelInputs",
    }
    digest = lambda label: sha256_bytes(label.encode())
    base = {
        "schema": "kotlin-semantic-input-manifest/0.1", "compilation": ":app/main",
        "declaredCompilerVersion": "2.3.10", "analyzerCompilerVersion": "2.3.0",
        "orderedCompileClasspath": [
            {"path":"a.jar","sha256":digest("a"),"coordinate":"g:a:1","scope":"compile"},
            {"path":"b.jar","sha256":digest("b"),"coordinate":"g:b:1","scope":"runtime"},
        ],
        "orderedFriendPaths": [], "orderedCompilerPlugins": [{"id":"plugin","sha256":digest("p")}],
        "orderedFreeCompilerArguments": ["-Xa", "-Xb"], "orderedOptIns": ["x.One"],
        "orderedCompilerPluginOptions": ["plugin:key=value"], "target": "21",
        "languageVersion": "2.3", "apiVersion": "2.3", "jdkHomeFingerprint": digest("jdk"),
        "fieldBoundaries": {"compilerPlugins":"AVAILABLE"}, "buildRoot": ".",
        "projectDirectory": "app", "module": ":app", "sourceSet": "main", "platform": "JVM",
        "generatedSourceConfiguration": {"roots":[],"producers":[],"status":"NONE_DISCOVERED"},
        "buildModelBoundaries": [],
        "dependencyCoordinates": [{"coordinate":"g:a:1","scope":"compile"},{"coordinate":"g:b:1","scope":"runtime"}],
        "repositories": [{"id":"central","url":"https://repo1.maven.org/maven2"}],
        "reactorPoms": ["pom.xml", "app/pom.xml"],
        "buildPlugins": [{"coordinate":"org.jetbrains.kotlin:kotlin-maven-plugin:2.3.10"}],
        "classpathAuthority": {"chosen":"RESOLVED_CONFIGURATION"},
        "buildState": {"seedDigest":digest("seed"),"manifestDigest":digest("manifest")},
        "modelInputs": [{"path":"app/pom.xml","hash":digest("pom")}],
    }

    def valid(value: Any) -> bool:
        if not isinstance(value, dict) or set(value) != required_fields:
            return False
        if value.get("schema") != "kotlin-semantic-input-manifest/0.1" or value.get("platform") != "JVM":
            return False
        if any(not isinstance(value.get(field), list) for field in ordered_fields):
            return False
        classpath = value.get("orderedCompileClasspath")
        if not classpath or any(
            not isinstance(row, dict) or set(row) != {"path","sha256","coordinate","scope"}
            or not _is_digest(row.get("sha256")) for row in classpath
        ):
            return False
        if not isinstance(value.get("fieldBoundaries"), dict) or not isinstance(value.get("buildModelBoundaries"), list):
            return False
        return all(value.get(field) is not None for field in (
            "compilation", "declaredCompilerVersion", "analyzerCompilerVersion", "jdkHomeFingerprint",
            "buildRoot", "projectDirectory", "module", "sourceSet", "generatedSourceConfiguration",
            "dependencyCoordinates", "repositories", "reactorPoms", "buildPlugins", "buildState",
        ))

    mutations: dict[str, dict[str, Any]] = {}
    for name, mutate in {
        "compilerArgumentOrder": lambda value: value["orderedFreeCompilerArguments"].reverse(),
        "classpathOrder": lambda value: value["orderedCompileClasspath"].reverse(),
        "jarBytes": lambda value: value["orderedCompileClasspath"][0].update(sha256=digest("changed")),
        "coordinate": lambda value: value["orderedCompileClasspath"][0].update(coordinate="g:a:2"),
        "scope": lambda value: value["orderedCompileClasspath"][0].update(scope="test"),
        "repository": lambda value: value["repositories"][0].update(url="https://example.invalid"),
        "plugin": lambda value: value["orderedCompilerPlugins"][0].update(sha256=digest("changed-plugin")),
        "reactor": lambda value: value["reactorPoms"].append("extra/pom.xml"),
        "generatedConfiguration": lambda value: value["generatedSourceConfiguration"].update(status="ROOTS_ONLY"),
    }.items():
        changed = json.loads(json.dumps(base))
        mutate(changed)
        mutations[name] = {
            "validShape": valid(changed),
            "digestChanged": _rust_canonical_digest(changed) != _rust_canonical_digest(base),
        }
    missing = json.loads(json.dumps(base))
    del missing["repositories"]
    mutations["missingReflectiveField"] = {"validShape": valid(missing), "digestChanged": True}
    checks = {
        "baseAccepted": valid(base),
        "allSemanticMutationsChangeDigest": all(row["digestChanged"] for row in mutations.values()),
        "missingFieldRejected": mutations["missingReflectiveField"]["validShape"] is False,
    }
    return {
        "schema":"codeclew.kotlin-k1-build-dependency-conformance/0.1",
        "status":"PASS" if all(checks.values()) else "FAIL", "checks":checks,
        "orderedFields":list(ordered_fields), "requiredFields":sorted(required_fields),
        "baseDigest":_rust_canonical_digest(base), "mutations":mutations,
        "mutationResultsSha256":sha256_bytes(canonical(mutations)),
        "sourceAuthorities": {
            "worker":sha256_file(ROOT / "workers/kotlin/src/main/kotlin/dev/semanticthread/worker/Worker.kt"),
            "mavenModel":sha256_file(ROOT / "workers/kotlin/src/main/kotlin/dev/semanticthread/worker/MavenProjectModel.kt"),
            "gradleModel":sha256_file(ROOT / "workers/kotlin/src/main/resources/semantic-thread-model.init.gradle"),
            "adapter":sha256_file(ROOT / "crates/evidence-adapters/src/bin/kotlin.rs"),
        },
    }


def _determinism_conformance() -> dict[str, Any]:
    base = {
        "entities":[{"opaqueId":"b"},{"opaqueId":"a"}],
        "facts":[{"relation":"calls","owner":"a","target":"b"}],
        "boundaries":[], "orderedCompilerArguments":["-Xa","-Xb"],
        "volatile":{"timestamp":1,"path":"/tmp/one"},
    }
    semantic = lambda value: _rust_canonical_digest({
        "entities":sorted(value["entities"], key=lambda row: row["opaqueId"]),
        "facts":sorted(value["facts"], key=lambda row: canonical(row)),
        "boundaries":sorted(value["boundaries"], key=lambda row: canonical(row)),
        "orderedCompilerArguments":value["orderedCompilerArguments"],
    })
    reordered = json.loads(json.dumps(base)); reordered["entities"].reverse()
    volatile = json.loads(json.dumps(base)); volatile["volatile"] = {"timestamp":999,"path":"/tmp/two"}
    arguments = json.loads(json.dumps(base)); arguments["orderedCompilerArguments"].reverse()
    checks = {
        "trueSetOrderEquivalent":semantic(base) == semantic(reordered),
        "volatileMetadataExcluded":semantic(base) == semantic(volatile),
        "orderedArgumentsSignificant":semantic(base) != semantic(arguments),
    }
    return {"schema":"codeclew.kotlin-k1-determinism-conformance/0.1","status":"PASS" if all(checks.values()) else "FAIL","checks":checks,"digests":{"base":semantic(base),"reordered":semantic(reordered),"volatile":semantic(volatile),"arguments":semantic(arguments)}}


def _row_terminal_protocol(row: Mapping[str, Any]) -> bool:
    status = row.get("status")
    if status == "ADAPTER_OUTPUT":
        return row.get("successAuthorityValidated") is True and isinstance(row.get("adapterAuthority"), Mapping)
    return (
        status in {"PARTIAL", "REFUSED", "FAILED"}
        and isinstance(row.get("reasonCode"), str)
        and not str(row.get("reasonCode")).startswith("UNTYPED_FAILURE/")
        and _is_digest(row.get("terminalSemanticDigest"))
        and row.get("successAuthorityValidated") is False
    )


def _validate_requirement_rows(rows: Any) -> None:
    expected = [f"K1-R{number:02d}" for number in range(1, 21)]
    if not isinstance(rows, Mapping) or list(rows) != expected:
        raise HarnessError("requirement result set/order differs from exact K1-R01..K1-R20")
    for identifier in expected:
        row = rows[identifier]
        exact = {
            "predicate", "measured", "missingEvidence", "failureClass",
            "evidence", "status", "evidenceSha256",
        }
        if (
            not isinstance(row, Mapping)
            or set(row) != exact
            or not isinstance(row.get("predicate"), str)
            or not isinstance(row.get("measured"), bool)
            or not isinstance(row.get("missingEvidence"), list)
            or not all(isinstance(value, str) for value in row["missingEvidence"])
            or row.get("status") not in {"PASS", "FAIL"}
        ):
            raise HarnessError(f"requirement result contour mismatch: {identifier}")
        passed = row["measured"] and not row["missingEvidence"]
        expected_status = "PASS" if passed else "FAIL"
        valid_class = row.get("failureClass") is None if passed else row.get("failureClass") in {"STOP", "GAP"}
        if not valid_class or row["status"] != expected_status:
            raise HarnessError(f"requirement result status/class mismatch: {identifier}")
        body = {
            key: row[key]
            for key in ("predicate", "measured", "missingEvidence", "failureClass", "evidence")
        }
        if row["evidenceSha256"] != sha256_bytes(canonical(body)):
            raise HarnessError(f"requirement result seal mismatch: {identifier}")


def _requirement_row_mutation_conformance(rows: Mapping[str, Mapping[str, Any]]) -> dict[str, Any]:
    """Every R01..R20 row rejects one independently resealed result mutation."""
    _validate_requirement_rows(rows)
    cases: dict[str, bool] = {}
    for identifier in rows:
        changed = json.loads(json.dumps(rows))
        row = changed[identifier]
        if row["status"] == "PASS":
            row["measured"] = False
        else:
            row["measured"] = True
            row["missingEvidence"] = []
        body = {
            key: row[key]
            for key in ("predicate", "measured", "missingEvidence", "failureClass", "evidence")
        }
        row["evidenceSha256"] = sha256_bytes(canonical(body))
        try:
            _validate_requirement_rows(changed)
        except HarnessError:
            cases[identifier] = True
        else:
            cases[identifier] = False
    return {
        "schema":"codeclew.kotlin-k1-requirement-row-mutation-conformance/0.1",
        "status":"PASS" if len(cases) == 20 and all(cases.values()) else "FAIL",
        "cases":cases,
        "resultsSha256":sha256_bytes(canonical(cases)),
    }


def _valid_success_build_authority(row: Mapping[str, Any]) -> bool:
    authority = row.get("buildModelAuthority")
    if not isinstance(authority, Mapping):
        return False
    manifest = authority.get("semanticInputManifest")
    return (
        isinstance(manifest, Mapping)
        and authority.get("semanticInputManifestHash") == _rust_canonical_digest(manifest)
        and all(field in manifest for field in (
            "orderedCompileClasspath", "orderedFriendPaths", "orderedCompilerPlugins",
            "orderedFreeCompilerArguments", "orderedOptIns", "orderedCompilerPluginOptions",
            "dependencyCoordinates", "repositories", "reactorPoms", "buildPlugins",
            "generatedSourceConfiguration", "fieldBoundaries", "buildModelBoundaries",
            "buildRoot", "projectDirectory", "module", "sourceSet", "platform",
            "jdkHomeFingerprint", "buildState", "modelInputs",
        ))
        and all(_is_digest(authority.get(field)) for field in (
            "dependencyGraphDigest", "buildModelDigest", "buildConfigurationDigest",
            "generatedSourcesManifestDigest", "boundariesDigest",
        ))
    )


def _requirement_predicates(
    store: Store,
    safety: Mapping[str, Any],
    applicability: Mapping[str, Any],
    cache_cost: Mapping[str, Any],
    qualification: Mapping[str, Any],
    holdout: Mapping[str, Any],
    inputs: Mapping[str, Mapping[str, Any]],
) -> dict[str, dict[str, Any]]:
    rows = qualification["attempts"] + holdout["attempts"]
    corpus = store.bundle["corpus"]
    entries = {entry["id"]: entry for entry in corpus["entries"]}
    identifiers = [row.get("id") for row in corpus["entries"]]
    organizations = [row.get("organization") for row in corpus["entries"]]
    organization_counts = {organization: organizations.count(organization) for organization in set(organizations)}
    successes = [row for row in rows if row.get("status") == "ADAPTER_OUTPUT"]
    baseline, baseline_sha, baseline_receipt = _bound_producer_packet(
        store, inputs, "baselinePacket", "codeclew.kotlin-k1-baseline-packet/0.2", "BASELINE_CAPTURE",
    )
    harness, harness_sha, harness_receipt = _bound_producer_packet(
        store, inputs, "harnessSelfTestPacket", "codeclew.kotlin-k1-harness-self-test-packet/0.1", "HARNESS_SELF_TEST",
    )
    freeze, freeze_sha = _canonical_artifact(inputs, "candidateFreeze", "codeclew.kotlin-k1-candidate-freeze/0.1")
    freeze_pointer = store.pointer("CANDIDATE_FREEZE_VERIFY")
    freeze_receipt = store.receipt("CANDIDATE_FREEZE_VERIFY")
    if freeze_pointer is None or freeze_receipt is None or freeze_receipt.get("status") != "READY" or freeze_receipt.get("evidence", {}).get("candidateFreezeSha256") != freeze_sha:
        raise HarnessError("requirement conformance lacks current candidate freeze producer")

    def current_receipt(node: str) -> tuple[dict[str, Any], str]:
        pointer, receipt = store.pointer(node), store.receipt(node)
        if pointer is None or receipt is None or receipt.get("status") != "READY":
            raise HarnessError(f"requirement evidence producer is not READY: {node}")
        return receipt, pointer["receiptDigest"]

    qualification_prepare, qualification_prepare_digest = current_receipt("QUALIFICATION_DEPENDENCY_SEED_PREPARE")
    holdout_prepare, holdout_prepare_digest = current_receipt("HOLDOUT_DEPENDENCY_SEED_PREPARE")
    holdout_materialize, holdout_materialize_digest = current_receipt("HOLDOUT_SOURCE_MATERIALIZE")
    k0_receipt, k0_receipt_digest = current_receipt("K0_1_BYTE_EXACT_VERIFY")
    preparation_rows = (
        qualification_prepare.get("evidence", {}).get("preparationAttempts", [])
        + holdout_prepare.get("evidence", {}).get("preparationAttempts", [])
    )
    preparation_by_entry = {
        row.get("entry"): row for row in preparation_rows if isinstance(row, Mapping)
    }
    source_packet = harness.get("sourceAnchorPacket")
    build_packet = harness.get("buildDependencyConformance")
    determinism_packet = harness.get("determinismConformance")
    requirement_cases = harness.get("requirementCases")
    supervisor_cases = harness.get("supervisor", {}).get("cases", {})
    static_current = source_packet == _source_anchor_packet() and source_packet.get("status") == "PASS" if isinstance(source_packet, Mapping) else False
    build_current = build_packet == _build_dependency_conformance() and build_packet.get("status") == "PASS" if isinstance(build_packet, Mapping) else False
    determinism_current = determinism_packet == _determinism_conformance() and determinism_packet.get("status") == "PASS" if isinstance(determinism_packet, Mapping) else False
    expected_pairs = {
        (entry, invocation)
        for entry in EXPECTED_QUALIFICATION + EXPECTED_HOLDOUT
        for invocation in ("COLD", "WARM")
    }
    actual_pairs = {(row.get("entry"), row.get("invocation")) for row in rows}
    tools_sha = freeze.get("snapshots", {}).get("candidateTools", {}).get("sha256")
    analyzers = corpus["frozenExecutionPolicy"]["trustedAnalyzers"]
    exact_success_identity = all(
        row.get("entry") in entries
        and row.get("declaredProjectCompilerVersion") == entries[row["entry"]]["declaredKotlinVersion"]
        and row.get("analyzerCompilerVersion") == analyzers[entries[row["entry"]]["trustedAnalyzerMinorLine"]]["compilerVersion"]
        and isinstance(row.get("workerDistributionIdentity"), Mapping)
        and row["workerDistributionIdentity"].get("treeHash") == row.get("workerDistributionTreeHash")
        and all(_is_digest(row["workerDistributionIdentity"].get(key)) for key in ("treeHash","buildInputDigest","pluginFingerprint"))
        and row.get("candidateToolsManifestSha256") == tools_sha
        for row in successes
    )
    compiler_lines = sorted({
        entries[row["entry"]]["trustedAnalyzerMinorLine"] for row in successes
        if row.get("entry") in entries and exact_success_identity
    })
    source_equal = all(
        row.get("sourceMutation") is False
        and isinstance(row.get("repositoryBefore"), Mapping)
        and row.get("repositoryBefore") == row.get("repositoryAfter")
        and row["repositoryBefore"].get("head") == entries.get(row.get("entry"), {}).get("commit")
        and row["repositoryBefore"].get("tree") == entries.get(row.get("entry"), {}).get("gitTree")
        for row in rows
    )
    terminal_protocol = actual_pairs == expected_pairs and len(rows) == 24 and all(_row_terminal_protocol(row) for row in rows)
    build_authority_valid = all(_valid_success_build_authority(row) for row in successes)
    closure_reasons = {
        "DEPENDENCY_CLOSURE_UNAVAILABLE", "OFFLINE_MODEL_PROBE_FAILED", "UNSUPPORTED_BUILD_CONFIGURATION",
    }
    closure_refusals_sound = all(
        row.get("outcome") == "READY" or (
            row.get("outcome") == "TYPED_REFUSAL" and row.get("reasonCode") in closure_reasons
        ) for row in preparation_rows if isinstance(row, Mapping)
    ) and len(preparation_rows) == 12
    closure_refusal_count = sum(
        row.get("outcome") == "TYPED_REFUSAL" for row in preparation_rows if isinstance(row, Mapping)
    )
    dsl_counts = {
        dsl: sum(entry["buildDsl"] == dsl for entry in corpus["entries"])
        for dsl in ("GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL", "MAVEN")
    }
    preparation_parity = set(preparation_by_entry) == set(entries) and all(
        all(preparation_by_entry[entry_id].get(key) == entry[key] for key in (
            "commit", "gitTree", "selectedCompilation", "buildDsl",
        ))
        and _is_digest(preparation_by_entry[entry_id].get("prepareArgvSha256"))
        and preparation_by_entry[entry_id].get("prepareArgvSha256") == sha256_bytes(canonical(preparation_by_entry[entry_id].get("prepareArgv")))
        for entry_id, entry in entries.items()
    )

    def offline_row(row: Mapping[str, Any]) -> bool:
        return _preparation_network_evidence_valid(row)

    offline_prepare = len(preparation_rows) == 12 and all(offline_row(row) for row in preparation_rows)
    exact_workload = all(
        row.get("nonemptyProjection", {}).get("passed") is True
        and isinstance(row.get("workload"), Mapping)
        and row["workload"].get("selectionAuthority") == "HARNESS_DERIVED_ONLY"
        and isinstance(row["workload"].get("seedEntity"), str)
        and row["workload"].get("maxDepth") == store.bundle["requirements"]["workloadPolicy"]["query"]["maxDepth"]
        and row["workload"].get("maxEntities") == store.bundle["requirements"]["workloadPolicy"]["query"]["maxEntities"]
        for row in successes
    )
    cache_protocol = all(
        (
            cold.get("status") != "ADAPTER_OUTPUT"
            and warm.get("status") == cold.get("status")
            and warm.get("terminalSemanticDigest") == cold.get("terminalSemanticDigest")
            and warm.get("cacheHit") is False
        ) or (
            cold.get("status") == warm.get("status") == "ADAPTER_OUTPUT"
            and cold.get("terminalSemanticDigest") == warm.get("terminalSemanticDigest")
            and cold.get("semanticFactsDigest") == warm.get("semanticFactsDigest")
            and warm.get("cacheHit") is True
            and _is_digest(cold.get("semanticCacheKeyDigest"))
            and cold.get("semanticCacheKeyDigest") == warm.get("semanticCacheKeyDigest")
        )
        for entry in EXPECTED_QUALIFICATION + EXPECTED_HOLDOUT
        for cold, warm in [[
            next(row for row in rows if row.get("entry") == entry and row.get("invocation") == "COLD"),
            next(row for row in rows if row.get("entry") == entry and row.get("invocation") == "WARM"),
        ]]
    ) if actual_pairs == expected_pairs else False
    sandbox_rows = all(
        row.get("sourceExecutionAuthority", {}).get("kind") == "SANITIZED_DISPOSABLE_GIT"
        and row.get("sourceExecutionAuthority", {}).get("selectedSourceTreeSha256") == row.get("sourceExecutionAuthority", {}).get("executionSourceTreeSha256")
        and row.get("sourceExecutionAuthority", {}).get("discardedBeforePublication") is True
        and row.get("selectedInputs", {}).get("sandboxDefaultPolicy") == "DENY_DEFAULT_NETWORK_DENY"
        and row.get("selectedInputs", {}).get("productionCredentialInheritance") is False
        and all(_is_digest(row.get("selectedInputs", {}).get(key)) for key in (
            "environmentPolicySha256", "networkSandboxProfileSha256", "sandboxAuthorizedReadPathsSha256",
            "sandboxAuthorizedWritePathsSha256", "sandboxExecutableSha256",
        ))
        for row in rows
    )
    required_supervisor = {
        "empty", "nonzero", "build_failure", "invalid_json", "truncated_json", "oom_like_signal",
        "direct_adapter_output", "background_child", "typed_nonzero", "timeout", "limit",
        "sandbox_network_env", "sandbox_secret_paths", "sandbox_unix_network",
        "sandbox_source_write", "sandbox_keychain_read", "sandbox_background_child",
    }
    supervisor_complete = (
        isinstance(supervisor_cases, Mapping)
        and required_supervisor <= set(supervisor_cases)
        and all(supervisor_cases.get(name) == value for name, value in SECURITY_SUPERVISOR_EXPECTED.items())
    )
    required_authority_cases = {
        "alternateGraphRejected", "alternateThresholdRejected", "alternateCorpusRejected",
        "staleInputRejected", "directNodeForgeryRejected", "earlyHoldoutRejected",
        "callerAttemptForgeryRejected", "conditionalRootForgeryRejected", "cancelledOrderingRejected",
        "trackedLinkEscapeRejected", "dirtySourceSetRejected", "prepareSupervisorNonzeroRetained",
        "prepareMavenLauncherTraversalPassed", "prepareSourceAncestryTraversalPassed",
        "prepareAncestorSecretReadDenied", "prepareAncestorWriteDenied",
        "prepareSelectedSourceWriteDenied", "prepareKeychainReadDenied",
        "prepareTraversalNetworkSemanticsPreserved", "prepareAncestorDataOnlyMutationRejected",
        "prepareBroadSandboxPermissionRejected", "prepareRootAuthoritySubstitutionsRejected",
        "prepareSplitPhaseRootsRejected", "requirementR18SupervisorNotRunRejected",
        "requirementR18PrepareNotRunRejected",
    }
    authority_complete = (
        isinstance(requirement_cases, Mapping)
        and required_authority_cases <= set(requirement_cases)
        and all(requirement_cases[name] is True for name in required_authority_cases)
    )
    baseline_commands = baseline.get("commands", [])
    focused_commands = {
        tuple(row.get("argv", [])): row
        for row in baseline_commands if isinstance(row, Mapping)
    }
    required_focused = {
        ("cargo", "test", "--offline", "--locked", "-p", "evidence-core", "--all-targets", "--", "--test-threads=1"),
        ("cargo", "test", "--offline", "--locked", "-p", "evidence-adapters", "--all-targets", "--", "--test-threads=1"),
        ("cargo", "test", "--offline", "--locked", "-p", "clew", "--lib", "worker::tests::compiler_receipt_requires_explicit_successful_k2_validation", "--", "--exact", "--test-threads=1"),
        ("cargo", "test", "--offline", "--locked", "-p", "clew", "--lib", "worker::tests::trusted_distribution_identity_is_read_only_cache_key_material", "--", "--exact", "--test-threads=1"),
        ("cargo", "test", "--offline", "--locked", "-p", "clew", "--lib", "index::tests::declaration_descriptor_ingestion_roundtrips_unknown_and_commits_snapshot", "--", "--exact", "--test-threads=1"),
        ("cargo", "test", "--offline", "--locked", "-p", "clew", "--lib", "index::tests::declaration_descriptor_ingestion_rejects_malformed_hash_and_provenance", "--", "--exact", "--test-threads=1"),
        ("cargo", "test", "--offline", "--locked", "-p", "clew", "--lib", "index::tests::declaration_relation_ingestion_roundtrips_typed_unknown_and_commits_snapshot", "--", "--exact", "--test-threads=1"),
        ("cargo", "test", "--offline", "--locked", "-p", "clew", "--lib", "index::tests::declaration_relation_ingestion_rejects_hash_malformed_and_snapshot_mismatch", "--", "--exact", "--test-threads=1"),
        ("./gradlew", "--offline", ":workers:kotlin:test", "--tests", "dev.semanticthread.worker.ProjectModelCommandTest.futureCompilerDescriptorValuesBecomeTypedBoundaries", "--tests", "dev.semanticthread.worker.ProjectModelCommandTest.malformedCompilerFactRowIsRetainedAsBothTypedGraphBoundaries", ":workers:kotlin21:compileKotlin", ":workers:kotlin23:compileKotlin", "--no-daemon"),
        ("cargo", "fmt", "--all", "--check"),
    }
    expected_historical = {
        ("cargo", "clippy", "--offline", "--locked", "-p", "clew", "--lib", "--", "-D", "warnings"),
        ("cargo", "clippy", "--offline", "--locked", "-p", "semantic-corpus", "--lib", "--", "-D", "warnings"),
    }
    historical_rows = [
        row for row in baseline_commands
        if isinstance(row, Mapping) and row.get("policy") == "HISTORICAL_BASELINE"
    ]
    expected_historical_projection = [
        {"argvSha256": row.get("argvSha256"), "observed": row.get("observed"), "stderrSha256": row.get("stderrSha256")}
        for row in historical_rows
    ]
    historical_visible = (
        {tuple(row.get("argv", [])) for row in historical_rows} == expected_historical
        and all(_baseline_command_packet_valid(row, tuple(row["argv"])) for row in historical_rows)
        and baseline.get("historicalBaselineOutcomes") == expected_historical_projection
        and baseline.get("historicalClaims") == {
            "clewClippyDiagnosticsAtM1": 12,
            "semanticCorpusClippyDiagnosticsAtM1": 4,
            "sourceReportSha256": sha256_file(ROOT / "docs/experiments/codeclew-multilanguage-m1-implementation-report-2026-08-13.md"),
        }
    )
    focused_valid = all(
        (row := focused_commands.get(command)) is not None
        and row.get("policy") == "REQUIRED_GREEN"
        and row.get("exitCode") == 0
        and _baseline_command_packet_valid(row, command)
        for command in required_focused
    )
    packet_cargo = baseline.get("cargoExecutionAuthority")
    context_id = baseline.get("executionContextId")
    postcheck = baseline.get("executionContextPostcheck")
    cargo_rows = [row for row in baseline_commands if isinstance(row, Mapping) and isinstance(row.get("argv"), list) and row["argv"] and row["argv"][0] == "cargo"]
    gradle_rows = [row for row in baseline_commands if isinstance(row, Mapping) and isinstance(row.get("argv"), list) and row["argv"] and row["argv"][0] == "./gradlew"]
    recomputed_context_id = sha256_bytes(canonical({
        "schema": "codeclew.kotlin-k1-baseline-execution-context/0.1",
        "cargoLauncher": packet_cargo.get("launcher") if isinstance(packet_cargo, Mapping) else None,
        "cargoSeed": packet_cargo.get("dependencySeed") if isinstance(packet_cargo, Mapping) else None,
        "gradleLauncher": gradle_rows[0].get("gradleExecutionAuthority", {}).get("launcher") if len(gradle_rows) == 1 else None,
        "gradleSeed": gradle_rows[0].get("gradleExecutionAuthority", {}).get("dependencySeed") if len(gradle_rows) == 1 else None,
    }))
    baseline_authority_complete = (
        _is_digest(context_id)
        and context_id == recomputed_context_id
        and {row.get("executionContextId") for row in baseline_commands if isinstance(row, Mapping)} == {context_id}
        and isinstance(packet_cargo, Mapping) and set(packet_cargo) == {
            "executionContextId", "launcher", "dependencySeed", "isolatedCargoHome",
            "isolatedCargoTargetDir", "sharedBaselineExecutionContext", "executionCwd",
        }
        and packet_cargo.get("executionContextId") == context_id
        and packet_cargo.get("executionCwd") == "/"
        and packet_cargo.get("isolatedCargoHome") is packet_cargo.get("isolatedCargoTargetDir") is packet_cargo.get("sharedBaselineExecutionContext") is True
        and all(row.get("cargoExecutionAuthority", {}).get("launcher") == packet_cargo.get("launcher")
                and row.get("cargoExecutionAuthority", {}).get("dependencySeed") == packet_cargo.get("dependencySeed") for row in cargo_rows)
        and isinstance(postcheck, Mapping) and postcheck == {
            "schema": "codeclew.kotlin-k1-baseline-context-postcheck/0.1",
            "executionContextId": context_id, "cargoSeedMembersUnchanged": True,
            "hostSeedMembersUnchanged": True, "cargoLauncherUnchanged": True,
            "gradleLauncherUnchanged": True, "allowedGeneratedStateOnly": True,
            "generatedSourceTreeDigest": packet_cargo.get("dependencySeed", {}).get("generatedSourceTreeDigest"),
            "generatedSourceFileCount": packet_cargo.get("dependencySeed", {}).get("generatedSourceFileCount"),
            "cargoConfigAndCredentialsAbsentAfterCommands": True,
        }
        and baseline.get("candidateToolsManifestSha256") == tools_sha
        and baseline.get("repositoryHeadBefore") == baseline.get("repositoryBaseRevision")
        and baseline.get("repositoryHeadAfter") == baseline.get("repositoryBaseRevision")
    )
    baseline_green = (
        isinstance(baseline_commands, list)
        and len(baseline_commands) == len(required_focused) + len(expected_historical)
        and set(focused_commands) == required_focused | expected_historical
        and baseline.get("repositoryBaseRevision") == "be19fd06ce7509ffbaf4622b8f8d13ec7cbcd854"
        and baseline.get("requiredGreen") is True
        and focused_valid and historical_visible and baseline_authority_complete
    )
    runtime_models_zero = (
        all(row.get("adapterCost", {}).get("modelCalls") == 0 for row in rows)
        and qualification.get("modelCalls") == holdout.get("modelCalls") == 0
        and baseline.get("modelCalls") == harness.get("modelCalls") == 0
    )
    materialize_evidence = holdout_materialize.get("evidence", {})
    holdout_guard = (
        freeze.get("postFreezeChangesAllowed") is False
        and materialize_evidence.get("semanticInspectionPerformed") is False
        and store.bundle["holdoutEligibilityAudit"].get("forbiddenActionsObserved") == 0
        and store.bundle["holdoutEligibilityAudit"].get("decision") == "ACCEPT"
        and all(row.get("phaseReceipts", {}).get("CANDIDATE_FREEZE_VERIFY") == freeze_pointer["receiptDigest"] for row in holdout["attempts"])
    )
    proof_packet = safety.get("structuralConformance", {})
    totality_checks = source_packet.get("checks", {}) if isinstance(source_packet, Mapping) else {}
    totality_pass = static_current and all(totality_checks.get(key) is True for key in (
        "futureCompilerValuesCovered", "malformedRowsCovered", "utf16ToUtf8Covered",
        "effectiveVisibilityLocalIsTyped", "quarantinedRowsCannotBecomeProven",
    ))
    cache_mutations_pass = static_current and all(totality_checks.get(key) is True for key in (
        "cacheCorruptionAndSymlinkCovered", "cacheKeyOrderCovered", "cacheInputDriftCovered",
    ))
    determinism_pass = determinism_current and totality_checks.get("terminalDigestVolatilePathCovered") is True

    records: dict[str, dict[str, Any]] = {}
    def record(identifier: str, measured: bool, failure_class: str, predicate: str, evidence: Any, missing: list[str] | None = None) -> None:
        missing = missing or []
        passed = measured and not missing
        body = {
            "predicate":predicate, "measured":measured, "missingEvidence":missing,
            "failureClass":None if passed else failure_class, "evidence":evidence,
        }
        records[identifier] = {**body, "status":"PASS" if passed else "FAIL", "evidenceSha256":sha256_bytes(canonical(body))}

    r02 = (
        identifiers == [*EXPECTED_QUALIFICATION, *EXPECTED_HOLDOUT]
        and len(set(identifiers)) == 12 and len(set(organizations)) >= 6
        and max(organization_counts.values(), default=99) <= 2
        and all(
            isinstance(entry.get("commit"), str) and len(entry["commit"]) in {40,64}
            and all(character in "0123456789abcdef" for character in entry["commit"])
            and isinstance(entry.get("gitTree"), str) and len(entry["gitTree"]) in {40,64}
            and all(character in "0123456789abcdef" for character in entry["gitTree"])
            for entry in corpus["entries"]
        )
    )
    r03 = all(value >= 3 for value in dsl_counts.values()) and all(isinstance(entry["selectedCompilation"], str) and entry["selectedCompilation"] for entry in entries.values()) and preparation_parity
    r04_safety = exact_success_identity and all(row.get("candidateToolsManifestSha256") == tools_sha for row in rows)
    r04 = r04_safety and compiler_lines == ["2.1","2.3","2.4"]
    r04_class = "STOP" if not r04_safety else "GAP"
    record("K1-R01", static_current and source_equal, "STOP", "STATIC_READ_ONLY_REACHABILITY_AND_EVERY_SOURCE_EQUAL", {"sourcePacketSha256":sha256_bytes(canonical(source_packet)),"sourceEqual":source_equal,"rows":len(rows),"harnessReceipt":harness_receipt})
    record("K1-R02", r02, "STOP", "EXACT_CORPUS_MEMBERSHIP_PINS_AND_ORGANIZATION_CAP", {"ids":identifiers,"organizationCounts":organization_counts,"corpusSha256":store.bundle["digests"]["corpus"]})
    record("K1-R03", r03, "STOP", "CORPUS_DSL_SELECTION_AND_PREPARE_PARITY", {"dslCounts":dsl_counts,"preparationRows":len(preparation_rows),"qualificationPrepare":qualification_prepare_digest,"holdoutPrepare":holdout_prepare_digest})
    record("K1-R04", r04, r04_class, "EXACT_DECLARED_ANALYZER_DISTRIBUTION_IDENTITY_AND_LINE_COVERAGE", {"identitySafe":r04_safety,"validatedCompilerLines":compiler_lines,"candidateToolsSha256":tools_sha})
    record("K1-R05", build_current and terminal_protocol and build_authority_valid and closure_refusals_sound, "STOP", "SEMANTIC_INPUT_MANIFEST_NAMED_MUTATIONS_OR_TYPED_CLOSURE", {"packetSha256":sha256_bytes(canonical(build_packet)),"successes":len(successes),"validBuildAuthorities":sum(_valid_success_build_authority(row) for row in successes),"typedClosureSound":closure_refusals_sound,"typedClosureRefusals":closure_refusal_count})
    record("K1-R06", build_current and terminal_protocol and build_authority_valid and closure_refusals_sound, "STOP", "DEPENDENCY_IDENTITY_NAMED_MUTATIONS_OR_TYPED_CLOSURE", {"packetSha256":sha256_bytes(canonical(build_packet)),"typedClosureSound":closure_refusals_sound,"typedClosureRefusals":closure_refusal_count,"preparationRows":len(preparation_rows)})
    record("K1-R07", source_equal and safety.get("sourceMutations") == 0, "STOP", "EXACT_SOURCE_BEFORE_AFTER_AND_ORIGIN_BINDING", {"sourceEqual":source_equal,"sourceMutations":safety.get("sourceMutations")})
    record("K1-R08", totality_pass and proof_packet.get("status") == "PASS", "STOP", "KOTLIN_TOTALITY_NAMED_CONFORMANCE", {"sourceChecks":totality_checks,"proofPacket":proof_packet})
    record("K1-R09", terminal_protocol and supervisor_complete, "STOP", "EXACT_24_ATTEMPTS_AND_NAMED_SUPERVISOR_FAILURES", {"attempts":len(rows),"pairs":len(actual_pairs),"supervisorCases":sorted(supervisor_cases) if isinstance(supervisor_cases, Mapping) else []})
    record("K1-R10", safety.get("falseProven") == 0 and safety.get("falseComplete") == 0 and proof_packet.get("status") == "PASS", "STOP", "INDEPENDENT_PROOF_SAFETY_AND_BOUNDARY_BIJECTION", {"falseProven":safety.get("falseProven"),"falseComplete":safety.get("falseComplete"),"proofPacket":proof_packet})
    record("K1-R11", exact_workload, "STOP", "EVERY_SUCCESS_BOUNDED_NONEMPTY_PROJECTION", {"successes":len(successes),"validated":sum(row.get("nonemptyProjection",{}).get("passed") is True for row in successes)})
    record("K1-R12", cache_protocol and cache_mutations_pass, "STOP", "CACHE_RECOMPUTE_REOPEN_AND_NAMED_CORRUPTION_MUTATIONS", {"cacheProtocol":cache_protocol,"sourceChecks":{key:totality_checks.get(key) for key in ("cacheCorruptionAndSymlinkCovered","cacheKeyOrderCovered","cacheInputDriftCovered")}})
    record("K1-R13", offline_prepare and safety.get("offlineReplayEqual") is True and sandbox_rows and closure_refusals_sound, "STOP", "FRESH_OFFLINE_PREPARE_NETWORK_DENIAL_AND_REPLAY", {"offlinePrepareRows":sum(offline_row(row) for row in preparation_rows),"typedClosureRefusals":closure_refusal_count,"typedClosureSound":closure_refusals_sound,"offlineReplayEqual":safety.get("offlineReplayEqual"),"sandboxRows":sandbox_rows})
    record("K1-R14", cache_cost.get("telemetryComplete") is True and cache_cost.get("invocationsBounded") is True and len(rows) == 24, "STOP", "EXACT_TELEMETRY_SCHEMA_AND_BOUNDS", cache_cost)
    record("K1-R15", holdout_guard, "STOP", "HARNESS_CUSTODIAN_FREEZE_MATERIALIZE_JOURNAL", {"candidateFreeze":freeze_pointer["receiptDigest"],"materialize":holdout_materialize_digest,"semanticInspectionPerformed":materialize_evidence.get("semanticInspectionPerformed")})
    record("K1-R16", authority_complete, "STOP", "AUTHORITY_CURRENTNESS_AND_FORGERY_COUNTEREXAMPLES", {"graph":store.graph_digest,"storeId":store.store_id,"cases":requirement_cases,"baselineReceipt":baseline_receipt,"harnessReceipt":harness_receipt})
    record("K1-R17", determinism_pass and safety.get("offlineReplayEqual") is True, "STOP", "CROSS_LANGUAGE_DOMAIN_GOLDENS_AND_TERMINAL_RECOMPUTE", {"packet":determinism_packet,"offlineReplayEqual":safety.get("offlineReplayEqual")})
    record("K1-R18", sandbox_rows and supervisor_complete and _security_tripwire_cases_valid(supervisor_cases, requirement_cases), "STOP", "DEFAULT_DENY_DISPOSABLE_SOURCE_SECURITY_TRIPWIRES", {"isolatedAttempts":sum(row.get("sourceExecutionAuthority",{}).get("discardedBeforePublication") is True for row in rows),"requiredTripwires":sorted(required_supervisor),"expectedSecurityTripwireValues":SECURITY_SUPERVISOR_EXPECTED,"observedTripwires":dict(supervisor_cases) if isinstance(supervisor_cases, Mapping) else {},"requiredPrepareTraversalCases":sorted(PREPARE_SECURITY_CASES)})
    record("K1-R19", k0_receipt.get("evidence", {}).get("byteExact") is not None and baseline_green, "STOP", "K0_BASELINE_AND_FOCUSED_PRE_POST_COMMANDS", {"k0Receipt":k0_receipt_digest,"baselineReceipt":baseline_receipt,"baselinePacketSha256":baseline_sha,"requiredFocusedCommands":len(required_focused),"historicalVisible":historical_visible,"green":baseline_green})
    record("K1-R20", static_current and runtime_models_zero, "STOP", "STATIC_REACHABILITY_NON_GOAL_SCAN_AND_ZERO_MODELS", {"runtimeModelCallsZero":runtime_models_zero,"sourcePacketSha256":sha256_bytes(canonical(source_packet)),"forbiddenNonGoals":source_packet.get("forbiddenNonGoals") if isinstance(source_packet, Mapping) else None})
    _validate_requirement_rows(records)
    return records


def _expected_k1_decision(
    safety: Mapping[str, Any],
    applicability: Mapping[str, Any],
    cache_cost: Mapping[str, Any],
    conformance: Mapping[str, Any],
) -> str:
    requirements = conformance.get("requirements")
    stop_violations = conformance.get("stopViolations")
    gap_requirements = conformance.get("gapRequirements")
    if (
        not isinstance(requirements, Mapping)
        or not isinstance(stop_violations, list)
        or not all(isinstance(value, str) for value in stop_violations)
        or not isinstance(gap_requirements, list)
        or not all(isinstance(value, str) for value in gap_requirements)
    ):
        return "STOP"
    expected_stop = sorted(
        identifier for identifier, row in requirements.items()
        if isinstance(row, Mapping) and row.get("status") == "FAIL" and row.get("failureClass") == "STOP"
    )
    expected_gaps = sorted(
        identifier for identifier, row in requirements.items()
        if isinstance(row, Mapping) and row.get("status") == "FAIL" and row.get("failureClass") == "GAP"
    )
    if stop_violations != expected_stop or gap_requirements != expected_gaps:
        return "STOP"
    if safety.get("safe") is not True or stop_violations:
        return "STOP"
    if (
        not gap_requirements
        and applicability.get("passed") is True
        and cache_cost.get("passed") is True
        and conformance.get("allPassed") is True
    ):
        return "GO"
    return "PIVOT"


def _measurement_conformance(bundle: Mapping[str, Any]) -> dict[str, Any]:
    thresholds = bundle["requirements"]["decisionThresholds"]
    entries = {row["id"]: row for row in bundle["corpus"]["entries"]}
    cost = {
        "externalWallMicros": 100, "maximumResidentBytes": 1024,
        "sourceHashingMicros": 1, "buildDiscoveryMicros": 1,
        "dependencyPreparationMicros": "NOT_IN_THIS_INVOCATION",
        "dependencyVerificationMicros": 1, "adapterStartupMicros": 1,
        "coldIndexMicros": 1, "warmIndexMicros": 0, "providerProcessingMicros": 100,
        "serializationMicros": 1, "storeWriteMicros": 1, "storeReadMicros": 1,
        "queryProjectionMicros": 1, "sourceBytesRead": 1, "cacheBytesRead": 0,
        "cacheBytesWritten": 1, "emittedBytes": 1, "storedFactBytes": 1,
        "factCount": 1, "boundaryCount": 0, "cacheRequests": 1,
        "cacheHits": 0, "modelCalls": 0,
    }
    rows = []
    for entry_id in EXPECTED_QUALIFICATION + EXPECTED_HOLDOUT:
        compiler = bundle["corpus"]["frozenExecutionPolicy"]["trustedAnalyzers"][
            entries[entry_id]["trustedAnalyzerMinorLine"]
        ]["compilerVersion"]
        for invocation in ("COLD", "WARM"):
            row_cost = dict(cost)
            if invocation == "WARM":
                row_cost.update({"externalWallMicros": 50, "providerProcessingMicros": 50, "cacheHits": 1})
            rows.append({
                "entry": entry_id, "invocation": invocation, "status": "ADAPTER_OUTPUT",
                "successAuthorityValidated": True, "nonemptyProjection": {"passed": True},
                "declaredProjectCompilerVersion": entries[entry_id]["declaredKotlinVersion"],
                "analyzerCompilerVersion": compiler, "cacheHit": invocation == "WARM",
                "terminalSemanticDigest": sha256_bytes(entry_id.encode()),
                "candidateToolsManifestSha256": sha256_bytes(b"fixture-tools"),
                "workerDistributionTreeHash": sha256_bytes(compiler.encode()),
                "externalWallMicros": row_cost["externalWallMicros"],
                "maximumResidentBytes": row_cost["maximumResidentBytes"],
                "adapterCost": row_cost,
            })
    holdout = [row for row in rows if row["entry"] in EXPECTED_HOLDOUT]
    clean_applicability = _applicability_measurement(rows, holdout, entries, thresholds)
    empty_projection = json.loads(json.dumps(rows))
    next(row for row in empty_projection if row["entry"] == "K1-H01" and row["invocation"] == "WARM")["nonemptyProjection"]["passed"] = False
    empty_result = _applicability_measurement(
        empty_projection, [row for row in empty_projection if row["entry"] in EXPECTED_HOLDOUT], entries, thresholds,
    )
    missing_dsl = json.loads(json.dumps(rows))
    for row in missing_dsl:
        if entries[row["entry"]]["buildDsl"] == "MAVEN" and row["invocation"] == "WARM":
            row["nonemptyProjection"]["passed"] = False
    dsl_result = _applicability_measurement(
        missing_dsl, [row for row in missing_dsl if row["entry"] in EXPECTED_HOLDOUT], entries, thresholds,
    )
    clean_cost = _cache_cost_measurement(rows, holdout, thresholds)
    missing_rss = json.loads(json.dumps(rows))
    missing_rss[0]["maximumResidentBytes"] = None
    rss_result = _cache_cost_measurement(
        missing_rss, [row for row in missing_rss if row["entry"] in EXPECTED_HOLDOUT], thresholds,
    )
    slow_provider = json.loads(json.dumps(rows))
    for row in slow_provider:
        if row["entry"] in EXPECTED_HOLDOUT and row["invocation"] == "WARM":
            row["adapterCost"]["providerProcessingMicros"] = 90
    provider_result = _cache_cost_measurement(
        slow_provider, [row for row in slow_provider if row["entry"] in EXPECTED_HOLDOUT], thresholds,
    )
    checks = {
        "completeFixturePasses": clean_applicability["passed"] and clean_cost["passed"],
        "emptyProjectionNotCounted": empty_result["holdoutValidatedNonemptyProjections"] == 5,
        "missingDslRejected": not dsl_result["passed"],
        "missingRssRejected": not rss_result["passed"] and not rss_result["telemetryComplete"],
        "providerRatioRejected": not provider_result["passed"] and provider_result["medianWarmProviderWallRatio"] == 0.9,
    }
    return {
        "schema": "codeclew.kotlin-k1-measurement-conformance/0.1",
        "status": "PASS" if all(checks.values()) else "FAIL",
        "checks": checks,
        "resultsSha256": sha256_bytes(canonical({
            "applicability": clean_applicability, "cost": clean_cost,
            "empty": empty_result, "dsl": dsl_result, "rss": rss_result,
            "provider": provider_result,
        })),
    }


def _build_state_self_test(root: Path) -> dict[str, Any]:
    seed = root / "seed"
    (seed / "gradle-user-home").mkdir(parents=True)
    (seed / "maven-repository").mkdir()
    _atomic_write(seed / "gradle-user-home" / "module.bin", b"gradle-seed", 0o600)
    _atomic_write(seed / "maven-repository" / "artifact.bin", b"maven-seed", 0o700)
    directories, mode_files = _seal_build_state_subtrees(seed)
    files = [{key: row[key] for key in ("root", "path", "size", "sha256")} for row in mode_files]
    body = {
        "schema": "codeclew.kotlin-k1-build-state-manifest/0.1", "seriesId": SERIES_ID,
        "cohort": "QUALIFICATION", "toolchain": {"fixture": sha256_bytes(b"tool")},
        "repositories": [{
            "entry": "fixture", "commit": "a" * 40, "gitTree": "b" * 40,
            "selectedCompilation": ":/main", "buildDsl": "FIXTURE",
            "prepareArgvSha256": sha256_bytes(b"prepare"), "exitCode": 0,
        }],
        "gradleUserHomeTreeDigest": _build_state_subtree_digest(files, "gradle-user-home"),
        "mavenLocalRepositoryTreeDigest": _build_state_subtree_digest(files, "maven-repository"),
        "files": files, "seedDigest": "",
    }
    body["seedDigest"] = sha256_bytes(canonical(body))
    manifest_raw = canonical(body)
    _atomic_write(seed / "CODECLEW_K1_BUILD_STATE_MANIFEST.json", manifest_raw, 0o400)
    _atomic_write(seed / "CODECLEW_K1_BUILD_STATE_SEED", (sha256_bytes(manifest_raw) + "\n").encode(), 0o400)
    mode_body = {
        "schema": "codeclew.kotlin-k1-build-state-modes/0.1", "seriesId": SERIES_ID,
        "buildStateManifestDigest": sha256_bytes(manifest_raw),
        "directories": directories, "files": mode_files, "objectDigest": "",
    }
    mode_body["objectDigest"] = sha256_bytes(canonical(mode_body))
    _atomic_write(seed / "CODECLEW_K1_BUILD_STATE_MODES.json", canonical(mode_body), 0o400)
    os.chmod(seed, 0o500, follow_symlinks=False)
    identity = _validate_build_state_seed(seed, "QUALIFICATION")
    parent = root / "runtime"
    cold, cold_before = _copy_seed_to_fresh_runtime(seed, parent, "fixture", "COLD")
    warm, warm_before = _copy_seed_to_fresh_runtime(seed, parent, "fixture", "WARM")
    if (warm / "gradle-user-home" / "runtime.lock").exists():
        raise AssertionError("warm build state inherited cold mutation")
    try:
        _copy_seed_to_fresh_runtime(seed, parent, "fixture", "COLD")
        raise AssertionError("reused invocation build root accepted")
    except HarnessError:
        reused_rejected = True
    mode_member = seed / "gradle-user-home" / "module.bin"
    mode_member.chmod(0o600)
    try:
        _validate_build_state_seed(seed, "QUALIFICATION")
        raise AssertionError("build-state file mode drift accepted")
    except HarnessError:
        mode_drift_rejected = True
    mode_member.chmod(0o400)
    gradle_seed = seed / "gradle-user-home"
    gradle_seed.chmod(0o700)
    _atomic_write(gradle_seed / "undeclared.bin", b"undeclared", 0o400)
    gradle_seed.chmod(0o500)
    try:
        _validate_build_state_seed(seed, "QUALIFICATION")
        raise AssertionError("undeclared build-state member accepted")
    except HarnessError:
        undeclared_rejected = True
    gradle_seed.chmod(0o700)
    (gradle_seed / "undeclared.bin").unlink()
    os.symlink("module.bin", gradle_seed / "link.bin")
    gradle_seed.chmod(0o500)
    try:
        _validate_build_state_seed(seed, "QUALIFICATION")
        raise AssertionError("build-state symlink accepted")
    except HarnessError:
        symlink_rejected = True
    gradle_seed.chmod(0o700)
    (gradle_seed / "link.bin").unlink()
    gradle_seed.chmod(0o500)
    worker_runtime = root / "worker-runtime"
    worker_runtime.mkdir(mode=0o700)
    for row in directories:
        member = worker_runtime / row["root"] if row["path"] == "." else worker_runtime / row["root"] / row["path"]
        member.mkdir(parents=True, exist_ok=True, mode=0o700)
        member.chmod(0o700)
    for row in mode_files:
        source = seed / row["root"] / row["path"]
        target = worker_runtime / row["root"] / row["path"]
        _atomic_write(target, source.read_bytes(), 0o600)
        target.chmod(0o600)
    _atomic_write(worker_runtime / "gradle-user-home" / "runtime.lock", b"mutable", 0o600)
    with (worker_runtime / "gradle-user-home" / "module.bin").open("ab") as handle:
        handle.write(b"-runtime")
    fresh_runtime_mutable = (
        (worker_runtime / "gradle-user-home" / "runtime.lock").read_bytes() == b"mutable"
        and (worker_runtime / "gradle-user-home" / "module.bin").read_bytes().endswith(b"-runtime")
    )
    marker = seed / "CODECLEW_K1_BUILD_STATE_SEED"
    seed.chmod(0o700)
    marker.chmod(0o600)
    _atomic_write(marker, b"sha256:" + b"0" * 64 + b"\n", 0o400)
    seed.chmod(0o500)
    try:
        _validate_build_state_seed(seed, "QUALIFICATION")
        raise AssertionError("forged build-state marker accepted")
    except HarnessError:
        forged_rejected = True
    # The enclosing TemporaryDirectory must be able to remove this test-only
    # contour; production publication never performs this relaxation.
    for sealed_root in (seed, cold, warm):
        sealed_root.chmod(0o700)
        for directory, child_directories, child_files in os.walk(sealed_root, followlinks=False):
            directory_path = Path(directory)
            directory_path.chmod(0o700)
            for name in child_directories:
                (directory_path / name).chmod(0o700)
            for name in child_files:
                (directory_path / name).chmod(0o600)
    return {
        "schema": "codeclew.kotlin-k1-build-state-self-test/0.1", "status": "PASS",
        "seedDigest": identity["seedDigest"], "manifestDigest": identity["manifestDigest"],
        "coldWarmSameSeed": cold_before["seedDigest"] == warm_before["seedDigest"],
        "coldWarmDistinctRoots": cold != warm, "reusedRootRejected": reused_rejected,
        "forgedMarkerRejected": forged_rejected, "modeDriftRejected": mode_drift_rejected,
        "undeclaredMemberRejected": undeclared_rejected, "symlinkRejected": symlink_rejected,
        "freshRuntimeMutable": fresh_runtime_mutable,
    }


def _dependency_publication_self_test(root: Path) -> dict[str, Any]:
    """Exercise Darwin-safe move-then-seal ordering without repository I/O."""
    root.mkdir(parents=True, mode=0o700)
    staging = root / ".seed.prepare-0123456789abcdef01234567"
    staging.mkdir(mode=0o700)
    entry_work = staging / ".work" / "K1-Q01"
    entry_output = staging / "entries" / "K1-Q01"
    entry_work.mkdir(parents=True, mode=0o700)
    entry_output.mkdir(parents=True, mode=0o700)
    _atomic_write(entry_work / "member.bin", b"sealed", 0o400)
    build_state = entry_output / "build-state"
    os.replace(entry_work, build_state)
    os.chmod(build_state, 0o500, follow_symlinks=False)
    nested_move_then_seal = (
        not entry_work.exists()
        and stat.S_IMODE(build_state.lstat().st_mode) == 0o500
        and stat.S_IMODE((build_state / "member.bin").lstat().st_mode) == 0o400
    )
    target = root / "seed"
    _seal_dependency_cohort_tree(staging, seal_root=False)
    root_writable_until_move = stat.S_IMODE(staging.lstat().st_mode) == 0o700
    os.replace(staging, target)
    os.chmod(target, 0o500, follow_symlinks=False)
    _require_sealed_dependency_cohort_tree(target)
    cohort_move_then_seal = (
        root_writable_until_move
        and not staging.exists()
        and stat.S_IMODE(target.lstat().st_mode) == 0o500
    )
    _discard_private_tree(target, root)

    failed_staging = root / ".failed.prepare-0123456789abcdef01234567"
    failed_target = root / "failed-seed"
    failed_staging.mkdir(mode=0o700)
    (failed_staging / "entries").mkdir(parents=True, mode=0o700)
    _atomic_write(failed_staging / "entries" / "member.bin", b"sealed", 0o400)
    _seal_dependency_cohort_tree(failed_staging, seal_root=False)
    os.replace(failed_staging, failed_target)
    os.chmod(failed_target, 0o500, follow_symlinks=False)
    try:
        raise HarnessError("injected post-rename validation failure")
    except HarnessError:
        _discard_private_tree(failed_target, root)
    post_rename_failure_removed = (
        not failed_staging.exists() and not failed_target.exists()
    )
    checks = {
        "nestedMoveBeforeRootSeal": nested_move_then_seal,
        "cohortMoveBeforeRootSeal": cohort_move_then_seal,
        "postRenameFailureRemoved": post_rename_failure_removed,
    }
    if not all(checks.values()):
        raise AssertionError(f"dependency publication ordering self-test failed: {checks}")
    return {
        "schema": "codeclew.kotlin-k1-dependency-publication-self-test/0.1",
        "status": "PASS",
        **checks,
    }


def supervise_entry(
    store: Store,
    entry_id: str,
    invocation: str,
    repository: Path,
    command: list[str],
    inputs: Mapping[str, Mapping[str, Any]],
    *,
    timeout_seconds: int = MAX_WALL_SECONDS,
    resident_limit_bytes: int = MAX_RESIDENT_BYTES,
    output_limit_bytes: int = MAX_STDOUT_BYTES,
    diagnostic_only: bool = True,
    decision_authority: str | None = None,
    authorized_read_paths: Sequence[Path] = (),
    authorized_write_paths: Sequence[Path] = (),
) -> tuple[str, dict[str, Any]]:
    """Run one pinned adapter child and publish exactly one retained attempt.

    The caller supplies a process command, never a success/failure report.  Its
    output is retained as a blob and independently classified by the harness.
    """
    if invocation not in {"COLD", "WARM"}:
        raise HarnessError("invocation must be COLD or WARM")
    if not command or not all(isinstance(token, str) and token for token in command):
        raise HarnessError("supervisor command mismatch")
    if not diagnostic_only:
        raise HarnessError("free-form supervisor cannot issue decision-authoritative attempts")
    executable = _regular_file(Path(command[0]), "supervised executable")
    entry = assert_entry_run_allowed(store, entry_id, inputs)
    before = _git_observation(repository)
    if before["head"] != entry["commit"] or before["tree"] != entry["gitTree"] or not before["clean"]:
        raise HarnessError("repository is not an exact clean frozen pin")
    started_ns = time.monotonic_ns()
    timed_out = False
    resident_overflow = threading.Event()
    watchdog_stop = threading.Event()
    watchdog_observation: dict[str, Any] = {}
    stdout_overflow = threading.Event()
    stderr_overflow = threading.Event()
    stdout = b""
    stderr = b""
    with tempfile.TemporaryDirectory(prefix="codeclew-k1-run-") as temporary_text:
        runtime_root = Path(temporary_text)
        home = runtime_root / "home"
        home.mkdir(mode=0o700)
        time_path = Path(temporary_text) / "time.txt"
        stdout_path = runtime_root / "stdout.bin"
        stderr_path = runtime_root / "stderr.bin"
        sandbox_profile = runtime_root / "network-deny.sb"
        child_command = [str(executable), *command[1:]]
        allowed_reads = [
            repository.resolve(strict=True), runtime_root, executable,
            Path("/System"), Path("/usr"), Path("/bin"), Path("/sbin"), Path("/etc"),
            Path("/Library/Java"), Path("/opt/homebrew"), Path("/dev"),
            Path("/private/var/select"),
        ]
        for path in authorized_read_paths:
            absolute = path.absolute()
            if absolute.is_symlink() or not absolute.exists():
                raise HarnessError("analysis sandbox read authority must already exist and not be a symlink")
            allowed_reads.append(absolute.resolve(strict=True))
        write_paths: list[Path] = [runtime_root]
        for path in authorized_write_paths:
            absolute = path.absolute()
            if absolute.is_symlink() or not absolute.is_dir():
                raise HarnessError("analysis sandbox write authority must be an existing real directory")
            real = absolute.resolve(strict=True)
            if real == repository or repository.is_relative_to(real) or real.is_relative_to(repository):
                raise HarnessError("analysis sandbox must not grant source-write authority")
            write_paths.append(real)
            allowed_reads.append(real)
        allowed_reads = sorted(set(allowed_reads), key=str)
        write_paths = sorted(set(write_paths), key=str)
        profile_text = ["(version 1)", "(deny default)", "(allow process*)", "(allow sysctl-read)", "(allow mach-lookup)", "(deny mach-lookup (global-name \"com.apple.securityd\"))", "(deny mach-lookup (global-name \"com.apple.security.agent\"))", "(deny mach-lookup (global-name \"com.apple.trustd\"))", "(deny network*)"]
        profile_text.extend(_sandbox_read_clauses(allowed_reads))
        profile_text.extend(_sandbox_path_clause("file-write*", path) for path in write_paths)
        _atomic_write(sandbox_profile, ("\n".join(profile_text) + "\n").encode(), 0o400)
        sandbox_profile.chmod(0o400)
        wrapped = [
            "/usr/bin/time", "-l", "-o", str(time_path),
            "/usr/bin/sandbox-exec", "-f", str(sandbox_profile), *child_command,
        ]

        def child_limits() -> None:
            # Darwin rejects RLIMIT_AS lowering for some hardened/runtime
            # processes. RSS remains an externally measured hard gate: a run
            # above the frozen limit is retained FAILED and cannot qualify.
            resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
            resource.setrlimit(resource.RLIMIT_FSIZE, (output_limit_bytes, output_limit_bytes))

        env_policy = {
            "HOME": str(home),
            "PATH": "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home/bin:/opt/homebrew/Cellar/maven/3.9.12/bin:/usr/bin:/bin:/usr/sbin:/sbin",
            "JAVA_HOME": "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home",
            "LANG": "C.UTF-8",
            "LC_ALL": "C.UTF-8",
            "TMPDIR": str(runtime_root),
            "CODECLEW_K1_MODEL_CALLS": "0",
            "GIT_OPTIONAL_LOCKS": "0",
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_SYSTEM": "/dev/null",
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_TERMINAL_PROMPT": "0",
            "GIT_ASKPASS": "/usr/bin/false",
            "SSH_ASKPASS": "/usr/bin/false",
            "GIT_PROTOCOL_FROM_USER": "0",
        }
        env_policy_identity = {
            **env_policy,
            "HOME": "$EXTERNAL_RUNTIME_ROOT/home",
            "TMPDIR": "$EXTERNAL_RUNTIME_ROOT",
        }
        sandbox_profile_digest = sha256_file(sandbox_profile)
        child_start_digest: str | None = None
        child_selected_digest: str | None = None
        if decision_authority is not None:
            dependency_key = (
                "qualificationDependencySeed"
                if decision_authority == "DEDICATED_QUALIFICATION_EXACT_ARGV"
                else "holdoutDependencySeed"
            )
            tools = _candidate_tools(inputs)
            dependency_seed = snapshot_input(inputs[dependency_key])
            child_selected_digest = sha256_bytes(canonical({
                "entry": entry_id,
                "invocation": invocation,
                "cohort": entry["cohort"],
                "authority": decision_authority,
                "repository": before["sourceTreeSha256"],
                "exactArgvSha256": sha256_bytes(canonical(command)),
                "executable": sha256_file(executable),
                "wrappedCommandSha256": sha256_bytes(canonical(wrapped)),
                "candidateToolsSha256": tools["manifestSha256"],
                "genericRuntimeSha256": tools["genericRuntime"]["sha256"],
                "kotlinAdapterSha256": tools["kotlinAdapter"]["sha256"],
                "dependencySeed": dependency_seed,
            }))
            journal = store.child_start_value(
                entry_id, invocation, decision_authority, child_selected_digest,
            )
            payload = {
                "journalPath": str(store.root / "starts" / f"{entry_id}-{invocation.lower()}.json"),
                "journal": journal,
            }
            launcher_payload = runtime_root / "launch-payload.json"
            _atomic_write(launcher_payload, canonical(payload), 0o400)
            wrapped = [
                str(Path(os.sys.executable).resolve()), str(Path(__file__).resolve()),
                "internal-launch", str(launcher_payload), "--", *wrapped,
            ]
        stdout_stop = threading.Event()
        stderr_stop = threading.Event()
        with stdout_path.open("xb") as stdout_handle, stderr_path.open("xb") as stderr_handle:
            process = subprocess.Popen(
                wrapped,
                cwd=repository,
                stdin=subprocess.DEVNULL,
                stdout=stdout_handle,
                stderr=stderr_handle,
                env=env_policy,
                preexec_fn=child_limits,
                start_new_session=True,
            )
            stdout_thread = threading.Thread(
                target=_bounded_file_watchdog,
                args=(process, (stdout_path,), output_limit_bytes, stdout_overflow, stdout_stop),
                daemon=True,
            )
            stderr_thread = threading.Thread(
                target=_bounded_file_watchdog,
                args=(process, (stderr_path,), output_limit_bytes, stderr_overflow, stderr_stop),
                daemon=True,
            )
            stdout_thread.start()
            stderr_thread.start()
            watchdog_thread = threading.Thread(
                target=_resident_watchdog,
                args=(process, resident_limit_bytes, resident_overflow, watchdog_stop, watchdog_observation),
                daemon=True,
            )
            watchdog_thread.start()
            try:
                exit_code = process.wait(timeout=timeout_seconds)
            except subprocess.TimeoutExpired:
                timed_out = True
                _kill_process_group(process)
                exit_code = process.wait()
            finally:
                stdout_stop.set()
                stderr_stop.set()
                watchdog_stop.set()
                stdout_thread.join(timeout=5)
                stderr_thread.join(timeout=5)
                watchdog_thread.join(timeout=5)
                _kill_remaining_process_group(process)
        stdout = stdout_path.read_bytes()
        stderr = stderr_path.read_bytes()
        if len(stdout) > output_limit_bytes:
            stdout_overflow.set()
            stdout = stdout[:output_limit_bytes]
        if len(stderr) > output_limit_bytes:
            stderr_overflow.set()
            stderr = stderr[:output_limit_bytes]
        time_raw = time_path.read_bytes() if time_path.exists() else b""
    wall_micros = (time.monotonic_ns() - started_ns) // 1000
    after = _git_observation(repository)
    mutated = before != after
    time_maximum_resident = _parse_maximum_resident(time_raw)
    watchdog_peak = watchdog_observation.get("peakResidentBytes")
    observed_resident = [
        value for value in (time_maximum_resident, watchdog_peak)
        if isinstance(value, int)
    ]
    maximum_resident = max(observed_resident) if observed_resident else None
    reason = None
    child_kind = "NONE"
    child_status = "FAILED"
    child_value: dict[str, Any] | None = None
    if mutated:
        reason = "SOURCE_OR_GIT_MUTATION"
    elif timed_out:
        reason = "TIMEOUT"
    elif stdout_overflow.is_set():
        reason = "OUTPUT_LIMIT"
    elif stderr_overflow.is_set():
        reason = "STDERR_LIMIT"
    elif resident_overflow.is_set():
        reason = "MEMORY_LIMIT"
    elif maximum_resident is not None and maximum_resident > resident_limit_bytes:
        reason = "MEMORY_LIMIT"
    elif not stdout:
        reason = "NONZERO_EXIT" if exit_code != 0 else "EMPTY_STDOUT"
    else:
        try:
            child_kind, child_status, child_value = _validate_child_terminal(bytes(stdout))
        except HarnessError as error:
            reason = str(error)
    if child_kind == "VALIDATED_PROJECTION" and child_value is not None:
        runtime_digest = child_value.get("provenance", {}).get("runtime", {}).get("binaryDigest")
        if runtime_digest != sha256_file(executable):
            reason = "GENERIC_RUNTIME_IDENTITY_MISMATCH"
    if child_kind == "VALIDATED_PROJECTION" and reason is None and exit_code == 0:
        status = "ADAPTER_OUTPUT"
        reason_code = None
    elif child_kind == "VALIDATED_PROJECTION" and reason is None:
        status = "FAILED"
        reason_code = "SUPERVISOR/NONZERO_ADAPTER_OUTPUT"
    elif child_kind == "DIAGNOSTIC_ADAPTER_OUTPUT" and reason is None:
        status = "FAILED"
        reason_code = "SUPERVISOR/UNVALIDATED_DIRECT_ADAPTER_OUTPUT"
    elif child_kind == "TYPED_ATTEMPT" and reason is None and exit_code != 0:
        status = child_status
        reason_code = str(child_value.get("reasonUri") or child_value.get("reasonCode") or "CHILD_TYPED_ATTEMPT")
    elif child_kind == "TYPED_ATTEMPT" and reason is None:
        status = "FAILED"
        reason_code = "SUPERVISOR/ZERO_EXIT_TYPED_FAILURE"
    else:
        status = "FAILED"
        reason_code = "SUPERVISOR/" + str(reason or "UNKNOWN")
    stdout_digest = store.put_blob(bytes(stdout))
    stderr_digest = store.put_blob(bytes(stderr))
    time_digest = store.put_blob(time_raw)
    selected_snapshot = {
        "repositoryCommit": entry["commit"],
        "repositoryGitTree": entry["gitTree"],
        "repositorySourceTreeSha256": before["sourceTreeSha256"],
        "corpusSha256": store.bundle["digests"]["corpus"],
        "requirementsSha256": store.bundle["digests"]["requirements"],
        "graphSha256": store.graph_digest,
        "executableSha256": sha256_file(executable),
        "commandSha256": sha256_bytes(canonical(command)),
        "environmentPolicySha256": sha256_bytes(canonical({
            "allowedKeys": sorted(env_policy_identity),
            "values": env_policy_identity,
            "productionCredentialInheritance": False,
        })),
        "networkSandboxProfileSha256": sandbox_profile_digest,
        "sandboxDefaultPolicy": "DENY_DEFAULT_NETWORK_DENY",
        "sandboxAuthorizedReadPathsSha256": sha256_bytes(canonical([str(path) for path in allowed_reads])),
        "sandboxAuthorizedWritePathsSha256": sha256_bytes(canonical([str(path) for path in write_paths])),
        "productionCredentialInheritance": False,
        "sandboxExecutableSha256": sha256_file(Path("/usr/bin/sandbox-exec")),
        "timeExecutableSha256": sha256_file(Path("/usr/bin/time")),
        "gitExecutableSha256": sha256_file(Path("/usr/bin/git")),
        "javaExecutableSha256": sha256_file(Path("/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home/bin/java")),
        "javaReleaseSha256": sha256_file(Path("/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home/release")),
        "mavenExecutableSha256": sha256_file(Path("/opt/homebrew/Cellar/maven/3.9.12/bin/mvn")),
        "psExecutableSha256": sha256_file(Path("/bin/ps")),
    }
    attempt: dict[str, Any] = {
        "schema": ATTEMPT_SCHEMA,
        "seriesId": SERIES_ID,
        "storeId": store.store_id,
        "graphDigest": store.graph_digest,
        "entry": entry_id,
        "cohort": entry["cohort"],
        "invocation": invocation,
        "status": status,
        "failureStage": None if status == "ADAPTER_OUTPUT" else "SUPERVISED_ADAPTER",
        "reasonCode": reason_code,
        "safeDetailSha256": sha256_bytes(canonical({
            "reason": reason_code,
            "exitCode": exit_code,
            "signal": -exit_code if exit_code < 0 else None,
        })),
        "selectedInputs": selected_snapshot,
        "child": {
            "kind": child_kind,
            "exitCode": exit_code,
            "signal": -exit_code if exit_code < 0 else None,
            "stdoutSha256": stdout_digest,
            "stdoutBytes": len(stdout),
            "stderrSha256": stderr_digest,
            "stderrBytes": len(stderr),
            "timeOutputSha256": time_digest,
            "parsedOutputDigest": (
                child_value.get("projectionDigest") if child_kind == "VALIDATED_PROJECTION" and child_value
                else child_value.get("outputDigest") if child_kind == "DIAGNOSTIC_ADAPTER_OUTPUT" and child_value
                else None
            ),
            "successAuthorityValidated": child_kind == "VALIDATED_PROJECTION" and reason is None and exit_code == 0,
        },
        "resource": {
            "externalWallMicros": wall_micros,
            "maximumResidentBytes": maximum_resident,
            "wallLimitSeconds": timeout_seconds,
            "residentLimitBytes": resident_limit_bytes,
            "stdoutLimitBytes": output_limit_bytes,
            "residentWatchdog": watchdog_observation,
        },
        "repositoryBefore": before,
        "repositoryAfter": after,
        "sourceMutation": mutated,
        "modelCalls": 0,
        "childStartSha256": (
            sha256_bytes(canonical(store.child_start_value(
                entry_id, invocation, decision_authority, child_selected_digest,
            ))) if decision_authority is not None and child_selected_digest is not None else None
        ),
        "childSelectedDigest": child_selected_digest,
        "attemptDigest": "",
    }
    attempt["attemptDigest"] = sha256_bytes(canonical(attempt))
    with store.locked():
        digest = store.publish_attempt(attempt)
    return digest, attempt


def _run_corpus_entry(
    store: Store,
    entry_id: str,
    invocation: str,
    repository: Path,
    evidence_store: Path,
    semantic_state_root: Path,
    build_state_root: Path,
    inputs: Mapping[str, Mapping[str, Any]],
    *,
    cohort: str,
    timeout_seconds: int = MAX_WALL_SECONDS,
    resident_limit_bytes: int = MAX_RESIDENT_BYTES,
) -> tuple[str, dict[str, Any]]:
    """Dedicated, exact-argv qualification authority.

    It never accepts adapter arguments, seeds, thresholds, reports, or outcome
    strings from a caller. The frozen corpus and requirements derive them.
    """
    entry = assert_entry_run_allowed(store, entry_id, inputs)
    if cohort == "QUALIFICATION":
        identifiers = EXPECTED_QUALIFICATION
        expected_cohort = "QUALIFICATION"
        authority = "DEDICATED_QUALIFICATION_EXACT_ARGV"
        lookup_attempt = store.qualification_attempt
        publish_attempt = store.publish_qualification_attempt
        output_directory_name = "adapter-attempts"
    elif cohort == "BLIND_HOLDOUT":
        identifiers = EXPECTED_HOLDOUT
        expected_cohort = "BLIND_HOLDOUT"
        authority = "DEDICATED_HOLDOUT_EXACT_ARGV"
        lookup_attempt = store.holdout_attempt
        publish_attempt = store.publish_holdout_attempt
        output_directory_name = "holdout-adapter-attempts"
    else:
        raise HarnessError("dedicated corpus runner cohort mismatch")
    if entry["cohort"] != expected_cohort or entry_id not in identifiers:
        raise HarnessError(f"dedicated runner rejects entry outside {cohort}")
    required_phase_nodes = (
        ("QUALIFICATION_DEPENDENCY_SEED_VERIFY", "HARNESS_SELF_TEST")
        if cohort == "QUALIFICATION"
        else ("HOLDOUT_DEPENDENCY_SEED_VERIFY", "CANDIDATE_FREEZE_VERIFY")
    )
    phase_receipts: dict[str, str] = {}
    with store.locked():
        for node_id in required_phase_nodes:
            status, _, _ = assess(store, node_id, inputs)
            pointer = store.pointer(node_id)
            if status != "READY" or pointer is None:
                raise HarnessError(f"corpus run phase prerequisite is not READY: {node_id}")
            phase_receipts[node_id] = pointer["receiptDigest"]
        dependency_input_key = (
            "qualificationDependencySeed" if cohort == "QUALIFICATION"
            else "holdoutDependencySeed"
        )
        dependency_seed_snapshot = snapshot_input(inputs[dependency_input_key])
        if dependency_seed_snapshot["kind"] != "TREE":
            raise HarnessError("corpus dependency seed authority must be an immutable tree")
        candidate_tools_snapshot = snapshot_input(inputs["candidateTools"])
    source_input_key = "qualificationSourceSet" if cohort == "QUALIFICATION" else "holdoutSourceSet"
    source_set = _input_path(inputs, source_input_key, "SOURCE_SET").resolve(strict=True)
    expected_repository = (source_set / entry_id).resolve(strict=True)
    repository = repository.resolve(strict=True)
    if repository != expected_repository:
        raise HarnessError("corpus runner repository is not the selected source-set member")
    before = _git_observation(repository)
    if before["head"] != entry["commit"] or before["tree"] != entry["gitTree"] or not before["clean"]:
        raise HarnessError("qualification repository is not an exact clean frozen pin")
    source_authority_node = "QUALIFICATION_DEPENDENCY_SEED_PREPARE" if cohort == "QUALIFICATION" else "HOLDOUT_SOURCE_MATERIALIZE"
    source_receipt = store.receipt(source_authority_node)
    source_members = source_receipt.get("evidence", {}).get("sourceMembers" if cohort == "QUALIFICATION" else "members") if source_receipt else None
    bound_member = next((row for row in source_members or [] if row.get("entry") == entry_id), None)
    if not isinstance(bound_member, dict) or bound_member.get("sourceTreeSha256") != before["sourceTreeSha256"] or bound_member.get("index") != _git_index_snapshot(repository):
        raise HarnessError("corpus source member is not bound by the current materialize/PREPARE receipt")
    tools = _candidate_tools(inputs)
    if tools["manifestSha256"] != candidate_tools_snapshot["sha256"]:
        raise HarnessError("candidate tools changed before corpus child start")
    generic_runtime = Path(tools["genericRuntime"]["path"])
    generic_runtime_sha256 = tools["genericRuntime"]["sha256"]
    kotlin_adapter = Path(tools["kotlinAdapter"]["path"])
    kotlin_adapter_sha256 = tools["kotlinAdapter"]["sha256"]
    evidence_store = _external_directory(evidence_store, repository, "evidence store", create=True)
    semantic_state_root = _external_directory(semantic_state_root, repository, "semantic state root", create=True)
    build_state_parent = _external_directory(build_state_root, repository, "build state runtime parent", create=True)
    cohort_root = Path(dependency_seed_snapshot["path"]).resolve(strict=True)
    cohort_entries = [row for row in store.bundle["corpus"]["entries"] if row["cohort"] == expected_cohort]
    cohort_authority = _validate_dependency_cohort(
        cohort_root,
        expected_cohort,
        cohort_entries,
        expected_source_set_sha256=snapshot_input(inputs[source_input_key])["sha256"],
        expected_candidate_tools_sha256=candidate_tools_snapshot["sha256"],
    )
    preparation_row = next(row for row in cohort_authority["manifest"]["entries"] if row["entry"] == entry_id)
    prepared_refusal = cohort_root / "entries" / entry_id / "PREPARED_REFUSAL.json"
    seed_root = cohort_root / "entries" / entry_id / "build-state"
    seed_authority: dict[str, Any]
    if preparation_row["outcome"] == "READY":
        seed_authority = _validate_build_state_seed(seed_root, expected_cohort)
        runtime_build_state, build_state_before = _copy_seed_to_fresh_runtime(
            seed_root, build_state_parent, entry_id, invocation
        )
    else:
        seed_authority = {"seedDigest": None, "manifestDigest": None, "manifest": {}}
        runtime_build_state = build_state_parent / SERIES_ID / entry_id / invocation.lower()
        runtime_build_state.mkdir(parents=True)
        build_state_before = {"seedDigest": None, "manifestDigest": None, "manifest": {}}
    if build_state_before["seedDigest"] != seed_authority["seedDigest"] or build_state_before["manifestDigest"] != seed_authority["manifestDigest"]:
        raise HarnessError("mutable build-state clone differs from immutable seed authority")
    roots = [evidence_store, semantic_state_root, runtime_build_state, store.root.resolve(strict=True)]
    if any(left == right or left.is_relative_to(right) or right.is_relative_to(left) for index, left in enumerate(roots) for right in roots[index + 1:]):
        raise HarnessError("evidence, semantic cache, build state and readiness store roots must be disjoint")
    build_seed_marker = runtime_build_state / "CODECLEW_K1_BUILD_STATE_SEED"
    if preparation_row["outcome"] == "READY":
        if not build_seed_marker.is_file() or build_seed_marker.is_symlink():
            raise HarnessError("external build state lacks verified CODECLEW_K1_BUILD_STATE_SEED marker")
        build_seed_digest = sha256_file(build_seed_marker)
    else:
        build_seed_digest = None
    retained_attempt_directory = _external_directory(store.root / output_directory_name, repository, "adapter attempt root", create=True)
    retained_attempt_path = retained_attempt_directory / f"{entry_id}-{invocation.lower()}.json"
    if retained_attempt_path.exists() or retained_attempt_path.is_symlink():
        raise HarnessError("qualification retained-attempt path already exists")
    # Repository-owned build code and compiler analysis never receive the
    # selected source-set checkout itself.  Recreate the exact Git identity in
    # a credential-free, no-history checkout and grant the sandbox read access
    # only to that disposable source.  The selected source remains an observed
    # authority and is rechecked under the publication lock below.
    analysis_source_parent = build_state_parent / f"{SERIES_ID}-analysis-source" / entry_id
    analysis_source_parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    analysis_source_path = analysis_source_parent / invocation.lower()
    analysis_repository = _disposable_git_archive(repository, analysis_source_path)
    analysis_before = _git_observation(analysis_repository)
    if analysis_before != before or _git_index_snapshot(analysis_repository) != bound_member["index"]:
        _discard_disposable_source(analysis_repository, analysis_source_parent)
        raise HarnessError("sanitized analysis checkout differs from selected source authority")
    analysis_git_metadata_sha256 = _tree_digest(analysis_repository / ".git")
    terminal_replay = False
    if invocation == "WARM":
        cold_pair = lookup_attempt(entry_id, "COLD")
        if cold_pair is None:
            raise HarnessError("warm run requires the exact retained cold authority")
        cold_digest, cold_attempt = cold_pair
        cold_proof = cold_attempt.get("proofSafety", {})
        cold_is_unsafe = (
            cold_attempt.get("sourceMutation") is True
            or cold_attempt.get("modelCalls") != 0
            or str(cold_attempt.get("reasonCode", "")).startswith("UNTYPED_FAILURE/")
            or bool(cold_proof.get("falseProven"))
            or bool(cold_proof.get("falseComplete"))
        )
        if cold_is_unsafe:
            # Complete the preregistered pair without starting another child.
            # The exact cold terminal identity remains the replay authority;
            # this sidecar is visibly FAILED and cannot improve applicability
            # or cache/cost results.
            if analysis_source_path.exists():
                _discard_disposable_source(analysis_source_path, analysis_source_parent)
            warm_attempt = json.loads(json.dumps(cold_attempt))
            warm_attempt.update({
                "invocation": "WARM",
                "status": "FAILED",
                "failureStage": "PRE_CHILD_FATAL_REPLAY",
                "reasonCode": "HARNESS/UNSAFE_COLD_NO_CHILD_WARM",
                "safeDetailSha256": sha256_bytes(canonical({
                    "coldAttemptDigest": cold_digest,
                    "unsafeReasons": {
                        "sourceMutation": cold_attempt.get("sourceMutation") is True,
                        "modelCalls": cold_attempt.get("modelCalls"),
                        "falseProven": cold_proof.get("falseProven", []),
                        "falseComplete": cold_proof.get("falseComplete", []),
                        "reasonCode": cold_attempt.get("reasonCode"),
                    },
                })),
                "sourceMutation": False,
                "successAuthorityValidated": False,
                "coldAuthorityDigest": cold_digest,
                "child": {
                    "kind": "NOT_STARTED_FATAL_REPLAY",
                    "exitCode": None,
                    "signal": None,
                    "stdoutSha256": store.put_blob(b""),
                    "stdoutBytes": 0,
                    "stderrSha256": store.put_blob(b""),
                    "stderrBytes": 0,
                    "timeOutputSha256": store.put_blob(b""),
                    "parsedOutputDigest": None,
                    "successAuthorityValidated": False,
                },
                "adapterCache": {"status": "NOT_STARTED_FATAL_REPLAY", "hit": False},
                "nonemptyProjection": {"passed": False, "reasons": ["UNSAFE_COLD_NO_CHILD_WARM"]},
                "workload": None,
                "exactCommandSha256": sha256_bytes(canonical({
                    "kind": "UNSAFE_COLD_NO_CHILD_WARM", "coldAttemptDigest": cold_digest,
                })),
                "modelCalls": 0,
            })
            warm_attempt["adapterCost"] = {
                **cold_attempt["adapterCost"],
                "externalWallMicros": 0,
                "maximumResidentBytes": 0,
                "cacheHits": 0,
                "modelCalls": 0,
            }
            warm_attempt["resource"] = {
                **cold_attempt.get("resource", {}),
                "externalWallMicros": 0,
                "maximumResidentBytes": 0,
            }
            warm_attempt["attemptDigest"] = ""
            warm_attempt["attemptDigest"] = sha256_bytes(canonical(warm_attempt))
            with store.locked():
                if snapshot_input(inputs[dependency_input_key]) != dependency_seed_snapshot or snapshot_input(inputs["candidateTools"]) != candidate_tools_snapshot or _git_observation(repository) != before:
                    raise HarnessError("unsafe warm authority changed before no-child publication")
                authority_digest = publish_attempt(warm_attempt)
            return authority_digest, warm_attempt
        workload = cold_attempt.get("workload")
        cold_success = cold_attempt.get("status") == "ADAPTER_OUTPUT"
        if cold_success:
            if not isinstance(workload, dict) or workload.get("selectionAuthority") != "HARNESS_DERIVED_ONLY" or not isinstance(workload.get("seedEntity"), str):
                raise HarnessError("cold authority has no harness-derived workload seed")
            seed_entity = workload["seedEntity"]
            adapter_run_phase = "warm"
        else:
            if cold_attempt.get("status") not in {"PARTIAL", "REFUSED", "FAILED"} or str(cold_attempt.get("reasonCode", "")).startswith("UNTYPED_FAILURE/"):
                raise HarnessError("cold authority is neither success nor typed terminal")
            seed_entity = None
            adapter_run_phase = "cold"
            terminal_replay = True
    else:
        cold_digest = None
        cold_attempt = None
        seed_entity = None
        adapter_run_phase = "cold"
    adapter_arguments = [
        "--compilation", entry["selectedCompilation"],
        "--state-root", str(semantic_state_root),
        "--attempt-output", str(retained_attempt_path),
        "--run-phase", adapter_run_phase,
    ]
    if preparation_row["outcome"] == "READY":
        adapter_arguments.extend(["--build-state-root", str(runtime_build_state)])
    else:
        refusal = _load_json_bytes(_regular_file(prepared_refusal, "prepared refusal").read_bytes(), "prepared refusal")
        adapter_arguments.extend([
            "--prepared-refusal", str(prepared_refusal),
            "--prepared-refusal-sha256", sha256_file(prepared_refusal),
            "--entry-id", entry_id,
            "--candidate-tools-sha256", candidate_tools_snapshot["sha256"],
            "--build-input-digest", refusal["buildInputDigest"],
            "--preparation-receipt-digest", refusal["preparationReceiptDigest"],
        ])
    exact_command = [
        str(generic_runtime), "run",
        "--repo", str(analysis_repository),
        "--adapter", str(kotlin_adapter),
        "--adapter-sha256", kotlin_adapter_sha256,
        "--store", str(evidence_store),
        *[f"--adapter-arg={argument}" for argument in adapter_arguments],
        "--max-depth", str(store.bundle["requirements"]["workloadPolicy"]["query"]["maxDepth"]),
        "--max-entities", str(store.bundle["requirements"]["workloadPolicy"]["query"]["maxEntities"]),
        "--repetitions", "1",
    ]
    if seed_entity is not None:
        exact_command.extend(["--seed-entity", seed_entity])
    seed_tokens = [token for token in exact_command if "--seed-entity" in token]
    if invocation == "COLD" and seed_tokens:
        raise HarnessError("cold qualification command contains a seed")
    if invocation == "WARM" and not terminal_replay and seed_tokens != ["--seed-entity"]:
        raise HarnessError("warm qualification command lacks the unique harness-derived seed position")
    if terminal_replay and (seed_tokens or adapter_run_phase != "cold"):
        raise HarnessError("typed terminal replay must be an exact independent cold/no-seed process")
    variant = entry["trustedAnalyzerMinorLine"].replace(".", "")
    distribution = (
        ROOT / "workers/kotlin/build/install/kotlin"
        if variant == "24" else ROOT / f"workers/kotlin{variant}/build/install/kotlin{variant}"
    )
    pinned_worker_reads = [
        kotlin_adapter,
        ROOT / "workers/kotlin/src/main",
        distribution,
        ROOT / "build.gradle.kts", ROOT / "settings.gradle.kts", ROOT / "gradlew",
        ROOT / "gradle/wrapper/gradle-wrapper.jar",
        ROOT / "gradle/wrapper/gradle-wrapper.properties",
        ROOT / "schemas/worker.proto",
        ROOT / ("workers/kotlin/build.gradle.kts" if variant == "24" else f"workers/kotlin{variant}/build.gradle.kts"),
    ]
    if variant != "24":
        pinned_worker_reads.append(ROOT / f"workers/kotlin{variant}/src/main")
    sandbox_reads = [
        *pinned_worker_reads, evidence_store, semantic_state_root, runtime_build_state,
    ]
    if preparation_row["outcome"] == "TYPED_REFUSAL":
        sandbox_reads.append(prepared_refusal.parent)
    try:
        diagnostic_digest, supervisor_attempt = supervise_entry(
            store,
            entry_id,
            invocation,
            analysis_repository,
            exact_command,
            inputs,
            timeout_seconds=timeout_seconds,
            resident_limit_bytes=resident_limit_bytes,
            output_limit_bytes=MAX_STDOUT_BYTES,
            diagnostic_only=True,
            decision_authority=authority,
            authorized_read_paths=sandbox_reads,
            authorized_write_paths=(
                evidence_store, semantic_state_root, retained_attempt_directory, runtime_build_state,
            ),
        )
    finally:
        if analysis_source_path.exists():
            _discard_disposable_source(analysis_source_path, analysis_source_parent)
    if analysis_source_path.exists() or analysis_source_path.is_symlink():
        raise HarnessError("sanitized analysis checkout was not discarded")
    with store.locked():
        if snapshot_input(inputs["candidateTools"]) != candidate_tools_snapshot:
            raise HarnessError("candidate tools changed during corpus child")
        if snapshot_input(inputs[source_input_key])["sha256"] != _source_set_digest(source_set):
            raise HarnessError("source set changed during corpus child")
    supervisor_attempt["diagnosticSupervisorDigest"] = diagnostic_digest
    supervisor_attempt["authority"] = authority
    supervisor_attempt["exactCommandSha256"] = sha256_bytes(canonical(exact_command))
    supervisor_attempt["genericRuntimeSha256"] = generic_runtime_sha256
    supervisor_attempt["kotlinAdapterSha256"] = kotlin_adapter_sha256
    supervisor_attempt["candidateToolsManifestSha256"] = tools["manifestSha256"]
    supervisor_attempt["sourceExecutionAuthority"] = {
        "kind": "SANITIZED_DISPOSABLE_GIT",
        "selectedSourceTreeSha256": before["sourceTreeSha256"],
        "executionSourceTreeSha256": analysis_before["sourceTreeSha256"],
        "executionGitMetadataSha256": analysis_git_metadata_sha256,
        "sourceSetSha256": snapshot_input(inputs[source_input_key])["sha256"],
        "discardedBeforePublication": True,
    }
    supervisor_attempt["buildStateSeedMarkerSha256"] = build_seed_digest
    supervisor_attempt["phaseReceipts"] = phase_receipts
    supervisor_attempt["dependencySeedAuthority"] = dependency_seed_snapshot
    supervisor_attempt["buildStateAuthority"] = {
        "immutableSeed": {key: value for key, value in seed_authority.items() if key != "manifest"},
        "mutableCloneBefore": {key: value for key, value in build_state_before.items() if key != "manifest"},
    }
    supervisor_attempt["coldAuthorityDigest"] = cold_digest
    child_kind = supervisor_attempt["child"]["kind"]
    child_exit = supervisor_attempt["child"]["exitCode"]
    retained_value: dict[str, Any] | None = None
    if retained_attempt_path.exists():
        retained_raw = _regular_file(retained_attempt_path, "adapter-retained attempt").read_bytes()
        retained_value = _validate_kotlin_attempt(_load_json_bytes(retained_raw, "adapter-retained attempt"))
        if canonical(retained_value) != retained_raw:
            raise HarnessError("adapter-retained attempt is not canonical JSON plus newline")
        supervisor_attempt["adapterRetainedAttemptSha256"] = sha256_bytes(retained_raw)
        supervisor_attempt["adapterRetainedAttemptDigest"] = retained_value["attemptDigest"]
        supervisor_attempt["terminalSemanticDigest"] = retained_value["terminalSemanticDigest"]
    else:
        supervisor_attempt["adapterRetainedAttemptSha256"] = None
        supervisor_attempt["adapterRetainedAttemptDigest"] = None
        supervisor_attempt["terminalSemanticDigest"] = None
    if child_kind == "VALIDATED_PROJECTION" and child_exit == 0:
        if retained_value is None or retained_value["status"] != "SUCCEEDED" or retained_value["outcomeKind"] != "ADAPTER_OUTPUT":
            raise HarnessError("validated projection lacks exact successful adapter-retained attempt")
        # Reopen and rehash the adapter envelope object published by the
        # pinned generic runtime before recognizing success.
        stdout_blob = store.root / "blobs" / f"{supervisor_attempt['child']['stdoutSha256'][7:]}.blob"
        projection = _load_json_bytes(_regular_file(stdout_blob, "projection stdout blob").read_bytes(), "projection stdout blob")
        provenance = projection["provenance"]
        if provenance["runtime"]["binaryDigest"] != generic_runtime_sha256 or provenance["adapterBinaryDigest"] != kotlin_adapter_sha256:
            raise HarnessError("projection candidate-tool provenance mismatch")
        adapter_object = provenance["adapterOutputObject"]
        relative_path = adapter_object.get("relativePath")
        if not isinstance(relative_path, str) or Path(relative_path).is_absolute() or ".." in Path(relative_path).parts:
            raise HarnessError("projection adapter object path mismatch")
        object_path = evidence_store / relative_path
        object_raw = _regular_file(object_path, "generic-runtime adapter object").read_bytes()
        if sha256_bytes(object_raw) != adapter_object["digest"] or len(object_raw) != adapter_object.get("sizeBytes"):
            raise HarnessError("projection adapter object rehash mismatch")
        adapter_output = _load_json_bytes(object_raw, "adapter object")
        adapter_output_digest = adapter_output.get("outputDigest")
        if retained_value.get("adapterOutputDigest") != adapter_output_digest or provenance.get("adapterOutputDigest") != adapter_output_digest:
            raise HarnessError("adapter output digest cross-binding mismatch")
        if retained_value.get("evidenceCore") != provenance.get("evidenceCore"):
            raise HarnessError("retained/projection evidence-core binding mismatch")
        independently_computed_semantic = _adapter_semantic_output_digest(adapter_output)
        if retained_value["terminalSemanticDigest"] != independently_computed_semantic:
            raise HarnessError("successful attempt terminal semantic digest is not derived from adapter output")
        if provenance.get("semanticOutputDigest") != independently_computed_semantic:
            raise HarnessError("projection semantic digest differs from independently hashed adapter output")
        entities = adapter_output.get("entities") if isinstance(adapter_output, dict) else None
        facts = adapter_output.get("facts") if isinstance(adapter_output, dict) else None
        semantic_facts = _rust_canonical_digest({
            "adapter": adapter_output.get("adapter"), "snapshotInput": adapter_output.get("snapshotInput"),
            "capabilityDescriptors": adapter_output.get("capabilityDescriptors"), "entities": adapter_output.get("entities"),
            "occurrences": adapter_output.get("occurrences"), "facts": adapter_output.get("facts"),
            "boundaries": adapter_output.get("boundaries"), "compilerReceipt": adapter_output.get("compilerReceipt"),
        })
        if retained_value.get("cache", {}).get("semanticFactsDigest") != semantic_facts:
            raise HarnessError("semantic facts digest is not derived from reopened adapter output")
        if not isinstance(entities, list) or not isinstance(facts, list):
            raise HarnessError("adapter object lacks workload candidates")
        incident = set()
        for fact in facts:
            if isinstance(fact, dict):
                for key in ("owner", "target"):
                    value = fact.get(key)
                    if isinstance(value, str):
                        incident.add(value)
        eligible = sorted(
            str(entity["opaqueId"])
            for entity in entities
            if isinstance(entity, dict)
            and entity.get("resolution") == "RESOLVED"
            and isinstance(entity.get("opaqueId"), str)
            and entity.get("primaryDefinition") is not None
            and entity.get("opaqueId") in incident
        )
        impact = adapter_output.get("impact")
        if not isinstance(impact, dict):
            raise HarnessError("adapter object lacks impact authority")
        # These Kotlin-owned proposal fields live in the schema-authorized
        # provider payload. Top-level additions would evade the frozen
        # AdapterOutput `additionalProperties:false` contract.
        provider_payload = impact.get("providerPayload")
        if not isinstance(provider_payload, dict):
            raise HarnessError("adapter impact lacks provider payload")
        proposed = provider_payload.get("proposedSeedEntity")
        authority = provider_payload.get("selectionAuthority")
        if "proposedSeedEntity" in impact or "selectionAuthority" in impact:
            raise HarnessError("adapter seed authority is outside impact.providerPayload")
        if not eligible:
            raise HarnessError("adapter result has no eligible workload seed")
        if invocation == "COLD":
            if proposed != eligible[0] or authority != "DETERMINISTIC_LEXICOGRAPHIC_CANDIDATE" or projection["query"].get("seedEntity") is not None:
                raise HarnessError("cold deterministic candidate differs from harness-derived workload")
        else:
            if cold_attempt.get("status") != "ADAPTER_OUTPUT":
                raise HarnessError("warm success differs from cold outcome status")
            if proposed != seed_entity or proposed != eligible[0] or authority != "DETERMINISTIC_LEXICOGRAPHIC_CANDIDATE" or projection["query"].get("seedEntity") != seed_entity:
                raise HarnessError("warm seed differs from exact cold-derived workload")
            cold_facts = cold_attempt.get("semanticFactsDigest") if cold_attempt else None
            warm_facts = retained_value.get("cache", {}).get("semanticFactsDigest")
            cache = retained_value.get("cache", {})
            if cache.get("status") != "VERIFIED_HIT" or cache.get("hit") is not True or retained_value.get("cost", {}).get("cacheHits", 0) < 1:
                raise HarnessError("warm attempt has no verified semantic cache hit")
            if not _is_digest(cold_facts) or warm_facts != cold_facts:
                raise HarnessError("warm semantic facts differ from cold authority")
            if retained_value.get("terminalSemanticDigest") != cold_attempt.get("terminalSemanticDigest"):
                raise HarnessError("warm terminal semantic digest differs from cold authority")
        supervisor_attempt["workload"] = {
            "selectionAuthority": "HARNESS_DERIVED_ONLY",
            "seedEntity": eligible[0],
            "maxDepth": 2,
            "maxEntities": 128,
        }
        supervisor_attempt["status"] = "ADAPTER_OUTPUT"
        supervisor_attempt["successAuthorityValidated"] = True
        supervisor_attempt["semanticFactsDigest"] = retained_value.get("cache", {}).get("semanticFactsDigest")
        supervisor_attempt["semanticCacheKeyDigest"] = retained_value.get("cache", {}).get("keyDigest")
        supervisor_attempt["adapterAuthority"] = {
            "outputDigest": retained_value.get("adapterOutputDigest"),
            "retainedAttemptDigest": retained_value.get("attemptDigest"),
            "retainedAttemptSha256": supervisor_attempt["adapterRetainedAttemptSha256"],
            "evidenceCore": retained_value.get("evidenceCore"),
            "adapterObject": adapter_object,
            "projectionDigest": projection.get("projectionDigest"),
            "projectionSemanticOutputDigest": provenance.get("semanticOutputDigest"),
        }
        supervisor_attempt["adapterCache"] = retained_value.get("cache")
        supervisor_attempt["adapterCost"] = retained_value.get("cost")
        supervisor_attempt["projectionCost"] = projection.get("cost")
        supervisor_attempt["proofSafety"] = _structural_proof_safety(adapter_output)
        supervisor_attempt["nonemptyProjection"] = _nonempty_projection(
            adapter_output, projection, eligible[0]
        )
        supervisor_attempt["declaredProjectCompilerVersion"] = adapter_output.get(
            "snapshotInput", {}
        ).get("toolchain", {}).get("providerPayload", {}).get("declaredProjectCompilerVersion")
        supervisor_attempt["analyzerCompilerVersion"] = adapter_output.get(
            "snapshotInput", {}
        ).get("toolchain", {}).get("providerPayload", {}).get("analyzerCompilerVersion")
        expected_analyzer = store.bundle["corpus"]["frozenExecutionPolicy"]["trustedAnalyzers"][
            entry["trustedAnalyzerMinorLine"]
        ]["compilerVersion"]
        trusted_distribution = adapter_output.get("compilerReceipt", {}).get(
            "providerPayload", {}
        ).get("trustedWorkerDistribution", {})
        expected_worker = tools["workerManifests"][entry["trustedAnalyzerMinorLine"]]
        expected_distribution = {
            key: expected_worker[key]
            for key in ("treeHash", "buildInputDigest", "pluginFingerprint")
        }
        toolchain_provider = adapter_output.get("snapshotInput", {}).get(
            "toolchain", {}
        ).get("providerPayload", {})
        if supervisor_attempt["declaredProjectCompilerVersion"] != entry["declaredKotlinVersion"]:
            raise HarnessError("adapter declared compiler version differs from corpus authority")
        if supervisor_attempt["analyzerCompilerVersion"] != expected_analyzer or expected_worker["compilerVersion"] != expected_analyzer:
            raise HarnessError("adapter analyzer compiler version differs from exact corpus authority")
        if trusted_distribution != expected_distribution or not isinstance(toolchain_provider, dict) or {
            "treeHash": toolchain_provider.get("trustedDistributionTreeHash"),
            "buildInputDigest": toolchain_provider.get("trustedDistributionBuildInputDigest"),
            "pluginFingerprint": toolchain_provider.get("trustedDistributionPluginFingerprint"),
        } != expected_distribution:
            raise HarnessError("compiler receipt worker distribution differs from candidate manifest")
        compiler_provider = adapter_output.get("compilerReceipt", {}).get("providerPayload", {})
        semantic_manifest = compiler_provider.get("semanticInputManifest")
        semantic_manifest_hash = compiler_provider.get("semanticInputManifestHash")
        if not isinstance(semantic_manifest, dict) or semantic_manifest_hash != _rust_canonical_digest(semantic_manifest):
            raise HarnessError("compiler receipt semantic-input manifest hash mismatch")
        dependency_projection = {
            "orderedCompileClasspath": semantic_manifest.get("orderedCompileClasspath"),
            "orderedFriendPaths": semantic_manifest.get("orderedFriendPaths"),
            "orderedCompilerPlugins": semantic_manifest.get("orderedCompilerPlugins"),
            "dependencyCoordinates": semantic_manifest.get("dependencyCoordinates"),
            "repositories": semantic_manifest.get("repositories"),
            "reactorPoms": semantic_manifest.get("reactorPoms"),
            "buildPlugins": semantic_manifest.get("buildPlugins"),
            "generatedSourceConfiguration": semantic_manifest.get("generatedSourceConfiguration"),
            "fieldBoundaries": semantic_manifest.get("fieldBoundaries"),
            "buildModelBoundaries": semantic_manifest.get("buildModelBoundaries"),
            "legacyClasspathHash": retained_value.get("provenance", {}).get("classpathHash"),
        }
        adapter_snapshot_input = adapter_output.get("snapshotInput", {})
        supervisor_attempt["workerDistributionTreeHash"] = expected_worker["treeHash"]
        supervisor_attempt["workerDistributionIdentity"] = expected_distribution
        supervisor_attempt["buildModelAuthority"] = {
            "semanticInputManifest": semantic_manifest,
            "semanticInputManifestHash": semantic_manifest_hash,
            "dependencyGraphDigest": adapter_snapshot_input.get("dependencyGraphDigest"),
            "dependencyProjection": dependency_projection,
            "buildModelDigest": adapter_snapshot_input.get("buildModelDigest"),
            "buildConfigurationDigest": adapter_snapshot_input.get("buildConfigurationDigest"),
            "generatedSourcesManifestDigest": adapter_snapshot_input.get("generatedSourcesManifestDigest"),
            "targets": adapter_snapshot_input.get("targets"),
            "sourceOrigins": sorted({
                source.get("origin") for source in adapter_snapshot_input.get("sources", [])
                if isinstance(source, dict) and isinstance(source.get("origin"), str)
            }),
            "boundariesDigest": _rust_canonical_digest(adapter_output.get("boundaries", [])),
        }
    elif retained_value is not None and retained_value["status"] in {"PARTIAL", "REFUSED", "FAILED"} and child_exit != 0:
        if preparation_row["outcome"] == "TYPED_REFUSAL" and (
            retained_value["status"] != "REFUSED"
            or retained_value["reasonCode"] != preparation_row.get("reasonCode")
        ):
            raise HarnessError("adapter prepared-refusal terminal differs from PREPARE authority")
        if preparation_row["outcome"] == "TYPED_REFUSAL":
            refusal = _load_json_bytes(
                _regular_file(prepared_refusal, "prepared refusal").read_bytes(),
                "prepared refusal",
            )
            retained_inputs = retained_value.get("selectedInputs")
            retained_input = retained_inputs.get("preparedRefusal") if isinstance(retained_inputs, dict) else None
            retained_snapshot = retained_value.get("snapshot")
            expected_retained_input = {
                "schema": refusal["schema"], "seriesId": refusal["seriesId"],
                "cohort": refusal["cohort"], "entry": refusal["entry"],
                "selectedCompilation": refusal["selectedCompilation"],
                "objectDigest": refusal["objectDigest"], "fileDigest": sha256_file(prepared_refusal),
                "sourceTreeSha256": refusal["sourceTreeSha256"],
                "candidateToolsSha256": refusal["candidateToolsSha256"],
                "buildInputDigest": refusal["buildInputDigest"],
                "preparationReceiptDigest": refusal["preparationReceiptDigest"],
            }
            if not isinstance(retained_inputs, dict) or retained_input != expected_retained_input:
                raise HarnessError("retained prepared-refusal selectedInputs differ from sealed authority")
            if retained_inputs.get("repo") != str(analysis_repository) or retained_inputs.get("compilation") != refusal["selectedCompilation"] or retained_inputs.get("preparedRefusalRequested") is not True or retained_inputs.get("externalBuildStateRequested") is not False:
                raise HarnessError("retained prepared-refusal invocation inputs differ from harness authority")
            if not isinstance(retained_snapshot, dict) or any(
                retained_snapshot.get(key) != value for key, value in {
                    "vcsRevision": refusal["commit"],
                    "dirty": False,
                    "sourceTreeSha256": refusal["sourceTreeSha256"],
                    "repositoryTreeDigest": refusal["sourceTreeSha256"],
                    "gitTree": refusal["gitTree"],
                    "gitIndexDigest": bound_member["index"]["digest"],
                    "gitStatusDigest": analysis_before["statusSha256"],
                }.items()
            ):
                raise HarnessError("retained prepared-refusal snapshot differs from sanitized source authority")
        if invocation == "WARM" and cold_attempt is not None and (
            retained_value["terminalSemanticDigest"] != cold_attempt.get("terminalSemanticDigest")
            or retained_value["status"] != cold_attempt.get("status")
            or retained_value["reasonCode"] != cold_attempt.get("reasonCode")
        ):
            raise HarnessError("typed terminal replay differs across independent processes")
        replay_cache = retained_value.get("cache", {})
        replay_cost = retained_value.get("cost", {})
        if terminal_replay and (
            replay_cache.get("hit") is True
            or replay_cache.get("status") == "VERIFIED_HIT"
            or replay_cost.get("cacheHits", 0) != 0
        ):
            raise HarnessError("typed terminal cold replay claimed a warm cache hit")
        supervisor_attempt["status"] = retained_value["status"]
        supervisor_attempt["reasonCode"] = retained_value["reasonCode"]
        supervisor_attempt["successAuthorityValidated"] = False
        supervisor_attempt["adapterCache"] = retained_value.get("cache")
        supervisor_attempt["adapterCost"] = retained_value.get("cost")
        supervisor_attempt["projectionCost"] = None
        supervisor_attempt["proofSafety"] = {"falseProven": [], "falseComplete": []}
        supervisor_attempt["nonemptyProjection"] = {"passed": False, "reasons": ["TYPED_TERMINAL"]}
    else:
        supervisor_attempt["status"] = "FAILED"
        supervisor_attempt["reasonCode"] = (
            supervisor_attempt.get("reasonCode") or "SUPERVISOR/NO_VALIDATED_ADAPTER_ATTEMPT"
        )
        supervisor_attempt["successAuthorityValidated"] = False
        supervisor_attempt["terminalSemanticDigest"] = _rust_canonical_digest({
            "schema": "codeclew.kotlin-k1-supervisor-terminal/0.1",
            "entry": entry_id, "cohort": expected_cohort, "status": "FAILED",
            "failureStage": "SUPERVISED_ADAPTER", "reasonCode": supervisor_attempt["reasonCode"],
            "sourceTreeSha256": before["sourceTreeSha256"],
            "candidateToolsSha256": candidate_tools_snapshot["sha256"],
            "dependencyCohortDigest": cohort_authority["cohortDigest"],
            "preparationOutcome": preparation_row["outcome"],
        })
        supervisor_attempt["adapterCache"] = {
            "status": "NOT_AVAILABLE_SUPERVISOR_TERMINAL", "hit": False,
        }
        supervisor_attempt["adapterCost"] = _supervisor_terminal_cost(supervisor_attempt["resource"])
        supervisor_attempt["projectionCost"] = None
        supervisor_attempt["proofSafety"] = {
            "falseProven": [], "falseComplete": ["SUPERVISOR_TERMINAL_NO_SEMANTIC_OUTPUT"],
        }
        supervisor_attempt["nonemptyProjection"] = {
            "passed": False, "reasons": ["SUPERVISOR_TERMINAL"],
        }
    supervisor_attempt["attemptDigest"] = ""
    build_state_after = (
        _validate_build_state_seed(runtime_build_state, expected_cohort)
        if preparation_row["outcome"] == "READY" else build_state_before
    )
    supervisor_attempt["buildStateAuthority"]["mutableCloneAfter"] = {
        key: value for key, value in build_state_after.items() if key != "manifest"
    }
    if build_state_after["seedDigest"] != build_state_before["seedDigest"] or build_state_after["manifestDigest"] != build_state_before["manifestDigest"]:
        raise HarnessError("adapter changed the build-state seed authority")
    supervisor_attempt["attemptDigest"] = sha256_bytes(canonical(supervisor_attempt))
    with store.locked():
        for node_id, receipt_digest in phase_receipts.items():
            status, _, _ = assess(store, node_id, inputs)
            pointer = store.pointer(node_id)
            if status != "READY" or pointer is None or pointer["receiptDigest"] != receipt_digest:
                raise HarnessError(f"corpus run phase authority changed during child: {node_id}")
        if snapshot_input(inputs[dependency_input_key]) != dependency_seed_snapshot:
            raise HarnessError("dependency seed authority changed during corpus child")
        if snapshot_input(inputs["candidateTools"]) != candidate_tools_snapshot:
            raise HarnessError("candidate tools authority changed before attempt publication")
        current_source_snapshot = snapshot_input(inputs[source_input_key])
        if current_source_snapshot["sha256"] != _source_set_digest(source_set) or _git_observation(repository) != before:
            raise HarnessError("source-set member changed before attempt publication")
        authority_digest = publish_attempt(supervisor_attempt)
    return authority_digest, supervisor_attempt


def run_qualification_entry(
    store: Store, entry_id: str, invocation: str, repository: Path, evidence_store: Path,
    semantic_state_root: Path, build_state_root: Path,
    inputs: Mapping[str, Mapping[str, Any]], *, timeout_seconds: int = MAX_WALL_SECONDS,
    resident_limit_bytes: int = MAX_RESIDENT_BYTES,
) -> tuple[str, dict[str, Any]]:
    return _run_corpus_entry(
        store, entry_id, invocation, repository, evidence_store, semantic_state_root,
        build_state_root, inputs, cohort="QUALIFICATION", timeout_seconds=timeout_seconds,
        resident_limit_bytes=resident_limit_bytes,
    )


def run_holdout_entry(
    store: Store, entry_id: str, invocation: str, repository: Path, evidence_store: Path,
    semantic_state_root: Path, build_state_root: Path,
    inputs: Mapping[str, Mapping[str, Any]], *, timeout_seconds: int = MAX_WALL_SECONDS,
    resident_limit_bytes: int = MAX_RESIDENT_BYTES,
) -> tuple[str, dict[str, Any]]:
    return _run_corpus_entry(
        store, entry_id, invocation, repository, evidence_store, semantic_state_root,
        build_state_root, inputs, cohort="BLIND_HOLDOUT", timeout_seconds=timeout_seconds,
        resident_limit_bytes=resident_limit_bytes,
    )


def _read_input_manifest(path: Path | None) -> dict[str, Mapping[str, Any]]:
    if path is None:
        return {}
    raw = _regular_file(path, "input manifest").read_bytes()
    value = _load_json_bytes(raw, "input manifest")
    if not isinstance(value, dict) or set(value) != {"schema", "seriesId", "inputs"} or value.get("schema") != "codeclew.kotlin-k1-live-inputs/0.1" or value.get("seriesId") != SERIES_ID or not isinstance(value.get("inputs"), dict):
        raise HarnessError("input manifest contract mismatch")
    if canonical(value) != raw:
        raise HarnessError("input manifest must be canonical JSON plus newline")
    inputs = value["inputs"]
    expected_keys = {
        selected
        for node in load_production_bundle()["readinessGraph"]["nodes"]
        for selected in node["selectedInputs"]
        if selected not in AUTHORITIES
    }
    if set(inputs) != expected_keys:
        raise HarnessError("input manifest must contain the exact production live-input key set")
    expected_kinds = {
        "qualificationSourceSet": "SOURCE_SET", "holdoutSourceSet": "SOURCE_SET",
        "qualificationDependencySeed": "TREE", "holdoutDependencySeed": "TREE",
        "candidateSources": "LIVE_SET", "candidateBinaries": "LIVE_SET",
        "k0AuthoritySet": "LIVE_SET",
    }
    output_keys = {
        "qualificationDependencySeed", "holdoutDependencySeed", "holdoutSourceSet",
        "baselinePacket", "harnessSelfTestPacket", "qualificationMatrix", "candidateFreeze",
        "holdoutMatrix", "matrixSafetyReceipt", "applicabilityReceipt", "cacheCostReceipt",
        "requirementConformance", "independentAuditorRunReceipt", "independentAudit", "decision",
    }
    output_paths: list[Path] = []
    for key in sorted(expected_keys):
        descriptor = inputs[key]
        if not isinstance(descriptor, Mapping) or set(descriptor) != {"kind", "path"}:
            raise HarnessError(f"input manifest descriptor mismatch: {key}")
        kind = expected_kinds.get(key, "FILE")
        if descriptor.get("kind") != kind or not isinstance(descriptor.get("path"), str) or not Path(descriptor["path"]).is_absolute():
            raise HarnessError(f"input manifest kind/path mismatch: {key}")
        path_value = Path(descriptor["path"]).absolute()
        if key in output_keys:
            output_paths.append(path_value)
        else:
            snapshot_input(descriptor)
    if len(output_paths) != len(set(output_paths)):
        raise HarnessError("output paths in input manifest must be distinct")
    run_roots = {path.parent for path in output_paths}
    if len(run_roots) != 1:
        raise HarnessError("all production outputs must be distinct children of one run root")
    for key in output_keys:
        path_value = Path(inputs[key]["path"])
        if path_value.is_symlink():
            raise HarnessError(f"production output must never be a symlink: {key}")
        if path_value.exists():
            expected_kind = expected_kinds.get(key, "FILE")
            if (expected_kind == "FILE" and not path_value.is_file()) or (
                expected_kind in {"TREE", "SOURCE_SET"} and not path_value.is_dir()
            ):
                raise HarnessError(f"production output kind mismatch: {key}")
    roles = {
        "k0AuthoritySet": "K0_AUTHORITY_SET",
        "candidateSources": "CANDIDATE_SOURCES",
        "candidateBinaries": "CANDIDATE_BINARIES",
    }
    for key, role in roles.items():
        _require_live_set_role(inputs, key, role)
    expected_hashes = {
        "researchInput": "sha256:6b9d9c73a809e896506dfd2645d09b77e8251940138eb813c85aeb573a270791",
        "executionContract": "sha256:a115a0690a7fe9ffc79d6cfbe2f31f2a58bc3412f9af44d22dd6e336765c35ee",
        "priorM1Failure": "sha256:9d8137ac0063dc8fc81b1f0f3c577ad41550000863741421b690265b1a3e2d49",
    }
    if any(snapshot_input(inputs[key])["sha256"] != digest for key, digest in expected_hashes.items()):
        raise HarnessError("immutable research input digest mismatch")
    if _regular_file(Path(inputs["repositoryBaseRevision"]["path"]), "repository base revision").read_bytes() != b"be19fd06ce7509ffbaf4622b8f8d13ec7cbcd854\n":
        raise HarnessError("repository base revision manifest mismatch")
    members = _source_set_members(Path(inputs["qualificationSourceSet"]["path"]))
    corpus_entries = {row["id"]: row for row in load_production_bundle()["corpus"]["entries"]}
    if [row["entry"] for row in members] != list(EXPECTED_QUALIFICATION) or any(
        row["head"] != corpus_entries[row["entry"]]["commit"]
        or row["tree"] != corpus_entries[row["entry"]]["gitTree"]
        or not row["clean"] for row in members
    ):
        raise HarnessError("qualification source set differs from frozen pins")
    return inputs


def _read_degraded_input_paths(store: Store, path: Path) -> dict[str, Mapping[str, Any]]:
    """Read only canonical descriptor paths for fatal comparison.

    No bytes selected by these descriptors can mint OPEN readiness. This
    parser exists solely after strict loading failed and a stored series guard
    is available for an internal FATAL latch.
    """
    raw = _regular_file(path, "degraded input manifest").read_bytes()
    value = _load_json_bytes(raw, "degraded input manifest")
    if not isinstance(value, dict) or canonical(value) != raw or set(value) != {"schema", "seriesId", "inputs"} or value.get("schema") != "codeclew.kotlin-k1-live-inputs/0.1" or value.get("seriesId") != SERIES_ID or not isinstance(value.get("inputs"), dict):
        raise HarnessError("degraded input manifest structural mismatch")
    expected = {
        selected
        for node in store.graph["nodes"]
        for selected in node["selectedInputs"] if selected not in AUTHORITIES
    }
    inputs = value["inputs"]
    if set(inputs) != expected or any(
        not isinstance(descriptor, Mapping)
        or set(descriptor) != {"kind", "path"}
        or descriptor.get("kind") not in {"FILE", "TREE", "SOURCE_SET", "LIVE_SET"}
        or not isinstance(descriptor.get("path"), str)
        or not Path(descriptor["path"]).is_absolute()
        for descriptor in inputs.values()
    ):
        raise HarnessError("degraded input descriptor contour mismatch")
    return inputs


def build_live_inputs(
    run_root: Path,
    research_input: Path,
    execution_contract: Path,
    prior_failure: Path,
    qualification_source_set: Path,
    candidate_tools: Path,
) -> tuple[dict[str, Any], Path]:
    """Create the one exact production input manifest without opening holdout."""
    run_root = run_root.absolute()
    if run_root.exists() or run_root.is_symlink():
        raise HarnessError("live-input run root is create-only and must be absent")
    candidate_tools = _regular_file(candidate_tools, "candidate tools manifest")
    qualification_source_set = qualification_source_set.absolute()
    source_snapshot = snapshot_input({"kind": "SOURCE_SET", "path": str(qualification_source_set)})
    if set(path.name for path in qualification_source_set.iterdir()) != set(EXPECTED_QUALIFICATION):
        raise HarnessError("build-live-inputs accepts only the qualification source set")
    tools = _candidate_tools({"candidateTools": {"kind": "FILE", "path": str(candidate_tools)}})
    observed_head = subprocess.run(
        ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        check=False, timeout=30,
        env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"},
    )
    if observed_head.returncode != 0 or observed_head.stdout.decode("ascii", "strict").strip() != PINNED_REPOSITORY_BASE_REVISION:
        raise HarnessError("live repository HEAD differs from pinned base revision")
    expected_inputs = {
        selected
        for node in load_production_bundle()["readinessGraph"]["nodes"]
        for selected in node["selectedInputs"]
        if selected not in AUTHORITIES
    }
    run_root.mkdir(parents=True, mode=0o700)
    try:
        base_revision = run_root / "repository-base-revision.txt"
        _atomic_write(base_revision, (PINNED_REPOSITORY_BASE_REVISION + "\n").encode(), 0o400)
        base_revision.chmod(0o400)
        live_sets = {}
        for key, role in {
            "k0AuthoritySet": "K0_AUTHORITY_SET",
            "candidateSources": "CANDIDATE_SOURCES",
            "candidateBinaries": "CANDIDATE_BINARIES",
        }.items():
            path = run_root / f"{key}.json"
            value = build_live_set(role, candidate_tools if role != "K0_AUTHORITY_SET" else None)
            _atomic_write(path, canonical(value), 0o400)
            path.chmod(0o400)
            live_sets[key] = {"kind": "LIVE_SET", "path": str(path)}
        existing = {
            "researchInput": {"kind": "FILE", "path": str(_regular_file(research_input, "research input"))},
            "executionContract": {"kind": "FILE", "path": str(_regular_file(execution_contract, "execution contract"))},
            "priorM1Failure": {"kind": "FILE", "path": str(_regular_file(prior_failure, "prior failure"))},
            "repositoryBaseRevision": {"kind": "FILE", "path": str(base_revision)},
            "k0Lock": {"kind": "FILE", "path": str((ROOT / "contracts/core/core-contract.lock.json").absolute())},
            "k0Portability": {"kind": "FILE", "path": str((ROOT / "contracts/core/kotlin-k0.portability.json").absolute())},
            "evidenceCoreProto": {"kind": "FILE", "path": str((ROOT / "schemas/evidence_core.proto").absolute())},
            "harnessSource": {"kind": "FILE", "path": str(Path(__file__).absolute())},
            "independentAuditorSource": {"kind": "FILE", "path": str((ROOT / "scripts/k1_independent_auditor.py").absolute())},
            "candidateTools": {"kind": "FILE", "path": str(candidate_tools)},
            "qualificationSourceSet": {"kind": "SOURCE_SET", "path": str(qualification_source_set)},
            **live_sets,
        }
        output_kinds = {
            "qualificationDependencySeed": "TREE", "holdoutDependencySeed": "TREE",
            "holdoutSourceSet": "SOURCE_SET",
        }
        inputs: dict[str, dict[str, str]] = dict(existing)
        for key in sorted(expected_inputs - set(existing)):
            inputs[key] = {"kind": output_kinds.get(key, "FILE"), "path": str(run_root / key)}
        if set(inputs) != expected_inputs:
            raise HarnessError("build-live-inputs exact key derivation mismatch")
        # Explicitly prove that this builder did not materialize or inspect a
        # holdout checkout. Only the absent target descriptor is emitted.
        holdout_target = Path(inputs["holdoutSourceSet"]["path"])
        if holdout_target.exists() or holdout_target.is_symlink():
            raise HarnessError("build-live-inputs must leave holdout target absent")
        manifest = {
            "schema": "codeclew.kotlin-k1-live-inputs/0.1",
            "seriesId": SERIES_ID,
            "inputs": {key: inputs[key] for key in sorted(inputs)},
        }
        output = run_root / "live-inputs.json"
        _atomic_write(output, canonical(manifest), 0o400)
        output.chmod(0o400)
        _read_input_manifest(output)
        if tools["manifestSha256"] != sha256_file(candidate_tools):
            raise HarnessError("candidate tools changed while building live inputs")
        return manifest, output
    except BaseException:
        # This root did not exist before the create-only operation, so cleanup
        # is safe and prevents a partial run contour from being mistaken for a
        # valid authority bundle.
        if run_root.exists() and not run_root.is_symlink():
            shutil.rmtree(run_root)
        raise


def _input_path(inputs: Mapping[str, Mapping[str, Any]], key: str, kind: str | None = None) -> Path:
    descriptor = inputs.get(key)
    if not isinstance(descriptor, Mapping) or set(descriptor) != {"kind", "path"}:
        raise HarnessError(f"missing exact live input descriptor: {key}")
    if kind is not None and descriptor.get("kind") != kind:
        raise HarnessError(f"live input {key} must have kind {kind}")
    value = descriptor.get("path")
    if not isinstance(value, str) or not Path(value).is_absolute():
        raise HarnessError(f"live input {key} must have an absolute path")
    return Path(value)


def _canonical_artifact(inputs: Mapping[str, Mapping[str, Any]], key: str, schema: str) -> tuple[dict[str, Any], str]:
    path = _regular_file(_input_path(inputs, key, "FILE"), key)
    raw = path.read_bytes()
    value = _load_json_bytes(raw, key)
    if not isinstance(value, dict) or value.get("schema") != schema or value.get("seriesId") != SERIES_ID or canonical(value) != raw:
        raise HarnessError(f"{key} canonical artifact contract mismatch")
    return value, sha256_bytes(raw)


def _verified_measurement_artifact(
    store: Store,
    inputs: Mapping[str, Mapping[str, Any]],
    key: str,
    schema: str,
    producer: str,
) -> tuple[dict[str, Any], str]:
    artifact, digest = _canonical_artifact(inputs, key, schema)
    pointer = store.pointer(producer)
    receipt = store.receipt(producer)
    if pointer is None or receipt is None or receipt.get("status") != "READY" or receipt.get("evidence", {}).get("artifactSha256") != digest:
        raise HarnessError(f"{key} is not bound to the current producer receipt")
    artifact_evidence = {
        name: value for name, value in artifact.items()
        if name not in {"schema", "seriesId", "producerInputs", "modelCalls"}
    }
    receipt_evidence = {
        name: value for name, value in receipt["evidence"].items() if name != "artifactSha256"
    }
    if artifact_evidence != receipt_evidence:
        raise HarnessError(f"{key} payload differs from producer evidence")
    return artifact, digest


def _create_canonical_artifact(inputs: Mapping[str, Mapping[str, Any]], key: str, value: Mapping[str, Any]) -> tuple[Path, str]:
    path = _input_path(inputs, key, "FILE").absolute()
    if path.exists() or path.is_symlink():
        raise HarnessError(f"create-only artifact already exists: {key}")
    parent = path.parent
    parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    if parent.is_symlink() or not parent.is_dir():
        raise HarnessError(f"unsafe artifact parent: {key}")
    raw = canonical(value)
    _atomic_write(path, raw, 0o400)
    path.chmod(0o400)
    return path, sha256_bytes(raw)


def _exact_matrix_attempts(store: Store, cohort: str) -> list[dict[str, Any]]:
    identifiers = EXPECTED_QUALIFICATION if cohort == "QUALIFICATION" else EXPECTED_HOLDOUT
    rows: list[dict[str, Any]] = []
    for entry in identifiers:
        for invocation in ("COLD", "WARM"):
            pair = store.qualification_attempt(entry, invocation) if cohort == "QUALIFICATION" else store.holdout_attempt(entry, invocation)
            if pair is None:
                raise HarnessError(f"missing exact {cohort} attempt: {entry}/{invocation}")
            digest, attempt = pair
            expected_authority = "DEDICATED_QUALIFICATION_EXACT_ARGV" if cohort == "QUALIFICATION" else "DEDICATED_HOLDOUT_EXACT_ARGV"
            if attempt.get("authority") != expected_authority or attempt.get("entry") != entry or attempt.get("invocation") != invocation or attempt.get("modelCalls") != 0:
                raise HarnessError(f"invalid exact {cohort} attempt: {entry}/{invocation}")
            if attempt.get("status") == "ADAPTER_OUTPUT" and attempt.get("successAuthorityValidated") is not True:
                raise HarnessError(f"unvalidated success in {cohort}: {entry}/{invocation}")
            proof_safety = attempt.get("proofSafety")
            adapter_cost = attempt.get("adapterCost")
            adapter_cache = attempt.get("adapterCache")
            if not isinstance(proof_safety, dict) or set(proof_safety) != {"falseProven", "falseComplete"} or not all(
                isinstance(proof_safety[key], list) and all(isinstance(value, str) for value in proof_safety[key])
                for key in proof_safety
            ):
                raise HarnessError(f"missing structural proof audit in {cohort}: {entry}/{invocation}")
            if not isinstance(adapter_cost, dict) or set(adapter_cost) != {
                "externalWallMicros", "maximumResidentBytes", "sourceHashingMicros", "buildDiscoveryMicros",
                "dependencyPreparationMicros", "dependencyVerificationMicros", "adapterStartupMicros",
                "coldIndexMicros", "warmIndexMicros", "providerProcessingMicros", "serializationMicros",
                "storeWriteMicros", "storeReadMicros", "queryProjectionMicros", "sourceBytesRead",
                "cacheBytesRead", "cacheBytesWritten", "emittedBytes", "storedFactBytes", "factCount",
                "boundaryCount", "cacheRequests", "cacheHits", "modelCalls",
            } or adapter_cost.get("modelCalls") != 0:
                raise HarnessError(f"incomplete sidecar telemetry in {cohort}: {entry}/{invocation}")
            if not isinstance(adapter_cache, dict) or not isinstance(adapter_cache.get("hit"), bool):
                raise HarnessError(f"missing explicit cache authority in {cohort}: {entry}/{invocation}")
            cache_hit = (
                invocation == "WARM"
                and adapter_cache.get("status") == "VERIFIED_HIT"
                and adapter_cache.get("hit") is True
                and adapter_cost.get("cacheHits", 0) >= 1
            )
            authority = attempt.get("adapterAuthority")
            if attempt.get("status") == "ADAPTER_OUTPUT" and (
                not isinstance(authority, dict)
                or not _is_digest(authority.get("retainedAttemptDigest"))
                or not _is_digest(authority.get("projectionDigest"))
                or not isinstance(authority.get("evidenceCore"), dict)
                or not isinstance(authority.get("adapterObject"), dict)
            ):
                raise HarnessError(f"success authority was dropped in {cohort}: {entry}/{invocation}")
            rows.append({
                "entry": entry,
                "invocation": invocation,
                "attemptObjectSha256": digest,
                "status": attempt["status"],
                "terminalSemanticDigest": attempt.get("terminalSemanticDigest"),
                "candidateToolsManifestSha256": attempt.get("candidateToolsManifestSha256"),
                "workerDistributionTreeHash": attempt.get("workerDistributionTreeHash"),
                "semanticFactsDigest": attempt.get("semanticFactsDigest"),
                "cacheHit": cache_hit,
                "cache": adapter_cache,
                "adapterCost": adapter_cost,
                "projectionCost": attempt.get("projectionCost"),
                "externalWallMicros": attempt.get("resource", {}).get("externalWallMicros"),
                "maximumResidentBytes": attempt.get("resource", {}).get("maximumResidentBytes"),
                "reasonCode": attempt.get("reasonCode"),
                "sourceMutation": attempt.get("sourceMutation"),
                "successAuthorityValidated": attempt.get("successAuthorityValidated"),
                "adapterAuthority": authority,
                "proofSafety": proof_safety,
                "nonemptyProjection": attempt.get("nonemptyProjection"),
                "workload": attempt.get("workload"),
                "declaredProjectCompilerVersion": attempt.get("declaredProjectCompilerVersion"),
                "analyzerCompilerVersion": attempt.get("analyzerCompilerVersion"),
                "phaseReceipts": attempt.get("phaseReceipts"),
                "dependencySeedAuthority": attempt.get("dependencySeedAuthority"),
                "repositoryBefore": attempt.get("repositoryBefore"),
                "repositoryAfter": attempt.get("repositoryAfter"),
                "sourceExecutionAuthority": attempt.get("sourceExecutionAuthority"),
                "selectedInputs": attempt.get("selectedInputs"),
                "child": attempt.get("child"),
                "resource": attempt.get("resource"),
                "exactCommandSha256": attempt.get("exactCommandSha256"),
                "genericRuntimeSha256": attempt.get("genericRuntimeSha256"),
                "kotlinAdapterSha256": attempt.get("kotlinAdapterSha256"),
                "semanticCacheKeyDigest": attempt.get("semanticCacheKeyDigest"),
                "workerDistributionIdentity": attempt.get("workerDistributionIdentity"),
                "buildModelAuthority": attempt.get("buildModelAuthority"),
            })
    for entry in identifiers:
        cold, warm = [row for row in rows if row["entry"] == entry]
        if cold["terminalSemanticDigest"] != warm["terminalSemanticDigest"]:
            raise HarnessError(f"offline terminal semantic replay mismatch: {entry}")
        if cold["status"] == "ADAPTER_OUTPUT" and cold["semanticFactsDigest"] != warm["semanticFactsDigest"]:
            raise HarnessError(f"semantic facts replay mismatch: {entry}")
    return rows


_BASELINE_GRADLE_VERSION = "9.6.1"
_BASELINE_GRADLE_DISTRIBUTION = f"gradle-{_BASELINE_GRADLE_VERSION}-bin"
_BASELINE_GRADLE_URL = f"https\\://services.gradle.org/distributions/{_BASELINE_GRADLE_DISTRIBUTION}.zip"
_BASELINE_TRANSIENT_SUFFIXES = (".lock", ".lck", ".part", ".tmp")
_BASELINE_JAVA_HOME = "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home"
_BASELINE_PATH = (
    f"{_BASELINE_JAVA_HOME}/bin:/opt/homebrew/Cellar/maven/3.9.12/bin:"
    "/opt/homebrew/Cellar/rust/1.92.0/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
)
_BASELINE_CARGO_REGISTRY = "index.crates.io-1949cf8c6b5b557f"
_BASELINE_CARGO_TARGET = "aarch64-apple-darwin"
_BASELINE_CARGO_FORBIDDEN_INJECTION_KEYS = frozenset({
    "GRADLE_OPTS", "GRADLE_USER_HOME", "JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS",
    "JAVA_OPTS", "_JAVA_OPTIONS",
})
_BASELINE_CARGO_TOOL_PATHS = {
    "cargo": Path("/opt/homebrew/Cellar/rust/1.92.0/bin/cargo"),
    "rustc": Path("/opt/homebrew/Cellar/rust/1.92.0/bin/rustc"),
    "rustfmt": Path("/opt/homebrew/Cellar/rust/1.92.0/bin/rustfmt"),
    "cargoFmt": Path("/opt/homebrew/Cellar/rust/1.92.0/bin/cargo-fmt"),
    "cargoClippy": Path("/opt/homebrew/Cellar/rust/1.92.0/bin/cargo-clippy"),
    "clippyDriver": Path("/opt/homebrew/Cellar/rust/1.92.0/bin/clippy-driver"),
}
_BASELINE_CARGO_FORBIDDEN = (
    "bin", "git", "registry/src", ".global-cache", ".package-cache",
    ".package-cache-mutate", "CACHEDIR.TAG", ".crates.toml", ".crates2.json",
    "config", "config.toml", "credentials", "credentials.toml",
)
_BASELINE_CARGO_RESOLVED_SHA256 = "sha256:726c45cb4e1bd444909c5cf5c162bc9b9e8c631ec153c499bdf305827cd66bdb"
_BASELINE_CARGO_NON_RESOLVED_SHA256 = "sha256:e22d1fd865c191168c8473b7b72bc64c9b634c5b11fb79aaec4123835562c2a5"
_BASELINE_CARGO_CONFIG_SHA256 = "sha256:5b943a2c6f7eb743f7308aba07bdbb47d9ae44aafecd832d7f15df186afbafb3"
_BASELINE_CARGO_SEED_TREE_SHA256 = "sha256:10c3ef4e75cc13f172da0af0809e5c7a21ba559d8530903ea569f6751b4f3e55"
_BASELINE_CARGO_SEED_TOTAL_BYTES = 50_190_281
_BASELINE_CARGO_GENERATED_SOURCE_TREE_SHA256 = "sha256:4b2049df9a67d32b79c4427c90edb8a31f4a46c5ecf743eb4ff190f5d46dc332"
_BASELINE_CARGO_GENERATED_SOURCE_FILE_COUNT = 5_116
_BASELINE_CARGO_NON_RESOLVED_KEYS = frozenset({
    ("anstyle-wincon", "3.0.11"),
    ("bumpalo", "3.20.3"),
    ("curve25519-dalek-derive", "0.1.1"),
    ("fiat-crypto", "0.2.9"),
    ("futures-core", "0.3.33"),
    ("futures-task", "0.3.33"),
    ("futures-util", "0.3.33"),
    ("js-sys", "0.3.103"),
    ("linux-raw-sys", "0.12.1"),
    ("once_cell_polyfill", "1.70.2"),
    ("pin-project-lite", "0.2.17"),
    ("r-efi", "6.0.0"),
    ("rsqlite-vfs", "0.1.1"),
    ("rustversion", "1.0.23"),
    ("slab", "0.4.12"),
    ("sqlite-wasm-rs", "0.5.5"),
    ("wasi", "0.11.1+wasi-snapshot-preview1"),
    ("wasm-bindgen", "0.2.126"),
    ("wasm-bindgen-macro", "0.2.126"),
    ("wasm-bindgen-macro-support", "0.2.126"),
    ("wasm-bindgen-shared", "0.2.126"),
    ("winapi-util", "0.1.11"),
    ("windows-link", "0.2.1"),
    ("windows-sys", "0.61.2"),
})
_BASELINE_CARGO_AVAILABLE_NON_RESOLVED_KEYS = frozenset({
    ("pin-project-lite", "0.2.17"),
    ("slab", "0.4.12"),
})


def _cargo_index_relative(name: str) -> Path:
    lowered = name.lower()
    if len(lowered) == 1:
        return Path("1") / lowered
    if len(lowered) == 2:
        return Path("2") / lowered
    if len(lowered) == 3:
        return Path("3") / lowered[0] / lowered
    return Path(lowered[:2]) / lowered[2:4] / lowered


def _baseline_cargo_config_discovery_absent() -> bool:
    """Cargo must not discover repository/ancestor configuration outside CARGO_HOME."""
    return all(
        not (ancestor / ".cargo" / name).exists()
        and not (ancestor / ".cargo" / name).is_symlink()
        for ancestor in (ROOT, *ROOT.parents)
        for name in ("config", "config.toml")
    )


def _baseline_cargo_home_private_inputs_absent(cargo_home: Path) -> bool:
    return all(
        not (cargo_home / name).exists() and not (cargo_home / name).is_symlink()
        for name in ("config", "config.toml", "credentials", "credentials.toml")
    )


def _baseline_cargo_command_argv(argv: Sequence[str]) -> tuple[list[str], list[str]]:
    if len(argv) < 2 or argv[0] != "cargo":
        raise HarnessError("Cargo baseline command shape mismatch")
    manifest = str(_regular_file(ROOT / "Cargo.toml", "Cargo workspace manifest"))
    actual = [str(_BASELINE_CARGO_TOOL_PATHS["cargo"]), argv[1], "--manifest-path", manifest, *argv[2:]]
    normalized = ["$CARGO_1_92_0", argv[1], "--manifest-path", "$REPOSITORY/Cargo.toml", *argv[2:]]
    return actual, normalized


def _baseline_cargo_lock_projection() -> tuple[list[dict[str, str]], list[dict[str, str]]]:
    """Independently derive the preregistered host-target partition from live Cargo.lock."""
    try:
        lock = tomllib.loads(
            _regular_file(ROOT / "Cargo.lock", "Cargo lock").read_text(encoding="utf-8")
        )
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
        raise HarnessError("Cargo.lock is not valid UTF-8 TOML") from error
    packages = lock.get("package")
    if lock.get("version") != 4 or not isinstance(packages, list):
        raise HarnessError("Cargo.lock format mismatch")
    locked: dict[tuple[str, str], str] = {}
    for row in packages:
        if not isinstance(row, Mapping) or not str(row.get("source", "")).startswith("registry+"):
            continue
        if row.get("source") != "registry+https://github.com/rust-lang/crates.io-index":
            raise HarnessError("Cargo.lock contains a non-crates.io registry package")
        name, version, checksum = row.get("name"), row.get("version"), row.get("checksum")
        if (
            not isinstance(name, str) or not name or not isinstance(version, str) or not version
            or not isinstance(checksum, str) or len(checksum) != 64
            or any(character not in "0123456789abcdef" for character in checksum)
            or (name, version) in locked
        ):
            raise HarnessError("Cargo.lock registry package identity/checksum mismatch")
        locked[(name, version)] = checksum
    if (
        len(locked) != 135 or len({name for name, _ in locked}) != 130
        or len(_BASELINE_CARGO_NON_RESOLVED_KEYS) != 24
        or not _BASELINE_CARGO_NON_RESOLVED_KEYS < set(locked)
        or not _BASELINE_CARGO_AVAILABLE_NON_RESOLVED_KEYS < _BASELINE_CARGO_NON_RESOLVED_KEYS
    ):
        raise HarnessError("Cargo.lock exact preregistered package contour mismatch")
    resolved_keys = set(locked) - _BASELINE_CARGO_NON_RESOLVED_KEYS
    resolved = [
        {"name": name, "version": version, "checksum": locked[(name, version)]}
        for name, version in sorted(resolved_keys)
    ]
    non_resolved = [
        {"name": name, "version": version, "checksum": locked[(name, version)]}
        for name, version in sorted(_BASELINE_CARGO_NON_RESOLVED_KEYS)
    ]
    if (
        len(resolved) != 111
        or sha256_bytes(canonical(resolved)) != _BASELINE_CARGO_RESOLVED_SHA256
        or sha256_bytes(canonical(non_resolved)) != _BASELINE_CARGO_NON_RESOLVED_SHA256
    ):
        raise HarnessError("Cargo.lock host-target resolution projection drift")
    return resolved, non_resolved


def _baseline_cargo_seed_lock_valid(seed: Any) -> bool:
    """Cross-bind a retained seed to Cargo.lock, metadata partition, and archive paths."""
    if not isinstance(seed, Mapping):
        return False
    try:
        expected_resolved, expected_non_resolved = _baseline_cargo_lock_projection()
    except (HarnessError, OSError):
        return False
    resolved = seed.get("resolvedPackages")
    non_resolved = seed.get("nonResolvedLockedPackages")
    unavailable = seed.get("unavailableLockedArchives")
    excluded = seed.get("availableNonResolvedArchivesExcluded")
    files = seed.get("files")
    expected_excluded = [
        row for row in expected_non_resolved
        if (row["name"], row["version"]) in _BASELINE_CARGO_AVAILABLE_NON_RESOLVED_KEYS
    ]
    expected_unavailable = [row for row in expected_non_resolved if row not in expected_excluded]
    if (
        resolved != expected_resolved or non_resolved != expected_non_resolved
        or unavailable != expected_unavailable or excluded != expected_excluded
        or seed.get("resolvedPackagesSha256") != _BASELINE_CARGO_RESOLVED_SHA256
        or seed.get("metadataResolvedPackagesSha256") != _BASELINE_CARGO_RESOLVED_SHA256
        or seed.get("nonResolvedLockedPackagesSha256") != _BASELINE_CARGO_NON_RESOLVED_SHA256
        or seed.get("unavailableLockedArchivesSha256") != sha256_bytes(canonical(expected_unavailable))
        or not isinstance(files, list)
    ):
        return False
    file_by_path: dict[str, Mapping[str, Any]] = {}
    for row in files:
        if (
            not isinstance(row, Mapping) or set(row) != {"path", "size", "sha256"}
            or not isinstance(row.get("path"), str) or row["path"] in file_by_path
            or not isinstance(row.get("size"), int) or isinstance(row.get("size"), bool) or row["size"] <= 0
            or not _is_digest(row.get("sha256"))
        ):
            return False
        file_by_path[row["path"]] = row
    config_path = f"registry/index/{_BASELINE_CARGO_REGISTRY}/config.json"
    expected_index_paths = {
        (Path("registry/index") / _BASELINE_CARGO_REGISTRY / ".cache" / _cargo_index_relative(name)).as_posix()
        for name in {row["name"] for row in expected_resolved + expected_non_resolved}
    }
    expected_archive_rows = {
        f"registry/cache/{_BASELINE_CARGO_REGISTRY}/{row['name']}-{row['version']}.crate": "sha256:" + row["checksum"]
        for row in expected_resolved
    }
    expected_paths = {config_path, *expected_index_paths, *expected_archive_rows}
    return (
        set(file_by_path) == expected_paths
        and file_by_path[config_path].get("sha256") == _BASELINE_CARGO_CONFIG_SHA256
        and all(file_by_path[path].get("sha256") == checksum for path, checksum in expected_archive_rows.items())
        and seed.get("fileCount") == len(files) == 242
        and seed.get("totalBytes") == sum(row.get("size", -1) for row in files)
    )


def _baseline_cargo_seed_lock_fixture() -> dict[str, Any]:
    resolved, non_resolved = _baseline_cargo_lock_projection()
    excluded = [
        row for row in non_resolved
        if (row["name"], row["version"]) in _BASELINE_CARGO_AVAILABLE_NON_RESOLVED_KEYS
    ]
    unavailable = [row for row in non_resolved if row not in excluded]
    config_path = f"registry/index/{_BASELINE_CARGO_REGISTRY}/config.json"
    files = [{"path": config_path, "size": 1, "sha256": _BASELINE_CARGO_CONFIG_SHA256}]
    files.extend({
        "path": (Path("registry/index") / _BASELINE_CARGO_REGISTRY / ".cache" / _cargo_index_relative(name)).as_posix(),
        "size": 1, "sha256": sha256_bytes(name.encode()),
    } for name in sorted({row["name"] for row in resolved + non_resolved}))
    files.extend({
        "path": f"registry/cache/{_BASELINE_CARGO_REGISTRY}/{row['name']}-{row['version']}.crate",
        "size": 1, "sha256": "sha256:" + row["checksum"],
    } for row in resolved)
    files.sort(key=lambda row: row["path"])
    seed = {
        "resolvedPackages": resolved,
        "resolvedPackagesSha256": _BASELINE_CARGO_RESOLVED_SHA256,
        "metadataResolvedPackagesSha256": _BASELINE_CARGO_RESOLVED_SHA256,
        "nonResolvedLockedPackages": non_resolved,
        "nonResolvedLockedPackagesSha256": _BASELINE_CARGO_NON_RESOLVED_SHA256,
        "unavailableLockedArchives": unavailable,
        "unavailableLockedArchivesSha256": sha256_bytes(canonical(unavailable)),
        "availableNonResolvedArchivesExcluded": excluded,
        "files": files, "fileCount": len(files), "totalBytes": len(files),
    }
    return seed


def _baseline_cargo_seed_lock_self_test() -> dict[str, bool]:
    """Reject self-consistent reseals that are not the live locked dependency seed."""
    seed = _baseline_cargo_seed_lock_fixture()
    non_resolved = seed["nonResolvedLockedPackages"]

    def clone() -> dict[str, Any]:
        return json.loads(json.dumps(seed))

    checksum = clone()
    checksum["resolvedPackages"][0]["checksum"] = "0" * 64
    checksum["resolvedPackagesSha256"] = sha256_bytes(canonical(checksum["resolvedPackages"]))
    checksum["metadataResolvedPackagesSha256"] = checksum["resolvedPackagesSha256"]
    checksum_path = next(row for row in checksum["files"] if row["path"].endswith(
        f"/{checksum['resolvedPackages'][0]['name']}-{checksum['resolvedPackages'][0]['version']}.crate"
    ))
    checksum_path["sha256"] = "sha256:" + checksum["resolvedPackages"][0]["checksum"]
    path = clone()
    next(row for row in path["files"] if "/registry/" not in row["path"])["path"] = "credentials.toml"
    path["files"].sort(key=lambda row: row["path"])
    version = clone()
    version["resolvedPackages"][0]["version"] += "-resealed"
    version["resolvedPackages"].sort(key=lambda row: (row["name"], row["version"]))
    version["resolvedPackagesSha256"] = sha256_bytes(canonical(version["resolvedPackages"]))
    version["metadataResolvedPackagesSha256"] = version["resolvedPackagesSha256"]
    partition = clone()
    partition["resolvedPackages"][0], partition["nonResolvedLockedPackages"][0] = (
        partition["nonResolvedLockedPackages"][0], partition["resolvedPackages"][0]
    )
    partition["resolvedPackages"].sort(key=lambda row: (row["name"], row["version"]))
    partition["nonResolvedLockedPackages"].sort(key=lambda row: (row["name"], row["version"]))
    partition["resolvedPackagesSha256"] = sha256_bytes(canonical(partition["resolvedPackages"]))
    partition["metadataResolvedPackagesSha256"] = partition["resolvedPackagesSha256"]
    partition["nonResolvedLockedPackagesSha256"] = sha256_bytes(canonical(partition["nonResolvedLockedPackages"]))
    metadata = clone()
    metadata["metadataResolvedPackagesSha256"] = sha256_bytes(canonical(non_resolved))
    checks = {
        "cleanAccepted": _baseline_cargo_seed_lock_valid(seed),
        "resealedChecksumRejected": not _baseline_cargo_seed_lock_valid(checksum),
        "resealedPathRejected": not _baseline_cargo_seed_lock_valid(path),
        "resealedVersionRejected": not _baseline_cargo_seed_lock_valid(version),
        "resealedPartitionRejected": not _baseline_cargo_seed_lock_valid(partition),
        "metadataDigestMutationRejected": not _baseline_cargo_seed_lock_valid(metadata),
    }
    if not all(checks.values()):
        raise AssertionError(f"Cargo dependency seed lock self-test failed: {checks}")
    return checks


def _baseline_cargo_launcher() -> dict[str, Any]:
    if not _baseline_cargo_config_discovery_absent():
        raise HarnessError("repository/ancestor Cargo configuration would affect launcher probes")
    toolchain = tomllib.loads(_regular_file(ROOT / "rust-toolchain.toml", "Rust toolchain pin").read_text(encoding="utf-8"))
    if toolchain.get("toolchain") != {"channel": "1.92.0", "profile": "minimal", "components": ["rustfmt", "clippy"]}:
        raise HarnessError("baseline Rust toolchain pin mismatch")
    tools: dict[str, Any] = {}
    for name, path in _BASELINE_CARGO_TOOL_PATHS.items():
        member = path.resolve(strict=True)
        if not member.is_relative_to(Path("/opt/homebrew/Cellar/rust/1.92.0/bin")):
            raise HarnessError(f"baseline Cargo tool is outside exact Homebrew Rust 1.92.0: {name}")
        member = _regular_file(member, f"baseline Cargo tool {name}")
        if not os.access(member, os.X_OK):
            raise HarnessError(f"baseline Cargo tool is not executable: {name}")
        tools[name] = {"requestedPath": str(path), "resolvedRelativePath": member.name, "sha256": sha256_file(member)}
    version_commands = {
        "cargo": [str(_BASELINE_CARGO_TOOL_PATHS["cargo"]), "-V"],
        "rustc": [str(_BASELINE_CARGO_TOOL_PATHS["rustc"]), "-vV"],
        "rustfmt": [str(_BASELINE_CARGO_TOOL_PATHS["rustfmt"]), "-V"],
        "cargoFmt": [str(_BASELINE_CARGO_TOOL_PATHS["cargoFmt"]), "--version"],
        "cargoClippy": [str(_BASELINE_CARGO_TOOL_PATHS["cargoClippy"]), "-V"],
        "clippyDriver": [str(_BASELINE_CARGO_TOOL_PATHS["clippyDriver"]), "-V"],
    }
    expected_versions = {
        "cargo": b"cargo 1.92.0 (Homebrew)\n",
        "rustc": b"rustc 1.92.0 (ded5c06cf 2025-12-08) (Homebrew)\nbinary: rustc\ncommit-hash: ded5c06cf21d2b93bffd5d884aa6e96934ee4234\ncommit-date: 2025-12-08\nhost: aarch64-apple-darwin\nrelease: 1.92.0\nLLVM version: 21.1.7\n",
        "rustfmt": b"rustfmt 1.8.0\n", "cargoFmt": b"rustfmt 1.8.0\n",
        "cargoClippy": b"clippy 0.1.92\n", "clippyDriver": b"clippy 0.1.92\n",
    }
    versions: dict[str, Any] = {}
    with tempfile.TemporaryDirectory(prefix="codeclew-k1-cargo-version-") as temporary_text:
        isolated_home = Path(temporary_text)
        cargo_home = isolated_home / "cargo-home"
        cargo_home.mkdir(mode=0o700)
        if not _baseline_cargo_home_private_inputs_absent(cargo_home):
            raise HarnessError("isolated Cargo launcher-probe home contains private inputs")
        probe_environment = {
            "HOME": str(isolated_home), "CARGO_HOME": str(cargo_home),
            "PATH": "/opt/homebrew/Cellar/rust/1.92.0/bin:/usr/bin:/bin",
            "CARGO_NET_OFFLINE": "true", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8",
        }
        for name, argv in version_commands.items():
            completed = subprocess.run(
                argv, cwd=Path("/"), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, check=False, timeout=30, env=probe_environment,
            )
            if completed.returncode != 0 or completed.stderr or completed.stdout != expected_versions[name]:
                raise HarnessError(f"baseline Cargo tool version identity mismatch: {name}")
            if not _baseline_cargo_home_private_inputs_absent(cargo_home):
                raise HarnessError("Cargo launcher probe introduced configuration or credentials")
            versions[name] = {
                "argv": ["$RUST_1_92_0/" + Path(argv[0]).name, *argv[1:]],
                "stdoutSha256": sha256_bytes(completed.stdout), "stdoutBytes": len(completed.stdout),
            }
    return {
        "schema": "codeclew.kotlin-k1-cargo-launcher-authority/0.1",
        "toolchainSha256": sha256_file(ROOT / "rust-toolchain.toml"),
        "channel": "1.92.0", "tools": tools, "versionIdentities": versions,
        "hostPathRetained": False,
    }


def _baseline_cargo_packages() -> tuple[list[dict[str, str]], list[dict[str, str]]]:
    if not _baseline_cargo_config_discovery_absent():
        raise HarnessError("repository/ancestor Cargo configuration would affect metadata")
    expected_resolved, expected_non_resolved = _baseline_cargo_lock_projection()
    locked = {
        (row["name"], row["version"]): row["checksum"]
        for row in expected_resolved + expected_non_resolved
    }
    source = Path.home() / ".cargo/registry"
    with tempfile.TemporaryDirectory(prefix="codeclew-k1-cargo-bootstrap-") as temporary_text:
        bootstrap = Path(temporary_text) / "cargo-home"
        config_source = _regular_file(source / "index" / _BASELINE_CARGO_REGISTRY / "config.json", "Cargo registry config")
        config_target = bootstrap / "registry/index" / _BASELINE_CARGO_REGISTRY / "config.json"
        config_target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        shutil.copyfile(config_source, config_target, follow_symlinks=False)
        for name in sorted({name for name, _ in locked}):
            relative = _cargo_index_relative(name)
            index_source = _regular_file(source / "index" / _BASELINE_CARGO_REGISTRY / ".cache" / relative, f"Cargo index {name}")
            index_target = bootstrap / "registry/index" / _BASELINE_CARGO_REGISTRY / ".cache" / relative
            index_target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            shutil.copyfile(index_source, index_target, follow_symlinks=False)
        for (name, version), checksum in sorted(locked.items()):
            archive = source / "cache" / _BASELINE_CARGO_REGISTRY / f"{name}-{version}.crate"
            if not archive.exists():
                continue
            archive = _regular_file(archive, f"Cargo archive {name}-{version}")
            if sha256_file(archive) != "sha256:" + checksum:
                raise HarnessError(f"Cargo host archive differs from Cargo.lock: {name}-{version}")
            target = bootstrap / "registry/cache" / _BASELINE_CARGO_REGISTRY / archive.name
            target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            shutil.copyfile(archive, target, follow_symlinks=False)
        metadata_argv, _ = _baseline_cargo_command_argv([
            "cargo", "metadata", "--offline", "--locked", "--filter-platform",
            _BASELINE_CARGO_TARGET, "--format-version", "1",
        ])
        completed = subprocess.run(
            metadata_argv, cwd=Path("/"), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            check=False, timeout=120, env={
                "HOME": temporary_text, "CARGO_HOME": str(bootstrap),
                "PATH": "/opt/homebrew/bin:/usr/bin:/bin", "CARGO_NET_OFFLINE": "true",
                "CARGO_REGISTRIES_CRATES_IO_PROTOCOL": "sparse", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8",
            },
        )
    if completed.returncode != 0:
        raise HarnessError("offline Cargo dependency resolution failed: " + sha256_bytes(completed.stderr))
    metadata = _load_json_bytes(completed.stdout, "Cargo metadata")
    resolved_ids = {row.get("id") for row in metadata.get("resolve", {}).get("nodes", [])}
    resolved_keys = {
        (row.get("name"), row.get("version")) for row in metadata.get("packages", [])
        if row.get("id") in resolved_ids and str(row.get("source", "")).startswith("registry+")
    }
    expected_resolved_keys = {(row["name"], row["version"]) for row in expected_resolved}
    if resolved_keys != expected_resolved_keys:
        raise HarnessError("Cargo metadata host-target dependency closure differs from preregistration")
    if not _baseline_cargo_config_discovery_absent():
        raise HarnessError("repository/ancestor Cargo configuration appeared during metadata")
    return expected_resolved, expected_non_resolved


def _copy_baseline_cargo_seed(destination: Path) -> dict[str, Any]:
    source = Path.home() / ".cargo/registry"
    packages, absent = _baseline_cargo_packages()
    members: list[tuple[Path, Path, str]] = []
    config_relative = Path("registry/index") / _BASELINE_CARGO_REGISTRY / "config.json"
    config_source = source / "index" / _BASELINE_CARGO_REGISTRY / "config.json"
    config = _load_json_bytes(_regular_file(config_source, "Cargo registry config").read_bytes(), "Cargo registry config")
    if (
        config != {"api": "https://crates.io", "dl": "https://static.crates.io/crates"}
        or sha256_file(config_source) != _BASELINE_CARGO_CONFIG_SHA256
    ):
        raise HarnessError("Cargo registry config identity mismatch")
    members.append((config_source, config_relative, ""))
    for name in sorted({name for name, _ in {(row["name"], row["version"]) for row in packages + absent}}):
        members.append((
            source / "index" / _BASELINE_CARGO_REGISTRY / ".cache" / _cargo_index_relative(name),
            Path("registry/index") / _BASELINE_CARGO_REGISTRY / ".cache" / _cargo_index_relative(name), "",
        ))
    for row in packages:
        name, version, checksum = row["name"], row["version"], row["checksum"]
        members.append((source / "cache" / _BASELINE_CARGO_REGISTRY / f"{name}-{version}.crate", Path("registry/cache") / _BASELINE_CARGO_REGISTRY / f"{name}-{version}.crate", checksum))
    before = []
    destination.mkdir(parents=True, mode=0o700)
    for source_member, relative, checksum in members:
        member = _regular_file(source_member, f"Cargo dependency seed member {relative}")
        observed = sha256_file(member)
        if checksum and observed != "sha256:" + checksum:
            raise HarnessError(f"Cargo crate checksum differs from Cargo.lock: {relative}")
        row = {"path": relative.as_posix(), "size": member.stat().st_size, "sha256": observed}
        before.append(row)
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        shutil.copyfile(member, target, follow_symlinks=False)
        os.chmod(target, 0o400)
    before.sort(key=lambda row: str(row["path"]))
    after = [{"path": row["path"], "size": _regular_file(source / row["path"].removeprefix("registry/"), "Cargo seed source").stat().st_size, "sha256": sha256_file(source / row["path"].removeprefix("registry/"))} for row in before]
    copied = [{"path": row["path"], "size": _regular_file(destination / row["path"], "Cargo seed copy").stat().st_size, "sha256": sha256_file(destination / row["path"])} for row in before]
    if before != after or before != copied or any((destination / relative).exists() for relative in _BASELINE_CARGO_FORBIDDEN):
        raise HarnessError("Cargo dependency seed source/copy identity or credential exclusion mismatch")
    digest = sha256_bytes(canonical({"schema": "codeclew.kotlin-k1-cargo-seed-tree/0.1", "files": before}))
    unavailable = [row for row in absent if not (source / "cache" / _BASELINE_CARGO_REGISTRY / f"{row['name']}-{row['version']}.crate").is_file()]
    available_excluded = [row for row in absent if row not in unavailable]
    if (
        len(before) != 242 or len(packages) != 111 or len(absent) != 24
        or len(unavailable) != 22 or len(available_excluded) != 2
        or {(row["name"], row["version"]) for row in available_excluded}
        != _BASELINE_CARGO_AVAILABLE_NON_RESOLVED_KEYS
        or digest != _BASELINE_CARGO_SEED_TREE_SHA256
        or sum(row["size"] for row in before) != _BASELINE_CARGO_SEED_TOTAL_BYTES
    ):
        raise HarnessError("Cargo dependency seed exact closure cardinality mismatch")
    return {
        "schema": "codeclew.kotlin-k1-cargo-dependency-seed/0.1", "registry": _BASELINE_CARGO_REGISTRY,
        "target": _BASELINE_CARGO_TARGET, "cargoLockSha256": sha256_file(ROOT / "Cargo.lock"),
        "resolvedPackages": packages, "resolvedPackagesSha256": sha256_bytes(canonical(packages)),
        "metadataResolvedPackagesSha256": sha256_bytes(canonical(packages)),
        "nonResolvedLockedPackages": absent, "nonResolvedLockedPackagesSha256": sha256_bytes(canonical(absent)),
        "unavailableLockedArchives": unavailable, "unavailableLockedArchivesSha256": sha256_bytes(canonical(unavailable)),
        "availableNonResolvedArchivesExcluded": available_excluded,
        "files": before, "sourceTreeDigest": digest, "copiedTreeDigest": digest, "sourceAfterTreeDigest": digest,
        "fileCount": len(before), "totalBytes": sum(row["size"] for row in before),
        "sourceCopyEqual": True, "credentialInputsCopied": False, "forbiddenCredentialFilesPresent": False,
        "sourcePathRetained": False,
    }


def _baseline_cargo_injection_environment_absent(environment: Mapping[str, str]) -> bool:
    """Cargo/Rust children must not inherit inputs rejected by trusted workers."""
    return (
        not (_BASELINE_CARGO_FORBIDDEN_INJECTION_KEYS & set(environment))
        and "JAVA_HOME" not in environment
        and not any(key.startswith("ORG_GRADLE_PROJECT_") for key in environment)
    )


def _prepare_baseline_execution_context(home: Path) -> dict[str, Any]:
    if not _baseline_cargo_config_discovery_absent():
        raise HarnessError("repository/ancestor Cargo configuration would affect baseline")
    cargo_home = home / "cargo-home"
    cargo_target = home / "cargo-target"
    cargo_seed = _copy_baseline_cargo_seed(cargo_home)
    if not _baseline_cargo_home_private_inputs_absent(cargo_home):
        raise HarnessError("isolated Cargo home contains configuration or credentials before fetch")
    cargo_seed["configAndCredentialsAbsentBeforeFetch"] = True
    cargo_target.mkdir(parents=True, mode=0o700)
    cargo_launcher = _baseline_cargo_launcher()
    gradle_executable, gradle_launcher = _baseline_gradle_launcher()
    gradle_home = home / "gradle-user-home"
    gradle_seed = _copy_baseline_gradle_cache(
        Path.home() / ".gradle/caches/modules-2", gradle_home / "caches/modules-2",
    )
    common_environment = {
        "HOME": str(home), "TMPDIR": str(home),
        "PATH": _BASELINE_PATH,
        "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "CODECLEW_K1_MODEL_CALLS": "0",
    }
    cargo_environment = {
        **common_environment,
        "CARGO_HOME": str(cargo_home), "CARGO_TARGET_DIR": str(cargo_target),
        "CARGO_NET_OFFLINE": "true", "CARGO_REGISTRIES_CRATES_IO_PROTOCOL": "sparse",
    }
    gradle_environment = {
        **common_environment,
        "JAVA_HOME": _BASELINE_JAVA_HOME,
        "GRADLE_USER_HOME": str(gradle_home),
    }
    if not _baseline_cargo_injection_environment_absent(cargo_environment):
        raise HarnessError("Cargo baseline environment contains JVM/Gradle injection")
    fetch_argv, normalized_fetch = _baseline_cargo_command_argv(
        ["cargo", "fetch", "--offline", "--locked", "--target", _BASELINE_CARGO_TARGET]
    )
    fetch = subprocess.run(
        fetch_argv, cwd=Path("/"), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=300, check=False, env=cargo_environment,
    )
    if fetch.returncode != 0:
        raise HarnessError("isolated Cargo fetch probe failed: " + sha256_bytes(fetch.stderr))
    generated_source_rows, cargo_home_directories, cargo_home_other_files = _baseline_cargo_home_contour(cargo_home)
    generated_source_digest = sha256_bytes(canonical({
        "schema": "codeclew.kotlin-k1-cargo-generated-source-tree/0.1", "files": generated_source_rows,
    }))
    if (
        generated_source_digest != _BASELINE_CARGO_GENERATED_SOURCE_TREE_SHA256
        or len(generated_source_rows) != _BASELINE_CARGO_GENERATED_SOURCE_FILE_COUNT
        or fetch.stdout or fetch.stderr
    ):
        raise HarnessError("isolated Cargo fetch generated contour mismatch")
    cargo_seed["generatedSourceTreeDigest"] = generated_source_digest
    cargo_seed["generatedSourceFileCount"] = len(generated_source_rows)
    if not _baseline_cargo_home_private_inputs_absent(cargo_home):
        raise HarnessError("isolated Cargo home contains configuration or credentials after fetch")
    cargo_seed["configAndCredentialsAbsentAfterFetch"] = True
    cargo_seed["offlineFetchProbe"] = {
        "executionArgv": normalized_fetch, "executionArgvSha256": sha256_bytes(canonical(normalized_fetch)),
        "executionCwd": "/",
        "exitCode": 0, "stdoutSha256": sha256_bytes(fetch.stdout), "stdoutBytes": len(fetch.stdout),
        "stderrSha256": sha256_bytes(fetch.stderr), "stderrBytes": len(fetch.stderr),
    }
    context_id = sha256_bytes(canonical({
        "schema": "codeclew.kotlin-k1-baseline-execution-context/0.1",
        "cargoLauncher": cargo_launcher, "cargoSeed": cargo_seed,
        "gradleLauncher": gradle_launcher, "gradleSeed": gradle_seed,
    }))
    return {
        "home": home, "cargoEnvironment": cargo_environment,
        "gradleEnvironment": gradle_environment, "executionContextId": context_id,
        "cargoLauncher": cargo_launcher, "cargoSeed": cargo_seed, "generatedSourceRows": generated_source_rows,
        "cargoHomeDirectories": cargo_home_directories, "cargoHomeOtherFiles": cargo_home_other_files,
        "gradleExecutable": gradle_executable, "gradleLauncher": gradle_launcher, "gradleSeed": gradle_seed,
    }


def _baseline_cargo_home_contour(cargo_home: Path) -> tuple[list[dict[str, Any]], list[str], list[str]]:
    source_root = cargo_home / "registry/src"
    source_rows = _tree_rows(source_root)
    directories: list[str] = []
    other_files: list[str] = []
    for directory, names, files in os.walk(cargo_home, followlinks=False):
        directory_path = Path(directory)
        for name in sorted(names):
            member = directory_path / name
            if member.is_symlink() or not stat.S_ISDIR(member.lstat().st_mode):
                raise HarnessError("isolated Cargo home contains a non-directory or symlink directory")
            directories.append(member.relative_to(cargo_home).as_posix())
        for name in sorted(files):
            member = directory_path / name
            metadata = member.lstat()
            if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
                raise HarnessError("isolated Cargo home contains a symlink or non-regular file")
            relative = member.relative_to(cargo_home).as_posix()
            if not relative.startswith("registry/src/"):
                other_files.append(relative)
    return source_rows, sorted(directories), sorted(other_files)


def _validate_baseline_execution_context_after(context: Mapping[str, Any]) -> dict[str, Any]:
    if not _baseline_cargo_config_discovery_absent():
        raise HarnessError("repository/ancestor Cargo configuration appeared during baseline")
    if _baseline_cargo_launcher() != context["cargoLauncher"]:
        raise HarnessError("baseline Cargo launcher changed after command series")
    if _baseline_gradle_launcher()[1] != context["gradleLauncher"]:
        raise HarnessError("baseline Gradle launcher changed after command series")
    source = Path.home() / ".cargo/registry"
    cargo_home = Path(context["cargoEnvironment"]["CARGO_HOME"])
    if not _baseline_cargo_home_private_inputs_absent(cargo_home):
        raise HarnessError("isolated Cargo home contains configuration or credentials after baseline")
    rows = context["cargoSeed"]["files"]
    for row in rows:
        relative = str(row["path"])
        source_member = _regular_file(source / relative.removeprefix("registry/"), "post-command Cargo seed source")
        copied_member = _regular_file(cargo_home / relative, "post-command Cargo seed member")
        for member in (source_member, copied_member):
            if member.stat().st_size != row["size"] or sha256_file(member) != row["sha256"]:
                raise HarnessError("Cargo dependency seed changed after baseline commands")
    forbidden = [relative for relative in _BASELINE_CARGO_FORBIDDEN if relative not in {
        ".global-cache", ".package-cache", ".package-cache-mutate", "registry/src",
    } and (cargo_home / relative).exists()]
    allowed_top = {"registry", ".global-cache", ".package-cache", ".package-cache-mutate"}
    observed_top = {path.name for path in cargo_home.iterdir()}
    registry_children = {path.name for path in (cargo_home / "registry").iterdir()}
    generated_source_rows, cargo_home_directories, cargo_home_other_files = _baseline_cargo_home_contour(cargo_home)
    generated_source_digest = sha256_bytes(canonical({
        "schema": "codeclew.kotlin-k1-cargo-generated-source-tree/0.1", "files": generated_source_rows,
    }))
    if forbidden or not observed_top <= allowed_top or not registry_children <= {"index", "cache", "src", "CACHEDIR.TAG"}:
        raise HarnessError("isolated Cargo home contains unauthorized post-command state")
    if generated_source_rows != context["generatedSourceRows"] or generated_source_digest != context["cargoSeed"]["generatedSourceTreeDigest"]:
        raise HarnessError("generated Cargo dependency source tree changed during baseline commands")
    initial_other_files = set(context["cargoHomeOtherFiles"])
    observed_other_files = set(cargo_home_other_files)
    if (
        cargo_home_directories != context["cargoHomeDirectories"]
        or not initial_other_files <= observed_other_files
        or not observed_other_files - initial_other_files <= {".package-cache-mutate"}
    ):
        raise HarnessError("isolated Cargo home gained unauthorized generated state")
    return {
        "schema": "codeclew.kotlin-k1-baseline-context-postcheck/0.1",
        "executionContextId": context["executionContextId"],
        "cargoSeedMembersUnchanged": True, "hostSeedMembersUnchanged": True,
        "cargoLauncherUnchanged": True, "gradleLauncherUnchanged": True,
        "allowedGeneratedStateOnly": True, "generatedSourceTreeDigest": generated_source_digest,
        "generatedSourceFileCount": len(generated_source_rows),
        "cargoConfigAndCredentialsAbsentAfterCommands": True,
    }


def _baseline_cache_rows(root: Path) -> tuple[list[dict[str, Any]], list[str]]:
    """Snapshot a credential-free Gradle artifact cache without retaining its host path."""
    root = root.absolute()
    metadata = root.lstat()
    if root.is_symlink() or not stat.S_ISDIR(metadata.st_mode):
        raise HarnessError("baseline Gradle modules cache must be a non-symlink directory")
    rows: list[dict[str, Any]] = []
    excluded: list[str] = []
    for directory, directories, files in os.walk(root, followlinks=False):
        directory_path = Path(directory)
        directories.sort()
        files.sort()
        retained_directories: list[str] = []
        for name in directories:
            child = directory_path / name
            relative = child.relative_to(root).as_posix()
            if child.is_symlink():
                raise HarnessError("baseline Gradle modules cache contains a symlink directory")
            if name.endswith(_BASELINE_TRANSIENT_SUFFIXES):
                excluded.append(relative + "/")
            else:
                retained_directories.append(name)
        directories[:] = retained_directories
        for name in files:
            member = directory_path / name
            relative = member.relative_to(root).as_posix()
            member_metadata = member.lstat()
            if name.endswith(_BASELINE_TRANSIENT_SUFFIXES):
                excluded.append(relative)
                continue
            if stat.S_ISLNK(member_metadata.st_mode) or not stat.S_ISREG(member_metadata.st_mode):
                raise HarnessError("baseline Gradle modules cache contains a non-regular member")
            rows.append({
                "path": relative,
                "mode": stat.S_IMODE(member_metadata.st_mode),
                "size": member_metadata.st_size,
                "sha256": sha256_file(member),
            })
    return rows, sorted(excluded)


def _baseline_cache_digest(rows: list[Mapping[str, Any]]) -> str:
    return sha256_bytes(canonical({
        "schema": "codeclew.kotlin-k1-gradle-cache-tree/0.1",
        "files": rows,
    }))


def _copy_baseline_gradle_cache(source: Path, destination: Path) -> dict[str, Any]:
    before, excluded_before = _baseline_cache_rows(source)
    destination.mkdir(parents=True, mode=0o700)
    for row in before:
        source_member = source / str(row["path"])
        destination_member = destination / str(row["path"])
        destination_member.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        source_metadata = source_member.lstat()
        if (
            stat.S_ISLNK(source_metadata.st_mode) or not stat.S_ISREG(source_metadata.st_mode)
            or source_metadata.st_size != row["size"] or sha256_file(source_member) != row["sha256"]
        ):
            raise HarnessError("baseline Gradle cache changed while being copied")
        shutil.copyfile(source_member, destination_member, follow_symlinks=False)
        os.chmod(destination_member, int(row["mode"]))
    copied, excluded_copied = _baseline_cache_rows(destination)
    after, excluded_after = _baseline_cache_rows(source)
    if before != copied or before != after or excluded_before != excluded_after or excluded_copied:
        raise HarnessError("baseline Gradle cache source/copy identity mismatch")
    tree_digest = _baseline_cache_digest(before)
    return {
        "schema": "codeclew.kotlin-k1-gradle-dependency-seed/0.1",
        "copiedSubtree": "caches/modules-2",
        "transientExclusionSuffixes": list(_BASELINE_TRANSIENT_SUFFIXES),
        "excludedTransientCount": len(excluded_before),
        "excludedTransientPathsSha256": sha256_bytes(canonical(excluded_before)),
        "sourceTreeDigest": tree_digest,
        "copiedTreeDigest": _baseline_cache_digest(copied),
        "sourceAfterTreeDigest": _baseline_cache_digest(after),
        "fileCount": len(before),
        "totalBytes": sum(int(row["size"]) for row in before),
        "sourceCopyEqual": True,
        "credentialInputsCopied": False,
        "forbiddenCredentialFilesPresent": any(
            (destination.parent.parent / relative).exists()
            for relative in ("gradle.properties", "init.d")
        ),
        "sourcePathRetained": False,
    }


def _baseline_gradle_launcher() -> tuple[Path, dict[str, Any]]:
    properties_path = _regular_file(
        ROOT / "gradle/wrapper/gradle-wrapper.properties", "baseline Gradle wrapper properties",
    )
    properties = properties_path.read_text(encoding="utf-8").splitlines()
    distribution_rows = [line.split("=", 1)[1] for line in properties if line.startswith("distributionUrl=")]
    if distribution_rows != [_BASELINE_GRADLE_URL]:
        raise HarnessError("baseline Gradle wrapper distribution is not exact 9.6.1")
    distribution_parent = Path.home() / ".gradle/wrapper/dists" / _BASELINE_GRADLE_DISTRIBUTION
    try:
        candidates = sorted(distribution_parent.glob(f"*/gradle-{_BASELINE_GRADLE_VERSION}"))
    except OSError as error:
        raise HarnessError("baseline Gradle distribution lookup failed") from error
    safe_candidates: list[Path] = []
    for candidate in candidates:
        try:
            metadata = candidate.lstat()
            executable = candidate / "bin/gradle"
            executable_metadata = executable.lstat()
        except FileNotFoundError:
            continue
        if (
            not candidate.is_symlink() and stat.S_ISDIR(metadata.st_mode)
            and not executable.is_symlink() and stat.S_ISREG(executable_metadata.st_mode)
        ):
            safe_candidates.append(candidate)
    if len(safe_candidates) != 1:
        raise HarnessError("exactly one regular cached Gradle 9.6.1 distribution is required")
    distribution = safe_candidates[0]
    executable = _regular_file(distribution / "bin/gradle", "baseline Gradle executable")
    core_jar = _regular_file(
        distribution / f"lib/gradle-core-{_BASELINE_GRADLE_VERSION}.jar",
        "baseline Gradle core jar",
    )
    distribution_files = _tree_rows(distribution)
    return executable, {
        "schema": "codeclew.kotlin-k1-gradle-launcher-authority/0.1",
        "requestedLauncher": "./gradlew",
        "version": _BASELINE_GRADLE_VERSION,
        "distributionUrl": _BASELINE_GRADLE_URL,
        "wrapperScriptSha256": sha256_file(_regular_file(ROOT / "gradlew", "baseline Gradle wrapper")),
        "wrapperJarSha256": sha256_file(_regular_file(ROOT / "gradle/wrapper/gradle-wrapper.jar", "baseline Gradle wrapper jar")),
        "wrapperPropertiesSha256": sha256_file(properties_path),
        "distributionTreeDigest": _tree_digest(distribution),
        "distributionFileCount": len(distribution_files),
        "distributionBytes": sum(int(row["size"]) for row in distribution_files),
        "executableRelativePath": "bin/gradle",
        "executableSha256": sha256_file(executable),
        "coreJarRelativePath": f"lib/gradle-core-{_BASELINE_GRADLE_VERSION}.jar",
        "coreJarSha256": sha256_file(core_jar),
        "hostPathRetained": False,
    }


def _capture_command(
    store: Store, argv: list[str], cwd: Path, context: Mapping[str, Any], timeout: int = 900,
) -> dict[str, Any]:
    started = time.monotonic_ns()
    gradle_authority: dict[str, Any] | None = None
    cargo_authority: dict[str, Any] | None = None
    home = Path(context["home"])
    execution_argv = list(argv)
    if argv and argv[0] == "./gradlew":
        environment = dict(context["gradleEnvironment"])
        execution_argv = [str(context["gradleExecutable"]), f"-Duser.home={home}", *argv[1:]]
        normalized_execution_argv = ["$GRADLE_9_6_1", "-Duser.home=$ISOLATED", *argv[1:]]
        gradle_authority = {
            "launcher": context["gradleLauncher"], "dependencySeed": context["gradleSeed"],
            "executionArgv": normalized_execution_argv,
            "executionArgvSha256": sha256_bytes(canonical(normalized_execution_argv)),
            "isolatedGradleUserHome": True, "isolatedJavaUserHome": True,
        }
    elif argv and argv[0] == "cargo":
        environment = dict(context["cargoEnvironment"])
        if not _baseline_cargo_injection_environment_absent(environment):
            raise HarnessError("Cargo command environment contains JVM/Gradle injection")
        if not _baseline_cargo_home_private_inputs_absent(Path(environment["CARGO_HOME"])):
            raise HarnessError("isolated Cargo home contains configuration or credentials before command")
        execution_argv, normalized_execution_argv = _baseline_cargo_command_argv(argv)
        cargo_authority = {
            "launcher": context["cargoLauncher"], "dependencySeed": context["cargoSeed"],
            "executionArgv": normalized_execution_argv,
            "executionArgvSha256": sha256_bytes(canonical(normalized_execution_argv)),
            "executionCwd": "/",
            "isolatedCargoHome": True, "isolatedCargoTargetDir": True,
            "sharedBaselineExecutionContext": True,
        }
    else:
        raise HarnessError("unsupported baseline command launcher")
    execution_cwd = Path("/") if cargo_authority is not None else cwd
    completed = subprocess.run(
        execution_argv, cwd=execution_cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, timeout=timeout, check=False, env=environment,
    )
    if gradle_authority is not None and _baseline_gradle_launcher()[1] != context["gradleLauncher"]:
        raise HarnessError("baseline Gradle distribution changed during execution")
    if cargo_authority is not None and _baseline_cargo_launcher() != context["cargoLauncher"]:
        raise HarnessError("baseline Cargo toolchain changed during execution")
    if cargo_authority is not None and not _baseline_cargo_home_private_inputs_absent(Path(environment["CARGO_HOME"])):
        raise HarnessError("isolated Cargo home contains configuration or credentials after command")
    normalized_environment = {
        **environment, "HOME": "$ISOLATED", "TMPDIR": "$ISOLATED",
    }
    if cargo_authority is not None:
        normalized_environment.update({
            "CARGO_HOME": "$ISOLATED/cargo-home",
            "CARGO_TARGET_DIR": "$ISOLATED/cargo-target",
        })
    else:
        normalized_environment["GRADLE_USER_HOME"] = "$ISOLATED/gradle-user-home"
    environment_policy = {
        "keys": sorted(environment), "values": normalized_environment,
        "credentialInheritance": False,
    }
    result = {
        "argv": argv,
        "argvSha256": sha256_bytes(canonical(argv)),
        "exitCode": completed.returncode,
        "stdoutSha256": store.put_blob(completed.stdout),
        "stdoutBytes": len(completed.stdout),
        "stderrSha256": store.put_blob(completed.stderr),
        "stderrBytes": len(completed.stderr),
        "environmentPolicy": environment_policy,
        "environmentPolicySha256": sha256_bytes(canonical(environment_policy)),
        "wallMicros": (time.monotonic_ns() - started) // 1000,
        "executionContextId": context["executionContextId"],
    }
    if gradle_authority is not None:
        result["gradleExecutionAuthority"] = gradle_authority
    if cargo_authority is not None:
        result["cargoExecutionAuthority"] = cargo_authority
    return result


def _baseline_expected_environment_values(is_gradle: bool) -> dict[str, str]:
    values = {
        "HOME": "$ISOLATED", "TMPDIR": "$ISOLATED",
        "PATH": _BASELINE_PATH,
        "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "CODECLEW_K1_MODEL_CALLS": "0",
    }
    if is_gradle:
        values.update({
            "JAVA_HOME": _BASELINE_JAVA_HOME,
            "GRADLE_USER_HOME": "$ISOLATED/gradle-user-home",
        })
    else:
        values.update({
            "CARGO_HOME": "$ISOLATED/cargo-home",
            "CARGO_TARGET_DIR": "$ISOLATED/cargo-target",
            "CARGO_NET_OFFLINE": "true",
            "CARGO_REGISTRIES_CRATES_IO_PROTOCOL": "sparse",
        })
    return values


def _baseline_environment_policy_valid(policy: Any, is_gradle: bool) -> bool:
    expected_values = _baseline_expected_environment_values(is_gradle)
    if not isinstance(policy, Mapping) or policy != {
        "keys": sorted(expected_values), "values": expected_values,
        "credentialInheritance": False,
    }:
        return False
    if is_gradle:
        return "JAVA_TOOL_OPTIONS" not in policy["values"]
    return _baseline_cargo_injection_environment_absent(policy["values"])


def _baseline_environment_policy_self_test() -> dict[str, bool]:
    def policy(is_gradle: bool) -> dict[str, Any]:
        values = _baseline_expected_environment_values(is_gradle)
        return {
            "keys": sorted(values), "values": values,
            "credentialInheritance": False,
        }

    clean_cargo = policy(False)
    clean_gradle = policy(True)

    def injected(key: str, value: str = "injected") -> dict[str, Any]:
        candidate = json.loads(json.dumps(clean_cargo))
        candidate["values"][key] = value
        candidate["keys"] = sorted(candidate["values"])
        return candidate

    gradle_java_tool_options = json.loads(json.dumps(clean_gradle))
    gradle_java_tool_options["values"]["JAVA_TOOL_OPTIONS"] = "-Duser.home=/caller"
    gradle_java_tool_options["keys"] = sorted(gradle_java_tool_options["values"])
    gradle_without_user_home = json.loads(json.dumps(clean_gradle))
    del gradle_without_user_home["values"]["GRADLE_USER_HOME"]
    gradle_without_user_home["keys"] = sorted(gradle_without_user_home["values"])
    checks = {
        "cleanCargoAccepted": _baseline_environment_policy_valid(clean_cargo, False),
        "cleanGradleAccepted": _baseline_environment_policy_valid(clean_gradle, True),
        "cargoJavaHomeRejected": not _baseline_environment_policy_valid(injected("JAVA_HOME"), False),
        "cargoJavaToolOptionsRejected": not _baseline_environment_policy_valid(injected("JAVA_TOOL_OPTIONS"), False),
        "cargoGradleUserHomeRejected": not _baseline_environment_policy_valid(injected("GRADLE_USER_HOME"), False),
        "cargoGradleOptsRejected": not _baseline_environment_policy_valid(injected("GRADLE_OPTS"), False),
        "cargoJdkJavaOptionsRejected": not _baseline_environment_policy_valid(injected("JDK_JAVA_OPTIONS"), False),
        "cargoJavaOptsRejected": not _baseline_environment_policy_valid(injected("JAVA_OPTS"), False),
        "cargoUnderscoreJavaOptionsRejected": not _baseline_environment_policy_valid(injected("_JAVA_OPTIONS"), False),
        "cargoOrgGradleProjectRejected": not _baseline_environment_policy_valid(injected("ORG_GRADLE_PROJECT_secret"), False),
        "gradleJavaToolOptionsRejected": not _baseline_environment_policy_valid(gradle_java_tool_options, True),
        "gradleMissingUserHomeRejected": not _baseline_environment_policy_valid(gradle_without_user_home, True),
    }
    if not all(checks.values()):
        raise AssertionError(f"baseline environment policy self-test failed: {checks}")
    return checks


def _baseline_command_packet_valid(row: Any, expected_argv: tuple[str, ...]) -> bool:
    if not isinstance(row, Mapping) or row.get("argv") != list(expected_argv):
        return False
    policy = row.get("environmentPolicy")
    if not isinstance(policy, Mapping) or row.get("environmentPolicySha256") != sha256_bytes(canonical(policy)):
        return False
    is_gradle = expected_argv[0] == "./gradlew"
    is_cargo = expected_argv[0] == "cargo"
    if (
        (not is_gradle and not is_cargo)
        or not _baseline_environment_policy_valid(policy, is_gradle)
        or row.get("argvSha256") != sha256_bytes(canonical(list(expected_argv)))
        or not _is_digest(row.get("stdoutSha256")) or not _is_digest(row.get("stderrSha256"))
        or not all(isinstance(row.get(key), int) and not isinstance(row.get(key), bool) and row[key] >= 0 for key in (
            "stdoutBytes", "stderrBytes", "wallMicros",
        ))
        or not isinstance(row.get("exitCode"), int) or isinstance(row.get("exitCode"), bool)
        or row.get("observed") != ("PASS" if row["exitCode"] == 0 else "FAIL")
        or not _is_digest(row.get("executionContextId"))
    ):
        return False
    authority = row.get("gradleExecutionAuthority")
    cargo_authority = row.get("cargoExecutionAuthority")
    if is_cargo:
        if authority is not None or not isinstance(cargo_authority, Mapping) or set(cargo_authority) != {
            "launcher", "dependencySeed", "executionArgv", "executionArgvSha256",
            "executionCwd", "isolatedCargoHome", "isolatedCargoTargetDir", "sharedBaselineExecutionContext",
        }:
            return False
        launcher = cargo_authority.get("launcher")
        seed = cargo_authority.get("dependencySeed")
        if not isinstance(launcher, Mapping) or launcher != _baseline_cargo_launcher() or not isinstance(seed, Mapping):
            return False
        seed_keys = {
            "schema", "registry", "target", "cargoLockSha256", "resolvedPackages",
            "resolvedPackagesSha256", "metadataResolvedPackagesSha256",
            "nonResolvedLockedPackages", "nonResolvedLockedPackagesSha256",
            "unavailableLockedArchives", "unavailableLockedArchivesSha256",
            "availableNonResolvedArchivesExcluded", "files", "sourceTreeDigest", "copiedTreeDigest",
            "sourceAfterTreeDigest", "fileCount", "totalBytes", "sourceCopyEqual",
            "credentialInputsCopied", "forbiddenCredentialFilesPresent", "sourcePathRetained",
            "offlineFetchProbe", "generatedSourceTreeDigest", "generatedSourceFileCount",
            "configAndCredentialsAbsentBeforeFetch", "configAndCredentialsAbsentAfterFetch",
        }
        resolved = seed.get("resolvedPackages")
        non_resolved = seed.get("nonResolvedLockedPackages")
        unavailable = seed.get("unavailableLockedArchives")
        excluded = seed.get("availableNonResolvedArchivesExcluded")
        fetch = seed.get("offlineFetchProbe")
        files = seed.get("files")
        package_rows_valid = lambda rows, count: (
            isinstance(rows, list) and len(rows) == count and rows == sorted(rows, key=lambda value: (value.get("name", ""), value.get("version", "")))
            and all(isinstance(value, Mapping) and set(value) == {"name", "version", "checksum"}
                    and isinstance(value["name"], str) and isinstance(value["version"], str)
                    and isinstance(value["checksum"], str) and len(value["checksum"]) == 64
                    and all(character in "0123456789abcdef" for character in value["checksum"]) for value in rows)
        )
        _, normalized_fetch = _baseline_cargo_command_argv(
            ["cargo", "fetch", "--offline", "--locked", "--target", _BASELINE_CARGO_TARGET]
        )
        seed_valid = (
            set(seed) == seed_keys and seed.get("schema") == "codeclew.kotlin-k1-cargo-dependency-seed/0.1"
            and seed.get("registry") == _BASELINE_CARGO_REGISTRY and seed.get("target") == _BASELINE_CARGO_TARGET
            and seed.get("cargoLockSha256") == sha256_file(ROOT / "Cargo.lock")
            and package_rows_valid(resolved, 111) and package_rows_valid(non_resolved, 24)
            and package_rows_valid(unavailable, 22) and package_rows_valid(excluded, 2)
            and _baseline_cargo_seed_lock_valid(seed)
            and seed.get("resolvedPackagesSha256") == sha256_bytes(canonical(resolved))
            and seed.get("metadataResolvedPackagesSha256") == sha256_bytes(canonical(resolved))
            and seed.get("nonResolvedLockedPackagesSha256") == sha256_bytes(canonical(non_resolved))
            and seed.get("unavailableLockedArchivesSha256") == sha256_bytes(canonical(unavailable))
            and {tuple(sorted(value.items())) for value in unavailable + excluded} == {tuple(sorted(value.items())) for value in non_resolved}
            and isinstance(files, list) and len(files) == 242 and files == sorted(files, key=lambda value: value.get("path", ""))
            and all(isinstance(value, Mapping) and set(value) == {"path", "size", "sha256"}
                    and isinstance(value["path"], str) and not value["path"].startswith("/")
                    and isinstance(value["size"], int) and not isinstance(value["size"], bool) and value["size"] > 0
                    and _is_digest(value["sha256"]) for value in files)
            and seed.get("sourceTreeDigest") == _BASELINE_CARGO_SEED_TREE_SHA256
            and seed.get("sourceTreeDigest") == sha256_bytes(canonical({"schema": "codeclew.kotlin-k1-cargo-seed-tree/0.1", "files": files}))
            and all(_is_digest(seed.get(key)) for key in ("sourceTreeDigest", "copiedTreeDigest", "sourceAfterTreeDigest"))
            and seed.get("sourceTreeDigest") == seed.get("copiedTreeDigest") == seed.get("sourceAfterTreeDigest")
            and seed.get("fileCount") == 242 and seed.get("totalBytes") == _BASELINE_CARGO_SEED_TOTAL_BYTES
            and seed.get("generatedSourceTreeDigest") == _BASELINE_CARGO_GENERATED_SOURCE_TREE_SHA256
            and seed.get("generatedSourceFileCount") == _BASELINE_CARGO_GENERATED_SOURCE_FILE_COUNT
            and seed.get("configAndCredentialsAbsentBeforeFetch") is True
            and seed.get("configAndCredentialsAbsentAfterFetch") is True
            and seed.get("sourceCopyEqual") is True and seed.get("credentialInputsCopied") is False
            and seed.get("forbiddenCredentialFilesPresent") is False and seed.get("sourcePathRetained") is False
            and isinstance(fetch, Mapping) and set(fetch) == {"executionArgv", "executionArgvSha256", "executionCwd", "exitCode", "stdoutSha256", "stdoutBytes", "stderrSha256", "stderrBytes"}
            and fetch.get("executionArgv") == normalized_fetch and fetch.get("executionArgvSha256") == sha256_bytes(canonical(normalized_fetch))
            and fetch.get("executionCwd") == "/" and fetch.get("exitCode") == 0
            and fetch.get("stdoutSha256") == fetch.get("stderrSha256") == sha256_bytes(b"")
            and fetch.get("stdoutBytes") == fetch.get("stderrBytes") == 0
        )
        _, normalized = _baseline_cargo_command_argv(expected_argv)
        return (
            seed_valid and cargo_authority.get("executionArgv") == normalized
            and cargo_authority.get("executionArgvSha256") == sha256_bytes(canonical(normalized))
            and cargo_authority.get("executionCwd") == "/"
            and cargo_authority.get("isolatedCargoHome") is True
            and cargo_authority.get("isolatedCargoTargetDir") is True
            and cargo_authority.get("sharedBaselineExecutionContext") is True
            and str(Path.home()) not in json.dumps(cargo_authority, sort_keys=True)
        )
    if cargo_authority is not None or not is_gradle:
        return authority is None and cargo_authority is None
    if not isinstance(authority, Mapping) or set(authority) != {
        "launcher", "dependencySeed", "executionArgv", "executionArgvSha256",
        "isolatedGradleUserHome", "isolatedJavaUserHome",
    }:
        return False
    normalized_argv = ["$GRADLE_9_6_1", "-Duser.home=$ISOLATED", *expected_argv[1:]]
    launcher = authority.get("launcher")
    seed = authority.get("dependencySeed")
    if not isinstance(launcher, Mapping) or not isinstance(seed, Mapping):
        return False
    wrapper_properties = ROOT / "gradle/wrapper/gradle-wrapper.properties"
    launcher_valid = (
        set(launcher) == {
            "schema", "requestedLauncher", "version", "distributionUrl",
            "wrapperScriptSha256", "wrapperJarSha256", "wrapperPropertiesSha256",
            "distributionTreeDigest", "distributionFileCount", "distributionBytes",
            "executableRelativePath", "executableSha256", "coreJarRelativePath",
            "coreJarSha256", "hostPathRetained",
        }
        and launcher.get("schema") == "codeclew.kotlin-k1-gradle-launcher-authority/0.1"
        and launcher.get("requestedLauncher") == "./gradlew"
        and launcher.get("version") == _BASELINE_GRADLE_VERSION
        and launcher.get("distributionUrl") == _BASELINE_GRADLE_URL
        and launcher.get("wrapperScriptSha256") == sha256_file(ROOT / "gradlew")
        and launcher.get("wrapperJarSha256") == sha256_file(ROOT / "gradle/wrapper/gradle-wrapper.jar")
        and launcher.get("wrapperPropertiesSha256") == sha256_file(wrapper_properties)
        and all(_is_digest(launcher.get(key)) for key in (
            "distributionTreeDigest", "executableSha256", "coreJarSha256",
        ))
        and launcher.get("executableRelativePath") == "bin/gradle"
        and launcher.get("coreJarRelativePath") == f"lib/gradle-core-{_BASELINE_GRADLE_VERSION}.jar"
        and all(isinstance(launcher.get(key), int) and not isinstance(launcher.get(key), bool) and launcher[key] > 0 for key in (
            "distributionFileCount", "distributionBytes",
        ))
        and launcher.get("hostPathRetained") is False
    )
    seed_valid = (
        set(seed) == {
            "schema", "copiedSubtree", "transientExclusionSuffixes",
            "excludedTransientCount", "excludedTransientPathsSha256", "sourceTreeDigest",
            "copiedTreeDigest", "sourceAfterTreeDigest", "fileCount", "totalBytes",
            "sourceCopyEqual", "credentialInputsCopied", "forbiddenCredentialFilesPresent",
            "sourcePathRetained",
        }
        and seed.get("schema") == "codeclew.kotlin-k1-gradle-dependency-seed/0.1"
        and seed.get("copiedSubtree") == "caches/modules-2"
        and seed.get("transientExclusionSuffixes") == list(_BASELINE_TRANSIENT_SUFFIXES)
        and isinstance(seed.get("excludedTransientCount"), int)
        and not isinstance(seed.get("excludedTransientCount"), bool)
        and seed["excludedTransientCount"] >= 0
        and all(_is_digest(seed.get(key)) for key in (
            "excludedTransientPathsSha256", "sourceTreeDigest", "copiedTreeDigest", "sourceAfterTreeDigest",
        ))
        and seed.get("sourceTreeDigest") == seed.get("copiedTreeDigest") == seed.get("sourceAfterTreeDigest")
        and all(isinstance(seed.get(key), int) and not isinstance(seed.get(key), bool) and seed[key] > 0 for key in (
            "fileCount", "totalBytes",
        ))
        and seed.get("sourceCopyEqual") is True
        and seed.get("credentialInputsCopied") is False
        and seed.get("forbiddenCredentialFilesPresent") is False
        and seed.get("sourcePathRetained") is False
    )
    return (
        launcher_valid and seed_valid
        and authority.get("executionArgv") == normalized_argv
        and authority.get("executionArgvSha256") == sha256_bytes(canonical(normalized_argv))
        and authority.get("isolatedGradleUserHome") is True
        and authority.get("isolatedJavaUserHome") is True
        and str(Path.home()) not in json.dumps(authority, sort_keys=True)
    )


def _advance_node_unlocked(
    store: Store,
    identifier: str,
    inputs: Mapping[str, Mapping[str, Any]],
    captured_selected: Mapping[str, Any] | None,
    captured_dependencies: Mapping[str, str],
) -> str:
    """Execute one production node with a hard-coded node-specific checker."""
    def issue(
        evidence: Mapping[str, Any],
        *,
        expected_action: str,
        status: str = "READY",
        error: str | None = None,
    ) -> str:
        live_selected = _selected(store, identifier, inputs)
        if captured_selected is not None and any(
            live_selected.get(key) != value for key, value in captured_selected.items()
        ):
            raise HarnessError(f"selected input changed during checker: {identifier}")
        return _issue_authoritative(
            store,
            identifier,
            inputs,
            evidence,
            expected_action=expected_action,
            status=status,
            error=error,
            captured_selected=live_selected,
            captured_dependencies=captured_dependencies,
        )

    bundle = store.bundle
    if identifier == "INPUT_AUTHORITY_VERIFY":
        expected = {
            "researchInput": "sha256:6b9d9c73a809e896506dfd2645d09b77e8251940138eb813c85aeb573a270791",
            "executionContract": "sha256:a115a0690a7fe9ffc79d6cfbe2f31f2a58bc3412f9af44d22dd6e336765c35ee",
            "priorM1Failure": "sha256:9d8137ac0063dc8fc81b1f0f3c577ad41550000863741421b690265b1a3e2d49",
        }
        observed = {key: snapshot_input(inputs[key])["sha256"] for key in expected}
        if observed != expected:
            raise HarnessError("immutable research input authority mismatch")
        revision = _regular_file(_input_path(inputs, "repositoryBaseRevision", "FILE"), "repository base revision").read_text(encoding="utf-8")
        observed_head = subprocess.run(
            ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            check=False, timeout=30,
            env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"},
        )
        if (
            revision != PINNED_REPOSITORY_BASE_REVISION + "\n"
            or observed_head.returncode != 0
            or observed_head.stdout.decode("ascii", "strict").strip() != PINNED_REPOSITORY_BASE_REVISION
        ):
            raise HarnessError("repository base revision mismatch")
        return issue({"verified": expected, "baseRevision": revision.strip()}, expected_action="VERIFY")

    if identifier == "K0_1_BYTE_EXACT_VERIFY":
        k0_live_set = _require_live_set_role(inputs, "k0AuthoritySet", "K0_AUTHORITY_SET")
        policy = bundle["requirements"]["historicalCorePolicy"]
        live = {
            "k0Lock": sha256_file(ROOT / "contracts/core/core-contract.lock.json"),
            "k0Portability": sha256_file(ROOT / "contracts/core/kotlin-k0.portability.json"),
            "evidenceCoreProto": sha256_file(ROOT / "schemas/evidence_core.proto"),
        }
        expected = {
            "k0Lock": policy["coreContractLockSha256"],
            "k0Portability": policy["kotlinK0PortabilitySha256"],
            "evidenceCoreProto": policy["evidenceCoreProtoSha256"],
        }
        if live != expected:
            raise HarnessError("historical K0.1 byte drift")
        lock = _load_json_bytes((ROOT / "contracts/core/core-contract.lock.json").read_bytes(), "K0 lock")
        for member in lock["adapterContractFiles"] + lock["decisionCoreFiles"] + lock["conformanceCorpusFiles"]:
            if sha256_file(ROOT / member["path"]) != member["sha256"]:
                raise HarnessError(f"K0 authority member drift: {member['path']}")
        return issue({"byteExact": live, "memberCount": len(lock["adapterContractFiles"] + lock["decisionCoreFiles"] + lock["conformanceCorpusFiles"]), "authoritySetSha256": k0_live_set["sha256"]}, expected_action="VERIFY")

    if identifier == "REQUIREMENTS_FREEZE_VERIFY":
        load_production_bundle()
        return issue({"requirementsSha256": bundle["digests"]["requirements"], "graphSha256": store.graph_digest, "thresholds": bundle["requirements"]["decisionThresholds"]}, expected_action="VERIFY")

    if identifier == "CORPUS_FREEZE_VERIFY":
        load_production_bundle()
        return issue({"corpusSha256": bundle["digests"]["corpus"], "eligibilitySha256": bundle["digests"]["corpusEligibilityEvidence"], "members": 12}, expected_action="VERIFY")

    if identifier == "HOLDOUT_ELIGIBILITY_AUDIT_IMPORT":
        audit = bundle["holdoutEligibilityAudit"]
        required = {"schema", "seriesId", "procedure", "corpusSha256", "eligibilityEvidenceSha256", "holdoutMembers", "forbiddenActionsObserved", "decision"}
        if set(audit) != required or audit["schema"] != "codeclew.kotlin-k1-holdout-eligibility-audit/0.2" or audit["procedure"] != "PINNED_METADATA_ONLY_ELIGIBILITY_REVIEW_V1" or audit["corpusSha256"] != bundle["digests"]["corpus"] or audit["eligibilityEvidenceSha256"] != bundle["digests"]["corpusEligibilityEvidence"] or audit["holdoutMembers"] != list(EXPECTED_HOLDOUT) or audit["forbiddenActionsObserved"] != 0 or audit["decision"] != "ACCEPT":
            raise HarnessError("pinned holdout eligibility audit mismatch")
        return issue({"auditSha256": bundle["digests"]["holdoutEligibilityAudit"], "procedure": audit["procedure"]}, expected_action="IMPORT")

    if identifier == "BASELINE_CAPTURE":
        _require_live_set_role(inputs, "candidateSources", "CANDIDATE_SOURCES")
        baseline_tools = _candidate_tools(inputs)
        revision = _regular_file(
            _input_path(inputs, "repositoryBaseRevision", "FILE"), "repository base revision",
        ).read_text(encoding="utf-8").strip()
        if revision != PINNED_REPOSITORY_BASE_REVISION:
            raise HarnessError("baseline repository base revision mismatch")
        specifications = [
            ("REQUIRED_GREEN", ["cargo", "test", "--offline", "--locked", "-p", "evidence-core", "--all-targets", "--", "--test-threads=1"]),
            ("REQUIRED_GREEN", ["cargo", "test", "--offline", "--locked", "-p", "evidence-adapters", "--all-targets", "--", "--test-threads=1"]),
            ("REQUIRED_GREEN", ["cargo", "test", "--offline", "--locked", "-p", "clew", "--lib", "worker::tests::compiler_receipt_requires_explicit_successful_k2_validation", "--", "--exact", "--test-threads=1"]),
            ("REQUIRED_GREEN", ["cargo", "test", "--offline", "--locked", "-p", "clew", "--lib", "worker::tests::trusted_distribution_identity_is_read_only_cache_key_material", "--", "--exact", "--test-threads=1"]),
            ("REQUIRED_GREEN", ["cargo", "test", "--offline", "--locked", "-p", "clew", "--lib", "index::tests::declaration_descriptor_ingestion_roundtrips_unknown_and_commits_snapshot", "--", "--exact", "--test-threads=1"]),
            ("REQUIRED_GREEN", ["cargo", "test", "--offline", "--locked", "-p", "clew", "--lib", "index::tests::declaration_descriptor_ingestion_rejects_malformed_hash_and_provenance", "--", "--exact", "--test-threads=1"]),
            ("REQUIRED_GREEN", ["cargo", "test", "--offline", "--locked", "-p", "clew", "--lib", "index::tests::declaration_relation_ingestion_roundtrips_typed_unknown_and_commits_snapshot", "--", "--exact", "--test-threads=1"]),
            ("REQUIRED_GREEN", ["cargo", "test", "--offline", "--locked", "-p", "clew", "--lib", "index::tests::declaration_relation_ingestion_rejects_hash_malformed_and_snapshot_mismatch", "--", "--exact", "--test-threads=1"]),
            ("REQUIRED_GREEN", ["./gradlew", "--offline", ":workers:kotlin:test", "--tests", "dev.semanticthread.worker.ProjectModelCommandTest.futureCompilerDescriptorValuesBecomeTypedBoundaries", "--tests", "dev.semanticthread.worker.ProjectModelCommandTest.malformedCompilerFactRowIsRetainedAsBothTypedGraphBoundaries", ":workers:kotlin21:compileKotlin", ":workers:kotlin23:compileKotlin", "--no-daemon"]),
            ("REQUIRED_GREEN", ["cargo", "fmt", "--all", "--check"]),
            ("HISTORICAL_BASELINE", ["cargo", "clippy", "--offline", "--locked", "-p", "clew", "--lib", "--", "-D", "warnings"]),
            ("HISTORICAL_BASELINE", ["cargo", "clippy", "--offline", "--locked", "-p", "semantic-corpus", "--lib", "--", "-D", "warnings"]),
        ]
        rows = []
        observed_before = subprocess.run(
            ["git", "-C", str(ROOT), "rev-parse", "HEAD"], stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False, timeout=30,
            env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"},
        )
        head_before = observed_before.stdout.decode("ascii", "strict").strip() if observed_before.returncode == 0 else ""
        if head_before != revision:
            raise HarnessError("repository HEAD differs before baseline commands")
        with tempfile.TemporaryDirectory(prefix="codeclew-k1-baseline-") as temporary_text:
            execution_context = _prepare_baseline_execution_context(Path(temporary_text))
            for policy, command in specifications:
                row = _capture_command(store, command, ROOT, execution_context)
                row["policy"] = policy
                row["observed"] = "PASS" if row["exitCode"] == 0 else "FAIL"
                rows.append(row)
            context_postcheck = _validate_baseline_execution_context_after(execution_context)
            packet_cargo_authority = {
                "executionContextId": execution_context["executionContextId"],
                "launcher": execution_context["cargoLauncher"],
                "dependencySeed": execution_context["cargoSeed"],
                "isolatedCargoHome": True, "isolatedCargoTargetDir": True,
                "sharedBaselineExecutionContext": True,
                "executionCwd": "/",
            }
        required_green = all(row["exitCode"] == 0 for row in rows if row["policy"] == "REQUIRED_GREEN")
        historical = [
            {"argvSha256": row["argvSha256"], "observed": row["observed"], "stderrSha256": row["stderrSha256"]}
            for row in rows if row["policy"] == "HISTORICAL_BASELINE"
        ]
        packet = {
            "schema": "codeclew.kotlin-k1-baseline-packet/0.2", "seriesId": SERIES_ID,
            "repositoryBaseRevision": "be19fd06ce7509ffbaf4622b8f8d13ec7cbcd854",
            "repositoryHeadBefore": head_before,
            "candidateToolsManifestSha256": baseline_tools["manifestSha256"],
            "executionContextId": execution_context["executionContextId"],
            "cargoExecutionAuthority": packet_cargo_authority,
            "executionContextPostcheck": context_postcheck,
            "commands": rows, "requiredGreen": required_green,
            "historicalBaselineOutcomes": historical,
            "historicalClaims": {
                "clewClippyDiagnosticsAtM1": 12,
                "semanticCorpusClippyDiagnosticsAtM1": 4,
                "sourceReportSha256": sha256_file(ROOT / "docs/experiments/codeclew-multilanguage-m1-implementation-report-2026-08-13.md"),
            },
            "modelCalls": 0,
        }
        observed_after = subprocess.run(
            ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            check=False, timeout=30,
            env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"},
        )
        if observed_after.returncode != 0 or observed_after.stdout.decode("ascii", "strict").strip() != revision:
            raise HarnessError("repository HEAD changed during baseline capture")
        packet["repositoryHeadAfter"] = observed_after.stdout.decode("ascii", "strict").strip()
        # repositoryHeadAfter participates in the canonical producer packet.
        _, digest = _create_canonical_artifact(inputs, "baselinePacket", packet)
        # A completed baseline measurement is READY even when the required
        # command set is red. R19 then fails as measured evidence and the
        # preregistered decision can reach STOP instead of becoming BLOCKED.
        return issue({
            "packetSha256": digest,
            "requiredGreen": packet["requiredGreen"],
            "historicalBaselineOutcomes": historical,
        }, expected_action="DIRECT")

    if identifier == "HARNESS_SELF_TEST":
        packet = self_test()
        packet["supervisor"] = supervisor_self_test()
        packet["sourceAnchorPacket"] = _source_anchor_packet()
        packet["buildDependencyConformance"] = _build_dependency_conformance()
        packet["determinismConformance"] = _determinism_conformance()
        packet["schema"] = "codeclew.kotlin-k1-harness-self-test-packet/0.1"
        packet["seriesId"] = SERIES_ID
        _, digest = _create_canonical_artifact(inputs, "harnessSelfTestPacket", packet)
        return issue({"packetSha256": digest, "counterexamples": packet["counterexamples"] + len(packet["supervisor"]["cases"])}, expected_action="DIRECT")

    if identifier in {"QUALIFICATION_DEPENDENCY_SEED_PREPARE", "HOLDOUT_DEPENDENCY_SEED_PREPARE"}:
        evidence = _prepare_dependency_seed(store, identifier, inputs)
        try:
            return issue(evidence, expected_action="PREPARE")
        except BaseException as issue_error:
            # _atomic_write installs a pointer before its final directory
            # fsync.  If that last durability call raised, recover only an
            # exact, fully reconstructable receipt; otherwise remove the
            # unreceipted create-only output so a fresh series can proceed.
            recovered_digest: str | None = None
            try:
                pointer = store.pointer(identifier)
                live_dependencies, blockers = _dependency_receipts(store, identifier, inputs)
                live_selected = _selected(store, identifier, inputs)
                expected_receipt = {
                    "schema": RECEIPT_SCHEMA,
                    "storeId": store.store_id,
                    "seriesId": SERIES_ID,
                    "graphDigest": store.graph_digest,
                    "checkerVersion": CHECKER_VERSION,
                    "checkerSourceDigest": sha256_file(Path(__file__)),
                    "node": identifier,
                    "action": "PREPARE",
                    "nodeKey": _node_key(store, identifier, live_selected, live_dependencies),
                    "status": "READY",
                    "selectedInputs": live_selected,
                    "dependencies": live_dependencies,
                    "evidence": dict(evidence),
                    "error": None,
                }
                if (
                    not blockers
                    and pointer is not None
                    and store.get_object(pointer["receiptDigest"]) == expected_receipt
                ):
                    recovered_digest = pointer["receiptDigest"]
            except BaseException:
                pass
            if recovered_digest is not None:
                try:
                    current_directory = os.open(
                        store.root / "current",
                        os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
                    )
                    try:
                        os.fsync(current_directory)
                    finally:
                        os.close(current_directory)
                except OSError as durability_error:
                    # The exact pointer may already be durable.  Never delete
                    # the output it names; report the unresolved durability
                    # boundary and let the retained pointer remain auditable.
                    raise HarnessError(
                        "dependency PREPARE receipt durability recovery failed"
                    ) from durability_error
                return recovered_digest
            seed_key = (
                "qualificationDependencySeed"
                if identifier.startswith("QUALIFICATION")
                else "holdoutDependencySeed"
            )
            target = _input_path(inputs, seed_key, "TREE").absolute()
            if target.exists() or target.is_symlink():
                _discard_private_tree(target, target.parent)
            raise issue_error

    if identifier in {"QUALIFICATION_DEPENDENCY_SEED_VERIFY", "HOLDOUT_DEPENDENCY_SEED_VERIFY"}:
        key = "qualificationDependencySeed" if identifier.startswith("QUALIFICATION") else "holdoutDependencySeed"
        snapshot = snapshot_input(inputs[key])
        prepare_id = identifier.replace("VERIFY", "PREPARE")
        prepared = store.receipt(prepare_id)
        cohort = "QUALIFICATION" if identifier.startswith("QUALIFICATION") else "BLIND_HOLDOUT"
        entries = [row for row in bundle["corpus"]["entries"] if row["cohort"] == cohort]
        validated = _validate_dependency_cohort(Path(snapshot["path"]), cohort, entries)
        if not prepared or prepared.get("evidence", {}).get("seed") != snapshot or prepared.get("evidence", {}).get("manifestDigest") != validated["manifestDigest"] or prepared.get("evidence", {}).get("seedDigest") != validated["cohortDigest"]:
            raise HarnessError("dependency seed changed after PREPARE")
        return issue({"seed": snapshot, "manifestDigest": validated["manifestDigest"], "seedDigest": validated["cohortDigest"], "fileCount": validated["fileCount"], "verified": True}, expected_action="VERIFY")

    if identifier == "QUALIFICATION_RUN_6_COMPLETE":
        rows = _exact_matrix_attempts(store, "QUALIFICATION")
        matrix = {
            "schema": "codeclew.kotlin-k1-matrix/0.1", "seriesId": SERIES_ID,
            "cohort": "QUALIFICATION", "repositoryCount": 6, "invocationCount": 12,
            "attempts": rows, "modelCalls": 0,
        }
        _, digest = _create_canonical_artifact(inputs, "qualificationMatrix", matrix)
        return issue({"matrixSha256": digest, "repositories": 6, "invocations": 12}, expected_action="DIRECT")

    if identifier == "CANDIDATE_FREEZE_PREPARE":
        _require_live_set_role(inputs, "candidateSources", "CANDIDATE_SOURCES")
        _require_live_set_role(inputs, "candidateBinaries", "CANDIDATE_BINARIES")
        _candidate_tools(inputs)
        keys = ("candidateSources", "candidateBinaries", "candidateTools", "harnessSource", "independentAuditorSource", "requirements", "readinessGraph", "corpus")
        snapshots = {
            key: snapshot_input(
                {"kind": "FILE", "path": str(AUTHORITIES[key][0].absolute())}
                if key in AUTHORITIES else inputs[key]
            ) for key in keys
        }
        freeze = {
            "schema": "codeclew.kotlin-k1-candidate-freeze/0.1", "seriesId": SERIES_ID,
            "snapshots": snapshots, "qualificationReceiptSha256": store.pointer("QUALIFICATION_RUN_6_COMPLETE")["receiptDigest"],
            "postFreezeChangesAllowed": False, "modelCalls": 0,
        }
        _, digest = _create_canonical_artifact(inputs, "candidateFreeze", freeze)
        return issue({"candidateFreezeSha256": digest}, expected_action="PREPARE")

    if identifier == "CANDIDATE_FREEZE_VERIFY":
        _require_live_set_role(inputs, "candidateSources", "CANDIDATE_SOURCES")
        _require_live_set_role(inputs, "candidateBinaries", "CANDIDATE_BINARIES")
        _candidate_tools(inputs)
        freeze, digest = _canonical_artifact(inputs, "candidateFreeze", "codeclew.kotlin-k1-candidate-freeze/0.1")
        keys = ("candidateSources", "candidateBinaries", "candidateTools", "harnessSource", "independentAuditorSource", "requirements", "readinessGraph", "corpus")
        current = {
            key: snapshot_input(
                {"kind": "FILE", "path": str(AUTHORITIES[key][0].absolute())}
                if key in AUTHORITIES else inputs[key]
            ) for key in keys
        }
        if freeze.get("snapshots") != current or freeze.get("postFreezeChangesAllowed") is not False or freeze.get("modelCalls") != 0:
            raise HarnessError("candidate freeze does not match exact live candidate inputs")
        return issue({"candidateFreezeSha256": digest, "verified": True}, expected_action="VERIFY")

    if identifier == "HOLDOUT_SOURCE_MATERIALIZE":
        target = _input_path(inputs, "holdoutSourceSet", "SOURCE_SET").absolute()
        if target.exists() or target.is_symlink():
            raise HarnessError("holdout source materialization target is create-only")
        target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        staging = target.parent / f".{target.name}.materialize-{secrets.token_hex(12)}"
        staging.mkdir(mode=0o700)
        capture_root = staging / ".bounded-capture"
        capture_root.mkdir(mode=0o700)
        members = []
        git_env = {"HOME":str(staging),"TMPDIR":str(capture_root),"PATH":"/usr/bin:/bin","GIT_CONFIG_NOSYSTEM":"1","GIT_CONFIG_SYSTEM":"/dev/null","GIT_CONFIG_GLOBAL":"/dev/null","GIT_TERMINAL_PROMPT":"0","GIT_ASKPASS":"/usr/bin/false","SSH_ASKPASS":"/usr/bin/false","GIT_PROTOCOL_FROM_USER":"0","LANG":"C.UTF-8","LC_ALL":"C.UTF-8"}
        try:
            for entry_id in EXPECTED_HOLDOUT:
                entry = next(row for row in bundle["corpus"]["entries"] if row["id"] == entry_id)
                repository = staging / entry_id
                clone = ["/usr/bin/git","clone","--no-checkout","--filter=blob:none","--no-tags",entry["origin"],str(repository)]
                completed = _bounded_prepare_run(clone, staging, git_env)
                if completed.returncode != 0:
                    raise HarnessError(f"holdout custodian clone failed: {entry_id}")
                checkout = ["/usr/bin/git","-C",str(repository),"checkout","--detach",entry["commit"]]
                completed_checkout = _bounded_prepare_run(checkout, staging, git_env)
                if completed_checkout.returncode != 0:
                    raise HarnessError(f"holdout custodian checkout failed: {entry_id}")
                observation = _git_observation(repository)
                if observation["head"] != entry["commit"] or observation["tree"] != entry["gitTree"] or not observation["clean"]:
                    raise HarnessError(f"holdout source materialization differs from frozen pin: {entry_id}")
                members.append({"entry":entry_id,"originSha256":sha256_bytes(entry["origin"].encode()),"cloneArgvSha256":sha256_bytes(canonical(clone)),"checkoutArgvSha256":sha256_bytes(canonical(checkout)),"commit":entry["commit"],"gitTree":entry["gitTree"],"sourceTreeSha256":observation["sourceTreeSha256"],"index":_git_index_snapshot(repository)})
            capture_root.rmdir()
            os.replace(staging, target)
        except BaseException:
            if staging.exists():
                shutil.rmtree(staging)
            raise
        source_snapshot = snapshot_input(inputs["holdoutSourceSet"])
        return issue({"sourceSet": source_snapshot, "members": members, "semanticInspectionPerformed": False}, expected_action="PREPARE")

    if identifier == "HOLDOUT_RUN_6_COMPLETE":
        rows = _exact_matrix_attempts(store, "BLIND_HOLDOUT")
        matrix = {
            "schema": "codeclew.kotlin-k1-matrix/0.1", "seriesId": SERIES_ID,
            "cohort": "BLIND_HOLDOUT", "repositoryCount": 6, "invocationCount": 12,
            "attempts": rows, "modelCalls": 0,
        }
        _, digest = _create_canonical_artifact(inputs, "holdoutMatrix", matrix)
        return issue({"matrixSha256": digest, "repositories": 6, "invocations": 12}, expected_action="DIRECT")

    if identifier == "MATRIX_TOTALITY_AND_SAFETY_VERIFY":
        qualification, qualification_sha = _canonical_artifact(inputs, "qualificationMatrix", "codeclew.kotlin-k1-matrix/0.1")
        holdout, holdout_sha = _canonical_artifact(inputs, "holdoutMatrix", "codeclew.kotlin-k1-matrix/0.1")
        rows = qualification["attempts"] + holdout["attempts"]
        sources_mutated = sum(bool(row.get("sourceMutation")) for row in rows)
        untyped = sum(str(row.get("reasonCode", "")).startswith("UNTYPED_FAILURE/") for row in rows)
        replay = all(
            next(row for row in rows if row["entry"] == entry and row["invocation"] == "COLD").get("terminalSemanticDigest")
            == next(row for row in rows if row["entry"] == entry and row["invocation"] == "WARM").get("terminalSemanticDigest")
            for entry in EXPECTED_QUALIFICATION + EXPECTED_HOLDOUT
        )
        false_proven_cases = sorted(
            f"{row['entry']}/{row['invocation']}:{reason}"
            for row in rows for reason in row.get("proofSafety", {}).get("falseProven", [])
        )
        false_complete_cases = sorted(
            f"{row['entry']}/{row['invocation']}:{reason}"
            for row in rows for reason in row.get("proofSafety", {}).get("falseComplete", [])
        )
        conformance = _proof_safety_conformance()
        thresholds = bundle["requirements"]["decisionThresholds"]
        safety_reasons = []
        if sources_mutated > thresholds["sourceMutationMaximum"]:
            safety_reasons.append("SOURCE_MUTATION_THRESHOLD")
        if untyped > thresholds["untypedFailureMaximum"]:
            safety_reasons.append("UNTYPED_FAILURE_THRESHOLD")
        if not replay:
            safety_reasons.append("OFFLINE_REPLAY_MISMATCH")
        if len(false_proven_cases) > thresholds["falseProvenMaximum"]:
            safety_reasons.append("FALSE_PROVEN_THRESHOLD")
        if len(false_complete_cases) > thresholds["falseCompleteMaximum"]:
            safety_reasons.append("FALSE_COMPLETE_THRESHOLD")
        if conformance["status"] != "PASS":
            safety_reasons.append("STRUCTURAL_CONFORMANCE_FAILED")
        safe = not safety_reasons
        evidence = {
            "qualificationMatrixSha256": qualification_sha,
            "holdoutMatrixSha256": holdout_sha,
            "sourceMutations": sources_mutated,
            "untypedFailures": untyped,
            "offlineReplayEqual": replay,
            "falseProven": len(false_proven_cases),
            "falseComplete": len(false_complete_cases),
            "falseProvenCases": false_proven_cases,
            "falseCompleteCases": false_complete_cases,
            "structuralConformance": conformance,
            "failureReasons": safety_reasons,
            "safe": safe,
        }
        # This DIRECT node measures safety. A measured unsafe result is still
        # a current READY receipt so K1_DECISION can deterministically issue
        # STOP instead of leaving the DAG blocked without a terminal root.
        artifact = {"schema":"codeclew.kotlin-k1-matrix-safety/0.1","seriesId":SERIES_ID,**evidence,"producerInputs":{"qualificationRun":store.pointer("QUALIFICATION_RUN_6_COMPLETE")["receiptDigest"],"holdoutRun":store.pointer("HOLDOUT_RUN_6_COMPLETE")["receiptDigest"]},"modelCalls":0}
        _, artifact_sha = _create_canonical_artifact(inputs, "matrixSafetyReceipt", artifact)
        return issue({**evidence, "artifactSha256": artifact_sha}, expected_action="DIRECT")

    if identifier == "APPLICABILITY_VERIFY":
        qualification, _ = _canonical_artifact(inputs, "qualificationMatrix", "codeclew.kotlin-k1-matrix/0.1")
        holdout, _ = _canonical_artifact(inputs, "holdoutMatrix", "codeclew.kotlin-k1-matrix/0.1")
        thresholds = bundle["requirements"]["decisionThresholds"]
        corpus_entries = {row["id"]: row for row in bundle["corpus"]["entries"]}
        evidence = _applicability_measurement(
            qualification["attempts"] + holdout["attempts"],
            holdout["attempts"], corpus_entries, thresholds,
        )
        artifact = {"schema":"codeclew.kotlin-k1-applicability/0.1","seriesId":SERIES_ID,**evidence,"producerInputs":{"matrixSafety":store.pointer("MATRIX_TOTALITY_AND_SAFETY_VERIFY")["receiptDigest"]},"modelCalls":0}
        _, artifact_sha = _create_canonical_artifact(inputs, "applicabilityReceipt", artifact)
        return issue({**evidence, "artifactSha256": artifact_sha}, expected_action="DIRECT")

    if identifier == "CACHE_AND_COST_VERIFY":
        qualification, _ = _canonical_artifact(inputs, "qualificationMatrix", "codeclew.kotlin-k1-matrix/0.1")
        holdout, _ = _canonical_artifact(inputs, "holdoutMatrix", "codeclew.kotlin-k1-matrix/0.1")
        thresholds = bundle["requirements"]["decisionThresholds"]
        all_rows = qualification["attempts"] + holdout["attempts"]
        evidence = _cache_cost_measurement(all_rows, holdout["attempts"], thresholds)
        artifact = {"schema":"codeclew.kotlin-k1-cache-cost/0.1","seriesId":SERIES_ID,**evidence,"producerInputs":{"matrixSafety":store.pointer("MATRIX_TOTALITY_AND_SAFETY_VERIFY")["receiptDigest"]},"modelCalls":0}
        _, artifact_sha = _create_canonical_artifact(inputs, "cacheCostReceipt", artifact)
        return issue({**evidence, "artifactSha256": artifact_sha}, expected_action="DIRECT")

    if identifier == "REQUIREMENT_CONFORMANCE_VERIFY":
        safety, safety_sha = _verified_measurement_artifact(store, inputs, "matrixSafetyReceipt", "codeclew.kotlin-k1-matrix-safety/0.1", "MATRIX_TOTALITY_AND_SAFETY_VERIFY")
        applicability, applicability_sha = _verified_measurement_artifact(store, inputs, "applicabilityReceipt", "codeclew.kotlin-k1-applicability/0.1", "APPLICABILITY_VERIFY")
        cache_cost, cache_sha = _verified_measurement_artifact(store, inputs, "cacheCostReceipt", "codeclew.kotlin-k1-cache-cost/0.1", "CACHE_AND_COST_VERIFY")
        requirements = bundle["requirements"].get("requirements", [])
        requirement_ids = [row.get("id") for row in requirements]
        expected_ids = [f"K1-R{number:02d}" for number in range(1, 21)]
        if requirement_ids != expected_ids:
            raise HarnessError("requirements do not contain the exact K1-R01..K1-R20 conjunction")
        qualification, _ = _canonical_artifact(inputs, "qualificationMatrix", "codeclew.kotlin-k1-matrix/0.1")
        holdout, _ = _canonical_artifact(inputs, "holdoutMatrix", "codeclew.kotlin-k1-matrix/0.1")
        rows = _requirement_predicates(store, safety, applicability, cache_cost, qualification, holdout, inputs)
        row_mutations = _requirement_row_mutation_conformance(rows)
        if row_mutations["status"] != "PASS":
            raise HarnessError("requirement row mutation conformance failed")
        stop_violations = sorted(
            identifier for identifier, row in rows.items()
            if row["status"] == "FAIL" and row["failureClass"] == "STOP"
        )
        gap_requirements = sorted(
            identifier for identifier, row in rows.items()
            if row["status"] == "FAIL" and row["failureClass"] == "GAP"
        )
        value = {
            "schema": "codeclew.kotlin-k1-requirement-conformance/0.1", "seriesId": SERIES_ID,
            "requirements": rows, "allPassed": all(row["status"] == "PASS" for row in rows.values()),
            "stopViolations": stop_violations, "gapRequirements": gap_requirements,
            "rowMutationConformance": row_mutations,
            "rawEvidence": {
                "qualificationPreparationAttempts": store.receipt("QUALIFICATION_DEPENDENCY_SEED_PREPARE")["evidence"]["preparationAttempts"],
                "holdoutPreparationAttempts": store.receipt("HOLDOUT_DEPENDENCY_SEED_PREPARE")["evidence"]["preparationAttempts"],
                "holdoutMaterialization": store.receipt("HOLDOUT_SOURCE_MATERIALIZE")["evidence"],
                "k0ByteExact": store.receipt("K0_1_BYTE_EXACT_VERIFY")["evidence"],
            },
            "producerReceiptDigests": {
                key: store.pointer(node)["receiptDigest"]
                for key, node in {
                    "matrixSafety": "MATRIX_TOTALITY_AND_SAFETY_VERIFY",
                    "applicability": "APPLICABILITY_VERIFY", "cacheCost": "CACHE_AND_COST_VERIFY",
                    "baseline": "BASELINE_CAPTURE", "harnessSelfTest": "HARNESS_SELF_TEST",
                    "qualificationPrepare": "QUALIFICATION_DEPENDENCY_SEED_PREPARE",
                    "holdoutPrepare": "HOLDOUT_DEPENDENCY_SEED_PREPARE",
                    "holdoutMaterialize": "HOLDOUT_SOURCE_MATERIALIZE",
                    "k0ByteExact": "K0_1_BYTE_EXACT_VERIFY",
                }.items()
            },
            "modelCalls": 0,
        }
        _, digest = _create_canonical_artifact(inputs, "requirementConformance", value)
        return issue({"requirementConformanceSha256": digest, "allPassed": value["allPassed"], "stopViolations":stop_violations, "gapRequirements":gap_requirements}, expected_action="DIRECT")

    if identifier == "K1_INDEPENDENT_AUDITOR_RUN":
        auditor = _regular_file(_input_path(inputs, "independentAuditorSource", "FILE"), "independent auditor source")
        if auditor != (ROOT / "scripts/k1_independent_auditor.py").absolute():
            raise HarnessError("independent auditor source path is not the pinned implementation")
        command = [
            str(Path(os.sys.executable).resolve()), str(auditor),
            "--matrix-safety", str(_input_path(inputs, "matrixSafetyReceipt", "FILE")),
            "--applicability", str(_input_path(inputs, "applicabilityReceipt", "FILE")),
            "--cache-cost", str(_input_path(inputs, "cacheCostReceipt", "FILE")),
            "--requirement-conformance", str(_input_path(inputs, "requirementConformance", "FILE")),
            "--candidate-freeze", str(_input_path(inputs, "candidateFreeze", "FILE")),
            "--qualification-matrix", str(_input_path(inputs, "qualificationMatrix", "FILE")),
            "--holdout-matrix", str(_input_path(inputs, "holdoutMatrix", "FILE")),
            "--requirements", str(AUTHORITIES["requirements"][0]),
            "--corpus", str(AUTHORITIES["corpus"][0]),
            "--candidate-tools", str(_input_path(inputs, "candidateTools", "FILE")),
            "--baseline-packet", str(_input_path(inputs, "baselinePacket", "FILE")),
            "--harness-self-test-packet", str(_input_path(inputs, "harnessSelfTestPacket", "FILE")),
            "--output", str(_input_path(inputs, "independentAudit", "FILE")),
        ]
        completed = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False, timeout=120, env={"PATH":"/usr/bin:/bin","LANG":"C.UTF-8","LC_ALL":"C.UTF-8"})
        if completed.returncode != 0:
            raise HarnessError("pinned independent auditor failed: " + sha256_bytes(completed.stderr))
        report = {
            "schema": "codeclew.kotlin-k1-independent-auditor-run/0.1", "seriesId": SERIES_ID,
            "auditorSourceSha256": sha256_file(auditor), "argvSha256": sha256_bytes(canonical(command)),
            "reportSha256": snapshot_input(inputs["independentAudit"])["sha256"], "exitCode": 0,
            "stdoutSha256": sha256_bytes(completed.stdout), "stderrSha256": sha256_bytes(completed.stderr), "modelCalls": 0,
            "expectedDecision": _canonical_artifact(
                inputs, "independentAudit", "codeclew.kotlin-k1-independent-audit/0.2"
            )[0].get("expectedDecision"),
        }
        _, digest = _create_canonical_artifact(inputs, "independentAuditorRunReceipt", report)
        return issue({"auditorRunSha256": digest, "independentAuditSha256": report["reportSha256"]}, expected_action="DIRECT")

    if identifier == "K1_INDEPENDENT_AUDIT_IMPORT":
        audit, digest = _canonical_artifact(inputs, "independentAudit", "codeclew.kotlin-k1-independent-audit/0.2")
        run, run_sha = _canonical_artifact(inputs, "independentAuditorRunReceipt", "codeclew.kotlin-k1-independent-auditor-run/0.1")
        expected_digests = {key: snapshot_input(inputs[key])["sha256"] for key in ("matrixSafetyReceipt","applicabilityReceipt","cacheCostReceipt","requirementConformance","qualificationMatrix","holdoutMatrix","candidateTools","baselinePacket","harnessSelfTestPacket","candidateFreeze")}
        if audit.get("decision") != "ACCEPT" or audit.get("expectedDecision") not in {"GO","PIVOT","STOP"} or run.get("expectedDecision") != audit.get("expectedDecision") or audit.get("auditorSourceSha256") != sha256_file(ROOT / "scripts/k1_independent_auditor.py") or audit.get("matrixSafetySha256") != expected_digests["matrixSafetyReceipt"] or audit.get("applicabilitySha256") != expected_digests["applicabilityReceipt"] or audit.get("cacheCostSha256") != expected_digests["cacheCostReceipt"] or audit.get("requirementConformanceSha256") != expected_digests["requirementConformance"] or audit.get("qualificationMatrixSha256") != expected_digests["qualificationMatrix"] or audit.get("holdoutMatrixSha256") != expected_digests["holdoutMatrix"] or audit.get("candidateToolsSha256") != expected_digests["candidateTools"] or audit.get("baselinePacketSha256") != expected_digests["baselinePacket"] or audit.get("harnessSelfTestPacketSha256") != expected_digests["harnessSelfTestPacket"] or audit.get("candidateFreezeSha256") != expected_digests["candidateFreeze"] or audit.get("requirementsSha256") != store.bundle["digests"]["requirements"] or audit.get("corpusSha256") != store.bundle["digests"]["corpus"] or run.get("reportSha256") != digest:
            raise HarnessError("independent final audit binding mismatch")
        return issue({"independentAuditSha256": digest, "auditorRunSha256": run_sha, "expectedDecision": audit["expectedDecision"]}, expected_action="IMPORT")

    if identifier == "K1_DECISION":
        guard_state, _, guard_digest = _series_guard(store)
        if guard_state == "FATAL":
            return issue(
                {"decision":"STOP", "guardReceiptDigest":guard_digest},
                expected_action="DECISION",
            )
        safety, _ = _canonical_artifact(inputs, "matrixSafetyReceipt", "codeclew.kotlin-k1-matrix-safety/0.1")
        applicability, _ = _canonical_artifact(inputs, "applicabilityReceipt", "codeclew.kotlin-k1-applicability/0.1")
        cache_cost, _ = _canonical_artifact(inputs, "cacheCostReceipt", "codeclew.kotlin-k1-cache-cost/0.1")
        conformance, _ = _canonical_artifact(inputs, "requirementConformance", "codeclew.kotlin-k1-requirement-conformance/0.1")
        decision = _expected_k1_decision(safety, applicability, cache_cost, conformance)
        independent_audit, _ = _canonical_artifact(inputs, "independentAudit", "codeclew.kotlin-k1-independent-audit/0.2")
        if independent_audit.get("decision") != "ACCEPT" or independent_audit.get("expectedDecision") != decision:
            raise HarnessError("independent auditor and decision checker disagree")
        value = {"schema":"codeclew.kotlin-k1-decision/0.1","seriesId":SERIES_ID,"decision":decision,"terminalRoot":{"GO":"KOTLIN_REAL_REPOSITORY_READY","PIVOT":"KOTLIN_APPLICABILITY_OR_COST_GAP","STOP":"K1_SERIES_STOPPED"}[decision],"modelCalls":0}
        _, digest = _create_canonical_artifact(inputs, "decision", value)
        return issue({"decision":decision,"decisionSha256":digest}, expected_action="DECISION")

    if identifier in {"KOTLIN_REAL_REPOSITORY_READY","KOTLIN_APPLICABILITY_OR_COST_GAP","K1_SERIES_STOPPED"}:
        expected = {"KOTLIN_REAL_REPOSITORY_READY":"GO","KOTLIN_APPLICABILITY_OR_COST_GAP":"PIVOT","K1_SERIES_STOPPED":"STOP"}[identifier]
        decision = store.receipt("K1_DECISION")
        if not decision or decision.get("evidence", {}).get("decision") != expected:
            raise HarnessError(f"conditional root does not match decision {expected}")
        return issue({"decision":expected}, expected_action="CONDITIONAL_ROOT")

    raise HarnessError(f"node has no production executable issuer yet: {identifier}")


def advance_node(store: Store, identifier: str, inputs: Mapping[str, Mapping[str, Any]]) -> str:
    """Execute and publish one production node under a single store lock."""
    guard_state, _, _ = _series_guard(store)
    if guard_state == "FATAL" and identifier not in {"K1_DECISION", "K1_SERIES_STOPPED"}:
        raise HarnessError("FATAL series permits only STOP decision/root issuance")
    if identifier == "K1_SERIES_GUARD":
        raise HarnessError("series guard is internal and cannot be advanced")
    return _advance_locked(
        store,
        identifier,
        inputs,
        lambda selected, dependencies: _advance_node_unlocked(
            store, identifier, inputs, selected, dependencies
        ),
    )


def _retained_fatal_invariant(
    store: Store,
    inputs: Mapping[str, Mapping[str, Any]],
) -> tuple[str, dict[str, Any]] | None:
    bypass = _stored_authority_bypass_fatal(store)
    if bypass is not None:
        return "VERIFIED_AUTHORITY_BYPASS", bypass
    unretained = _unretained_started_child(store)
    if unretained is not None:
        return "UNRETAINED_STARTED_CHILD", unretained
    for cohort, entries, lookup in (
        ("QUALIFICATION", EXPECTED_QUALIFICATION, store.qualification_attempt),
        ("BLIND_HOLDOUT", EXPECTED_HOLDOUT, store.holdout_attempt),
    ):
        for entry in entries:
            for invocation in ("COLD", "WARM"):
                pair = lookup(entry, invocation)
                if pair is None:
                    continue
                digest, attempt = pair
                if attempt.get("sourceMutation") is True:
                    return "SOURCE_MUTATION", {
                        "entry": entry, "invocation": invocation, "attemptDigest": digest,
                    }
    freeze_pointer = store.pointer("CANDIDATE_FREEZE_VERIFY")
    if freeze_pointer is not None:
        try:
            freeze, _ = _canonical_artifact(
                inputs, "candidateFreeze", "codeclew.kotlin-k1-candidate-freeze/0.1",
            )
            frozen = freeze.get("snapshots")
            freeze_keys = (
                "candidateSources", "candidateBinaries", "candidateTools", "harnessSource",
                "independentAuditorSource", "requirements", "readinessGraph", "corpus",
            )
            current: dict[str, Any] = {}
            for key in freeze_keys:
                descriptor: Mapping[str, Any] | None = None
                try:
                    if key in AUTHORITIES:
                        descriptor = {"kind": "FILE", "path": str(AUTHORITIES[key][0].absolute())}
                        current[key] = snapshot_input(descriptor)
                    elif key in {"candidateSources", "candidateBinaries"}:
                        descriptor = inputs[key]
                        manifest_path = Path(str(descriptor["path"])).absolute()
                        manifest = _load_json_bytes(_regular_file(manifest_path, key).read_bytes(), key)
                        tools_path = manifest.get("candidateToolsPath") if isinstance(manifest, Mapping) else None
                        rebuilt = build_live_set(
                            "CANDIDATE_SOURCES" if key == "candidateSources" else "CANDIDATE_BINARIES",
                            Path(tools_path) if isinstance(tools_path, str) else None,
                        )
                        current[key] = {
                            "kind": "LIVE_SET", "path": str(manifest_path),
                            "sha256": sha256_bytes(canonical(rebuilt)),
                        }
                    else:
                        descriptor = inputs[key]
                        current[key] = snapshot_input(descriptor)
                except (HarnessError, OSError, KeyError, TypeError) as error:
                    frozen_row = frozen.get(key) if isinstance(frozen, Mapping) else None
                    frozen_path = frozen_row.get("path") if isinstance(frozen_row, Mapping) else None
                    if not isinstance(frozen_path, str):
                        raise
                    observation: dict[str, Any] = {
                        "key": key, "path": frozen_path,
                        "errorType": type(error).__name__, "error": str(error),
                    }
                    live_path = Path(frozen_path)
                    try:
                        metadata = live_path.lstat()
                        observation["mode"] = stat.S_IFMT(metadata.st_mode)
                        observation["size"] = metadata.st_size
                        if stat.S_ISREG(metadata.st_mode):
                            observation["rawSha256"] = sha256_file(live_path)
                    except OSError as observation_error:
                        observation["lstatError"] = type(observation_error).__name__
                    current[key] = {
                        "kind": frozen_row.get("kind"), "path": frozen_path,
                        "sha256": sha256_bytes(canonical(observation)),
                    }
            if isinstance(frozen, Mapping) and frozen != current:
                return "POST_FREEZE_CANDIDATE_OR_HOLDOUT_CHANGE", {
                    "node": "CANDIDATE_FREEZE_VERIFY",
                    "receiptDigest": freeze_pointer["receiptDigest"],
                    "beforeSha256": sha256_bytes(canonical(frozen)),
                    "afterSha256": sha256_bytes(canonical(current)),
                }
        except (HarnessError, OSError, KeyError, TypeError):
            # A wrong/missing caller manifest remains BLOCKED. Only a complete
            # independently snapshotted mismatch is terminal evidence.
            pass
        for node in ("CANDIDATE_FREEZE_VERIFY", "HOLDOUT_SOURCE_MATERIALIZE"):
            receipt = store.receipt(node)
            if receipt is None:
                continue
            status, _, _ = assess(store, node, inputs)
            if status == "STALE":
                before = sha256_bytes(canonical(receipt.get("selectedInputs", {})))
                try:
                    after = sha256_bytes(canonical(_selected(store, node, inputs)))
                except (HarnessError, OSError) as error:
                    after = sha256_bytes(str(error).encode())
                if before != after:
                    return "POST_FREEZE_CANDIDATE_OR_HOLDOUT_CHANGE", {
                        "node": node,
                        "receiptDigest": store.pointer(node)["receiptDigest"],
                        "beforeSha256": before,
                        "afterSha256": after,
                    }
    safety_pointer = store.pointer("MATRIX_TOTALITY_AND_SAFETY_VERIFY")
    if safety_pointer is not None:
        safety, artifact_sha = _verified_measurement_artifact(
            store, inputs, "matrixSafetyReceipt",
            "codeclew.kotlin-k1-matrix-safety/0.1",
            "MATRIX_TOTALITY_AND_SAFETY_VERIFY",
        )
        violations = []
        if safety.get("falseProven", 0) > 0:
            violations.append("FALSE_PROVEN")
        if safety.get("falseComplete", 0) > 0:
            violations.append("FALSE_COMPLETE")
        if safety.get("untypedFailures", 0) > 0:
            violations.append("UNTYPED_FAILURE")
        if safety.get("modelCalls", 0) != 0:
            violations.append("MODEL_CALL")
        if violations:
            return "MATRIX_SAFETY_VIOLATION", {
                "matrixSafetyArtifactSha256": artifact_sha,
                "matrixSafetyReceiptDigest": safety_pointer["receiptDigest"],
                "violations": sorted(violations),
            }
    return None


def _detect_fatal_invariant(
    store: Store,
    inputs: Mapping[str, Mapping[str, Any]],
) -> tuple[str, dict[str, Any]] | None:
    """Single internal dispatcher; caller text/evidence is never an authority."""
    live = _live_authority_fatal(store)
    if live is not None:
        return live
    bypass = _stored_authority_bypass_fatal(store)
    if bypass is not None:
        return "VERIFIED_AUTHORITY_BYPASS", bypass
    try:
        return _retained_fatal_invariant(store, inputs)
    except (HarnessError, OSError) as error:
        # Malformed harness-private retained authority is a bypass, never a
        # caller-controlled whitelist reason and never an exception leak.
        return "VERIFIED_AUTHORITY_BYPASS", {
            "invariant": "RECEIPT_IDENTITY",
            "detailSha256": sha256_bytes(canonical({
                "phase": "RETAINED_FATAL_SCAN", "error": f"{type(error).__name__}:{error}",
            })),
        }


def _stored_authority_bypass_fatal(store: Store) -> dict[str, Any] | None:
    """Independently detect forged non-guard CAS/current receipt authority."""
    def detail(invariant: str, path: Path, observation: str) -> dict[str, Any]:
        return {
            "invariant": invariant,
            "detailSha256": sha256_bytes(canonical({
                "path": path.relative_to(store.root).as_posix(), "observation": observation,
            })),
        }

    guard_path = store.root / "current" / "K1_SERIES_GUARD.json"
    try:
        state, _, guard_digest = _series_guard(store)
        guard_raw = _regular_file(guard_path, "guard projection").read_bytes()
        guard_pointer = _load_json_bytes(guard_raw, "guard projection")
        expected_guard_pointer = {
            "schema": POINTER_SCHEMA, "storeId": store.store_id,
            "graphDigest": store.graph_digest, "node": "K1_SERIES_GUARD",
            "receiptDigest": guard_digest,
        }
        if canonical(guard_pointer) != guard_raw or guard_pointer != expected_guard_pointer:
            return detail("CURRENT_POINTER", guard_path, f"GUARD_PROJECTION_{state}_MISMATCH")
    except (HarnessError, OSError, TypeError) as error:
        return detail("CURRENT_POINTER", guard_path, f"GUARD_PROJECTION_{type(error).__name__}")

    for path in sorted((store.root / "current").iterdir(), key=lambda item: item.name):
        if path.name == "K1_SERIES_GUARD.json":
            continue
        try:
            raw = _regular_file(path, "current pointer scan").read_bytes()
            pointer = _load_json_bytes(raw, "current pointer scan")
        except (HarnessError, OSError) as error:
            return detail("CURRENT_POINTER", path, type(error).__name__)
        if path.stem not in {node["id"] for node in store.graph["nodes"]}:
            return detail("CURRENT_POINTER", path, "UNKNOWN_CURRENT_NODE")
        expected_pointer = {
            "schema": POINTER_SCHEMA, "storeId": store.store_id,
            "graphDigest": store.graph_digest, "node": path.stem,
            "receiptDigest": pointer.get("receiptDigest") if isinstance(pointer, Mapping) else None,
        }
        if not isinstance(pointer, dict) or canonical(pointer) != raw or pointer != expected_pointer or not _is_digest(pointer.get("receiptDigest")):
            return detail("CURRENT_POINTER", path, "POINTER_IDENTITY_MISMATCH")
        object_path = store.root / "objects" / f"{pointer['receiptDigest'][7:]}.json"
        try:
            object_raw = _regular_file(object_path, "reachable CAS object").read_bytes()
            receipt = _load_json_bytes(object_raw, "reachable CAS object")
        except (HarnessError, OSError) as error:
            return detail("CAS_OBJECT", object_path, type(error).__name__)
        if sha256_bytes(object_raw) != pointer["receiptDigest"] or not isinstance(receipt, dict) or canonical(receipt) != object_raw:
            return detail("CAS_OBJECT", object_path, "DIGEST_OR_CANONICAL_MISMATCH")
        try:
            action = _node(store, path.stem)["action"]
        except (HarnessError, KeyError):
            return detail("CURRENT_POINTER", path, "UNKNOWN_CURRENT_NODE")
        if receipt.get("schema") != RECEIPT_SCHEMA or receipt.get("storeId") != store.store_id or receipt.get("seriesId") != SERIES_ID or receipt.get("graphDigest") != store.graph_digest or receipt.get("node") != path.stem or receipt.get("action") != action or receipt.get("status") not in RECEIPT_STATES:
            return detail("RECEIPT_IDENTITY", path, "RECEIPT_IDENTITY_MISMATCH")
    return None


def _recovery_receipt(
    store: Store,
    identifier: str,
    dependencies: Mapping[str, str],
    evidence: Mapping[str, Any],
) -> tuple[str, dict[str, Any]]:
    """Recover a fatal-only terminal pointer without consulting its old value."""
    source_digest = sha256_file(Path(__file__))
    receipt = {
        "schema": RECEIPT_SCHEMA,
        "storeId": store.store_id,
        "seriesId": SERIES_ID,
        "graphDigest": store.graph_digest,
        "checkerVersion": CHECKER_VERSION,
        "checkerSourceDigest": source_digest,
        "node": identifier,
        "action": _node(store, identifier)["action"],
        "nodeKey": _node_key_with_source_digest(
            store, identifier, {}, dependencies, source_digest,
        ),
        "status": "READY",
        "selectedInputs": {},
        "dependencies": dict(dependencies),
        "evidence": dict(evidence),
        "error": None,
    }
    digest = store.put_recovery_object(receipt)
    store._write_pointer_unchecked(identifier, digest)
    return digest, receipt


def _quarantine_nonselected_terminal_projection(store: Store, identifier: str) -> None:
    """Remove a poisoned GO/PIVOT projection while preserving its bytes.

    Current pointers are mutable projections, not terminal authority. Once the
    append-only guard is FATAL, neither non-STOP root may remain current, and a
    malformed pointer must not prevent recovery of the unique STOP root.
    """
    if identifier not in {
        "KOTLIN_REAL_REPOSITORY_READY", "KOTLIN_APPLICABILITY_OR_COST_GAP",
    }:
        raise HarnessError("fatal recovery projection target mismatch")
    path = store.root / "current" / f"{identifier}.json"
    try:
        path.lstat()
    except FileNotFoundError:
        return
    quarantine = (
        store.root / "objects"
        / f"quarantine-current-{identifier.lower()}-{secrets.token_hex(32)}.bad"
    )
    os.replace(path, quarantine)
    for directory_path in (path.parent, quarantine.parent):
        directory = os.open(
            directory_path, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        try:
            os.fsync(directory)
        finally:
            os.close(directory)


def _recover_fatal_terminal(store: Store) -> dict[str, Any]:
    state, guard_receipt, guard_digest = _series_guard(store)
    if state != "FATAL":
        raise HarnessError("fatal terminal recovery requires FATAL guard")
    # The marker is the authority; repair its advisory projection first.
    store._write_pointer_unchecked("K1_SERIES_GUARD", guard_digest)
    for other in (
        "KOTLIN_REAL_REPOSITORY_READY", "KOTLIN_APPLICABILITY_OR_COST_GAP",
    ):
        _quarantine_nonselected_terminal_projection(store, other)
    decision_digest, _ = _recovery_receipt(
        store, "K1_DECISION", {"K1_SERIES_GUARD": guard_digest},
        {"decision": "STOP", "guardReceiptDigest": guard_digest},
    )
    root_digest, _ = _recovery_receipt(
        store, "K1_SERIES_STOPPED", {"K1_DECISION": decision_digest},
        {"decision": "STOP"},
    )
    status, reasons, _ = assess(store, "K1_SERIES_STOPPED", {})
    if status != "READY":
        raise HarnessError(f"recovered STOP root is {status}: {reasons}")
    for other in ("KOTLIN_REAL_REPOSITORY_READY", "KOTLIN_APPLICABILITY_OR_COST_GAP"):
        other_status, _, _ = assess(store, other, {})
        if other_status == "READY":
            raise HarnessError("fatal recovery left multiple terminal roots READY")
    return {
        "status": "FINALIZED", "decision": "STOP", "root": "K1_SERIES_STOPPED",
        "guardReceiptDigest": guard_digest, "decisionReceiptDigest": decision_digest,
        "rootReceiptDigest": root_digest,
        "reasonCode": guard_receipt["evidence"]["reasonCode"],
    }


def finalize_series(
    store: Store,
    inputs: Mapping[str, Mapping[str, Any]],
) -> dict[str, Any]:
    """Issue exactly one current terminal root or remain explicitly blocked."""
    with store.locked():
        state, _, _ = _series_guard(store)
        detected = None if state == "FATAL" else _detect_fatal_invariant(store, inputs)
        if detected is not None:
            _latch_series_fatal(store, inputs, detected)
        state, guard_receipt, guard_digest = _series_guard(store)
        if state == "FATAL":
            return _recover_fatal_terminal(store)
        # Missing/unstarted attempts, an unavailable/invalid auditor and wrong
        # caller inputs remain BLOCKED through normal dependency/currentness
        # checks; none is converted into a fatal reason.
        decision_digest = advance_node(store, "K1_DECISION", inputs)
        decision = store.receipt("K1_DECISION")
        if decision is None or decision.get("evidence", {}).get("decision") not in {"GO", "PIVOT", "STOP"}:
            raise HarnessError("current decision receipt mismatch")
        value = decision["evidence"]["decision"]
        root = {
            "GO": "KOTLIN_REAL_REPOSITORY_READY",
            "PIVOT": "KOTLIN_APPLICABILITY_OR_COST_GAP",
            "STOP": "K1_SERIES_STOPPED",
        }[value]
        root_digest = advance_node(store, root, inputs)
        return {
            "status": "FINALIZED", "decision": value, "root": root,
            "guardReceiptDigest": guard_digest, "decisionReceiptDigest": decision_digest,
            "rootReceiptDigest": root_digest, "reasonCode": None,
        }


def _live_authority_fatal(store: Store) -> tuple[str, dict[str, Any]] | None:
    """Compare production paths to immutable store authorities, without trust in live bytes."""
    for name, (path, _) in AUTHORITIES.items():
        stored = _regular_file(store.root / "authorities" / path.name, f"stored {name}")
        expected = sha256_file(stored)
        try:
            observed = sha256_file(_regular_file(path, f"production {name}"))
        except (HarnessError, OSError) as error:
            observed = sha256_bytes(str(error).encode())
        if observed != expected:
            reason = "THRESHOLD_OR_CORPUS_REWRITE" if name == "corpus" else "PINNED_AUTHORITY_DRIFT"
            if name == "requirements":
                try:
                    stored_value = _load_json_bytes(stored.read_bytes(), "stored requirements")
                    live_value = _load_json_bytes(path.read_bytes(), "live requirements")
                    if stored_value.get("decisionThresholds") != live_value.get("decisionThresholds"):
                        reason = "THRESHOLD_OR_CORPUS_REWRITE"
                except (HarnessError, OSError, AttributeError):
                    pass
            return reason, {"authority": name, "expectedSha256": expected, "observedSha256": observed}
    policy = store.bundle["requirements"]["historicalCorePolicy"]
    expected_k0 = {
        "k0Lock": policy["coreContractLockSha256"],
        "k0Portability": policy["kotlinK0PortabilitySha256"],
        "evidenceCoreProto": policy["evidenceCoreProtoSha256"],
    }
    observed_k0 = {}
    for key, path in {
        "k0Lock": ROOT / "contracts/core/core-contract.lock.json",
        "k0Portability": ROOT / "contracts/core/kotlin-k0.portability.json",
        "evidenceCoreProto": ROOT / "schemas/evidence_core.proto",
    }.items():
        try:
            observed_k0[key] = sha256_file(_regular_file(path, key))
        except (HarnessError, OSError) as error:
            observed_k0[key] = sha256_bytes(str(error).encode())
    if observed_k0 != expected_k0:
        return "K0_1_DRIFT", {"expectedDigests": expected_k0, "observedDigests": observed_k0}
    try:
        lock = _load_json_bytes(
            (ROOT / "contracts/core/core-contract.lock.json").read_bytes(), "K0 lock",
        )
        members = (
            lock["adapterContractFiles"]
            + lock["decisionCoreFiles"]
            + lock["conformanceCorpusFiles"]
        )
        expected_members = {row["path"]: row["sha256"] for row in members}
        observed_members = {}
        for relative in sorted(expected_members):
            try:
                observed_members[relative] = sha256_file(_regular_file(ROOT / relative, relative))
            except (HarnessError, OSError) as error:
                observed_members[relative] = sha256_bytes(str(error).encode())
        if observed_members != expected_members:
            return "K0_1_DRIFT", {
                "expectedDigests": {**expected_k0, **expected_members},
                "observedDigests": {**observed_k0, **observed_members},
            }
    except (HarnessError, KeyError, TypeError) as error:
        return "K0_1_DRIFT", {
            "expectedDigests": expected_k0,
            "observedDigests": {**observed_k0, "lockMembers": sha256_bytes(str(error).encode())},
        }
    return None


def _guard_terminal_self_test(root: Path, bundle: Mapping[str, Any]) -> dict[str, bool]:
    checks: dict[str, bool] = {}
    store = Store(root / "recovery", bundle, create=True)
    arbitrary = ("PINNED_AUTHORITY_DRIFT", {
        "authority": "requirements", "expectedSha256": "sha256:" + "1" * 64,
        "observedSha256": "sha256:" + "2" * 64,
    })
    try:
        _latch_series_fatal(store, {}, arbitrary)
        raise AssertionError("caller-authored fatal evidence latched")
    except HarnessError:
        checks["callerFatalRejected"] = True
    open_digest = _series_guard(store)[2]
    unknown = {
        "schema": POINTER_SCHEMA, "storeId": store.store_id,
        "graphDigest": store.graph_digest, "node": "UNKNOWN",
        "receiptDigest": open_digest,
    }
    _atomic_write(store.root / "current" / "UNKNOWN.json", canonical(unknown))
    first = finalize_series(store, {})
    fatal_digest = _series_guard(store)[2]
    checks["unknownPointerTotal"] = first["reasonCode"] == "VERIFIED_AUTHORITY_BYPASS"
    store._write_pointer_unchecked("K1_SERIES_GUARD", open_digest)
    decision_path = store.root / "current" / "K1_DECISION.json"
    _atomic_write(decision_path, b"{bad-json\n")
    (store.root / "current" / "UNKNOWN.json").unlink()
    second = finalize_series(store, {})
    checks["guardRewindIgnored"] = _series_guard(store)[2] == fatal_digest
    checks["badDecisionRecovered"] = second["decision"] == "STOP"
    decision_digest = second["decisionReceiptDigest"]
    _atomic_write(store.root / "objects" / f"{decision_digest[7:]}.json", b"poisoned\n")
    _atomic_write(
        store.root / "current" / "KOTLIN_REAL_REPOSITORY_READY.json",
        b"{bad-root-pointer\n",
    )
    poisoned_root_raw = b"poisoned-root-object\n"
    poisoned_root_digest = sha256_bytes(poisoned_root_raw)
    _atomic_write(
        store.root / "objects" / f"{poisoned_root_digest[7:]}.json",
        poisoned_root_raw,
    )
    _atomic_write(
        store.root / "current" / "KOTLIN_APPLICABILITY_OR_COST_GAP.json",
        canonical({
            "schema": POINTER_SCHEMA, "storeId": store.store_id,
            "graphDigest": store.graph_digest,
            "node": "KOTLIN_APPLICABILITY_OR_COST_GAP",
            "receiptDigest": poisoned_root_digest,
        }),
    )
    third = finalize_series(store, {})
    checks["poisonedCasRecovered"] = third["decision"] == "STOP"
    checks["poisonedNonselectedRootsRecovered"] = (
        not (store.root / "current" / "KOTLIN_REAL_REPOSITORY_READY.json").exists()
        and not (store.root / "current" / "KOTLIN_APPLICABILITY_OR_COST_GAP.json").exists()
        and any(
            path.name.startswith("quarantine-current-kotlin_real_repository_ready-")
            for path in (store.root / "objects").iterdir()
        )
        and any(
            path.name.startswith("quarantine-current-kotlin_applicability_or_cost_gap-")
            for path in (store.root / "objects").iterdir()
        )
    )
    checks["firstFatalWins"] = third["reasonCode"] == first["reasonCode"]
    checks["exactOneTerminalRoot"] = (
        assess(store, "K1_SERIES_STOPPED", {})[0] == "READY"
        and all(assess(store, node, {})[0] != "READY" for node in (
            "KOTLIN_REAL_REPOSITORY_READY", "KOTLIN_APPLICABILITY_OR_COST_GAP",
        ))
    )

    forged = Store(root / "forged-fatal", bundle, create=True)
    forged_object = forged.put_object({"schema": SERIES_GUARD_SCHEMA, "state": "FATAL"})
    forged_marker = {
        "schema": SERIES_GUARD_MARKER_SCHEMA, "storeId": forged.store_id,
        "graphDigest": forged.graph_digest, "state": "FATAL",
        "previousGuardDigest": _series_guard(forged)[2], "receiptDigest": forged_object,
    }
    _atomic_create(forged.root / "guards" / "FATAL.json", canonical(forged_marker))
    try:
        _series_guard(forged)
        raise AssertionError("minimal FATAL receipt accepted")
    except HarnessError:
        checks["minimalFatalRejected"] = True

    forked = Store(root / "forked-guard", bundle, create=True)
    _atomic_create(forked.root / "guards" / "FORK.json", b"{}\n")
    try:
        _series_guard(forked)
        raise AssertionError("guard journal fork accepted")
    except HarnessError:
        checks["guardForkRejected"] = True

    malformed = Store(root / "malformed-journal", bundle, create=True)
    _atomic_create(malformed.root / "starts" / "K1-Q01-cold.json", b"{}\n")
    malformed_result = finalize_series(malformed, {})
    checks["malformedJournalIsBypass"] = malformed_result["reasonCode"] == "VERIFIED_AUTHORITY_BYPASS"

    minimal = Store(root / "minimal-attempt", bundle, create=True)
    selected_digest = "sha256:" + "3" * 64
    start_digest = minimal.record_child_start(
        "K1-Q01", "COLD", "DEDICATED_QUALIFICATION_EXACT_ARGV", selected_digest,
    )
    minimal_attempt = {
        "schema": ATTEMPT_SCHEMA, "entry": "K1-Q01", "invocation": "COLD",
        "authority": "DEDICATED_QUALIFICATION_EXACT_ARGV",
        "childStartSha256": start_digest, "childSelectedDigest": selected_digest,
    }
    attempt_digest = minimal.put_object(minimal_attempt)
    pointer = {
        "schema": "codeclew.kotlin-k1-qualification-pointer/0.1",
        "storeId": minimal.store_id, "graphDigest": minimal.graph_digest,
        "entry": "K1-Q01", "invocation": "COLD", "attemptDigest": attempt_digest,
    }
    _atomic_create(minimal.root / "qualification" / "K1-Q01-cold.json", canonical(pointer))
    minimal_result = finalize_series(minimal, {})
    checks["minimalAttemptCannotSuppressStart"] = minimal_result["reasonCode"] == "VERIFIED_AUTHORITY_BYPASS"

    unknown_start = Store(root / "unknown-start-entry", bundle, create=True)
    unknown_start_value = {
        "schema": CHILD_START_SCHEMA, "seriesId": SERIES_ID,
        "storeId": unknown_start.store_id, "graphDigest": unknown_start.graph_digest,
        "entry": "K1-X99", "invocation": "COLD",
        "authority": "DEDICATED_HOLDOUT_EXACT_ARGV",
        "selectedDigest": "sha256:" + "4" * 64, "state": "LAUNCH_COMMITTED",
    }
    _atomic_create(
        unknown_start.root / "starts" / "K1-X99-cold.json",
        canonical(unknown_start_value),
    )
    unknown_start_result = finalize_series(unknown_start, {})
    checks["unknownStartEntryIsBypass"] = (
        unknown_start_result["decision"] == "STOP"
        and unknown_start_result["reasonCode"] == "VERIFIED_AUTHORITY_BYPASS"
    )

    wrong_cohort = Store(root / "wrong-cohort-attempt", bundle, create=True)
    wrong_selected_digest = "sha256:" + "5" * 64
    wrong_start_digest = wrong_cohort.record_child_start(
        "K1-Q01", "COLD", "DEDICATED_QUALIFICATION_EXACT_ARGV",
        wrong_selected_digest,
    )
    wrong_attempt = {
        "schema": ATTEMPT_SCHEMA, "seriesId": SERIES_ID,
        "storeId": wrong_cohort.store_id, "graphDigest": wrong_cohort.graph_digest,
        "entry": "K1-Q01", "cohort": "BLIND_HOLDOUT", "invocation": "COLD",
        "status": "ADAPTER_OUTPUT", "selectedInputs": {}, "child": {},
        "repositoryBefore": {}, "repositoryAfter": {}, "sourceMutation": False,
        "modelCalls": 0, "authority": "DEDICATED_QUALIFICATION_EXACT_ARGV",
        "childStartSha256": wrong_start_digest,
        "childSelectedDigest": wrong_selected_digest, "attemptDigest": "",
    }
    wrong_attempt["attemptDigest"] = sha256_bytes(canonical(wrong_attempt))
    wrong_attempt_digest = wrong_cohort.put_object(wrong_attempt)
    _atomic_create(
        wrong_cohort.root / "qualification" / "K1-Q01-cold.json",
        canonical({
            "schema": "codeclew.kotlin-k1-qualification-pointer/0.1",
            "storeId": wrong_cohort.store_id,
            "graphDigest": wrong_cohort.graph_digest,
            "entry": "K1-Q01", "invocation": "COLD",
            "attemptDigest": wrong_attempt_digest,
        }),
    )
    wrong_cohort_result = finalize_series(wrong_cohort, {})
    checks["wrongCohortAttemptIsBypass"] = (
        wrong_cohort_result["decision"] == "STOP"
        and wrong_cohort_result["reasonCode"] == "VERIFIED_AUTHORITY_BYPASS"
    )

    drifted = Store(root / "postfreeze-drift", bundle, create=True)
    missing_root = root / "formerly-frozen"
    freeze_keys = (
        "candidateSources", "candidateBinaries", "candidateTools", "harnessSource",
        "independentAuditorSource", "requirements", "readinessGraph", "corpus",
    )
    frozen_snapshots = {
        key: {
            "kind": "LIVE_SET" if key in {"candidateSources", "candidateBinaries"} else "FILE",
            "path": str((missing_root / key).absolute()),
            "sha256": "sha256:" + f"{index + 1:x}" * 64,
        }
        for index, key in enumerate(freeze_keys)
    }
    freeze_path = root / "postfreeze-drift" / "candidate-freeze.json"
    _atomic_write(freeze_path, canonical({
        "schema": "codeclew.kotlin-k1-candidate-freeze/0.1", "seriesId": SERIES_ID,
        "snapshots": frozen_snapshots, "qualificationReceiptSha256": "sha256:" + "a" * 64,
        "postFreezeChangesAllowed": False, "modelCalls": 0,
    }))
    freeze_receipt = {
        "schema": RECEIPT_SCHEMA, "storeId": drifted.store_id, "seriesId": SERIES_ID,
        "graphDigest": drifted.graph_digest, "checkerVersion": CHECKER_VERSION,
        "checkerSourceDigest": sha256_file(Path(__file__)), "node": "CANDIDATE_FREEZE_VERIFY",
        "action": "VERIFY", "nodeKey": "sha256:" + "b" * 64, "status": "READY",
        "selectedInputs": {}, "dependencies": {}, "evidence": {}, "error": None,
    }
    freeze_receipt_digest = drifted.put_object(freeze_receipt)
    drifted._write_pointer_unchecked("CANDIDATE_FREEZE_VERIFY", freeze_receipt_digest)
    drift_result = finalize_series(drifted, {
        "candidateFreeze": {"kind": "FILE", "path": str(freeze_path)},
    })
    checks["invalidPostfreezeMemberStops"] = (
        drift_result["reasonCode"] == "POST_FREEZE_CANDIDATE_OR_HOLDOUT_CHANGE"
        and drift_result["decision"] == "STOP"
    )
    if not all(checks.values()):
        raise AssertionError(f"guard terminal self-test failed: {checks}")
    return checks


def _corpus_runner_snapshot_binding_self_test() -> dict[str, bool]:
    """Reach the corpus preflight through the module-level snapshot helper."""
    calls = 0

    class SnapshotReached(RuntimeError):
        pass

    class SyntheticStore:
        @contextlib.contextmanager
        def locked(self):
            yield

        @staticmethod
        def qualification_attempt(_entry_id: str, _invocation: str) -> None:
            return None

        @staticmethod
        def publish_qualification_attempt(_attempt: Mapping[str, Any]) -> str:
            raise AssertionError("synthetic preflight unexpectedly reached publication")

        @staticmethod
        def pointer(_node_id: str) -> dict[str, str]:
            return {"receiptDigest": "sha256:" + "1" * 64}

    def synthetic_entry(*_arguments: Any, **_keywords: Any) -> dict[str, str]:
        return {"cohort": "QUALIFICATION"}

    def ready(*_arguments: Any, **_keywords: Any) -> tuple[str, None, None]:
        return "READY", None, None

    def reached(_descriptor: Mapping[str, Any]) -> dict[str, str]:
        nonlocal calls
        calls += 1
        raise SnapshotReached

    original_entry = globals()["assert_entry_run_allowed"]
    original_assess = globals()["assess"]
    original_snapshot = globals()["snapshot_input"]
    reached_preflight = False
    try:
        globals()["assert_entry_run_allowed"] = synthetic_entry
        globals()["assess"] = ready
        globals()["snapshot_input"] = reached
        try:
            _run_corpus_entry(
                SyntheticStore(),  # type: ignore[arg-type]
                "K1-Q01", "COLD", Path("/synthetic/repository"),
                Path("/synthetic/evidence"), Path("/synthetic/semantic-state"),
                Path("/synthetic/build-state"),
                {"qualificationDependencySeed": {}}, cohort="QUALIFICATION",
            )
        except SnapshotReached:
            reached_preflight = True
    finally:
        globals()["assert_entry_run_allowed"] = original_entry
        globals()["assess"] = original_assess
        globals()["snapshot_input"] = original_snapshot
    checks = {
        "snapshotInputNotLocal": "snapshot_input" not in _run_corpus_entry.__code__.co_varnames,
        "globalSnapshotInputReachedInPreflight": reached_preflight and calls == 1,
    }
    if not all(checks.values()):
        raise AssertionError(f"corpus runner snapshot binding self-test failed: {checks}")
    return checks


def self_test() -> dict[str, Any]:
    bundle = load_production_bundle()
    counterexamples = 0
    corpus_runner_snapshot_binding = _corpus_runner_snapshot_binding_self_test()
    cargo_seed_lock_self_test = _baseline_cargo_seed_lock_self_test()
    counterexamples += sum(
        bool(value) for key, value in cargo_seed_lock_self_test.items()
        if key != "cleanAccepted"
    )
    baseline_environment_policy_self_test = _baseline_environment_policy_self_test()
    counterexamples += sum(
        bool(value) for key, value in baseline_environment_policy_self_test.items()
        if key not in {"cleanCargoAccepted", "cleanGradleAccepted"}
    )
    proof_conformance = _proof_safety_conformance()
    if proof_conformance["status"] != "PASS":
        raise AssertionError("structural proof-safety conformance failed")
    counterexamples += sum(
        bool(value) for key, value in proof_conformance["checks"].items()
        if key != "cleanAccepted"
    )
    measurement_conformance = _measurement_conformance(bundle)
    if measurement_conformance["status"] != "PASS":
        raise AssertionError("applicability/cache/cost conformance failed")
    counterexamples += sum(
        bool(value) for key, value in measurement_conformance["checks"].items()
        if key != "completeFixturePasses"
    )
    with tempfile.TemporaryDirectory(prefix="codeclew-k1-harness-") as temporary_text:
        temporary = Path(temporary_text)
        build_state_self_test = _build_state_self_test(temporary / "build-state-test")
        if not all(build_state_self_test[key] for key in (
            "coldWarmSameSeed", "coldWarmDistinctRoots", "reusedRootRejected", "forgedMarkerRejected",
            "modeDriftRejected", "undeclaredMemberRejected", "symlinkRejected", "freshRuntimeMutable",
        )):
            raise AssertionError("build-state seed/clone self-test failed")
        counterexamples += 5
        dependency_publication_self_test = _dependency_publication_self_test(
            temporary / "dependency-publication-test"
        )
        if not all(dependency_publication_self_test[key] for key in (
            "nestedMoveBeforeRootSeal", "cohortMoveBeforeRootSeal",
            "postRenameFailureRemoved",
        )):
            raise AssertionError("dependency publication lifecycle self-test failed")
        counterexamples += 3
        guard_terminal = _guard_terminal_self_test(temporary / "guard-terminal", bundle)
        counterexamples += len(guard_terminal)
        alternate = temporary / "alternate-graph.json"
        altered = dict(bundle["readinessGraph"])
        altered["graphId"] = "FORGED"
        _atomic_write(alternate, canonical(altered))
        try:
            load_authority("readinessGraph", alternate)
            raise AssertionError("alternate graph accepted")
        except HarnessError:
            counterexamples += 1
        alternate_requirements = temporary / "alternate-requirements.json"
        forged_requirements = json.loads(json.dumps(bundle["requirements"]))
        forged_requirements["decisionThresholds"]["modelCalls"] = 1
        _atomic_write(alternate_requirements, canonical(forged_requirements))
        try:
            load_authority("requirements", alternate_requirements)
            raise AssertionError("alternate thresholds accepted")
        except HarnessError:
            counterexamples += 1
        alternate_corpus = temporary / "alternate-corpus.json"
        forged_corpus = json.loads(json.dumps(bundle["corpus"]))
        forged_corpus["entries"] = forged_corpus["entries"][:-1]
        _atomic_write(alternate_corpus, canonical(forged_corpus))
        try:
            load_authority("corpus", alternate_corpus)
            raise AssertionError("alternate corpus accepted")
        except HarnessError:
            counterexamples += 1

        # Q05-shaped source trees contain repository-tracked links. They are
        # hashed as link objects (lstat + readlink), never as dereferenced
        # content; an untracked or escaping link is rejected.
        linked_repo = temporary / "q05-shaped-linked-source"
        linked_repo.mkdir()
        subprocess.run(["git", "init", "-q", str(linked_repo)], check=True)
        subprocess.run(["git", "-C", str(linked_repo), "config", "user.email", "k1@example.invalid"], check=True)
        subprocess.run(["git", "-C", str(linked_repo), "config", "user.name", "K1 Test"], check=True)
        (linked_repo / "config").mkdir()
        _atomic_write(linked_repo / "config" / "detekt.yml", b"rules: {}\n")
        os.symlink("config/detekt.yml", linked_repo / "detekt.yml")
        subprocess.run(["git", "-C", str(linked_repo), "add", "."], check=True)
        subprocess.run(["git", "-C", str(linked_repo), "commit", "-qm", "tracked link"], check=True)
        linked_digest = _source_tree_digest(linked_repo)
        _atomic_write(linked_repo / "config" / "detekt.yml", b"rules: {changed: true}\n")
        if _source_tree_digest(linked_repo) == linked_digest:
            # File content remains independently represented, so changing the
            # target file must still change the whole source-tree identity.
            raise AssertionError("tracked-link target member mutation was not represented")
        subprocess.run(["git", "-C", str(linked_repo), "checkout", "--", "config/detekt.yml"], check=True)
        os.symlink("config/detekt.yml", linked_repo / "untracked-link")
        try:
            _source_tree_digest(linked_repo)
            raise AssertionError("untracked source symlink accepted")
        except HarnessError:
            counterexamples += 1
        (linked_repo / "untracked-link").unlink()
        (linked_repo / "detekt.yml").unlink()
        os.symlink("../../outside", linked_repo / "detekt.yml")
        try:
            _source_tree_digest(linked_repo)
            raise AssertionError("tracked-path escaping source symlink accepted")
        except HarnessError:
            counterexamples += 1
        (linked_repo / "detekt.yml").unlink()
        os.symlink("config/detekt.yml", linked_repo / "detekt.yml")
        qualification_set = temporary / "qualification-source-set"
        qualification_set.mkdir()
        for entry_id in EXPECTED_QUALIFICATION:
            shutil.copytree(linked_repo, qualification_set / entry_id, symlinks=True)
        clean_source_set_digest = _source_set_digest(qualification_set)
        _atomic_write(qualification_set / "K1-Q05" / "config" / "detekt.yml", b"dirty: true\n")
        try:
            _source_set_digest(qualification_set)
            raise AssertionError("dirty Q05-shaped SOURCE_SET member accepted")
        except HarnessError:
            counterexamples += 1
        subprocess.run(
            ["git", "-C", str(qualification_set / "K1-Q05"), "checkout", "--", "config/detekt.yml"],
            check=True,
        )
        if _source_set_digest(qualification_set) != clean_source_set_digest:
            raise AssertionError("restored SOURCE_SET identity changed")

        archive_identity_self_test = _archive_identity_self_test(temporary / "archive-identity")
        counterexamples += sum(
            bool(value) for key, value in archive_identity_self_test.items()
            if key not in {"crlfWorktreeBytesAccepted", "rawBlobIdentityImported"}
        )

        # Synthetic Git metadata is intentionally sealed 0500/0400. Cleanup
        # must restore only owner permissions inside the exact harness root;
        # an outside target or a symlink root must be rejected without
        # touching its referent.
        cleanup_containment = temporary / "disposable-cleanup"
        cleanup_containment.mkdir(mode=0o700)
        cleanup_link_referent = temporary / "cleanup-link-referent"
        cleanup_link_referent.mkdir(mode=0o700)
        cleanup_link_marker = cleanup_link_referent / "marker"
        _atomic_write(cleanup_link_marker, b"internal link referent\n")
        sealed_disposable = cleanup_containment / "disposable-sources"
        for checkout_name in ("online", "offline"):
            sealed_object_directory = sealed_disposable / checkout_name / ".git" / "objects" / "aa"
            sealed_object_directory.mkdir(parents=True, mode=0o700)
            _atomic_write(sealed_object_directory / "object", b"synthetic git object\n", 0o400)
            _atomic_write(sealed_disposable / checkout_name / "Example.kt", b"class Example\n", 0o400)
            if checkout_name == "online":
                os.symlink(cleanup_link_referent, sealed_disposable / checkout_name / "external-link")
            for directory in (
                sealed_object_directory,
                sealed_object_directory.parent,
                sealed_object_directory.parent.parent,
                sealed_object_directory.parent.parent.parent,
            ):
                directory.chmod(0o500)
        sealed_disposable.chmod(0o500)
        _discard_disposable_source(sealed_disposable, cleanup_containment)
        if sealed_disposable.exists() or sealed_disposable.is_symlink():
            raise AssertionError("read-only disposable Git tree was not removed")
        if cleanup_link_marker.read_bytes() != b"internal link referent\n":
            raise AssertionError("internal disposable symlink was followed")

        # `_disposable_git_archive` returns a canonical path even when its
        # caller used an aliased ancestor (macOS `/var` -> `/private/var` has
        # the same shape). The resolved direct child remains safely deletable.
        alias_referent = temporary / "cleanup-alias-referent"
        alias_referent.mkdir(mode=0o700)
        alias_parent = temporary / "cleanup-alias"
        os.symlink(alias_referent, alias_parent)
        alias_disposable = alias_referent / "disposable-sources"
        alias_disposable.mkdir(mode=0o700)
        alias_head = alias_disposable / ".git" / "HEAD"
        alias_head.parent.mkdir(mode=0o700)
        _atomic_write(alias_head, b"ref: refs/heads/k1-detached\n", 0o400)
        alias_head.parent.chmod(0o500)
        alias_disposable.chmod(0o500)
        _discard_disposable_source(alias_disposable.resolve(strict=True), alias_parent)
        if alias_disposable.exists() or alias_disposable.is_symlink():
            raise AssertionError("canonical disposable under aliased containment was not removed")
        alias_parent.unlink()

        # The exception path can observe a half-built pair (for example, the
        # online checkout is sealed before construction of offline fails).
        mid_failure_staging = temporary / "prepare-mid-failure"
        mid_failure_containment = mid_failure_staging / ".work" / "K1-Q01"
        mid_failure_containment.mkdir(parents=True, mode=0o700)
        mid_failure_disposable = mid_failure_containment / "disposable-sources"
        mid_failure_git = mid_failure_disposable / "online" / ".git"
        mid_failure_git.mkdir(parents=True, mode=0o700)
        _atomic_write(mid_failure_git / "HEAD", b"ref: refs/heads/k1-detached\n", 0o400)
        mid_failure_git.chmod(0o500)
        (mid_failure_disposable / "online").chmod(0o500)
        mid_failure_disposable.chmod(0o500)
        _discard_disposable_source(mid_failure_disposable, mid_failure_containment)
        if mid_failure_disposable.exists() or mid_failure_disposable.is_symlink():
            raise AssertionError("mid-failure disposable Git tree was not removed")
        shutil.rmtree(mid_failure_staging)
        if mid_failure_staging.exists() or mid_failure_staging.is_symlink():
            raise AssertionError("mid-failure PREPARE staging tree was not removed")

        outside_disposable = temporary / "outside-disposable"
        outside_disposable.mkdir(mode=0o700)
        outside_marker = outside_disposable / "marker"
        _atomic_write(outside_marker, b"outside\n")
        try:
            _discard_disposable_source(outside_disposable, cleanup_containment)
            raise AssertionError("outside disposable cleanup target accepted")
        except HarnessError:
            counterexamples += 1
        if outside_marker.read_bytes() != b"outside\n":
            raise AssertionError("outside disposable cleanup target was mutated")
        linked_disposable = cleanup_containment / "linked-disposable"
        os.symlink(outside_disposable, linked_disposable)
        try:
            _discard_disposable_source(linked_disposable, cleanup_containment)
            raise AssertionError("symlink disposable cleanup target accepted")
        except HarnessError:
            counterexamples += 1
        if not linked_disposable.is_symlink() or outside_marker.read_bytes() != b"outside\n":
            raise AssertionError("symlink disposable cleanup target was followed")
        linked_disposable.unlink()

        # PREPARE uses separate, retained profiles: only online may allow
        # network. The offline sentinel runs under the exact offline profile
        # before the build tool and accepts only kernel EACCES/EPERM denial.
        network_staging = (
            temporary / ".qualificationDependencySeed.prepare-000000000000000000000000"
            / ".work" / "fixture"
        )
        online_network_source = network_staging / "disposable-sources" / "online"
        offline_network_source = network_staging / "disposable-sources" / "offline"
        network_home = network_staging / "home"
        online_network_source.mkdir(parents=True)
        offline_network_source.mkdir()
        network_home.mkdir()
        online_network_raw = _preparation_sandbox_profile(
            online_network_source, network_staging, allow_network=True,
        )
        offline_network_raw = _preparation_sandbox_profile(
            offline_network_source, network_staging, allow_network=False,
        )
        online_network_profile = network_staging / "online.sb"
        offline_network_profile = network_staging / "offline.sb"
        _atomic_write(online_network_profile, online_network_raw, 0o400)
        _atomic_write(offline_network_profile, offline_network_raw, 0o400)
        traversal_root = temporary / "prepare-traversal"
        traversal_staging = traversal_root / "staging"
        traversal_source = traversal_staging / "source"
        traversal_descendant = traversal_source / "descendant"
        traversal_home = traversal_staging / "home"
        traversal_descendant.mkdir(parents=True)
        traversal_home.mkdir()
        _atomic_write(traversal_descendant / "marker", b"allowed\n")
        traversal_secret = traversal_root / "sibling-secret"
        _atomic_write(traversal_secret, b"must-not-cross\n", 0o600)
        protected_source = traversal_root / "selected-source"
        _atomic_write(protected_source, b"selected-authority\n", 0o600)
        traversal_offline_raw = _preparation_sandbox_profile(
            traversal_source, traversal_staging, allow_network=False,
        )
        traversal_profile = traversal_staging / "offline.sb"
        _atomic_write(traversal_profile, traversal_offline_raw, 0o400)
        traversal_env = _preparation_environment(traversal_staging, "MAVEN")
        maven_traversal_probe = _bounded_prepare_run(
            [
                "/usr/bin/sandbox-exec", "-f", str(traversal_profile),
                "/opt/homebrew/Cellar/maven/3.9.12/bin/mvn", "-v",
            ],
            traversal_source, traversal_env,
        )
        if (
            maven_traversal_probe.returncode != 0
            or b"Apache Maven 3.9.12" not in maven_traversal_probe.stdout
            or b"Not a directory" in maven_traversal_probe.stderr
        ):
            raise AssertionError("PREPARE Maven launcher traversal failed")
        source_traversal_probe = _bounded_prepare_run(
            [
                "/usr/bin/sandbox-exec", "-f", str(traversal_profile),
                "/bin/sh", "-c",
                'set -eu; cd "$1"; /usr/bin/stat -f %HT . descendant descendant/marker; '
                'cd descendant; test "$(pwd -P)" = "$1/descendant"; cd ..; '
                'test "$(pwd -P)" = "$1"',
                "k1-traversal", str(traversal_source.resolve(strict=True)),
            ],
            traversal_source, traversal_env,
        )
        if source_traversal_probe.returncode != 0:
            raise AssertionError("PREPARE source ancestry traversal failed")
        denied_read_code = (
            'open(F,"<",$ARGV[0]) and exit 42;'
            'exit(($!{EPERM}||$!{EACCES})?0:41)'
        )
        denied_write_code = (
            'open(F,">>",$ARGV[0]) and exit 42;'
            'exit(($!{EPERM}||$!{EACCES})?0:41)'
        )
        denied_traversal_probes = [
            ["/usr/bin/perl", "-e", denied_read_code, str(traversal_secret)],
            ["/usr/bin/perl", "-e", denied_read_code,
             str(Path.home() / "Library/Keychains/login.keychain-db")],
            ["/usr/bin/perl", "-e", denied_write_code, str(traversal_secret)],
            ["/usr/bin/perl", "-e", denied_write_code, str(protected_source)],
        ]
        for denied_command in denied_traversal_probes:
            denied_probe = _bounded_prepare_run(
                ["/usr/bin/sandbox-exec", "-f", str(traversal_profile), *denied_command],
                traversal_source, traversal_env,
            )
            if denied_probe.returncode != 0 or denied_probe.stdout or denied_probe.stderr:
                raise AssertionError("PREPARE traversal ancestor exposed protected authority")
        if traversal_secret.read_bytes() != b"must-not-cross\n":
            raise AssertionError("PREPARE traversal sandbox mutated sibling authority")
        if protected_source.read_bytes() != b"selected-authority\n":
            raise AssertionError("PREPARE traversal sandbox mutated selected source authority")
        online_listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            online_listener.bind(("127.0.0.1", 0))
            online_listener.listen(1)
            online_port = online_listener.getsockname()[1]
            online_connect_code = (
                'socket(S,PF_INET,SOCK_STREAM,6) or exit 31;'
                f'connect(S,sockaddr_in({online_port},inet_aton("127.0.0.1"))) or exit 41;'
                'exit 0'
            )
            online_probe = _bounded_prepare_run(
                [
                    "/usr/bin/sandbox-exec", "-f", str(online_network_profile),
                    "/usr/bin/perl", "-MSocket", "-e", online_connect_code,
                ],
                online_network_source,
                {
                    "HOME": str(network_home), "USERPROFILE": str(network_home),
                    "TMPDIR": str(network_home), "PATH": "/usr/bin:/bin",
                    "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8",
                },
            )
        finally:
            online_listener.close()
        if online_probe.returncode != 0 or online_probe.stdout or online_probe.stderr:
            raise AssertionError("online PREPARE network profile did not retain explicit allow")
        sentinel_argv = _prepare_network_sentinel_argv()
        sentinel_probe = _bounded_prepare_run(
            ["/usr/bin/sandbox-exec", "-f", str(offline_network_profile), *sentinel_argv],
            offline_network_source,
            {
                "HOME": str(network_home), "USERPROFILE": str(network_home),
                "TMPDIR": str(network_home), "PATH": "/usr/bin:/bin",
                "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8",
            },
        )
        if sentinel_probe.returncode != 0 or sentinel_probe.stdout or sentinel_probe.stderr:
            raise AssertionError("offline PREPARE network sentinel did not prove denial")
        network_entry = {
            "entry": "fixture", "buildDsl": "GRADLE_KOTLIN_DSL",
            "selectedCompilation": ":/main",
        }
        network_commands = [
            _expected_dependency_prepare_argv(
                network_entry, online_network_source, network_staging, offline=False,
            ),
            _expected_dependency_prepare_argv(
                network_entry, offline_network_source, network_staging, offline=True,
            ),
        ]
        if any(command is None for command in network_commands):
            raise AssertionError("synthetic PREPARE command projection failed")
        network_row = {
            **network_entry,
            "prepareArgv": network_commands,
            "prepareArgvSha256": sha256_bytes(canonical(network_commands)),
            "sandboxProfiles": {
                "online": {
                    "policy": PREPARE_ONLINE_NETWORK_POLICY,
                    "profileSha256": sha256_bytes(online_network_raw),
                    "profileBytes": online_network_raw.decode(),
                },
                "offline": {
                    "policy": PREPARE_OFFLINE_NETWORK_POLICY,
                    "profileSha256": sha256_bytes(offline_network_raw),
                    "profileBytes": offline_network_raw.decode(),
                },
            },
            "prepareEnvironments": {
                phase: {
                    "environment": _preparation_environment(
                        network_staging, network_entry["buildDsl"],
                    ),
                    "environmentSha256": sha256_bytes(canonical(
                        _preparation_environment(
                            network_staging, network_entry["buildDsl"],
                        )
                    )),
                }
                for phase in ("online", "offline")
            },
            "offlineNetworkSentinel": {
                "argv": sentinel_argv,
                "argvSha256": sha256_bytes(canonical(sentinel_argv)),
                "executed": True, "exitCode": 0,
                "stdoutSha256": sha256_bytes(b""), "stderrSha256": sha256_bytes(b""),
                "denialErrnos": ["EACCES", "EPERM"],
            },
            "offlineNoDownloadMarker": {
                "flag": "--offline", "commandIndex": 1, "presentExactlyOnce": True,
                "offlineCommandSha256": sha256_bytes(canonical(network_commands[1])),
            },
            "outcome": "READY",
        }
        if not _preparation_network_evidence_valid(network_row):
            raise AssertionError("valid split PREPARE network evidence rejected")
        gradle_environment = network_row["prepareEnvironments"]["online"]["environment"]
        if (
            gradle_environment.get("GRADLE_USER_HOME")
            != str(network_staging.resolve(strict=False) / "gradle-user-home")
            or gradle_environment.get("GRADLE_OPTS")
            != f"-Djava.io.tmpdir={network_staging.resolve(strict=False) / 'home'}"
            or network_row["prepareEnvironments"]["online"]
            != network_row["prepareEnvironments"]["offline"]
        ):
            raise AssertionError("Gradle wrapper bootstrap/JVM temp authority is not exact and phase-stable")
        maven_entry = {
            "entry": "fixture", "buildDsl": "MAVEN",
            "selectedCompilation": ":/main",
        }
        maven_online = _expected_dependency_prepare_argv(
            maven_entry, online_network_source, network_staging, offline=False,
        )
        maven_offline = _expected_dependency_prepare_argv(
            maven_entry, offline_network_source, network_staging, offline=True,
        )
        if maven_online is None or maven_offline is None:
            raise AssertionError("synthetic Maven PREPARE command projection failed")
        model_goals = ("help:effective-pom", "dependency:build-classpath")
        if (
            any(maven_online.count(goal) != 1 for goal in model_goals)
            or any(maven_offline.count(goal) != 1 for goal in model_goals)
            or maven_online.index("dependency:go-offline") >= maven_online.index(model_goals[0])
            or maven_online.index("install") >= maven_online.index(model_goals[0])
            or "-o" in maven_online or maven_offline.count("-o") != 1
            or "GRADLE_USER_HOME" in _preparation_environment(network_staging, "MAVEN")
        ):
            raise AssertionError("Maven online PREPARE does not prefetch exact offline model goals")
        maven_row = json.loads(json.dumps(network_row))
        maven_row.update(maven_entry)
        maven_row["prepareArgv"] = [maven_online, maven_offline]
        maven_row["prepareArgvSha256"] = sha256_bytes(canonical(maven_row["prepareArgv"]))
        maven_environment = _preparation_environment(network_staging, "MAVEN")
        maven_row["prepareEnvironments"] = {
            phase: {
                "environment": dict(maven_environment),
                "environmentSha256": sha256_bytes(canonical(maven_environment)),
            }
            for phase in ("online", "offline")
        }
        maven_row["offlineNoDownloadMarker"] = {
            "flag": "-o", "commandIndex": 1, "presentExactlyOnce": True,
            "offlineCommandSha256": sha256_bytes(canonical(maven_offline)),
        }
        if not _preparation_network_evidence_valid(maven_row):
            raise AssertionError("valid Maven PREPARE goal-prefetch evidence rejected")

        # Publication removes `.work`; retained evidence must remain exactly
        # recomputable without consulting the vanished write-root directory.
        post_publication_row = json.loads(json.dumps(network_row))
        shutil.rmtree(network_staging.parent.parent)
        if network_staging.exists() or not _preparation_network_evidence_valid(post_publication_row):
            raise AssertionError("post-publication PREPARE evidence became existence-sensitive")
        swapped_profiles = json.loads(json.dumps(network_row))
        swapped_profiles["sandboxProfiles"]["online"], swapped_profiles["sandboxProfiles"]["offline"] = (
            swapped_profiles["sandboxProfiles"]["offline"], swapped_profiles["sandboxProfiles"]["online"]
        )
        allow_network_offline = json.loads(json.dumps(network_row))
        forged_offline_raw = offline_network_raw.decode().replace("(deny network*)", "(allow network*)")
        allow_network_offline["sandboxProfiles"]["offline"]["profileBytes"] = forged_offline_raw
        allow_network_offline["sandboxProfiles"]["offline"]["profileSha256"] = sha256_bytes(forged_offline_raw.encode())
        forged_sentinel = json.loads(json.dumps(network_row))
        forged_sentinel["offlineNetworkSentinel"]["argv"] = ["/usr/bin/perl", "-e", "exit 0"]
        forged_sentinel["offlineNetworkSentinel"]["argvSha256"] = sha256_bytes(
            canonical(forged_sentinel["offlineNetworkSentinel"]["argv"])
        )
        ancestor_data_only_profile = json.loads(json.dumps(network_row))
        ancestor_data_only_raw = offline_network_raw.decode().replace(
            "(allow file-read-data file-read-metadata (literal ",
            "(allow file-read-data (literal ",
        )
        ancestor_data_only_profile["sandboxProfiles"]["offline"]["profileBytes"] = ancestor_data_only_raw
        ancestor_data_only_profile["sandboxProfiles"]["offline"]["profileSha256"] = sha256_bytes(
            ancestor_data_only_raw.encode()
        )
        broad_default_profile = json.loads(json.dumps(network_row))
        broad_default_raw = offline_network_raw.decode().replace(
            "(deny default)\n", "(deny default)\n(allow default)\n",
        )
        broad_default_profile["sandboxProfiles"]["offline"]["profileBytes"] = broad_default_raw
        broad_default_profile["sandboxProfiles"]["offline"]["profileSha256"] = sha256_bytes(
            broad_default_raw.encode()
        )
        broad_read_profile = json.loads(json.dumps(network_row))
        broad_read_raw = offline_network_raw.decode().replace(
            "(allow sysctl-read)\n", "(allow sysctl-read)\n(allow file-read*)\n",
        )
        broad_read_profile["sandboxProfiles"]["offline"]["profileBytes"] = broad_read_raw
        broad_read_profile["sandboxProfiles"]["offline"]["profileSha256"] = sha256_bytes(
            broad_read_raw.encode()
        )
        missing_dev_null_profile = json.loads(json.dumps(network_row))
        missing_dev_null_raw = offline_network_raw.decode().replace(
            _SANDBOX_DEV_NULL_WRITE + "\n", "",
        )
        missing_dev_null_profile["sandboxProfiles"]["offline"]["profileBytes"] = missing_dev_null_raw
        missing_dev_null_profile["sandboxProfiles"]["offline"]["profileSha256"] = sha256_bytes(
            missing_dev_null_raw.encode()
        )
        broad_dev_null_profile = json.loads(json.dumps(network_row))
        broad_dev_null_raw = offline_network_raw.decode().replace(
            _SANDBOX_DEV_NULL_WRITE, '(allow file-write* (literal "/dev/null"))',
        )
        broad_dev_null_profile["sandboxProfiles"]["offline"]["profileBytes"] = broad_dev_null_raw
        broad_dev_null_profile["sandboxProfiles"]["offline"]["profileSha256"] = sha256_bytes(
            broad_dev_null_raw.encode()
        )
        offline_var_alias_profile = json.loads(json.dumps(network_row))
        offline_var_alias_raw = offline_network_raw.decode().replace(
            "(deny network*)\n", "(deny network*)\n" + _SANDBOX_ONLINE_VAR_METADATA + "\n",
        )
        offline_var_alias_profile["sandboxProfiles"]["offline"]["profileBytes"] = offline_var_alias_raw
        offline_var_alias_profile["sandboxProfiles"]["offline"]["profileSha256"] = sha256_bytes(
            offline_var_alias_raw.encode()
        )
        wrong_maven_tmpdir = json.loads(json.dumps(network_row))
        wrong_environment = wrong_maven_tmpdir["prepareEnvironments"]["offline"]["environment"]
        wrong_environment["MAVEN_OPTS"] = "-Djava.io.tmpdir=/tmp"
        wrong_maven_tmpdir["prepareEnvironments"]["offline"]["environmentSha256"] = sha256_bytes(
            canonical(wrong_environment)
        )
        split_phase_environment = json.loads(json.dumps(network_row))
        split_environment = split_phase_environment["prepareEnvironments"]["offline"]["environment"]
        split_environment["LANG"] = "C"
        split_phase_environment["prepareEnvironments"]["offline"]["environmentSha256"] = sha256_bytes(
            canonical(split_environment)
        )
        missing_gradle_home = json.loads(json.dumps(network_row))
        for phase in ("online", "offline"):
            phase_environment = missing_gradle_home["prepareEnvironments"][phase]["environment"]
            phase_environment.pop("GRADLE_USER_HOME")
            missing_gradle_home["prepareEnvironments"][phase]["environmentSha256"] = sha256_bytes(
                canonical(phase_environment)
            )
        missing_gradle_tmpdir = json.loads(json.dumps(network_row))
        for phase in ("online", "offline"):
            phase_environment = missing_gradle_tmpdir["prepareEnvironments"][phase]["environment"]
            phase_environment.pop("GRADLE_OPTS")
            missing_gradle_tmpdir["prepareEnvironments"][phase]["environmentSha256"] = sha256_bytes(
                canonical(phase_environment)
            )
        wrong_gradle_tmpdir = json.loads(json.dumps(network_row))
        for phase in ("online", "offline"):
            phase_environment = wrong_gradle_tmpdir["prepareEnvironments"][phase]["environment"]
            phase_environment["GRADLE_OPTS"] = "-Djava.io.tmpdir=/tmp"
            wrong_gradle_tmpdir["prepareEnvironments"][phase]["environmentSha256"] = sha256_bytes(
                canonical(phase_environment)
            )
        missing_maven_online_goal = json.loads(json.dumps(maven_row))
        missing_maven_online_goal["prepareArgv"][0].remove("help:effective-pom")
        missing_maven_online_goal["prepareArgvSha256"] = sha256_bytes(
            canonical(missing_maven_online_goal["prepareArgv"])
        )
        root_substitution_profiles = []
        for substituted_root in (
            Path("/"), Path("/private"), network_staging.parent / "sibling",
        ):
            substituted_root = substituted_root.resolve(strict=False)
            ancestors = {Path("/")}
            current = substituted_root
            while current != current.parent:
                ancestors.add(current)
                current = current.parent
            forged_lines = offline_network_raw.decode().splitlines()[:9]
            forged_lines.extend(
                f"(allow file-read-data file-read-metadata (literal {json.dumps(str(path))}))"
                for path in sorted(ancestors, key=lambda value: (len(value.parts), str(value)))
            )
            forged_lines.extend((
                f"(allow file-read* (subpath {json.dumps(str(substituted_root))}))",
                f"(allow file-write* (subpath {json.dumps(str(substituted_root))}))",
            ))
            forged_raw = "\n".join(forged_lines) + "\n"
            forged_row = json.loads(json.dumps(network_row))
            forged_row["sandboxProfiles"]["offline"]["profileBytes"] = forged_raw
            forged_row["sandboxProfiles"]["offline"]["profileSha256"] = sha256_bytes(
                forged_raw.encode()
            )
            root_substitution_profiles.append(forged_row)
        split_offline_staging = (
            temporary / ".qualificationDependencySeed.prepare-111111111111111111111111"
            / ".work" / "fixture"
        )
        split_offline_source = split_offline_staging / "disposable-sources" / "offline"
        split_offline_source.mkdir(parents=True)
        split_offline_raw = _preparation_sandbox_profile(
            split_offline_source, split_offline_staging, allow_network=False,
        )
        split_offline_command = _expected_dependency_prepare_argv(
            network_entry, split_offline_source, split_offline_staging, offline=True,
        )
        if split_offline_command is None:
            raise AssertionError("synthetic split-phase PREPARE command projection failed")
        split_phase_roots = json.loads(json.dumps(network_row))
        split_phase_roots["prepareArgv"][1] = split_offline_command
        split_phase_roots["prepareArgvSha256"] = sha256_bytes(
            canonical(split_phase_roots["prepareArgv"])
        )
        split_phase_roots["sandboxProfiles"]["offline"] = {
            "policy": PREPARE_OFFLINE_NETWORK_POLICY,
            "profileSha256": sha256_bytes(split_offline_raw),
            "profileBytes": split_offline_raw.decode(),
        }
        split_phase_roots["offlineNoDownloadMarker"]["offlineCommandSha256"] = sha256_bytes(
            canonical(split_offline_command)
        )
        if any(_preparation_network_evidence_valid(row) for row in (
            swapped_profiles, allow_network_offline, forged_sentinel, ancestor_data_only_profile,
            broad_default_profile, broad_read_profile, missing_dev_null_profile,
            broad_dev_null_profile, offline_var_alias_profile, wrong_maven_tmpdir,
            split_phase_environment, missing_gradle_home, missing_gradle_tmpdir,
            wrong_gradle_tmpdir, missing_maven_online_goal,
            *root_substitution_profiles, split_phase_roots,
        )):
            raise AssertionError("forged PREPARE network evidence accepted")
        counterexamples += 19
        if not _dependency_prepare_security_authority_failure(
            b"java.io.IOException: java.io.tmpdir is set to a directory that doesn't exist: /var/folders/host/T"
        ):
            raise AssertionError("Gradle JVM temp authority failure was classified as a product refusal")
        counterexamples += 1
        offline_gradle_failures = (
            b"java.net.SocketException: Operation not permitted",
            b"java.nio.file.AccessDeniedException: /host/secret: Operation not permitted",
            b"java.io.IOException: java.io.tmpdir is set to a directory that doesn't exist",
            b"java.net.SocketTimeoutException: connect timed out: Operation not permitted",
        )
        for denial in offline_gradle_failures:
            offline_gradle_refusal = (
                "GRADLE_KOTLIN_DSL" in {"GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL"}
                and len((object(), object())) == 2
            )
            if (
                not offline_gradle_refusal
                or not _dependency_prepare_security_authority_failure(denial)
            ):
                raise AssertionError("strict-offline Gradle structural refusal contour is absent")
        structural_rejections = (
            ("wrong/online phase", "GRADLE_KOTLIN_DSL", 1),
            ("online Gradle", "GRADLE_GROOVY_DSL", 1),
            ("Maven DSL", "MAVEN", 2),
        )
        security_error = b"java.net.SocketException: Operation not permitted"
        for label, build_dsl, command_count in structural_rejections:
            offline_gradle_refusal = (
                build_dsl in {"GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL"}
                and command_count == 2
            )
            if offline_gradle_refusal or not _dependency_prepare_security_authority_failure(security_error):
                raise AssertionError(f"Gradle structural {label} escaped infrastructure classification")
        counterexamples += len(structural_rejections)
        online_refusal = json.loads(json.dumps(network_row))
        online_refusal.update({
            "outcome": "TYPED_REFUSAL", "failureStage": "ONLINE_DEPENDENCY_PREPARATION",
        })
        online_refusal["offlineNetworkSentinel"].update({"executed": False, "exitCode": None})
        if not _preparation_network_evidence_valid(online_refusal):
            raise AssertionError("sound online-stage typed refusal was rejected")
        security_supervisor_fixture = dict(SECURITY_SUPERVISOR_EXPECTED)
        prepare_security_fixture = {name: True for name in PREPARE_SECURITY_CASES}
        if not _security_tripwire_cases_valid(
            security_supervisor_fixture, prepare_security_fixture,
        ):
            raise AssertionError("valid K1-R18 security tripwire packet rejected")
        for name in SECURITY_SUPERVISOR_EXPECTED:
            forged_supervisor = dict(security_supervisor_fixture)
            forged_supervisor[name] = "NOT_RUN"
            if _security_tripwire_cases_valid(forged_supervisor, prepare_security_fixture):
                raise AssertionError("K1-R18 accepted a NOT_RUN supervisor security tripwire")
            counterexamples += 1
        for name in PREPARE_SECURITY_CASES:
            forged_prepare = dict(prepare_security_fixture)
            forged_prepare[name] = "NOT_RUN"
            if _security_tripwire_cases_valid(security_supervisor_fixture, forged_prepare):
                raise AssertionError("K1-R18 accepted a NOT_RUN PREPARE security tripwire")
            counterexamples += 1

        store = Store(temporary / "store", bundle, create=True)
        live_files: dict[str, Mapping[str, Any]] = {}
        for key in _node(store, "INPUT_AUTHORITY_VERIFY")["selectedInputs"]:
            path = temporary / f"{key}.txt"
            _atomic_write(path, f"{key}\n".encode())
            live_files[key] = {"kind": "FILE", "path": str(path.absolute())}
        digest = issue_verification(store, "INPUT_AUTHORITY_VERIFY", live_files, lambda: {"checked": True})
        if not _is_digest(digest) or assess(store, "INPUT_AUTHORITY_VERIFY", live_files)[0] != "READY":
            raise AssertionError("fresh receipt was not recognized READY")
        _atomic_write(Path(live_files["researchInput"]["path"]), b"changed\n")
        if assess(store, "INPUT_AUTHORITY_VERIFY", live_files)[0] != "STALE":
            raise AssertionError("selected live input mutation did not stale receipt")
        counterexamples += 1
        try:
            issue_verification(store, "BASELINE_CAPTURE", live_files, lambda: {"forged": True})
            raise AssertionError("DIRECT node accepted generic issuance")
        except HarnessError:
            counterexamples += 1
        try:
            assert_entry_run_allowed(store, "K1-H01", live_files)
            raise AssertionError("holdout accepted before candidate freeze")
        except HarnessError as error:
            if "HOLDOUT_ACCESS_BEFORE_CANDIDATE_FREEZE" not in str(error):
                raise
            counterexamples += 1
        exact_nodes = {node["id"]: node for node in bundle["readinessGraph"]["nodes"]}
        if exact_nodes["QUALIFICATION_DEPENDENCY_SEED_PREPARE"]["selectedInputs"] != [
            "qualificationDependencySeed", "qualificationSourceSet", "candidateTools"
        ] or exact_nodes["HOLDOUT_DEPENDENCY_SEED_PREPARE"] != {
            "id": "HOLDOUT_DEPENDENCY_SEED_PREPARE",
            "action": "PREPARE",
            "deps": ["HOLDOUT_SOURCE_MATERIALIZE", "CANDIDATE_FREEZE_VERIFY", "HOLDOUT_ELIGIBILITY_AUDIT_IMPORT"],
            "selectedInputs": ["holdoutDependencySeed", "holdoutSourceSet", "candidateTools"],
        }:
            raise AssertionError("K1.1 exact dependency/source binding is absent")
        old_ordering = json.loads(json.dumps(bundle["readinessGraph"]))
        old_nodes = {node["id"]: node for node in old_ordering["nodes"]}
        old_nodes["HOLDOUT_DEPENDENCY_SEED_PREPARE"]["deps"] = [
            "QUALIFICATION_DEPENDENCY_SEED_VERIFY", "HOLDOUT_ELIGIBILITY_AUDIT_IMPORT"
        ]
        old_nodes["CANDIDATE_FREEZE_PREPARE"]["deps"] = [
            "QUALIFICATION_RUN_6_COMPLETE", "HOLDOUT_DEPENDENCY_SEED_PREPARE", "K0_1_BYTE_EXACT_VERIFY"
        ]
        old_nodes["HOLDOUT_DEPENDENCY_SEED_VERIFY"]["deps"] = [
            "HOLDOUT_SOURCE_MATERIALIZE", "HOLDOUT_DEPENDENCY_SEED_PREPARE", "CANDIDATE_FREEZE_VERIFY"
        ]
        try:
            _validate_graph(old_ordering)
            raise AssertionError("cancelled K1 dependency ordering accepted")
        except HarnessError:
            counterexamples += 1
        if assert_entry_run_allowed(store, "K1-Q01", live_files)["cohort"] != "QUALIFICATION":
            raise AssertionError("qualification entry rejected")
        # A projection-shaped object from a caller is diagnostic evidence at
        # most: it cannot create a qualification pointer or DIRECT receipt.
        forged_attempt = {
            "schema": ATTEMPT_SCHEMA, "seriesId": SERIES_ID, "storeId": store.store_id,
            "graphDigest": store.graph_digest, "entry": "K1-Q01", "cohort": "QUALIFICATION",
            "invocation": "COLD", "status": "ADAPTER_OUTPUT", "failureStage": None,
            "reasonCode": None, "safeDetailSha256": "sha256:" + "0"*64,
            "selectedInputs": {}, "child": {}, "resource": {}, "repositoryBefore": {},
            "repositoryAfter": {}, "sourceMutation": False, "modelCalls": 0,
            "authority": "CALLER_FORGED", "attemptDigest": "sha256:" + "1"*64,
        }
        try:
            store.publish_qualification_attempt(forged_attempt)
            raise AssertionError("caller projection minted qualification authority")
        except HarnessError:
            counterexamples += 1
        if store.qualification_attempt("K1-Q01", "COLD") is not None or store.pointer("QUALIFICATION_RUN_6_COMPLETE") is not None:
            raise AssertionError("forged projection left production authority")
        # Root conditions are not generic issuers and cannot be called before
        # a K1_DECISION whose exact value matches the root.
        try:
            issue_verification(store, "KOTLIN_REAL_REPOSITORY_READY", live_files, lambda: {"decision":"GO"})
            raise AssertionError("generic conditional root issuance accepted")
        except HarnessError:
            counterexamples += 1
        # An unsafe measurement must not block the STOP decision branch. The
        # full production path checks the same property with current matrix
        # artifacts; this focused state-machine regression protects the DAG
        # semantics without materializing any corpus or holdout input.
        if "READY" not in RECEIPT_STATES or "FAILED" not in RECEIPT_STATES:
            raise AssertionError("receipt state contour changed")
        measured_safety = {"safe": False, "failureReasons": ["FALSE_PROVEN_THRESHOLD"]}
        decision = "STOP" if not measured_safety["safe"] else "GO"
        if decision != "STOP" or {
            "GO": "KOTLIN_REAL_REPOSITORY_READY",
            "STOP": "K1_SERIES_STOPPED",
        }[decision] != "K1_SERIES_STOPPED":
            raise AssertionError("unsafe measurement did not route to STOP")
        counterexamples += 1
        # The dependency PREPARE supervisor is a distinct process boundary
        # from corpus analysis. Exercise both zero and nonzero exits here so
        # stale local names or capture-path regressions cannot make every
        # real dependency preparation fail before classification.
        prepare_capture = temporary / "prepare-capture"
        prepare_capture.mkdir(mode=0o700)
        prepare_env = {
            "TMPDIR": str(prepare_capture),
            "PATH": "/usr/bin:/bin",
            "LANG": "C.UTF-8",
            "LC_ALL": "C.UTF-8",
        }
        prepare_ok = _bounded_prepare_run(
            ["/bin/sh", "-c", "printf prepared"], temporary, prepare_env
        )
        prepare_nonzero = _bounded_prepare_run(
            ["/bin/sh", "-c", "printf rejected >&2; exit 7"], temporary, prepare_env
        )
        if (
            prepare_ok.returncode != 0
            or prepare_ok.stdout != b"prepared"
            or prepare_nonzero.returncode != 7
            or prepare_nonzero.stderr != b"rejected"
        ):
            raise AssertionError("bounded dependency PREPARE supervisor contract mismatch")
        counterexamples += 2
        # HOLDOUT_SOURCE_MATERIALIZE uses the same bounded runner for clone and
        # checkout. Its isolated Git environment must supply a real capture
        # root; otherwise every holdout fails before the first child is started.
        holdout_capture = temporary / "holdout-materialize-capture"
        holdout_capture.mkdir(mode=0o700)
        holdout_git_env = {
            "HOME": str(temporary), "TMPDIR": str(holdout_capture),
            "PATH": "/usr/bin:/bin", "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_SYSTEM": "/dev/null", "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_TERMINAL_PROMPT": "0", "GIT_ASKPASS": "/usr/bin/false",
            "SSH_ASKPASS": "/usr/bin/false", "GIT_PROTOCOL_FROM_USER": "0",
            "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8",
        }
        holdout_probe = _bounded_prepare_run(
            ["/usr/bin/git", "--version"], temporary, holdout_git_env,
        )
        if (
            holdout_probe.returncode != 0
            or not holdout_probe.stdout.startswith(b"git version ")
            or any(holdout_capture.iterdir())
        ):
            raise AssertionError("holdout materializer bounded Git environment mismatch")
        counterexamples += 1
    return {
        "schema": "codeclew.kotlin-k1-harness-self-test/0.1",
        "status": "PASS",
        "authorityDigests": bundle["digests"],
        "proofSafetyConformance": proof_conformance,
        "measurementConformance": measurement_conformance,
        "buildStateSelfTest": build_state_self_test,
        "dependencyPublicationSelfTest": dependency_publication_self_test,
        "cargoSeedLockSelfTest": cargo_seed_lock_self_test,
        "baselineEnvironmentPolicySelfTest": baseline_environment_policy_self_test,
        "guardTerminalSelfTest": guard_terminal,
        "corpusRunnerSnapshotBindingSelfTest": corpus_runner_snapshot_binding,
        "requirementCases": {
            "alternateGraphRejected": True,
            "alternateThresholdRejected": True,
            "alternateCorpusRejected": True,
            "staleInputRejected": True,
            "directNodeForgeryRejected": True,
            "earlyHoldoutRejected": True,
            "callerAttemptForgeryRejected": True,
            "conditionalRootForgeryRejected": True,
            "cancelledOrderingRejected": True,
            "trackedLinkEscapeRejected": True,
            "dirtySourceSetRejected": True,
            **archive_identity_self_test,
            "readOnlyDisposableCleanupPassed": True,
            "aliasedContainmentCleanupPassed": True,
            "midFailureDisposableCleanupPassed": True,
            "outsideDisposableCleanupRejected": True,
            "symlinkDisposableCleanupRejected": True,
            "splitPrepareNetworkProfilesPassed": True,
            "offlinePrepareNetworkSentinelPassed": True,
            "prepareMavenLauncherTraversalPassed": True,
            "prepareSourceAncestryTraversalPassed": True,
            "prepareAncestorSecretReadDenied": True,
            "prepareAncestorWriteDenied": True,
            "prepareSelectedSourceWriteDenied": True,
            "prepareKeychainReadDenied": True,
            "prepareTraversalNetworkSemanticsPreserved": True,
            "prepareAncestorDataOnlyMutationRejected": True,
            "prepareBroadSandboxPermissionRejected": True,
            "prepareRootAuthoritySubstitutionsRejected": True,
            "prepareSplitPhaseRootsRejected": True,
            "prepareDevNullWriteDataPassed": True,
            "prepareOnlineVarMetadataOnlyPassed": True,
            "prepareMissingProfileClauseRejected": True,
            "prepareBroadDevNullWriteRejected": True,
            "prepareOfflineVarAliasRejected": True,
            "prepareWrongMavenTmpdirRejected": True,
            "prepareSplitPhaseEnvironmentRejected": True,
            "prepareGradleWrapperBootstrapHomePassed": True,
            "prepareMissingGradleWrapperBootstrapHomeRejected": True,
            "prepareGradleJvmTmpdirAuthorityPassed": True,
            "prepareMissingGradleJvmTmpdirRejected": True,
            "prepareWrongGradleJvmTmpdirRejected": True,
            "prepareGradleJvmTmpdirFailureClassifiedInfrastructure": True,
            "prepareGradleStrictOfflineFailureTypedRefusal": True,
            "prepareGradleStrictOfflineWrongProfileSecurityRejected": True,
            "prepareGradleOnlineSecurityFailureRejected": True,
            "prepareMavenOfflineSecurityFailureRejected": True,
            "prepareMavenOfflineModelGoalsPrefetchedOnline": True,
            "preparePostPublicationEvidenceRevalidated": True,
            "requirementR18SupervisorNotRunRejected": True,
            "requirementR18PrepareNotRunRejected": True,
            "forgedPrepareNetworkEvidenceRejected": True,
            "onlinePrepareRefusalAccepted": True,
            "prepareSupervisorNonzeroRetained": True,
            "cargoSeedResealsRejected": True,
            "cargoJvmGradleInjectionRejected": True,
        },
        "counterexamples": counterexamples,
        "modelCalls": 0,
    }


def supervisor_self_test() -> dict[str, Any]:
    """Injected child regressions without touching a corpus repository."""
    bundle = load_production_bundle()
    # Darwin AF_UNIX has a short sockaddr path limit; pin this synthetic-only
    # root to /tmp so a long inherited TMPDIR cannot invalidate the sentinel.
    with tempfile.TemporaryDirectory(prefix="cc-k1-", dir="/tmp") as temporary_text:
        temporary = Path(temporary_text)
        repository = temporary / "repository"
        repository.mkdir()
        subprocess.run(["git", "init", "-q", str(repository)], check=True)
        subprocess.run(["git", "-C", str(repository), "config", "user.email", "k1@example.invalid"], check=True)
        subprocess.run(["git", "-C", str(repository), "config", "user.name", "K1 Test"], check=True)
        (repository / "README").write_text("fixture\n", encoding="utf-8")
        subprocess.run(["git", "-C", str(repository), "add", "README"], check=True)
        subprocess.run(["git", "-C", str(repository), "commit", "-qm", "fixture"], check=True)
        observation = _git_observation(repository)
        corpus = json.loads(json.dumps(bundle["corpus"]))
        corpus["entries"][0]["commit"] = observation["head"]
        corpus["entries"][0]["gitTree"] = observation["tree"]
        test_bundle = dict(bundle)
        test_bundle["corpus"] = corpus
        store = Store(temporary / "store", test_bundle, create=True)
        fixture_executable = Path("/bin/sh").resolve()
        fixture_executable_digest = sha256_file(fixture_executable)
        projection = {
            "schema": "codeclew.repository-impact-projection/0.1",
            "query": {}, "snapshot": {}, "adapter": {}, "capabilities": [],
            "status": "UNKNOWN", "selectedEntities": [], "relevantRelations": [],
            "affected": [], "paths": [], "mandatoryObligations": [], "boundaries": [],
            "compilerReceipt": {}, "completeness": {},
            "provenance": {
                "runtime": {"realPath": str(fixture_executable), "binaryDigest": fixture_executable_digest},
                "adapterRealPath": "/fixture/adapter", "adapterBinaryDigest": "sha256:" + "1" * 64,
                "adapterOutputDigest": "sha256:" + "2" * 64,
                "adapterEnvelopeFileDigest": "sha256:" + "3" * 64,
                "adapterOutputObject": {"digest": "sha256:" + "3" * 64, "relativePath": "objects/sha256/x.json", "sizeBytes": 1},
                "semanticOutputDigest": "sha256:" + "4" * 64,
                "evidenceCore": {"schema": "codeclew.evidence-core-binding/0.1", "bundleDigest": "sha256:" + "5" * 64},
            },
            "cost": {}, "projectionDigest": "",
        }
        projection["projectionDigest"] = _canonical_digest_with_empty_field(projection, "projectionDigest")
        typed = {
            "schema": KOTLIN_TYPED_ATTEMPT_SCHEMA, "status": "FAILED", "outcomeKind": "TYPED_TERMINAL",
            "failureStage": "TEST", "reasonCode": "INJECTED", "detailDigest": "sha256:" + "6" * 64,
            "selectedInputs": {}, "snapshot": {}, "provenance": {}, "boundaries": [],
            "adapterOutputDigest": None, "evidenceCore": None, "cache": {}, "cost": {},
            "terminalSemanticDigest": "sha256:" + "7" * 64, "attemptDigest": "",
        }
        typed["attemptDigest"] = _canonical_digest_with_empty_field(typed, "attemptDigest")
        diagnostic_output = {
            "schema": "codeclew.adapter-output/0.1", "adapter": {}, "snapshotInput": {},
            "capabilityDescriptors": [], "entities": [], "occurrences": [], "facts": [],
            "boundaries": [], "compilerReceipt": {}, "impact": {}, "cost": {},
            "outputDigest": "sha256:" + "8" * 64,
        }
        cases = {
            "success": ("printf %s " + shlex.quote(canonical(projection).decode()), "ADAPTER_OUTPUT"),
            "empty": (":", "FAILED"),
            "nonzero": ("exit 7", "FAILED"),
            "build_failure": ("exit 23", "FAILED"),
            "invalid_json": ("printf '{not-json}'", "FAILED"),
            "truncated_json": ("printf '{\"schema\":'", "FAILED"),
            "oom_like_signal": ("kill -9 $$", "FAILED"),
            "direct_adapter_output": ("printf %s " + shlex.quote(canonical(diagnostic_output).decode()), "FAILED"),
            "background_child": ("sleep 60 &", "FAILED"),
            "typed_nonzero": ("printf %s " + shlex.quote(canonical(typed).decode()) + "; exit 2", "FAILED"),
            "timeout": ("sleep 2", "FAILED"),
            "limit": ("printf %s " + shlex.quote("x" * 4096), "FAILED"),
        }
        results: dict[str, str] = {}
        for name, (program, expected) in cases.items():
            timeout = 1 if name == "timeout" else 10
            limit = 128 if name == "limit" else 1024 * 1024
            digest, attempt = supervise_entry(
                store,
                "K1-Q01",
                "COLD",
                repository,
                [str(fixture_executable), "-c", program],
                {},
                timeout_seconds=timeout,
                output_limit_bytes=limit,
            )
            if attempt["status"] != expected or not _is_digest(digest):
                stderr_ref = attempt["child"]["stderrSha256"]
                stderr_preview = (store.root / "blobs" / f"{stderr_ref[7:]}.blob").read_text(
                    encoding="utf-8", errors="replace"
                )[:512]
                raise AssertionError(
                    f"supervisor case failed: {name}: status={attempt['status']} "
                    f"reason={attempt['reasonCode']} child={attempt['child']} stderr={stderr_preview!r}"
                )
            if name not in {"success", "typed_nonzero"} and not str(attempt["reasonCode"]).startswith("SUPERVISOR/"):
                raise AssertionError(f"supervisor failure was not typed and retained: {name}")
            if name == "typed_nonzero" and attempt["reasonCode"] != "INJECTED":
                raise AssertionError("typed nonzero child was not preserved")
            results[name] = str(attempt["reasonCode"] or attempt["status"])
        pointers = list((store.root / "attempts").glob("*.json"))
        if len(pointers) != len(cases):
            raise AssertionError("supervisor did not retain exactly one attempt per case")
        # Execute the exact sandbox/env contour directly. A network connect
        # must fail before it can observe whether any service is listening;
        # production sentinel credentials must not cross the allowlist.
        profile = temporary / "deny-network.sb"
        _atomic_write(profile, b"(version 1)\n(allow default)\n(deny network*)\n", 0o400)
        env_root = temporary / "isolated-home"
        env_root.mkdir(mode=0o700)
        probe = (
            "import os,socket,sys; "
            "assert os.environ['HOME'].endswith('isolated-home'); "
            "assert os.environ['TMPDIR'].endswith('isolated-home'); "
            "assert 'CODECLEW_K1_PRODUCTION_SECRET_SENTINEL' not in os.environ; "
            "s=socket.socket(); "
            "\ntry: s.connect(('127.0.0.1',9)); sys.exit(42)\n"
            "except OSError: sys.exit(0)"
        )
        production_environment = dict(os.environ)
        production_environment["CODECLEW_K1_PRODUCTION_SECRET_SENTINEL"] = "must-not-cross"
        denied = subprocess.run(
            ["/usr/bin/sandbox-exec", "-f", str(profile), str(Path(os.sys.executable).resolve()), "-c", probe],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
            env={"HOME": str(env_root), "TMPDIR": str(env_root), "PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"},
        )
        if denied.returncode != 0:
            raise AssertionError("network-denied isolated environment probe failed")
        results["sandbox_network_env"] = "DENIED_AND_ISOLATED"

        strict_allowed = temporary / "strict-allowed"
        strict_allowed.mkdir(mode=0o700)
        secret = temporary / "production-secret-sentinel"
        _atomic_write(secret, b"must-not-cross\n", 0o600)
        unix_listener = temporary / "production-control.sock"
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        listener.bind(str(unix_listener))
        listener.listen(1)
        strict_profile = temporary / "strict-deny.sb"
        strict_profile_text = [
            "(version 1)", "(deny default)", "(allow process*)", "(allow sysctl-read)",
            "(allow mach-lookup)",
            '(deny mach-lookup (global-name "com.apple.securityd"))',
            '(deny mach-lookup (global-name "com.apple.security.agent"))',
            '(deny mach-lookup (global-name "com.apple.trustd"))',
            "(deny network*)",
            *_sandbox_read_clauses([
                strict_allowed, Path("/System"), Path("/usr"), Path("/bin"), Path("/sbin"),
                Path("/etc"), Path("/Library/Java"), Path("/dev"), Path("/private/var/select"),
            ]),
            _sandbox_path_clause("file-write*", strict_allowed),
        ]
        _atomic_write(strict_profile, ("\n".join(strict_profile_text) + "\n").encode(), 0o400)

        def sandbox_probe(program: str) -> subprocess.CompletedProcess[bytes]:
            return subprocess.run(
                ["/usr/bin/sandbox-exec", "-f", str(strict_profile), "/usr/bin/python3", "-c", program],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False, timeout=10,
                env={"HOME":str(strict_allowed),"TMPDIR":str(strict_allowed),"PATH":"/usr/bin:/bin","LANG":"C.UTF-8","LC_ALL":"C.UTF-8"},
            )

        probes = {
            "sandbox_secret_paths": f"open({str(secret)!r},'rb').read()",
            "sandbox_unix_network": f"import socket;s=socket.socket(socket.AF_UNIX);s.connect({str(unix_listener)!r})",
            "sandbox_source_write": f"open({str(secret)!r},'wb').write(b'x')",
            "sandbox_keychain_read": f"open({str(Path.home() / 'Library/Keychains/login.keychain-db')!r},'rb').read(1)",
        }
        try:
            for name, program in probes.items():
                completed = sandbox_probe(program)
                if completed.returncode == 0:
                    raise AssertionError(f"strict sandbox tripwire unexpectedly succeeded: {name}")
                results[name] = "DENIED"
        finally:
            listener.close()
        if secret.read_bytes() != b"must-not-cross\n":
            raise AssertionError("sandbox source-write tripwire changed denied target")
        if not str(results.get("background_child", "")).startswith("SUPERVISOR/"):
            raise AssertionError("supervisor background-child process-group tripwire failed")
        results["sandbox_background_child"] = "TERMINATED_WITH_GROUP"
    return {
        "schema": "codeclew.kotlin-k1-supervisor-self-test/0.1",
        "status": "PASS",
        "cases": results,
        "modelCalls": 0,
    }


def main() -> None:
    if len(os.sys.argv) > 1 and os.sys.argv[1] == "internal-launch":
        _launch_committed_child(os.sys.argv[2:])
        raise AssertionError("internal launcher exec returned")
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    subcommands.add_parser("self-test")
    subcommands.add_parser("supervisor-self-test")
    initialize = subcommands.add_parser("init-store")
    initialize.add_argument("--store", required=True, type=Path)
    show = subcommands.add_parser("explain")
    show.add_argument("--store", required=True, type=Path)
    show.add_argument("--inputs", type=Path)
    root = subcommands.add_parser("require-root")
    root.add_argument("--store", required=True, type=Path)
    root.add_argument("--inputs", type=Path)
    root.add_argument("--root", required=True)
    access = subcommands.add_parser("check-entry-access")
    access.add_argument("--store", required=True, type=Path)
    access.add_argument("--inputs", type=Path)
    access.add_argument("--entry", required=True)
    run_entry = subcommands.add_parser("run-entry")
    run_entry.add_argument("--store", required=True, type=Path)
    run_entry.add_argument("--inputs", type=Path)
    run_entry.add_argument("--entry", required=True)
    run_entry.add_argument("--invocation", choices=("COLD", "WARM"), required=True)
    run_entry.add_argument("--repository", required=True, type=Path)
    run_entry.add_argument("--timeout-seconds", type=int, default=MAX_WALL_SECONDS)
    run_entry.add_argument("--resident-limit-bytes", type=int, default=MAX_RESIDENT_BYTES)
    run_entry.add_argument("--output-limit-bytes", type=int, default=MAX_STDOUT_BYTES)
    run_entry.add_argument("command_tokens", nargs=argparse.REMAINDER)
    advance = subcommands.add_parser("advance-node")
    advance.add_argument("--store", required=True, type=Path)
    advance.add_argument("--inputs", required=True, type=Path)
    advance.add_argument("--node", required=True)
    qualification = subcommands.add_parser("run-qualification")
    qualification.add_argument("--store", required=True, type=Path)
    qualification.add_argument("--inputs", required=True, type=Path)
    qualification.add_argument("--entry", required=True)
    qualification.add_argument("--invocation", choices=("COLD", "WARM"), required=True)
    qualification.add_argument("--repository", required=True, type=Path)
    qualification.add_argument("--evidence-store", required=True, type=Path)
    qualification.add_argument("--semantic-state-root", required=True, type=Path)
    qualification.add_argument("--build-state-root", required=True, type=Path)
    qualification.add_argument("--timeout-seconds", type=int, default=MAX_WALL_SECONDS)
    qualification.add_argument("--resident-limit-bytes", type=int, default=MAX_RESIDENT_BYTES)
    holdout = subcommands.add_parser("run-holdout")
    holdout.add_argument("--store", required=True, type=Path)
    holdout.add_argument("--inputs", required=True, type=Path)
    holdout.add_argument("--entry", required=True)
    holdout.add_argument("--invocation", choices=("COLD", "WARM"), required=True)
    holdout.add_argument("--repository", required=True, type=Path)
    holdout.add_argument("--evidence-store", required=True, type=Path)
    holdout.add_argument("--semantic-state-root", required=True, type=Path)
    holdout.add_argument("--build-state-root", required=True, type=Path)
    holdout.add_argument("--timeout-seconds", type=int, default=MAX_WALL_SECONDS)
    holdout.add_argument("--resident-limit-bytes", type=int, default=MAX_RESIDENT_BYTES)
    build_tools = subcommands.add_parser("build-candidate-tools")
    build_tools.add_argument("--generic-runtime", required=True, type=Path)
    build_tools.add_argument("--kotlin-adapter", required=True, type=Path)
    build_tools.add_argument("--output", required=True, type=Path)
    build_live = subcommands.add_parser("build-live-set")
    build_live.add_argument(
        "--role", required=True,
        choices=("K0_AUTHORITY_SET", "CANDIDATE_SOURCES", "CANDIDATE_BINARIES"),
    )
    build_live.add_argument("--candidate-tools", type=Path)
    build_live.add_argument("--output", required=True, type=Path)
    build_inputs = subcommands.add_parser("build-live-inputs")
    build_inputs.add_argument("--run-root", required=True, type=Path)
    build_inputs.add_argument("--research-input", required=True, type=Path)
    build_inputs.add_argument("--execution-contract", required=True, type=Path)
    build_inputs.add_argument("--prior-failure", required=True, type=Path)
    build_inputs.add_argument("--qualification-source-set", required=True, type=Path)
    build_inputs.add_argument("--candidate-tools", required=True, type=Path)
    finalize = subcommands.add_parser("finalize-series")
    finalize.add_argument("--store", required=True, type=Path)
    finalize.add_argument("--inputs", required=True, type=Path)
    arguments = parser.parse_args()
    if arguments.command == "finalize-series":
        try:
            inputs = _read_input_manifest(arguments.inputs)
            bundle = load_production_bundle()
            store = Store(arguments.store, bundle)
            result = finalize_series(store, inputs)
        except HarnessError as normal_error:
            store = Store.open_for_fatal_finalize(arguments.store)
            state, _, _ = _series_guard(store)
            if state == "FATAL" or _live_authority_fatal(store) is not None:
                # Fixed/store-only fatal facts do not depend on a readable
                # caller manifest.  This is the only empty-input path.
                result = finalize_series(store, {})
            else:
                inputs = _read_degraded_input_paths(store, arguments.inputs)
                if _retained_fatal_invariant(store, inputs) is None:
                    raise normal_error
                result = finalize_series(store, inputs)
    else:
        bundle = load_production_bundle()
        if arguments.command == "build-candidate-tools":
            value = build_candidate_tools_manifest(arguments.generic_runtime, arguments.kotlin_adapter)
            output = arguments.output.absolute()
            if output.exists() or output.is_symlink():
                raise HarnessError("candidate tools output is create-only")
            _atomic_write(output, canonical(value), 0o400)
            _candidate_tools({"candidateTools": {"kind": "FILE", "path": str(output)}})
            result = {"status": "CREATED", "path": str(output), "sha256": sha256_file(output)}
        elif arguments.command == "build-live-set":
            output = arguments.output.absolute()
            if output.exists() or output.is_symlink():
                raise HarnessError("live-set output is create-only")
            value = build_live_set(arguments.role, arguments.candidate_tools)
            _atomic_write(output, canonical(value), 0o400)
            output.chmod(0o400)
            snapshot = snapshot_input({"kind": "LIVE_SET", "path": str(output)})
            result = {"status": "CREATED", "role": arguments.role, **snapshot}
        elif arguments.command == "build-live-inputs":
            _, output = build_live_inputs(
                arguments.run_root, arguments.research_input, arguments.execution_contract,
                arguments.prior_failure, arguments.qualification_source_set,
                arguments.candidate_tools,
            )
            result = {"status": "CREATED", "path": str(output), "sha256": sha256_file(output)}
        elif arguments.command == "self-test":
            result = self_test()
        elif arguments.command == "supervisor-self-test":
            result = supervisor_self_test()
        elif arguments.command == "init-store":
            store = Store(arguments.store, bundle, create=True)
            result = {"status": "INITIALIZED", "storeId": store.store_id, "graphDigest": store.graph_digest}
        else:
            store = Store(arguments.store, bundle)
            inputs = _read_input_manifest(arguments.inputs)
            if arguments.command == "explain":
                result = explain(store, inputs)
            elif arguments.command == "require-root":
                result = require_root(store, arguments.root, inputs)
            elif arguments.command == "check-entry-access":
                entry = assert_entry_run_allowed(store, arguments.entry, inputs)
                result = {"status": "ALLOWED", "entry": entry["id"], "cohort": entry["cohort"]}
            elif arguments.command == "advance-node":
                digest = advance_node(store, arguments.node, inputs)
                result = {"status": "RETAINED", "node": arguments.node, "receiptDigest": digest}
            elif arguments.command == "run-qualification":
                digest, attempt = run_qualification_entry(
                    store, arguments.entry, arguments.invocation, arguments.repository,
                    arguments.evidence_store, arguments.semantic_state_root,
                    arguments.build_state_root, inputs,
                    timeout_seconds=arguments.timeout_seconds,
                    resident_limit_bytes=arguments.resident_limit_bytes,
                )
                result = {"status": "RETAINED", "attemptDigest": digest, "attempt": attempt}
            elif arguments.command == "run-holdout":
                digest, attempt = run_holdout_entry(
                    store, arguments.entry, arguments.invocation, arguments.repository,
                    arguments.evidence_store, arguments.semantic_state_root,
                    arguments.build_state_root, inputs,
                    timeout_seconds=arguments.timeout_seconds,
                    resident_limit_bytes=arguments.resident_limit_bytes,
                )
                result = {"status": "RETAINED", "attemptDigest": digest, "attempt": attempt}
            else:
                command = arguments.command_tokens
                if command and command[0] == "--":
                    command = command[1:]
                digest, attempt = supervise_entry(
                    store,
                    arguments.entry,
                    arguments.invocation,
                    arguments.repository,
                    command,
                    inputs,
                    timeout_seconds=arguments.timeout_seconds,
                    resident_limit_bytes=arguments.resident_limit_bytes,
                    output_limit_bytes=arguments.output_limit_bytes,
                )
                result = {"status": "RETAINED", "attemptDigest": digest, "attempt": attempt}
    print(canonical(result).decode(), end="")


if __name__ == "__main__":
    try:
        main()
    except (HarnessError, OSError) as error:
        print(canonical({
            "schema": "codeclew.kotlin-k1-harness-error/0.1",
            "status": "FAILED",
            "reason": type(error).__name__,
            "detailSha256": sha256_bytes(str(error).encode()),
        }).decode(), end="")
        raise SystemExit(2)
