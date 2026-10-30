#!/usr/bin/env python3
"""Read SOURCE from one selected retained Work declaration, without symbol lookup.

Input is a saved declaration_catalogue.py result and an explicit declaration ID.
Only supported native Work reads acquire declaration/source facts. Native read
receipts are written to the caller-selected disposable docs root; no capture,
source file/private-store read, selector synthesis or direct-source fallback.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import runpy
import shutil


native = runpy.run_path(str(Path(__file__).with_name("declaration_catalogue.py")))
Catalogue, Stop, encode, limitations = (native[k] for k in ("Catalogue", "Stop", "encode", "limitations"))
SCHEMA = "codeclew-work-bound-source/1.0"


def digest(value):
    return "sha256:" + hashlib.sha256(encode(value)[:-1].encode("utf-8")).hexdigest()


class BoundSource(Catalogue):
    def __init__(self, args):
        args.work = None
        super().__init__(args)
        self.binding, self.catalogue_input = None, None
        self.envelopes, self.parts = [], []

    def select(self):
        raw = self.args.catalogue_result.read_bytes()
        (self.args.output_dir / "catalogue.input.json").write_bytes(raw)
        self.catalogue_input = {"path": str(self.args.catalogue_result),
                                "sha256": "sha256:" + hashlib.sha256(raw).hexdigest()}
        try:
            packet = json.loads(raw)
        except (ValueError, UnicodeDecodeError) as error:
            raise Stop("INVALID_CATALOGUE_RESULT", exit_code=2) from error
        if (not isinstance(packet, dict) or packet.get("schema") != "codeclew-declaration-catalogue/1.0"
                or not isinstance(packet.get("selection"), dict) or not isinstance(packet.get("items"), list)
                or not isinstance(packet.get("work"), str) or not packet["work"]):
            raise Stop("INVALID_CATALOGUE_RESULT", exit_code=2)
        if any(packet["selection"].get(k) != getattr(self.args, k) for k in ("service", "snapshot")):
            raise Stop("CATALOGUE_SCOPE_MISMATCH", exit_code=2)
        rows = [r for r in packet["items"] if isinstance(r, dict) and r.get("id") == self.args.declaration_id]
        if len(rows) != 1:
            raise Stop("EXPLICIT_DECLARATION_CHOICE_REQUIRED", candidates=[r.get("id") for r in packet["items"] if isinstance(r, dict)])
        row = rows[0]
        if (row.get("kind") != "DECLARATION" or row.get("service") != self.args.service
                or not isinstance(row.get("reference"), str) or not row["reference"]
                or not isinstance(row.get("revision"), str) or not row["revision"]
                or not isinstance(row.get("symbol"), str) or not isinstance(row.get("sourceIds"), list)):
            raise Stop("INVALID_CATALOGUE_BINDING", exit_code=2)
        self.work, self.binding = packet["work"], row
        self.catalogue_input["catalogueRoot"] = packet["selection"].get("root")
        return row

    def declaration(self, row):
        cursor, seen = None, set()
        while True:
            selection = {"references": [row["reference"]]}
            if cursor is not None:
                selection["cursor"] = cursor
            page, receipt = self.call("declaration", ["docs", "work", "read", "--root", self.args.root,
                                                        "--work", self.work], input_value=selection)
            self.validate_page(page, "codeclew-documentation-work-page/1.0")
            if page.get("work") != self.work:
                raise Stop("NATIVE_WORK_MISMATCH", exit_code=2)
            self.envelopes.append({"raw": receipt["stdout"], "metadata": {k: v for k, v in page.items() if k != "items"},
                                   "coverage": [item for item in page["items"] if item.get("kind") == "COVERAGE"]})
            matches = [(i, item) for i, item in enumerate(page["items"])
                       if item.get("kind") == "DEPENDENCY" and item.get("id") == row["id"]]
            if len(matches) > 1:
                raise Stop("AMBIGUOUS_NATIVE_DECLARATION_RECORD", exit_code=2)
            if matches:
                index, item = matches[0]
                record = item.get("record")
                if (not isinstance(record, dict) or record.get("id") != row["id"]
                        or record.get("kind") != "SYMBOL" or record.get("service") != self.args.service
                        or record.get("symbol") != row["symbol"] or record.get("sourceIds") != row["sourceIds"]
                        or not isinstance(record.get("normalized"), dict)
                        or item.get("reference") != row["reference"]
                        or record.get("digest") != row.get("observationDigest")):
                    raise Stop("NATIVE_DECLARATION_BINDING_CHANGED", exit_code=2)
                references = item.get("sourceReferences")
                if not isinstance(references, list) or not all(isinstance(r, str) and r for r in references):
                    raise Stop("MISSING_NATIVE_SOURCE_REFERENCES", declaration=record)
                return record, references, {"stdout": receipt["stdout"], "pointer": f"/items/{index}"}
            if any(item.get("id") == row["id"] for item in page["omitted"] if isinstance(item, dict)):
                raise Stop("DECLARATION_BINDING_OMITTED", omitted=page["omitted"],
                           recovery={"command": "docs work read-dependency-part", "work": self.work,
                                     "input": {"schema": "codeclew-documentation-dependency-part-request/1.0",
                                               "kind": "DEPENDENCY", "id": row["id"]}})
            cursor = page["nextCursor"]
            if cursor is None:
                raise Stop("SELECTED_DECLARATION_NOT_RETURNED", omitted=page["omitted"])
            if cursor in seen:
                raise Stop("NON_PROGRESSING_NATIVE_CONTINUATION", exit_code=2)
            seen.add(cursor)

    def source(self, source_id, reference):
        cursor, seen, offset, chunks, first = None, set(), 0, [], None
        while True:
            request = {"schema": "codeclew-documentation-source-part-request/1.0", "reference": reference}
            if cursor is not None:
                request["cursor"] = cursor
            part, receipt = self.call("source-part", ["docs", "work", "read-part", "--root", self.args.root,
                                                       "--work", self.work], input_value=request)
            if (part.get("schema") != "codeclew-documentation-source-part/1.0"
                    or part.get("work") != self.work or part.get("snapshot") != self.args.snapshot
                    or part.get("reference") != reference or part.get("sourceId") != source_id
                    or not isinstance(part.get("source"), dict) or not isinstance(part.get("text"), str)
                    or "nextCursor" not in part):
                raise Stop("SOURCE_PART_IDENTITY_MISMATCH", exit_code=2, rawReceipt=receipt)
            metadata = part["source"]
            if (metadata.get("id") != source_id or metadata.get("service") != self.args.service
                    or metadata.get("revision") != self.binding["revision"]):
                raise Stop("SOURCE_BINDING_SCOPE_MISMATCH", exit_code=2, source=metadata)
            identity = {k: part.get(k) for k in ("source", "recordDigest", "totalTextBytes", "authority")}
            if first is not None and identity != first:
                raise Stop("SOURCE_PART_IDENTITY_CHANGED", exit_code=2)
            first = identity
            text = part["text"].encode("utf-8")
            end, total = part.get("endByte"), part.get("totalTextBytes")
            if (type(part.get("startByte")) is not int or part["startByte"] != offset
                    or type(end) is not int or end != offset + len(text) or type(total) is not int
                    or not 0 <= end <= total or not text and total > 0
                    or part.get("fragmentDigest") != "sha256:" + hashlib.sha256(text).hexdigest()
                    or part.get("receiptDigest") != digest({k: v for k, v in part.items() if k != "receiptDigest"})):
                raise Stop("SOURCE_PART_RANGE_OR_DIGEST_MISMATCH", exit_code=2, rawReceipt=receipt)
            chunks.append(part["text"])
            self.parts.append({"stdout": receipt["stdout"], "startByte": offset, "endByte": end,
                               "recordDigest": part["recordDigest"], "receiptDigest": part["receiptDigest"],
                               "authority": part.get("authority"), "nextCursor": part["nextCursor"]})
            offset, cursor = end, part["nextCursor"]
            if cursor is None:
                if offset != total:
                    raise Stop("SOURCE_PART_INCOMPLETE", exit_code=2)
                break
            if not isinstance(cursor, str) or not cursor or cursor in seen or offset >= total:
                raise Stop("INVALID_SOURCE_PART_CONTINUATION", exit_code=2)
            seen.add(cursor)
        source = {**first["source"], "text": "".join(chunks)}
        if (source.get("textDigest") != "sha256:" + hashlib.sha256(source["text"].encode("utf-8")).hexdigest()
                or digest(source) != first["recordDigest"]):
            raise Stop("SOURCE_RECORD_DIGEST_MISMATCH", exit_code=2)
        if (not all(isinstance(source.get(k), str) and source[k] for k in ("file", "revision", "authority"))
                or type(source.get("startLine")) is not int or type(source.get("endLine")) is not int
                or not 1 <= source["startLine"] <= source["endLine"]):
            raise Stop("INVALID_SOURCE_PROVENANCE", exit_code=2)
        if not source["text"]:
            raise Stop("EMPTY_DECLARATION_SOURCE", source=source)
        return source

    def read(self):
        row = self.select()
        declaration, references, raw = self.declaration(row)
        ids = declaration["sourceIds"]
        if not all(isinstance(s, str) and s for s in ids):
            raise Stop("INVALID_NATIVE_SOURCE_BINDING", exit_code=2)
        if not ids:
            raise Stop("MISSING_DECLARATION_SOURCE")
        source_id = self.args.source_id
        if source_id is None:
            if len(set(ids)) != 1:
                raise Stop("EXPLICIT_SOURCE_CHOICE_REQUIRED", sourceIds=ids, sourceReferences=references)
            source_id = ids[0]
        if source_id not in ids:
            raise Stop("SOURCE_OUTSIDE_SELECTED_DECLARATION", exit_code=2, sourceIds=ids)
        if not references:
            raise Stop("MISSING_NATIVE_SOURCE_REFERENCES", sourceIds=ids,
                       recovery={"command": "docs context", "service": self.args.service,
                                 "snapshot": self.args.snapshot, "source": source_id,
                                 "instruction": "Read only the selected returned ID through the supported SOURCE API; preserve any native omission/error."})
        reference = self.args.source_reference
        if reference is None:
            if len(set(references)) != 1:
                raise Stop("EXPLICIT_SOURCE_REFERENCE_CHOICE_REQUIRED", sourceIds=ids, sourceReferences=references,
                           recovery={"command": "docs work read", "instruction": "Read returned SOURCE references in native batches of at most eight to map exact IDs, then explicitly select the matching ID/reference pair."})
            reference = references[0]
        if reference not in references:
            raise Stop("SOURCE_REFERENCE_OUTSIDE_SELECTED_DECLARATION", exit_code=2, sourceReferences=references)
        source = self.source(source_id, reference)
        documentation = declaration["normalized"].get("documentation")
        return {"status": "DECLARATION_SOURCE_RETURNED", "sourceItems": [{"kind": "SOURCE", "id": source_id, "record": source}],
                "sourceDelivery": "VERIFIED_NATIVE_SOURCE_PARTS", "sourceTextBytes": len(source["text"].encode("utf-8")),
                "declaration": {"id": declaration["id"], "symbol": declaration["symbol"], "service": declaration["service"],
                    "sourceIds": ids, "sourceReferences": references, "selectedSourceId": source_id,
                    "selectedSourceReference": reference, "observationDigest": declaration["digest"], "raw": raw,
                    "descriptor": {k: declaration["normalized"][k] for k in ("scope", "kind", "name", "ownerIdentity", "declarationKind", "syntaxKind") if k in declaration["normalized"]},
                    "bodyStatus": documentation.get("bodyStatus") if isinstance(documentation, dict) else None,
                    "limits": limitations(declaration["normalized"])}}

    def result(self):
        code = 0
        try:
            value = self.read()
        except Stop as error:
            value, code = {"status": error.status, **error.details}, error.exit_code
        value.update(schema=SCHEMA, work=self.work, catalogueInput=self.catalogue_input,
            selection={"root": self.args.root, "service": self.args.service, "snapshot": self.args.snapshot,
                       "declarationId": self.args.declaration_id, "sourceId": self.args.source_id,
                       "sourceReference": self.args.source_reference},
            sourceFallbackAllowed=False, completeDeclarationClaimed=False, callFlowClosureClaimed=False,
            nativePages=self.envelopes, sourceParts=self.parts,
            limitations=["Verified part assembly establishes only exact retained SOURCE fragment bytes, not whole-class/declaration completeness or runtime flow.",
                         "Producer bodyStatus is retained; absence is unspecified. No source is reconstructed from absent evidence.",
                         "Native Work read/part receipts mutate the selected disposable root; include all reads in warm authoring.",
                         "Root paths may differ after a caller-selected documentation-root copy; native Work/snapshot/service/declaration/binding are revalidated.",
                         "Full native streams/result are saved locally; host display completeness is not attested."],
            artifacts={"directory": str(self.args.output_dir), "result": "result.json", "receipts": "receipts.json"})
        if code:
            value["nextAction"] = "Keep the exact gap, explicit choices and native recovery. Direct-source fallback requires explicit user permission; documentation permission is insufficient."
        for name, data in [("result.json", value), ("receipts.json", self.receipts)]:
            (self.args.output_dir / name).write_text(encode(data), encoding="utf-8")
        return value, code


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("clew", "root", "service", "snapshot", "declaration-id"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--catalogue-result", type=Path, required=True, help="Saved declaration_catalogue.py result; not raw source")
    parser.add_argument("--source-id", help="Explicit returned source ID when multiple native bindings exist")
    parser.add_argument("--source-reference", help="Explicit returned Work SOURCE reference; returned part must confirm its ID")
    parser.add_argument("--output-dir", type=Path, required=True, help="New directory for all native inputs, streams and part receipts")
    args = parser.parse_args(argv)
    args.clew = shutil.which(args.clew) or args.clew
    try:
        args.output_dir.mkdir(parents=True, exist_ok=False)
        value, code = BoundSource(args).result()
    except OSError as error:
        value, code = {"schema": SCHEMA, "status": "LOCAL_IO_ERROR", "sourceFallbackAllowed": False, "message": str(error)}, 2
    print(encode(value), end="")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
