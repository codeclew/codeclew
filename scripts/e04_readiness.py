#!/usr/bin/env python3
"""Content-addressed readiness DAG for E04.  Standard library only."""
from __future__ import annotations

import fcntl
import hashlib
import json
import os
import secrets
import shutil
import stat
import subprocess
import tempfile
import threading
import time
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path
from typing import Any

SCHEMA = "semantic-editing-e04-readiness-receipt/0.1"
POINTER_SCHEMA = "semantic-editing-e04-readiness-pointer/0.1"
ATTEMPT_SCHEMA = "semantic-editing-e04-readiness-attempt-pointer/0.1"
INDEPENDENT_AUDITOR_ID = "e02-independent-verifier"
ANNOTATOR_A_ID = "R1_BLIND_ANNOTATOR_A"
ANNOTATOR_B_ID = "R1_BLIND_ANNOTATOR_B"
STORE_SCHEMA = "semantic-editing-e04-readiness-store/0.1"
CHECKER_VERSION = "e04-readiness-phase1/0.1"
STATUSES = {"READY", "FAILED", "STALE", "BLOCKED"}
ABSENT = "ABSENT:semantic-editing-e04-absent/0.1"
CONTEXT_KEYS = {
    "binarySha256","binaryRealPath","catalogSha256","adapterSha256","runnerSha256","populationSha256","outputSchemaSha256","commonPromptSha256","corpusSha256","readinessCheckerSha256","codexVersion","astBinarySha256","semanticCorpusBinarySha256","semanticCorpusBinaryRealPath","dependencySeedManifestSha256","publicSetSha256","diagnosticPublicSetSha256","diagnosticFreezeSha256","productCoverageSha256","productCoverageAuditSha256","productCoverageFailedReceiptSha256","diagnosticPreflightSha256","diagnosticAuditSha256","diagnosticCanaryPacketSetSha256","r1DecisionSha256","r1PublicSetSha256","r1TargetTreeSha256","r1ControllerTreeSha256","r1AnnotationASha256","r1AnnotationBSha256","r1HiddenVerifySha256","r1CoverageSha256","r1PreflightSha256","r1PreflightAuditSha256","finalPacketSetSha256","judgmentsSha256","summarySha256","resultsAuditSha256","ge1EvidenceSha256","setupFailure",
}
BASE_AUTHORITY_KEYS={"binarySha256","binaryRealPath","catalogSha256","adapterSha256","runnerSha256","populationSha256","outputSchemaSha256","commonPromptSha256","corpusSha256","readinessCheckerSha256","codexVersion","astBinarySha256"}
DIRECT_NODES={"PRODUCT_COVERAGE_GUARD","DIAGNOSTIC_FULL_PREFLIGHT_42","DIAGNOSTIC_CANARY_3_COMPLETE","R1_CORPUS_42_MATERIALIZED","R1_HIDDEN_VERIFY_COMPLETE","R1_COVERAGE_GUARD_COMPLETE","R1_FULL_PREFLIGHT_42","FINAL_MATRIX_126_COMPLETE","JUDGE_COMPLETE","SUMMARY_COMPLETE"}
IMPORT_NODES={"PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT","DIAGNOSTIC_AUDIT_IMPORT","R1_BLIND_ANNOTATION_A_IMPORT","R1_BLIND_ANNOTATION_B_IMPORT","R1_PREFLIGHT_AUDIT_IMPORT","RESULTS_AUDIT_IMPORT","GE1_EVIDENCE_RECORDED"}
ROOT_NODES={"PRODUCT_COVERAGE_START_READY","E04_COVERAGE_NO_GO_COMPLETE","DIAGNOSTIC_FULL_PREFLIGHT_START_READY","DIAGNOSTIC_PREFLIGHT_READY","DIAGNOSTIC_CANARY_START_READY","R1_MATERIALIZE_START_READY","R1_ANNOTATION_START_READY","R1_HIDDEN_VERIFY_START_READY","R1_COVERAGE_START_READY","R1_FULL_PREFLIGHT_START_READY","R1_PREFLIGHT_READY","FINAL_MATRIX_START_READY","JUDGE_START_READY","SUMMARIZE_START_READY","E04_RESULTS_COMPLETE"}
_THREAD_LOCK = threading.Lock()


def canonical(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False) + "\n").encode()


def digest_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def digest_file(path: Path) -> str:
    return digest_bytes(path.read_bytes())


def validated_ast_provenance(value: Any) -> dict[str,str]:
    keys={"realPath","binarySha256","version"}
    if not isinstance(value,dict) or set(value)!=keys or not all(isinstance(value[key],str) and value[key] for key in keys):
        raise RuntimeError("AST executable provenance contract mismatch")
    digest=value["binarySha256"]
    if len(digest)!=64 or any(character not in "0123456789abcdef" for character in digest):
        raise RuntimeError("AST executable provenance binarySha256 mismatch")
    return value


def atomic_bytes(path: Path, value: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.parent / f".{path.name}.tmp-{os.getpid()}-{secrets.token_hex(6)}"
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
    try:
        os.write(descriptor, value); os.fsync(descriptor)
    finally:
        os.close(descriptor)
    os.replace(temporary, path)


def load_json(path: Path) -> Any:
    with path.open(encoding="utf-8") as handle: return json.load(handle)


def load_graph(path: Path) -> dict[str, Any]:
    value = load_json(path)
    if not isinstance(value, dict) or set(value) != {"schema", "version", "nodes", "roots"} or value["schema"] != "semantic-editing-e04-readiness-graph/0.1":
        raise RuntimeError("invalid readiness graph schema")
    ids = [node.get("id") for node in value["nodes"]]
    if len(ids) != len(set(ids)) or any(set(node) != {"id","action","checker","dependencies","inputSelectors"} or node.get("action") not in {"PREPARE","VERIFY"} or not isinstance(node.get("checker"),str) or not node["checker"] or not isinstance(node.get("inputSelectors"),list) or len(node["inputSelectors"])!=len(set(node["inputSelectors"])) or not all(isinstance(item,str) and item in CONTEXT_KEYS for item in node["inputSelectors"]) for node in value["nodes"]):
        raise RuntimeError("invalid readiness graph nodes")
    known = set(ids)
    if any(len(node["dependencies"])!=len(set(node["dependencies"])) for node in value["nodes"]): raise RuntimeError("duplicate readiness graph dependency")
    if any(dependency not in known for node in value["nodes"] for dependency in node["dependencies"]) or any(root not in known for root in value["roots"]):
        raise RuntimeError("readiness graph contains unknown dependency/root")
    dependencies={node["id"]:node["dependencies"] for node in value["nodes"]}; visiting=set(); visited=set()
    def visit(node: str) -> None:
        if node in visiting: raise RuntimeError("readiness graph contains a cycle")
        if node in visited: return
        visiting.add(node)
        for dependency in dependencies[node]: visit(dependency)
        visiting.remove(node); visited.add(node)
    for node in ids: visit(node)
    artifact=next((node for node in value["nodes"] if node["id"]=="ARTIFACT_PROVENANCE"),None)
    if artifact is not None and set(artifact["inputSelectors"])!=BASE_AUTHORITY_KEYS: raise RuntimeError("ARTIFACT_PROVENANCE selector set is incomplete")
    return value


def load_production_graph(provided_path:Path)->dict[str,Any]:
    canonical_path=Path(__file__).resolve().parents[1]/"benchmarks/semantic-change/e04-readiness-graph.json"
    for path,label in ((canonical_path,"canonical"),(provided_path.absolute(),"provided")):
        if path.is_symlink() or not path.is_file() or not stat.S_ISREG(path.lstat().st_mode): raise RuntimeError(f"{label} production readiness graph is absent or unsafe")
    canonical_bytes=canonical_path.read_bytes(); provided_bytes=provided_path.absolute().read_bytes()
    if provided_bytes!=canonical_bytes: raise RuntimeError("provided readiness graph is not byte-exact production authority")
    return load_graph(canonical_path)


def graph_hash(graph: dict[str, Any]) -> str: return digest_bytes(canonical(graph))


class Store:
    def __init__(self, root: Path, graph: dict[str, Any], create: bool = False):
        self.root = root.absolute(); self.graph = graph; self.graph_hash = graph_hash(graph)
        if self.root.is_symlink(): raise RuntimeError("readiness store root must not be a symlink")
        if create:
            self.root.mkdir(parents=True, exist_ok=True)
            identity = self.root / "STORE.json"
            if not identity.exists():
                atomic_bytes(identity, canonical({"schema":STORE_SCHEMA,"storeId":secrets.token_hex(32)})); identity.chmod(0o444)
            graph_path = self.root / "graphs" / f"{self.graph_hash}.json"
            if not graph_path.exists(): atomic_bytes(graph_path, canonical(graph)); graph_path.chmod(0o444)
        identity = self.root / "STORE.json"
        if not identity.is_file() or identity.is_symlink(): raise RuntimeError("readiness store identity missing")
        store = load_json(identity)
        if set(store) != {"schema","storeId"} or store["schema"] != STORE_SCHEMA: raise RuntimeError("invalid readiness store identity")
        self.store_id = store["storeId"]
        frozen_graph = self.root / "graphs" / f"{self.graph_hash}.json"
        if not frozen_graph.is_file() or frozen_graph.read_bytes() != canonical(graph): raise RuntimeError("readiness graph object mismatch")
        if create:
            (self.root / "objects").mkdir(exist_ok=True); (self.root / "current").mkdir(exist_ok=True)
        elif not (self.root/"objects").is_dir() or not (self.root/"current").is_dir():
            raise RuntimeError("readiness store object/current directories missing")

    def locked(self):
        return _Lock(self.root / "LOCK")

    def object(self, value: dict[str, Any]) -> str:
        raw = canonical(value); identity = digest_bytes(raw); path = self.root / "objects" / f"{identity}.json"
        if path.exists() and path.read_bytes() != raw: raise RuntimeError("readiness object hash collision")
        if not path.exists(): atomic_bytes(path, raw); path.chmod(0o444)
        return identity

    def pointer(self, node: str) -> dict[str, Any] | None:
        path = self.root / "current" / f"{node}.json"
        if not path.exists(): return None
        if path.is_symlink(): raise RuntimeError(f"forged readiness pointer path: {node}")
        value = load_json(path)
        if set(value) != {"schema","storeId","graphHash","node","receiptHash"} or value["schema"] != POINTER_SCHEMA or value["storeId"] != self.store_id or value["graphHash"] != self.graph_hash or value["node"] != node:
            raise RuntimeError(f"forged readiness pointer: {node}")
        receipt_path = self.root / "objects" / f"{value['receiptHash']}.json"
        if receipt_path.is_symlink() or not receipt_path.is_file() or digest_file(receipt_path) != value["receiptHash"]: raise RuntimeError(f"readiness receipt object missing: {node}")
        return value

    def receipt(self, node: str) -> dict[str, Any] | None:
        pointer = self.pointer(node)
        return load_json(self.root / "objects" / f"{pointer['receiptHash']}.json") if pointer else None

    def publish(self, receipt: dict[str, Any]) -> str:
        receipt_hash = self.object(receipt); node = receipt["node"]
        pointer = {"schema":POINTER_SCHEMA,"storeId":self.store_id,"graphHash":self.graph_hash,"node":node,"receiptHash":receipt_hash}
        atomic_bytes(self.root / "current" / f"{node}.json", canonical(pointer)); return receipt_hash


class _Lock:
    def __init__(self, path: Path): self.path = path; self.handle = None
    def __enter__(self):
        _THREAD_LOCK.acquire(); self.handle = self.path.open("a+"); fcntl.flock(self.handle, fcntl.LOCK_EX); return self
    def __exit__(self, *_):
        fcntl.flock(self.handle, fcntl.LOCK_UN); self.handle.close(); _THREAD_LOCK.release()


def dependency_map(graph: dict[str, Any]) -> dict[str, list[str]]:
    return {node["id"]:node["dependencies"] for node in graph["nodes"]}


def node_spec(store: Store, node: str) -> dict[str,Any]: return next(item for item in store.graph["nodes"] if item["id"]==node)


def selected_inputs(store: Store, node: str, inputs: dict[str,str]) -> dict[str,str]:
    selectors=node_spec(store,node)["inputSelectors"]
    missing=[key for key in selectors if key not in inputs]
    if missing: raise RuntimeError(f"readiness context missing selected input:{','.join(missing)}")
    selected={key:inputs[key] for key in selectors}
    if any(not isinstance(value,str) or not value for value in selected.values()): raise RuntimeError("readiness selected input must be a nonempty string")
    return selected


def node_key(store: Store, node: str, inputs: dict[str, str], dependencies: dict[str, str]) -> str:
    spec=node_spec(store,node)
    selected=selected_inputs(store,node,inputs)
    return digest_bytes(canonical({"storeId":store.store_id,"graphHash":store.graph_hash,"checkerVersion":CHECKER_VERSION,"checker":spec["checker"],"checkerSourceSha256":digest_file(Path(__file__)),"node":node,"inputs":selected,"dependencies":dependencies}))


def publish(store: Store, node: str, status: str, inputs: dict[str, str], dependencies: dict[str, str], evidence: dict[str, Any], error: str | None = None) -> str:
    if status not in STATUSES: raise RuntimeError("invalid readiness status")
    receipt = {"schema":SCHEMA,"storeId":store.store_id,"graphHash":store.graph_hash,"checkerVersion":CHECKER_VERSION,"node":node,"nodeKey":node_key(store,node,inputs,dependencies),"status":status,"selectedInputs":selected_inputs(store,node,inputs),"dependencies":dependencies,"evidence":evidence,"error":error,"createdUnixNs":time.time_ns()}
    return store.publish(receipt)


def current_dependency_receipts(store: Store, node: str, inputs: dict[str,str] | None = None) -> tuple[dict[str,str], list[str]]:
    dependencies = {}; blockers = []
    for dependency in dependency_map(store.graph)[node]:
        pointer = store.pointer(dependency); receipt = store.receipt(dependency) if pointer else None
        dependency_status = assess(store,dependency,inputs)[0] if inputs is not None else (receipt or {}).get("status")
        if not pointer or not receipt or dependency_status != "READY": blockers.append(dependency)
        else: dependencies[dependency] = pointer["receiptHash"]
    return dependencies, blockers


def publish_checked(store: Store, node: str, inputs: dict[str,str], checker: Any) -> str:
    with store.locked():
        dependencies, blockers = current_dependency_receipts(store,node,inputs)
        if blockers: return publish(store,node,"BLOCKED",inputs,dependencies,{"blockers":blockers},"dependency not READY")
        try:
            evidence = checker(); status, error = "READY", None
        except Exception as exception:
            evidence, status, error = {}, "FAILED", f"{type(exception).__name__}:{exception}"
        existing=store.receipt(node); pointer=store.pointer(node)
        if existing and pointer and existing.get("nodeKey")==node_key(store,node,inputs,dependencies) and existing.get("status")==status and existing.get("evidence")==evidence and existing.get("error")==error:
            return pointer["receiptHash"]
        return publish(store,node,status,inputs,dependencies,evidence,error)


def assess(store: Store, node: str, inputs: dict[str,str]) -> tuple[str,list[str],dict[str,Any]|None]:
    receipt = store.receipt(node)
    if receipt is None: return "BLOCKED", ["missing receipt"], None
    dependencies, blockers = current_dependency_receipts(store,node,inputs)
    if blockers:
        blocked_by = {}
        for dependency in blockers:
            pointer = store.pointer(dependency)
            dependency_status = assess(store,dependency,inputs)[0] if pointer else "BLOCKED"
            blocked_by[dependency] = {"status":dependency_status,"receiptHash":pointer["receiptHash"] if pointer else None}
        return "BLOCKED", ["blockedBy:" + json.dumps(blocked_by,sort_keys=True,separators=(",",":"))], receipt
    expected = node_key(store,node,inputs,dependencies)
    if receipt.get("storeId") != store.store_id or receipt.get("graphHash") != store.graph_hash or receipt.get("checkerVersion") != CHECKER_VERSION or receipt.get("selectedInputs")!=selected_inputs(store,node,inputs) or receipt.get("nodeKey") != expected:
        return "STALE", ["node key/current inputs changed"], receipt
    return receipt["status"], ([receipt["error"]] if receipt.get("error") else []), receipt


def root_receipt(store: Store, root: str, inputs: dict[str,str]) -> dict[str,Any]:
    if root not in store.graph["roots"]: raise RuntimeError(f"wrong readiness root: {root}")
    status, reasons, receipt = assess(store,root,inputs)
    if status != "READY" or receipt is None: raise RuntimeError(f"readiness root {root} is {status}: {reasons}")
    return receipt


class ContextSnapshot:
    def __init__(self,harness:Any,binary:Path,seed:Path,diagnostic_experiment:Path,r1_experiment:Path|None=None,diagnostic_output:Path|None=None,r1_output:Path|None=None,diagnostic_preflight:Path|None=None,diagnostic_audit:Path|None=None,store:Store|None=None,semantic_corpus:Path|None=None):
        self.harness=harness; self.binary=binary; self.seed=seed; self.diagnostic_experiment=diagnostic_experiment; self.r1_experiment=r1_experiment; self.diagnostic_output=diagnostic_output; self.r1_output=r1_output; self.diagnostic_preflight=diagnostic_preflight; self.diagnostic_audit=diagnostic_audit; self.store=store; self.semantic_corpus=semantic_corpus; self.memo:dict[str,str]={}; self.errors:dict[str,str]={}
    def _public(self,root:Path,series:str)->str:
        rows=self.harness.discover_public(root)
        members=[{"taskId":value["taskId"],"manifestSha256":digest_file(path),"sourceSha256":self.harness.source_digest(path.parent/value["repository"])} for path,value in rows]
        if len(members)!=42 or len({item["taskId"] for item in members})!=42: raise RuntimeError(f"{series} public corpus denominator mismatch")
        return digest_bytes(canonical({"schema":"e04-public-set-envelope/0.1","series":series,"root":str(root.resolve()),"members":sorted(members,key=lambda item:item["taskId"])}))
    def _json_artifact(self,path:Path,schema:str,series:str)->str:
        if path.is_symlink() or not path.is_file() or not stat.S_ISREG(path.lstat().st_mode): raise RuntimeError(f"{series} artifact is absent or unsafe")
        value=load_json(path)
        if not isinstance(value,dict) or value.get("schema")!=schema: raise RuntimeError(f"{series} artifact schema mismatch")
        return digest_bytes(canonical({"schema":"e04-live-artifact-envelope/0.1","series":series,"root":str(path.parent.resolve()),"artifact":value}))
    def _store_artifact(self,name:str)->str:
        pointer=self.store.root/f"artifacts/{name}.json" if self.store is not None else None
        if pointer is None or (not pointer.exists() and not pointer.is_symlink()): return ABSENT
        if pointer.is_symlink() or not pointer.is_file(): raise RuntimeError(f"{name} pointer is unsafe")
        value=load_json(pointer); identity=value.get("sha256") if isinstance(value,dict) and set(value)=={"schema","sha256"} and value.get("schema")=="e04-readiness-artifact-pointer/0.1" else None
        artifact=self.store.root/"objects"/f"{identity}.json"
        if not isinstance(identity,str) or artifact.is_symlink() or not artifact.is_file() or digest_file(artifact)!=identity: raise RuntimeError(f"{name} live artifact mismatch")
        return identity
    def _jsonl_set(self,path:Path,count:int,series:str,schemas:set[str]|None=None)->str:
        if path.is_symlink() or not path.is_file() or not stat.S_ISREG(path.lstat().st_mode): raise RuntimeError(f"{series} packet set is absent or unsafe")
        rows=[]
        for line in path.read_text(encoding="utf-8").splitlines():
            value=json.loads(line)
            if not isinstance(value,dict) or not isinstance(value.get("runId"),str): raise RuntimeError(f"{series} packet contract mismatch")
            if schemas is not None and value.get("schema") not in schemas: raise RuntimeError(f"{series} packet schema mismatch")
            rows.append(value)
        if len(rows)!=count or len({row["runId"] for row in rows})!=count: raise RuntimeError(f"{series} packet denominator mismatch")
        tasks={row.get("taskId") for row in rows}
        if count==3:
            public_ids=sorted(value["taskId"] for _,value in self.harness.discover_public(self.diagnostic_experiment))
            if len(tasks)!=1 or {row.get("arm") for row in rows}!={"default","ast-index","codeclew"} or tasks!={public_ids[0]}: raise RuntimeError("diagnostic canary is not the preregistered exact triplet")
        if count==126 and (len(tasks)!=42 or any({item.get("arm") for item in rows if item.get("taskId")==task}!={"default","ast-index","codeclew"} for task in tasks)): raise RuntimeError(f"{series} matrix layout mismatch")
        return digest_bytes(canonical({"schema":"e04-live-packet-set-envelope/0.1","series":series,"root":str(path.parent.resolve()),"members":sorted(rows,key=lambda row:row["runId"])}))
    def get(self,key:str)->str:
        if key not in CONTEXT_KEYS: raise RuntimeError(f"unknown readiness context key:{key}")
        if key in self.memo:return self.memo[key]
        if key in BASE_AUTHORITY_KEYS:
            catalog=self.harness.load_typed_goal_catalog(self.binary.resolve()); adapter=self.harness.load_refusal_adapter(catalog); ast=validated_ast_provenance(self.harness.ast_index_provenance())
            base={"binarySha256":catalog["binarySha256"],"catalogSha256":catalog["catalogSha256"],"adapterSha256":adapter["adapterSha256"],"binaryRealPath":str(self.binary.resolve()),"runnerSha256":digest_file(Path(self.harness.__file__)),"populationSha256":digest_file(self.harness.POPULATION),"outputSchemaSha256":digest_file(self.harness.OUTPUT_SCHEMA),"commonPromptSha256":digest_bytes(self.harness.common_prompt(self.harness.population(),catalog).encode()),"corpusSha256":digest_bytes(canonical({str(path.relative_to(self.harness.ROOT)):digest_file(path) for path in self.harness.CORPUS_FILES})),"readinessCheckerSha256":digest_file(Path(__file__)),"codexVersion":subprocess.run(["codex","--version"],text=True,capture_output=True,check=True).stdout.strip(),"astBinarySha256":ast["binarySha256"]}
            self.memo.update(base)
        elif key=="dependencySeedManifestSha256": self.memo[key]=self.harness.validate_dependency_seed(self.seed)["manifestSha256"]
        elif key=="diagnosticPublicSetSha256": self.memo[key]=self._public(self.diagnostic_experiment,"DIAGNOSTIC")
        elif key=="diagnosticFreezeSha256": self.memo[key]=self._store_artifact("diagnostic-freeze")
        elif key in {"semanticCorpusBinarySha256","semanticCorpusBinaryRealPath"}:
            path=self.semantic_corpus
            if path is None or not path.is_absolute() or path.is_symlink() or not path.is_file() or not stat.S_ISREG(path.lstat().st_mode): raise RuntimeError("semantic-corpus executable provenance is absent or unsafe")
            self.memo[key]=digest_file(path) if key.endswith("Sha256") else str(path.resolve())
        elif key=="productCoverageSha256": self.memo[key]=self._store_artifact("product-coverage")
        elif key=="productCoverageAuditSha256": self.memo[key]=self._store_artifact("product-coverage-audit")
        elif key=="productCoverageFailedReceiptSha256":
            pointer=self.store.pointer("PRODUCT_COVERAGE_GUARD") if self.store is not None else None; receipt=self.store.receipt("PRODUCT_COVERAGE_GUARD") if pointer else None; report=self._store_artifact("product-coverage")
            if not pointer or not receipt or receipt.get("status")!="FAILED" or receipt.get("error")!="COVERAGE_BELOW_PREREGISTERED_THRESHOLD" or ((receipt.get("evidence") or {}).get("coverageReportSha256"))!=report: raise RuntimeError("current product coverage guard is not the exact FAILED authority receipt")
            self.memo[key]=pointer["receiptHash"]
        elif key=="r1DecisionSha256": self.memo[key]=self._store_artifact("r1-decision")
        elif key=="r1PublicSetSha256": self.memo[key]=ABSENT if self.r1_experiment is None else self._public(self.r1_experiment,"R1")
        elif key=="r1ControllerTreeSha256": self.memo[key]=ABSENT if self.r1_experiment is None or not self.r1_experiment.exists() else inspect_r1_materialized(self.harness,self.r1_experiment)["controllerTreeSha256"]
        elif key=="r1AnnotationASha256": self.memo[key]=self._store_artifact("r1-annotation-a")
        elif key=="r1AnnotationBSha256": self.memo[key]=self._store_artifact("r1-annotation-b")
        elif key=="r1HiddenVerifySha256": self.memo[key]=self._store_artifact("r1-hidden-verify")
        elif key=="r1CoverageSha256": self.memo[key]=self._store_artifact("r1-coverage")
        elif key=="diagnosticPreflightSha256": self.memo[key]=ABSENT if self.diagnostic_preflight is None or (not self.diagnostic_preflight.exists() and not self.diagnostic_preflight.is_symlink()) else self._json_artifact(self.diagnostic_preflight,"semantic-editing-e04-preflight/0.2","DIAGNOSTIC_PREFLIGHT")
        elif key=="diagnosticAuditSha256": self.memo[key]=ABSENT if self.diagnostic_audit is None or (not self.diagnostic_audit.exists() and not self.diagnostic_audit.is_symlink()) else self._json_artifact(self.diagnostic_audit,"semantic-editing-e04-independent-audit/0.1","DIAGNOSTIC_AUDIT")
        elif key=="diagnosticCanaryPacketSetSha256":
            path=self.diagnostic_output/"runs.jsonl" if self.diagnostic_output is not None else None
            self.memo[key]=ABSENT if path is None or (not path.exists() and not path.is_symlink()) else self._jsonl_set(path,3,"DIAGNOSTIC_CANARY")
        elif key=="finalPacketSetSha256":
            if self.r1_output is None or not (self.r1_output/"runs.jsonl").exists() or not (self.r1_output/"plan.json").exists(): self.memo[key]=ABSENT
            else:
                plan=load_json(self.r1_output/"plan.json")
                if not isinstance(plan,dict) or set(plan)!={"schema","freeze","experimentRoot","runs","r7CanaryTaskIds"} or plan.get("schema")!="semantic-editing-e04-plan/0.1" or plan.get("experimentRoot")!=str(self.r1_experiment.resolve()) or not isinstance(plan.get("runs"),list) or len(plan["runs"])!=126 or not isinstance(plan.get("r7CanaryTaskIds"),list) or len(plan["r7CanaryTaskIds"])!=2 or len(set(plan["r7CanaryTaskIds"]))!=2: raise RuntimeError("final plan contract mismatch")
                packet_rows=[json.loads(line) for line in (self.r1_output/"runs.jsonl").read_text(encoding="utf-8").splitlines()]
                public_ids={value["taskId"] for _,value in self.harness.discover_public(self.r1_experiment)}; packet_ids={row.get("taskId") for row in packet_rows}
                if len(public_ids)!=42 or packet_ids!=public_ids or {row.get("runId") for row in plan["runs"]}!={row.get("runId") for row in packet_rows}: raise RuntimeError("final plan/packet/public set mismatch")
                self.memo[key]=digest_bytes(canonical({"packetSet":self._jsonl_set(self.r1_output/"runs.jsonl",126,"FINAL_MATRIX"),"plan":self._json_artifact(self.r1_output/"plan.json","semantic-editing-e04-plan/0.1","FINAL_PLAN")}))
        elif key=="judgmentsSha256": self.memo[key]=ABSENT if self.r1_output is None or not (self.r1_output/"judgments.jsonl").exists() else self._jsonl_set(self.r1_output/"judgments.jsonl",126,"JUDGMENTS",{"semantic-editing-e04-judgment/0.2"})
        elif key=="summarySha256": self.memo[key]=ABSENT if self.r1_output is None or not (self.r1_output/"summary.json").exists() else self._json_artifact(self.r1_output/"summary.json","semantic-editing-e04-summary/0.2","SUMMARY")
        elif key=="publicSetSha256": self.memo[key]=self._public(self.r1_experiment or self.diagnostic_experiment,"CURRENT")
        else: self.memo[key]=ABSENT
        return self.memo[key]
    def for_node(self,store:Store,node:str)->dict[str,str]:
        keys:set[str]=set()
        def visit(current:str)->None:
            spec=node_spec(store,current); keys.update(spec["inputSelectors"])
            for dependency in spec["dependencies"]:visit(dependency)
        visit(node)
        context={}
        for key in sorted(keys):
            try: context[key]=self.get(key)
            except Exception as error:
                token="ERROR:"+digest_bytes(f"{type(error).__name__}:{error}".encode()); self.errors[key]=f"{type(error).__name__}:{error}"; context[key]=token
        return context
    def raise_selected(self,store:Store,node:str)->None:
        failures={key:self.errors[key] for key in node_spec(store,node)["inputSelectors"] if key in self.errors}
        if failures: raise RuntimeError("selected context provider failed:"+json.dumps(failures,sort_keys=True,separators=(",",":")))


def exact_tree_digest(root:Path)->str:
    if root.is_symlink() or not root.is_dir(): raise RuntimeError("tree root must be a real directory")
    digest=hashlib.sha256()
    for path in sorted(root.rglob("*"),key=lambda item:item.relative_to(root).as_posix()):
        metadata=path.lstat()
        if stat.S_ISLNK(metadata.st_mode): raise RuntimeError("tree contains a symlink")
        relative=path.relative_to(root).as_posix()
        if stat.S_ISDIR(metadata.st_mode): continue
        if not stat.S_ISREG(metadata.st_mode): raise RuntimeError("tree contains a non-regular entry")
        digest.update(relative.encode()); digest.update(b"\0"); digest.update(path.read_bytes()); digest.update(b"\0")
    return digest.hexdigest()


def inspect_r1_materialized(harness:Any,root:Path,expected_series:str|None=None)->dict[str,Any]:
    if root.is_symlink() or not root.is_dir(): raise RuntimeError("R1 experiment root must be a real directory")
    agent=root/"agent"; controller=root/"controller"
    for directory in (agent,controller):
        if directory.is_symlink() or not directory.is_dir(): raise RuntimeError("R1 agent/controller root is unsafe")
    def task_dirs(directory:Path)->dict[str,Path]:
        entries={}
        for path in directory.iterdir():
            if path.is_symlink() or not path.is_dir(): raise RuntimeError("R1 task root contains an extra/unsafe entry")
            if path.name in entries: raise RuntimeError("duplicate R1 task ID")
            entries[path.name]=path
        if len(entries)!=42:return {}
        return entries
    agents=task_dirs(agent); controllers=task_dirs(controller)
    if len(agents)!=42 or set(agents)!=set(controllers): raise RuntimeError("R1 materialized denominator mismatch")
    public_members=[]; controller_members=[]; series=None
    controller_keys=["schema","taskId","seriesId","controllerSeedCommitment","slot","seed","binderFreeze","binderTreeSha256","populationSha256","requiredBindings","requiredObligations","expectedOutcome","expectedOracleClass","ambiguousChoices","refusalReason","commitments","publicManifestSha256","commitment"]
    for task_id in sorted(agents):
        public_path=agents[task_id]/"task-manifest.json"; repository=agents[task_id]/"repository"; controller_path=controllers[task_id]/"manifest.json"
        for path in (public_path,controller_path):
            if path.is_symlink() or not path.is_file(): raise RuntimeError("R1 manifest is absent or unsafe")
        if repository.is_symlink() or not repository.is_dir(): raise RuntimeError("R1 repository is absent or unsafe")
        public=load_json(public_path); hidden=load_json(controller_path)
        if not isinstance(public,dict) or public.get("schema")!="semantic-editing-e04-public-task/0.1" or public.get("taskId")!=task_id or public.get("repository")!="repository": raise RuntimeError("R1 public manifest contract mismatch")
        if not isinstance(hidden,dict) or list(hidden)!=controller_keys or hidden.get("schema")!="semantic-editing-e04-controller/0.2" or hidden.get("taskId")!=task_id: raise RuntimeError("R1 controller 0.2 contract mismatch")
        if series is None: series=hidden.get("seriesId")
        if not isinstance(series,str) or len(series)!=64 or hidden.get("seriesId")!=series or (expected_series is not None and series!=expected_series): raise RuntimeError("R1 controller series mismatch")
        stable=dict(hidden); stable["publicManifestSha256"]=""; stable["commitment"]=""
        commitment=digest_bytes(json.dumps(stable,separators=(",",":"),ensure_ascii=False).encode())
        public_bytes=public_path.read_bytes(); source=harness.source_digest(repository)
        if hidden.get("commitment")!=commitment or public.get("controllerManifestCommitment")!=commitment or hidden.get("publicManifestSha256")!=digest_bytes(public_bytes) or public.get("sourceSnapshotSha256")!=source: raise RuntimeError("R1 manifest authority binding mismatch")
        public_members.append({"taskId":task_id,"publicManifestSha256":digest_bytes(public_bytes),"repositorySourceSha256":source})
        controller_members.append({"taskId":task_id,"controllerManifestSha256":digest_file(controller_path)})
        if set(path.name for path in controllers[task_id].iterdir())!={"manifest.json"}: raise RuntimeError("R1 controller tree contains extra entries")
    controller_selector_members=[{"taskId":item["taskId"],"manifestSha256":item["controllerManifestSha256"]} for item in controller_members]
    controller_selector=digest_bytes(canonical({"schema":"e04-controller-set-envelope/0.1","series":"R1","root":str(root.resolve()),"members":controller_selector_members}))
    return {"seriesId":series,"taskIds":sorted(agents),"agentPublicMembers":public_members,"agentPublicSetSha256":digest_bytes(canonical(public_members)),"controllerMembers":controller_members,"controllerSetSha256":digest_bytes(canonical(controller_members)),"controllerTreeSha256":controller_selector,"controllerRawTreeSha256":exact_tree_digest(controller),"canonicalRoot":str(root.resolve())}


def validate_materialization_result(harness:Any,result:Any,root:Path,decision_sha:str,root_receipt_sha:str,authorization_sha:str)->dict[str,Any]:
    keys={"schema","authorizationEnvelopeSha256","rootReceiptSha256","decisionFreezeSha256","seriesId","outputPath","taskCount","agentPublicMembers","agentPublicSetSha256","controllerMembers","controllerSetSha256"}
    if not isinstance(result,dict) or set(result)!=keys or result.get("schema")!="semantic-editing-e04-r1-materialization-result/0.1" or result.get("taskCount")!=42: raise RuntimeError("R1 materialization result contract mismatch")
    live=inspect_r1_materialized(harness,root,result.get("seriesId"))
    if result.get("authorizationEnvelopeSha256")!=authorization_sha or result.get("rootReceiptSha256")!=root_receipt_sha or result.get("decisionFreezeSha256")!=decision_sha or result.get("outputPath")!=live["canonicalRoot"] or result.get("agentPublicMembers")!=live["agentPublicMembers"] or result.get("agentPublicSetSha256")!=live["agentPublicSetSha256"] or result.get("controllerMembers")!=live["controllerMembers"] or result.get("controllerSetSha256")!=live["controllerSetSha256"]: raise RuntimeError("R1 materialization result/live output mismatch")
    return live


def check_public(harness: Any, experiment: Path) -> dict[str,Any]:
    tasks = harness.discover_public(experiment); counts = {"GRADLE":0,"MAVEN":0}; maven_leaf_count=0; manifests=[]
    for manifest_path, public in tasks:
        build = str(public["buildSystem"]).upper(); counts[build] += 1; repository=manifest_path.parent/public["repository"]
        if harness.source_digest(repository) != public["sourceSnapshotSha256"]: raise RuntimeError(f"source mismatch:{public['taskId']}")
        if build == "GRADLE":
            script, jar, properties = repository/"gradlew", repository/"gradle/wrapper/gradle-wrapper.jar", repository/"gradle/wrapper/gradle-wrapper.properties"
            if script.is_symlink() or not script.is_file() or not script.stat().st_mode & stat.S_IXUSR or not script.read_bytes().startswith(b"#!") or jar.is_symlink() or not zipfile.is_zipfile(jar): raise RuntimeError(f"invalid Gradle assets:{public['taskId']}")
            with zipfile.ZipFile(jar) as archive:
                if "org/gradle/wrapper/GradleWrapperMain.class" not in archive.namelist(): raise RuntimeError("invalid Gradle wrapper jar")
            if "distributionUrl=https\\://services.gradle.org/distributions/gradle-" not in properties.read_text(): raise RuntimeError("invalid Gradle wrapper properties")
        else:
            gavs=set()
            for leaf in harness.maven_reactor_leaves(repository):
                if leaf["gav"] in gavs: raise RuntimeError(f"duplicate Maven GAV:{leaf['gav']}")
                gavs.add(leaf["gav"]); maven_leaf_count+=1
            for pom in repository.rglob("pom.xml"):
                tree=ET.parse(pom).getroot()
                for plugin in (node for node in tree.iter() if node.tag.rsplit('}',1)[-1]=="plugin"):
                    artifact=next((child.text or "" for child in plugin if child.tag.rsplit('}',1)[-1]=="artifactId"),"").strip()
                    group=next((child.text or "" for child in plugin if child.tag.rsplit('}',1)[-1]=="groupId"),"").strip()
                    if artifact=="maven-surefire-plugin" and group!="org.apache.maven.plugins": raise RuntimeError("wrong surefire coordinate")
        manifests.append(digest_file(manifest_path))
    if counts != {"GRADLE":21,"MAVEN":21}: raise RuntimeError(f"public denominator mismatch:{counts}")
    return {"tasks":42,"buildCounts":counts,"publicManifestSetSha256":digest_bytes(canonical(sorted(manifests))),"mavenLeafGavs":maven_leaf_count}


def check_seed(harness: Any, seed: Path) -> dict[str,Any]:
    value=harness.validate_dependency_seed(seed); augmentation=value.get("augmentation")
    if not isinstance(augmentation,dict): raise RuntimeError("dependency seed lacks augmentation")
    offline=augmentation.get("offlineVerificationCommands")
    if not isinstance(offline,list) or len(offline)!=21: raise RuntimeError("offline Maven denominator is not 21")
    for task in offline:
        leaves=task.get("leaves")
        if not leaves or any(leaf.get("exitCode")!=0 or (leaf.get("dependencyBearing") and not leaf.get("artifacts")) for leaf in leaves): raise RuntimeError("offline Maven leaf verification incomplete")
    return {"manifestSha256":value["manifestSha256"],"sealSha256":value.get("sealSha256"),"offlineMavenTasks":21,"maven":value["maven"]}


def check_preflight(path: Path, harness: Any, experiment: Path, inputs: dict[str,str], freeze_hash: str, start_receipt_hash: str, binary: Path, store_root: Path, captured_report: dict[str,Any] | None = None, captured_report_sha: str | None = None) -> dict[str,Any]:
    value=captured_report if captured_report is not None else load_json(path)
    top={"schema","modelCalls","dependencySeed","typedGoalCatalog","astIndexExecutable","refusalAdapterSha256","diagnosticFreezeArtifactHash","readinessRootReceiptHash","tasks","allInfrastructureValid","allAstReady","allCodeclewReady","buildCounts","expectedBuildCounts","selectedTaskIds","provenanceError","productUnsupported","wallMilliseconds","rows","status","aggregatePostconditionErrors"}
    if not isinstance(value,dict) or set(value)!=top or value.get("schema")!="semantic-editing-e04-preflight/0.2" or value.get("status")!="PREFLIGHT_PASSED": raise RuntimeError("full preflight top-level contract mismatch")
    dependency_seed=value["dependencySeed"]; typed_catalog=value["typedGoalCatalog"]; top_ast=value["astIndexExecutable"]
    if not isinstance(dependency_seed,dict) or not isinstance(typed_catalog,dict) or not isinstance(top_ast,dict): raise RuntimeError("full preflight provenance object type mismatch")
    if set(typed_catalog)!={"catalogSha256","binarySha256"} or not all(isinstance(typed_catalog[key],str) for key in typed_catalog): raise RuntimeError("full preflight typed catalog contract mismatch")
    if not isinstance(value["selectedTaskIds"],list) or not all(isinstance(item,str) for item in value["selectedTaskIds"]): raise RuntimeError("full preflight selected task IDs type mismatch")
    if not isinstance(value["rows"],list) or not isinstance(value["buildCounts"],dict) or not isinstance(value["expectedBuildCounts"],dict) or not isinstance(value["aggregatePostconditionErrors"],list): raise RuntimeError("full preflight aggregate object type mismatch")
    if not all(isinstance(value[key],str) for key in ("refusalAdapterSha256","diagnosticFreezeArtifactHash","readinessRootReceiptHash")) or not all(isinstance(value[key],int) and not isinstance(value[key],bool) for key in ("modelCalls","tasks","productUnsupported","wallMilliseconds")): raise RuntimeError("full preflight scalar type mismatch")
    current_ast=validated_ast_provenance(harness.ast_index_provenance())
    public_rows=harness.discover_public(experiment)
    if not isinstance(public_rows,list) or len(public_rows)!=42 or any(not isinstance(item,dict) or not isinstance(item.get("taskId"),str) for _,item in public_rows) or len({item["taskId"] for _,item in public_rows})!=42: raise RuntimeError("current public population contract mismatch")
    public={item["taskId"]:(manifest,item) for manifest,item in public_rows}; selected=value.get("selectedTaskIds"); rows=value.get("rows")
    if value.get("modelCalls")!=0 or value.get("tasks")!=42 or not isinstance(rows,list) or len(rows)!=42 or not isinstance(selected,list) or len(selected)!=42 or len(set(selected))!=42 or set(selected)!=set(public): raise RuntimeError("full preflight denominator/public set mismatch")
    if value.get("buildCounts")!={"GRADLE":21,"MAVEN":21} or value.get("expectedBuildCounts")!={"GRADLE":21,"MAVEN":21} or value.get("aggregatePostconditionErrors")!=[] or value.get("provenanceError") is not None or not all(value.get(key) is True for key in ("allInfrastructureValid","allAstReady","allCodeclewReady")): raise RuntimeError("full preflight aggregate invariant mismatch")
    if value.get("diagnosticFreezeArtifactHash")!=freeze_hash or value.get("readinessRootReceiptHash")!=start_receipt_hash or typed_catalog!={"catalogSha256":inputs["catalogSha256"],"binarySha256":inputs["binarySha256"]} or value.get("refusalAdapterSha256")!=inputs["adapterSha256"] or dependency_seed.get("manifestSha256")!=inputs["dependencySeedManifestSha256"] or top_ast!=current_ast: raise RuntimeError("full preflight top-level provenance mismatch")
    keys={"taskId","publicManifestSha256","publicSourceSnapshotSha256","sourceBeforeSha256","sourceAfterSha256","buildSystem","gitHead","projectRoot","discoveredCompilations","selectedCompilation","compilationDiscoveryEvidence","sourceStable","checkoutCleanBeforeAllTools","checkoutCleanAfterAllTools","toolCleanliness","stateRootOutsideCheckout","externalResultsStateRootOutsideCheckout","repositoryOwnedMutableState","dependencySeedManifestSha256","typedGoalCatalogSha256","codeclewBinarySha256","astIndexExecutable","astRebuildStdoutSha256","astStatsStdoutSha256","astReadinessSummary","astDbSha256","astDbActualSizeBytes","astStateAnchor","offlineHermetic","astReady","codeclewProjectReady","projectSchemaValid","projectRequestCompilation","infrastructureValid","productUnsupported","astExitCode","astStatsExitCode","codeclewExitCode","codeclewDiagnostic","wallMilliseconds"}
    compilation_keys={"buildSystem","projectRoot","projectPath","sourceSet","compilation","compileTask"}
    digest_fields=("publicManifestSha256","publicSourceSnapshotSha256","sourceBeforeSha256","sourceAfterSha256","dependencySeedManifestSha256","typedGoalCatalogSha256","codeclewBinarySha256","astRebuildStdoutSha256","astStatsStdoutSha256","astDbSha256")
    def is_digest(item: Any) -> bool: return isinstance(item,str) and len(item)==64 and all(character in "0123456789abcdef" for character in item)
    seen=set()
    for row in rows:
        if not isinstance(row,dict) or set(row)!=keys: raise RuntimeError("full preflight row key contract mismatch")
        if not isinstance(row["taskId"],str) or not isinstance(row["buildSystem"],str) or not isinstance(row["gitHead"],str) or not isinstance(row["projectRoot"],str) or not isinstance(row["projectRequestCompilation"],str) or not isinstance(row["codeclewDiagnostic"],str): raise RuntimeError("full preflight row string type mismatch")
        if not isinstance(row["discoveredCompilations"],list) or not all(isinstance(item,dict) for item in row["discoveredCompilations"]): raise RuntimeError("full preflight row compilations type mismatch")
        if not isinstance(row["selectedCompilation"],dict) or not isinstance(row["compilationDiscoveryEvidence"],dict) or not isinstance(row["repositoryOwnedMutableState"],dict) or not isinstance(row["astIndexExecutable"],dict) or not isinstance(row["astReadinessSummary"],dict) or not isinstance(row["astStateAnchor"],dict) or not isinstance(row["toolCleanliness"],list): raise RuntimeError("full preflight row nested object type mismatch")
        bool_fields=("sourceStable","checkoutCleanBeforeAllTools","checkoutCleanAfterAllTools","stateRootOutsideCheckout","externalResultsStateRootOutsideCheckout","offlineHermetic","astReady","codeclewProjectReady","projectSchemaValid","infrastructureValid","productUnsupported")
        if not all(isinstance(row[field],bool) for field in bool_fields): raise RuntimeError("full preflight row boolean type mismatch")
        int_fields=("astDbActualSizeBytes","astExitCode","astStatsExitCode","codeclewExitCode","wallMilliseconds")
        if not all(isinstance(row[field],int) and not isinstance(row[field],bool) for field in int_fields): raise RuntimeError("full preflight row integer type mismatch")
        if not all(is_digest(row[field]) for field in digest_fields): raise RuntimeError("full preflight row digest type mismatch")
        task=row.get("taskId")
        if task in seen or task not in public or row.get("buildSystem")!=str(public[task][1]["buildSystem"]).upper(): raise RuntimeError("full preflight row task/build mismatch")
        seen.add(task)
        manifest,public_item=public[task]
        source_sha=public_item.get("sourceSnapshotSha256")
        if row["publicManifestSha256"]!=digest_file(manifest) or row["publicSourceSnapshotSha256"]!=source_sha or row["sourceBeforeSha256"]!=source_sha or row["sourceAfterSha256"]!=source_sha or not is_digest(source_sha): raise RuntimeError("full preflight row source/public provenance mismatch")
        if len(row["gitHead"])!=40 or any(character not in "0123456789abcdef" for character in row["gitHead"]): raise RuntimeError("full preflight row Git HEAD mismatch")
        flags=("sourceStable","checkoutCleanBeforeAllTools","checkoutCleanAfterAllTools","stateRootOutsideCheckout","externalResultsStateRootOutsideCheckout","offlineHermetic","astReady","codeclewProjectReady","projectSchemaValid","infrastructureValid")
        if not all(row.get(field) is True for field in flags) or row.get("productUnsupported") is not False: raise RuntimeError("full preflight row readiness false")
        if row.get("dependencySeedManifestSha256")!=inputs["dependencySeedManifestSha256"] or row.get("typedGoalCatalogSha256")!=inputs["catalogSha256"] or row.get("codeclewBinarySha256")!=inputs["binarySha256"] or row.get("astIndexExecutable")!=current_ast: raise RuntimeError("full preflight row provenance mismatch")
        state=row["repositoryOwnedMutableState"]
        cache_keys={"gradleModules","gradleWrapper","mavenRepository"}
        if set(state)!={"insideCheckout","ignoredByGit","regularDirectories","seedCloneSha256","currentTreeSha256"} or not all(isinstance(state[field],bool) and state[field] is True for field in ("insideCheckout","ignoredByGit","regularDirectories")) or not isinstance(state["seedCloneSha256"],dict) or not isinstance(state["currentTreeSha256"],dict) or set(state["seedCloneSha256"])!=cache_keys or set(state["currentTreeSha256"])!=cache_keys or not all(is_digest(item) for item in state["seedCloneSha256"].values()) or state["seedCloneSha256"]!=state["currentTreeSha256"]: raise RuntimeError("full preflight row cache mismatch")
        compilations=row["discoveredCompilations"]; selected_compilation=row["selectedCompilation"]
        if not compilations or set(selected_compilation)!=compilation_keys or any(set(item)!=compilation_keys or not all(isinstance(item[key],str) and item[key] for key in compilation_keys) for item in compilations): raise RuntimeError("full preflight compilation contract mismatch")
        if selected_compilation not in compilations or compilations.count(selected_compilation)!=1 or selected_compilation["buildSystem"]!=row["buildSystem"] or selected_compilation["sourceSet"]!="main" or selected_compilation["projectRoot"]!=row["projectRoot"] or row["projectRequestCompilation"]!=selected_compilation["compilation"]: raise RuntimeError("full preflight selected compilation mismatch")
        if len({item["compilation"] for item in compilations})!=len(compilations): raise RuntimeError("full preflight duplicate compilation identity")
        tests=[item for item in compilations if item["sourceSet"]=="test" and item["projectRoot"]==selected_compilation["projectRoot"] and item["projectPath"]==selected_compilation["projectPath"]]
        if len(tests)!=1: raise RuntimeError("full preflight matching test compilation mismatch")
        test_compilation=tests[0]
        if test_compilation["buildSystem"]!=row["buildSystem"]: raise RuntimeError("full preflight test build mismatch")
        if row["buildSystem"]=="GRADLE":
            prefix=selected_compilation["projectPath"] if selected_compilation["projectPath"]!=":" else ""
            if selected_compilation["compilation"]!=f"{selected_compilation['projectPath']}/main" or selected_compilation["compileTask"]!=f"{prefix}:compileKotlin" or test_compilation["compilation"]!=f"{selected_compilation['projectPath']}/test" or test_compilation["compileTask"]!=f"{prefix}:compileTestKotlin": raise RuntimeError("full preflight Gradle compilation DTO mismatch")
        elif selected_compilation["projectPath"]!=":" or selected_compilation["compilation"]!=":/main" or selected_compilation["compileTask"]!="compile" or test_compilation["compilation"]!=":/test" or test_compilation["compileTask"]!="test-compile": raise RuntimeError("full preflight Maven compilation DTO mismatch")
        ast=row["astReadinessSummary"]; ast_keys={"schema","status","dbPath","dbSizeBytes","actualDbSizeBytes","dbSha256","fileCount","moduleCount","symbolCount","refsCount"}; anchor=row["astStateAnchor"]
        if set(ast)!=ast_keys or not all(isinstance(ast[key],str) for key in ("schema","status","dbPath","dbSha256")) or ast.get("schema")!="semantic-editing-e04-ast-readiness/0.1" or ast.get("status")!="READY" or not Path(ast["dbPath"]).is_absolute() or "state" not in Path(ast["dbPath"]).parts or ast.get("dbSha256")!=row.get("astDbSha256") or len(ast["dbSha256"])!=64 or ast.get("actualDbSizeBytes")!=row.get("astDbActualSizeBytes") or ast.get("actualDbSizeBytes")!=ast.get("dbSizeBytes") or any(not isinstance(ast.get(key),int) or isinstance(ast.get(key),bool) or ast[key]<=0 for key in ("dbSizeBytes","actualDbSizeBytes","fileCount","moduleCount","symbolCount","refsCount")) or set(anchor)!={"rootName","parentIdentity","rootIdentity"} or anchor.get("rootName")!="state" or any(not isinstance(anchor[key],list) or len(anchor[key])!=2 or not all(isinstance(item,int) and item>0 for item in anchor[key]) for key in ("parentIdentity","rootIdentity")) or anchor["parentIdentity"]==anchor["rootIdentity"]: raise RuntimeError("full preflight row AST mismatch")
        observations=row.get("toolCleanliness")
        if any(not isinstance(item,dict) or set(item)!={"command","exitCode","checkoutCleanBefore","checkoutCleanAfter"} or not isinstance(item["command"],list) or not item["command"] or not all(isinstance(token,str) for token in item["command"]) or not isinstance(item["exitCode"],int) or item["exitCode"]!=0 or item["checkoutCleanBefore"] is not True or item["checkoutCleanAfter"] is not True for item in observations): raise RuntimeError("full preflight tool observation mismatch")
        ast_real=current_ast["realPath"]
        ast_commands=[[ast_real,"rebuild","--format","json"],[ast_real,"stats","--format","json"]]
        clew_command=[str(binary.resolve()),"project","inspect","--repo",".","--compilation",selected_compilation["compilation"]]
        if row["buildSystem"]=="GRADLE":
            if len(observations)!=4: raise RuntimeError("full preflight Gradle observation denominator mismatch")
            discovery=observations[0]["command"]; repository=Path(discovery[0]).parent
            expected_discovery=[str(repository/"gradlew"),"--offline","--gradle-user-home",str((repository/".gradle").resolve()),"--no-daemon","--console=plain","-q","tasks","--all"]
            expected_commands=[expected_discovery,*ast_commands,clew_command]
            discovery_evidence={"method":"GRADLE_WRAPPER_TASKS","inputsSha256":digest_bytes(json.dumps(expected_discovery,sort_keys=True,separators=(",",":"),ensure_ascii=False).encode())}
        else:
            if len(observations)!=3: raise RuntimeError("full preflight Maven observation denominator mismatch")
            expected_commands=[*ast_commands,clew_command]
            repository=manifest.parent/"repository"; poms=[{"path":str(pom.relative_to(repository)),"sha256":digest_file(pom)} for pom in sorted(repository.rglob("pom.xml"))]
            discovery_evidence={"method":"MAVEN_REACTOR_POMS","inputsSha256":digest_bytes(json.dumps(poms,sort_keys=True,separators=(",",":"),ensure_ascii=False).encode())}
        if row["compilationDiscoveryEvidence"]!=discovery_evidence: raise RuntimeError("full preflight compilation discovery evidence mismatch")
        if [item["command"] for item in observations]!=expected_commands: raise RuntimeError("full preflight exact tool command mismatch")
        if row.get("astExitCode")!=0 or row.get("astStatsExitCode")!=0 or row.get("codeclewExitCode")!=0: raise RuntimeError("full preflight tool/result mismatch")
    if seen!=set(public): raise RuntimeError("full preflight row public set mismatch")
    canonical_report_sha=digest_bytes(canonical(value))
    if captured_report is not None:
        if path.is_symlink() or not path.is_file() or captured_report_sha!=canonical_report_sha or digest_file(path)!=canonical_report_sha: raise RuntimeError("full preflight diagnostic report linkage mismatch")
        report_sha=canonical_report_sha
    else:
        report_sha=digest_file(path)
    if not is_digest(report_sha): raise RuntimeError("full preflight diagnostic report digest mismatch")
    return {"preflightSha256":report_sha,"diagnosticFreezeArtifactHash":freeze_hash,"startRootReceiptHash":start_receipt_hash,"currentInputsSha256":digest_bytes(canonical(inputs)),"publicSetSha256":inputs["diagnosticPublicSetSha256"],"tasks":42,"buildCounts":value["buildCounts"]}


def explain(store: Store, inputs: dict[str,str]) -> dict[str,Any]:
    nodes=[]; first=[]
    for node in dependency_map(store.graph):
        status,reasons,receipt=assess(store,node,inputs); nodes.append({"node":node,"status":status,"reasons":reasons})
        if status!="READY" and not first: first=[node]
    return {"schema":"semantic-editing-e04-readiness-explain/0.1","graphHash":store.graph_hash,"storeId":store.store_id,"firstBlockers":first,"nextAction":f"prepare or verify {first[0]}" if first else "root is ready","nodes":nodes}


def explain_snapshot(store:Store,snapshot:ContextSnapshot)->dict[str,Any]:
    nodes=[]; first=[]
    for node in dependency_map(store.graph):
        inputs=snapshot.for_node(store,node); own_errors={key:snapshot.errors[key] for key in node_spec(store,node)["inputSelectors"] if key in snapshot.errors}
        if own_errors: status,reasons="FAILED",["provider:"+json.dumps(own_errors,sort_keys=True,separators=(",",":"))]
        else: status,reasons,_=assess(store,node,inputs)
        nodes.append({"node":node,"status":status,"reasons":reasons})
        if status!="READY" and not first:first=[node]
    return {"schema":"semantic-editing-e04-readiness-explain/0.1","graphHash":store.graph_hash,"storeId":store.store_id,"firstBlockers":first,"nextAction":f"prepare or verify {first[0]}" if first else "root is ready","nodes":nodes}


def validate_independent_audit(path: Path, store: Store) -> tuple[dict[str,Any],str]:
    if not path.is_absolute() or path.is_symlink() or not path.is_file() or not stat.S_ISREG(path.lstat().st_mode): raise RuntimeError("independent audit must be an absolute regular non-symlink file")
    raw=path.read_bytes()
    try: audit=json.loads(raw)
    except (UnicodeDecodeError,json.JSONDecodeError): raise RuntimeError("independent audit is not canonical JSON")
    keys={"schema","decision","auditor","fullPreflightReceiptHash","graphHash","storeId"}
    if not isinstance(audit,dict) or set(audit)!=keys or raw!=canonical(audit): raise RuntimeError("independent audit exact contract mismatch")
    full=store.pointer("DIAGNOSTIC_FULL_PREFLIGHT_42"); full_receipt=store.receipt("DIAGNOSTIC_FULL_PREFLIGHT_42") if full else None
    if audit!={"schema":"semantic-editing-e04-independent-audit/0.1","decision":"ACCEPT","auditor":INDEPENDENT_AUDITOR_ID,"fullPreflightReceiptHash":(full or {}).get("receiptHash"),"graphHash":store.graph_hash,"storeId":store.store_id} or not full_receipt or full_receipt.get("status")!="READY": raise RuntimeError("independent audit receipt binding mismatch")
    return audit,digest_bytes(raw)


def synthetic_regressions(base: Path) -> dict[str,int]:
    real_ast_shape={"realPath":"/usr/local/bin/ast-index","binarySha256":"a"*64,"version":"ast-index 1.0"}
    if validated_ast_provenance(real_ast_shape)!=real_ast_shape: raise AssertionError("real-shaped AST provenance rejected")
    provenance_counterexamples=1
    for malformed in (
        {"realPath":"/usr/local/bin/ast-index","version":"ast-index 1.0"},
        {"realPath":"/usr/local/bin/ast-index","sha256":"a"*64,"version":"ast-index 1.0"},
        {"realPath":"/usr/local/bin/ast-index","binarySha256":"wrong","version":"ast-index 1.0"},
    ):
        try: validated_ast_provenance(malformed); raise AssertionError("malformed AST provenance accepted")
        except RuntimeError: provenance_counterexamples+=1
    common_fixture=base/"common-inputs"; common_fixture.mkdir(parents=True,exist_ok=True)
    for name in ("runner.py","population.json","schema.json","corpus.txt","public.json"):
        atomic_bytes(common_fixture/name,b"{}\n")
    class CommonHarness:
        __file__=str(common_fixture/"runner.py"); POPULATION=common_fixture/"population.json"; OUTPUT_SCHEMA=common_fixture/"schema.json"; CORPUS_FILES=(common_fixture/"corpus.txt",); ROOT=common_fixture
        provenance=real_ast_shape
        @staticmethod
        def load_typed_goal_catalog(_): return {"binarySha256":"b"*64,"catalogSha256":"c"*64}
        @staticmethod
        def load_refusal_adapter(_): return {"adapterSha256":"d"*64}
        @staticmethod
        def discover_public(_): return [(common_fixture/"public.json",{"taskId":"fixture","repository":"repository"})]
        @staticmethod
        def source_digest(_): return "e"*64
        @staticmethod
        def validate_dependency_seed(_): return {"manifestSha256":"f"*64}
        @staticmethod
        def population(): return {}
        @staticmethod
        def common_prompt(*_): return "prompt"
        @classmethod
        def ast_index_provenance(cls): return cls.provenance
    common_harness=CommonHarness(); common_snapshot=ContextSnapshot(common_harness,Path("/clew"),Path("/seed"),common_fixture)
    common_value={key:ABSENT for key in CONTEXT_KEYS}; common_value.update({key:common_snapshot.get(key) for key in BASE_AUTHORITY_KEYS}); common_value.update({"diagnosticPublicSetSha256":"1"*64,"dependencySeedManifestSha256":"2"*64})
    if common_value.get("astBinarySha256")!="a"*64: raise AssertionError("common_inputs rejected real-shaped AST provenance")
    provenance_counterexamples+=1
    for malformed in ({"realPath":"/ast","version":"v"},{"realPath":"/ast","version":"v","sha256":"a"*64}):
        CommonHarness.provenance=malformed
        try: ContextSnapshot(common_harness,Path("/clew"),Path("/seed"),common_fixture).get("astBinarySha256"); raise AssertionError("base context accepted malformed AST provenance")
        except RuntimeError as error:
            if "AST executable provenance" not in str(error): raise
            provenance_counterexamples+=1
    CommonHarness.provenance=real_ast_shape
    selector_graph={"schema":"semantic-editing-e04-readiness-graph/0.1","version":"selector-test","nodes":[
        {"id":"ARTIFACT_PROVENANCE","action":"VERIFY","checker":"artifact/1","dependencies":[],"inputSelectors":sorted(BASE_AUTHORITY_KEYS)},
        {"id":"PUBLIC","action":"VERIFY","checker":"public/1","dependencies":["ARTIFACT_PROVENANCE"],"inputSelectors":["diagnosticPublicSetSha256"]},
        {"id":"SEED","action":"VERIFY","checker":"seed/1","dependencies":["ARTIFACT_PROVENANCE"],"inputSelectors":["dependencySeedManifestSha256"]},
        {"id":"FUTURE","action":"VERIFY","checker":"future/1","dependencies":["PUBLIC","SEED"],"inputSelectors":["finalPacketSetSha256"]}],"roots":["FUTURE"]}
    selector_store=Store(common_fixture/"selector-store",selector_graph,True)
    for node in ("ARTIFACT_PROVENANCE","PUBLIC","SEED","FUTURE"): publish_checked(selector_store,node,common_value,lambda:{"ok":True})
    for key in BASE_AUTHORITY_KEYS:
        if assess(selector_store,"ARTIFACT_PROVENANCE",{**common_value,key:"changed"})[0]!="STALE": raise AssertionError(f"base selector drift accepted:{key}")
        provenance_counterexamples+=1
    public_changed={**common_value,"diagnosticPublicSetSha256":"changed"}
    if assess(selector_store,"ARTIFACT_PROVENANCE",public_changed)[0]!="READY" or assess(selector_store,"PUBLIC",public_changed)[0]!="STALE" or assess(selector_store,"SEED",public_changed)[0]!="READY": raise AssertionError("public-only invalidation escaped its owner")
    provenance_counterexamples+=1
    seed_changed={**common_value,"dependencySeedManifestSha256":"changed"}
    if assess(selector_store,"ARTIFACT_PROVENANCE",seed_changed)[0]!="READY" or assess(selector_store,"SEED",seed_changed)[0]!="STALE" or assess(selector_store,"PUBLIC",seed_changed)[0]!="READY": raise AssertionError("seed-only invalidation escaped its owner")
    provenance_counterexamples+=1
    future_present={**common_value,"finalPacketSetSha256":"9"*64}
    if any(assess(selector_store,node,future_present)[0]!="READY" for node in ("ARTIFACT_PROVENANCE","PUBLIC","SEED")) or assess(selector_store,"FUTURE",future_present)[0]!="STALE": raise AssertionError("future artifact presence staled an ancestor")
    provenance_counterexamples+=1
    if assess(selector_store,"ARTIFACT_PROVENANCE",{**common_value,"callerFakeDigest":"fake"})[0]!="READY": raise AssertionError("caller fake digest affected selectors")
    provenance_counterexamples+=1
    try: node_key(selector_store,"ARTIFACT_PROVENANCE",{},{}); raise AssertionError("missing selected input accepted")
    except RuntimeError: provenance_counterexamples+=1
    unknown_graph=json.loads(json.dumps(selector_graph)); unknown_graph["nodes"][0]["inputSelectors"].append("callerFakeDigest"); unknown_path=common_fixture/"unknown-selector.json"; atomic_bytes(unknown_path,canonical(unknown_graph))
    try: load_graph(unknown_path); raise AssertionError("unknown input selector accepted")
    except RuntimeError: provenance_counterexamples+=1
    lazy_root=base/"lazy-public"; lazy_root.mkdir()
    lazy_rows=[]
    for index in range(42):
        manifest=lazy_root/f"task-{index:02d}.json"; atomic_bytes(manifest,canonical({"taskId":f"task-{index:02d}"})); lazy_rows.append((manifest,{"taskId":f"task-{index:02d}","repository":"repository"}))
    class LazyHarness(CommonHarness):
        seed_calls=0; public_calls=0
        @classmethod
        def discover_public(cls,_): cls.public_calls+=1; return lazy_rows
        @classmethod
        def validate_dependency_seed(cls,_): cls.seed_calls+=1; raise RuntimeError("seed unavailable")
    lazy=LazyHarness()
    artifact_snapshot=ContextSnapshot(lazy,Path("/clew"),Path("/missing-seed"),lazy_root,store=selector_store)
    artifact_inputs=artifact_snapshot.for_node(selector_store,"ARTIFACT_PROVENANCE")
    if LazyHarness.seed_calls or LazyHarness.public_calls or artifact_snapshot.errors: raise AssertionError("ARTIFACT_PROVENANCE eagerly evaluated unrelated leaves")
    if set(artifact_inputs)!=BASE_AUTHORITY_KEYS: raise AssertionError("lazy artifact selector closure mismatch")
    seed_inputs=artifact_snapshot.for_node(selector_store,"SEED")
    if LazyHarness.seed_calls!=1 or "dependencySeedManifestSha256" not in artifact_snapshot.errors or not seed_inputs["dependencySeedManifestSha256"].startswith("ERROR:"): raise AssertionError("selected seed provider failure was not owned by SEED")
    provenance_counterexamples+=2
    second_root=base/"lazy-r1"; second_root.mkdir()
    dual_snapshot=ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,second_root)
    if dual_snapshot.get("diagnosticPublicSetSha256")==dual_snapshot.get("r1PublicSetSha256"): raise AssertionError("diagnostic and R1 public identities aliased")
    provenance_counterexamples+=1
    diagnostic_output=base/"diagnostic-output"; r1_output=base/"r1-output"; diagnostic_output.mkdir(); r1_output.mkdir()
    arms=("default","ast-index","codeclew")
    canary=[{"runId":f"task-00--{arm}","taskId":"task-00","arm":arm} for arm in arms]
    atomic_bytes(diagnostic_output/"runs.jsonl",b"".join(canonical(row) for row in canary))
    live_snapshot=ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,second_root,diagnostic_output,r1_output)
    canary_digest=live_snapshot.get("diagnosticCanaryPacketSetSha256")
    mutated=json.loads(json.dumps(canary)); mutated[2]["arm"]="default"; atomic_bytes(diagnostic_output/"runs.jsonl",b"".join(canonical(row) for row in mutated))
    try: ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,second_root,diagnostic_output,r1_output).get("diagnosticCanaryPacketSetSha256"); raise AssertionError("relabeled canary packet set accepted")
    except RuntimeError: provenance_counterexamples+=1
    atomic_bytes(diagnostic_output/"runs.jsonl",b"".join(canonical(row) for row in canary)); restored=ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,second_root,diagnostic_output,r1_output).get("diagnosticCanaryPacketSetSha256")
    if restored!=canary_digest: raise AssertionError("canonical canary packet set digest was unstable")
    provenance_counterexamples+=1
    (diagnostic_output/"runs.jsonl").unlink()
    if ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,second_root,diagnostic_output,r1_output).get("diagnosticCanaryPacketSetSha256")!=ABSENT: raise AssertionError("deleted future canary artifact was not ABSENT")
    provenance_counterexamples+=1
    final_rows=[{"runId":f"task-{index:02d}--{arm}","taskId":f"task-{index:02d}","arm":arm} for index in range(42) for arm in arms]
    atomic_bytes(r1_output/"runs.jsonl",b"".join(canonical(row) for row in final_rows))
    final_plan={"schema":"semantic-editing-e04-plan/0.1","freeze":{},"experimentRoot":str(second_root.resolve()),"runs":final_rows,"r7CanaryTaskIds":["task-00","task-21"]}
    atomic_bytes(r1_output/"plan.json",canonical(final_plan))
    final_digest=ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,second_root,diagnostic_output,r1_output).get("finalPacketSetSha256")
    final_plan["runs"]=final_rows[:-1]; atomic_bytes(r1_output/"plan.json",canonical(final_plan))
    try: ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,second_root,diagnostic_output,r1_output).get("finalPacketSetSha256"); raise AssertionError("partial final plan accepted")
    except RuntimeError: provenance_counterexamples+=1
    final_plan["runs"]=final_rows; atomic_bytes(r1_output/"plan.json",canonical(final_plan))
    if ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,second_root,diagnostic_output,r1_output).get("finalPacketSetSha256")!=final_digest: raise AssertionError("canonical final packet/plan digest was unstable")
    provenance_counterexamples+=1
    judgments=[{"schema":"semantic-editing-e04-judgment/0.2",**row} for row in final_rows]
    atomic_bytes(r1_output/"judgments.jsonl",b"".join(canonical(row) for row in judgments))
    judgment_digest=ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,second_root,diagnostic_output,r1_output).get("judgmentsSha256")
    judgments[0]["accepted"]=True; atomic_bytes(r1_output/"judgments.jsonl",b"".join(canonical(row) for row in judgments))
    if ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,second_root,diagnostic_output,r1_output).get("judgmentsSha256")==judgment_digest: raise AssertionError("judgment mutation did not stale live identity")
    provenance_counterexamples+=1
    summary_path=r1_output/"summary.json"; atomic_bytes(summary_path,canonical({"schema":"semantic-editing-e04-summary/0.2","arms":{}}))
    summary_digest=ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,second_root,diagnostic_output,r1_output).get("summarySha256")
    atomic_bytes(summary_path,canonical({"schema":"semantic-editing-e04-summary/0.2","arms":{"default":{}}}))
    if ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,second_root,diagnostic_output,r1_output).get("summarySha256")==summary_digest: raise AssertionError("summary mutation did not stale live identity")
    provenance_counterexamples+=1
    freeze_graph={"schema":"semantic-editing-e04-readiness-graph/0.1","version":"freeze-two-phase","nodes":[{"id":"FREEZE","action":"PREPARE","checker":"freeze/1","dependencies":[],"inputSelectors":["diagnosticFreezeSha256"]}],"roots":["FREEZE"]}
    freeze_store=Store(base/"freeze-two-phase-store",freeze_graph,True)
    pre_freeze=ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,store=freeze_store).for_node(freeze_store,"FREEZE")
    if pre_freeze["diagnosticFreezeSha256"]!=ABSENT: raise AssertionError("future diagnostic freeze was not initially absent")
    freeze_identity=freeze_store.object({"schema":"fixture-diagnostic-freeze/0.1"})
    atomic_bytes(freeze_store.root/"artifacts/diagnostic-freeze.json",canonical({"schema":"e04-readiness-artifact-pointer/0.1","sha256":freeze_identity}))
    post_freeze=ContextSnapshot(LazyHarness(),Path("/clew"),Path("/seed"),lazy_root,store=freeze_store).for_node(freeze_store,"FREEZE")
    publish_checked(freeze_store,"FREEZE",post_freeze,lambda:{"diagnosticFreezeArtifactHash":freeze_identity})
    if assess(freeze_store,"FREEZE",post_freeze)[0]!="READY": raise AssertionError("two-phase diagnostic freeze became stale after creation")
    provenance_counterexamples+=1
    coverage_graph={"schema":"semantic-editing-e04-readiness-graph/0.1","version":"coverage-no-go-test","nodes":[
        {"id":"PRODUCT_COVERAGE_START_READY","action":"VERIFY","checker":"start/1","dependencies":[],"inputSelectors":[]},
        {"id":"PRODUCT_COVERAGE_GUARD","action":"VERIFY","checker":"guard/1","dependencies":["PRODUCT_COVERAGE_START_READY"],"inputSelectors":["semanticCorpusBinarySha256","semanticCorpusBinaryRealPath","productCoverageSha256"]},
        {"id":"PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT","action":"VERIFY","checker":"audit/1","dependencies":["PRODUCT_COVERAGE_START_READY"],"inputSelectors":["semanticCorpusBinarySha256","semanticCorpusBinaryRealPath","productCoverageSha256","productCoverageAuditSha256","productCoverageFailedReceiptSha256"]},
        {"id":"E04_COVERAGE_NO_GO_COMPLETE","action":"VERIFY","checker":"root/1","dependencies":["PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT"],"inputSelectors":[]}],"roots":["PRODUCT_COVERAGE_START_READY","E04_COVERAGE_NO_GO_COMPLETE"]}
    def coverage_store(name:str)->tuple[Store,str]:
        value=Store(base/name,coverage_graph,True); freeze=value.object({"schema":"fixture-freeze/0.1"}); atomic_bytes(value.root/"artifacts/diagnostic-freeze.json",canonical({"schema":"e04-readiness-artifact-pointer/0.1","sha256":freeze})); publish_checked(value,"PRODUCT_COVERAGE_START_READY",{},lambda:{"ready":True}); return value,freeze
    coverage_product=base/"coverage-product"; coverage_product.mkdir(); atomic_bytes(coverage_product/"product.txt",b"product\n"); subprocess.run(["git","init","-q"],cwd=coverage_product,check=True); subprocess.run(["git","add","product.txt"],cwd=coverage_product,check=True); subprocess.run(["git","-c","user.name=E04","-c","user.email=e04@example.invalid","commit","-qm","fixture"],cwd=coverage_product,check=True)
    coverage_binary=base/"coverage-clew"; coverage_authority_binary=base/"semantic-corpus"; atomic_bytes(coverage_binary,b"binary\n"); atomic_bytes(coverage_authority_binary,b"authority\n")
    repository_root=Path(__file__).resolve().parents[1]; fixture_raw_catalog={"schema":"typed-goal-language-schema/0.1","version":"0.1","executableDomains":["MAP","TYPE"],"operators":[
        {"operator":"MAP_EDGE","arity":3,"auxiliaryOnly":False,"constraintDomain":"MAP"},
        {"operator":"PROPAGATE_DECLARED_TYPE","arity":2,"auxiliaryOnly":False,"constraintDomain":"TYPE"},
        {"operator":"NULL_HANDLES","arity":3,"auxiliaryOnly":False,"constraintDomain":"NULL"},
        {"operator":"PROJECTS_VALUE","arity":3,"auxiliaryOnly":False,"constraintDomain":"PROJECTION"},
        {"operator":"PRESERVE_OWNER_BOUNDARY","arity":2,"auxiliaryOnly":False,"constraintDomain":"OWNER"},
        {"operator":"PRESERVE_RESOURCE_LIFETIME","arity":1,"auxiliaryOnly":False,"constraintDomain":"RESOURCE"},
        {"operator":"REQUIRE_OMISSION_DETECTION","arity":1,"auxiliaryOnly":False,"constraintDomain":"TEST"},
    ]}; fixture_catalog_sha=digest_bytes(canonical(fixture_raw_catalog))
    class CoverageHarness:
        ROOT=repository_root; POPULATION=repository_root/"benchmarks/semantic-change/editing-population-v1.json"; __file__=str(Path(__file__).resolve())
        @staticmethod
        def coverage_product_paths(_): return ["product.txt"]
        @staticmethod
        def load_typed_goal_catalog(_): return {**fixture_raw_catalog,"derivedCapabilities":{},"catalogSha256":fixture_catalog_sha,"binarySha256":digest_file(coverage_binary)}
        @staticmethod
        def population(): return load_json(CoverageHarness.POPULATION)
    coverage_contract=load_json(repository_root/"benchmarks/semantic-change/e04-product-coverage-v1.json")
    authority_value={"schema":"semantic-editing-e04-product-coverage/0.1","contractSha256":PRODUCT_COVERAGE_CONTRACT_SHA256,"populationSha256":PRODUCT_COVERAGE_POPULATION_SHA256,"catalogSha256":fixture_catalog_sha,"positiveCells":14,"supportedUpperBound":2,"cellResults":coverage_contract["cells"]}
    def exact_authority_runner(command:Any,**_:Any)->subprocess.CompletedProcess[str]: return subprocess.CompletedProcess(command,0,canonical(authority_value).decode(),"")
    def coverage_factory(value:Store)->Any: return lambda:ContextSnapshot(CoverageHarness,coverage_binary,Path("/seed"),coverage_product,store=value,semantic_corpus=coverage_authority_binary)
    failed_store,failed_freeze=coverage_store("product-coverage-failed-store"); failed_hash=issue_product_coverage(CoverageHarness,failed_store,coverage_binary,coverage_product,coverage_authority_binary,coverage_factory(failed_store),authority_runner=exact_authority_runner); failed_report=load_json(failed_store.root/f"objects/{failed_store.receipt('PRODUCT_COVERAGE_GUARD')['evidence']['coverageReportSha256']}.json")
    if failed_store.receipt("PRODUCT_COVERAGE_GUARD").get("status")!="FAILED" or failed_store.pointer("PRODUCT_COVERAGE_GUARD")["receiptHash"]!=failed_hash: raise AssertionError("below-threshold product coverage minted READY")
    repeated_failed_hash=issue_product_coverage(CoverageHarness,failed_store,coverage_binary,coverage_product,coverage_authority_binary,coverage_factory(failed_store),authority_runner=exact_authority_runner)
    if repeated_failed_hash!=failed_hash: raise AssertionError("identical failed product coverage was not idempotent")
    audit={"schema":"semantic-editing-e04-product-coverage-audit/0.1","decision":"ACCEPT_NO_GO","auditor":INDEPENDENT_AUDITOR_ID,"coverageFailedReceiptHash":failed_hash,"coverageReportSha256":failed_store.receipt("PRODUCT_COVERAGE_GUARD")["evidence"]["coverageReportSha256"],"graphHash":failed_store.graph_hash,"storeId":failed_store.store_id,"diagnosticFreezeSha256":failed_freeze,"productRevision":failed_report["productRevision"],"binarySha256":failed_report["productBinarySha256"],"semanticCorpusBinarySha256":failed_report["semanticCorpusBinarySha256"],"semanticCorpusBinaryRealPath":failed_report["semanticCorpusBinaryRealPath"],"catalogSha256":failed_report["catalogSha256"],"recomputedRequired":PRODUCT_COVERAGE_REQUIRED,"recomputedObserved":failed_report["observed"],"recomputedUpperBound":2,"r1Materialized":False,"controllersOpened":False,"modelCalls":0}
    audit_path=base/"product-coverage-audit.json"; atomic_bytes(audit_path,canonical(audit)); import_product_coverage_failure_audit(CoverageHarness,failed_store,audit_path,coverage_binary,coverage_product,coverage_authority_binary,coverage_factory(failed_store),authority_runner=exact_authority_runner); terminal_inputs=coverage_factory(failed_store)().for_node(failed_store,"E04_COVERAGE_NO_GO_COMPLETE"); publish_checked(failed_store,"E04_COVERAGE_NO_GO_COMPLETE",terminal_inputs,lambda:{"noGo":True}); root_receipt(failed_store,"E04_COVERAGE_NO_GO_COMPLETE",terminal_inputs)
    authority_bytes=coverage_authority_binary.read_bytes(); atomic_bytes(coverage_authority_binary,authority_bytes+b"changed")
    changed_authority_inputs=coverage_factory(failed_store)().for_node(failed_store,"E04_COVERAGE_NO_GO_COMPLETE")
    if assess(failed_store,"E04_COVERAGE_NO_GO_COMPLETE",changed_authority_inputs)[0] not in {"STALE","BLOCKED"}: raise AssertionError("authority executable mutation did not stale terminal coverage")
    atomic_bytes(coverage_authority_binary,authority_bytes)
    original_guard_pointer=failed_store.pointer("PRODUCT_COVERAGE_GUARD"); original_guard=failed_store.receipt("PRODUCT_COVERAGE_GUARD"); guard_inputs=coverage_factory(failed_store)().for_node(failed_store,"PRODUCT_COVERAGE_GUARD"); start_pointer=failed_store.pointer("PRODUCT_COVERAGE_START_READY")
    with failed_store.locked(): publish(failed_store,"PRODUCT_COVERAGE_GUARD","FAILED",guard_inputs,{"PRODUCT_COVERAGE_START_READY":start_pointer["receiptHash"]},{**original_guard["evidence"],"replacement":True},"COVERAGE_BELOW_PREREGISTERED_THRESHOLD")
    replaced_inputs=coverage_factory(failed_store)().for_node(failed_store,"E04_COVERAGE_NO_GO_COMPLETE")
    if assess(failed_store,"E04_COVERAGE_NO_GO_COMPLETE",replaced_inputs)[0] not in {"STALE","BLOCKED"}: raise AssertionError("replaced failed guard pointer did not stale terminal coverage")
    atomic_bytes(failed_store.root/"current/PRODUCT_COVERAGE_GUARD.json",canonical(original_guard_pointer)); restored_terminal_inputs=coverage_factory(failed_store)().for_node(failed_store,"E04_COVERAGE_NO_GO_COMPLETE"); root_receipt(failed_store,"E04_COVERAGE_NO_GO_COMPLETE",restored_terminal_inputs)
    if coverage_factory(failed_store)().for_node(failed_store,"PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT")["productCoverageFailedReceiptSha256"]=="0"*64: raise AssertionError("caller-forged failed receipt digest was trusted")
    forged_store,_=coverage_store("product-coverage-forged-store")
    def forged_authority_runner(command:Any,**_:Any)->subprocess.CompletedProcess[str]:
        forged={**authority_value,"supportedUpperBound":9}; return subprocess.CompletedProcess(command,0,canonical(forged).decode(),"")
    try: issue_product_coverage(CoverageHarness,forged_store,coverage_binary,coverage_product,coverage_authority_binary,coverage_factory(forged_store),authority_runner=forged_authority_runner); raise AssertionError("forged upper-nine authority output was accepted")
    except RuntimeError:
        if forged_store.pointer("PRODUCT_COVERAGE_GUARD") is not None: raise AssertionError("forged authority minted a coverage pointer")
    missing_authority_store,_=coverage_store("product-coverage-missing-authority-store"); missing_factory=lambda:ContextSnapshot(CoverageHarness,coverage_binary,Path("/seed"),coverage_product,store=missing_authority_store)
    try: issue_product_coverage(CoverageHarness,missing_authority_store,coverage_binary,coverage_product,coverage_authority_binary,missing_factory,authority_runner=exact_authority_runner); raise AssertionError("missing semantic-corpus provenance was accepted")
    except RuntimeError:
        if missing_authority_store.pointer("PRODUCT_COVERAGE_GUARD") is not None: raise AssertionError("missing authority provenance minted a coverage pointer")
    positive_threshold=json.loads(json.dumps(failed_report)); positive_threshold.update({"upperBoundPositiveCells":9,"status":"COVERAGE_ACCEPTED","decision":"GO"})
    if not validate_product_coverage_report(positive_threshold,failed_store,failed_freeze)["passes"]: raise AssertionError("pure coverage threshold control failed")
    mutated=json.loads(json.dumps(failed_report)); mutated["upperBoundPositiveCells"]=9
    try: validate_product_coverage_report(mutated,failed_store,failed_freeze); raise AssertionError("forged product coverage upper bound accepted")
    except RuntimeError: provenance_counterexamples+=4
    production_graph=load_graph(Path(__file__).resolve().parents[1]/"benchmarks/semantic-change/e04-readiness-graph.json")
    production_actions={node["id"]:node["action"] for node in production_graph["nodes"]}
    first_twelve=[("ARTIFACT_PROVENANCE","VERIFY"),("DIAGNOSTIC_PUBLIC_CORPUS_42","VERIFY"),("DEPENDENCY_SEED_VERIFY","VERIFY"),("HARNESS_SELF_TEST","VERIFY"),("DIAGNOSTIC_FREEZE_PREPARE","PREPARE"),("DIAGNOSTIC_FREEZE_VERIFY","VERIFY"),("PRODUCT_COVERAGE_START_READY","VERIFY"),("PRODUCT_COVERAGE_GUARD","VERIFY"),("PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT","VERIFY"),("E04_COVERAGE_NO_GO_COMPLETE","VERIFY"),("DIAGNOSTIC_FULL_PREFLIGHT_START_READY","VERIFY"),("DIAGNOSTIC_FULL_PREFLIGHT_42","VERIFY")]
    if [(node["id"],node["action"]) for node in production_graph["nodes"][:12]]!=first_twelve or not {"PRODUCT_COVERAGE_GUARD","DIAGNOSTIC_FULL_PREFLIGHT_42","DIAGNOSTIC_CANARY_3_COMPLETE"}<=DIRECT_NODES or not {"PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT","DIAGNOSTIC_AUDIT_IMPORT"}<=IMPORT_NODES: raise AssertionError("first 12 readiness dispatch/action contract drifted")
    expected_verify={"ARTIFACT_PROVENANCE","DIAGNOSTIC_PUBLIC_CORPUS_42","DEPENDENCY_SEED_VERIFY","HARNESS_SELF_TEST"}
    if {node for node in expected_verify if production_actions.get(node)!="VERIFY"}: raise AssertionError("read-only readiness checker is not VERIFY")
    if {node for node,action in production_actions.items() if action=="PREPARE"}!={"DIAGNOSTIC_FREEZE_PREPARE","R1_DECISION_FREEZE_PREPARE","R1_CORPUS_42_MATERIALIZED"}: raise AssertionError("readiness graph exposes an unexpected PREPARE node")
    if any(node["inputSelectors"] for node in production_graph["nodes"] if node["id"] in ROOT_NODES): raise AssertionError("pure readiness root owns redundant live inputs")
    graph={"schema":"semantic-editing-e04-readiness-graph/0.1","version":"test","nodes":[{"id":"A","action":"PREPARE","checker":"a/1","dependencies":[],"inputSelectors":["binary"]},{"id":"B","action":"VERIFY","checker":"b/1","dependencies":["A"],"inputSelectors":["catalog"]},{"id":"PREFLIGHT_READY","action":"VERIFY","checker":"p/1","dependencies":["B"],"inputSelectors":[]},{"id":"CANARY_START_READY","action":"VERIFY","checker":"c/1","dependencies":["PREFLIGHT_READY"],"inputSelectors":[]}],"roots":["PREFLIGHT_READY","CANARY_START_READY"]}
    store=Store(base/"store",graph,True); inputs={"binary":"a","catalog":"b","runner":"c","public":"d","seed":"e"}; count=1+provenance_counterexamples
    publish_checked(store,"A",inputs,lambda:{"ok":True}); publish_checked(store,"B",inputs,lambda:{"ok":True}); publish_checked(store,"PREFLIGHT_READY",inputs,lambda:{"ok":True}); publish_checked(store,"CANARY_START_READY",inputs,lambda:{"ok":True})
    root_receipt(store,"CANARY_START_READY",inputs); count+=1
    if assess(store,"A",{**inputs,"binary":"changed"})[0]!="STALE" or assess(store,"A",{**inputs,"catalog":"changed"})[0]!="READY": raise AssertionError("node input selectors are not isolated")
    count+=2
    if assess(store,"B",{**inputs,"binary":"changed"})[0]!="BLOCKED" or assess(store,"B",{**inputs,"catalog":"changed"})[0]!="STALE": raise AssertionError("selector/dependency staleness is incorrect")
    count+=2
    other=Store(base/"other",graph,True); shutil.copyfile(store.root/"current/A.json",other.root/"current/A.json")
    try: other.pointer("A"); raise AssertionError("cross-store replay accepted")
    except RuntimeError: count+=1
    atomic_bytes(store.root/"current/.crash.tmp",b"partial")
    root_receipt(store,"CANARY_START_READY",inputs); count+=1
    try: root_receipt(store,"WRONG",inputs); raise AssertionError("wrong root accepted")
    except RuntimeError: count+=1
    original_pointer=(store.root/"current/A.json").read_bytes(); forged=load_json(store.root/"current/A.json"); forged["receiptHash"]="0"*64; atomic_bytes(store.root/"current/A.json",canonical(forged))
    try: store.pointer("A"); raise AssertionError("forged READY accepted")
    except RuntimeError: count+=1
    atomic_bytes(store.root/"current/A.json",original_pointer)
    first=publish_checked(store,"B",inputs,lambda:{"ok":True}); second=publish_checked(store,"B",inputs,lambda:{"ok":True})
    if first!=second: raise AssertionError("idempotent publish changed receipt")
    count+=1
    results=[]
    threads=[threading.Thread(target=lambda:results.append(publish_checked(store,"B",inputs,lambda:{"ok":True}))) for _ in range(2)]
    [thread.start() for thread in threads]; [thread.join() for thread in threads]
    if len(set(results))!=1 or results[0]!=first: raise AssertionError("concurrent publish was not idempotent")
    count+=1
    b_pointer=store.root/"current/B.json"; b_pointer_bytes=b_pointer.read_bytes(); b_pointer.unlink(); b_pointer.symlink_to(store.root/"current/A.json")
    try: store.pointer("B"); raise AssertionError("forged readiness pointer path accepted")
    except RuntimeError: count+=1
    b_pointer.unlink(); atomic_bytes(b_pointer,b_pointer_bytes)
    partial=base/"partial-preflight.json"; atomic_bytes(partial,canonical({"schema":"semantic-editing-e04-preflight/0.2","status":"PREFLIGHT_PASSED","tasks":41,"buildCounts":{"GRADLE":21,"MAVEN":20},"allInfrastructureValid":True,"allAstReady":True,"allCodeclewReady":True}))
    try: check_preflight(partial,None,base,inputs,"x","y",Path("/clew"),base); raise AssertionError("partial denominator accepted")
    except RuntimeError: count+=1
    source_sha="4"*64
    class FakeHarness:
        @staticmethod
        def discover_public(_): return [(base/f"{index}.json",{"taskId":f"t{index:02d}","buildSystem":"GRADLE" if index<21 else "MAVEN","sourceSnapshotSha256":source_sha}) for index in range(42)]
        @staticmethod
        def ast_index_provenance(): return {"realPath":"/ast","version":"v","binarySha256":"a"*64}
    for index in range(42): atomic_bytes(base/f"{index}.json",canonical({"taskId":f"t{index:02d}"}))
    atomic_bytes(base/"repository"/"pom.xml",b"<project/>\n")
    pins={"binarySha256":"b"*64,"catalogSha256":"c"*64,"adapterSha256":"d"*64,"dependencySeedManifestSha256":"e"*64,"diagnosticPublicSetSha256":"f"*64}
    ast_provenance=FakeHarness.ast_index_provenance(); rows=[]; blob_bytes=b"x"; blob_sha=digest_bytes(blob_bytes); blob_path=base/"blobs"/f"{blob_sha}.bin"; atomic_bytes(blob_path,blob_bytes); blob_path.chmod(0o444)
    for index in range(42):
        task=f"t{index:02d}"; build="GRADLE" if index<21 else "MAVEN"; compilation=":/main"
        main={"buildSystem":build,"projectRoot":".","projectPath":":","sourceSet":"main","compilation":compilation,"compileTask":":compileKotlin" if build=="GRADLE" else "compile"}
        test={"buildSystem":build,"projectRoot":".","projectPath":":","sourceSet":"test","compilation":":/test","compileTask":":compileTestKotlin" if build=="GRADLE" else "test-compile"}
        ast={"schema":"semantic-editing-e04-ast-readiness/0.1","status":"READY","dbPath":f"/tmp/{task}/state/ast.db","dbSizeBytes":1,"actualDbSizeBytes":1,"dbSha256":blob_sha,"fileCount":1,"moduleCount":1,"symbolCount":1,"refsCount":1}
        repo=Path(f"/tmp/{task}/repository").resolve()
        commands=[["/ast","rebuild","--format","json"],["/ast","stats","--format","json"],["/clew","project","inspect","--repo",".","--compilation",compilation]]
        if build=="GRADLE": commands.insert(0,[str(repo/"gradlew"),"--offline","--gradle-user-home",str(repo/".gradle"),"--no-daemon","--console=plain","-q","tasks","--all"])
        if build=="GRADLE": discovery_evidence={"method":"GRADLE_WRAPPER_TASKS","inputsSha256":digest_bytes(json.dumps(commands[0],sort_keys=True,separators=(",",":"),ensure_ascii=False).encode())}
        else:
            pom=base/"repository"/"pom.xml"; discovery_evidence={"method":"MAVEN_REACTOR_POMS","inputsSha256":digest_bytes(json.dumps([{"path":"pom.xml","sha256":digest_file(pom)}],sort_keys=True,separators=(",",":"),ensure_ascii=False).encode())}
        cache={"gradleModules":"6"*64,"gradleWrapper":"7"*64,"mavenRepository":"8"*64}
        rows.append({"taskId":task,"publicManifestSha256":digest_file(base/f"{index}.json"),"publicSourceSnapshotSha256":source_sha,"sourceBeforeSha256":source_sha,"sourceAfterSha256":source_sha,"buildSystem":build,"gitHead":"5"*40,"projectRoot":".","discoveredCompilations":[main,test],"selectedCompilation":main,"compilationDiscoveryEvidence":discovery_evidence,"sourceStable":True,"checkoutCleanBeforeAllTools":True,"checkoutCleanAfterAllTools":True,"toolCleanliness":[{"command":command,"exitCode":0,"checkoutCleanBefore":True,"checkoutCleanAfter":True} for command in commands],"stateRootOutsideCheckout":True,"externalResultsStateRootOutsideCheckout":True,"repositoryOwnedMutableState":{"insideCheckout":True,"ignoredByGit":True,"regularDirectories":True,"seedCloneSha256":cache,"currentTreeSha256":cache},"dependencySeedManifestSha256":pins["dependencySeedManifestSha256"],"typedGoalCatalogSha256":pins["catalogSha256"],"codeclewBinarySha256":pins["binarySha256"],"astIndexExecutable":ast_provenance,"astRebuildStdoutSha256":"2"*64,"astStatsStdoutSha256":"3"*64,"astReadinessSummary":ast,"astDbSha256":blob_sha,"astDbActualSizeBytes":1,"astStateAnchor":{"rootName":"state","parentIdentity":[1,2],"rootIdentity":[3,4]},"offlineHermetic":True,"astReady":True,"codeclewProjectReady":True,"projectSchemaValid":True,"projectRequestCompilation":compilation,"infrastructureValid":True,"productUnsupported":False,"astExitCode":0,"astStatsExitCode":0,"codeclewExitCode":0,"codeclewDiagnostic":"ok","wallMilliseconds":1})
    valid_report={"schema":"semantic-editing-e04-preflight/0.2","modelCalls":0,"dependencySeed":{"manifestSha256":pins["dependencySeedManifestSha256"]},"typedGoalCatalog":{"catalogSha256":pins["catalogSha256"],"binarySha256":pins["binarySha256"]},"astIndexExecutable":ast_provenance,"refusalAdapterSha256":pins["adapterSha256"],"diagnosticFreezeArtifactHash":"9"*64,"readinessRootReceiptHash":"8"*64,"tasks":42,"allInfrastructureValid":True,"allAstReady":True,"allCodeclewReady":True,"buildCounts":{"GRADLE":21,"MAVEN":21},"expectedBuildCounts":{"GRADLE":21,"MAVEN":21},"selectedTaskIds":[f"t{i:02d}" for i in range(42)],"provenanceError":None,"productUnsupported":0,"wallMilliseconds":1,"rows":rows,"status":"PREFLIGHT_PASSED","aggregatePostconditionErrors":[]}
    valid_path=base/"valid-preflight.json"; atomic_bytes(valid_path,canonical(valid_report)); check_preflight(valid_path,FakeHarness(),base,pins,"9"*64,"8"*64,Path("/clew"),base)
    class ChangingAstHarness(FakeHarness):
        calls=0; values=[ast_provenance,{"realPath":"/changed-ast","version":"changed","binarySha256":"9"*64}]
        @classmethod
        def ast_index_provenance(cls):
            value=cls.values[min(cls.calls,len(cls.values)-1)]; cls.calls+=1; return value
    check_preflight(valid_path,ChangingAstHarness(),base,pins,"9"*64,"8"*64,Path("/clew"),base)
    if ChangingAstHarness.calls!=1: raise AssertionError("check_preflight sampled changing AST provider more than once")
    count+=1
    class InvalidFirstAstHarness(FakeHarness):
        calls=0
        @classmethod
        def ast_index_provenance(cls):
            cls.calls+=1
            return {"realPath":"/ast","version":"v"} if cls.calls==1 else ast_provenance
    try: check_preflight(valid_path,InvalidFirstAstHarness(),base,pins,"9"*64,"8"*64,Path("/clew"),base); raise AssertionError("invalid first AST provenance sample accepted")
    except RuntimeError as error:
        if "AST executable provenance" not in str(error) or InvalidFirstAstHarness.calls!=1: raise
        count+=1
    for field in ("modelCalls","refusalAdapterSha256","diagnosticFreezeArtifactHash","astIndexExecutable","typedGoalCatalog","dependencySeed"):
        forged=json.loads(json.dumps(valid_report)); forged[field]=1; path=base/f"forged-{field}.json"; atomic_bytes(path,canonical(forged))
        try: check_preflight(path,FakeHarness(),base,pins,"9"*64,"8"*64,Path("/clew"),base); raise AssertionError(f"forged preflight {field} accepted")
        except RuntimeError: count+=1
    duplicate=json.loads(json.dumps(valid_report)); duplicate["rows"]=[duplicate["rows"][0] for _ in range(42)]; duplicate_path=base/"duplicate-preflight.json"; atomic_bytes(duplicate_path,canonical(duplicate))
    try: check_preflight(duplicate_path,FakeHarness(),base,pins,"9"*64,"8"*64,Path("/clew"),base); raise AssertionError("duplicated preflight rows accepted")
    except RuntimeError: count+=1
    fake=json.loads(json.dumps(valid_report)); fake["rows"][0]["taskId"]="fake"; fake_path=base/"fake-row-preflight.json"; atomic_bytes(fake_path,canonical(fake))
    try: check_preflight(fake_path,FakeHarness(),base,pins,"9"*64,"8"*64,Path("/clew"),base); raise AssertionError("fake preflight row accepted")
    except RuntimeError: count+=1
    authority_forgery=json.loads(json.dumps(valid_report)); authority_forgery["rows"][0].update({"discoveredCompilations":[],"selectedCompilation":{"compilation":":/main"},"toolCleanliness":[{"command":["totally-fake"],"exitCode":0,"checkoutCleanBefore":True,"checkoutCleanAfter":True}],"astStateAnchor":{"rootName":"forged","parentIdentity":[0,0],"rootIdentity":[0,0]}}); forged_path=base/"authority-forgery.json"; atomic_bytes(forged_path,canonical(authority_forgery))
    try: check_preflight(forged_path,FakeHarness(),base,pins,"9"*64,"8"*64,Path("/clew"),base); raise AssertionError("shallow authority forgery accepted")
    except RuntimeError: count+=1
    command_cases=((0,0),(0,1),(0,2),(0,3),(21,0),(21,1),(21,2))
    for row_index,command_index in command_cases:
        forged=json.loads(json.dumps(valid_report)); forged["rows"][row_index]["toolCleanliness"][command_index]["command"]=["totally-fake"]
        command_path=base/f"forged-command-{row_index}-{command_index}.json"; atomic_bytes(command_path,canonical(forged))
        try: check_preflight(command_path,FakeHarness(),base,pins,"9"*64,"8"*64,Path("/clew"),base); raise AssertionError("forged stage command accepted")
        except RuntimeError: count+=1
    changed_graph={**graph,"version":"changed"}
    try: Store(store.root,changed_graph,False); raise AssertionError("changed graph accepted")
    except RuntimeError: count+=1
    for name,candidate in (
        ("duplicate-dependency",{**graph,"nodes":[*graph["nodes"][:1],{**graph["nodes"][1],"dependencies":["A","A"]},*graph["nodes"][2:]]}),
        ("cycle",{**graph,"nodes":[{**graph["nodes"][0],"dependencies":["B"]},*graph["nodes"][1:]]}),
        ("bad-action",{**graph,"nodes":[{**graph["nodes"][0],"action":"RUN"},*graph["nodes"][1:]]}),
    ):
        graph_path=base/f"{name}.json"; atomic_bytes(graph_path,canonical(candidate))
        try: load_graph(graph_path); raise AssertionError(f"invalid graph {name} accepted")
        except RuntimeError: count+=1
    canonical_graph_path=Path(__file__).resolve().parents[1]/"benchmarks/semantic-change/e04-readiness-graph.json"; identical_graph_path=base/"identical-production-graph.json"; atomic_bytes(identical_graph_path,canonical_graph_path.read_bytes())
    if load_production_graph(identical_graph_path)!=load_production_graph(canonical_graph_path): raise AssertionError("byte-identical production graph copy was rejected")
    alternate_graph={"schema":"semantic-editing-e04-readiness-graph/0.1","version":"forged-terminal","nodes":[{"id":"PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT","action":"VERIFY","checker":"forged/1","dependencies":[],"inputSelectors":[]},{"id":"E04_COVERAGE_NO_GO_COMPLETE","action":"VERIFY","checker":"forged/2","dependencies":["PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT"],"inputSelectors":[]}],"roots":["E04_COVERAGE_NO_GO_COMPLETE"]}; alternate_path=base/"alternate-production-graph.json"; atomic_bytes(alternate_path,canonical(alternate_graph))
    for label,invoke in (
        ("loader",lambda:load_production_graph(alternate_path)),
        ("terminal",lambda:run_command(type("Args",(),{"readiness_action":"root","graph":str(alternate_path),"readiness_store":str(base/"forged-run-store")})(),FakeHarness())),
        ("preflight-root",lambda:require_root(FakeHarness(),alternate_path,base/"forged-root-store","E04_COVERAGE_NO_GO_COMPLETE",base,Path("/clew"),Path("/seed"))),
        ("preflight-issuer",lambda:issue_failed_preflight(FakeHarness(),alternate_path,base/"forged-preflight-store",{},"0"*64,base,Path("/clew"),Path("/seed"),Path("/semantic-corpus"))),
    ):
        try: invoke(); raise AssertionError(f"alternate production graph authorized {label}")
        except RuntimeError: count+=1
    if any((base/name).exists() for name in ("forged-run-store","forged-root-store","forged-preflight-store")): raise AssertionError("alternate production graph created a readiness store")
    changed_production_path=base/"changed-production-graph.json"; atomic_bytes(changed_production_path,canonical_graph_path.read_bytes()+b"\n")
    try: load_production_graph(changed_production_path); raise AssertionError("mutated production graph bytes were accepted")
    except RuntimeError: count+=2
    valid=store.receipt("B"); forged_checker={**valid,"checkerVersion":"other"}; forged_hash=store.object(forged_checker)
    atomic_bytes(store.root/"current/B.json",canonical({"schema":POINTER_SCHEMA,"storeId":store.store_id,"graphHash":store.graph_hash,"node":"B","receiptHash":forged_hash}))
    if assess(store,"B",inputs)[0]!="STALE": raise AssertionError("changed checker accepted")
    count+=1
    failure_graph={"schema":"semantic-editing-e04-readiness-graph/0.1","version":"failure-test","nodes":[{"id":"DIAGNOSTIC_FULL_PREFLIGHT_START_READY","action":"VERIFY","checker":"start/1","dependencies":[],"inputSelectors":[]},{"id":"DIAGNOSTIC_FULL_PREFLIGHT_42","action":"VERIFY","checker":"full/1","dependencies":["DIAGNOSTIC_FULL_PREFLIGHT_START_READY"],"inputSelectors":[]}],"roots":["DIAGNOSTIC_FULL_PREFLIGHT_START_READY"]}
    failure_store=Store(base/"failure-store",failure_graph,True); failure_inputs={"diagnosticPublicSetSha256":"1"*64}; start_hash=publish_checked(failure_store,"DIAGNOSTIC_FULL_PREFLIGHT_START_READY",failure_inputs,lambda:{"ready":True})
    for stage in ("COPY_SNAPSHOT","AST_STATS"):
        packet={"schema":"semantic-editing-e04-preflight/0.2","status":"PREFLIGHT_ROW_FAILED","stoppedAt":"task","stage":stage}
        failed_hash=publish_failed_preflight_attempt(failure_store,failure_inputs,start_hash,packet,digest_bytes(canonical(packet)))
        failed=load_json(failure_store.root/"objects"/f"{failed_hash}.json")
        if failed.get("status")!="FAILED" or failed.get("evidence",{}).get("stage")!=stage: raise AssertionError("failed preflight attempt was not retained")
        count+=1
    ready_hash=publish_checked(failure_store,"DIAGNOSTIC_FULL_PREFLIGHT_42",failure_inputs,lambda:{"accepted":True})
    packet={"schema":"semantic-editing-e04-preflight/0.2","status":"PREFLIGHT_ROW_FAILED","stoppedAt":"task","stage":"ROW_CONSTRUCTION"}
    publish_failed_preflight_attempt(failure_store,failure_inputs,start_hash,packet,digest_bytes(canonical(packet)))
    if failure_store.pointer("DIAGNOSTIC_FULL_PREFLIGHT_42")["receiptHash"]!=ready_hash or failure_store.receipt("DIAGNOSTIC_FULL_PREFLIGHT_42").get("status")!="READY": raise AssertionError("FAILED attempt replaced an existing READY pointer")
    count+=1
    audit={"schema":"semantic-editing-e04-independent-audit/0.1","decision":"ACCEPT","auditor":INDEPENDENT_AUDITOR_ID,"fullPreflightReceiptHash":ready_hash,"graphHash":failure_store.graph_hash,"storeId":failure_store.store_id}
    audit_path=(base/"independent-audit.json").absolute(); atomic_bytes(audit_path,canonical(audit)); validated,audit_sha=validate_independent_audit(audit_path,failure_store)
    if validated!=audit or audit_sha!=digest_bytes(canonical(audit)): raise AssertionError("valid independent audit rejected")
    count+=1
    for name,mutated in (
        ("extra",{**audit,"extra":True}),
        ("missing",{key:value for key,value in audit.items() if key!="decision"}),
        ("auditor",{**audit,"auditor":"self-authored"}),
    ):
        candidate=(base/f"independent-audit-{name}.json").absolute(); atomic_bytes(candidate,canonical(mutated))
        try: validate_independent_audit(candidate,failure_store); raise AssertionError(f"invalid independent audit {name} accepted")
        except RuntimeError: count+=1
    audit_link=(base/"independent-audit-link.json").absolute(); audit_link.symlink_to(audit_path)
    try: validate_independent_audit(audit_link,failure_store); raise AssertionError("symlink independent audit accepted")
    except RuntimeError: count+=1
    return {"counterexamples":count}


def run_command(args: Any, harness: Any) -> dict[str,Any]:
    action=args.readiness_action
    graph=load_production_graph(Path(args.graph)); store=Store(Path(args.readiness_store),graph,action not in {"plan","explain","root"})
    diagnostic_experiment=Path(args.diagnostic_experiment_root); r1_experiment=Path(args.r1_experiment_root) if getattr(args,"r1_experiment_root",None) else None; experiment=r1_experiment or diagnostic_experiment; binary=Path(args.codeclew_bin); seed=Path(args.dependency_seed); semantic_corpus=Path(args.semantic_corpus_bin) if getattr(args,"semantic_corpus_bin",None) else None
    audit_path=getattr(args,"diagnostic_audit_receipt",None) or (getattr(args,"audit_receipt",None) if action=="import-audit" else None)
    snapshot=ContextSnapshot(harness,binary,seed,diagnostic_experiment,r1_experiment,Path(args.diagnostic_output_root),Path(args.r1_output_root) if getattr(args,"r1_output_root",None) else None,Path(args.diagnostic_preflight_report) if getattr(args,"diagnostic_preflight_report",None) else None,Path(audit_path) if audit_path else None,store,semantic_corpus)
    annotation_id=getattr(args,"annotator_id",None)
    target=args.node if action in {"prepare","verify"} else ("DIAGNOSTIC_AUDIT_IMPORT" if action=="import-audit" else ("PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT" if action=="import-coverage-audit" else ("PRODUCT_COVERAGE_GUARD" if action=="product-coverage" else (("R1_BLIND_ANNOTATION_A_IMPORT" if annotation_id==ANNOTATOR_A_ID else "R1_BLIND_ANNOTATION_B_IMPORT") if action=="import-annotation" else (args.root if action=="root" else None)))))
    try:
        inputs=snapshot.for_node(store,target) if target else {}
    except Exception as exception:
        inputs={key:ABSENT for key in CONTEXT_KEYS}; inputs["setupFailure"]=digest_bytes(str(exception).encode())
        if args.readiness_action not in {"explain"}:
            with store.locked(): publish(store,"ARTIFACT_PROVENANCE","FAILED",inputs,{}, {},f"{type(exception).__name__}:{exception}")
        raise
    if action=="plan": return explain_snapshot(store,snapshot)
    if action=="explain": return explain_snapshot(store,snapshot)
    def provider_checked(node: str, checker: Any) -> Any:
        def run() -> Any: snapshot.raise_selected(store,node); return checker()
        return run
    if action=="prepare":
        node=args.node
        if node_spec(store,node)["action"]!="PREPARE": raise RuntimeError(f"node action is not PREPARE: {node}")
        if node not in {"DIAGNOSTIC_FREEZE_PREPARE","R1_DECISION_FREEZE_PREPARE"}: raise RuntimeError(f"node is not preparable: {node}")
        snapshot.raise_selected(store,node)
        with store.locked():
            dependencies,blockers=current_dependency_receipts(store,node,inputs)
            if blockers: receipt_hash=publish(store,node,"BLOCKED",inputs,dependencies,{"blockers":blockers},"dependency not READY")
            else:
                if node=="DIAGNOSTIC_FREEZE_PREPARE": evidence=_prepare_diagnostic_freeze(store,harness,inputs)
                else:
                    identity=capture_materializer_identity(Path(args.semantic_corpus_bin)); evidence=prepare_r1_decision(store,Path(args.r1_experiment_root).absolute(),read_secret(Path(args.agent_seed_file)),read_secret(Path(args.controller_seed_file)),read_secret(Path(args.series_nonce_file)),identity["materializerContractSha256"])
                post=ContextSnapshot(harness,binary,seed,diagnostic_experiment,r1_experiment,Path(args.diagnostic_output_root),Path(args.r1_output_root) if getattr(args,"r1_output_root",None) else None,Path(args.diagnostic_preflight_report) if getattr(args,"diagnostic_preflight_report",None) else None,Path(audit_path) if audit_path else None,store,semantic_corpus)
                post_inputs=post.for_node(store,node); post.raise_selected(store,node)
                post_dependencies,post_blockers=current_dependency_receipts(store,node,post_inputs)
                if post_blockers: receipt_hash=publish(store,node,"BLOCKED",post_inputs,post_dependencies,{"blockers":post_blockers},"dependency not READY after prepare")
                else:
                    existing=store.receipt(node); pointer=store.pointer(node)
                    if existing and pointer and existing.get("status")=="READY" and existing.get("nodeKey")==node_key(store,node,post_inputs,post_dependencies) and existing.get("evidence")==evidence: receipt_hash=pointer["receiptHash"]
                    else: receipt_hash=publish(store,node,"READY",post_inputs,post_dependencies,evidence)
        return {"status":"PUBLISHED","node":node,"receiptHash":receipt_hash}
    if action=="verify":
        node=args.node
        if node_spec(store,node)["action"]!="VERIFY": raise RuntimeError(f"node action is not VERIFY: {node}")
        if node=="ARTIFACT_PROVENANCE":
            checker=lambda:{"selectedInputsSha256":digest_bytes(canonical(selected_inputs(store,node,inputs)))}
        elif node=="DIAGNOSTIC_PUBLIC_CORPUS_42":
            checker=lambda:check_public(harness,experiment)
        elif node=="DEPENDENCY_SEED_VERIFY":
            checker=lambda:check_seed(harness,seed)
        elif node=="HARNESS_SELF_TEST":
            checker=lambda:_run_harness_self_test(Path(harness.__file__))
        elif node=="DIAGNOSTIC_FREEZE_VERIFY":
            checker=lambda:_verify_prepared(store,inputs)
        elif node=="R1_DECISION_FREEZE_VERIFY":
            checker=lambda:verify_r1_decision(store,inputs)
        elif node=="R1_CORPUS_42_VERIFY":
            checker=lambda:verify_r1_corpus(harness,Path(args.r1_experiment_root),store,inputs)
        elif node in DIRECT_NODES or node in IMPORT_NODES:
            raise RuntimeError(f"{node} is issued only by its gated execution authority")
        elif node in ROOT_NODES:
            checker=lambda:{"dependenciesReady":True}
        else: raise RuntimeError(f"node is not verifiable: {node}")
        receipt_hash=publish_checked(store,node,inputs,provider_checked(node,checker)); return {"status":"PUBLISHED","node":node,"receiptHash":receipt_hash}
    if action=="import-audit":
        audit,audit_sha=validate_independent_audit(Path(args.audit_receipt),store)
        if store.object(audit)!=audit_sha: raise RuntimeError("independent audit object digest mismatch")
        receipt_hash=publish_checked(store,"DIAGNOSTIC_AUDIT_IMPORT",inputs,lambda:{"auditSha256":audit_sha,"auditor":audit["auditor"]})
        return {"status":"PUBLISHED","node":"DIAGNOSTIC_AUDIT_IMPORT","receiptHash":receipt_hash}
    if action=="import-annotation":
        receipt_hash=import_blind_annotation(harness,Path(args.graph),Path(args.readiness_store),Path(args.annotation),args.annotator_id,diagnostic_experiment,Path(args.r1_experiment_root),binary,seed,Path(args.diagnostic_output_root),Path(args.diagnostic_preflight_report),Path(args.diagnostic_audit_receipt))
        return {"status":"PUBLISHED","node":target,"receiptHash":receipt_hash}
    if action=="import-coverage-audit":
        if semantic_corpus is None: raise RuntimeError("product coverage audit requires --semantic-corpus-bin")
        factory=lambda:ContextSnapshot(harness,binary,seed,diagnostic_experiment,r1_experiment,Path(args.diagnostic_output_root),Path(args.r1_output_root) if getattr(args,"r1_output_root",None) else None,Path(args.diagnostic_preflight_report) if getattr(args,"diagnostic_preflight_report",None) else None,Path(audit_path) if audit_path else None,store,semantic_corpus)
        receipt_hash=import_product_coverage_failure_audit(harness,store,Path(args.coverage_audit),binary,Path(harness.ROOT),semantic_corpus,factory)
        return {"status":"PUBLISHED","node":"PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT","receiptHash":receipt_hash}
    if action=="product-coverage":
        if semantic_corpus is None: raise RuntimeError("product coverage requires --semantic-corpus-bin")
        factory=lambda:ContextSnapshot(harness,binary,seed,diagnostic_experiment,r1_experiment,Path(args.diagnostic_output_root),Path(args.r1_output_root) if getattr(args,"r1_output_root",None) else None,Path(args.diagnostic_preflight_report) if getattr(args,"diagnostic_preflight_report",None) else None,Path(audit_path) if audit_path else None,store,semantic_corpus)
        receipt_hash=issue_product_coverage(harness,store,binary,Path(harness.ROOT),semantic_corpus,factory)
        receipt=store.receipt("PRODUCT_COVERAGE_GUARD")
        return {"status":receipt["status"],"node":"PRODUCT_COVERAGE_GUARD","receiptHash":receipt_hash,"error":receipt.get("error")}
    if action=="root":
        root_receipt(store,args.root,inputs); pointer=store.pointer(args.root)
        return {"status":"READY","root":args.root,"receiptHash":pointer["receiptHash"],"graphHash":store.graph_hash,"storeId":store.store_id}
    raise RuntimeError(f"unknown readiness action: {action}")


def _run_harness_self_test(runner: Path) -> dict[str,Any]:
    result=subprocess.run([os.environ.get("PYTHON","python3"),str(runner),"self-test"],text=True,capture_output=True,check=False)
    if result.returncode: raise RuntimeError(f"harness self-test failed:{result.stderr[-1000:]}")
    return {"runnerSha256":digest_file(runner),"stdoutSha256":digest_bytes(result.stdout.encode())}


def _verify_prepared(store: Store, inputs: dict[str,str]) -> dict[str,Any]:
    prepared=store.receipt("DIAGNOSTIC_FREEZE_PREPARE")
    evidence=(prepared or {}).get("evidence") or {}
    identity=inputs.get("diagnosticFreezeSha256")
    if not prepared or prepared.get("status")!="READY" or evidence.get("diagnosticFreezeArtifactHash")!=identity:
        raise RuntimeError("diagnostic freeze prepare receipt mismatch")
    artifact=store.root/"objects"/f"{evidence.get('diagnosticFreezeArtifactHash')}.json"
    if not artifact.is_file() or digest_file(artifact)!=evidence.get("diagnosticFreezeArtifactHash"): raise RuntimeError("diagnostic freeze artifact missing")
    return {"preparedReceiptHash":store.pointer("DIAGNOSTIC_FREEZE_PREPARE")["receiptHash"],"diagnosticFreezeArtifact":str(artifact),"diagnosticFreezeArtifactHash":identity}


def _prepare_diagnostic_freeze(store: Store, harness: Any, inputs: dict[str,str]) -> dict[str,Any]:
    manifest={
        "schema":"semantic-editing-e04-freeze/0.1","state":"DIAGNOSTIC_READINESS_PREPARED",
        "harnessCommit":subprocess.run(["git","rev-parse","HEAD"],cwd=harness.ROOT,text=True,capture_output=True,check=True).stdout.strip(),
        "productBaseCommit":harness.BASE,"binderTreeSha256":harness.binder_tree_sha256(),"populationSha256":harness.POP_SHA,
        "model":harness.MODEL,"reasoning":harness.EFFORT,"outputSchemaSha256":inputs["outputSchemaSha256"],
        "commonPromptSha256":inputs["commonPromptSha256"],"runnerSha256":inputs["runnerSha256"],
        "corpusFileSha256":{str(path.relative_to(harness.ROOT)):digest_file(path) for path in harness.CORPUS_FILES},
        "dependencySeedManifestSha256":inputs["dependencySeedManifestSha256"],"typedGoalCatalogSha256":inputs["catalogSha256"],
        "refusalAdapterSha256":inputs["adapterSha256"],"codeclewBinarySha256":inputs["binarySha256"],
        "diagnosticOnly":True,"finalSeedsMaterializedAtFreeze":False,"readinessGraphSha256":store.graph_hash,"readinessStoreId":store.store_id,
        "readinessInputs":inputs,
    }
    artifact_hash=store.object(manifest)
    pointer=store.root/"artifacts/diagnostic-freeze.json"
    pointer_value={"schema":"e04-readiness-artifact-pointer/0.1","sha256":artifact_hash}
    if pointer.exists():
        if pointer.is_symlink() or pointer.read_bytes()!=canonical(pointer_value): raise RuntimeError("diagnostic freeze artifact already exists with different content")
    else: atomic_bytes(pointer,canonical(pointer_value))
    return {"contextSha256":digest_bytes(canonical(inputs)),"diagnosticFreezeArtifactHash":artifact_hash,"oldFreezeUntouched":digest_file(harness.FREEZE_MANIFEST)}


def store_artifact_pointer(store:Store,name:str,value:dict[str,Any])->str:
    identity=store.object(value); pointer=store.root/f"artifacts/{name}.json"; expected={"schema":"e04-readiness-artifact-pointer/0.1","sha256":identity}
    if pointer.exists() or pointer.is_symlink():
        if pointer.is_symlink() or pointer.read_bytes()!=canonical(expected): raise RuntimeError(f"{name} artifact collision")
    else: atomic_bytes(pointer,canonical(expected))
    return identity


def derive_series_id(agent_seed:bytes,controller_seed:bytes,series_nonce:bytes)->str:
    digest=hashlib.sha256(); digest.update(b"semantic-editing-e04-r1-series/0.1\0"); digest.update(series_nonce); digest.update(b"\0"); digest.update(agent_seed); digest.update(b"\0"); digest.update(controller_seed); return digest.hexdigest()


def read_secret(path:Path)->bytes:
    if not path.is_absolute() or path.is_symlink() or not path.is_file(): raise RuntimeError("R1 secret must be an absolute regular non-symlink file")
    value=path.read_bytes()
    if value.endswith(b"\n"): value=value[:-1]
    if not value or len(value)>4096 or b"\0" in value: raise RuntimeError("R1 secret file contract mismatch")
    return value


def capture_materializer_identity(binary:Path,runner:Any=subprocess.run)->dict[str,Any]:
    if not binary.is_absolute() or binary.is_symlink() or not binary.is_file(): raise RuntimeError("semantic-corpus binary is unsafe")
    result=runner([str(binary),"e04-materializer-identity"],text=True,capture_output=True,check=False)
    if result.returncode!=0: raise RuntimeError("materializer identity command failed")
    try:value=json.loads(result.stdout)
    except json.JSONDecodeError: raise RuntimeError("materializer identity output is not JSON")
    if result.stdout.encode()!=canonical(value): raise RuntimeError("materializer identity output is not canonical")
    return value


def prepare_r1_decision(store:Store,output:Path,agent_seed:bytes,controller_seed:bytes,series_nonce:bytes,materializer_contract_sha:str)->dict[str,Any]:
    if not all(value and len(value)<=4096 for value in (agent_seed,controller_seed,series_nonce)): raise RuntimeError("R1 seed contract mismatch")
    if not output.is_absolute() or output.exists() or output.is_symlink() or not output.parent.is_dir(): raise RuntimeError("R1 output must be an absent absolute path under an existing parent")
    canary_pointer=store.pointer("DIAGNOSTIC_CANARY_3_COMPLETE"); canary=store.receipt("DIAGNOSTIC_CANARY_3_COMPLETE")
    packet_sha=((canary or {}).get("selectedInputs") or {}).get("diagnosticCanaryPacketSetSha256")
    if not canary_pointer or not canary or canary.get("status")!="READY" or not isinstance(packet_sha,str): raise RuntimeError("R1 decision lacks READY diagnostic canary")
    series=derive_series_id(agent_seed,controller_seed,series_nonce)
    nested={"outputPath":str(output),"agentSeedSha256":digest_bytes(agent_seed),"controllerSeedSha256":digest_bytes(controller_seed),"seriesNonceSha256":digest_bytes(series_nonce),"seriesId":series,"materializerContractSha256":materializer_contract_sha}
    decision={"schema":"semantic-editing-e04-r1-decision-freeze/0.1","decision":"PROCEED","storeId":store.store_id,"graphHash":store.graph_hash,"diagnosticCanaryReceiptSha256":canary_pointer["receiptHash"],"diagnosticCanaryPacketSetSha256":packet_sha,"seriesId":series,"r1Materialization":nested}
    identity=store_artifact_pointer(store,"r1-decision",decision)
    return {"decisionSha256":identity,"seriesId":series,"outputPath":str(output)}


def verify_r1_decision(store:Store,inputs:dict[str,str])->dict[str,Any]:
    identity=inputs.get("r1DecisionSha256"); path=store.root/f"objects/{identity}.json"; value=load_json(path)
    top={"schema","decision","storeId","graphHash","diagnosticCanaryReceiptSha256","diagnosticCanaryPacketSetSha256","seriesId","r1Materialization"}; nested={"outputPath","agentSeedSha256","controllerSeedSha256","seriesNonceSha256","seriesId","materializerContractSha256"}
    if not isinstance(value,dict) or set(value)!=top or value.get("schema")!="semantic-editing-e04-r1-decision-freeze/0.1" or value.get("decision")!="PROCEED" or value.get("storeId")!=store.store_id or value.get("graphHash")!=store.graph_hash or not isinstance(value.get("r1Materialization"),dict) or set(value["r1Materialization"])!=nested or value["r1Materialization"].get("seriesId")!=value.get("seriesId"): raise RuntimeError("R1 decision exact contract mismatch")
    canary=store.pointer("DIAGNOSTIC_CANARY_3_COMPLETE"); receipt=store.receipt("DIAGNOSTIC_CANARY_3_COMPLETE")
    if not canary or not receipt or receipt.get("status")!="READY" or value.get("diagnosticCanaryReceiptSha256")!=canary["receiptHash"] or value.get("diagnosticCanaryPacketSetSha256")!=((receipt.get("selectedInputs") or {}).get("diagnosticCanaryPacketSetSha256")): raise RuntimeError("R1 decision canary replay mismatch")
    for key in ("agentSeedSha256","controllerSeedSha256","seriesNonceSha256","seriesId","materializerContractSha256"):
        item=value["r1Materialization"].get(key)
        if not isinstance(item,str) or len(item)!=64 or any(char not in "0123456789abcdef" for char in item): raise RuntimeError("R1 decision digest contract mismatch")
    output=Path(value["r1Materialization"].get("outputPath",""))
    if not output.is_absolute() or output.exists() or output.is_symlink(): raise RuntimeError("R1 decision output is no longer absent")
    return {"decisionSha256":identity,"seriesId":value["seriesId"],"outputPath":str(output)}


def execute_r1_materialization(harness:Any,store:Store,semantic_corpus:Path,diagnostic_root:Path,r1_root:Path,binary:Path,dependency_seed:Path,diagnostic_output:Path,diagnostic_preflight:Path,diagnostic_audit:Path,agent_seed:bytes,controller_seed:bytes,series_nonce:bytes,materializer_identity:dict[str,Any],signer:Any,runner:Any=subprocess.run,tooling:dict[str,str]|None=None)->str:
    if not semantic_corpus.is_absolute() or semantic_corpus.is_symlink() or not semantic_corpus.is_file(): raise RuntimeError("semantic-corpus executable is unsafe")
    decision_identity=ContextSnapshot(harness,binary,dependency_seed,diagnostic_root,r1_root,diagnostic_output,r1_root,diagnostic_preflight,diagnostic_audit,store).get("r1DecisionSha256"); decision=load_json(store.root/f"objects/{decision_identity}.json")
    root_pointer=store.pointer("R1_MATERIALIZE_START_READY"); root_value=store.receipt("R1_MATERIALIZE_START_READY")
    if not root_pointer or not root_value or root_value.get("status")!="READY": raise RuntimeError("R1 materialization start root is not READY")
    if not isinstance(materializer_identity,dict) or set(materializer_identity)!={"schema","materializer","materializerContractSha256","readinessGraphSha256","readinessCheckerSourceSha256","issuer","purpose","authorizationEnvelopeSchema","authorizationPayloadSchema","materializationResultSchema"} or materializer_identity.get("schema")!="semantic-editing-e04-r1-materializer-identity/0.1" or materializer_identity.get("purpose")!="codeclew/e04/r1-materialization/0.1" or materializer_identity.get("issuer")!="codeclew-e04-production-2026-08": raise RuntimeError("materializer identity contract mismatch")
    nested=decision["r1Materialization"]
    if materializer_identity.get("materializerContractSha256")!=nested["materializerContractSha256"]: raise RuntimeError("materializer contract changed after decision")
    payload={"schema":"semantic-editing-e04-r1-materialization-authorization/0.1","storeId":store.store_id,"graphHash":store.graph_hash,"rootNode":"R1_MATERIALIZE_START_READY","rootReceiptSha256":root_pointer["receiptHash"],"decisionFreezeSha256":decision_identity,"outputPath":str(r1_root),"agentSeedSha256":digest_bytes(agent_seed),"controllerSeedSha256":digest_bytes(controller_seed),"seriesNonceSha256":digest_bytes(series_nonce),"seriesId":derive_series_id(agent_seed,controller_seed,series_nonce),"materializer":materializer_identity["materializer"]}
    if any(payload[key]!=nested[key] for key in ("outputPath","agentSeedSha256","controllerSeedSha256","seriesNonceSha256","seriesId")): raise RuntimeError("R1 secret/output binding changed after decision")
    purpose="codeclew/e04/r1-materialization/0.1"; signature=signer(purpose.encode()+b"\0"+canonical(payload))
    if not isinstance(signature,str) or len(signature)!=128 or any(char not in "0123456789abcdef" for char in signature): raise RuntimeError("fixed-key signer returned a malformed signature")
    envelope={"schema":"semantic-editing-e04-r1-materialization-authorization-envelope/0.1","issuer":"codeclew-e04-production-2026-08","purpose":purpose,"payload":payload,"signature":signature}; authorization_sha=digest_bytes(canonical(envelope)); authorization=store.root/f"authorizations/{authorization_sha}.json"
    if authorization.exists() or authorization.is_symlink(): raise RuntimeError("materialization authorization path already exists")
    atomic_bytes(authorization,canonical(envelope)); authorization.chmod(0o444)
    command=[str(semantic_corpus),"materialize-e04","--experiment-root",str(r1_root),"--readiness-store",str(store.root),"--authorization",str(authorization),"--root-receipt",str(store.root/f"objects/{root_pointer['receiptHash']}.json"),"--agent-seed",agent_seed.decode(),"--controller-seed",controller_seed.decode(),"--series-nonce",series_nonce.decode(),"--binder-tree-sha256",harness.binder_tree_sha256()]
    if tooling is not None:
        exact=("tooling-root","gradle-wrapper-script","gradle-wrapper-jar","gradle-wrapper-properties","tooling-manifest","codeclew-binary-sha256","typed-goal-catalog-sha256")
        if set(tooling)!=set(exact): raise RuntimeError("materializer tooling argv contract mismatch")
        for name in exact: command.extend([f"--{name}",tooling[name]])
    completed=runner(command,text=True,capture_output=True,check=False)
    if completed.returncode!=0: raise RuntimeError("R1 materializer exited nonzero")
    try: result=json.loads(completed.stdout)
    except json.JSONDecodeError: raise RuntimeError("R1 materializer stdout is not JSON")
    if completed.stdout.encode()!=canonical(result): raise RuntimeError("R1 materializer stdout is not exact canonical result")
    live=validate_materialization_result(harness,result,r1_root,decision_identity,root_pointer["receiptHash"],authorization_sha)
    snapshot=ContextSnapshot(harness,binary,dependency_seed,diagnostic_root,r1_root,diagnostic_output,r1_root,diagnostic_preflight,diagnostic_audit,store); inputs=snapshot.for_node(store,"R1_CORPUS_42_MATERIALIZED")
    def captured()->dict[str,Any]:
        snapshot.raise_selected(store,"R1_CORPUS_42_MATERIALIZED")
        return {"command":command,"exitCode":0,"stdoutSha256":digest_bytes(completed.stdout.encode()),"authorizationEnvelopeSha256":authorization_sha,"materializationResult":result,"agentPublicSetSha256":live["agentPublicSetSha256"],"controllerTreeSha256":live["controllerTreeSha256"]}
    receipt_hash=publish_checked(store,"R1_CORPUS_42_MATERIALIZED",inputs,captured)
    if store.receipt("R1_CORPUS_42_MATERIALIZED").get("status")!="READY": raise RuntimeError("R1 materialization direct receipt refused")
    return receipt_hash


def verify_r1_corpus(harness:Any,root:Path,store:Store,inputs:dict[str,str])->dict[str,Any]:
    materialized=store.receipt("R1_CORPUS_42_MATERIALIZED"); evidence=(materialized or {}).get("evidence") or {}; result=evidence.get("materializationResult")
    live=validate_materialization_result(harness,result,root,result.get("decisionFreezeSha256") if isinstance(result,dict) else "",result.get("rootReceiptSha256") if isinstance(result,dict) else "",result.get("authorizationEnvelopeSha256") if isinstance(result,dict) else "")
    if inputs.get("r1PublicSetSha256")!=ContextSnapshot(harness,Path("/unused"),Path("/unused"),root,root)._public(root,"R1") or inputs.get("r1ControllerTreeSha256")!=live["controllerTreeSha256"]: raise RuntimeError("R1 corpus live selector mismatch")
    return {"materializedReceiptSha256":store.pointer("R1_CORPUS_42_MATERIALIZED")["receiptHash"],"seriesId":live["seriesId"],"tasks":42}


def validate_blind_annotation(harness:Any,path:Path,annotator_id:str,root:Path,store:Store)->tuple[dict[str,Any],str]:
    if annotator_id not in {ANNOTATOR_A_ID,ANNOTATOR_B_ID} or path.is_symlink() or not path.is_file(): raise RuntimeError("blind annotation path/role contract mismatch")
    raw=path.read_bytes()
    try:value=json.loads(raw)
    except (UnicodeDecodeError,json.JSONDecodeError): raise RuntimeError("blind annotation is not JSON")
    keys={"schema","annotatorId","seriesId","r1PublicSetSha256","tasks"}; label_keys={"taskId","family","outcome","requiredObligations","requiredBindings","ambiguousChoices","refusalCode","oracleClass","evidence"}
    if not isinstance(value,dict) or set(value)!=keys or raw!=canonical(value) or value.get("schema")!="semantic-editing-e04-r1-blind-annotation/0.1" or value.get("annotatorId")!=annotator_id or not isinstance(value.get("tasks"),list) or len(value["tasks"])!=42: raise RuntimeError("blind annotation exact contract mismatch")
    materialized=store.pointer("R1_CORPUS_42_MATERIALIZED"); live=inspect_r1_materialized(harness,root,value.get("seriesId")); public_digest=ContextSnapshot(harness,Path("/unused"),Path("/unused"),root,root)._public(root,"R1")
    if not materialized or value.get("r1PublicSetSha256")!=public_digest: raise RuntimeError("blind annotation materialized/public binding mismatch")
    task_ids=[]
    for label in value["tasks"]:
        if not isinstance(label,dict) or set(label)!=label_keys or not isinstance(label.get("taskId"),str) or not isinstance(label.get("family"),str) or not isinstance(label.get("outcome"),str) or not isinstance(label.get("requiredObligations"),list) or not isinstance(label.get("requiredBindings"),list) or not isinstance(label.get("ambiguousChoices"),list) or not isinstance(label.get("evidence"),dict): raise RuntimeError("blind annotation label contract mismatch")
        family=label["family"]; outcome=label["outcome"]; contract=harness.FAMILY_CONTRACTS.get(family)
        if contract is None or label["requiredObligations"]!=contract["obligations"] or outcome not in {"BOUND","AMBIGUOUS","REFUSED"}: raise RuntimeError("blind annotation family/outcome mismatch")
        roles=set(contract["roles"])
        def binding_set(items:Any)->tuple[tuple[str,str],...]:
            if not isinstance(items,list) or any(not isinstance(item,dict) or set(item)!={"role","symbol"} or not isinstance(item["role"],str) or not isinstance(item["symbol"],str) or not item["symbol"] for item in items): raise RuntimeError("blind annotation binding contract mismatch")
            pairs=tuple((item["role"],item["symbol"]) for item in items)
            if tuple(sorted(pairs))!=pairs or len(set(pairs))!=len(pairs) or {role for role,_ in pairs}!=roles: raise RuntimeError("blind annotation bindings are not complete/canonical")
            return pairs
        required=binding_set(label["requiredBindings"])
        choices=[binding_set(choice) for choice in label["ambiguousChoices"]]
        if choices!=sorted(choices) or len(set(choices))!=len(choices): raise RuntimeError("blind annotation ambiguity choices are not canonical/distinct")
        if (outcome=="BOUND" and (choices or label["refusalCode"] is not None or not isinstance(label["oracleClass"],str))) or (outcome=="AMBIGUOUS" and (len(choices)<2 or label["refusalCode"] is not None or label["oracleClass"] is not None)) or (outcome=="REFUSED" and (choices or label["refusalCode"] not in harness.REFUSALS or label["oracleClass"] is not None)): raise RuntimeError("blind annotation outcome semantics mismatch")
        evidence=label["evidence"]; evidence_keys={"publicManifestSha256","repositorySourceSha256","anchors"}
        task_root=root/"agent"/label["taskId"]; manifest=task_root/"task-manifest.json"; repository=task_root/"repository"
        if set(evidence)!=evidence_keys or evidence.get("publicManifestSha256")!=digest_file(manifest) or evidence.get("repositorySourceSha256")!=harness.source_digest(repository) or not isinstance(evidence.get("anchors"),list): raise RuntimeError("blind annotation public evidence mismatch")
        symbols=sorted({symbol for _,symbol in required}|{symbol for choice in choices for _,symbol in choice}); anchors=evidence["anchors"]
        if len(anchors)!=len(symbols): raise RuntimeError("blind annotation anchor denominator mismatch")
        for anchor,symbol in zip(anchors,symbols):
            if not isinstance(anchor,dict) or set(anchor)!={"symbol","relativePath","fileSha256"} or anchor.get("symbol")!=symbol or not isinstance(anchor.get("relativePath"),str): raise RuntimeError("blind annotation anchor contract mismatch")
            relative=Path(anchor["relativePath"])
            if relative.is_absolute() or ".." in relative.parts: raise RuntimeError("blind annotation anchor escapes repository")
            file=repository/relative
            if file.is_symlink() or not file.is_file() or anchor.get("fileSha256")!=digest_file(file) or symbol.rsplit(".",1)[-1] not in file.read_text(encoding="utf-8",errors="ignore"): raise RuntimeError("blind annotation anchor evidence mismatch")
        task_ids.append(label["taskId"])
    if task_ids!=sorted(live["taskIds"]) or len(set(task_ids))!=42: raise RuntimeError("blind annotation task denominator/order mismatch")
    return value,digest_bytes(raw)


def import_blind_annotation(harness:Any,graph_path:Path,store_path:Path,path:Path,annotator_id:str,diagnostic_root:Path,r1_root:Path,binary:Path,seed:Path,diagnostic_output:Path,diagnostic_preflight:Path,diagnostic_audit:Path)->str:
    graph=load_production_graph(graph_path); store=Store(store_path,graph,False); node="R1_BLIND_ANNOTATION_A_IMPORT" if annotator_id==ANNOTATOR_A_ID else "R1_BLIND_ANNOTATION_B_IMPORT"; name="r1-annotation-a" if annotator_id==ANNOTATOR_A_ID else "r1-annotation-b"
    value,identity=validate_blind_annotation(harness,path,annotator_id,r1_root,store)
    other_name="r1-annotation-b" if name.endswith("a") else "r1-annotation-a"; other=ContextSnapshot(harness,binary,seed,diagnostic_root,r1_root,diagnostic_output,r1_root,diagnostic_preflight,diagnostic_audit,store)._store_artifact(other_name)
    if other!=ABSENT:
        other_value=load_json(store.root/f"objects/{other}.json")
        if other_value.get("annotatorId")==annotator_id or other==identity or other_value.get("tasks")!=value.get("tasks"): raise RuntimeError("blind annotations are not independent or disagree")
    if store.object(value)!=identity: raise RuntimeError("blind annotation object digest mismatch")
    store_artifact_pointer(store,name,value)
    snapshot=ContextSnapshot(harness,binary,seed,diagnostic_root,r1_root,diagnostic_output,r1_root,diagnostic_preflight,diagnostic_audit,store); inputs=snapshot.for_node(store,node)
    receipt=publish_checked(store,node,inputs,lambda:{"annotationSha256":identity,"annotatorId":annotator_id,"materializedReceiptSha256":store.pointer("R1_CORPUS_42_MATERIALIZED")["receiptHash"],"seriesId":value["seriesId"]})
    if store.receipt(node).get("status")!="READY": raise RuntimeError("blind annotation import refused")
    return receipt


def execute_r1_hidden_verification(harness:Any,store:Store,semantic_corpus:Path,diagnostic_root:Path,r1_root:Path,binary:Path,dependency_seed:Path,diagnostic_output:Path,diagnostic_preflight:Path,diagnostic_audit:Path,report_path:Path,signer:Any,runner:Any=subprocess.run)->str:
    if report_path.exists() or report_path.is_symlink() or not report_path.is_absolute() or not report_path.parent.is_dir(): raise RuntimeError("hidden report must be an absent absolute path")
    root_pointer=store.pointer("R1_HIDDEN_VERIFY_START_READY"); root_receipt_value=store.receipt("R1_HIDDEN_VERIFY_START_READY")
    if not root_pointer or not root_receipt_value or root_receipt_value.get("status")!="READY": raise RuntimeError("hidden verification start root is not READY")
    snapshot=ContextSnapshot(harness,binary,dependency_seed,diagnostic_root,r1_root,diagnostic_output,r1_root,diagnostic_preflight,diagnostic_audit,store)
    annotation_a=snapshot.get("r1AnnotationASha256"); annotation_b=snapshot.get("r1AnnotationBSha256"); receipt_a=store.pointer("R1_BLIND_ANNOTATION_A_IMPORT"); receipt_b=store.pointer("R1_BLIND_ANNOTATION_B_IMPORT")
    if ABSENT in {annotation_a,annotation_b} or not receipt_a or not receipt_b: raise RuntimeError("hidden verification lacks two annotation imports")
    path_a=store.root/f"objects/{annotation_a}.json"; path_b=store.root/f"objects/{annotation_b}.json"; live=inspect_r1_materialized(harness,r1_root)
    public_selector=snapshot._public(r1_root,"R1"); controller_selector=live["controllerTreeSha256"]
    payload={"schema":"semantic-editing-e04-r1-hidden-verification-authorization/0.1","storeId":store.store_id,"graphHash":store.graph_hash,"readinessCheckerSourceSha256":digest_file(Path(__file__)),"rootNode":"R1_HIDDEN_VERIFY_START_READY","rootReceiptSha256":root_pointer["receiptHash"],"experimentPath":str(r1_root.resolve()),"reportPath":str(report_path),"seriesId":live["seriesId"],"agentPublicMembers":live["agentPublicMembers"],"agentPublicSetSha256":live["agentPublicSetSha256"],"controllerMembers":live["controllerMembers"],"controllerSetSha256":live["controllerSetSha256"],"r1PublicSetSha256":public_selector,"r1ControllerTreeSha256":controller_selector,"annotationASha256":annotation_a,"annotationBSha256":annotation_b,"annotationAReceiptSha256":receipt_a["receiptHash"],"annotationBReceiptSha256":receipt_b["receiptHash"],"annotationAPath":str(path_a.resolve()),"annotationBPath":str(path_b.resolve())}
    purpose="codeclew/e04/r1-hidden-verify/0.1"; signature=signer(purpose.encode()+b"\0"+canonical(payload))
    if not isinstance(signature,str) or len(signature)!=128 or any(char not in "0123456789abcdef" for char in signature): raise RuntimeError("hidden fixed-key signer returned malformed signature")
    envelope={"schema":"semantic-editing-e04-r1-hidden-verification-authorization-envelope/0.1","issuer":"codeclew-e04-production-2026-08","purpose":purpose,"payload":payload,"signature":signature}; authorization_sha=digest_bytes(canonical(envelope)); authorization=store.root/f"authorizations/{authorization_sha}.json"
    if authorization.exists() or authorization.is_symlink(): raise RuntimeError("hidden authorization path already exists")
    atomic_bytes(authorization,canonical(envelope)); authorization.chmod(0o444)
    command=[str(semantic_corpus),"verify-e04-hidden","--experiment-root",str(r1_root),"--readiness-store",str(store.root),"--authorization",str(authorization),"--root-receipt",str(store.root/f"objects/{root_pointer['receiptHash']}.json"),"--report",str(report_path),"--annotation-a",str(path_a),"--annotation-b",str(path_b)]
    completed=runner(command,text=True,capture_output=True,check=False)
    if completed.returncode!=0 or not report_path.is_file() or report_path.is_symlink(): raise RuntimeError("hidden verifier failed or omitted report")
    report=load_json(report_path)
    if completed.stdout.encode()!=canonical(report) or report_path.read_bytes()!=canonical(report): raise RuntimeError("hidden stdout/report mismatch")
    keys={"schema","authorizationEnvelopeSha256","rootReceiptSha256","seriesId","experimentPath","reportPath","taskCount","agentPublicMembers","agentPublicSetSha256","controllerMembers","controllerSetSha256","verifiedTaskIds","r1PublicSetSha256","r1ControllerTreeSha256","annotationASha256","annotationBSha256","annotationAReceiptSha256","annotationBReceiptSha256","verdicts"}
    verdict_keys={"taskId","family","outcome","requiredObligations","bindingCount","ambiguousChoiceCount","refusalCode","oracleClass","decisionSha256","evidenceSha256","status"}; verdicts=report.get("verdicts") if isinstance(report,dict) else None
    if not isinstance(report,dict) or set(report)!=keys or report.get("schema")!="semantic-editing-e04-r1-hidden-verification-report/0.1" or report.get("authorizationEnvelopeSha256")!=authorization_sha or report.get("rootReceiptSha256")!=root_pointer["receiptHash"] or report.get("seriesId")!=live["seriesId"] or report.get("experimentPath")!=str(r1_root.resolve()) or report.get("reportPath")!=str(report_path) or report.get("taskCount")!=42 or report.get("verifiedTaskIds")!=live["taskIds"] or report.get("r1PublicSetSha256")!=public_selector or report.get("r1ControllerTreeSha256")!=controller_selector or report.get("annotationASha256")!=annotation_a or report.get("annotationBSha256")!=annotation_b or report.get("annotationAReceiptSha256")!=receipt_a["receiptHash"] or report.get("annotationBReceiptSha256")!=receipt_b["receiptHash"] or not isinstance(verdicts,list) or len(verdicts)!=42 or [item.get("taskId") for item in verdicts if isinstance(item,dict)]!=live["taskIds"] or any(not isinstance(item,dict) or set(item)!=verdict_keys or item.get("status")!="VERIFIED" for item in verdicts): raise RuntimeError("hidden verification report contract mismatch")
    report_sha=store_artifact_pointer(store,"r1-hidden-verify",report); post=ContextSnapshot(harness,binary,dependency_seed,diagnostic_root,r1_root,diagnostic_output,r1_root,diagnostic_preflight,diagnostic_audit,store); inputs=post.for_node(store,"R1_HIDDEN_VERIFY_COMPLETE")
    receipt_hash=publish_checked(store,"R1_HIDDEN_VERIFY_COMPLETE",inputs,lambda:{"reportSha256":report_sha,"reportPath":str(report_path),"authorizationEnvelopeSha256":authorization_sha,"exitCode":0,"stdoutSha256":digest_bytes(completed.stdout.encode())})
    if store.receipt("R1_HIDDEN_VERIFY_COMPLETE").get("status")!="READY": raise RuntimeError("hidden direct receipt refused")
    return receipt_hash


def issue_r1_coverage_guard(harness:Any,store:Store,packets_path:Path,hidden_report_path:Path,product_root:Path,diagnostic_root:Path,r1_root:Path,binary:Path,dependency_seed:Path,diagnostic_output:Path,diagnostic_preflight:Path,diagnostic_audit:Path)->str:
    root=store.pointer("R1_COVERAGE_START_READY")
    if not root or store.receipt("R1_COVERAGE_START_READY").get("status")!="READY": raise RuntimeError("coverage start root is not READY")
    for path,label in ((packets_path,"coverage packets"),(hidden_report_path,"hidden report")):
        if path.is_symlink() or not path.is_file(): raise RuntimeError(f"{label} path is unsafe")
    packets=[json.loads(line) for line in packets_path.read_text(encoding="utf-8").splitlines()]
    if len(packets)!=42: raise RuntimeError("coverage requires exact 42 real packets")
    hidden=load_json(hidden_report_path); hidden_pointer=store.pointer("R1_HIDDEN_VERIFY_COMPLETE")
    if not hidden_pointer or ((store.receipt("R1_HIDDEN_VERIFY_COMPLETE").get("evidence") or {}).get("reportSha256")!=digest_bytes(canonical(hidden))): raise RuntimeError("coverage hidden report/receipt binding mismatch")
    guard=harness.guard_zero_model_coverage(packets,r1_root,binary,product_root)
    report={"schema":"semantic-editing-e04-r1-coverage/0.1","seriesId":hidden.get("seriesId"),"hiddenReceiptSha256":hidden_pointer["receiptHash"],"hiddenReportSha256":digest_bytes(canonical(hidden)),"r1PublicSetSha256":hidden.get("r1PublicSetSha256"),"r1ControllerTreeSha256":hidden.get("r1ControllerTreeSha256"),"packetsSha256":digest_bytes(canonical(sorted(packets,key=lambda item:item["taskId"]))),"productProvenance":{key:value for key,value in guard.items() if key.endswith("Sha256")},"result":guard}
    identity=store_artifact_pointer(store,"r1-coverage",report); snapshot=ContextSnapshot(harness,binary,dependency_seed,diagnostic_root,r1_root,diagnostic_output,r1_root,diagnostic_preflight,diagnostic_audit,store); inputs=snapshot.for_node(store,"R1_COVERAGE_GUARD_COMPLETE")
    receipt=publish_checked(store,"R1_COVERAGE_GUARD_COMPLETE",inputs,lambda:{"coverageSha256":identity,"coverageStatus":guard.get("status"),"positiveCells":guard.get("positiveCells"),"denominator":guard.get("denominator")})
    if store.receipt("R1_COVERAGE_GUARD_COMPLETE").get("status")!="READY": raise RuntimeError("coverage direct receipt refused")
    return receipt


PRODUCT_COVERAGE_REQUIRED={"compositionIds":5,"positiveCells":9,"denominator":14,"exactAmbiguity":True,"mustRefuse":14,"falseBound":0,"builds":["GRADLE","MAVEN"]}
PRODUCT_COVERAGE_CONTRACT_SHA256="2b5092965614ead650f2a892e703daf4f6ef024d2b34d3c6f4321a2af81883f1"
PRODUCT_COVERAGE_POPULATION_SHA256="a209f115b0a175bb74859b0539f75932cd664a495332ccf10b634b3cf1c2b9f2"


def derive_product_coverage(contract:Any,population:Any,catalog:Any)->dict[str,Any]:
    if not isinstance(contract,dict) or set(contract)!={"schema","populationSchema","populationSha256","typedGoalSchema","typedGoalVersion","positiveCellCount","currentSupportedUpperBound","cells"} or contract.get("schema")!="semantic-editing-e04-product-coverage-contract/0.1" or not isinstance(population,dict) or contract.get("populationSchema")!=population.get("schema") or contract.get("typedGoalSchema")!=catalog.get("schema") or contract.get("typedGoalVersion")!=catalog.get("version"): raise RuntimeError("product coverage frozen identity mismatch")
    operators={item["operator"]:item for item in catalog.get("operators",[]) if isinstance(item,dict) and isinstance(item.get("operator"),str)}; executable=set(catalog.get("executableDomains",[])); cells=contract.get("cells")
    expected=[(family["id"],build.lower(),family["requiredObligations"]) for family in population.get("families",[]) for build in family.get("buildSystems",[])]
    if not isinstance(cells,list) or len(cells)!=14 or contract.get("positiveCellCount")!=14 or len(expected)!=14: raise RuntimeError("product coverage frozen denominator mismatch")
    supported=0
    for cell,(family,build,obligations) in zip(cells,expected):
        keys={"family","buildSystem","requiredRoles","requiredObligations","requiredRoot","expectedProviderBindingCardinality","status","unsupportedReason"}
        if not isinstance(cell,dict) or set(cell)!=keys or cell.get("family")!=family or cell.get("buildSystem")!=build or cell.get("requiredObligations")!=obligations or not isinstance(cell.get("requiredRoles"),list) or len(cell["requiredRoles"])!=3 or len(set(cell["requiredRoles"]))!=3: raise RuntimeError("product coverage cell differs from frozen population")
        root=cell.get("requiredRoot"); executable_cell=False; actual_arity=0
        if root is not None:
            if not isinstance(root,dict) or set(root)!={"operator","operandRoles","compositionSource"} or root.get("compositionSource")!="TYPED_GOAL_MANDATORY_CLOSURE" or not isinstance(root.get("operandRoles"),list) or len(set(root["operandRoles"]))!=len(root["operandRoles"]) or not set(root["operandRoles"])<=set(cell["requiredRoles"]): raise RuntimeError("product coverage root contract mismatch")
            operator=operators.get(root["operator"]); actual_arity=operator.get("arity",0) if operator else 0
            executable_cell=bool(operator and operator.get("auxiliaryOnly") is False and operator.get("constraintDomain") in executable and actual_arity==len(root["operandRoles"])==len(cell["requiredRoles"]) and set(root["operandRoles"])==set(cell["requiredRoles"]))
        if cell.get("expectedProviderBindingCardinality")!=actual_arity: raise RuntimeError("product coverage provider cardinality drift")
        if executable_cell:
            supported+=1
            if cell.get("status")!="SUPPORTED" or cell.get("unsupportedReason") is not None: raise RuntimeError("supported product coverage cell mislabeled")
        elif cell.get("status")=="SUPPORTED" or not isinstance(cell.get("unsupportedReason"),str) or not cell["unsupportedReason"]: raise RuntimeError("unsupported product coverage cell lacks reason")
    if supported!=contract.get("currentSupportedUpperBound"): raise RuntimeError("product coverage upper bound does not recompute")
    return {"upperBound":supported,"cells":cells}


def validate_product_coverage_report(report:Any,store:Store,diagnostic_freeze_sha:str)->dict[str,Any]:
    keys={"schema","status","decision","graphHash","storeId","diagnosticFreezeSha256","productRevision","productBinarySha256","semanticCorpusBinarySha256","semanticCorpusBinaryRealPath","catalogSha256","runnerSha256","populationSha256","productPathDigestsSha256","packetSetSha256","required","observed","upperBoundPositiveCells","cellResults"}
    if not isinstance(report,dict) or set(report)!=keys or report.get("schema")!="semantic-editing-e04-product-coverage/0.1" or report.get("graphHash")!=store.graph_hash or report.get("storeId")!=store.store_id or report.get("diagnosticFreezeSha256")!=diagnostic_freeze_sha or report.get("required")!=PRODUCT_COVERAGE_REQUIRED: raise RuntimeError("product coverage report exact contract mismatch")
    observed=report.get("observed"); cells=report.get("cellResults")
    if not isinstance(observed,dict) or set(observed)!=set(PRODUCT_COVERAGE_REQUIRED) or not isinstance(cells,list) or len(cells)!=14: raise RuntimeError("product coverage denominator contract mismatch")
    upper=report.get("upperBoundPositiveCells")
    if not isinstance(upper,int) or isinstance(upper,bool) or observed.get("positiveCells",0)>upper or observed.get("denominator")!=14: raise RuntimeError("product coverage observed/upper-bound mismatch")
    # This pre-R1 gate is a conservative feasibility ceiling.  The remaining
    # preregistered metrics are measured only after the product can possibly
    # cover nine cells; an upper bound below nine is already a decisive NO-GO.
    passes=upper>=PRODUCT_COVERAGE_REQUIRED["positiveCells"]
    expected_status=("COVERAGE_ACCEPTED","GO") if passes else ("COVERAGE_REJECTED","NO_GO")
    if (report.get("status"),report.get("decision"))!=expected_status: raise RuntimeError("product coverage decision does not follow preregistered threshold")
    for field in ("productBinarySha256","semanticCorpusBinarySha256","catalogSha256","runnerSha256","populationSha256","productPathDigestsSha256","packetSetSha256"):
        value=report.get(field)
        if not isinstance(value,str) or len(value)!=64: raise RuntimeError("product coverage provenance digest malformed")
    if not isinstance(report.get("semanticCorpusBinaryRealPath"),str) or not Path(report["semanticCorpusBinaryRealPath"]).is_absolute(): raise RuntimeError("product coverage authority path malformed")
    return {"passes":passes,"upperBoundPositiveCells":upper,"observed":observed}


def raw_typed_goal_catalog(catalog:dict[str,Any])->dict[str,Any]:
    return {key:value for key,value in catalog.items() if key not in {"derivedCapabilities","catalogSha256","binarySha256","refusalMapping","adapterSha256"}}


def invoke_product_coverage_authority(harness:Any,semantic_corpus:Path,binary:Path,store:Store,runner:Any=subprocess.run)->dict[str,Any]:
    if not semantic_corpus.is_absolute() or semantic_corpus.is_symlink() or not semantic_corpus.is_file(): raise RuntimeError("semantic-corpus coverage authority is unsafe")
    catalog=harness.load_typed_goal_catalog(binary.resolve()); raw_catalog=raw_typed_goal_catalog(catalog); catalog_sha=digest_bytes(canonical(raw_catalog)); catalog_object=store.object(raw_catalog); catalog_path=store.root/f"objects/{catalog_object}.json"
    completed=runner([str(semantic_corpus),"e04-product-coverage","--typed-goal-catalog",str(catalog_path)],text=True,capture_output=True,check=False)
    if completed.returncode!=0: raise RuntimeError("semantic-corpus product coverage authority exited nonzero")
    try:value=json.loads(completed.stdout)
    except json.JSONDecodeError: raise RuntimeError("semantic-corpus product coverage authority output is not JSON")
    keys={"schema","contractSha256","populationSha256","catalogSha256","positiveCells","supportedUpperBound","cellResults"}
    contract_path=Path(harness.ROOT)/"benchmarks/semantic-change/e04-product-coverage-v1.json"; contract=load_json(contract_path); python_summary=derive_product_coverage(contract,harness.population(),raw_catalog)
    if completed.stdout.encode()!=canonical(value) or not isinstance(value,dict) or set(value)!=keys or value.get("schema")!="semantic-editing-e04-product-coverage/0.1" or value.get("contractSha256")!=PRODUCT_COVERAGE_CONTRACT_SHA256 or digest_file(contract_path)!=PRODUCT_COVERAGE_CONTRACT_SHA256 or value.get("populationSha256")!=PRODUCT_COVERAGE_POPULATION_SHA256 or digest_file(harness.POPULATION)!=PRODUCT_COVERAGE_POPULATION_SHA256 or value.get("catalogSha256")!=catalog_sha or value.get("positiveCells")!=14 or value.get("supportedUpperBound")!=contract.get("currentSupportedUpperBound") or value.get("supportedUpperBound")!=python_summary["upperBound"] or value.get("cellResults")!=contract.get("cells") or value.get("cellResults")!=python_summary["cells"]:
        raise RuntimeError("semantic-corpus product coverage authority contract mismatch")
    return {"authority":value,"authorityStdoutSha256":digest_bytes(completed.stdout.encode()),"catalog":catalog,"catalogSha256":catalog_sha,"semanticCorpusBinarySha256":digest_file(semantic_corpus),"semanticCorpusBinaryRealPath":str(semantic_corpus.resolve())}


def build_product_coverage_report(harness:Any,store:Store,binary:Path,product_root:Path,authority:dict[str,Any],freeze:str)->dict[str,Any]:
    result=authority["authority"]; upper=result["supportedUpperBound"]; paths=harness.coverage_product_paths(product_root); path_digests={path:digest_file(product_root/path) for path in paths}
    observed={"compositionIds":len({cell["requiredRoot"]["operator"] for cell in result["cellResults"] if cell.get("status")=="SUPPORTED" and cell.get("requiredRoot")}),"positiveCells":upper,"denominator":14,"exactAmbiguity":False,"mustRefuse":0,"falseBound":0,"builds":["GRADLE","MAVEN"]}
    passed=upper>=PRODUCT_COVERAGE_REQUIRED["positiveCells"]
    semantic_path=Path(authority["semanticCorpusBinaryRealPath"])
    return {"schema":"semantic-editing-e04-product-coverage/0.1","status":"COVERAGE_ACCEPTED" if passed else "COVERAGE_REJECTED","decision":"GO" if passed else "NO_GO","graphHash":store.graph_hash,"storeId":store.store_id,"diagnosticFreezeSha256":freeze,"productRevision":subprocess.run(["git","rev-parse","HEAD"],cwd=product_root,text=True,capture_output=True,check=True).stdout.strip(),"productBinarySha256":digest_file(binary),"semanticCorpusBinarySha256":digest_file(semantic_path),"semanticCorpusBinaryRealPath":str(semantic_path.resolve()),"catalogSha256":authority["catalogSha256"],"runnerSha256":digest_file(Path(harness.__file__)),"populationSha256":digest_file(harness.POPULATION),"productPathDigestsSha256":digest_bytes(canonical(path_digests)),"packetSetSha256":digest_bytes(canonical([])),"required":PRODUCT_COVERAGE_REQUIRED,"observed":observed,"upperBoundPositiveCells":upper,"cellResults":result["cellResults"]}


def issue_product_coverage(harness:Any,store:Store,binary:Path,product_root:Path,semantic_corpus:Path,snapshot_factory:Any,authority_runner:Any=subprocess.run)->str:
    pre=snapshot_factory(); start_inputs=pre.for_node(store,"PRODUCT_COVERAGE_START_READY"); root_receipt(store,"PRODUCT_COVERAGE_START_READY",start_inputs); freeze=pre.get("diagnosticFreezeSha256")
    authority=invoke_product_coverage_authority(harness,semantic_corpus,binary,store,authority_runner); report=build_product_coverage_report(harness,store,binary,product_root,authority,freeze); checked=validate_product_coverage_report(report,store,freeze); identity=store_artifact_pointer(store,"product-coverage",report)
    with store.locked():
        post=snapshot_factory(); inputs=post.for_node(store,"PRODUCT_COVERAGE_GUARD"); post.raise_selected(store,"PRODUCT_COVERAGE_GUARD")
        dependencies,blockers=current_dependency_receipts(store,"PRODUCT_COVERAGE_GUARD",inputs)
        if blockers: return publish(store,"PRODUCT_COVERAGE_GUARD","BLOCKED",inputs,dependencies,{"blockers":blockers},"dependency not READY")
        if inputs.get("semanticCorpusBinarySha256")!=report["semanticCorpusBinarySha256"] or inputs.get("semanticCorpusBinaryRealPath")!=report["semanticCorpusBinaryRealPath"] or inputs.get("productCoverageSha256")!=identity: raise RuntimeError("product coverage authority changed before publication")
        status="READY" if checked["passes"] else "FAILED"; error=None if checked["passes"] else "COVERAGE_BELOW_PREREGISTERED_THRESHOLD"; evidence={"coverageReportSha256":identity,"authorityStdoutSha256":authority["authorityStdoutSha256"],"upperBoundPositiveCells":checked["upperBoundPositiveCells"],"observed":checked["observed"]}
        existing=store.receipt("PRODUCT_COVERAGE_GUARD"); pointer=store.pointer("PRODUCT_COVERAGE_GUARD"); expected_key=node_key(store,"PRODUCT_COVERAGE_GUARD",inputs,dependencies)
        if existing and pointer and existing.get("nodeKey")==expected_key and existing.get("status")==status and existing.get("error")==error and existing.get("evidence")==evidence: return pointer["receiptHash"]
        return publish(store,"PRODUCT_COVERAGE_GUARD",status,inputs,dependencies,evidence,error)


def import_product_coverage_failure_audit(harness:Any,store:Store,path:Path,binary:Path,product_root:Path,semantic_corpus:Path,snapshot_factory:Any,authority_runner:Any=subprocess.run)->str:
    if path.is_symlink() or not path.is_file(): raise RuntimeError("product coverage audit path is unsafe")
    raw=path.read_bytes(); audit=json.loads(raw); keys={"schema","decision","auditor","coverageFailedReceiptHash","coverageReportSha256","graphHash","storeId","diagnosticFreezeSha256","productRevision","binarySha256","semanticCorpusBinarySha256","semanticCorpusBinaryRealPath","catalogSha256","recomputedRequired","recomputedObserved","recomputedUpperBound","r1Materialized","controllersOpened","modelCalls"}
    if not isinstance(audit,dict) or set(audit)!=keys or raw!=canonical(audit) or audit.get("schema")!="semantic-editing-e04-product-coverage-audit/0.1" or audit.get("decision")!="ACCEPT_NO_GO" or audit.get("auditor")!=INDEPENDENT_AUDITOR_ID or audit.get("graphHash")!=store.graph_hash or audit.get("storeId")!=store.store_id or audit.get("recomputedRequired")!=PRODUCT_COVERAGE_REQUIRED or audit.get("r1Materialized") is not False or audit.get("controllersOpened") is not False or audit.get("modelCalls")!=0: raise RuntimeError("product coverage audit exact contract mismatch")
    coverage_pointer=store.pointer("PRODUCT_COVERAGE_GUARD"); coverage=store.receipt("PRODUCT_COVERAGE_GUARD"); report_sha=ContextSnapshot(None,Path("/unused"),Path("/unused"),Path("/unused"),store=store)._store_artifact("product-coverage"); report=load_json(store.root/f"objects/{report_sha}.json"); freeze=ContextSnapshot(None,Path("/unused"),Path("/unused"),Path("/unused"),store=store)._store_artifact("diagnostic-freeze")
    checked=validate_product_coverage_report(report,store,freeze); authority=invoke_product_coverage_authority(harness,semantic_corpus,binary,store,authority_runner); recomputed=build_product_coverage_report(harness,store,binary,product_root,authority,freeze)
    if not coverage_pointer or not coverage or coverage.get("status")!="FAILED" or coverage.get("error")!="COVERAGE_BELOW_PREREGISTERED_THRESHOLD" or audit.get("coverageFailedReceiptHash")!=coverage_pointer["receiptHash"] or audit.get("coverageReportSha256")!=report_sha or audit.get("diagnosticFreezeSha256")!=freeze or audit.get("productRevision")!=report["productRevision"] or audit.get("binarySha256")!=report["productBinarySha256"] or audit.get("semanticCorpusBinarySha256")!=report["semanticCorpusBinarySha256"] or audit.get("semanticCorpusBinaryRealPath")!=report["semanticCorpusBinaryRealPath"] or audit.get("catalogSha256")!=report["catalogSha256"] or audit.get("recomputedObserved")!=checked["observed"] or audit.get("recomputedUpperBound")!=checked["upperBoundPositiveCells"] or recomputed!=report: raise RuntimeError("product coverage audit recomputation/binding mismatch")
    pre=snapshot_factory(); root_receipt(store,"PRODUCT_COVERAGE_START_READY",pre.for_node(store,"PRODUCT_COVERAGE_START_READY")); audit_sha=store_artifact_pointer(store,"product-coverage-audit",audit)
    with store.locked():
        locked_snapshot=snapshot_factory(); locked_inputs=locked_snapshot.for_node(store,"PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT"); locked_snapshot.raise_selected(store,"PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT"); dependencies,blockers=current_dependency_receipts(store,"PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT",locked_inputs)
        if blockers: return publish(store,"PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT","BLOCKED",locked_inputs,dependencies,{"blockers":blockers},"dependency not READY")
        locked_pointer=store.pointer("PRODUCT_COVERAGE_GUARD"); locked_coverage=store.receipt("PRODUCT_COVERAGE_GUARD"); locked_report_sha=locked_snapshot.get("productCoverageSha256"); locked_audit_sha=locked_snapshot.get("productCoverageAuditSha256"); locked_freeze=locked_snapshot.get("diagnosticFreezeSha256"); locked_report=load_json(store.root/f"objects/{locked_report_sha}.json")
        if locked_pointer!=coverage_pointer or locked_coverage!=coverage or locked_report_sha!=report_sha or locked_audit_sha!=audit_sha or locked_freeze!=freeze or locked_report!=report or recomputed!=locked_report or locked_inputs.get("productCoverageSha256")!=locked_report_sha or locked_inputs.get("productCoverageAuditSha256")!=locked_audit_sha or locked_inputs.get("productCoverageFailedReceiptSha256")!=locked_pointer["receiptHash"] or locked_inputs.get("semanticCorpusBinarySha256")!=report["semanticCorpusBinarySha256"] or locked_inputs.get("semanticCorpusBinaryRealPath")!=report["semanticCorpusBinaryRealPath"]: raise RuntimeError("product coverage audit authority changed before publication")
        return publish(store,"PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT","READY",locked_inputs,dependencies,{"auditSha256":audit_sha,"coverageFailedReceiptHash":coverage_pointer["receiptHash"]})


def require_root(harness: Any, graph_path: Path, store_path: Path, root: str, experiment: Path, binary: Path, seed: Path, diagnostic_experiment: Path | None = None, diagnostic_output: Path | None = None, r1_output: Path | None = None, diagnostic_preflight: Path | None = None, diagnostic_audit: Path | None = None, semantic_corpus: Path | None = None) -> dict[str,Any]:
    graph=load_production_graph(graph_path); store=Store(store_path,graph,False); diagnostic=diagnostic_experiment or experiment; r1=experiment if diagnostic_experiment is not None else None; snapshot=ContextSnapshot(harness,binary,seed,diagnostic,r1,diagnostic_output,r1_output,diagnostic_preflight,diagnostic_audit,store,semantic_corpus); inputs=snapshot.for_node(store,root); snapshot.raise_selected(store,root)
    receipt=root_receipt(store,root,inputs); pointer=store.pointer(root)
    freeze_receipt=store.receipt("DIAGNOSTIC_FREEZE_VERIFY"); evidence=(freeze_receipt or {}).get("evidence") or {}
    freeze_hash=evidence.get("diagnosticFreezeArtifactHash"); freeze_path=store.root/"objects"/f"{freeze_hash}.json"
    if not freeze_hash or not freeze_path.is_file() or digest_file(freeze_path)!=freeze_hash: raise RuntimeError("readiness closure lacks diagnostic freeze artifact")
    return {"receipt":receipt,"receiptHash":pointer["receiptHash"],"graphHash":store.graph_hash,"storeId":store.store_id,"diagnosticFreezeArtifact":str(freeze_path),"diagnosticFreezeArtifactHash":freeze_hash,"inputs":inputs}


def issue_full_preflight(harness: Any, graph_path: Path, store_path: Path, report_path: Path, report: dict[str,Any], report_sha: str, experiment: Path, binary: Path, seed: Path, semantic_corpus:Path) -> str:
    graph=load_production_graph(graph_path); store=Store(store_path,graph,False); snapshot=ContextSnapshot(harness,binary,seed,experiment,diagnostic_output=report_path.parent,diagnostic_preflight=report_path,store=store,semantic_corpus=semantic_corpus); inputs=snapshot.for_node(store,"DIAGNOSTIC_FULL_PREFLIGHT_42")
    start=root_receipt(store,"DIAGNOSTIC_FULL_PREFLIGHT_START_READY",inputs); start_pointer=store.pointer("DIAGNOSTIC_FULL_PREFLIGHT_START_READY")
    freeze_receipt=store.receipt("DIAGNOSTIC_FREEZE_VERIFY"); freeze_hash=((freeze_receipt or {}).get("evidence") or {}).get("diagnosticFreezeArtifactHash")
    if store.object(report)!=report_sha: raise RuntimeError("diagnostic preflight object digest mismatch")
    receipt_hash=publish_checked(store,"DIAGNOSTIC_FULL_PREFLIGHT_42",inputs,lambda:check_preflight(report_path,harness,experiment,inputs,freeze_hash,start_pointer["receiptHash"],binary,store.root,report,report_sha))
    receipt=load_json(store.root/"objects"/f"{receipt_hash}.json")
    if receipt.get("status")!="READY": raise RuntimeError(f"DIAGNOSTIC_FULL_PREFLIGHT_42 issuance failed:{receipt.get('error')}")
    return receipt_hash


def publish_failed_preflight_attempt(store: Store, inputs: dict[str,str], start_receipt_hash: str, packet: dict[str,Any], packet_sha: str) -> str:
    with store.locked():
        dependencies,blockers=current_dependency_receipts(store,"DIAGNOSTIC_FULL_PREFLIGHT_42",inputs)
        if blockers or dependencies.get("DIAGNOSTIC_FULL_PREFLIGHT_START_READY")!=start_receipt_hash: raise RuntimeError("failed preflight attempt lacks recognized start root")
        evidence={"packetSha256":packet_sha,"packetStatus":packet.get("status"),"stoppedAt":packet.get("stoppedAt"),"stage":packet.get("stage"),"startRootReceiptHash":start_receipt_hash,"currentInputsSha256":digest_bytes(canonical(inputs)),"publicSetSha256":inputs["diagnosticPublicSetSha256"]}
        receipt={"schema":SCHEMA,"storeId":store.store_id,"graphHash":store.graph_hash,"checkerVersion":CHECKER_VERSION,"node":"DIAGNOSTIC_FULL_PREFLIGHT_42","nodeKey":node_key(store,"DIAGNOSTIC_FULL_PREFLIGHT_42",inputs,dependencies),"status":"FAILED","selectedInputs":selected_inputs(store,"DIAGNOSTIC_FULL_PREFLIGHT_42",inputs),"dependencies":dependencies,"evidence":evidence,"error":f"{packet.get('status')}:{packet.get('stage')}","createdUnixNs":time.time_ns()}
        receipt_hash=store.object(receipt); attempts=store.root/"attempts"; attempts.mkdir(exist_ok=True)
        attempt={"schema":ATTEMPT_SCHEMA,"storeId":store.store_id,"graphHash":store.graph_hash,"node":"DIAGNOSTIC_FULL_PREFLIGHT_42","receiptHash":receipt_hash}
        atomic_bytes(attempts/f"DIAGNOSTIC_FULL_PREFLIGHT_42-{receipt_hash}.json",canonical(attempt))
        current=store.receipt("DIAGNOSTIC_FULL_PREFLIGHT_42")
        if not current or current.get("status")!="READY":
            pointer={"schema":POINTER_SCHEMA,"storeId":store.store_id,"graphHash":store.graph_hash,"node":"DIAGNOSTIC_FULL_PREFLIGHT_42","receiptHash":receipt_hash}
            atomic_bytes(store.root/"current/DIAGNOSTIC_FULL_PREFLIGHT_42.json",canonical(pointer))
        return receipt_hash


def issue_failed_preflight(harness: Any, graph_path: Path, store_path: Path, packet: dict[str,Any], packet_sha: str, experiment: Path, binary: Path, seed: Path, semantic_corpus:Path) -> str:
    graph=load_production_graph(graph_path); store=Store(store_path,graph,False); snapshot=ContextSnapshot(harness,binary,seed,experiment,store=store,semantic_corpus=semantic_corpus); inputs=snapshot.for_node(store,"DIAGNOSTIC_FULL_PREFLIGHT_42")
    root_receipt(store,"DIAGNOSTIC_FULL_PREFLIGHT_START_READY",inputs); start=store.pointer("DIAGNOSTIC_FULL_PREFLIGHT_START_READY")
    if not start: raise RuntimeError("failed preflight attempt lacks start pointer")
    return publish_failed_preflight_attempt(store,inputs,start["receiptHash"],packet,packet_sha)


def issue_authority_completion(harness: Any, graph_path: Path, store_path: Path, node: str, required_root: str, evidence: dict[str,Any], experiment: Path, binary: Path, seed: Path, diagnostic_experiment: Path | None = None, diagnostic_output: Path | None = None, r1_output: Path | None = None, diagnostic_preflight: Path | None = None, diagnostic_audit: Path | None = None, semantic_corpus:Path|None=None) -> str:
    allowed={"DIAGNOSTIC_CANARY_3_COMPLETE":"DIAGNOSTIC_CANARY_START_READY","FINAL_MATRIX_126_COMPLETE":"FINAL_MATRIX_START_READY","JUDGE_COMPLETE":"JUDGE_START_READY","SUMMARY_COMPLETE":"SUMMARIZE_START_READY"}
    if allowed.get(node)!=required_root: raise RuntimeError("invalid readiness authority completion route")
    graph=load_production_graph(graph_path); store=Store(store_path,graph,False); diagnostic=diagnostic_experiment or experiment; r1=experiment if diagnostic_experiment is not None else None; snapshot=ContextSnapshot(harness,binary,seed,diagnostic,r1,diagnostic_output,r1_output,diagnostic_preflight,diagnostic_audit,store,semantic_corpus); inputs=snapshot.for_node(store,node)
    root_receipt(store,required_root,inputs)
    def captured() -> dict[str,Any]:
        snapshot.raise_selected(store,node)
        return {"authority":"captured-execution","rootReceiptHash":store.pointer(required_root)["receiptHash"],**evidence}
    receipt_hash=publish_checked(store,node,inputs,captured)
    receipt=load_json(store.root/"objects"/f"{receipt_hash}.json")
    if receipt.get("status")!="READY": raise RuntimeError(f"{node} authority issuance failed:{receipt.get('error')}")
    return receipt_hash
