#!/usr/bin/env python3
"""Verify the path-free public result of the frozen Kotlin descriptor pilot."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tempfile
from pathlib import Path
from typing import Any


SCHEMA = "codeclew-thread-kotlin-descriptor-pilot/1.0"
EXPECTED_PRIVATE_CORPUS_DIGEST = (
    "sha256:7b49161fb1c1c322c47b318f002d7ea9ae9efb024e7d7be33a7427295668969c"
)
EXPECTED_BENCHMARK_DIGEST = (
    "sha256:0793f3020fb3b58cce97d78598f8c75944a2ffa29bd72bf061b86d6e4ee0a54c"
)
MAX_EVIDENCE_BYTES = 256 * 1024
SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")
ABSOLUTE_PATH = re.compile(r"^(?:/|~[/\\]|[A-Za-z]:[/\\])")
EXPECTED_TASKS = [f"task-{index:02}" for index in range(1, 11)]
EXPECTED_PAIRS = [
    "pair-01", "pair-02", "pair-03", "pair-04", "pair-05",
    "pair-06", "pair-07", "pair-08", "pair-01", "pair-05",
]
EXPECTED_MANUAL_COUNTS = [8, 8, 8, 8, 8, 8, 8, 8, 5, 5]
CRITERIA = {
    "exactAuthority",
    "approvedFileBothSides",
    "callableNavigation",
    "typeNavigation",
    "boundedSourceEvidence",
    "completeManualVerification",
    "zeroFalseExactClaims",
    "resourceBudgetsPass",
}
ARM_FIELDS = {
    "result",
    "criteria",
    "declaredMemberHits",
    "top10RelevantFileHits",
    "descriptorSlotHits",
    "manualCategoryExpectedCount",
    "manualCategoryHits",
    "falseExactClaimCount",
    "elapsedMillis",
    "openedSourceBytes",
    "openedSourceFiles",
    "toolStarts",
    "noncachedInputTokens",
    "queryTerms",
    "returnedFacts",
    "sourceWindows",
    "agentVisibleEvidenceBytes",
    "answerBytes",
    "contextCreates",
    "contextExpansions",
    "maxSemanticCommandMillis",
    "selectedFiles",
    "sourceEvidenceSideCount",
    "declaredTopologyBound",
    "capabilityViolations",
    "budgetRefusals",
    "semanticContextCommands",
    "semanticCallablesCommands",
    "semanticImpactCommands",
}
AGGREGATE_FIELDS = {
    "taskPassCount",
    "declaredMemberHitCount",
    "top10RelevantFileHitCount",
    "descriptorSlotHitCount",
    "manualCategoryHitCount",
    "falseExactClaimCount",
    "totalElapsedMillis",
    "totalOpenedSourceBytes",
    "totalOpenedSourceFiles",
    "totalToolStarts",
    "totalNoncachedInputTokens",
}
FAILED_GATES = {
    "BENCHMARK_AGGREGATE",
    "CRITICAL_MEMBER_OMISSION",
    "DECLARED_MEMBER_RECALL",
    "FIXTURE_FALSE_EXACT",
    "FIXTURE_SHAPE_RECALL",
    "IMPLEMENTATION_REVIEW",
    "OPENED_FILE_NONINCREASE",
    "PUBLIC_PRIVACY",
    "TASK_PASS_NONREGRESSION",
    "TOKEN_REDUCTION",
    "TOP10_FILE_RECALL",
    "VALUE_REVIEW",
    "WARM_CACHE_ACCESS",
    "WARM_NETWORK",
    "WARM_P95",
    "WARM_PROHIBITED_PROCESS",
    "WARM_RUN_COUNT",
    "WARM_STDOUT_IDENTITY",
}


class EvidenceError(ValueError):
    """The checked pilot evidence is malformed or internally inconsistent."""


def canonical_bytes(value: Any) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")


def authority_digest(value: Any) -> str:
    return f"sha256:{hashlib.sha256(canonical_bytes(value)).hexdigest()}"


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise EvidenceError("duplicate JSON key")
        result[key] = value
    return result


def require_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != expected:
        raise EvidenceError(f"{label} fields are not closed")
    return value


def require_int(value: Any, label: str, minimum: int = 0) -> int:
    if type(value) is not int or value < minimum:
        raise EvidenceError(f"{label} is not a bounded integer")
    return value


def require_bool(value: Any, label: str) -> bool:
    if type(value) is not bool:
        raise EvidenceError(f"{label} is not boolean")
    return value


def require_digest(value: Any, label: str) -> str:
    if not isinstance(value, str) or SHA256.fullmatch(value) is None:
        raise EvidenceError(f"{label} is not a canonical digest")
    return value


def require_safe_strings(value: Any) -> None:
    if isinstance(value, str):
        if len(value.encode("utf-8")) > 256 or ABSOLUTE_PATH.match(value):
            raise EvidenceError("public evidence contains an unsafe string")
    elif isinstance(value, list):
        for item in value:
            require_safe_strings(item)
    elif isinstance(value, dict):
        for key, item in value.items():
            require_safe_strings(key)
            require_safe_strings(item)


def _resource_pass(row: dict[str, Any]) -> bool:
    return (
        row["elapsedMillis"] <= 600_000
        and row["noncachedInputTokens"] <= 40_000
        and row["toolStarts"] <= 40
        and row["queryTerms"] <= 16
        and row["returnedFacts"] <= 128
        and row["selectedFiles"] <= 12
        and row["selectedFiles"] >= row["openedSourceFiles"]
        and row["sourceWindows"] <= 24
        and row["agentVisibleEvidenceBytes"] <= 8 * 1024 * 1024
        and row["answerBytes"] <= 65_536
        and row["contextCreates"] <= 1
        and row["contextExpansions"] <= 1
        and row["maxSemanticCommandMillis"] <= 60_000
        and row["capabilityViolations"] == 0
        and row["budgetRefusals"] == 0
    )


def _verify_arm(
    value: Any, label: str, manual_count: int, *, arm: str
) -> dict[str, Any]:
    row = require_keys(value, ARM_FIELDS, label)
    if row["result"] not in {"PASS", "FAIL"}:
        raise EvidenceError(f"{label} result is invalid")
    criteria = require_keys(row["criteria"], CRITERIA, f"{label} criteria")
    for name in sorted(CRITERIA):
        require_bool(criteria[name], f"{label} {name}")
    require_bool(row["declaredTopologyBound"], f"{label} declaredTopologyBound")
    for name in ARM_FIELDS - {"result", "criteria", "declaredTopologyBound"}:
        require_int(row[name], f"{label} {name}")
    if not 0 <= row["declaredMemberHits"] <= 2:
        raise EvidenceError(f"{label} declared-member count is invalid")
    if not 0 <= row["top10RelevantFileHits"] <= 2:
        raise EvidenceError(f"{label} relevant-file count is invalid")
    if not 0 <= row["descriptorSlotHits"] <= 2:
        raise EvidenceError(f"{label} descriptor-slot count is invalid")
    if not 0 <= row["sourceEvidenceSideCount"] <= 2:
        raise EvidenceError(f"{label} source-evidence side count is invalid")
    if (
        row["manualCategoryExpectedCount"] != manual_count
        or row["manualCategoryHits"] > manual_count
    ):
        raise EvidenceError(f"{label} manual-category count is invalid")
    if arm == "DEFAULT" and (
        row["contextCreates"] != 0
        or row["contextExpansions"] != 0
        or row["maxSemanticCommandMillis"] != 0
        or row["semanticContextCommands"] != 0
        or row["semanticCallablesCommands"] != 0
        or row["semanticImpactCommands"] != 0
    ):
        raise EvidenceError(f"{label} used a Codeclew-only capability")

    expected_criteria = {
        "exactAuthority": (
            row["declaredMemberHits"] == 2 and row["declaredTopologyBound"]
            and (
                arm == "DEFAULT"
                or (
                    row["semanticContextCommands"] == 1
                    and row["semanticCallablesCommands"] == 1
                    and row["semanticImpactCommands"] >= 1
                )
            )
        ),
        "approvedFileBothSides": row["top10RelevantFileHits"] == 2,
        "callableNavigation": criteria["callableNavigation"],
        "typeNavigation": criteria["typeNavigation"],
        "boundedSourceEvidence": (
            row["sourceEvidenceSideCount"] == 2
            and 0 < row["openedSourceBytes"] <= 8 * 1024 * 1024
            and 0 < row["openedSourceFiles"]
            and row["selectedFiles"] <= 12
            and 0 < row["sourceWindows"] <= 24
            and row["openedSourceFiles"] >= row["sourceEvidenceSideCount"]
            and row["agentVisibleEvidenceBytes"] >= row["openedSourceBytes"]
        ),
        "completeManualVerification": row["manualCategoryHits"] == manual_count,
        "zeroFalseExactClaims": row["falseExactClaimCount"] == 0,
        "resourceBudgetsPass": _resource_pass(row),
    }
    for name in CRITERIA - {"callableNavigation", "typeNavigation"}:
        if criteria[name] != expected_criteria[name]:
            raise EvidenceError(f"{label} {name} is inconsistent")
    expected_slots = int(criteria["callableNavigation"]) + int(criteria["typeNavigation"])
    if row["descriptorSlotHits"] != expected_slots:
        raise EvidenceError(f"{label} descriptor slots are inconsistent")
    expected_result = "PASS" if all(criteria.values()) else "FAIL"
    if row["result"] != expected_result:
        raise EvidenceError(f"{label} task result is inconsistent")
    return row


def _aggregate(rows: list[dict[str, Any]]) -> dict[str, int]:
    return {
        "taskPassCount": sum(row["result"] == "PASS" for row in rows),
        "declaredMemberHitCount": sum(row["declaredMemberHits"] for row in rows),
        "top10RelevantFileHitCount": sum(row["top10RelevantFileHits"] for row in rows),
        "descriptorSlotHitCount": sum(row["descriptorSlotHits"] for row in rows),
        "manualCategoryHitCount": sum(row["manualCategoryHits"] for row in rows),
        "falseExactClaimCount": sum(row["falseExactClaimCount"] for row in rows),
        "totalElapsedMillis": sum(row["elapsedMillis"] for row in rows),
        "totalOpenedSourceBytes": sum(row["openedSourceBytes"] for row in rows),
        "totalOpenedSourceFiles": sum(row["openedSourceFiles"] for row in rows),
        "totalToolStarts": sum(row["toolStarts"] for row in rows),
        "totalNoncachedInputTokens": sum(row["noncachedInputTokens"] for row in rows),
    }


def _verify_aggregate(value: Any, expected: dict[str, int], label: str) -> None:
    aggregate = require_keys(value, AGGREGATE_FIELDS, label)
    for key, expected_value in expected.items():
        require_int(aggregate[key], f"{label} {key}")
        if aggregate[key] != expected_value:
            raise EvidenceError(f"{label} is inconsistent")


def _middle_pair(rows: list[dict[str, Any]], field: str) -> tuple[int, int]:
    values = sorted(row[field] for row in rows)
    return values[4], values[5]


def _expected_failed_gates(
    evidence: dict[str, Any],
    default_rows: list[dict[str, Any]],
    codeclew_rows: list[dict[str, Any]],
) -> tuple[list[str], bool, bool]:
    fixture = evidence["fixture"]
    comparison = evidence["comparison"]
    warm = evidence["warmAudit"]
    verdict = evidence["verdict"]
    privacy = evidence["privacy"]
    default = _aggregate(default_rows)
    codeclew = _aggregate(codeclew_rows)
    failures: list[str] = []
    if fixture["matchedShapeCount"] != fixture["expectedShapeCount"]:
        failures.append("FIXTURE_SHAPE_RECALL")
    if fixture["falseExactClaimCount"] != 0:
        failures.append("FIXTURE_FALSE_EXACT")
    if 10 * codeclew["declaredMemberHitCount"] < 9 * comparison["declaredMemberDenominator"]:
        failures.append("DECLARED_MEMBER_RECALL")
    if codeclew["declaredMemberHitCount"] != comparison["declaredMemberDenominator"]:
        failures.append("CRITICAL_MEMBER_OMISSION")
    if 5 * codeclew["top10RelevantFileHitCount"] < 4 * comparison["top10RelevantFileDenominator"]:
        failures.append("TOP10_FILE_RECALL")
    if codeclew["taskPassCount"] < default["taskPassCount"]:
        failures.append("TASK_PASS_NONREGRESSION")
    default_tokens = _middle_pair(default_rows, "noncachedInputTokens")
    codeclew_tokens = _middle_pair(codeclew_rows, "noncachedInputTokens")
    if 10 * sum(codeclew_tokens) > 7 * sum(default_tokens):
        failures.append("TOKEN_REDUCTION")
    default_files = _middle_pair(default_rows, "openedSourceFiles")
    codeclew_files = _middle_pair(codeclew_rows, "openedSourceFiles")
    if sum(codeclew_files) > sum(default_files):
        failures.append("OPENED_FILE_NONINCREASE")
    if warm["runCount"] != 30 or len(warm["samplesNanos"]) != 30:
        failures.append("WARM_RUN_COUNT")
    if warm["p95Nanos"] > 10_000_000_000:
        failures.append("WARM_P95")
    if not warm["stdoutByteIdentical"]:
        failures.append("WARM_STDOUT_IDENTITY")
    if not warm["networkDenied"]:
        failures.append("WARM_NETWORK")
    if warm["prohibitedProcessCount"] != 0:
        failures.append("WARM_PROHIBITED_PROCESS")
    if warm["cacheAccessCount"] != 0:
        failures.append("WARM_CACHE_ACCESS")
    if any(value != 0 for value in privacy.values()):
        failures.append("PUBLIC_PRIVACY")
    if verdict["implementationReview"] != "PASS":
        failures.append("IMPLEMENTATION_REVIEW")
    if verdict["valueReview"] != "PASS":
        failures.append("VALUE_REVIEW")
    benchmark_pass = codeclew["taskPassCount"] == comparison["taskDenominator"]
    if not benchmark_pass:
        failures.append("BENCHMARK_AGGREGATE")
    comparative_failures = set(failures) - {"BENCHMARK_AGGREGATE"}
    return sorted(failures), benchmark_pass, not comparative_failures


def verify_value(evidence: Any) -> dict[str, Any]:
    evidence = require_keys(
        evidence,
        {"schema", "status", "authority", "fixture", "comparison", "warmAudit", "verdict", "privacy"},
        "evidence",
    )
    require_safe_strings(evidence)
    if evidence["schema"] != SCHEMA or evidence["status"] not in {"PASS", "FAIL"}:
        raise EvidenceError("pilot schema or status is invalid")

    authority = require_keys(
        evidence["authority"],
        {
            "privateCorpusDigest", "benchmarkDigest", "protocolDigest",
            "shapeOracleDigest", "modelConfigurationDigest",
            "implementationReviewManifestDigest", "valueReviewManifestDigest",
        },
        "authority",
    )
    for key, value in authority.items():
        require_digest(value, key)
    if (
        authority["privateCorpusDigest"] != EXPECTED_PRIVATE_CORPUS_DIGEST
        or authority["benchmarkDigest"] != EXPECTED_BENCHMARK_DIGEST
    ):
        raise EvidenceError("frozen authority was substituted")

    fixture = require_keys(
        evidence["fixture"],
        {"expectedShapeCount", "matchedShapeCount", "falseExactClaimCount", "result"},
        "fixture",
    )
    for key in {"expectedShapeCount", "matchedShapeCount", "falseExactClaimCount"}:
        require_int(fixture[key], f"fixture {key}")
    if (
        fixture["expectedShapeCount"] != 5
        or fixture["matchedShapeCount"] > 5
        or fixture["result"]
        != ("PASS" if fixture["matchedShapeCount"] == 5 and fixture["falseExactClaimCount"] == 0 else "FAIL")
    ):
        raise EvidenceError("fixture result is inconsistent")

    comparison = require_keys(
        evidence["comparison"],
        {
            "taskDenominator", "declaredMemberDenominator",
            "top10RelevantFileDenominator", "descriptorSlotDenominator",
            "manualCategoryDenominator", "default", "codeclew", "taskResults",
        },
        "comparison",
    )
    if (
        comparison["taskDenominator"] != 10
        or comparison["declaredMemberDenominator"] != 20
        or comparison["top10RelevantFileDenominator"] != 20
        or comparison["descriptorSlotDenominator"] != 20
        or comparison["manualCategoryDenominator"] != 74
    ):
        raise EvidenceError("comparison denominators are not frozen")
    tasks = comparison["taskResults"]
    if not isinstance(tasks, list) or len(tasks) != 10:
        raise EvidenceError("task result cardinality is invalid")
    default_rows: list[dict[str, Any]] = []
    codeclew_rows: list[dict[str, Any]] = []
    for index, value in enumerate(tasks):
        task = require_keys(
            value, {"taskId", "pairId", "armOrder", "default", "codeclew"}, "task result"
        )
        if task["taskId"] != EXPECTED_TASKS[index] or task["pairId"] != EXPECTED_PAIRS[index]:
            raise EvidenceError("task authority or order changed")
        expected_order = ["DEFAULT", "CODECLEW"] if index % 2 == 0 else ["CODECLEW", "DEFAULT"]
        if task["armOrder"] != expected_order:
            raise EvidenceError("arm order is not the frozen alternating order")
        default_rows.append(
            _verify_arm(
                task["default"],
                f"{task['taskId']} default",
                EXPECTED_MANUAL_COUNTS[index],
                arm="DEFAULT",
            )
        )
        codeclew_rows.append(
            _verify_arm(
                task["codeclew"],
                f"{task['taskId']} codeclew",
                EXPECTED_MANUAL_COUNTS[index],
                arm="CODECLEW",
            )
        )
    _verify_aggregate(comparison["default"], _aggregate(default_rows), "default aggregate")
    _verify_aggregate(comparison["codeclew"], _aggregate(codeclew_rows), "codeclew aggregate")

    warm = require_keys(
        evidence["warmAudit"],
        {
            "runCount", "p95Rank", "samplesNanos", "p95Nanos",
            "stdoutByteIdentical", "networkDenied", "prohibitedProcessCount", "cacheAccessCount",
        },
        "warm audit",
    )
    require_int(warm["runCount"], "warm run count")
    require_int(warm["p95Rank"], "warm p95 rank")
    require_int(warm["p95Nanos"], "warm p95")
    for key in {"prohibitedProcessCount", "cacheAccessCount"}:
        require_int(warm[key], f"warm {key}")
    for key in {"stdoutByteIdentical", "networkDenied"}:
        require_bool(warm[key], f"warm {key}")
    if not isinstance(warm["samplesNanos"], list) or any(
        type(sample) is not int or sample < 0 for sample in warm["samplesNanos"]
    ):
        raise EvidenceError("warm samples are invalid")
    if warm["p95Rank"] != 29 or (
        len(warm["samplesNanos"]) == 30
        and warm["p95Nanos"] != sorted(warm["samplesNanos"])[28]
    ):
        raise EvidenceError("warm p95 accounting is inconsistent")

    privacy = require_keys(
        evidence["privacy"],
        {"absolutePathCount", "sourceBodyCount", "privateIdentifierCount", "credentialMatchCount"},
        "privacy",
    )
    for key, value in privacy.items():
        require_int(value, f"privacy {key}")
    verdict = require_keys(
        evidence["verdict"],
        {
            "benchmarkAggregatePass", "s4ComparativeGatePass",
            "implementationReview", "valueReview", "qualifiedAdoption",
            "qualification", "failedGates",
        },
        "verdict",
    )
    for key in {"benchmarkAggregatePass", "s4ComparativeGatePass", "qualifiedAdoption"}:
        require_bool(verdict[key], f"verdict {key}")
    if verdict["implementationReview"] not in {"PASS", "FAIL"} or verdict["valueReview"] not in {"PASS", "FAIL"}:
        raise EvidenceError("review verdict is invalid")
    if (
        not isinstance(verdict["failedGates"], list)
        or verdict["failedGates"] != sorted(set(verdict["failedGates"]))
        or any(gate not in FAILED_GATES for gate in verdict["failedGates"])
    ):
        raise EvidenceError("failed-gate projection is invalid")
    failures, benchmark_pass, comparative_pass = _expected_failed_gates(
        evidence, default_rows, codeclew_rows
    )
    qualified = benchmark_pass and comparative_pass
    if (
        verdict["failedGates"] != failures
        or verdict["benchmarkAggregatePass"] != benchmark_pass
        or verdict["s4ComparativeGatePass"] != comparative_pass
        or verdict["qualifiedAdoption"] != qualified
        or evidence["status"] != ("PASS" if qualified else "FAIL")
        or verdict["qualification"]
        != ("LOCAL_KOTLIN_STRUCTURAL_NAVIGATION_PILOT" if qualified else "NOT_QUALIFIED")
    ):
        raise EvidenceError("adoption verdict is inconsistent")
    return evidence


def verify(path: Path) -> dict[str, Any]:
    raw = path.read_bytes()
    if not 0 < len(raw) <= MAX_EVIDENCE_BYTES:
        raise EvidenceError("evidence size is invalid")
    try:
        value = json.loads(raw, object_pairs_hook=reject_duplicate_keys)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise EvidenceError("evidence is not canonical JSON") from error
    if raw != canonical_bytes(value) + b"\n":
        raise EvidenceError("evidence bytes are not canonical")
    return verify_value(value)


def _synthetic_arm(*, files: int, tokens: int) -> dict[str, Any]:
    return {
        "result": "PASS",
        "criteria": {name: True for name in CRITERIA},
        "declaredMemberHits": 2,
        "top10RelevantFileHits": 2,
        "descriptorSlotHits": 2,
        "manualCategoryExpectedCount": 8,
        "manualCategoryHits": 8,
        "falseExactClaimCount": 0,
        "elapsedMillis": 100,
        "openedSourceBytes": 1000,
        "openedSourceFiles": files,
        "toolStarts": 3,
        "noncachedInputTokens": tokens,
        "queryTerms": 2,
        "returnedFacts": 4,
        "sourceWindows": 2,
        "agentVisibleEvidenceBytes": 2000,
        "answerBytes": 1000,
        "contextCreates": 0,
        "contextExpansions": 0,
        "maxSemanticCommandMillis": 0,
        "selectedFiles": files,
        "sourceEvidenceSideCount": 2,
        "declaredTopologyBound": True,
        "capabilityViolations": 0,
        "budgetRefusals": 0,
        "semanticContextCommands": 0,
        "semanticCallablesCommands": 0,
        "semanticImpactCommands": 0,
    }


def synthetic_evidence() -> dict[str, Any]:
    tasks: list[dict[str, Any]] = []
    default_rows: list[dict[str, Any]] = []
    codeclew_rows: list[dict[str, Any]] = []
    for index, (task_id, pair_id, manual_count) in enumerate(
        zip(EXPECTED_TASKS, EXPECTED_PAIRS, EXPECTED_MANUAL_COUNTS, strict=True)
    ):
        default = _synthetic_arm(files=4, tokens=10_000)
        codeclew = _synthetic_arm(files=3, tokens=7_000)
        codeclew["semanticContextCommands"] = 1
        codeclew["semanticCallablesCommands"] = 1
        codeclew["semanticImpactCommands"] = 1
        default["manualCategoryExpectedCount"] = manual_count
        default["manualCategoryHits"] = manual_count
        codeclew["manualCategoryExpectedCount"] = manual_count
        codeclew["manualCategoryHits"] = manual_count
        default_rows.append(default)
        codeclew_rows.append(codeclew)
        tasks.append(
            {
                "taskId": task_id,
                "pairId": pair_id,
                "armOrder": ["DEFAULT", "CODECLEW"] if index % 2 == 0 else ["CODECLEW", "DEFAULT"],
                "default": default,
                "codeclew": codeclew,
            }
        )
    return {
        "schema": SCHEMA,
        "status": "PASS",
        "authority": {
            "privateCorpusDigest": EXPECTED_PRIVATE_CORPUS_DIGEST,
            "benchmarkDigest": EXPECTED_BENCHMARK_DIGEST,
            "protocolDigest": authority_digest("protocol"),
            "shapeOracleDigest": authority_digest("oracle"),
            "modelConfigurationDigest": authority_digest("model"),
            "implementationReviewManifestDigest": authority_digest("implementation-review"),
            "valueReviewManifestDigest": authority_digest("value-review"),
        },
        "fixture": {
            "expectedShapeCount": 5,
            "matchedShapeCount": 5,
            "falseExactClaimCount": 0,
            "result": "PASS",
        },
        "comparison": {
            "taskDenominator": 10,
            "declaredMemberDenominator": 20,
            "top10RelevantFileDenominator": 20,
            "descriptorSlotDenominator": 20,
            "manualCategoryDenominator": 74,
            "default": _aggregate(default_rows),
            "codeclew": _aggregate(codeclew_rows),
            "taskResults": tasks,
        },
        "warmAudit": {
            "runCount": 30,
            "p95Rank": 29,
            "samplesNanos": [1_000_000] * 30,
            "p95Nanos": 1_000_000,
            "stdoutByteIdentical": True,
            "networkDenied": True,
            "prohibitedProcessCount": 0,
            "cacheAccessCount": 0,
        },
        "verdict": {
            "benchmarkAggregatePass": True,
            "s4ComparativeGatePass": True,
            "implementationReview": "PASS",
            "valueReview": "PASS",
            "qualifiedAdoption": True,
            "qualification": "LOCAL_KOTLIN_STRUCTURAL_NAVIGATION_PILOT",
            "failedGates": [],
        },
        "privacy": {
            "absolutePathCount": 0,
            "sourceBodyCount": 0,
            "privateIdentifierCount": 0,
            "credentialMatchCount": 0,
        },
    }


def self_test() -> None:
    valid = synthetic_evidence()
    verify_value(valid)
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "pilot.json"
        path.write_bytes(canonical_bytes(valid) + b"\n")
        verify(path)
    tampered = json.loads(json.dumps(valid))
    tampered["comparison"]["codeclew"]["taskPassCount"] = 9
    try:
        verify_value(tampered)
    except EvidenceError:
        pass
    else:
        raise AssertionError("aggregate tamper was accepted")
    tampered = json.loads(json.dumps(valid))
    for task in tampered["comparison"]["taskResults"]:
        task["codeclew"]["noncachedInputTokens"] = 7_001
    tampered["comparison"]["codeclew"] = _aggregate(
        [row["codeclew"] for row in tampered["comparison"]["taskResults"]]
    )
    try:
        verify_value(tampered)
    except EvidenceError:
        pass
    else:
        raise AssertionError("false token verdict was accepted")
    tampered = json.loads(json.dumps(valid))
    tampered["authority"]["protocolDigest"] = "/private/path"
    try:
        verify_value(tampered)
    except EvidenceError:
        pass
    else:
        raise AssertionError("private path was accepted")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", nargs="?", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    try:
        if args.self_test:
            self_test()
            print(json.dumps({"schema": SCHEMA, "selfTest": "PASS"}, sort_keys=True))
            return 0
        if args.evidence is None:
            parser.error("evidence is required")
        value = verify(args.evidence)
    except (EvidenceError, OSError):
        print("FAIL: S4K_PILOT_EVIDENCE_INVALID", file=sys.stderr)
        return 1
    print(
        json.dumps(
            {
                "schema": SCHEMA,
                "verification": "PASS",
                "status": value["status"],
                "evidenceDigest": authority_digest(value),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
