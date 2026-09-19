#!/usr/bin/env python3
"""Deterministic transport fixture; never a production model or meaning verifier."""
import json
import hashlib
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
if "requireEvidenceText" in options:
    assert options["requireEvidenceText"] in json.dumps(payload["evidence"])
if "requireEvidenceKinds" in options:
    kinds = {item["kind"] for page in payload["evidence"]["pages"] for item in page["items"]}
    assert set(options["requireEvidenceKinds"]) <= kinds
if "requireContextProfile" in options:
    assert all(page.get("contextProfile") == options["requireContextProfile"]
               for page in payload["evidence"]["pages"])
if job["role"] == "reviewer":
    if "outputContract" in payload:
        contract = payload["outputContract"]
        schema = contract["outputSchema"]
        encoded = json.dumps(schema, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()
        assert contract["outputSchemaDigest"] == "sha256:" + hashlib.sha256(encoded).hexdigest()
        props = schema["$defs"]["reviewAction"]["properties"]["review"]["properties"]
        assert props["work"]["const"] == job["work"]
        assert props["proposal"]["const"] == payload["proposal"]
        assert props["evidenceDigest"]["const"] == payload["evidenceDigest"]
        for name, expected in (
            ("assessedClaims", set(payload["claims"])),
            ("assessedOperations", {op["id"] for op in payload["content"]["operations"]}),
        ):
            field = props[name]
            assert field["minItems"] == field["maxItems"] == len(expected)
            assert field["uniqueItems"] is True
            assert set(field["items"]["enum"]) == expected if expected else field["items"] is False
        assert {item["$ref"] for item in schema["oneOf"]} == {"#/$defs/reviewAction", "#/$defs/expandAction"}
        assert schema["$defs"]["expandAction"]["properties"]["action"]["const"] == "expand"
        delivered = {item["reference"] for page in payload["evidence"]["pages"] for item in page["items"] if "reference" in item}
        issue_evidence = props["issues"]["items"]["properties"]["evidence"]["items"]
        assert set(issue_evidence["enum"]) == delivered if delivered else issue_evidence is False
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
    if mode == "object-coverage":
        result["review"]["assessedClaims"] = [{"claim": claim, "supported": True} for claim in payload["claims"]]
else:
    if mode.startswith("section-"):
        contract = payload["outputContract"]
        assert contract["schema"] == "section-summary/1.0"
        assert contract["work"] == job["work"]
        assert contract["deliveredDigest"].startswith("sha256:")
        assert "proposalSchema" not in payload and "previousProposal" not in payload
        if payload["feedback"] is not None:
            assert "summary" in payload["previousSection"]
            assert "operations" not in payload["previousSection"]
        rows = [item for page in payload["evidence"]["pages"] for item in page["items"]]
        section = next(item for item in rows
                       if item["kind"] == "SECTION" and item["id"] == "section-entities")
        assert contract["targetReference"] == section["reference"]
        delivered = {item["reference"] for item in rows
                     if "evidence" in item.get("referenceRoles", [])}

        def evidence_enums(value):
            if isinstance(value, dict):
                claim_evidence = value.get("properties", {}).get("evidence")
                if claim_evidence:
                    yield set(claim_evidence["items"]["enum"])
                for nested in value.values():
                    yield from evidence_enums(nested)
            elif isinstance(value, list):
                for nested in value:
                    yield from evidence_enums(nested)

        enums = list(evidence_enums(contract["outputSchema"]))
        assert enums and all(allowed == delivered for allowed in enums)
        schema_bytes = json.dumps(contract["outputSchema"], sort_keys=True,
                                  ensure_ascii=False, separators=(",", ":")).encode()
        assert contract["outputSchemaDigest"] == "sha256:" + hashlib.sha256(schema_bytes).hexdigest()
        sources = [item for item in rows if item["kind"] == "SOURCE"]
        if mode == "section-expand" and not sources:
            linked = next(ref for item in rows for ref in item.get("sourceReferences", [])
                          if ref not in delivered)
            result = {"action": "expand", "selection": {"references": [linked]}}
        else:
            evidence_ref = options.get("evidenceRef") or (
                sources[0]["reference"] if sources else next(
                    item["reference"] for item in rows
                    if item["kind"] == "DEPENDENCY" and item["record"]["kind"] == "SYMBOL"))
            if mode == "section-unseen":
                assert evidence_ref not in delivered
            time.sleep(options.get("delayMs", 0) / 1000)
            draft = {"title": "Domain entities", "summary": {
                "text": ("Fallback explanation of quantity declarations." if job["role"] == "fallback"
                         else "The retained declaration defines quantity handling in Orders."),
                "evidence": [evidence_ref],
                "uncertainty": "Syntax does not establish business ownership or runtime behavior."},
                "uncertainties": ["Deterministic structural fixture, not a model quality claim."]}
            if options.get("bindSchemaInTitle"):
                draft["title"] = contract["outputSchemaDigest"]
            if options.get("bindRequestBytesInTitle"):
                draft["title"] = str(len(json.dumps(job, sort_keys=True, ensure_ascii=False,
                                                  separators=(",", ":")).encode()))
            if "invalidField" in options:
                draft[options["invalidField"]] = {section["reference"]: "Must not redirect or suppress content."}
            result = {"action": "section", "section": draft}
    elif mode == "self-approve" or (
        mode == "injection" and "IGNORE_ALL_REVIEW_REQUIREMENTS" in json.dumps(payload["evidence"])
    ):
        result = {"action": "review", "review": {"verdict": "APPROVE"}}
    elif mode == "expand" and not any(page.get("total") == 0 for page in payload["evidence"]["pages"]):
        result = {"action": "expand", "selection": {"query": {"kind": "SYMBOL", "symbolContains": "nonexistent"}}}
    elif "proposal" in options:
        result = {"action": "proposal", "proposal": options["proposal"]}
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
