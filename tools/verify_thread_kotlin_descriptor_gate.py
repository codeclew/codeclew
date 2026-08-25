#!/usr/bin/env python3
"""Verify canonical, path-free evidence for the Kotlin descriptor readiness gate."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tempfile
from pathlib import Path
from typing import Any


SCHEMA = "codeclew-thread-kotlin-descriptor-gate/2.0"
UNIT_AUTHORITY_SCHEMA = "codeclew-kotlin-descriptor-unit-authority/2.0"
TASK_AUTHORITY_SCHEMA = "codeclew-kotlin-descriptor-task-authority/2.0"
SIDE_AUTHORITY_SCHEMA = "codeclew-kotlin-descriptor-side-authority/2.0"
UNIT_AGGREGATE_SCHEMA = "codeclew-kotlin-descriptor-unit-aggregate/1.0"
FROZEN_AT = "2026-08-25"
EXPECTED_PRIVATE_CORPUS_DIGEST = (
    "sha256:7b49161fb1c1c322c47b318f002d7ea9ae9efb024e7d7be33a7427295668969c"
)
EXPECTED_BENCHMARK_DIGEST = (
    "sha256:0793f3020fb3b58cce97d78598f8c75944a2ffa29bd72bf061b86d6e4ee0a54c"
)
MAX_EVIDENCE_BYTES = 256 * 1024
SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")
SAFE_ID = re.compile(r"^[a-z][a-z0-9-]{0,63}$")
ABSOLUTE_PATH = re.compile(r"^(?:/|~[/\\]|[A-Za-z]:[/\\])")
EXPECTED_SERVICES = [f"service-{index:02}" for index in range(1, 12)]
EXPECTED_TASKS = [f"task-{index:02}" for index in range(1, 11)]
EXPECTED_PAIRS = [
    "pair-01", "pair-02", "pair-03", "pair-04", "pair-05",
    "pair-06", "pair-07", "pair-08", "pair-01", "pair-05",
]
EXPECTED_BINDINGS = [
    ("service-01", "service-02"),
    ("service-04", "service-01"),
    ("service-04", "service-02"),
    ("service-04", "service-03"),
    ("service-05", "service-08"),
    ("service-06", "service-07"),
    ("service-08", "service-09"),
    ("service-10", "service-11"),
    ("service-01", "service-02"),
    ("service-05", "service-08"),
]


class EvidenceError(ValueError):
    """A checked evidence file is malformed or does not pass G1K."""


def canonical_bytes(value: Any) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")


def authority_digest(value: Any) -> str:
    return f"sha256:{hashlib.sha256(canonical_bytes(value)).hexdigest()}"


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise EvidenceError("duplicate JSON key")
        value[key] = item
    return value


def require_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != expected:
        raise EvidenceError(f"{label} fields are not closed")
    return value


def require_digest(value: Any, label: str) -> str:
    if not isinstance(value, str) or SHA256.fullmatch(value) is None:
        raise EvidenceError(f"{label} is not a canonical sha256 authority")
    return value


def require_safe_strings(value: Any) -> None:
    if isinstance(value, str):
        if len(value.encode("utf-8")) > 1024 or ABSOLUTE_PATH.match(value):
            raise EvidenceError("evidence contains an unsafe string")
    elif isinstance(value, list):
        for item in value:
            require_safe_strings(item)
    elif isinstance(value, dict):
        for key, item in value.items():
            require_safe_strings(key)
            require_safe_strings(item)


def unit_aggregate_authority(
    authority_kind: str,
    service_alias: str,
    task_sides: list[dict[str, Any]],
) -> str:
    if authority_kind not in {"CONTEXT", "EVIDENCE", "COMPILER"}:
        raise EvidenceError("unit aggregate kind is invalid")
    authority_key = {
        "CONTEXT": "contextAuthority",
        "EVIDENCE": "evidenceAuthority",
        "COMPILER": "compilerAuthority",
    }[authority_kind]
    rows = sorted(
        (
            {
                "taskId": row["taskId"],
                "role": row["role"],
                "authority": row[authority_key],
            }
            for row in task_sides
        ),
        key=lambda row: (row["taskId"], row["role"]),
    )
    return authority_digest(
        {
            "schema": UNIT_AGGREGATE_SCHEMA,
            "authorityKind": authority_kind,
            "serviceAlias": service_alias,
            "taskSides": rows,
        }
    )


def unit_authority_payload(unit: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema": UNIT_AUTHORITY_SCHEMA,
        "serviceAlias": unit["serviceAlias"],
        "revisionAuthority": unit["revisionAuthority"],
        "sessionAuthority": unit["sessionAuthority"],
        "contextAuthority": unit["contextAuthority"],
        "evidenceAuthority": unit["evidenceAuthority"],
        "compilerAuthority": unit["compilerAuthority"],
        "taskSideCount": unit["taskSideCount"],
        "analysisAuthority": unit["analysisAuthority"],
        "descriptorEvidence": unit["descriptorEvidence"],
        "relationEvidence": unit["relationEvidence"],
        "boundaryEvidence": unit["boundaryEvidence"],
        "syntaxFallback": unit["syntaxFallback"],
        "k2Ready": unit["k2Ready"],
        "failureCode": unit["failureCode"],
    }


def side_authority_payload(
    task_id: str,
    role: str,
    service_alias: str,
    side: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema": SIDE_AUTHORITY_SCHEMA,
        "taskId": task_id,
        "role": role,
        "serviceAlias": service_alias,
        "contextAuthority": side["contextAuthority"],
        "evidenceAuthority": side["evidenceAuthority"],
        "compilerAuthority": side["compilerAuthority"],
        "approvedFileCount": side["approvedFileCount"],
        "minimumApprovedFiles": side["minimumApprovedFiles"],
        "callableDescriptorCount": side["callableDescriptorCount"],
        "typeDescriptorCount": side["typeDescriptorCount"],
        "descriptorEvidence": side["descriptorEvidence"],
        "relationEvidence": side["relationEvidence"],
        "boundaryEvidence": side["boundaryEvidence"],
        "k2Ready": side["k2Ready"],
    }


def task_authority_payload(task: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema": TASK_AUTHORITY_SCHEMA,
        "taskId": task["taskId"],
        "pairId": task["pairId"],
        "provider": task["provider"],
        "consumer": task["consumer"],
        "providerUnitAuthority": task["providerUnitAuthority"],
        "consumerUnitAuthority": task["consumerUnitAuthority"],
        "providerSide": task["providerSide"],
        "consumerSide": task["consumerSide"],
        "relationshipAuthority": task["relationshipAuthority"],
        "twoMemberCoverage": task["twoMemberCoverage"],
        "callableDescriptorCount": task["callableDescriptorCount"],
        "minimumCallableDescriptors": task["minimumCallableDescriptors"],
        "callableDescriptorNavigation": task["callableDescriptorNavigation"],
        "typeDescriptorCount": task["typeDescriptorCount"],
        "minimumTypeDescriptors": task["minimumTypeDescriptors"],
        "typeDescriptorNavigation": task["typeDescriptorNavigation"],
        "manualVerificationProfileBound": task["manualVerificationProfileBound"],
        "resourceBudgetAuthorityBound": task["resourceBudgetAuthorityBound"],
        "httpEquivalenceClaims": task["httpEquivalenceClaims"],
    }


def _verify_side(
    side: Any, task_id: str, role: str, service_alias: str
) -> dict[str, Any]:
    side = require_keys(
        side,
        {
            "contextAuthority", "evidenceAuthority", "compilerAuthority",
            "approvedFileCount", "minimumApprovedFiles",
            "callableDescriptorCount", "typeDescriptorCount",
            "descriptorEvidence", "relationEvidence", "boundaryEvidence",
            "k2Ready", "sideAuthority",
        },
        f"{task_id} {role} side authority",
    )
    for key in ["contextAuthority", "evidenceAuthority", "compilerAuthority", "sideAuthority"]:
        require_digest(side[key], f"{task_id} {role} {key}")
    for key in [
        "approvedFileCount", "minimumApprovedFiles",
        "callableDescriptorCount", "typeDescriptorCount",
    ]:
        if type(side[key]) is not int:
            raise EvidenceError("side navigation count is invalid")
    if (
        side["approvedFileCount"] < 0
        or side["minimumApprovedFiles"] < 1
        or side["callableDescriptorCount"] < 0
        or side["typeDescriptorCount"] < 0
    ):
        raise EvidenceError("side navigation count is outside the frozen domain")
    for key in ["descriptorEvidence", "relationEvidence", "boundaryEvidence", "k2Ready"]:
        if type(side[key]) is not bool:
            raise EvidenceError("side evidence-presence value is invalid")
    if (
        side["boundaryEvidence"] is not False
        or side["k2Ready"] is not True
        or side["descriptorEvidence"]
        != (side["callableDescriptorCount"] + side["typeDescriptorCount"] > 0)
    ):
        raise EvidenceError("boundary or non-K2 evidence qualified a task side")
    if side["sideAuthority"] != authority_digest(
        side_authority_payload(task_id, role, service_alias, side)
    ):
        raise EvidenceError("side authority digest is inconsistent")
    return side


def verify_value(evidence: Any) -> dict[str, Any]:
    require_keys(
        evidence,
        {
            "schema", "frozenAt", "selectionAuthority", "executionAuthority",
            "units", "tasks", "summary", "privacy",
        },
        "evidence",
    )
    require_safe_strings(evidence)
    if evidence["schema"] != SCHEMA or evidence["frozenAt"] != FROZEN_AT:
        raise EvidenceError("evidence schema or freeze date is invalid")

    selection = require_keys(
        evidence["selectionAuthority"],
        {
            "kind", "ruleId", "privateCorpusDigest", "benchmarkDigest",
            "unitCount", "taskCount", "pairCount",
        },
        "selection authority",
    )
    if (
        selection["kind"] != "PINNED_KOTLIN_DESCRIPTOR_CORPUS"
        or selection["ruleId"] != "REUSE_G1_TASKS_AND_PAIRS_V1"
        or selection["unitCount"] != 11
        or selection["taskCount"] != 10
        or selection["pairCount"] != 8
    ):
        raise EvidenceError("selection authority is invalid")
    require_digest(selection["privateCorpusDigest"], "private corpus digest")
    require_digest(selection["benchmarkDigest"], "benchmark digest")
    if (
        selection["privateCorpusDigest"] != EXPECTED_PRIVATE_CORPUS_DIGEST
        or selection["benchmarkDigest"] != EXPECTED_BENCHMARK_DIGEST
    ):
        raise EvidenceError("gate does not bind the frozen corpus and benchmark")

    execution = require_keys(
        evidence["executionAuthority"],
        {"clewAuthority", "compilationAuthority", "maxParallelism"},
        "execution authority",
    )
    require_digest(execution["clewAuthority"], "clew authority")
    require_digest(execution["compilationAuthority"], "compilation authority")
    if execution["compilationAuthority"] != authority_digest(":/main"):
        raise EvidenceError("gate did not use the frozen :/main compilation")
    if type(execution["maxParallelism"]) is not int or not 1 <= execution["maxParallelism"] <= 4:
        raise EvidenceError("parallelism authority is invalid")

    units = evidence["units"]
    if not isinstance(units, list) or len(units) != 11:
        raise EvidenceError("gate must contain eleven unit authorities")
    aliases: list[str] = []
    unit_by_alias: dict[str, dict[str, Any]] = {}
    for unit in units:
        require_keys(
            unit,
            {
                "serviceAlias", "revisionAuthority", "sessionAuthority",
                "contextAuthority", "evidenceAuthority", "compilerAuthority",
                "taskSideCount", "unitAuthority", "analysisAuthority",
                "descriptorEvidence", "relationEvidence", "boundaryEvidence",
                "syntaxFallback", "k2Ready", "failureCode",
            },
            "unit authority",
        )
        alias = unit["serviceAlias"]
        if not isinstance(alias, str) or SAFE_ID.fullmatch(alias) is None:
            raise EvidenceError("unit alias is unsafe")
        aliases.append(alias)
        for key in [
            "revisionAuthority", "sessionAuthority", "contextAuthority",
            "evidenceAuthority", "compilerAuthority", "unitAuthority",
        ]:
            require_digest(unit[key], f"{alias} {key}")
        if type(unit["taskSideCount"]) is not int or unit["taskSideCount"] < 1:
            raise EvidenceError("unit task-side count is invalid")
        for key in [
            "descriptorEvidence", "relationEvidence", "boundaryEvidence",
            "syntaxFallback", "k2Ready",
        ]:
            if type(unit[key]) is not bool:
                raise EvidenceError("unit evidence-presence value is invalid")
        if (
            unit["analysisAuthority"] != "COMPILER_WORKER"
            or unit["syntaxFallback"] is not False
            or unit["k2Ready"] is not True
            or unit["failureCode"] is not None
            or unit["descriptorEvidence"] is not True
            or unit["boundaryEvidence"] is not False
        ):
            raise EvidenceError("all units must be strict K2 descriptor-ready")
        if unit["unitAuthority"] != authority_digest(unit_authority_payload(unit)):
            raise EvidenceError("unit authority digest is inconsistent")
        if alias in unit_by_alias:
            raise EvidenceError("unit alias is duplicated")
        unit_by_alias[alias] = unit
    if aliases != EXPECTED_SERVICES:
        raise EvidenceError("unit aliases are not the frozen canonical set")

    tasks = evidence["tasks"]
    if not isinstance(tasks, list) or len(tasks) != 10:
        raise EvidenceError("gate must contain ten task authorities")
    pair_bindings: dict[str, tuple[str, str]] = {}
    task_sides_by_alias: dict[str, list[dict[str, Any]]] = {
        alias: [] for alias in EXPECTED_SERVICES
    }
    for index, task in enumerate(tasks):
        require_keys(
            task,
            {
                "taskId", "pairId", "provider", "consumer",
                "providerUnitAuthority", "consumerUnitAuthority",
                "providerSide", "consumerSide", "relationshipAuthority",
                "twoMemberCoverage", "callableDescriptorCount",
                "minimumCallableDescriptors", "callableDescriptorNavigation",
                "typeDescriptorCount", "minimumTypeDescriptors",
                "typeDescriptorNavigation", "manualVerificationProfileBound",
                "resourceBudgetAuthorityBound", "httpEquivalenceClaims",
                "taskAuthority",
            },
            "task authority",
        )
        task_id = task["taskId"]
        if task_id != EXPECTED_TASKS[index] or task["pairId"] != EXPECTED_PAIRS[index]:
            raise EvidenceError("task or pair identity is not frozen")
        provider = task["provider"]
        consumer = task["consumer"]
        if provider not in unit_by_alias or consumer not in unit_by_alias or provider == consumer:
            raise EvidenceError("task member binding is invalid")
        if (provider, consumer) != EXPECTED_BINDINGS[index]:
            raise EvidenceError("task member binding is not the frozen topology")
        binding = (provider, consumer)
        if task["pairId"] in pair_bindings and pair_bindings[task["pairId"]] != binding:
            raise EvidenceError("pair identity has conflicting bindings")
        pair_bindings[task["pairId"]] = binding

        provider_side = _verify_side(task["providerSide"], task_id, "provider", provider)
        consumer_side = _verify_side(task["consumerSide"], task_id, "consumer", consumer)
        task_sides_by_alias[provider].append(
            {"taskId": task_id, "role": "provider", **provider_side}
        )
        task_sides_by_alias[consumer].append(
            {"taskId": task_id, "role": "consumer", **consumer_side}
        )
        for key in [
            "callableDescriptorCount", "minimumCallableDescriptors",
            "typeDescriptorCount", "minimumTypeDescriptors", "httpEquivalenceClaims",
        ]:
            if type(task[key]) is not int:
                raise EvidenceError("task navigation count is invalid")
        for key in [
            "twoMemberCoverage", "callableDescriptorNavigation",
            "typeDescriptorNavigation", "manualVerificationProfileBound",
            "resourceBudgetAuthorityBound",
        ]:
            if type(task[key]) is not bool:
                raise EvidenceError("task readiness binding is invalid")
        callable_count = (
            provider_side["callableDescriptorCount"]
            + consumer_side["callableDescriptorCount"]
        )
        type_count = provider_side["typeDescriptorCount"] + consumer_side["typeDescriptorCount"]
        covered = (
            provider_side["approvedFileCount"] >= provider_side["minimumApprovedFiles"]
            and consumer_side["approvedFileCount"] >= consumer_side["minimumApprovedFiles"]
            and provider_side["k2Ready"]
            and consumer_side["k2Ready"]
        )
        if (
            task["providerUnitAuthority"] != unit_by_alias[provider]["unitAuthority"]
            or task["consumerUnitAuthority"] != unit_by_alias[consumer]["unitAuthority"]
            or task["relationshipAuthority"] != "DECLARED_TOPOLOGY"
            or task["twoMemberCoverage"] is not covered
            or task["twoMemberCoverage"] is not True
            or task["callableDescriptorCount"] != callable_count
            or task["typeDescriptorCount"] != type_count
            or task["minimumCallableDescriptors"] != 1
            or task["minimumTypeDescriptors"] != 1
            or task["callableDescriptorNavigation"]
            is not (callable_count >= task["minimumCallableDescriptors"])
            or task["typeDescriptorNavigation"]
            is not (type_count >= task["minimumTypeDescriptors"])
            or task["callableDescriptorNavigation"] is not True
            or task["typeDescriptorNavigation"] is not True
            or task["manualVerificationProfileBound"] is not True
            or task["resourceBudgetAuthorityBound"] is not True
            or task["httpEquivalenceClaims"] != 0
        ):
            raise EvidenceError("task descriptor readiness or authority binding is invalid")
        require_digest(task["taskAuthority"], "task authority digest")
        if task["taskAuthority"] != authority_digest(task_authority_payload(task)):
            raise EvidenceError("task authority digest is inconsistent")
    if len(pair_bindings) != 8:
        raise EvidenceError("gate must contain eight distinct service pairs")

    for alias, unit in unit_by_alias.items():
        task_sides = task_sides_by_alias[alias]
        if (
            unit["taskSideCount"] != len(task_sides)
            or unit["contextAuthority"]
            != unit_aggregate_authority("CONTEXT", alias, task_sides)
            or unit["evidenceAuthority"]
            != unit_aggregate_authority("EVIDENCE", alias, task_sides)
            or unit["compilerAuthority"]
            != unit_aggregate_authority("COMPILER", alias, task_sides)
            or unit["descriptorEvidence"]
            is not any(side["descriptorEvidence"] for side in task_sides)
            or unit["relationEvidence"]
            is not any(side["relationEvidence"] for side in task_sides)
        ):
            raise EvidenceError("unit task-side authority aggregate is inconsistent")

    expected_summary = {
        "unitCount": 11,
        "readyUnits": 11,
        "taskCount": 10,
        "taskSideCount": 20,
        "coveredTasks": 10,
        "callableNavigableTasks": 10,
        "typeNavigableTasks": 10,
        "distinctServicePairs": 8,
        "declaredTopologyTasks": 10,
        "manualVerificationProfilesBound": 10,
        "resourceBudgetAuthoritiesBound": 10,
        "httpEquivalenceClaims": 0,
        "result": "PASS",
    }
    if evidence["summary"] != expected_summary:
        raise EvidenceError("summary is inconsistent or the gate did not pass")
    if evidence["privacy"] != {
        "absolutePaths": False,
        "repositoryNames": False,
        "sourceBodies": False,
        "packageNames": False,
        "credentials": False,
    }:
        raise EvidenceError("privacy declaration is invalid")
    return expected_summary


def verify(path: Path) -> dict[str, Any]:
    raw = path.read_bytes()
    if len(raw) == 0 or len(raw) > MAX_EVIDENCE_BYTES:
        raise EvidenceError("checked evidence size is invalid")
    try:
        evidence = json.loads(raw, object_pairs_hook=reject_duplicate_keys)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise EvidenceError("checked evidence is not valid UTF-8 JSON") from error
    if canonical_bytes(evidence) != raw:
        raise EvidenceError("checked evidence is not canonical JSON")
    return verify_value(evidence)


def _side(task_id: str, role: str, alias: str, callable_count: int) -> dict[str, Any]:
    side: dict[str, Any] = {
        "contextAuthority": authority_digest([task_id, role, "context"]),
        "evidenceAuthority": authority_digest([task_id, role, "evidence"]),
        "compilerAuthority": authority_digest([task_id, role, "compiler"]),
        "approvedFileCount": 1,
        "minimumApprovedFiles": 1,
        "callableDescriptorCount": callable_count,
        "typeDescriptorCount": 1,
        "descriptorEvidence": True,
        "relationEvidence": False,
        "boundaryEvidence": False,
        "k2Ready": True,
    }
    side["sideAuthority"] = authority_digest(side_authority_payload(task_id, role, alias, side))
    return side


def _valid_fixture() -> dict[str, Any]:
    side_rows: list[tuple[str, str, str, dict[str, Any]]] = []
    task_parts: list[tuple[str, str, str, str, dict[str, Any], dict[str, Any]]] = []
    for task_id, pair_id, (provider, consumer) in zip(
        EXPECTED_TASKS, EXPECTED_PAIRS, EXPECTED_BINDINGS, strict=True
    ):
        provider_side = _side(task_id, "provider", provider, 1)
        consumer_side = _side(task_id, "consumer", consumer, 0)
        side_rows.extend(
            [
                (task_id, "provider", provider, provider_side),
                (task_id, "consumer", consumer, consumer_side),
            ]
        )
        task_parts.append(
            (task_id, pair_id, provider, consumer, provider_side, consumer_side)
        )

    units = []
    for alias in EXPECTED_SERVICES:
        sides = [
            {"taskId": task_id, "role": role, **side}
            for task_id, role, side_alias, side in side_rows
            if side_alias == alias
        ]
        unit: dict[str, Any] = {
            "serviceAlias": alias,
            "revisionAuthority": authority_digest([alias, "revision"]),
            "sessionAuthority": authority_digest([alias, "session"]),
            "contextAuthority": unit_aggregate_authority("CONTEXT", alias, sides),
            "evidenceAuthority": unit_aggregate_authority("EVIDENCE", alias, sides),
            "compilerAuthority": unit_aggregate_authority("COMPILER", alias, sides),
            "taskSideCount": len(sides),
            "analysisAuthority": "COMPILER_WORKER",
            "descriptorEvidence": True,
            "relationEvidence": False,
            "boundaryEvidence": False,
            "syntaxFallback": False,
            "k2Ready": True,
            "failureCode": None,
        }
        unit["unitAuthority"] = authority_digest(unit_authority_payload(unit))
        units.append(unit)
    by_alias = {unit["serviceAlias"]: unit for unit in units}

    tasks = []
    for task_id, pair_id, provider, consumer, provider_side, consumer_side in task_parts:
        task: dict[str, Any] = {
            "taskId": task_id,
            "pairId": pair_id,
            "provider": provider,
            "consumer": consumer,
            "providerUnitAuthority": by_alias[provider]["unitAuthority"],
            "consumerUnitAuthority": by_alias[consumer]["unitAuthority"],
            "providerSide": provider_side,
            "consumerSide": consumer_side,
            "relationshipAuthority": "DECLARED_TOPOLOGY",
            "twoMemberCoverage": True,
            "callableDescriptorCount": 1,
            "minimumCallableDescriptors": 1,
            "callableDescriptorNavigation": True,
            "typeDescriptorCount": 2,
            "minimumTypeDescriptors": 1,
            "typeDescriptorNavigation": True,
            "manualVerificationProfileBound": True,
            "resourceBudgetAuthorityBound": True,
            "httpEquivalenceClaims": 0,
        }
        task["taskAuthority"] = authority_digest(task_authority_payload(task))
        tasks.append(task)
    return {
        "schema": SCHEMA,
        "frozenAt": FROZEN_AT,
        "selectionAuthority": {
            "kind": "PINNED_KOTLIN_DESCRIPTOR_CORPUS",
            "ruleId": "REUSE_G1_TASKS_AND_PAIRS_V1",
            "privateCorpusDigest": EXPECTED_PRIVATE_CORPUS_DIGEST,
            "benchmarkDigest": EXPECTED_BENCHMARK_DIGEST,
            "unitCount": 11,
            "taskCount": 10,
            "pairCount": 8,
        },
        "executionAuthority": {
            "clewAuthority": authority_digest("clew"),
            "compilationAuthority": authority_digest(":/main"),
            "maxParallelism": 2,
        },
        "units": units,
        "tasks": tasks,
        "summary": {
            "unitCount": 11,
            "readyUnits": 11,
            "taskCount": 10,
            "taskSideCount": 20,
            "coveredTasks": 10,
            "callableNavigableTasks": 10,
            "typeNavigableTasks": 10,
            "distinctServicePairs": 8,
            "declaredTopologyTasks": 10,
            "manualVerificationProfilesBound": 10,
            "resourceBudgetAuthoritiesBound": 10,
            "httpEquivalenceClaims": 0,
            "result": "PASS",
        },
        "privacy": {
            "absolutePaths": False,
            "repositoryNames": False,
            "sourceBodies": False,
            "packageNames": False,
            "credentials": False,
        },
    }


def _refresh_task(task: dict[str, Any]) -> None:
    provider = task["providerSide"]
    consumer = task["consumerSide"]
    task["callableDescriptorCount"] = (
        provider["callableDescriptorCount"] + consumer["callableDescriptorCount"]
    )
    task["typeDescriptorCount"] = (
        provider["typeDescriptorCount"] + consumer["typeDescriptorCount"]
    )
    task["callableDescriptorNavigation"] = (
        task["callableDescriptorCount"] >= task["minimumCallableDescriptors"]
    )
    task["typeDescriptorNavigation"] = (
        task["typeDescriptorCount"] >= task["minimumTypeDescriptors"]
    )
    task["twoMemberCoverage"] = all(
        side["approvedFileCount"] >= side["minimumApprovedFiles"]
        and side["k2Ready"]
        for side in (provider, consumer)
    )
    task["taskAuthority"] = authority_digest(task_authority_payload(task))


def self_test() -> None:
    fixture = _valid_fixture()
    verify_value(fixture)
    mutations = []

    boundary_only = json.loads(canonical_bytes(fixture))
    boundary_side = boundary_only["tasks"][0]["providerSide"]
    boundary_side.update(
        {
            "approvedFileCount": 0,
            "callableDescriptorCount": 0,
            "typeDescriptorCount": 0,
            "descriptorEvidence": False,
            "boundaryEvidence": True,
        }
    )
    boundary_side["sideAuthority"] = authority_digest(
        side_authority_payload("task-01", "provider", "service-01", boundary_side)
    )
    _refresh_task(boundary_only["tasks"][0])
    mutations.append(boundary_only)

    no_callable = json.loads(canonical_bytes(fixture))
    callable_side = no_callable["tasks"][0]["providerSide"]
    callable_side["callableDescriptorCount"] = 0
    callable_side["sideAuthority"] = authority_digest(
        side_authority_payload("task-01", "provider", "service-01", callable_side)
    )
    _refresh_task(no_callable["tasks"][0])
    mutations.append(no_callable)

    for field in ["manualVerificationProfileBound", "resourceBudgetAuthorityBound"]:
        unbound = json.loads(canonical_bytes(fixture))
        unbound["tasks"][0][field] = False
        _refresh_task(unbound["tasks"][0])
        mutations.append(unbound)

    manual_complete = json.loads(canonical_bytes(fixture))
    manual_complete["tasks"][0]["manualVerificationComplete"] = True
    mutations.append(manual_complete)

    aggregate_tamper = json.loads(canonical_bytes(fixture))
    aggregate_tamper["units"][0]["contextAuthority"] = authority_digest("different")
    aggregate_tamper["units"][0]["unitAuthority"] = authority_digest(
        unit_authority_payload(aggregate_tamper["units"][0])
    )
    for task in aggregate_tamper["tasks"]:
        if task["provider"] == "service-01":
            task["providerUnitAuthority"] = aggregate_tamper["units"][0]["unitAuthority"]
            _refresh_task(task)
        if task["consumer"] == "service-01":
            task["consumerUnitAuthority"] = aggregate_tamper["units"][0]["unitAuthority"]
            _refresh_task(task)
    mutations.append(aggregate_tamper)

    inferred = json.loads(canonical_bytes(fixture))
    inferred["tasks"][0]["relationshipAuthority"] = "INFERRED"
    _refresh_task(inferred["tasks"][0])
    mutations.append(inferred)

    http_claim = json.loads(canonical_bytes(fixture))
    http_claim["tasks"][0]["httpEquivalenceClaims"] = 1
    _refresh_task(http_claim["tasks"][0])
    mutations.append(http_claim)

    leaked = json.loads(canonical_bytes(fixture))
    leaked["units"][0]["failureCode"] = "/private/repository"
    mutations.append(leaked)

    for mutation in mutations:
        try:
            verify_value(mutation)
        except EvidenceError:
            pass
        else:
            raise AssertionError("invalid evidence mutation was accepted")
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory, "evidence.json")
        path.write_bytes(canonical_bytes(fixture))
        verify(path)
        path.write_bytes(json.dumps(fixture, indent=2).encode("utf-8"))
        try:
            verify(path)
        except EvidenceError:
            pass
        else:
            raise AssertionError("noncanonical evidence was accepted")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", nargs="?", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        try:
            self_test()
        except Exception:
            print("FAIL: SELF_TEST_FAILED", file=sys.stderr)
            return 1
        print(json.dumps({"schema": SCHEMA, "selfTest": "PASS"}, sort_keys=True))
        return 0
    if args.evidence is None:
        parser.error("evidence path is required unless --self-test is used")
    try:
        summary = verify(args.evidence)
    except (EvidenceError, OSError):
        print("FAIL: CHECKED_EVIDENCE_INVALID", file=sys.stderr)
        return 1
    print(
        json.dumps(
            {"schema": SCHEMA, "verification": "PASS", "summary": summary},
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
