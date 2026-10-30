#!/usr/bin/env python3
"""Read-only validation of caller-trusted native READ/citation evidence.

No discovery, acquisition, subprocesses or private store reads. Digests establish
internal consistency, not producer authenticity, semantic truth or fresh runtime
behavior. The existing READ/page/source-part proof contract remains authoritative.
"""
from __future__ import annotations
import copy
import hashlib
import json
from pathlib import Path

def encode(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True) + "\n"

class Stop(Exception):
    def __init__(self, status, *, exit_code=3, **details):
        self.status, self.exit_code, self.details = status, exit_code, details

def limitations(value):
    if isinstance(value, dict):
        result = {}
        for key, child in value.items():
            if key.lower().endswith(("authority", "coverage", "boundaries", "unresolved", "limits")) or key in (
                "omitted", "truncated", "targetStatus", "targetResolution", "sourceMapping",
                "bodyStatus", "resolution", "evidenceValidation",
            ):
                result[key] = child
            elif retained := limitations(child):
                result[key] = retained
        return result
    if isinstance(value, list):
        return {str(i): retained for i, child in enumerate(value) if (retained := limitations(child))}
    return {}

def digest(value):
    return "sha256:" + hashlib.sha256(encode(value)[:-1].encode("utf-8")).hexdigest()


def source_reference_pages(pages, scope, work, source_id, reference):
    """Validate complete native reference delivery, including recoverable omission.

    A SOURCE row or explicit native size omission binds the reference. Parts
    recover the latter; neither is a declaration or whole-file capture proof.
    """
    membership, coverage, selected, cursors, consumed = None, [], [], set(), 0
    for index, page in enumerate(pages):
        if (not isinstance(page, dict) or page.get("schema") != "codeclew-documentation-work-page/1.0"
                or page.get("work") != work or page.get("snapshot") != scope["snapshot"]
                or page.get("subject") != "service:" + scope["service"]
                or not isinstance(page.get("items"), list) or not isinstance(page.get("omitted"), list)
                or "nextCursor" not in page
                or page.get("receiptDigest") != digest({k:v for k,v in page.items() if k != "receiptDigest"})):
            raise Stop("SOURCE_REFERENCE_PAGE_MISMATCH", exit_code=2)
        current = {k:page.get(k) for k in ("contextDigest", "inputDigest", "membershipDigest", "total", "authority")}
        if (type(current["total"]) is not int or current["total"] < 1
                or membership is not None and current != membership):
            raise Stop("SOURCE_REFERENCE_MEMBERSHIP_CHANGED", exit_code=2)
        membership = current
        cursor = page["nextCursor"]
        if (cursor is not None and (not isinstance(cursor, str) or not cursor or cursor in cursors)
                or (index < len(pages)-1) != (cursor is not None)):
            raise Stop("SOURCE_REFERENCE_CONTINUATION_INCOMPLETE", exit_code=2)
        cursors.add(cursor)
        for section in ("items", "omitted"):
            for offset, item in enumerate(page[section]):
                consumed += 1
                if not isinstance(item, dict):
                    raise Stop("INVALID_SOURCE_REFERENCE_ROW", exit_code=2)
                if section == "items" and item.get("kind") == "COVERAGE" and item.get("id") == scope["service"]:
                    coverage.append(item)
                elif (item.get("kind") == "SOURCE" and item.get("id") == source_id
                        and item.get("reference") == reference
                        and (section == "items" or item.get("reason") == "ITEM_EXCEEDS_WORK_BYTE_BUDGET")):
                    if section == "items" and not isinstance(item.get("record"), dict):
                        raise Stop("INVALID_SOURCE_REFERENCE_RECORD", exit_code=2)
                    selected.append((item.get("record") if section == "items" else None,
                                     index, f"/{section}/{offset}"))
                else:
                    raise Stop("SOURCE_REFERENCE_IDENTITY_OR_OMISSION_MISMATCH", exit_code=2)
    if (not pages or consumed != membership["total"] or len(coverage) != 1 or len(selected) != 1
            or not isinstance(coverage[0].get("revision"), str) or not coverage[0]["revision"]):
        raise Stop("SOURCE_REFERENCE_DELIVERY_INCOMPLETE", exit_code=2)
    record, index, pointer = selected[0]
    if record is not None and (not isinstance(record, dict) or record.get("id") != source_id
            or record.get("service") != scope["service"] or record.get("revision") != coverage[0]["revision"]):
        raise Stop("SOURCE_REFERENCE_RECORD_MISMATCH", exit_code=2)
    return record, coverage[0], index, pointer


class ReadProof:
    def __init__(self, path, scope, work):
        self.path = path.resolve(strict=True); self.root = self.path.parent
        self.packet = json.loads(self.path.read_bytes())
        if (not isinstance(self.packet, dict) or self.packet.get("schema") != "codeclew-auto-doc-evidence/1.0"
                or self.packet.get("kind") != "READ" or self.packet.get("work") != work
                or not isinstance(self.packet.get("scope"), dict)
                or any(self.packet.get("scope", {}).get(k) != scope[k] for k in ("service", "snapshot"))):
            raise Stop("REUSE_SCOPE_MISMATCH", exit_code=2)
        summary = json.loads(self.file("read-summary.json"))
        if not isinstance(summary, dict) or summary.get("packetDigest") != digest(self.packet):
            raise Stop("REUSE_PACKET_DIGEST_MISMATCH", exit_code=2)
        self.scope, self.work = scope, work
        manifest = "evidence-receipts.json" if (self.root / "evidence-receipts.json").is_file() else "receipts.json"
        receipts = json.loads(self.file(manifest))
        if not isinstance(receipts, list):
            raise Stop("INVALID_REUSE_RECEIPTS", exit_code=2)
        self.records, self.payloads, self.used, self.remap = {}, {}, set(), {}
        for receipt in receipts:
            if (not isinstance(receipt, dict) or not isinstance(receipt.get("command"), list)
                    or receipt["command"][1:4] not in [["docs", "work", "read"], ["docs", "work", "read-part"],
                                                      ["docs", "work", "read-dependency-part"]]):
                raise Stop("INVALID_REUSE_RECEIPT_COMMAND", exit_code=2)
            for stream in ("stdout", "stderr"):
                raw = self.file(receipt[stream])
                if hashlib.sha256(raw).hexdigest() != receipt.get(stream + "Sha256"):
                    raise Stop("REUSE_NATIVE_STREAM_DIGEST_MISMATCH", exit_code=2, stream=receipt[stream])
            name = receipt["stdout"]
            if name in self.records and self.records[name] != receipt:
                raise Stop("CONFLICTING_REUSE_RECEIPTS", exit_code=2)
            self.records[name] = receipt
            self.payloads[name] = json.loads(self.file(name))

    def file(self, name):
        relative = Path(name)
        if relative.is_absolute() or not relative.parts or any(p in (".", "..") for p in relative.parts):
            raise Stop("UNSAFE_REUSE_ARTIFACT_PATH", exit_code=2)
        path = self.root
        for part in relative.parts:
            path = path / part
            if path.is_symlink():
                raise Stop("UNSAFE_REUSE_ARTIFACT_PATH", exit_code=2)
        if not path.is_file():
            raise Stop("REUSE_ARTIFACT_UNAVAILABLE", exit_code=2, artifact=name)
        return path.read_bytes()

    def payload(self, name):
        if name not in self.records or self.records[name]["exitCode"] != 0:
            raise Stop("REUSE_NATIVE_PROOF_UNAVAILABLE", exit_code=2, artifact=name)
        value = self.payloads[name]
        if (not isinstance(value, dict) or value.get("work") != self.work
                or value.get("snapshot") != self.scope["snapshot"]):
            raise Stop("REUSE_NATIVE_SCOPE_MISMATCH", exit_code=2)
        self.used.add(name)
        return value

    def parts(self, names, identity, reference=None):
        parts = [self.payload(name) for name in names]
        if not parts:
            raise Stop("REUSE_PART_PROOF_UNAVAILABLE", exit_code=2)
        chunks, offset, first = [], 0, None
        for part in parts:
            source = reference is not None
            schema = "codeclew-documentation-source-part/1.0" if source else "codeclew-documentation-dependency-part/1.0"
            total_key = "totalTextBytes" if source else "totalRecordBytes"
            if (part.get("schema") != schema or part.get("sourceId" if source else "id") != identity
                    or source and part.get("reference") != reference
                    or not source and part.get("kind") != "DEPENDENCY"
                    or not isinstance(part.get("text"), str)):
                raise Stop("REUSE_PART_IDENTITY_MISMATCH", exit_code=2)
            binding = {k: part.get(k) for k in ("recordDigest", total_key, "source", "authority")}
            if first is not None and binding != first:
                raise Stop("REUSE_PART_IDENTITY_CHANGED", exit_code=2)
            first = binding; text = part["text"].encode("utf-8")
            end, total = part.get("endByte"), part.get(total_key)
            if (type(part.get("startByte")) is not int or part["startByte"] != offset
                    or type(end) is not int or end != offset + len(text) or type(total) is not int or not end <= total
                    or not text and total > 0 or "nextCursor" not in part
                    or part.get("fragmentDigest") != "sha256:" + hashlib.sha256(text).hexdigest()
                    or part.get("receiptDigest") != digest({k: v for k, v in part.items() if k != "receiptDigest"})):
                raise Stop("REUSE_PART_RANGE_OR_DIGEST_MISMATCH", exit_code=2)
            chunks.append(part["text"]); offset = end
            if (offset < total and not isinstance(part["nextCursor"], str)) or (offset == total and part["nextCursor"] is not None):
                raise Stop("REUSE_PART_CONTINUATION_MISMATCH", exit_code=2)
        if offset != first[total_key] or parts[-1]["nextCursor"] is not None:
            raise Stop("REUSE_PARTS_INCOMPLETE", exit_code=2)
        text = "".join(chunks)
        value = {**first["source"], "text": text} if reference is not None else json.loads(text)
        if digest(value) != first["recordDigest"]:
            raise Stop("REUSE_RECORD_DIGEST_MISMATCH", exit_code=2)
        return value

    def declaration(self, outcome):
        declaration = outcome["declaration"]; raw = declaration["raw"]; name = raw["stdout"]
        if name in self.payloads:
            page = self.payload(name)
            if page.get("subject") != "service:" + self.scope["service"]:
                raise Stop("REUSE_NATIVE_SCOPE_MISMATCH", exit_code=2)
            selected = page
            for part in raw["pointer"].strip("/").split("/"):
                selected = selected[int(part)] if isinstance(selected, list) else selected[part]
            if selected.get("kind") != "DEPENDENCY" or selected.get("id") != declaration["id"]:
                raise Stop("REUSE_DECLARATION_PROOF_MISMATCH", exit_code=2)
            return selected["record"], selected.get("sourceReferences", []), selected.get("reference")
        record = json.loads(self.file(name)); self.used.add(name)
        candidates = [(name, p) for name, p in self.payloads.items() if isinstance(p, dict)
                      and p.get("schema") == "codeclew-documentation-dependency-part/1.0" and p.get("id") == declaration["id"]]
        unique = {}
        for name, part in candidates:
            start = part["startByte"]
            if start in unique and unique[start][1] != part:
                raise Stop("CONFLICTING_REUSE_PARTS", exit_code=2)
            unique[start] = (name, part)
        recovered = self.parts([unique[k][0] for k in sorted(unique)], declaration["id"])
        if recovered != record:
            raise Stop("REUSE_RECOVERED_DECLARATION_CHANGED", exit_code=2)
        for name, page in self.payloads.items():
            if not isinstance(page, dict) or page.get("schema") != "codeclew-documentation-work-page/1.0":
                continue
            for row in page.get("items", []):
                nav = row.get("record", {})
                if (row.get("kind") == "CALLABLE_SUMMARY" and row.get("id") == declaration["id"]
                        and nav.get("provenance", {}).get("observationDigest") == record["digest"]
                        and nav.get("symbol") == record["symbol"] and nav.get("service") == self.scope["service"]):
                    self.payload(name)
                    return record, nav["sourceReferences"], nav.get("fullRecordReference")
        raise Stop("REUSE_NAVIGATION_PROOF_UNAVAILABLE", exit_code=2)

    def find(self, row, selection):
        candidates = [o for o in self.packet.get("outcomes", []) if o.get("status") == "DECLARATION_SOURCE_RETURNED"
                      and o.get("declaration", {}).get("id") == row["id"]]
        if not candidates:
            return None
        if any(o["declaration"].get("sourceIds") != row["sourceIds"]
               or o["declaration"].get("sourceReferences") != row.get("sourceReferences", [])
               or o["declaration"].get("symbol") != row["symbol"]
               or o["declaration"].get("observationDigest") != row.get("observationDigest") for o in candidates):
            raise Stop("REUSE_BINDING_CHANGED", exit_code=2)
        ids = row["sourceIds"]; references = row.get("sourceReferences", [])
        source_id, reference = selection.get("sourceId"), selection.get("sourceReference")
        if source_id is None and len(set(ids)) == 1:source_id = ids[0]
        if reference is None and len(set(references)) == 1:reference = references[0]
        if source_id is None:
            raise Stop("EXPLICIT_SOURCE_CHOICE_REQUIRED", sourceIds=ids)
        if reference is None:
            raise Stop("EXPLICIT_SOURCE_REFERENCE_CHOICE_REQUIRED", sourceReferences=references)
        matches = [o for o in candidates if o["declaration"]["selectedSourceId"] == source_id
                   and o["declaration"]["selectedSourceReference"] == reference]
        if not matches:return None
        if len(matches) != 1:raise Stop("AMBIGUOUS_REUSE_BINDING", exit_code=2)
        outcome = matches[0]; decl = outcome["declaration"]
        record, refs, reference_id = self.declaration(outcome)
        if (record.get("kind") != "SYMBOL" or record.get("service") != self.scope["service"]
                or any(record.get(k) != row.get(k) for k in ("id", "symbol", "sourceIds"))
                or record.get("digest") != row.get("observationDigest") or record.get("digest") != decl.get("observationDigest")
                or reference_id != row.get("reference")
                or any(decl.get(k) != record.get(k) for k in ("id", "symbol", "service", "sourceIds"))
                or source_id not in record["sourceIds"] or reference not in refs
                or decl.get("sourceIds") != record["sourceIds"] or decl.get("sourceReferences") != refs):
            raise Stop("REUSE_BINDING_CHANGED", exit_code=2)
        for proof in outcome["sourceParts"]:
            part = self.payload(proof["stdout"])
            if any(proof.get(k) != part.get(k) for k in ("startByte", "endByte", "recordDigest", "receiptDigest", "authority", "nextCursor")):
                raise Stop("REUSE_PART_PROOF_CHANGED", exit_code=2)
        for proof in outcome["nativePages"]:
            page = self.payload(proof["raw"])
            if (proof.get("metadata") != {k: v for k, v in page.items() if k != "items"}
                    or proof.get("coverage") != [r for r in page["items"] if r.get("kind") == "COVERAGE"]):
                raise Stop("REUSE_PAGE_PROOF_CHANGED", exit_code=2)
        source = self.parts([p["stdout"] for p in outcome["sourceParts"]], source_id, reference)
        expected = {**outcome["source"], "text": self.packet["texts"][outcome["textKey"]]}
        if (source != expected or digest(source) != outcome["sourceRecordDigest"]
                or source.get("service") != self.scope["service"] or source.get("revision") != row["revision"]
                or source.get("textDigest") != outcome["textKey"]
                or source["textDigest"] != "sha256:" + hashlib.sha256(source["text"].encode()).hexdigest()):
            raise Stop("REUSE_SOURCE_METADATA_OR_TEXT_CHANGED", exit_code=2)
        doc = record.get("normalized", {}).get("documentation")
        status = doc.get("bodyStatus") if isinstance(doc, dict) else None
        if decl.get("bodyStatus") != status:
            raise Stop("REUSE_DECLARATION_METADATA_CHANGED", exit_code=2)
        norm = record["normalized"]
        descriptor = {k: norm[k] for k in ("scope", "kind", "name", "ownerIdentity", "declarationKind", "syntaxKind") if k in norm}
        if decl.get("limits") != limitations(norm) or decl.get("descriptor") != descriptor:
            raise Stop("REUSE_DECLARATION_METADATA_CHANGED", exit_code=2)
        return copy.deepcopy(outcome), source

    def find_source(self, row, selection):
        candidates = [o for o in self.packet.get("outcomes", [])
                      if o.get("status") == "RETAINED_SOURCE_RETURNED" and o.get("sourceBinding", {}).get("id") == row["id"]]
        if not candidates:return None
        if len(candidates) != 1:raise Stop("AMBIGUOUS_REUSE_SOURCE", exit_code=2)
        outcome = candidates[0]; binding = outcome["sourceBinding"]
        if {k:v for k,v in binding.items() if k != "raw"} != {k:v for k,v in row.items() if k != "raw"}:
            raise Stop("REUSE_SOURCE_BINDING_CHANGED", exit_code=2)
        page = self.payload(binding["raw"]["stdout"])
        if page.get("subject") != "service:" + self.scope["service"]:
            raise Stop("REUSE_NATIVE_SCOPE_MISMATCH", exit_code=2)
        selected = page
        for part in binding["raw"]["pointer"].strip("/").split("/"):
            selected = selected[int(part)] if isinstance(selected, list) else selected[part]
        record = selected.get("record", {})
        if outcome.get("fileBinding", {}).get("wholeFiles") and record.get("wholeFileCapture") != "EXACT_RETAINED_FILE":
            raise Stop("REUSE_WHOLE_FILE_PROOF_MISSING", exit_code=2)
        direct = binding.get("kind") == "SOURCE_REFERENCE"
        revision = binding["source"].get("revision")
        if direct:
            if (selection != {"sourceId":binding["id"], "sourceReference":binding["reference"]}
                    or outcome.get("fileBinding") is not None):
                raise Stop("REUSE_SOURCE_REFERENCE_SELECTION_CHANGED", exit_code=2)
            names = binding.get("pages", [])
            record, coverage, index, pointer = source_reference_pages(
                [self.payload(name) for name in names], self.scope, self.work, binding["id"], binding["reference"])
            revision = coverage["revision"]
            if binding["raw"] != {"stdout":names[index], "pointer":pointer}:
                raise Stop("REUSE_SOURCE_REFERENCE_PROOF_CHANGED", exit_code=2)
            if record is not None and (digest(record) != binding["recordDigest"]
                    or {k:v for k,v in record.items() if k != "text"} != binding["source"]):
                raise Stop("REUSE_SOURCE_REFERENCE_RECORD_CHANGED", exit_code=2)
        elif (selected.get("kind") != "SOURCE_NAVIGATION" or selected.get("id") != binding["id"]
                or record.get("source") != binding["source"] or record.get("recordDigest") != binding["recordDigest"]
                or record.get("fullRecordReference") != binding["reference"] or record.get("textDelivered") is not False):
            raise Stop("REUSE_SOURCE_NAVIGATION_CHANGED", exit_code=2)
        source = self.parts([p["stdout"] for p in outcome["sourceParts"]], binding["id"], binding["reference"])
        expected = {**outcome["source"], "text": self.packet["texts"][outcome["textKey"]]}
        if (source != expected or {k:v for k,v in source.items() if k != "text"} != binding["source"]
                or digest(source) != binding["recordDigest"] or digest(source) != outcome["sourceRecordDigest"]
                or source.get("service") != self.scope["service"] or source.get("revision") != revision
                or not direct and source.get("file") != selection.get("file")
                or source.get("textDigest") != outcome["textKey"]
                or source["textDigest"] != "sha256:" + hashlib.sha256(source["text"].encode()).hexdigest()):
            raise Stop("REUSE_SOURCE_METADATA_OR_TEXT_CHANGED", exit_code=2)
        for proof in outcome["sourceParts"]:
            part = self.payload(proof["stdout"])
            if any(proof.get(k) != part.get(k) for k in ("startByte", "endByte", "recordDigest", "receiptDigest", "authority", "nextCursor")):
                raise Stop("REUSE_PART_PROOF_CHANGED", exit_code=2)
        return copy.deepcopy(outcome), source
