#!/usr/bin/env python3
"""Verify the aggregate-only S3K Kotlin change-coverage acceptance receipt."""

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


SCHEMA = "codeclew-thread-kotlin-change-coverage-acceptance/1.0"
RESULT_SCHEMA = "codeclew-thread-kotlin-change-coverage-acceptance-verification/1.0"
MAX_EVIDENCE_BYTES = 64 * 1024
DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")


def digest(value: Any, label: str) -> str:
    require(
        isinstance(value, str) and DIGEST.fullmatch(value) is not None,
        f"{label} is invalid",
    )
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
            "coverageResult",
            "identity",
            "warmAcceptance",
            "retention",
            "verification",
        },
        "evidence",
    )
    require(root["schema"] == SCHEMA, "evidence schema is unsupported")
    require(root["status"] == "PASS", "acceptance status is not PASS")
    require(
        root["profile"] == "KOTLIN_DESCRIPTOR_NAVIGATION_V1",
        "profile changed",
    )

    authority = closed(
        root["authority"],
        {
            "parentRuntimeMode",
            "parentRuntimeDigest",
            "validatorRuntimeMode",
            "validatorRuntimeDigest",
            "validatorManifestDigest",
            "rulesDigest",
            "comparisonDigest",
            "validationBindingDigest",
            "beforeFactSetDigest",
            "afterFactSetDigest",
            "beforeImpactDigest",
            "afterImpactDigest",
            "changeSetDigest",
        },
        "authority",
    )
    for key in ("parentRuntimeMode", "validatorRuntimeMode"):
        require(authority[key] in {"DEVELOPMENT", "RELEASE"}, f"{key} is invalid")
    for key, value in authority.items():
        if key.endswith("Digest"):
            digest(value, key)
    require(
        authority["parentRuntimeDigest"] != authority["validatorRuntimeDigest"],
        "validator capsule was not independently rebuilt",
    )

    fixture = closed(
        root["fixture"],
        {
            "repositoryCount",
            "threadCount",
            "memberCountPerThread",
            "compilationCountPerThread",
            "compilerVersion",
            "independentLocalGitRepositories",
            "controlledChange",
            "coordinateOrPathIncluded",
        },
        "fixture",
    )
    require(integer(fixture["repositoryCount"], "repositoryCount") == 2, "fixture needs two repositories")
    require(integer(fixture["threadCount"], "threadCount") == 2, "fixture needs before and after threads")
    require(integer(fixture["memberCountPerThread"], "memberCountPerThread") == 2, "each thread needs two members")
    require(integer(fixture["compilationCountPerThread"], "compilationCountPerThread") == 2, "each thread needs two compilations")
    require(fixture["compilerVersion"] == "2.4.10", "compiler version changed")
    require(fixture["independentLocalGitRepositories"] is True, "repositories are not independent")
    require(fixture["controlledChange"] == "EXPLICIT_RETURN_NULLABILITY", "controlled change changed")
    require(fixture["coordinateOrPathIncluded"] is False, "checked evidence contains a coordinate or path")

    result = closed(
        root["coverageResult"],
        {
            "status",
            "relationshipAuthority",
            "targetCount",
            "memberTargetCount",
            "observationCount",
            "obligationCount",
            "coveredTargetCount",
            "missingTargetCount",
            "newDerivedCasObjectCount",
            "retainedCasBytes",
            "observationCodes",
            "compatibilityClaimCount",
            "breakageClaimCount",
        },
        "coverageResult",
    )
    require(result["status"] == "VALIDATED_CONDITIONAL", "coverage status was upgraded or is incomplete")
    require(result["relationshipAuthority"] == "DECLARED_TOPOLOGY", "relationship authority was upgraded")
    targets = integer(result["targetCount"], "targetCount", minimum=1)
    require(integer(result["memberTargetCount"], "memberTargetCount") == 2, "member coverage is not pair-scoped")
    observations = integer(result["observationCount"], "observationCount", minimum=1)
    obligations = integer(result["obligationCount"], "obligationCount", minimum=1)
    require(targets == 2 + observations + obligations, "target counters do not add up")
    require(integer(result["coveredTargetCount"], "coveredTargetCount") == targets, "not every target is covered")
    require(integer(result["missingTargetCount"], "missingTargetCount") == 0, "coverage is incomplete")
    require(integer(result["newDerivedCasObjectCount"], "newDerivedCasObjectCount") == 2, "publication did not derive exactly two CAS objects")
    require(integer(result["retainedCasBytes"], "retainedCasBytes", minimum=1) <= 64 * 1024 * 1024, "retained closure exceeds 64 MiB")
    require(
        result["observationCodes"]
        == ["KCD_NULLABILITY_CHANGED", "KCD_UNSUPPORTED_COMPARISON"],
        "controlled observation set changed",
    )
    require(integer(result["compatibilityClaimCount"], "compatibilityClaimCount") == 0, "a compatibility claim escaped the contour")
    require(integer(result["breakageClaimCount"], "breakageClaimCount") == 0, "a breakage claim escaped the contour")

    identity = closed(
        root["identity"],
        {
            "validatorRuntimeCount",
            "comparisonDigestStable",
            "targetIdsStable",
            "validationBindingDigestStable",
            "finalAuthorityChanged",
        },
        "identity",
    )
    require(integer(identity["validatorRuntimeCount"], "validatorRuntimeCount", minimum=2) >= 2, "validator rebuild coverage is missing")
    for key in (
        "comparisonDigestStable",
        "targetIdsStable",
        "validationBindingDigestStable",
        "finalAuthorityChanged",
    ):
        require(identity[key] is True, f"{key} is not proven")

    warm = closed(
        root["warmAcceptance"],
        {
            "validationRunMillis",
            "validationRunCount",
            "maximumValidationRunMillis",
            "stdoutBytes",
            "stdoutLimitBytes",
            "stdoutDigest",
            "byteIdentical",
            "emptyToolPath",
            "prohibitedProcessCount",
            "stdoutForbiddenMatchCount",
        },
        "warmAcceptance",
    )
    runs = warm["validationRunMillis"]
    require(isinstance(runs, list) and len(runs) >= 2, "warm validation timings are incomplete")
    run_values = [integer(value, "validationRunMillis", minimum=1) for value in runs]
    require(integer(warm["validationRunCount"], "validationRunCount") == len(run_values), "warm run count is inconsistent")
    require(integer(warm["maximumValidationRunMillis"], "maximumValidationRunMillis") == max(run_values), "maximum timing is inconsistent")
    require(max(run_values) <= 10_000, "warm validation exceeds the product budget")
    require(integer(warm["stdoutLimitBytes"], "stdoutLimitBytes") == 65_536, "stdout limit changed")
    require(integer(warm["stdoutBytes"], "stdoutBytes") <= 65_536, "stdout exceeds its limit")
    digest(warm["stdoutDigest"], "stdoutDigest")
    require(warm["byteIdentical"] is True, "warm output identity changed")
    require(warm["emptyToolPath"] is True, "warm path did not poison executable lookup")
    require(integer(warm["prohibitedProcessCount"], "prohibitedProcessCount") == 0, "warm validation started a prohibited process")
    require(integer(warm["stdoutForbiddenMatchCount"], "stdoutForbiddenMatchCount") == 0, "stdout contains forbidden text")

    retention = closed(
        root["retention"],
        {
            "threadsGarbageCollected",
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
    require(integer(retention["threadsGarbageCollected"], "threadsGarbageCollected") == 2, "both threads were not garbage-collected")
    require(integer(retention["memberSessionsGarbageCollected"], "memberSessionsGarbageCollected") == 4, "all before/after sessions were not garbage-collected")
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
            "atomicRaceAndTamperCoverage",
            "validatorCapsuleIdentityCoverage",
            "s3FocusedTests",
            "s2RegressionTests",
            "fullLibraryTestsPassed",
            "fullLibraryTestsIgnored",
            "mainCliTests",
            "managedCliTests",
            "clippyAllTargets",
            "diffCheck",
            "independentReview",
            "independentP0",
            "independentP1",
            "independentP2",
        },
        "verification",
    )
    for key in (
        "exactLimitAndLimitPlusOneCoverage",
        "atomicRaceAndTamperCoverage",
        "validatorCapsuleIdentityCoverage",
    ):
        require(checks[key] is True, f"{key} is missing")
    require(integer(checks["s3FocusedTests"], "s3FocusedTests", minimum=34) >= 34, "S3 focused tests are incomplete")
    require(integer(checks["s2RegressionTests"], "s2RegressionTests", minimum=23) >= 23, "S2 regression tests are incomplete")
    require(integer(checks["fullLibraryTestsPassed"], "fullLibraryTestsPassed", minimum=320) >= 320, "full library tests are incomplete")
    integer(checks["fullLibraryTestsIgnored"], "fullLibraryTestsIgnored")
    require(integer(checks["mainCliTests"], "mainCliTests", minimum=13) >= 13, "main CLI tests are incomplete")
    require(integer(checks["managedCliTests"], "managedCliTests", minimum=7) >= 7, "managed CLI tests are incomplete")
    for key in ("clippyAllTargets", "diffCheck", "independentReview"):
        require(checks[key] == "PASS", f"{key} is not PASS")
    for key in ("independentP0", "independentP1", "independentP2"):
        require(integer(checks[key], key) == 0, f"{key} findings remain")

    return {
        "schema": RESULT_SCHEMA,
        "status": "PASS",
        "changeSetDigest": authority["changeSetDigest"],
        "targetCount": targets,
        "observationCount": observations,
        "obligationCount": obligations,
        "maximumValidationRunMillis": max(run_values),
        "retainedObjectCount": reachable,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence", type=Path)
    arguments = parser.parse_args()
    try:
        result = verify(arguments.evidence)
    except (OSError, VerificationError) as error:
        print(
            json.dumps(
                {"schema": RESULT_SCHEMA, "status": "FAIL", "reason": str(error)},
                separators=(",", ":"),
                sort_keys=True,
            )
        )
        return 1
    print(json.dumps(result, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
