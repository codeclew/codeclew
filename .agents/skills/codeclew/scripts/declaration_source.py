#!/usr/bin/env python3
"""Read one exact retained declaration SOURCE through the supported clew CLI.

Navigation responses stay in fresh local receipts. Stdout contains the selected
native SOURCE, provenance and limitations, or an explicit failure. No source
checkout, refresh, guessed identity or body reconstruction is used.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time


SCHEMA = "codeclew-declaration-source/1.0"


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


class Reader:
    def __init__(self, args):
        self.args = args
        self.directory = args.output_dir
        self.receipts, self.coverage, self.envelopes, self.unread = [], [], [], []
        self.declaration = None
        self.sources = {}

    def call(self, stage, selection, cursor=None, page_size=None):
        command = [self.args.clew, "docs", "context", "--root", self.args.root,
                   "--service", self.args.service, "--snapshot", self.args.snapshot,
                   "--format", "compact", "--limit", str(page_size or self.args.page_size),
                   *selection]
        if cursor is not None:
            command += ["--cursor", cursor]
        stem = self.directory / f"{len(self.receipts):04d}-{stage}"
        started = time.monotonic()
        try:
            process = subprocess.run(command, stdin=subprocess.DEVNULL,
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        except OSError as error:
            raise Stop("LAUNCHER_EXECUTION_FAILED", exit_code=2, message=str(error)) from error
        stem.with_suffix(".stdout.json").write_bytes(process.stdout)
        stem.with_suffix(".stderr.log").write_bytes(process.stderr)
        receipt = {"stage": stage, "command": command, "exitCode": process.returncode,
                   "stdout": stem.name + ".stdout.json", "stderr": stem.name + ".stderr.log",
                   "stdoutSha256": hashlib.sha256(process.stdout).hexdigest(),
                   "stderrSha256": hashlib.sha256(process.stderr).hexdigest(),
                   "elapsedSeconds": time.monotonic() - started}
        self.receipts.append(receipt)
        stem.with_suffix(".command.json").write_text(encode(receipt), encoding="utf-8")
        try:
            packet = json.loads(process.stdout)
        except (ValueError, UnicodeDecodeError) as error:
            raise Stop("INVALID_NATIVE_OUTPUT", exit_code=2, rawReceipt=receipt) from error
        if process.returncode != 0 or (isinstance(packet, dict) and "error" in packet):
            code = (process.returncode if process.returncode > 0 else
                    128 - process.returncode if process.returncode < 0 else 2)
            raise Stop("NATIVE_ERROR", exit_code=min(code or 2, 255),
                       nativeReturnCode=process.returncode, nativeOutput=packet, rawReceipt=receipt)
        if (not isinstance(packet, dict) or packet.get("schema") != "codeclew-docs-page/1.0"
                or not isinstance(packet.get("items"), list)
                or not isinstance(packet.get("omitted"), list) or "nextCursor" not in packet):
            raise Stop("INVALID_NATIVE_PAGE", exit_code=2, rawReceipt=receipt)
        if packet.get("snapshot") != self.args.snapshot or packet.get("subject") != "service:" + self.args.service:
            raise Stop("NATIVE_SCOPE_MISMATCH", exit_code=2, rawReceipt=receipt)
        envelope = {k: v for k, v in packet.items() if k not in ("items", "nextCursor")}
        self.envelopes.append({"receipt": receipt["stdout"], "limits": limitations(envelope),
                               "omitted": packet["omitted"]})
        for index, item in enumerate(packet["items"]):
            if not isinstance(item, dict):
                raise Stop("INVALID_NATIVE_ITEM", exit_code=2, rawReceipt=receipt)
            if item.get("kind") == "COVERAGE" and item not in self.coverage:
                self.coverage.append(item)
            if item.get("kind") == "SOURCE":
                identity = item.get("id")
                if isinstance(identity, str):
                    previous = self.sources.get(identity)
                    if previous and previous[0] != item:
                        raise Stop("CONFLICTING_NATIVE_SOURCE", exit_code=2, sourceId=identity)
                    self.sources[identity] = (item, {"stdout": receipt["stdout"],
                                                     "pointer": f"/items/{index}"})
        return packet, receipt

    def find(self, stage, selection, predicate, *, page_size=None):
        cursor, seen = None, set()
        while True:
            packet, receipt = self.call(stage, selection, cursor, page_size)
            candidates = [(i, item) for i, item in enumerate(packet["items"]) if predicate(item)]
            if len(candidates) > 1:
                raise Stop("AMBIGUOUS_NATIVE_BINDING", candidates=[v for _, v in candidates])
            if candidates:
                index, item = candidates[0]
                if packet["nextCursor"] is not None:
                    self.unread.append({"stage": stage, "nextCursor": packet["nextCursor"],
                                        "reason": "Selected binding found; remaining navigation is not source/flow completeness proof."})
                return item, {"stdout": receipt["stdout"], "pointer": f"/items/{index}"}
            cursor = packet["nextCursor"]
            if cursor is None:
                raise Stop("SELECTED_BINDING_NOT_RETURNED", stage=stage,
                           omitted=[e for e in self.envelopes if e["omitted"]])
            if not isinstance(cursor, str) or not cursor or cursor in seen:
                raise Stop("INVALID_NATIVE_CONTINUATION", exit_code=2, stage=stage)
            seen.add(cursor)

    def read(self):
        symbol = self.args.symbol
        entrypoint = None
        if self.args.entrypoint is not None:
            item, pointer = self.find("entrypoint", ["--entrypoint", self.args.entrypoint],
                                     lambda i: i.get("kind") == "ENTRYPOINT" and i.get("id") == self.args.entrypoint,
                                     page_size=2)
            entrypoint = {"record": item.get("record"), "raw": pointer}
            record = item.get("record")
            if not isinstance(record, dict) or record.get("service") != self.args.service:
                raise Stop("INVALID_ENTRYPOINT_BINDING", exit_code=2)
            symbol = record.get("symbol")
        if not isinstance(symbol, str) or not symbol:
            raise Stop("MISSING_EXACT_SYMBOL")
        # The native --symbol selector rejects ambiguous declarations before paging.
        item, pointer = self.find("declaration", ["--symbol", symbol],
                                 lambda i: i.get("kind") == "DEPENDENCY"
                                 and isinstance(i.get("record"), dict)
                                 and i["record"].get("kind") == "SYMBOL"
                                 and i["record"].get("symbol") == symbol
                                 and i["record"].get("service") == self.args.service)
        declaration = item["record"]
        ids = declaration.get("sourceIds")
        normalized = declaration.get("normalized")
        if not isinstance(ids, list) or not all(isinstance(s, str) and s for s in ids) or not isinstance(normalized, dict):
            raise Stop("INVALID_DECLARATION_BINDING", exit_code=2)
        documentation = normalized.get("documentation")
        self.declaration = {"id": declaration.get("id"), "symbol": symbol,
                            "service": declaration.get("service"), "sourceIds": ids,
                            "fullRecordDigest": declaration.get("fullRecordDigest", declaration.get("digest")),
                            "raw": pointer, "limits": limitations(normalized),
                            "descriptor": {k: normalized[k] for k in (
                                "kind", "schema", "name", "ownerIdentity", "symbolIdentity", "scope",
                                "declarationKind", "syntaxKind", "outlineOwnerSource") if k in normalized},
                            "bodyStatus": documentation.get("bodyStatus") if isinstance(documentation, dict) else None}
        if not ids:
            raise Stop("MISSING_DECLARATION_SOURCE")
        if len(set(ids)) != 1:
            raise Stop("AMBIGUOUS_DECLARATION_SOURCE", sourceIds=ids)
        source_id = ids[0]
        if source_id in self.sources:
            source_item, source_pointer = self.sources[source_id]
        else:
            source_item, source_pointer = self.find("source", ["--source", source_id],
                                                   lambda i: i.get("kind") == "SOURCE" and i.get("id") == source_id)
        source = source_item.get("record")
        if (not isinstance(source, dict) or source.get("id") != source_id
                or source.get("service") != self.args.service or not isinstance(source.get("text"), str)):
            raise Stop("INVALID_SOURCE_BINDING", exit_code=2)
        if not source["text"]:
            raise Stop("EMPTY_DECLARATION_SOURCE", source=source_item)
        if (not all(isinstance(source.get(k), str) and source[k] for k in ("file", "revision", "authority"))
                or type(source.get("startLine")) is not int or type(source.get("endLine")) is not int
                or not 1 <= source["startLine"] <= source["endLine"]):
            raise Stop("INVALID_SOURCE_PROVENANCE", exit_code=2, source=source_item)
        expected = "sha256:" + hashlib.sha256(source["text"].encode("utf-8")).hexdigest()
        if source.get("textDigest") != expected:
            raise Stop("SOURCE_TEXT_DIGEST_MISMATCH", exit_code=2)
        revisions = {c["revision"] for c in self.coverage if "revision" in c}
        if revisions and revisions != {source.get("revision")}:
            raise Stop("SOURCE_REVISION_MISMATCH", exit_code=2)
        return {"status": "DECLARATION_SOURCE_RETURNED", "entrypoint": entrypoint,
                "sourceItems": [source_item], "sourceRaw": source_pointer,
                "sourceTextBytes": len(source["text"].encode("utf-8"))}

    def result(self):
        code = 0
        try:
            value = self.read()
        except Stop as error:
            code = error.exit_code
            value = {"status": error.status, **error.details}
        value.update(schema=SCHEMA, selection={"root": self.args.root, "service": self.args.service,
                     "snapshot": self.args.snapshot, "entrypoint": self.args.entrypoint, "symbol": self.args.symbol},
                     declaration=self.declaration, coverage=self.coverage,
                     evidenceLimits=self.envelopes, unreadNavigation=self.unread,
                     completeDeclarationClaimed=False, callFlowClosureClaimed=False,
                     sourceFallbackAllowed=False,
                     limitations=["Producer bodyStatus is retained when present; absence is unspecified.",
                                  "Exact source text is not proof of whole-file, dependency or runtime completeness.",
                                  "Host display completeness is not attested; complete result and native streams remain local."],
                     artifacts={"directory": str(self.directory), "result": "result.json", "receipts": "receipts.json"})
        if code:
            value["nextAction"] = ("Preserve this gap and native receipts. Recover through supported clew commands; "
                                   "request explicit user permission before any direct-source fallback. "
                                   "Documentation-generation permission does not authorize that fallback.")
        (self.directory / "receipts.json").write_text(encode(self.receipts), encoding="utf-8")
        (self.directory / "result.json").write_text(encode(value), encoding="utf-8")
        return value, code


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clew", required=True, help="Previously selected supported launcher")
    parser.add_argument("--root", required=True)
    parser.add_argument("--service", required=True)
    parser.add_argument("--snapshot", required=True)
    selector = parser.add_mutually_exclusive_group(required=True)
    selector.add_argument("--entrypoint", help="Exact returned entrypoint ID")
    selector.add_argument("--symbol", help="Exact returned canonical symbol")
    parser.add_argument("--output-dir", type=Path, required=True, help="New directory for complete native receipts")
    parser.add_argument("--page-size", type=int, default=100, choices=range(1, 101), metavar="1..100",
                        help="Internal native response size, not an investigation budget; metadata is saved locally")
    args = parser.parse_args(argv)
    args.clew = shutil.which(args.clew) or args.clew
    try:
        args.output_dir.mkdir(parents=True, exist_ok=False)
        value, code = Reader(args).result()
    except OSError as error:
        value, code = {"schema": SCHEMA, "status": "LOCAL_IO_ERROR", "message": str(error)}, 2
    print(encode(value), end="")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
