#!/usr/bin/env python3
"""Verify the frozen, path-free OpenAPI corpus-fit gate evidence."""

from __future__ import annotations

import json
import re
import sys
from collections import Counter
from pathlib import Path
from typing import Any

MAX_EVIDENCE_BYTES = 256 * 1024
SCHEMA = "codeclew-thread-contract-corpus-gate/1.0"
SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")
SAFE_ID = re.compile(r"^[a-z][a-z0-9-]{0,63}$")
ABSOLUTE_PATH = re.compile(r"^(?:/|~[/\\]|[A-Za-z]:[/\\])")
CLASSIFICATIONS = {
    "EXACT_COMPARABLE",
    "DECLARED_NAVIGABLE",
    "NOT_USABLE_FOR_V1",
}
SCENARIOS = {
    "PROVIDER_CONTRACT_CHANGE",
    "CONSUMER_REQUEST_SHAPE",
    "PROVIDER_RESPONSE_SHAPE",
}
THRESHOLDS = {
    "exactComparableMin": 4,
    "usableMin": 6,
    "distinctServicePairsMin": 3,
}


class EvidenceError(ValueError):
    pass


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise EvidenceError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def require_keys(value: dict[str, Any], expected: set[str], label: str) -> None:
    if set(value) != expected:
        raise EvidenceError(f"{label} fields are not closed")


def require_safe_strings(value: Any) -> None:
    if isinstance(value, str):
        if len(value.encode("utf-8")) > 1024:
            raise EvidenceError("evidence string exceeds 1 KiB")
        if ABSOLUTE_PATH.match(value):
            raise EvidenceError("evidence contains an absolute path")
    elif isinstance(value, list):
        for item in value:
            require_safe_strings(item)
    elif isinstance(value, dict):
        for key, item in value.items():
            require_safe_strings(key)
            require_safe_strings(item)


def require_digest(value: Any, label: str) -> str:
    if not isinstance(value, str) or SHA256.fullmatch(value) is None:
        raise EvidenceError(f"{label} is not a sha256 authority")
    return value


def expected_classification(task: dict[str, Any]) -> str:
    provider = task["providerOpenApiArtifacts"]
    consumer = task["consumerOpenApiArtifacts"]
    declared = task["explicitDependencySeed"]
    if provider > 0 and consumer > 0:
        return "EXACT_COMPARABLE"
    if provider > 0 and consumer == 0 and declared:
        return "DECLARED_NAVIGABLE"
    return "NOT_USABLE_FOR_V1"


def verify(path: Path) -> dict[str, Any]:
    raw = path.read_bytes()
    if len(raw) > MAX_EVIDENCE_BYTES:
        raise EvidenceError("corpus evidence exceeds 256 KiB")
    try:
        evidence = json.loads(raw, object_pairs_hook=reject_duplicate_keys)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise EvidenceError("corpus evidence is not valid UTF-8 JSON") from error
    if not isinstance(evidence, dict):
        raise EvidenceError("corpus evidence root must be an object")
    require_safe_strings(evidence)
    require_keys(
        evidence,
        {
            "schema",
            "frozenAt",
            "selectionAuthority",
            "thresholds",
            "services",
            "tasks",
            "summary",
            "benchmarkSeals",
            "privacy",
        },
        "corpus evidence",
    )
    if evidence["schema"] != SCHEMA or evidence["frozenAt"] != "2026-08-25":
        raise EvidenceError("corpus schema or freeze date is invalid")

    selection = evidence["selectionAuthority"]
    if not isinstance(selection, dict):
        raise EvidenceError("selection authority must be an object")
    require_keys(
        selection,
        {"kind", "ruleId", "privateCorpusDigest", "taskCount"},
        "selection authority",
    )
    if (
        selection["kind"] != "PINNED_LOCAL_CANONICAL_HTTP_TOPOLOGY"
        or selection["ruleId"] != "LOCAL_REQUIRED_HTTP_EDGES_PLUS_TWO_FIXED_VARIANTS_V1"
        or selection["taskCount"] != 10
    ):
        raise EvidenceError("selection authority is invalid")
    require_digest(selection["privateCorpusDigest"], "private corpus digest")
    if evidence["thresholds"] != THRESHOLDS:
        raise EvidenceError("fit thresholds differ from the frozen gate")

    services = evidence["services"]
    if not isinstance(services, list) or len(services) != 11:
        raise EvidenceError("service authority set must contain eleven aliases")
    service_ids: set[str] = set()
    for service in services:
        if not isinstance(service, dict):
            raise EvidenceError("service authority must be an object")
        require_keys(service, {"serviceAlias", "revisionAuthority"}, "service authority")
        alias = service["serviceAlias"]
        if not isinstance(alias, str) or SAFE_ID.fullmatch(alias) is None:
            raise EvidenceError("service alias is unsafe")
        if alias in service_ids:
            raise EvidenceError("service alias is duplicated")
        service_ids.add(alias)
        require_digest(service["revisionAuthority"], "service revision authority")
    if [service["serviceAlias"] for service in services] != sorted(service_ids):
        raise EvidenceError("service authorities are not canonically ordered")

    tasks = evidence["tasks"]
    if not isinstance(tasks, list) or len(tasks) != 10:
        raise EvidenceError("corpus must contain exactly ten tasks")
    expected_task_ids = [f"task-{index:02}" for index in range(1, 11)]
    pair_bindings: dict[str, tuple[str, str]] = {}
    classifications: Counter[str] = Counter()
    for index, task in enumerate(tasks):
        if not isinstance(task, dict):
            raise EvidenceError("task evidence must be an object")
        require_keys(
            task,
            {
                "taskId",
                "pairId",
                "provider",
                "consumer",
                "protocol",
                "scenario",
                "providerOpenApiArtifacts",
                "consumerOpenApiArtifacts",
                "explicitDependencySeed",
                "classification",
                "rationaleCode",
            },
            "task evidence",
        )
        if task["taskId"] != expected_task_ids[index]:
            raise EvidenceError("task identities are not frozen and ordered")
        pair_id = task["pairId"]
        if not isinstance(pair_id, str) or SAFE_ID.fullmatch(pair_id) is None:
            raise EvidenceError("pair identity is unsafe")
        provider = task["provider"]
        consumer = task["consumer"]
        if provider not in service_ids or consumer not in service_ids or provider == consumer:
            raise EvidenceError("task service binding is invalid")
        binding = (provider, consumer)
        if pair_id in pair_bindings and pair_bindings[pair_id] != binding:
            raise EvidenceError("pair identity has conflicting service bindings")
        pair_bindings[pair_id] = binding
        if task["protocol"] != "HTTP_OPENAPI_3_X" or task["scenario"] not in SCENARIOS:
            raise EvidenceError("task protocol or scenario is invalid")
        for key in ["providerOpenApiArtifacts", "consumerOpenApiArtifacts"]:
            if type(task[key]) is not int or task[key] < 0 or task[key] > 64:
                raise EvidenceError("artifact count is invalid")
        if type(task["explicitDependencySeed"]) is not bool:
            raise EvidenceError("dependency-seed authority is invalid")
        classification = task["classification"]
        if classification not in CLASSIFICATIONS or classification != expected_classification(task):
            raise EvidenceError("task classification is inconsistent with artifact authority")
        expected_rationale = {
            "EXACT_COMPARABLE": "BOTH_TRACKED_OPENAPI_AUTHORITIES",
            "DECLARED_NAVIGABLE": "PROVIDER_OPENAPI_WITH_DECLARED_CONSUMER",
            "NOT_USABLE_FOR_V1": "MISSING_TRACKED_PROVIDER_OR_CONSUMER_OPENAPI_AUTHORITY",
        }[classification]
        if task["rationaleCode"] != expected_rationale:
            raise EvidenceError("task rationale is inconsistent")
        classifications[classification] += 1

    exact = classifications["EXACT_COMPARABLE"]
    declared = classifications["DECLARED_NAVIGABLE"]
    unusable = classifications["NOT_USABLE_FOR_V1"]
    distinct_pairs = len(pair_bindings)
    passed = (
        exact >= THRESHOLDS["exactComparableMin"]
        and exact + declared >= THRESHOLDS["usableMin"]
        and distinct_pairs >= THRESHOLDS["distinctServicePairsMin"]
    )
    result = "PASS" if passed else "STOP_PROFILE_SELECTION"
    summary = evidence["summary"]
    expected_summary = {
        "exactComparable": exact,
        "declaredNavigable": declared,
        "notUsableForV1": unusable,
        "distinctServicePairs": distinct_pairs,
        "result": result,
    }
    if summary != expected_summary:
        raise EvidenceError("summary does not match task classifications")
    seals = evidence["benchmarkSeals"]
    if passed:
        if not isinstance(seals, dict) or set(seals) != {"oracle", "prompt", "rubric", "budget"}:
            raise EvidenceError("passing corpus lacks frozen benchmark seals")
        for label, digest in seals.items():
            require_digest(digest, f"{label} seal")
    elif seals is not None:
        raise EvidenceError("stopped corpus must not claim benchmark seals")
    if evidence["privacy"] != {
        "absolutePaths": False,
        "repositoryNames": False,
        "contractBodies": False,
        "credentials": False,
    }:
        raise EvidenceError("privacy declaration is invalid")
    return expected_summary


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: verify_thread_corpus_gate.py EVIDENCE.json", file=sys.stderr)
        return 2
    try:
        summary = verify(Path(sys.argv[1]))
    except (EvidenceError, OSError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print(json.dumps({"schema": SCHEMA, "verification": "PASS", "summary": summary}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
