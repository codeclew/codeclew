#!/usr/bin/env python3
"""Exact exported reader reference -> existing guarded Markdown repair.

Native lookup validates the exported bytes and original retained registry. This
adapter never picks a latest Work, reads private state, acquires source, or
implements another repair engine. The original citation/renderer checks remain
authoritative for the supported mechanical paragraph mutation.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys
import time


def encode(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n"


def command(argv, output, label):
    start = time.monotonic()
    result = subprocess.run(argv, capture_output=True)
    (output / (label + ".stdout")).write_bytes(result.stdout)
    (output / (label + ".stderr")).write_bytes(result.stderr)
    (output / (label + ".command.json")).write_text(encode({
        "argv": list(map(str, argv)), "exitCode": result.returncode,
        "seconds": time.monotonic() - start,
    }))
    if result.returncode:
        raise ValueError(f"NATIVE_OR_GUARDED_REPAIR_FAILED:{label}:{result.returncode}")
    return json.loads(result.stdout)


def repair(args):
    output = args.output_dir
    output.mkdir(parents=True, exist_ok=False)
    replacement = json.loads(args.replacement.read_bytes())
    if (not isinstance(replacement, dict)
            or not {"old", "new"} <= replacement.keys()
            or replacement.keys() - {"old", "new", "evidence", "rebind"}):
        raise ValueError("EXPLICIT_OLD_NEW_PARAGRAPH_REPLACEMENT_REQUIRED")
    base = [str(args.clew), "docs", "reader"]
    selector = ["--root", str(args.root), "--reference", args.reference,
                "--file", str(args.file)]
    # Drain only the selected section's bounded metadata. SOURCE bodies remain
    # behind the returned existing Work/read-part handles, never automatically read.
    cursor = None
    seen = set()
    section = None
    for page in range(128):
        argv = [*base, "query", *selector, "--max-items", "20", "--max-bytes", "12288"]
        if cursor is not None:
            argv.extend(["--cursor", cursor])
        value = command(argv, output, f"query-{page:03}")
        if value.get("reference") != args.reference or value.get("omitted"):
            raise ValueError("INCOMPLETE_OR_FOREIGN_READER_QUERY")
        for item in value.get("items", []):
            if item.get("kind") == "SECTION":
                if section is not None:
                    raise ValueError("AMBIGUOUS_READER_SECTION")
                section = item
        cursor = value.get("nextCursor")
        if cursor is None:
            break
        if cursor in seen:
            raise ValueError("REPEATED_READER_CURSOR")
        seen.add(cursor)
    else:
        raise ValueError("READER_METADATA_PAGINATION_LIMIT")
    if section is None or not section.get("repairSupported"):
        raise ValueError("READER_SECTION_HAS_NO_GUARDED_MARKDOWN_REPAIR_ADAPTER")
    receipt = command([*base, "repair-base", *selector, "--output", str(output / "base")],
                      output, "repair-base")
    if receipt.get("claim") != section["id"] or receipt.get("old") != replacement["old"]:
        raise ValueError("QUERIED_PARAGRAPH_DOES_NOT_MATCH_EXPLICIT_OLD_TEXT")
    patch = {"schema": "codeclew-reader-repair/1.0",
             "baseMarkdownDigest": receipt["baseMarkdownDigest"],
             "baseMapDigest": receipt["baseMapDigest"],
             "replacements": [{"claim": section["id"], **replacement}]}
    patch_file = output / "patch.json"
    patch_file.write_text(encode(patch))
    argv = [sys.executable, "-I", "-S", str(Path(__file__).with_name("author_citations.py")),
            "repair", "--base", str(output / "base"), "--patch", str(patch_file),
            "--renderer-command", str(args.renderer_command),
            "--output-dir", str(output / "repair")]
    for manifest in args.manifest:
        argv.extend(["--manifest", str(manifest)])
    if args.repositories:
        argv.extend(["--repositories", str(args.repositories)])
    value = command(argv, output, "guarded-repair")
    if value.get("status") != "ADDRESSED_REPAIR_RENDERED":
        raise ValueError("GUARDED_REPAIR_DID_NOT_ESTABLISH_RENDERED_RESULT")
    return {"status": "QUERIED_READER_PARAGRAPH_REPAIR_RENDERED", "reference": args.reference,
            "claim": section["id"], "repair": value, "sourceAcquisitionCommandsIssued": 0,
            "semanticTruthAssessed": False, "nativePublicationApproval": False,
            "scope": "Explicit whole paragraph only; original inline code/links and unrelated bytes guarded by existing helper"}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clew", type=Path, required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--reference", required=True)
    parser.add_argument("--file", type=Path, required=True)
    parser.add_argument("--replacement", type=Path, required=True,
                        help="Explicit JSON {old,new}; optional evidence/rebind")
    parser.add_argument("--manifest", type=Path, action="append", required=True)
    parser.add_argument("--renderer-command", type=Path, required=True)
    parser.add_argument("--repositories", type=Path)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args(argv)
    existed = args.output_dir.exists()
    try:
        if existed:
            raise ValueError("OUTPUT_ALREADY_EXISTS")
        value = repair(args)
        code = 0
    except (ValueError, OSError, KeyError, TypeError) as error:
        value = {"status": "READER_REPAIR_REFUSED", "reason": str(error),
                 "sourceFallbackAllowed": False, "semanticTruthAssessed": False}
        code = 2
    if not existed and args.output_dir.is_dir():
        (args.output_dir / "result.json").write_text(encode(value))
    print(encode(value), end="")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
