#!/usr/bin/env python3
"""Verify the checked Kotlin callable acceptance summary.

The checked evidence is intentionally aggregate-only.  This verifier rejects
open schemas, duplicate keys, private locators, coordinates, and internally
inconsistent counters before accepting the S1K result.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
from typing import Any


SCHEMA = "codeclew-thread-kotlin-callables-acceptance/1.0"
MAX_EVIDENCE_BYTES = 64 * 1024
DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")
UNSAFE_TEXT = (
    re.compile(r"(?:\A|[\s\"'])(?:/Users/|/private/|/home/|[A-Za-z]:[\\/])"),
    re.compile(r"://"),
    re.compile(r"@"),
    re.compile(r"(?:\A|[./])com/"),
    re.compile(r"\bpackage\s+", re.IGNORECASE),
)
UNSAFE_TOKEN = re.compile(r"[A-Za-z0-9]+")
UNSAFE_TOKEN_SHA256 = {
    "0b3e1b057983454547b3bcbf2fac99b7309dbb5737a2d387f9f4a9bddf895147"
}


class VerificationError(RuntimeError):
    pass


def object_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise VerificationError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def closed(value: Any, keys: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != keys:
        raise VerificationError(f"{label} has an open or malformed shape")
    return value


def integer(value: Any, label: str, *, minimum: int = 0) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < minimum:
        raise VerificationError(f"{label} is not a valid integer")
    return value


def require(value: bool, message: str) -> None:
    if not value:
        raise VerificationError(message)


def scan_text(value: Any) -> None:
    pending = [value]
    while pending:
        current = pending.pop()
        if isinstance(current, dict):
            pending.extend(current.keys())
            pending.extend(current.values())
        elif isinstance(current, list):
            pending.extend(current)
        elif isinstance(current, str):
            unsafe_token = any(
                hashlib.sha256(token.lower().encode("utf-8")).hexdigest()
                in UNSAFE_TOKEN_SHA256
                for token in UNSAFE_TOKEN.findall(current)
            )
            if unsafe_token or any(pattern.search(current) for pattern in UNSAFE_TEXT):
                raise VerificationError("evidence contains private locator or coordinate text")


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
            "semanticResult",
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
        "acceptance profile is not Kotlin descriptor navigation v1",
    )

    authority = closed(
        root["authority"],
        {"runtimeMode", "runtimeDigest", "factSetDigest", "bindingDigest"},
        "authority",
    )
    require(authority["runtimeMode"] in {"DEVELOPMENT", "RELEASE"}, "runtime mode is invalid")
    for key in ("runtimeDigest", "factSetDigest", "bindingDigest"):
        require(isinstance(authority[key], str) and DIGEST.fullmatch(authority[key]) is not None, f"{key} is invalid")

    fixture = closed(
        root["fixture"],
        {
            "repositoryCount",
            "memberCount",
            "compilationCount",
            "compilerVersion",
            "independentLocalGitRepositories",
            "packageCoordinateOrPathIncluded",
        },
        "fixture",
    )
    require(integer(fixture["repositoryCount"], "repositoryCount") == 2, "fixture must use two repositories")
    require(integer(fixture["memberCount"], "memberCount") == 2, "fixture must use two members")
    require(integer(fixture["compilationCount"], "compilationCount") == 2, "fixture must use two compilations")
    require(fixture["compilerVersion"] == "2.4.10", "fixture compiler version changed")
    require(fixture["independentLocalGitRepositories"] is True, "fixture repositories are not independent")
    require(fixture["packageCoordinateOrPathIncluded"] is False, "checked evidence contains a package, coordinate, or path")

    semantic = closed(
        root["semanticResult"],
        {
            "coverage",
            "certainty",
            "relationshipAuthority",
            "visitedInputFacts",
            "visitedInputPayloadBytes",
            "declarations",
            "exactDeclarations",
            "uses",
            "exactUses",
            "boundaries",
            "totalFacts",
            "obligationCount",
        },
        "semanticResult",
    )
    require(semantic["coverage"] == "PARTIAL", "coverage must remain PARTIAL")
    require(semantic["certainty"] == "UNSURE", "certainty must remain UNSURE")
    require(semantic["relationshipAuthority"] == "DECLARED_TOPOLOGY", "relationship authority was upgraded")
    declarations = integer(semantic["declarations"], "declarations", minimum=1)
    exact_declarations = integer(semantic["exactDeclarations"], "exactDeclarations", minimum=1)
    uses = integer(semantic["uses"], "uses")
    exact_uses = integer(semantic["exactUses"], "exactUses")
    boundaries = integer(semantic["boundaries"], "boundaries", minimum=1)
    total = integer(semantic["totalFacts"], "totalFacts", minimum=1)
    obligations = integer(semantic["obligationCount"], "obligationCount", minimum=1)
    integer(semantic["visitedInputFacts"], "visitedInputFacts", minimum=total)
    integer(semantic["visitedInputPayloadBytes"], "visitedInputPayloadBytes", minimum=1)
    require(exact_declarations <= declarations, "exact declaration count exceeds declarations")
    require(exact_uses == 0 and exact_uses <= uses, "fixture must not claim exact cross-repository uses")
    require(total == declarations + uses + boundaries, "semantic fact counters do not add up")
    require(obligations == boundaries, "every boundary must remain an obligation")

    warm = closed(
        root["warmAcceptance"],
        {
            "threadContextMillis",
            "callableRunMillis",
            "callableRunCount",
            "concurrentRunCount",
            "maximumCallableRunMillis",
            "identityStable",
            "emptyToolPath",
            "prohibitedProcessCount",
            "generationAuthorityRevalidated",
            "stdoutBytes",
            "stdoutLimitBytes",
            "stdoutForbiddenMatchCount",
        },
        "warmAcceptance",
    )
    integer(warm["threadContextMillis"], "threadContextMillis", minimum=1)
    runs = warm["callableRunMillis"]
    require(isinstance(runs, list) and runs, "callable timings are missing")
    run_values = [integer(value, "callableRunMillis", minimum=1) for value in runs]
    require(integer(warm["callableRunCount"], "callableRunCount") == len(run_values), "callable run count is inconsistent")
    require(integer(warm["concurrentRunCount"], "concurrentRunCount") == 2, "concurrent acceptance did not run twice")
    require(integer(warm["maximumCallableRunMillis"], "maximumCallableRunMillis") == max(run_values), "maximum callable timing is inconsistent")
    require(max(run_values) <= 30_000, "warm callable run exceeds the S1K budget")
    require(warm["identityStable"] is True, "concurrent callable identity changed")
    require(warm["emptyToolPath"] is True, "warm acceptance did not use an empty tool PATH")
    require(integer(warm["prohibitedProcessCount"], "prohibitedProcessCount") == 0, "warm acceptance started a prohibited process")
    require(warm["generationAuthorityRevalidated"] is True, "generation authority was not revalidated")
    stdout_limit = integer(warm["stdoutLimitBytes"], "stdoutLimitBytes", minimum=1)
    stdout_bytes = integer(warm["stdoutBytes"], "stdoutBytes")
    require(stdout_limit <= 65_536 and stdout_bytes <= stdout_limit, "stdout exceeds its frozen budget")
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
    require(retention["threadGarbageCollected"] is True, "thread GC was not completed")
    require(integer(retention["memberSessionsGarbageCollected"], "memberSessionsGarbageCollected") == 2, "member-session GC count changed")
    for key in ("rootDigestBefore", "rootDigestAfter", "closureDigestBefore", "closureDigestAfter"):
        require(isinstance(retention[key], str) and DIGEST.fullmatch(retention[key]) is not None, f"{key} is invalid")
    require(retention["rootDigestBefore"] == retention["rootDigestAfter"], "retained root changed after GC")
    require(retention["closureDigestBefore"] == retention["closureDigestAfter"], "retained closure changed after GC")
    declared_objects = integer(retention["declaredObjectCount"], "declaredObjectCount", minimum=1)
    reachable_objects = integer(retention["reachableObjectCount"], "reachableObjectCount", minimum=1)
    integer(retention["reachableBytes"], "reachableBytes", minimum=1)
    require(reachable_objects >= declared_objects, "declared closure is not fully reachable")
    require(retention["retainedLoadVerified"] is True, "retained load was not verified")

    checks = closed(
        root["verification"],
        {
            "frozenLimitAndLimitPlusOneCoverage",
            "callableFocusedTests",
            "semanticValidatorTests",
            "clippyAllTargets",
            "diffCheck",
            "independentReview",
        },
        "verification",
    )
    require(checks["frozenLimitAndLimitPlusOneCoverage"] is True, "numeric limit/+1 coverage is missing")
    require(integer(checks["callableFocusedTests"], "callableFocusedTests", minimum=24) >= 24, "callable focused tests are incomplete")
    require(integer(checks["semanticValidatorTests"], "semanticValidatorTests", minimum=10) >= 10, "semantic validator tests are incomplete")
    for key in ("clippyAllTargets", "diffCheck", "independentReview"):
        require(checks[key] == "PASS", f"{key} is not PASS")

    return {
        "schema": "codeclew-thread-kotlin-callables-acceptance-verification/1.0",
        "status": "PASS",
        "factSetDigest": authority["factSetDigest"],
        "exactDeclarations": exact_declarations,
        "obligationCount": obligations,
        "maximumCallableRunMillis": max(run_values),
        "retainedObjectCount": reachable_objects,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence", type=Path)
    arguments = parser.parse_args()
    try:
        result = verify(arguments.evidence)
    except (OSError, VerificationError) as error:
        print(json.dumps({"schema": "codeclew-thread-kotlin-callables-acceptance-verification/1.0", "status": "FAIL", "reason": str(error)}, separators=(",", ":"), sort_keys=True))
        return 1
    print(json.dumps(result, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
