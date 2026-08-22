#!/usr/bin/env python3
"""Independent receipt verifier for the stabilization-first controller."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent
PLAN = ROOT / "docs" / "stabilization-plan.json"
CONTROLLER = ROOT / "scripts" / "stabilization_control.py"
SCHEMA = "codeclew-stabilization-receipt/1.0"
DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")


def digest_bytes(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def load_json(path: Path) -> object:
    with path.open("r", encoding="utf-8") as stream:
        return json.load(stream)


def git(*arguments: str) -> str:
    return subprocess.check_output(
        ("git", *arguments), cwd=ROOT, stderr=subprocess.DEVNULL, text=True
    ).strip()


def require_digest(value: object, label: str) -> str:
    if not isinstance(value, str) or not DIGEST_RE.fullmatch(value):
        raise ValueError(f"{label} is not a SHA-256 authority")
    return value


def verify(request: object) -> dict[str, object]:
    if not isinstance(request, dict):
        raise ValueError("verification request must be an object")
    expected_fields = {
        "checkId",
        "clean",
        "command",
        "commandDigest",
        "controllerDigest",
        "durationMillis",
        "environmentDigest",
        "exitCode",
        "inputDigest",
        "memoryBytes",
        "physicalCores",
        "planDigest",
        "sourceRevision",
        "stderrDigest",
        "stdoutDigest",
        "stepId",
        "tier",
        "verifierDigest",
    }
    if set(request) != expected_fields:
        raise ValueError("verification request fields differ from the closed schema")

    plan = load_json(PLAN)
    assert isinstance(plan, dict)
    checks = {item["id"]: item for item in plan["checks"]}
    tiers = {item["id"]: item for item in plan["tiers"]}
    check_id = request["checkId"]
    if check_id not in checks:
        raise ValueError("unknown check")
    check = checks[check_id]
    tier = tiers[check["tier"]]
    if request["stepId"] != check["step"] or request["tier"] != check["tier"]:
        raise ValueError("check step/tier authority mismatch")
    if request["command"] != check["command"]:
        raise ValueError("executed command differs from the plan")

    plan_digest = digest_bytes(canonical(plan))
    controller_digest = digest_bytes(CONTROLLER.read_bytes())
    verifier_digest = digest_bytes(Path(__file__).read_bytes())
    if request["planDigest"] != plan_digest:
        raise ValueError("plan digest mismatch")
    if request["controllerDigest"] != controller_digest:
        raise ValueError("controller digest mismatch")
    if request["verifierDigest"] != verifier_digest:
        raise ValueError("verifier digest mismatch")
    if request["commandDigest"] != digest_bytes(canonical(request["command"])):
        raise ValueError("command digest mismatch")
    for label in ("environmentDigest", "inputDigest", "stdoutDigest", "stderrDigest"):
        require_digest(request[label], label)
    if request["sourceRevision"] != git("rev-parse", "HEAD"):
        raise ValueError("source revision mismatch")

    physical = request["physicalCores"]
    memory = request["memoryBytes"]
    duration = request["durationMillis"]
    exit_code = request["exitCode"]
    clean = request["clean"]
    if not all(isinstance(value, int) and not isinstance(value, bool) for value in (physical, memory, duration, exit_code)):
        raise ValueError("numeric verification fields must be integers")
    if duration < 0 or physical < 0 or memory < 0 or not isinstance(clean, bool):
        raise ValueError("invalid verification measurement")

    qualified = (
        physical >= tier["minimumPhysicalCores"]
        and memory >= tier["minimumMemoryBytes"]
    )
    if not qualified:
        status = "UNQUALIFIED_HOST"
    elif tier["cleanRequired"] and not clean:
        status = "FAIL"
    elif exit_code != 0:
        status = "FAIL"
    elif duration > tier["budgetSeconds"] * 1000:
        status = "BUDGET_EXCEEDED"
    else:
        status = "PASS"

    receipt: dict[str, object] = {
        "checkId": check_id,
        "commandDigest": request["commandDigest"],
        "controllerDigest": controller_digest,
        "durationMillis": duration,
        "environmentDigest": request["environmentDigest"],
        "exitCode": exit_code,
        "inputDigest": request["inputDigest"],
        "planDigest": plan_digest,
        "schema": SCHEMA,
        "sourceRevision": request["sourceRevision"],
        "status": status,
        "stderrDigest": request["stderrDigest"],
        "stdoutDigest": request["stdoutDigest"],
        "stepId": request["stepId"],
        "tier": request["tier"],
        "verifierDigest": verifier_digest,
    }
    receipt["receiptDigest"] = digest_bytes(canonical(receipt))
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--request", type=Path, required=True)
    arguments = parser.parse_args()
    try:
        receipt = verify(load_json(arguments.request))
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(
            canonical(
                {
                    "error": str(error),
                    "schema": "codeclew-stabilization-verifier-error/1.0",
                }
            ).decode("utf-8")
        )
        return 2
    print(canonical(receipt).decode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
