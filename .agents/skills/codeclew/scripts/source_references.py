#!/usr/bin/env python3
"""Acquire explicitly selected genuine Work SOURCE references, never raw JSON.

No source inventory, declaration, whole-file or runtime conformance is inferred.
Native reference pages and exact part receipts close the ordinary citation proof.
"""
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import runpy

B = globals()["SHARED_BOUND_SOURCE"]
Stop, digest, encode = (B[k] for k in ("Stop", "digest", "encode"))
F = runpy.run_path(str(Path(__file__).with_name("retained_files.py")), init_globals={"SHARED_BOUND_SOURCE":B})
C = runpy.run_path(str(Path(__file__).with_name("citation_evidence.py")))


def read(owner):
    a = owner.args
    selections = json.loads(a.selection.read_bytes())
    if not isinstance(selections, list) or not selections:
        raise Stop("EXPLICIT_SOURCE_REFERENCE_SELECTION_REQUIRED", exit_code=2)
    if getattr(a, "source_batch_size", 1) > 1:
        return read_batches(owner, selections)
    outcomes, texts, choices, seen_ids, seen_refs = [], {}, [], set(), set()
    for selection in selections:
        try:
            if (not isinstance(selection, dict) or set(selection) != {"sourceId", "sourceReference"}
                    or any(not isinstance(v, str) or not v for v in selection.values())):
                raise Stop("INVALID_SOURCE_REFERENCE_SELECTION", exit_code=2)
            identity, reference = selection["sourceId"], selection["sourceReference"]
            if identity in seen_ids or reference in seen_refs:
                raise Stop("DUPLICATE_SOURCE_REFERENCE_SELECTION", exit_code=2)
            seen_ids.add(identity); seen_refs.add(reference)
            pages, names, cursor, cursors = [], [], None, set()
            while True:
                request = {"references":[reference], **({"cursor":cursor} if cursor else {})}
                page, receipt = owner.work_page("source-reference", request)
                pages.append(page); names.append(receipt["stdout"])
                cursor = page["nextCursor"]
                if cursor is None:
                    break
                if cursor in cursors:
                    raise Stop("NON_PROGRESSING_NATIVE_CONTINUATION", exit_code=2)
                cursors.add(cursor)
            try:
                original, coverage, index, pointer = C["source_reference_pages"](
                    pages, owner.scope(), owner.work, identity, reference)
            except C["Stop"] as error:
                raise Stop(error.status, exit_code=error.exit_code, **error.details) from error
            owner.coverage = {"record":coverage, "authority":pages[0].get("authority"), "omitted":[]}
            reader = F["SourceReader"](owner); reader.binding = {"revision":coverage["revision"]}
            source = reader.source(identity, reference)
            if original is not None and original != source:
                raise Stop("SOURCE_REFERENCE_PART_BINDING_CHANGED", exit_code=2)
            row = {"kind":"SOURCE_REFERENCE", "id":identity, "reference":reference,
                   "source":{k:v for k,v in source.items() if k != "text"}, "recordDigest":digest(source),
                   "raw":{"stdout":names[index], "pointer":pointer}, "pages":names}
            key = source["textDigest"]
            if key in texts and texts[key] != source["text"]:
                raise Stop("SOURCE_TEXT_DIGEST_COLLISION", exit_code=2)
            texts[key] = source["text"]; choices.append(row)
            owner.source_cache[(owner.work,a.snapshot,a.service,identity,reference)] = (source,reader.parts)
            outcomes.append({"selection":selection, "status":"RETAINED_SOURCE_RETURNED", "sourceBinding":row,
                "source":row["source"], "sourceRecordDigest":row["recordDigest"], "textKey":key,
                "sourceParts":reader.parts, "nativePages":[], "evidenceReused":False})
        except Stop as error:
            failure = {"selection":selection, "status":error.status, "exitCode":error.exit_code, **error.details}
            owner.failures.append(failure); outcomes.append(failure)
    discovery = {"schema":"codeclew-auto-doc-evidence/1.0", "kind":"SOURCE_REFERENCE_SELECTION",
        "scope":owner.scope(), "work":owner.work, "choices":choices, "queryComplete":not owner.failures,
        "inventoryEnumerated":False}
    raw = encode(discovery).encode("utf-8"); (a.output_dir/"discovery.input.json").write_bytes(raw)
    return {"kind":"READ", "status":"SELECTED_SOURCES_INCOMPLETE" if owner.failures else "SELECTED_SOURCES_READY",
        "scope":owner.scope(), "work":owner.work, "coverage":owner.coverage, "outcomes":outcomes, "texts":texts,
        "discoveryQueryComplete":not owner.failures, "inventoryEnumerated":False,
        "requestedSelections":len(selections), "inputPacketDigest":"sha256:"+hashlib.sha256(raw).hexdigest()}, 3 if owner.failures else 0


def read_batches(owner, selections):
    a = owner.args
    ids, refs = set(), set()
    for selection in selections:
        if (not isinstance(selection, dict) or set(selection) != {"sourceId", "sourceReference"}
                or any(not isinstance(v, str) or not v for v in selection.values())):
            raise Stop("INVALID_SOURCE_REFERENCE_SELECTION", exit_code=2)
        if selection["sourceId"] in ids or selection["sourceReference"] in refs:
            raise Stop("DUPLICATE_SOURCE_REFERENCE_SELECTION", exit_code=2)
        ids.add(selection["sourceId"]); refs.add(selection["sourceReference"])
    owner.prefetch_sources([{"id":s["sourceId"], "reference":s["sourceReference"]} for s in selections])
    outcomes, texts, choices = [], {}, []
    for selection in selections:
        try:
            identity, reference = selection["sourceId"], selection["sourceReference"]
            key=(owner.work,a.snapshot,a.service,identity,reference)
            if key in owner.source_batch_errors:raise owner.source_batch_errors[key]
            reader = F["SourceReader"](owner)
            reader.binding = {"revision":owner.coverage["record"]["revision"]}
            source = reader.source(identity, reference)
            proof = reader.batch
            row = {"kind":"SOURCE_REFERENCE", "id":identity, "reference":reference,
                   "source":{k:v for k,v in source.items() if k != "text"}, "recordDigest":digest(source),
                   "raw":proof["raw"], "pages":proof["pages"], "batchChoices":proof["choices"]}
            key = source["textDigest"]
            if key in texts and texts[key] != source["text"]:
                raise Stop("SOURCE_TEXT_DIGEST_COLLISION", exit_code=2)
            texts[key] = source["text"]; choices.append(row)
            outcomes.append({"selection":selection, "status":"RETAINED_SOURCE_RETURNED", "sourceBinding":row,
                "source":row["source"], "sourceRecordDigest":digest(source), "textKey":key,
                "sourceParts":reader.parts, "sourceBatch":proof, "nativePages":[], "evidenceReused":False})
        except Stop as error:
            failure = {"selection":selection, "status":error.status, "exitCode":error.exit_code, **error.details}
            owner.failures.append(failure); outcomes.append(failure)
    discovery = {"schema":"codeclew-auto-doc-evidence/1.0", "kind":"SOURCE_REFERENCE_SELECTION",
        "scope":owner.scope(), "work":owner.work, "choices":choices, "queryComplete":not owner.failures, "inventoryEnumerated":False}
    raw = encode(discovery).encode("utf-8"); (a.output_dir/"discovery.input.json").write_bytes(raw)
    return {"kind":"READ", "status":"SELECTED_SOURCES_INCOMPLETE" if owner.failures else "SELECTED_SOURCES_READY", "scope":owner.scope(), "work":owner.work,
        "coverage":owner.coverage, "outcomes":outcomes, "texts":texts, "discoveryQueryComplete":not owner.failures,
        "inventoryEnumerated":False, "requestedSelections":len(selections),
        "inputPacketDigest":"sha256:"+hashlib.sha256(raw).hexdigest()}, 3 if owner.failures else 0
