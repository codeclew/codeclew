#!/usr/bin/env python3
"""Retained file discovery and exact non-declaration SOURCE reads through Work.

Called by evidence_packet.py; uses the same native parts and reuse proof checks.
File grouping is navigation only, never a whole-file or parsed-semantics claim.
"""
from __future__ import annotations
import hashlib
import copy
import json
from pathlib import Path
import runpy

B = globals()["SHARED_BOUND_SOURCE"]
Stop, digest, BoundSource = (B[k] for k in ("Stop", "digest", "BoundSource"))
R = runpy.run_path(str(Path(__file__).with_name("reusable_evidence.py")),
                  init_globals={"SHARED_BOUND_SOURCE": B})["ReusableEvidence"]


def pages(owner, query):
    cursor, seen, membership, delivered = None, set(), None, 0
    while True:
        request = {"query": query, **({"cursor":cursor} if cursor else {})}
        page, receipt = owner.work_page("source-metadata", request)
        current = {k:page.get(k) for k in ("contextDigest","inputDigest","membershipDigest","total","authority")}
        if membership is not None and current != membership:raise Stop("NATIVE_QUERY_MEMBERSHIP_CHANGED", exit_code=2)
        membership = current
        if page["omitted"]:raise Stop("SOURCE_METADATA_OMITTED", omitted=page["omitted"])
        delivered += len(page["items"])
        yield page, receipt
        cursor = page["nextCursor"]
        if cursor is None:
            if type(page.get("total")) is not int or delivered != page["total"]:
                raise Stop("SOURCE_METADATA_MEMBERSHIP_INCOMPLETE",exit_code=2)
            break
        if cursor in seen:raise Stop("NON_PROGRESSING_NATIVE_CONTINUATION", exit_code=2)
        seen.add(cursor)


def discover(owner):
    a = owner.args
    if a.prepare_work:
        page, _ = owner.call("prepare", ["docs","work","prepare","--root",a.root,"--subject","service:"+a.service,"--snapshot",a.snapshot],
            input_value={"schema":"codeclew-documentation-work-request/1.0","audience":"Retained file metadata discovery","maxItems":a.page_items,"maxBytes":49152})
        owner.validate_page(page,"codeclew-documentation-work-page/1.0");owner.work=page.get("work")
    if not isinstance(owner.work,str) or not owner.work:raise Stop("MISSING_NATIVE_WORK_ID",exit_code=2)
    files = []; seen = set()
    for page, receipt in pages(owner,{"kind":"SOURCE","projection":"FILES","fileContains":a.path_contains}):
        for i,item in enumerate(page["items"]):
            record=item.get("record",{})
            if (item.get("kind")!="SOURCE_FILE" or item.get("id")!=record.get("file")
                    or record.get("service")!=a.service or not isinstance(record.get("revision"),str)
                    or type(record.get("sourceRecords")) is not int or record["sourceRecords"]<1
                    or record.get("authority")!="NAVIGATION_ONLY" or "text" in record or item["id"] in seen):
                raise Stop("INVALID_NATIVE_FILE_METADATA",exit_code=2)
            seen.add(item["id"]);files.append({"kind":"FILE","id":item["id"],**record})
    return {"kind":"FILE_DISCOVERY","status":"FILES_READY","scope":{**owner.scope(),"pathContains":a.path_contains},"work":owner.work,
        "coverage":None,"choices":files,"queryComplete":True,"sourceBodiesRequested":False},0


def collect_sources(owner,file,file_row,whole):
    a=owner.args
    sources=[];ids=set()
    for page,receipt in pages(owner,{"kind":"SOURCE","projection":"WHOLE_FILE" if whole else "NAVIGATION","file":file}):
        for i,item in enumerate(page["items"]):
            record=item.get("record",{});source=record.get("source",{})
            if (item.get("kind")!="SOURCE_NAVIGATION" or item.get("id")!=source.get("id")
                    or source.get("file")!=file or source.get("service")!=a.service or source.get("revision")!=file_row["revision"]
                    or "text" in source or record.get("textDelivered") is not False
                    or not isinstance(record.get("fullRecordReference"),str) or item["id"] in ids):
                raise Stop("INVALID_NATIVE_SOURCE_METADATA",exit_code=2)
            if whole and record.get("wholeFileCapture") != "EXACT_RETAINED_FILE":
                raise Stop("WHOLE_FILE_NOT_RETAINED", exit_code=2)
            ids.add(item["id"]);sources.append({"id":item["id"],"reference":record["fullRecordReference"],
                "source":source,"recordDigest":record["recordDigest"],"raw":{"stdout":receipt["stdout"],"pointer":f"/items/{i}"}})
    if len(sources)!=(1 if whole else file_row["sourceRecords"]):raise Stop("FILE_SOURCE_MEMBERSHIP_CHANGED",exit_code=2)
    return sources

def read(owner):
    a=owner.args;raw=a.packet.read_bytes();packet=json.loads(raw)
    if (packet.get("schema")!="codeclew-auto-doc-evidence/1.0" or packet.get("kind")!="FILE_DISCOVERY"
            or any(packet.get("scope",{}).get(k)!=getattr(a,k) for k in ("service","snapshot"))
            or not isinstance(packet.get("choices"),list) or not isinstance(packet.get("work"),str)):
        raise Stop("FILE_DISCOVERY_SCOPE_MISMATCH",exit_code=2)
    owner.work=packet["work"];(a.output_dir/"discovery.input.json").write_bytes(raw)
    rows={r["file"]:r for r in packet["choices"]}
    if len(rows)!=len(packet["choices"]):raise Stop("AMBIGUOUS_FILE_METADATA",exit_code=2)
    selections=json.loads(a.selection.read_bytes())
    if not isinstance(selections,list) or not selections:raise Stop("EXPLICIT_FILE_SELECTION_REQUIRED",exit_code=2)
    if a.reuse_read is not None:owner.reuse=R(a.reuse_read,packet["scope"],owner.work)
    outcomes,texts,selected_files=[],{},set()
    whole = getattr(a, "whole_files", False)
    prepared = {}
    if getattr(a,"native_batch_size",1)>1 and getattr(a,"source_batch_size",1)>1:
        candidates = {s["file"]:rows[s["file"]] for s in selections
            if isinstance(s,dict) and set(s)=={"file"} and isinstance(s["file"],str) and s["file"] in rows
            and not (owner.reuse and any(o.get("selection")==s for o in owner.reuse.packet.get("outcomes",[])))}
        owner.prefetch_work_pages([{"query":{"kind":"SOURCE","projection":"WHOLE_FILE" if whole else "NAVIGATION","file":file}} for file in candidates])
        for file,file_row in candidates.items():
            try:prepared[file]=collect_sources(owner,file,file_row,whole)
            except Stop as error:prepared[file]=error
        owner.prefetch_sources([{**r,"revision":rows[file]["revision"]} for file,items in prepared.items()
            if isinstance(items,list) for r in items])
    for selection in selections:
        try:
            if (not isinstance(selection,dict) or set(selection)!={"file"} or not isinstance(selection["file"],str)
                    or selection["file"] not in rows or selection["file"] in selected_files):
                raise Stop("EXPLICIT_UNIQUE_RETURNED_FILE_REQUIRED",exit_code=2)
            file=selection["file"];selected_files.add(file);file_row=rows[file]
            binding={k:file_row[k] for k in ("file","revision","service","sourceRecords","authorities","wholeFileCapture")}
            if whole:binding["wholeFiles"] = True
            old=[o for o in owner.reuse.packet.get("outcomes",[]) if o.get("selection")==selection] if owner.reuse else []
            if old:
                if (owner.reuse.packet.get("inputPacketDigest")!="sha256:"+hashlib.sha256(raw).hexdigest()
                        or len(old)!=(1 if whole else file_row["sourceRecords"]) or any(o.get("fileBinding")!=binding for o in old)):
                    raise Stop("REUSE_FILE_DISCOVERY_CHANGED",exit_code=2)
                for outcome in old:
                    retained=owner.reuse.find_source(outcome["sourceBinding"],selection)
                    if retained is None:raise Stop("REUSE_SOURCE_BINDING_UNAVAILABLE",exit_code=2)
                    item,source=retained;item=owner.reuse.retain(item,a.output_dir)
                    outcomes.append(item);texts[source["textDigest"]]=source["text"]
                continue
            sources = prepared.get(file)
            if isinstance(sources, Stop):raise sources
            if sources is None:sources=collect_sources(owner,file,file_row,whole)
            owner.prefetch_sources([{**row, "revision":file_row["revision"]} for row in sources])
            for row in sources:
                reader=SourceReader(owner);reader.binding={"revision":file_row["revision"]}
                source=reader.source(row["id"],row["reference"])
                if ({k:v for k,v in source.items() if k!="text"}!=row["source"] or digest(source)!=row["recordDigest"]):
                    raise Stop("SOURCE_METADATA_BINDING_CHANGED",exit_code=2)
                if whole:
                    occurrence = source.get("occurrence", {})
                    if (source.get("authority") != "EXACT_SNAPSHOT_TEXT" or source.get("startLine") != 1
                            or occurrence.get("generatedOrigin") is not None or occurrence.get("startByte") != 0
                            or occurrence.get("endByte") != len(source["text"].encode("utf-8"))):
                        raise Stop("WHOLE_FILE_BINDING_INVALID", exit_code=2)
                owner.source_cache[(owner.work,a.snapshot,a.service,row["id"],row["reference"])]=(source,reader.parts)
                outcomes.append({"selection":selection,"status":"RETAINED_SOURCE_RETURNED","sourceBinding":row,
                    "fileBinding":binding,"source":row["source"],"sourceRecordDigest":digest(source),"textKey":source["textDigest"],
                    "sourceParts":reader.parts,"nativePages":[],"evidenceReused":False})
                if getattr(reader, "batch", None):outcomes[-1]["sourceBatch"] = reader.batch
                texts[source["textDigest"]]=source["text"]
        except Stop as error:
            if whole and "WHOLE_FILE_NOT_RETAINED" in str(error.details.get("nativeOutput", {}).get("error", {})):
                error = Stop("WHOLE_FILE_NOT_RETAINED", exit_code=2, **error.details)
            failure={"selection":selection,"status":error.status,"exitCode":error.exit_code,**error.details}
            owner.failures.append(failure);outcomes.append(failure)
    return {"kind":"READ","status":"SELECTED_SOURCES_INCOMPLETE" if owner.failures else "SELECTED_SOURCES_READY",
        "scope":packet["scope"],"work":owner.work,"coverage":None,"discoveryQueryComplete":packet["queryComplete"],
        "outcomes":outcomes,"texts":texts,"requestedSelections":len(selections),
        "inputPacketDigest":"sha256:"+hashlib.sha256(raw).hexdigest()},3 if owner.failures else 0


class SourceReader(BoundSource):
    def __init__(self,owner):
        super().__init__(copy.copy(owner.args));self.owner=owner;self.work=owner.work
    def call(self,*args,**kwargs):return self.owner.call(*args,**kwargs)
    def source(self,*args):
        key=(self.work,self.args.snapshot,self.args.service,*args)
        if key in self.owner.source_batch_errors:
            raise self.owner.source_batch_errors[key]
        self.batch=copy.deepcopy(self.owner.source_batch_proofs.get(key))
        try:
            if key in self.owner.source_cache:
                source,self.parts=self.owner.source_cache[key]
                if source["revision"] != self.binding["revision"]:
                    raise Stop("REUSED_SOURCE_REVISION_MISMATCH",exit_code=2)
                return source
            source=super().source(*args)
            self.owner.source_cache[key]=(source,self.parts)
            if self.batch:
                self.batch["recordDigest"]=digest(source)
                self.owner.source_batch_proofs[key]=copy.deepcopy(self.batch)
            return source
        except Stop as error:
            if error.status=="EMPTY_DECLARATION_SOURCE":raise Stop("EMPTY_RETAINED_SOURCE",exit_code=error.exit_code) from error
            raise
