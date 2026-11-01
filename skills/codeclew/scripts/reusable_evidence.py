#!/usr/bin/env python3
"""Validate and retain an owned earlier native READ packet for same-input repair.

Checksums establish internal consistency of caller-trusted native artifacts, not
producer authenticity. An attacker able to replace packet, receipts and digests
can forge a matching bundle. This is retained evidence, never current-source or
runtime re-verification, and never native proposal/publication approval.
"""
from __future__ import annotations
import copy
import hashlib
import json
from pathlib import Path
import runpy

B = globals().get("SHARED_BOUND_SOURCE") or runpy.run_path(str(Path(__file__).with_name("bound_source.py")))
Stop, digest, encode = (B[k] for k in ("Stop", "digest", "encode"))
C = runpy.run_path(str(Path(__file__).with_name("citation_evidence.py")))


class ReusableEvidence:
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
        self.validated_native_batches, self.validated_source_batches = set(), {}
        for receipt in receipts:
            if (not isinstance(receipt, dict) or not isinstance(receipt.get("command"), list)
                    or receipt["command"][1:4] not in [["docs", "work", "read"], ["docs", "work", "read-part"],
                                                      ["docs", "work", "read-dependency-part"], ["docs", "work", "read-batch"]]):
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
        try:
            return C["native_payload"](self, name)
        except C["Stop"] as error:
            raise Stop(error.status, exit_code=error.exit_code, **error.details) from error

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

    def source(self, outcome, identity, reference):
        try:
            return C["ReadProof"].source(self, outcome, identity, reference)
        except C["Stop"] as error:
            raise Stop(error.status, exit_code=error.exit_code, **error.details) from error

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
        source = self.source(outcome, source_id, reference)
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
        if decl.get("limits") != B["limitations"](norm) or decl.get("descriptor") != descriptor:
            raise Stop("REUSE_DECLARATION_METADATA_CHANGED", exit_code=2)
        return copy.deepcopy(outcome), source

    def retain(self, outcome, target):
        def transfer(name):
            base, marker, index = name.partition("#")
            if marker:
                return transfer(base) + "#" + index
            if name not in self.remap:
                raw = self.file(name)
                suffix = Path(name).suffix
                mapped = "retained-" + hashlib.sha256((name + hashlib.sha256(raw).hexdigest()).encode()).hexdigest()[:24] + suffix
                (target / mapped).write_bytes(raw); self.remap[name] = mapped
                if name in self.records:transfer(self.records[name]["stderr"])
            return self.remap[name]
        for name in self.used:
            transfer(name)
            if name in self.records:transfer(self.records[name]["stderr"])
        binding = outcome.get("sourceBinding", outcome.get("declaration"))
        binding["raw"]["stdout"] = transfer(binding["raw"]["stdout"])
        for part in outcome["sourceParts"]:part["stdout"] = transfer(part["stdout"])
        for page in outcome["nativePages"]:page["raw"] = transfer(page["raw"])
        if outcome.get("sourceBatch"):
            batch = outcome["sourceBatch"]
            batch["pages"] = [transfer(name) for name in batch["pages"]]
            batch["raw"]["stdout"] = transfer(batch["raw"]["stdout"])
        outcome["evidenceReused"] = True
        return outcome

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
        if (selected.get("kind") != "SOURCE_NAVIGATION" or selected.get("id") != binding["id"]
                or record.get("source") != binding["source"] or record.get("recordDigest") != binding["recordDigest"]
                or record.get("fullRecordReference") != binding["reference"] or record.get("textDelivered") is not False):
            raise Stop("REUSE_SOURCE_NAVIGATION_CHANGED", exit_code=2)
        source = self.source(outcome, binding["id"], binding["reference"])
        expected = {**outcome["source"], "text": self.packet["texts"][outcome["textKey"]]}
        if (source != expected or {k:v for k,v in source.items() if k != "text"} != binding["source"]
                or digest(source) != binding["recordDigest"] or digest(source) != outcome["sourceRecordDigest"]
                or source.get("service") != self.scope["service"] or source.get("file") != selection.get("file")
                or source.get("textDigest") != outcome["textKey"]
                or source["textDigest"] != "sha256:" + hashlib.sha256(source["text"].encode()).hexdigest()):
            raise Stop("REUSE_SOURCE_METADATA_OR_TEXT_CHANGED", exit_code=2)
        for proof in outcome["sourceParts"]:
            part = self.payload(proof["stdout"])
            if any(proof.get(k) != part.get(k) for k in ("startByte", "endByte", "recordDigest", "receiptDigest", "authority", "nextCursor")):
                raise Stop("REUSE_PART_PROOF_CHANGED", exit_code=2)
        return copy.deepcopy(outcome), source

    def retained_receipts(self):
        return [{**r, "stdout": self.remap[name], "stderr": self.remap[r["stderr"]]}
                for name, r in self.records.items() if name in self.remap]
