#!/usr/bin/env python3
"""Evaluate private 20-case pilot evidence without exposing case data."""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import stat


ROOT = Path(__file__).resolve().parent.parent
MAX_CASE_SET_BYTES = 1024 * 1024
CASE_COUNT = 20
CASE_ID = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}$")
SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")
HMAC_SHA256 = re.compile(r"^hmac-sha256:[0-9a-f]{64}$")
ERROR_CODE = re.compile(r"^[A-Z][A-Z0-9_]{0,63}$")
PROJECT_CLASS = "kotlin24-gradle-single-compilation"
OUTCOMES = {
    "PUBLISHED",
    "PUBLISHED_CONDITIONAL",
    "VALIDATED_CONDITIONAL",
    "FAILED",
    "CANCELLED",
    "RECOVERY_REQUIRED",
}
RUNTIME_MODES = {"DEVELOPMENT", "RELEASE"}
CASE_KEYS = {
    "attestation", "caseId", "durationsMs", "errorCode", "evidenceDigest",
    "idempotentRetry", "outcome", "pilotId",
    "preparedWithoutManualCleanup", "privateDataLeak", "projectClass",
    "recoveryResolved", "runtimeMode", "schema", "sourcePreservedBeforePublish",
    "typedOutcome", "validationPassed",
}
DURATION_KEYS = {"open", "prepareToReady", "publish", "total"}


class GateInputError(Exception):
    pass


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def private_key(path: Path) -> bytes:
    if not path.is_absolute():
        raise GateInputError("attestation key path must be absolute")
    try:
        metadata = path.lstat()
        resolved = path.resolve(strict=True)
    except OSError as error:
        raise GateInputError("attestation key is unavailable") from error
    if (
        resolved != path
        or not stat.S_ISREG(metadata.st_mode)
        or metadata.st_mode & 0o077
        or metadata.st_size > 1024
    ):
        raise GateInputError("attestation key must be a physical private file")
    try:
        resolved.relative_to(ROOT)
    except ValueError:
        pass
    else:
        raise GateInputError("attestation key must be outside the repository")
    try:
        value = json.loads(path.read_bytes())
        encoded = value.get("keyHex") if isinstance(value, dict) else None
        if (
            not isinstance(value, dict)
            or set(value) != {"keyHex", "schema"}
            or value.get("schema") != "codeclew-pilot-attestation-key/1.0"
            or not isinstance(encoded, str)
            or not re.fullmatch(r"[0-9a-f]{64}", encoded)
        ):
            raise GateInputError("attestation key schema is invalid")
        return bytes.fromhex(encoded)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise GateInputError("attestation key schema is invalid") from error


def pilot_id(key: bytes) -> str:
    return "sha256:" + hashlib.sha256(key).hexdigest()


def attest_case(case: dict[str, object], key: bytes) -> dict[str, object]:
    result = dict(case)
    result["pilotId"] = pilot_id(key)
    result.pop("attestation", None)
    result["attestation"] = "hmac-sha256:" + hmac.new(
        key, canonical(result), hashlib.sha256
    ).hexdigest()
    return result


def private_case_set(path: Path) -> dict[str, object]:
    if not path.is_absolute():
        raise GateInputError("case set path must be absolute")
    try:
        metadata = path.lstat()
        resolved = path.resolve(strict=True)
    except OSError as error:
        raise GateInputError("case set is unavailable") from error
    if resolved != path or not stat.S_ISREG(metadata.st_mode):
        raise GateInputError("case set must be a physical regular file")
    try:
        resolved.relative_to(ROOT)
    except ValueError:
        pass
    else:
        raise GateInputError("case set must be outside the repository")
    if metadata.st_mode & 0o077:
        raise GateInputError("case set must be private (0600)")
    if metadata.st_size <= 0 or metadata.st_size > MAX_CASE_SET_BYTES:
        raise GateInputError("case set is empty or oversized")
    try:
        value = json.loads(path.read_bytes())
    except (OSError, json.JSONDecodeError, UnicodeDecodeError) as error:
        raise GateInputError("case set is not valid JSON") from error
    if not isinstance(value, dict):
        raise GateInputError("case set must be an object")
    return value


def require_bool(case: dict[str, object], key: str) -> bool:
    value = case.get(key)
    if not isinstance(value, bool):
        raise GateInputError(f"case field {key} must be boolean")
    return value


def validate_case(case: object, key: bytes) -> dict[str, object]:
    if not isinstance(case, dict) or set(case) != CASE_KEYS:
        raise GateInputError("case fields do not match the closed schema")
    if case.get("schema") != "codeclew-pilot-case/1.0":
        raise GateInputError("case schema is invalid")
    evidence_digest = case.get("evidenceDigest")
    attestation = case.get("attestation")
    if (
        not isinstance(evidence_digest, str)
        or not SHA256.fullmatch(evidence_digest)
        or case.get("pilotId") != pilot_id(key)
        or not isinstance(attestation, str)
        or not HMAC_SHA256.fullmatch(attestation)
    ):
        raise GateInputError("case recorder provenance is invalid")
    unsigned = dict(case)
    unsigned.pop("attestation")
    expected = "hmac-sha256:" + hmac.new(
        key, canonical(unsigned), hashlib.sha256
    ).hexdigest()
    if not hmac.compare_digest(attestation, expected):
        raise GateInputError("case attestation is invalid")
    case_id = case.get("caseId")
    if not isinstance(case_id, str) or not CASE_ID.fullmatch(case_id):
        raise GateInputError("case ID is invalid")
    if case.get("projectClass") != PROJECT_CLASS:
        raise GateInputError("case is outside the supported project contour")
    if case.get("runtimeMode") not in RUNTIME_MODES:
        raise GateInputError("runtime mode is invalid")
    outcome = case.get("outcome")
    if outcome not in OUTCOMES:
        raise GateInputError("case outcome is invalid")
    durations = case.get("durationsMs")
    if not isinstance(durations, dict) or set(durations) != DURATION_KEYS:
        raise GateInputError("case durations do not match the closed schema")
    if any(
        not isinstance(value, int) or isinstance(value, bool) or not 0 <= value <= 86_400_000
        for value in durations.values()
    ):
        raise GateInputError("case duration is invalid")
    if durations["total"] < sum(
        durations[key] for key in ["open", "prepareToReady", "publish"]
    ):
        raise GateInputError("case total duration is inconsistent")
    error_code = case.get("errorCode")
    if error_code is not None and (
        not isinstance(error_code, str) or not ERROR_CODE.fullmatch(error_code)
    ):
        raise GateInputError("case error code is invalid")
    typed = require_bool(case, "typedOutcome")
    published = outcome in {"PUBLISHED", "PUBLISHED_CONDITIONAL"}
    if published and error_code is not None:
        raise GateInputError("published case cannot carry an error code")
    if not published and (not typed or error_code is None):
        raise GateInputError("non-published case must have a typed error")
    for key in [
        "idempotentRetry", "preparedWithoutManualCleanup", "privateDataLeak",
        "recoveryResolved", "sourcePreservedBeforePublish", "validationPassed",
    ]:
        require_bool(case, key)
    prepared = bool(case["preparedWithoutManualCleanup"])
    validation_passed = bool(case["validationPassed"])
    recovery_resolved = bool(case["recoveryResolved"])
    if published and (not prepared or not validation_passed or not recovery_resolved):
        raise GateInputError("published case lacks prepared validation authority")
    if outcome == "VALIDATED_CONDITIONAL" and (not prepared or not validation_passed):
        raise GateInputError("validated conditional case lacks validation authority")
    if outcome in {"FAILED", "CANCELLED"} and (prepared or validation_passed):
        raise GateInputError("failed case contradicts prepared validation authority")
    if outcome == "RECOVERY_REQUIRED" and recovery_resolved:
        raise GateInputError("recovery-required case cannot be marked resolved")
    return case


def validate_case_set(value: dict[str, object], key: bytes) -> list[dict[str, object]]:
    if (
        set(value) != {"cases", "schema"}
        or value.get("schema") != "codeclew-pilot-case-set/1.0"
    ):
        raise GateInputError("case set schema is invalid")
    raw_cases = value.get("cases")
    if not isinstance(raw_cases, list) or len(raw_cases) != CASE_COUNT:
        raise GateInputError("case set must contain exactly 20 cases")
    cases = [validate_case(case, key) for case in raw_cases]
    identifiers = [str(case["caseId"]) for case in cases]
    if len(set(identifiers)) != CASE_COUNT:
        raise GateInputError("case IDs must be unique")
    evidence_digests = [str(case["evidenceDigest"]) for case in cases]
    if len(set(evidence_digests)) != CASE_COUNT:
        raise GateInputError("case evidence digests must be unique")
    return cases


def evaluate(value: dict[str, object], key: bytes) -> dict[str, object]:
    cases = validate_case_set(value, key)
    prepared = sum(bool(case["preparedWithoutManualCleanup"]) for case in cases)
    source_preserved = sum(bool(case["sourcePreservedBeforePublish"]) for case in cases)
    idempotent = sum(bool(case["idempotentRetry"]) for case in cases)
    typed = sum(bool(case["typedOutcome"]) for case in cases)
    validation_passed = sum(bool(case["validationPassed"]) for case in cases)
    recovery_resolved = sum(bool(case["recoveryResolved"]) for case in cases)
    leaks = sum(bool(case["privateDataLeak"]) for case in cases)
    release_cases = sum(case["runtimeMode"] == "RELEASE" for case in cases)
    published_cases = sum(
        case["outcome"] in {"PUBLISHED", "PUBLISHED_CONDITIONAL"} for case in cases
    )
    criteria = {
        "idempotentRetries": idempotent == CASE_COUNT,
        "noPrivateDataLeak": leaks == 0,
        "preparedWithoutManualCleanup": prepared >= 19,
        "publishedCases": published_cases >= 19,
        "releaseRuntime": release_cases == CASE_COUNT,
        "recoveryResolved": recovery_resolved == CASE_COUNT,
        "sourcePreservedBeforePublish": source_preserved == CASE_COUNT,
        "typedOutcomes": typed == CASE_COUNT,
        "validationPassed": validation_passed >= 19,
    }
    passed = all(criteria.values())
    runtime_modes = Counter(str(case["runtimeMode"]) for case in cases)
    outcomes = Counter(str(case["outcome"]) for case in cases)
    return {
        "caseSetDigest": "sha256:" + hashlib.sha256(canonical(value)).hexdigest(),
        "criteria": criteria,
        "decision": "SIGNED_RELEASE_ELIGIBLE" if passed else "NOT_ELIGIBLE",
        "metrics": {
            "cases": CASE_COUNT,
            "idempotentRetries": idempotent,
            "preparedWithoutManualCleanup": prepared,
            "privateDataLeaks": leaks,
            "publishedCases": published_cases,
            "releaseRuntimeCases": release_cases,
            "recoveryResolved": recovery_resolved,
            "sourcePreservedBeforePublish": source_preserved,
            "typedOutcomes": typed,
            "validationPassed": validation_passed,
        },
        "outcomes": dict(sorted(outcomes.items())),
        "pilotId": pilot_id(key),
        "runtimeModes": dict(sorted(runtime_modes.items())),
        "schema": "codeclew-pilot-release-decision/1.0",
        "status": "PASS" if passed else "FAIL",
    }


def write_private_decision(path: Path, decision: dict[str, object]) -> str:
    if not path.is_absolute() or path.exists():
        raise GateInputError("receipt must be a new absolute path")
    try:
        parent = path.parent.resolve(strict=True)
    except OSError as error:
        raise GateInputError("receipt parent is unavailable") from error
    if parent != path.parent:
        raise GateInputError("receipt parent must be physical and canonical")
    try:
        path.resolve().relative_to(ROOT)
    except ValueError:
        pass
    else:
        raise GateInputError("receipt must be outside the repository")
    data = canonical(decision) + b"\n"
    try:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
    except OSError as error:
        try:
            path.unlink()
        except OSError:
            pass
        raise GateInputError("receipt cannot be written") from error
    return "sha256:" + hashlib.sha256(data).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--attestation-key", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    arguments = parser.parse_args()
    try:
        decision = evaluate(
            private_case_set(arguments.cases), private_key(arguments.attestation_key)
        )
        decision["receiptDigest"] = write_private_decision(arguments.receipt, decision)
    except GateInputError:
        print(json.dumps({
            "errorCode": "INVALID_PILOT_EVIDENCE",
            "schema": "codeclew-pilot-release-decision/1.0",
            "status": "INVALID",
        }, sort_keys=True, separators=(",", ":")))
        return 2
    print(json.dumps(decision, sort_keys=True, separators=(",", ":")))
    return 0 if decision["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
