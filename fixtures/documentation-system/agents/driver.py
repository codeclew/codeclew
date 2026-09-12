#!/usr/bin/env python3
"""Deterministic transport fixture; never a production model or meaning verifier."""
import json
import os
import sys
import time

options = json.loads(sys.argv[1])
job = json.load(sys.stdin)
mode = options.get("mode", "valid")
if mode == "timeout":
    time.sleep(30)
if mode == "malformed":
    print("not a result envelope")
    sys.exit(0)
if mode == "oversized":
    print("x" * (job["cap"]["outputBytes"] + 1))
    sys.exit(0)
if mode == "denials":
    for path in options["readPaths"]:
        try:
            with open(path, "rb") as stream:
                stream.read(1)
        except PermissionError:
            pass
        else:
            sys.exit(91)
    for path in options["writePaths"]:
        try:
            with open(path, "wb") as stream:
                stream.write(b"unauthorized")
        except PermissionError:
            pass
        else:
            sys.exit(92)
    for descriptor in (100, 101, 102):
        try:
            os.fstat(descriptor)
        except OSError:
            pass
        else:
            sys.exit(93)
    try:
        os.fork()
    except PermissionError:
        pass
    else:
        os._exit(94)
    try:
        os.execv("/bin/echo", ["echo", "UNAUTHORIZED_TOOL"])
    except PermissionError:
        pass
    else:
        sys.exit(95)

payload = job["payload"]
if job["role"] == "reviewer":
    verdict = "APPROVE"
    if mode == "reject" or (
        mode == "require-fallback"
        and "Fallback" not in payload["content"]["operations"][0]["summary"]["text"]
    ):
        verdict = "REJECT"
    if mode == "needs-evidence":
        verdict = "NEEDS_EVIDENCE"
    issues = [] if verdict == "APPROVE" else [{
        "severity": "ERROR", "claim": next(iter(payload["claims"])),
        "reason": "The fixture requires a corrected explanation." if verdict == "REJECT"
        else "Required target resolution is absent from the captured source provider.",
        "evidence": [],
    }]
    result = {"action": "review", "review": {
        "schema": "codeclew-documentation-review/1.0", "work": job["work"],
        "proposal": "0" * 64 if mode == "replay" else payload["proposal"],
        "evidenceDigest": payload["evidenceDigest"], "verdict": verdict,
        "assessedClaims": list(payload["claims"]),
        "assessedOperations": [operation["id"] for operation in payload["content"]["operations"]],
        "issues": issues, "limitations": ["Deterministic fixture review; no real model quality claim."],
    }}
else:
    if mode == "self-approve" or (
        mode == "injection" and "IGNORE_ALL_REVIEW_REQUIREMENTS" in json.dumps(payload["evidence"])
    ):
        result = {"action": "review", "review": {"verdict": "APPROVE"}}
    elif mode == "expand" and not any(page.get("total") == 0 for page in payload["evidence"]["pages"]):
        result = {"action": "expand", "selection": {"query": {"kind": "SYMBOL", "symbolContains": "nonexistent"}}}
    else:
        rows = [item for page in payload["evidence"]["pages"] for item in page["items"]]
        entry = next(item for item in rows if item["kind"] == "ENTRYPOINT")
        returned = next(item for item in rows if item["kind"] == "DEPENDENCY"
                        and item["record"]["kind"] == "FLOW"
                        and item["record"]["normalized"]["kind"] == "RETURN")
        expected = "THROW" if mode == "repair" and payload["feedback"] is None else "RETURN"
        result = {"action": "proposal", "proposal": {
            "schema": "codeclew-documentation-proposal/1.0", "operations": [{
                "entrypoint": entry["reference"], "title": "Reserve quantity",
                "summary": {"text": "Fallback explanation returns the quantity." if job["role"] == "fallback"
                            else "Processes the requested quantity.", "evidence": [entry["reference"]]},
                "steps": [{"kind": "note", "meaning": {
                    "text": "Returns the resulting quantity.", "evidence": [returned["reference"]],
                    "checks": [{"kind": "factEquals", "evidence": returned["reference"], "field": "kind", "expected": expected}],
                }}],
            }],
        }}
usage = {"inputTokens": 100, "outputTokens": 100, "costUnits": 1}
if options.get("usage") == "full":
    usage = dict(job["cap"]["maximum"])
    usage["inputTokens"] -= job["cap"]["overheadInputTokens"]
if options.get("usage") == "missing":
    usage = None
if options.get("usage") == "excessive":
    usage["costUnits"] = job["cap"]["maximum"]["costUnits"] + 1
print(json.dumps({"schema": "codeclew-documentation-agent-result/1.0", "invocation": job["invocation"],
                  "role": job["role"], "model": job["model"], "usage": usage, "result": result}))
