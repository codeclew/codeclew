#!/usr/bin/env python3
"""Discover one page of retained service SYMBOL declarations through supported APIs.

Work preparation/read writes native Work metadata and recorded-read receipts;
neither captures source. Use a caller-selected isolated docs root when an existing
root must stay byte-identical. This script never reads target files/private CAS.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import runpy
import shutil
import subprocess
import time


shared = runpy.run_path(str(Path(__file__).with_name("declaration_source.py")))
encode, Stop, limitations = (shared[name] for name in ("encode", "Stop", "limitations"))
SCHEMA = "codeclew-declaration-catalogue/1.0"


class Catalogue:
    def __init__(self, args):
        self.args, self.receipts = args, []
        self.work, self.coverage = args.work, None

    def call(self, label, arguments, *, input_value=None):
        stem = self.args.output_dir / f"{len(self.receipts):04d}-{label}"
        if input_value is not None:
            path = stem.with_suffix(".input.json")
            path.write_text(encode(input_value), encoding="utf-8")
            arguments = [*arguments, "--input", str(path)]
        command = [self.args.clew, *arguments]
        started = time.monotonic()
        try:
            process = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True)
        except OSError as error:
            raise Stop("LAUNCHER_EXECUTION_FAILED", exit_code=2, message=str(error)) from error
        stem.with_suffix(".stdout.json").write_bytes(process.stdout)
        stem.with_suffix(".stderr.log").write_bytes(process.stderr)
        receipt = {"command": command, "exitCode": process.returncode,
                   "elapsedSeconds": time.monotonic() - started,
                   "stdout": stem.name + ".stdout.json", "stderr": stem.name + ".stderr.log",
                   "stdoutSha256": hashlib.sha256(process.stdout).hexdigest(),
                   "stderrSha256": hashlib.sha256(process.stderr).hexdigest()}
        self.receipts.append(receipt)
        stem.with_suffix(".command.json").write_text(encode(receipt), encoding="utf-8")
        try:
            packet = json.loads(process.stdout)
        except (ValueError, UnicodeDecodeError) as error:
            raise Stop("INVALID_NATIVE_OUTPUT", exit_code=2, rawReceipt=receipt) from error
        if process.returncode or isinstance(packet, dict) and "error" in packet:
            code = process.returncode if process.returncode > 0 else 128 - process.returncode if process.returncode < 0 else 2
            raise Stop("NATIVE_ERROR", exit_code=min(code, 255), nativeReturnCode=process.returncode,
                       nativeOutput=packet, rawReceipt=receipt)
        if not isinstance(packet, dict):
            raise Stop("INVALID_NATIVE_OUTPUT", exit_code=2, rawReceipt=receipt)
        return packet, receipt

    def validate_page(self, page, schema):
        if (page.get("schema") != schema or not isinstance(page.get("items"), list)
                or not isinstance(page.get("omitted"), list) or "nextCursor" not in page):
            raise Stop("INVALID_NATIVE_PAGE", exit_code=2)
        if page.get("subject") != "service:" + self.args.service or page.get("snapshot") != self.args.snapshot:
            raise Stop("NATIVE_SCOPE_MISMATCH", exit_code=2)
        if any(not isinstance(item, dict) for item in page["items"]):
            raise Stop("INVALID_NATIVE_ITEM", exit_code=2)
        if page["nextCursor"] is not None and (not isinstance(page["nextCursor"], str) or not page["nextCursor"]):
            raise Stop("INVALID_NATIVE_CONTINUATION", exit_code=2)

    def read(self):
        if self.args.prepare_work:
            page, _ = self.call("prepare", ["docs", "work", "prepare", "--root", self.args.root,
                "--subject", "service:" + self.args.service, "--snapshot", self.args.snapshot],
                input_value={"schema": "codeclew-documentation-work-request/1.0",
                             "audience": "Retained declaration discovery", "maxItems": self.args.page_items or 6,
                             "maxBytes": 49152})
            self.validate_page(page, "codeclew-documentation-work-page/1.0")
            self.work = page.get("work")
        if not isinstance(self.work, str) or not self.work:
            raise Stop("MISSING_NATIVE_WORK_ID", exit_code=2)
        # One ordinary catalogue row supplies service revision/producer coverage;
        # its unread ENTRYPOINT continuation is intentionally not a discovery scan.
        page, _ = self.call("coverage", ["docs", "context", "--root", self.args.root,
            "--service", self.args.service, "--snapshot", self.args.snapshot,
            "--format", "compact", "--limit", "1"])
        self.validate_page(page, "codeclew-docs-page/1.0")
        coverage = [item for item in page["items"] if item.get("kind") == "COVERAGE"
                    and item.get("id") == self.args.service]
        if len(coverage) != 1 or not isinstance(coverage[0].get("revision"), str):
            raise Stop("MISSING_NATIVE_SERVICE_COVERAGE", exit_code=2)
        self.coverage = {"record": coverage[0], "authority": page.get("authority"),
                         "omitted": page["omitted"], "unreadEntrypointCursor": page["nextCursor"],
                         "limits": limitations({k: v for k, v in page.items() if k != "items"})}
        query = {"query": {"kind": "SYMBOL", "symbolContains": self.args.contains}}
        if self.args.cursor is not None:
            query["cursor"] = self.args.cursor
        page, receipt = self.call("declarations", ["docs", "work", "read", "--root", self.args.root,
                                                   "--work", self.work], input_value=query)
        self.validate_page(page, "codeclew-documentation-work-page/1.0")
        if page.get("work") != self.work:
            raise Stop("NATIVE_WORK_MISMATCH", exit_code=2)
        if page["nextCursor"] is not None and page["nextCursor"] == self.args.cursor:
            raise Stop("NON_PROGRESSING_NATIVE_CONTINUATION", exit_code=2)
        rows = []
        for index, item in enumerate(page["items"]):
            record = item.get("record")
            if (item.get("kind") != "DEPENDENCY" or not isinstance(record, dict)
                    or record.get("kind") != "SYMBOL" or record.get("service") != self.args.service
                    or not isinstance(record.get("symbol"), str) or record.get("id") != item.get("id")
                    or not isinstance(record.get("normalized"), dict)
                    or not isinstance(record.get("sourceIds"), list)
                    or not all(isinstance(s, str) and s for s in record["sourceIds"])):
                raise Stop("INVALID_DECLARATION_ROW", exit_code=2)
            normalized = record["normalized"]
            doc = normalized.get("documentation")
            rows.append({"kind": "DECLARATION", "id": record["id"], "symbol": record["symbol"],
                "service": record["service"], "revision": coverage[0]["revision"],
                "owner": normalized.get("ownerIdentity"), "name": normalized.get("name"),
                "observationKind": record["kind"], "descriptorKind": normalized.get("kind"),
                "scope": normalized.get("scope"),
                "declarationKind": normalized.get("declarationKind"), "syntaxKind": normalized.get("syntaxKind"),
                "sourceIds": record["sourceIds"],
                "sourceBindingStatus": "MISSING" if not record["sourceIds"] else
                    "MULTIPLE_UNREAD" if len(set(record["sourceIds"])) > 1 else "PRESENT_UNREAD",
                "bodyStatus": doc.get("bodyStatus") if isinstance(doc, dict) else None,
                "parameterTypes": doc.get("parameterTypes") if isinstance(doc, dict) else None,
                "authority": page.get("authority"), "limits": limitations(normalized),
                "observationDigest": record.get("digest"), "reference": item.get("reference"),
                "raw": {"stdout": receipt["stdout"], "pointer": f"/items/{index}"}})
        return {"status": "DECLARATIONS_RETURNED" if rows else "NO_DECLARATIONS_ON_PAGE",
                "items": rows, "total": page.get("total"), "nextCursor": page["nextCursor"],
                "omitted": page["omitted"], "nativePageMetadata": {k: v for k, v in page.items() if k != "items"},
                "selectionExhausted": page["nextCursor"] is None,
                "pageHasNoOmissions": not page["omitted"]}

    def result(self):
        code = 0
        try:
            value = self.read()
        except Stop as error:
            value, code = {"status": error.status, **error.details}, error.exit_code
        value.update(schema=SCHEMA, work=self.work, coverage=self.coverage,
            selection={"root": self.args.root, "service": self.args.service, "snapshot": self.args.snapshot,
                       "contains": self.args.contains, "cursor": self.args.cursor},
            sourceFallbackAllowed=False, sourceBodyReadAttested=False, activeEntrypointsClaimed=False,
            selectionDeliveryCompleteClaimed=False,
            limitations=["Literal case-sensitive native symbol substring filter; no semantic classification.",
                         "Declarations are not proof of public/active entrypoints, background execution or complete source bodies.",
                         "Source bindings remain unread; use declaration_source.py with a returned exact canonical symbol.",
                         "Raw native SYMBOL receipts can include additional token/descriptor fields; this catalogue does not attest a SOURCE body read.",
                         "Native Work preparation/read writes Work metadata/read receipts, not source capture.",
                         "Null cursor with omissions is not complete delivery; preserve native part-read recovery.",
                         "An empty filtered result is not proof that the service has no corresponding behavior."],
            artifacts={"directory": str(self.args.output_dir), "result": "result.json", "receipts": "receipts.json"})
        if code:
            value["nextAction"] = "Preserve native failure; recover through supported clew commands. Direct-source fallback requires explicit user permission."
        for name, data in [("result.json", value), ("receipts.json", self.receipts)]:
            (self.args.output_dir / name).write_text(encode(data), encoding="utf-8")
        return value, code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["clew", "root", "service", "snapshot"]:
        parser.add_argument("--" + name, required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--work", help="Existing saved service Work from exactly this snapshot")
    mode.add_argument("--prepare-work", action="store_true", help="Explicitly allow native Work metadata creation")
    parser.add_argument("--contains", default="", help="Literal case-sensitive symbol substring; empty lists all retained SYMBOLs")
    parser.add_argument("--cursor", help="Native cursor returned with the same Work and filter")
    parser.add_argument("--page-items", type=int, choices=range(1, 101), metavar="1..100",
                        help="Native per-page capacity for newly prepared Work; not a total investigation budget")
    parser.add_argument("--output-dir", type=Path, required=True, help="New local directory for full receipts")
    args = parser.parse_args()
    if args.cursor is not None and args.work is None:
        parser.error("--cursor requires the returned --work; do not prepare new Work for a continuation")
    if args.page_items is not None and not args.prepare_work:
        parser.error("--page-items applies only to --prepare-work; existing Work keeps its native page capacity")
    args.clew = shutil.which(args.clew) or args.clew
    try:
        args.output_dir.mkdir(parents=True, exist_ok=False)
        value, code = Catalogue(args).result()
    except OSError as error:
        value, code = {"schema": SCHEMA, "status": "LOCAL_IO_ERROR", "message": str(error)}, 2
    print(encode(value), end="")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
