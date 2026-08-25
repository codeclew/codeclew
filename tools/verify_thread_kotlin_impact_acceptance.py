#!/usr/bin/env python3
"""Verify the aggregate-only S2K Kotlin impact acceptance receipt."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from verify_thread_kotlin_callables_acceptance import (  # noqa: E402
    VerificationError,
    closed,
    integer,
    object_pairs,
    require,
    scan_text,
)


SCHEMA = "codeclew-thread-kotlin-impact-acceptance/1.0"
RESULT_SCHEMA = "codeclew-thread-kotlin-impact-acceptance-verification/1.0"
MAX_EVIDENCE_BYTES = 64 * 1024
DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")


def digest(value: Any, label: str) -> str:
    require(isinstance(value, str) and DIGEST.fullmatch(value) is not None, f"{label} is invalid")
    return value


def verify(path: Path) -> dict[str, Any]:
    metadata = path.lstat()
    if path.is_symlink() or not path.is_file() or metadata.st_size > MAX_EVIDENCE_BYTES:
        raise VerificationError("evidence is missing, unsafe, or exceeds 64 KiB")
    try:
        evidence = json.loads(path.read_bytes(), object_pairs_hook=object_pairs)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise VerificationError("evidence is not valid JSON") from error
    scan_text(evidence)

    root = closed(
        evidence,
        {
            "schema",
            "status",
            "profile",
            "authority",
            "fixture",
            "impactResult",
            "warmAcceptance",
            "retention",
            "verification",
        },
        "evidence",
    )
    require(root["schema"] == SCHEMA, "evidence schema is unsupported")
    require(root["status"] == "PASS", "acceptance status is not PASS")
    require(root["profile"] == "KOTLIN_DESCRIPTOR_NAVIGATION_V1", "profile changed")

    authority = closed(
        root["authority"],
        {"runtimeMode", "runtimeDigest", "factSetDigest", "impactDigest", "bindingDigest"},
        "authority",
    )
    require(authority["runtimeMode"] in {"DEVELOPMENT", "RELEASE"}, "runtime mode is invalid")
    for key in ("runtimeDigest", "factSetDigest", "impactDigest", "bindingDigest"):
        digest(authority[key], key)

    fixture = closed(
        root["fixture"],
        {
            "repositoryCount",
            "memberCount",
            "compilationCount",
            "compilerVersion",
            "independentLocalGitRepositories",
            "coordinateOrPathIncluded",
        },
        "fixture",
    )
    require(integer(fixture["repositoryCount"], "repositoryCount") == 2, "fixture needs two repositories")
    require(integer(fixture["memberCount"], "memberCount") == 2, "fixture needs two members")
    require(integer(fixture["compilationCount"], "compilationCount") == 2, "fixture needs two compilations")
    require(fixture["compilerVersion"] == "2.4.10", "compiler version changed")
    require(fixture["independentLocalGitRepositories"] is True, "repositories are not independent")
    require(fixture["coordinateOrPathIncluded"] is False, "checked evidence contains a coordinate or path")

    result = closed(
        root["impactResult"],
        {
            "subjectKind",
            "shapeStatus",
            "certainty",
            "relationshipAuthority",
            "providerObserved",
            "consumerObserved",
            "projectedDeclarationCount",
            "findingCount",
            "obligationCount",
            "sourceWindowCount",
            "findingsTruncated",
            "sourceWindowsTruncated",
            "httpClaimCount",
            "compatibilityClaimCount",
        },
        "impactResult",
    )
    require(result["subjectKind"] == "CALLABLE_FAMILY", "subject kind changed")
    require(result["shapeStatus"] == "UNSURE", "fixture must preserve boundary uncertainty")
    require(result["certainty"] == "UNSURE", "fixture certainty was upgraded")
    require(result["relationshipAuthority"] == "DECLARED_TOPOLOGY", "relationship authority was upgraded")
    require(result["providerObserved"] is True and result["consumerObserved"] is True, "one side was not observed")
    require(integer(result["projectedDeclarationCount"], "projectedDeclarationCount") == 2, "both projected declarations are required")
    findings = integer(result["findingCount"], "findingCount", minimum=2)
    obligations = integer(result["obligationCount"], "obligationCount", minimum=1)
    windows = integer(result["sourceWindowCount"], "sourceWindowCount", minimum=1)
    require(result["findingsTruncated"] is False, "fixture findings unexpectedly truncated")
    require(result["sourceWindowsTruncated"] is False, "fixture source windows unexpectedly truncated")
    require(integer(result["httpClaimCount"], "httpClaimCount") == 0, "HTTP claim escaped the contour")
    require(integer(result["compatibilityClaimCount"], "compatibilityClaimCount") == 0, "compatibility claim escaped the contour")

    warm = closed(
        root["warmAcceptance"],
        {
            "impactRunMillis",
            "impactRunCount",
            "maximumImpactRunMillis",
            "identityStable",
            "emptyToolPath",
            "prohibitedProcessCount",
            "stdoutBytes",
            "stdoutLimitBytes",
            "stdoutForbiddenMatchCount",
        },
        "warmAcceptance",
    )
    runs = warm["impactRunMillis"]
    require(isinstance(runs, list) and len(runs) >= 2, "warm impact timings are incomplete")
    run_values = [integer(value, "impactRunMillis", minimum=1) for value in runs]
    require(integer(warm["impactRunCount"], "impactRunCount") == len(run_values), "impact run count is inconsistent")
    require(integer(warm["maximumImpactRunMillis"], "maximumImpactRunMillis") == max(run_values), "maximum timing is inconsistent")
    require(max(run_values) <= 10_000, "warm impact exceeds the product budget")
    require(warm["identityStable"] is True, "warm impact identity changed")
    require(warm["emptyToolPath"] is True, "warm impact did not poison the tool PATH")
    require(integer(warm["prohibitedProcessCount"], "prohibitedProcessCount") == 0, "warm impact started a prohibited process")
    stdout_limit = integer(warm["stdoutLimitBytes"], "stdoutLimitBytes", minimum=1)
    require(stdout_limit == 65_536, "stdout limit changed")
    require(integer(warm["stdoutBytes"], "stdoutBytes") <= stdout_limit, "stdout exceeds its limit")
    require(integer(warm["stdoutForbiddenMatchCount"], "stdoutForbiddenMatchCount") == 0, "stdout contains forbidden text")

    retention = closed(
        root["retention"],
        {
            "threadGarbageCollected",
            "memberSessionsGarbageCollected",
            "rootDigestBefore",
            "rootDigestAfter",
            "closureDigestBefore",
            "closureDigestAfter",
            "declaredObjectCount",
            "reachableObjectCount",
            "reachableBytes",
            "retainedLoadVerified",
        },
        "retention",
    )
    require(retention["threadGarbageCollected"] is True, "thread GC did not complete")
    require(integer(retention["memberSessionsGarbageCollected"], "memberSessionsGarbageCollected") == 2, "member GC count changed")
    for key in ("rootDigestBefore", "rootDigestAfter", "closureDigestBefore", "closureDigestAfter"):
        digest(retention[key], key)
    require(retention["rootDigestBefore"] == retention["rootDigestAfter"], "retained root changed after GC")
    require(retention["closureDigestBefore"] == retention["closureDigestAfter"], "retained closure changed after GC")
    declared = integer(retention["declaredObjectCount"], "declaredObjectCount", minimum=1)
    reachable = integer(retention["reachableObjectCount"], "reachableObjectCount", minimum=declared)
    integer(retention["reachableBytes"], "reachableBytes", minimum=1)
    require(retention["retainedLoadVerified"] is True, "retained load was not verified")

    checks = closed(
        root["verification"],
        {
            "exactLimitAndLimitPlusOneCoverage",
            "coreFocusedTests",
            "managedServiceTests",
            "atomicRaceAndTamperCoverage",
            "clippyAllTargets",
            "diffCheck",
            "independentReview",
        },
        "verification",
    )
    require(checks["exactLimitAndLimitPlusOneCoverage"] is True, "limit/+1 coverage is missing")
    require(integer(checks["coreFocusedTests"], "coreFocusedTests", minimum=15) >= 15, "core tests are incomplete")
    require(integer(checks["managedServiceTests"], "managedServiceTests", minimum=6) >= 6, "managed tests are incomplete")
    require(checks["atomicRaceAndTamperCoverage"] is True, "race/tamper coverage is missing")
    for key in ("clippyAllTargets", "diffCheck", "independentReview"):
        require(checks[key] == "PASS", f"{key} is not PASS")

    return {
        "schema": RESULT_SCHEMA,
        "status": "PASS",
        "impactDigest": authority["impactDigest"],
        "findingCount": findings,
        "obligationCount": obligations,
        "sourceWindowCount": windows,
        "maximumImpactRunMillis": max(run_values),
        "retainedObjectCount": reachable,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence", type=Path)
    arguments = parser.parse_args()
    try:
        result = verify(arguments.evidence)
    except (OSError, VerificationError) as error:
        print(json.dumps({"schema": RESULT_SCHEMA, "status": "FAIL", "reason": str(error)}, separators=(",", ":"), sort_keys=True))
        return 1
    print(json.dumps(result, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
