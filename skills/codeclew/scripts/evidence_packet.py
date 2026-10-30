#!/usr/bin/env python3
"""Automatic-doc evidence: scoped declarations/files or explicit native SOURCE references.

Two external invocations replace manual paging and receipt reconstruction. All
code evidence comes through supported native Work/context APIs. Native Work and
read-ledger writes require a caller-selected disposable documentation root.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
from pathlib import Path
import runpy
import shutil
import time


B = runpy.run_path(str(Path(__file__).with_name("bound_source.py")))
BoundSource, Catalogue, Stop, encode, digest, limitations = (B[k] for k in
    ("BoundSource", "Catalogue", "Stop", "encode", "digest", "limitations"))
SCHEMA = "codeclew-auto-doc-evidence/1.0"


class Packet(Catalogue):
    def __init__(self, args):
        super().__init__(args)
        self.rows, self.omissions, self.failures, self.proofs = [], [], [], []
        self.source_cache = {}
        self.reuse = None

    def work_page(self, label, selection):
        page, receipt = self.call(label, ["docs", "work", "read", "--root", self.args.root,
                                          "--work", self.work], input_value=selection)
        self.validate_page(page, "codeclew-documentation-work-page/1.0")
        if page.get("work") != self.work:
            raise Stop("NATIVE_WORK_MISMATCH", exit_code=2)
        return page, receipt

    def dependency(self, identity):
        cursor, seen, offset, fragments, first = None, set(), 0, [], None
        while True:
            request = {"schema": "codeclew-documentation-dependency-part-request/1.0",
                       "kind": "DEPENDENCY", "id": identity}
            if cursor is not None:
                request["cursor"] = cursor
            part, receipt = self.call("dependency-part", ["docs", "work", "read-dependency-part",
                "--root", self.args.root, "--work", self.work], input_value=request)
            if (part.get("schema") != "codeclew-documentation-dependency-part/1.0"
                    or part.get("work") != self.work or part.get("snapshot") != self.args.snapshot
                    or part.get("kind") != "DEPENDENCY" or part.get("id") != identity
                    or not isinstance(part.get("text"), str) or "nextCursor" not in part):
                raise Stop("DEPENDENCY_PART_IDENTITY_MISMATCH", exit_code=2, rawReceipt=receipt)
            binding = {k: part.get(k) for k in ("recordDigest", "totalRecordBytes", "authority")}
            if first is not None and binding != first:
                raise Stop("DEPENDENCY_PART_IDENTITY_CHANGED", exit_code=2)
            first = binding
            text = part["text"].encode("utf-8")
            end, total = part.get("endByte"), part.get("totalRecordBytes")
            if (type(part.get("startByte")) is not int or part["startByte"] != offset
                    or type(end) is not int or end != offset + len(text) or not text
                    or type(total) is not int or not end <= total
                    or part.get("fragmentDigest") != "sha256:" + hashlib.sha256(text).hexdigest()
                    or part.get("receiptDigest") != digest({k: v for k, v in part.items() if k != "receiptDigest"})):
                raise Stop("DEPENDENCY_PART_RANGE_OR_DIGEST_MISMATCH", exit_code=2, rawReceipt=receipt)
            fragments.append(part["text"])
            self.proofs.append({"kind": "DEPENDENCY_PART", "id": identity, "raw": receipt["stdout"],
                                "startByte": offset, "endByte": end, "recordDigest": part["recordDigest"]})
            offset, cursor = end, part["nextCursor"]
            if cursor is None:
                if offset != total:
                    raise Stop("DEPENDENCY_PART_INCOMPLETE", exit_code=2)
                break
            if not isinstance(cursor, str) or not cursor or cursor in seen or offset >= total:
                raise Stop("INVALID_DEPENDENCY_CONTINUATION", exit_code=2)
            seen.add(cursor)
        text = "".join(fragments)
        if "sha256:" + hashlib.sha256(text.encode()).hexdigest() != first["recordDigest"]:
            raise Stop("DEPENDENCY_RECORD_DIGEST_MISMATCH", exit_code=2)
        try:
            record = json.loads(text)
        except ValueError as error:
            raise Stop("INVALID_DEPENDENCY_RECORD", exit_code=2) from error
        if not isinstance(record, dict):
            raise Stop("INVALID_DEPENDENCY_RECORD", exit_code=2)
        if digest(record) != first["recordDigest"]:
            raise Stop("DEPENDENCY_CANONICAL_DIGEST_MISMATCH", exit_code=2)
        path = f"recovered-{len(self.proofs):04d}.json"
        (self.args.output_dir / path).write_text(text, encoding="utf-8")
        return record, {"stdout": path, "pointer": ""}

    def navigation(self, record):
        cursor, seen = None, set()
        while True:
            query = {"query": {"kind": "SYMBOL", "projection": "NAVIGATION", "symbolContains": record["symbol"]}}
            if cursor is not None:
                query["cursor"] = cursor
            page, _ = self.work_page("binding-navigation", query)
            matches = [r for r in page["items"] if r.get("kind") == "CALLABLE_SUMMARY" and r.get("id") == record["id"]]
            if len(matches) > 1:
                raise Stop("AMBIGUOUS_NATIVE_NAVIGATION", exit_code=2)
            if matches:
                row = matches[0].get("record", {})
                if (row.get("service") != self.args.service or row.get("symbol") != record["symbol"]
                        or row.get("provenance", {}).get("observationDigest") != record["digest"]
                        or row.get("provenance", {}).get("dependencyId") != record["id"]
                        or not isinstance(row.get("sourceReferences"), list)
                        or not all(isinstance(r, str) and r for r in row["sourceReferences"])):
                    raise Stop("NATIVE_NAVIGATION_BINDING_CHANGED", exit_code=2)
                return row["sourceReferences"]
            cursor = page["nextCursor"]
            if cursor is None:
                raise Stop("SELECTED_NAVIGATION_NOT_RETURNED", omitted=page["omitted"], id=record["id"])
            if cursor in seen:
                raise Stop("NON_PROGRESSING_NATIVE_CONTINUATION", exit_code=2)
            seen.add(cursor)

    def project(self, record, reference, sources, raw):
        if (not isinstance(record, dict) or record.get("kind") != "SYMBOL"
                or record.get("service") != self.args.service or not isinstance(record.get("id"), str)
                or not isinstance(record.get("symbol"), str) or not isinstance(record.get("normalized"), dict)
                or not isinstance(record.get("sourceIds"), list)
                or not all(isinstance(s, str) and s for s in record["sourceIds"])
                or not isinstance(reference, str) or not reference
                or not isinstance(sources, list) or not all(isinstance(s, str) and s for s in sources)):
            raise Stop("INVALID_NATIVE_DECLARATION", exit_code=2)
        norm = record["normalized"]; doc = norm.get("documentation")
        return {"kind": "DECLARATION", "id": record["id"], "reference": reference,
            "symbol": record["symbol"], "service": record["service"], "revision": self.coverage["record"]["revision"],
            "name": norm.get("name"), "owner": norm.get("ownerIdentity"), "scope": norm.get("scope"),
            "declarationKind": norm.get("declarationKind"), "syntaxKind": norm.get("syntaxKind"),
            "parameterTypes": doc.get("parameterTypes") if isinstance(doc, dict) else None,
            "bodyStatus": doc.get("bodyStatus") if isinstance(doc, dict) else None,
            "sourceIds": record["sourceIds"], "sourceReferences": sources, "observationDigest": record.get("digest"),
            "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED", "limits": limitations(norm), "raw": raw}

    def discover(self):
        if self.args.prepare_work:
            page, _ = self.call("prepare", ["docs", "work", "prepare", "--root", self.args.root,
                "--subject", "service:" + self.args.service, "--snapshot", self.args.snapshot],
                input_value={"schema": "codeclew-documentation-work-request/1.0", "audience": "Automatic documentation evidence",
                             "maxItems": self.args.page_items, "maxBytes": 49152})
            self.validate_page(page, "codeclew-documentation-work-page/1.0")
            self.work = page.get("work")
        if not isinstance(self.work, str) or not self.work:
            raise Stop("MISSING_NATIVE_WORK_ID", exit_code=2)
        page, _ = self.call("coverage", ["docs", "context", "--root", self.args.root, "--service", self.args.service,
            "--snapshot", self.args.snapshot, "--format", "compact", "--limit", "1"])
        self.validate_page(page, "codeclew-docs-page/1.0")
        coverage = [r for r in page["items"] if r.get("kind") == "COVERAGE" and r.get("id") == self.args.service]
        if len(coverage) != 1 or not isinstance(coverage[0].get("revision"), str):
            raise Stop("MISSING_NATIVE_COVERAGE", exit_code=2)
        self.coverage = {"record": coverage[0], "authority": page.get("authority"), "limits": limitations(page), "omitted": page["omitted"]}
        cursor, seen, ids, membership = None, set(), set(), None
        while True:
            query = {"query": {"kind": "SYMBOL", "symbolContains": self.args.contains}}
            if cursor is not None:
                query["cursor"] = cursor
            page, receipt = self.work_page("catalogue", query)
            current = {k: page.get(k) for k in ("contextDigest", "inputDigest", "membershipDigest", "total", "authority")}
            if membership is not None and current != membership:
                raise Stop("NATIVE_QUERY_MEMBERSHIP_CHANGED", exit_code=2)
            membership = current
            delivered = []
            for index, item in enumerate(page["items"]):
                if item.get("kind") != "DEPENDENCY" or not isinstance(item.get("record"), dict) or item.get("id") != item["record"].get("id"):
                    raise Stop("INVALID_NATIVE_DECLARATION_ROW", exit_code=2)
                delivered.append(self.project(item["record"], item.get("reference"), item.get("sourceReferences", []),
                                              {"stdout": receipt["stdout"], "pointer": f"/items/{index}"}))
            for omission in page["omitted"]:
                recovery = {"omission": omission, "status": "UNRESOLVED"}
                self.omissions.append(recovery)
                try:
                    if (not isinstance(omission, dict) or omission.get("kind") != "DEPENDENCY"
                            or not isinstance(omission.get("id"), str) or not isinstance(omission.get("reference"), str)):
                        raise Stop("UNSUPPORTED_NATIVE_OMISSION", omitted=omission)
                    record, raw = self.dependency(omission["id"])
                    if record.get("id") != omission["id"]:
                        raise Stop("RECOVERED_DEPENDENCY_ID_CHANGED", exit_code=2)
                    row = self.project(record, omission["reference"], self.navigation(record), raw)
                    delivered.append(row); recovery["status"] = "RECOVERED_NATIVE_DEPENDENCY_PARTS"
                except Stop as error:
                    recovery["failure"] = {"status": error.status, "exitCode": error.exit_code, **error.details}
            for row in delivered:
                if row["id"] in ids:
                    raise Stop("DUPLICATE_NATIVE_DECLARATION", exit_code=2, id=row["id"])
                ids.add(row["id"]); self.rows.append(row)
            cursor = page["nextCursor"]
            if cursor is None:
                break
            if cursor in seen:
                raise Stop("NON_PROGRESSING_NATIVE_CONTINUATION", exit_code=2)
            seen.add(cursor)
        unresolved = [o for o in self.omissions if o["status"] == "UNRESOLVED"]
        complete = not unresolved and not self.coverage["omitted"] and len(self.rows) == membership.get("total")
        return {"kind": "DISCOVERY", "status": "DISCOVERY_READY" if complete else "DISCOVERY_INCOMPLETE",
            "scope": self.scope(), "work": self.work, "coverage": self.coverage, "choices": sorted(self.rows, key=lambda r: r["id"]),
            "queryComplete": complete, "omissions": self.omissions, "dependencyParts": self.proofs,
            "sourceBodiesRequested": False}, 0 if complete else 3

    def scope(self):
        return {"service": self.args.service, "snapshot": self.args.snapshot, "contains": getattr(self.args, "contains", "")}

    def read_selected(self):
        raw = self.args.packet.read_bytes(); packet = json.loads(raw)
        if (packet.get("schema") != SCHEMA or packet.get("kind") != "DISCOVERY"
                or not isinstance(packet.get("choices"), list) or not isinstance(packet.get("work"), str)
                or any(packet.get("scope", {}).get(k) != getattr(self.args, k) for k in ("service", "snapshot"))):
            raise Stop("DISCOVERY_PACKET_SCOPE_MISMATCH", exit_code=2)
        self.work = packet["work"]; self.coverage = packet.get("coverage")
        (self.args.output_dir / "discovery.input.json").write_bytes(raw)
        if getattr(self.args, "reuse_read", None) is not None:
            reuse = runpy.run_path(str(Path(__file__).with_name("reusable_evidence.py")),
                                   init_globals={"SHARED_BOUND_SOURCE": B})["ReusableEvidence"]
            self.reuse = reuse(self.args.reuse_read, packet["scope"], self.work)
        selections = json.loads(self.args.selection.read_bytes())
        if not isinstance(selections, list) or not selections:
            raise Stop("EXPLICIT_SELECTION_LIST_REQUIRED", exit_code=2)
        rows = {r["id"]: r for r in packet["choices"]}
        if len(rows) != len(packet["choices"]):
            raise Stop("DUPLICATE_PACKET_DECLARATION", exit_code=2)
        outcomes, texts, seen = [], {}, set()
        for selection in selections:
            try:
                if not isinstance(selection, dict) or set(selection) - {"declarationId", "sourceId", "sourceReference"}:
                    raise Stop("INVALID_EXPLICIT_SELECTION", exit_code=2)
                identity = selection.get("declarationId")
                if not isinstance(identity, str) or identity not in rows:
                    raise Stop("DECLARATION_NOT_IN_PACKET", exit_code=2)
                if any(selection.get(k) is not None and not isinstance(selection[k], str) for k in ("sourceId", "sourceReference")):
                    raise Stop("INVALID_EXPLICIT_SELECTION", exit_code=2)
                choice = (identity, selection.get("sourceId"), selection.get("sourceReference"))
                if choice in seen:
                    raise Stop("DUPLICATE_EXPLICIT_SELECTION", exit_code=2)
                seen.add(choice)
                retained = self.reuse.find(rows[identity], selection) if self.reuse is not None else None
                if retained is not None:
                    outcome, source = retained
                    outcome = self.reuse.retain(outcome, self.args.output_dir)
                    outcome["selection"] = selection
                else:
                    reader = Selected(self, rows[identity], selection)
                    value = reader.read(); source = value["sourceItems"][0]["record"]
                    outcome = {"selection": selection, "status": value["status"], "declaration": value["declaration"],
                        "source": {k: v for k, v in source.items() if k != "text"}, "textKey": source["textDigest"],
                        "sourceRecordDigest": digest(source), "sourceParts": reader.parts,
                        "sourceReadReused": reader.reused, "nativePages": reader.envelopes, "evidenceReused": False}
                key = source["textDigest"]
                if key in texts and texts[key] != source["text"]:
                    raise Stop("SOURCE_TEXT_DIGEST_COLLISION", exit_code=2)
                texts[key] = source["text"]
                outcomes.append(outcome)
            except Stop as error:
                full = {"selection": selection, "status": error.status, "exitCode": error.exit_code, **error.details}
                self.failures.append(full)
                outcome = {k: full[k] for k in ("selection", "status", "exitCode", "sourceIds", "sourceReferences", "recovery", "nativeReturnCode") if k in full}
                if isinstance(full.get("nativeOutput"), dict):
                    outcome["nativeFailure"] = full["nativeOutput"].get("error")
                outcomes.append(outcome)
        return {"kind": "READ", "status": "SELECTED_SOURCES_READY" if not self.failures else "SELECTED_SOURCES_INCOMPLETE",
            "scope": packet["scope"], "work": self.work, "coverage": self.coverage,
            "discoveryQueryComplete": packet["queryComplete"], "outcomes": outcomes, "texts": texts,
            "inputPacketDigest": "sha256:" + hashlib.sha256(raw).hexdigest(),
            "reuseInput": {"path": str(self.reuse.path), "packetDigest": digest(self.reuse.packet)} if self.reuse is not None else None}, 0 if not self.failures else 3

    def run(self):
        acquisition_started = time.perf_counter()
        try:
            if self.args.action == "read-sources":
                sources = runpy.run_path(str(Path(__file__).with_name("source_references.py")), init_globals={"SHARED_BOUND_SOURCE":B})
                value, code = sources["read"](self)
            elif self.args.action in ("discover-files", "read-files"):
                files = runpy.run_path(str(Path(__file__).with_name("retained_files.py")), init_globals={"SHARED_BOUND_SOURCE":B})
                value, code = files["discover" if self.args.action == "discover-files" else "read"](self)
            else:
                value, code = self.discover() if self.args.action == "discover" else self.read_selected()
        except (Stop, ValueError, TypeError, KeyError) as error:
            value = {"kind": self.args.action.upper(), "status": error.status if isinstance(error, Stop) else "INVALID_PACKET_INPUT",
                     "queryComplete": False, "details": error.details if isinstance(error, Stop) else str(error)}
            code = error.exit_code if isinstance(error, Stop) else 2
            value.update(scope=self.scope(), work=self.work, coverage=self.coverage)
            if self.args.action == "discover":
                value.update(choices=self.rows, omissions=self.omissions)
        acquisition_seconds = time.perf_counter() - acquisition_started
        value.update(schema=SCHEMA, sourceFallbackAllowed=False, completeDeclarationClaimed=False,
                     wholeProcessOrRuntimeCompleteClaimed=False, freshSourceOrRuntimeVerificationClaimed=False,
                     nativePublicationApprovalClaimed=False)
        packet_text = encode(value); (self.args.output_dir / "packet.json").write_text(packet_text, encoding="utf-8")
        counts = {}
        for receipt in self.receipts:
            command = " ".join(receipt["command"][1:4]); counts[command] = counts.get(command, 0) + 1
        summary = {"schema": "codeclew-auto-doc-read-summary/1.0", "status": value["status"],
            "observedNativeCommands": len(self.receipts), "commandsByPrefix": counts,
            "nativeSeconds": sum(r["elapsedSeconds"] for r in self.receipts),
            "nativeStdoutBytes": sum((self.args.output_dir / r["stdout"]).stat().st_size for r in self.receipts),
            "nativeStderrBytes": sum((self.args.output_dir / r["stderr"]).stat().st_size for r in self.receipts),
            "helperInvocationsObserved": 1, "agentTranscriptCountsAttested": False,
            "packetDigest": digest(value), "packetBytes": len(packet_text.encode()),
            "nativeWrites": "Explicit Work preparation and native read ledger only, in caller's disposable root",
            "warmAuthoring": "Include preparation and all observed reads; no free precomputed author packet"}
        reused_keys = set()
        if value.get("kind") == "READ":
            reused_keys = {o["textKey"] for o in value.get("outcomes", []) if o.get("evidenceReused")}
            summary.update(selectedRequests=value.get("requestedSelections", len(value.get("outcomes", []))), failedRequests=len(self.failures),
                verifiedSourceRecords=len(self.source_cache), uniqueTextPayloads=len(value.get("texts", {})),
                verifiedSourceReadsReused=sum(bool(o.get("sourceReadReused")) for o in value.get("outcomes", [])),
                uniqueTextBytes=sum(len(t.encode()) for t in value.get("texts", {}).values()),
                priorEvidenceBindingsReused=sum(bool(o.get("evidenceReused")) for o in value.get("outcomes", [])),
                reusedTextBytesNotReEmitted=sum(len(value["texts"][k].encode()) for k in reused_keys),
                retainedNativeCommandArtifacts=len(self.reuse.retained_receipts()) if self.reuse is not None else 0,
                reuseAuthority="Caller-trusted earlier native artifacts; consistency checked, not producer-authenticated or fresh source/runtime verification" if self.reuse is not None else "Supported native reads of retained immutable Work; no current-source/runtime verification")
            if self.reuse is not None and not self.receipts:
                summary["nativeWrites"] = "No new native commands or documentation-root writes; retained own-run API artifacts copied into evidence closure"
        else:
            summary.update(declarationChoices=len(value.get("choices", [])), queryComplete=value.get("queryComplete", False),
                recoveredOmissions=sum(o["status"] != "UNRESOLVED" for o in self.omissions),
                unresolvedOmissions=sum(o["status"] == "UNRESOLVED" for o in self.omissions))
            if value.get("kind") == "FILE_DISCOVERY":
                summary["fileChoices"] = summary.pop("declarationChoices")
        evidence_receipts = self.receipts + (self.reuse.retained_receipts() if self.reuse is not None else [])
        for name, data in [("read-summary.json", summary), ("receipts.json", self.receipts),
                           ("evidence-receipts.json", evidence_receipts), ("failure-details.json", self.failures)]:
            (self.args.output_dir / name).write_text(encode(data), encoding="utf-8")
        compact = {k: value[k] for k in ("schema", "kind", "status", "scope", "work") if k in value}
        if value.get("kind") == "DISCOVERY":
            compact.update(queryComplete=value.get("queryComplete", False),
                choices=[{k: r.get(k) for k in ("id", "name", "owner", "scope", "parameterTypes", "bodyStatus", "sourceIds")}
                         for r in value.get("choices", [])], unresolvedOmissions=sum(o["status"] == "UNRESOLVED" for o in self.omissions))
        elif value.get("kind") == "FILE_DISCOVERY":
            compact.update(choices=value["choices"], queryComplete=value["queryComplete"], sourceBodiesRequested=False)
        else:
            compact.update(texts={k: v for k, v in value.get("texts", {}).items() if k not in reused_keys},
                reusedTextKeys=sorted(reused_keys),
                outcomes=[{k: r[k] for k in r if k not in ("nativePages", "sourceParts", "nativeOutput")}
                                                                  for r in value.get("outcomes", [])])
        compact.update(sourceFallbackAllowed=False, completeDeclarationClaimed=False, wholeProcessOrRuntimeCompleteClaimed=False,
            authority="IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED", nativeCommands=summary["observedNativeCommands"], packetDigest=summary["packetDigest"],
            artifacts={"directory": str(self.args.output_dir), "packet": "packet.json", "readSummary": "read-summary.json", "receipts": "receipts.json"})
        if code:
            compact["failure"] = {"status": value["status"]}
            if isinstance(value.get("details"), dict):
                compact["failure"]["nativeFailure"] = value["details"].get("nativeOutput", {}).get("error")
            compact["nextAction"] = "Inspect exact unresolved choices/failures in packet.json. Recover through supported APIs; direct-source fallback requires explicit human permission."
        if (getattr(self.args, "output_format", "packet") in ("author", "metadata")
                and value.get("kind") == "READ" and "outcomes" in value):
            adapter = runpy.run_path(str(Path(__file__).with_name("author_citations.py")),
                                    init_globals={"SHARED_BOUND_SOURCE": B})
            discovery = json.loads((self.args.output_dir / "discovery.input.json").read_bytes())
            projection_started = time.perf_counter()
            compact = adapter["project"](value, discovery, self.args.output_dir,
                                         source_union=getattr(self.args, "source_union", False),
                                         include_texts=self.args.output_format != "metadata")
            if self.args.output_format == "metadata":
                compact["sourceBodiesDisplayed"] = False
            if getattr(self.args, "source_union", False):
                summary.update(acquisitionSeconds=acquisition_seconds,
                               authorProjectionSeconds=time.perf_counter() - projection_started,
                               authorViewBytes=len(encode(compact).encode("utf-8")),
                               sourceUnion=compact["sourceUnion"]["metrics"])
                (self.args.output_dir / "read-summary.json").write_text(encode(summary), encoding="utf-8")
        return compact, code


class Selected(BoundSource):
    def __init__(self, owner, row, selection):
        args = copy.copy(owner.args); args.declaration_id = row["id"]
        args.source_id, args.source_reference = selection.get("sourceId"), selection.get("sourceReference")
        super().__init__(args)
        self.owner, self.row, self.reused = owner, row, False
        self.work = owner.work

    def select(self):
        self.binding = self.row
        return self.row

    def call(self, stage, arguments, *, input_value=None):
        return self.owner.call(stage, arguments, input_value=input_value)

    def declaration(self, row):
        try:
            return super().declaration(row)
        except Stop as error:
            if error.status != "DECLARATION_BINDING_OMITTED":
                raise
        record, raw = self.owner.dependency(row["id"])
        if (record.get("id") != row["id"] or record.get("service") != self.args.service
                or record.get("kind") != "SYMBOL" or record.get("symbol") != row["symbol"]
                or record.get("sourceIds") != row["sourceIds"] or record.get("digest") != row["observationDigest"]):
            raise Stop("NATIVE_DECLARATION_BINDING_CHANGED", exit_code=2)
        return record, self.owner.navigation(record), raw

    def source(self, source_id, reference):
        key = (self.work, self.args.snapshot, self.args.service, source_id, reference)
        if key in self.owner.source_cache:
            source, proof = self.owner.source_cache[key]
            if source["revision"] != self.binding["revision"]:
                raise Stop("REUSED_SOURCE_REVISION_MISMATCH", exit_code=2)
            self.parts, self.reused = proof, True
            return source
        source = super().source(source_id, reference)
        self.owner.source_cache[key] = (source, self.parts)
        return source


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["discover", "read", "discover-files", "read-files", "read-sources"])
    for name in ("clew", "root", "service", "snapshot"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--work")
    parser.add_argument("--source-union", action="store_true", help="READ author view: combine exact agreeing retained byte ranges; canonical packet/citations stay unchanged")
    parser.add_argument("--whole-files", action="store_true", help="read-files: require one complete original retained record per selected file; old/missing captures refuse without fallback")
    parser.add_argument("--prepare-work", action="store_true")
    parser.add_argument("--contains", default="", help="Case-sensitive native symbol substring; empty enumerates scoped declarations")
    parser.add_argument("--path-contains", default="", help="discover-files: literal case-sensitive retained path substring; empty lists every retained file")
    parser.add_argument("--page-items", type=int, choices=range(1, 101), default=20, metavar="1..100", help="Native Work delivery capacity, not an investigation budget")
    parser.add_argument("--packet", type=Path)
    parser.add_argument("--selection", type=Path, help='Explicit list of declaration/file choices; read-sources requires [{"sourceId":"RETURNED_ID", "sourceReference":"RETURNED_REFERENCE"}]')
    parser.add_argument("--reuse-read", type=Path, help="Own earlier native READ packet for repair of the same admitted service/snapshot/Work/bindings")
    parser.add_argument("--output-format", choices=("author", "metadata", "packet"), default="author",
                        help="READ stdout: author citations (default), metadata without source bodies, or legacy packet; full packet/citations retained for inspect")
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args(argv)
    if args.whole_files and args.action != "read-files":
        parser.error("--whole-files requires read-files")
    if args.source_union and (args.action not in ("read", "read-files", "read-sources") or args.output_format != "author"):
        parser.error("--source-union requires a read action with --output-format author")
    if args.action in ("discover", "discover-files") and bool(args.work) == args.prepare_work:
        parser.error("discover requires exactly one of --work or --prepare-work")
    if args.action in ("read", "read-files") and (args.packet is None or args.selection is None or args.prepare_work or args.work):
        parser.error("read requires --packet and --selection; Work comes from the scoped discovery packet")
    if args.action == "read-sources" and (not args.work or args.selection is None or args.packet is not None
            or args.prepare_work or args.reuse_read is not None):
        parser.error("read-sources requires --work and --selection; no imported packet, prepare-work or reuse-read")
    if args.action in ("discover", "discover-files") and args.reuse_read is not None:
        parser.error("--reuse-read applies to same-input read/repair, not discovery")
    args.clew = shutil.which(args.clew) or args.clew
    try:
        args.output_dir.mkdir(parents=True, exist_ok=False)
        value, code = Packet(args).run()
    except OSError as error:
        value, code = {"schema": SCHEMA, "status": "LOCAL_IO_ERROR", "message": str(error), "sourceFallbackAllowed": False}, 2
    print(encode(value), end="")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
