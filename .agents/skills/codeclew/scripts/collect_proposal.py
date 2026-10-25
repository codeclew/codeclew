#!/usr/bin/env python3
"""Collect native proposal or recorded Work pages; never author or publish."""
from __future__ import annotations

import argparse
from collections import Counter
import json
from pathlib import Path
import subprocess
import time


class Incomplete(ValueError):
    pass


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise Incomplete("DUPLICATE_JSON_KEY")
        result[key] = value
    return result


def reject_constant(_value):
    raise Incomplete("NONFINITE_JSON_NUMBER")


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def page_identity(page, proposal, kind="proposal"):
    if not isinstance(page, dict):
        raise Incomplete("INVALID_PAGE_OBJECT")
    if "error" in page:
        raise Incomplete("NATIVE_ERROR_RESPONSE")
    if kind.startswith("work-"):
        if page.get("schema") != "codeclew-documentation-work-page/1.0":
            raise Incomplete("UNSUPPORTED_PAGE_SCHEMA")
        if page.get("work") != proposal:
            raise Incomplete("WORK_IDENTITY_CHANGED")
        required = ("snapshot", "subject", "contextDigest", "inputDigest", "membershipDigest", "receiptDigest")
        if page.get("influenceCoverage") != "RECORDED_READS_ONLY_EXECUTION_NOT_ATTESTED":
            raise Incomplete("UNTRACKED_OR_UNKNOWN_READ_COVERAGE")
    else:
        if page.get("schema") != "codeclew-docs-page/1.0" or page.get("reportSchema") != "codeclew-documentation-proposal-result/1.0":
            raise Incomplete("UNSUPPORTED_PAGE_SCHEMA")
        if page.get("proposal") != proposal:
            raise Incomplete("PROPOSAL_IDENTITY_CHANGED")
        required = ("work", "status", "meaningReview", "readDigest")
    for name in required:
        if not isinstance(page.get(name), str) or not page[name]:
            raise Incomplete("MISSING_PAGE_IDENTITY_" + name.upper())
    if kind == "proposal" and ("narrativeDigest" not in page or (page["narrativeDigest"] is not None and not isinstance(page["narrativeDigest"], str))):
        raise Incomplete("INVALID_NARRATIVE_DIGEST")
    if type(page.get("total")) is not int or page["total"] < 0:
        raise Incomplete("INVALID_TOTAL")
    if not isinstance(page.get("items"), list) or not isinstance(page.get("omitted"), list):
        raise Incomplete("MISSING_ITEMS_OR_OMITTED")
    if "nextCursor" not in page or (page["nextCursor"] is not None and (not isinstance(page["nextCursor"], str) or not page["nextCursor"])):
        raise Incomplete("MISSING_OR_INVALID_TERMINAL_CURSOR")
    # Work rows are heterogeneous: native COVERAGE is flat, while evidence and
    # section rows carry record. Preserve either shape without manufacturing it.
    if any(not isinstance(row, dict) or not isinstance(row.get("kind"), str) or not row["kind"]
           or not isinstance(row.get("id"), str) or not row["id"]
           or (kind == "proposal" and "record" not in row) for row in page["items"]):
        raise Incomplete("INVALID_ITEM")
    # Keep future identity/authority fields too. Only delivery fields vary by page.
    delivery = ("items", "omitted", "nextCursor", "receiptDigest") if kind.startswith("work-") else ("items", "omitted", "nextCursor")
    return {k: v for k, v in page.items() if k not in delivery}


def collect(launcher, root, proposal, output, *, limit=20, max_pages=1000,
            timeout=120, max_page_bytes=256 * 1024, kind="proposal", references=None, selection=None):
    if kind not in ("proposal", "work-initial", "work-evidence", "work-selection"):
        raise Incomplete("UNSUPPORTED_COLLECTION_KIND")
    if kind == "work-evidence" and (not isinstance(references, list) or not 1 <= len(references) <= 8
            or any(not isinstance(ref, str) or not ref for ref in references)
            or len(set(references)) != len(references)):
        raise Incomplete("INVALID_EXACT_REFERENCES")
    if kind == "work-selection" and not isinstance(selection, dict):
        raise Incomplete("MISSING_EXPLICIT_WORK_SELECTION")
    selected_base = dict(selection or {})
    initial_cursor = selected_base.pop("cursor", None)
    tail = kind == "work-selection" and initial_cursor is not None
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    identity = None
    items, omitted, pages = [], [], []
    cursor = initial_cursor
    seen_cursors, seen_rows = set(), set()
    terminal = False
    reason = None
    try:
        for index in range(max_pages):
            stem = f"page-{index + 1:04d}"
            command = [str(launcher), "docs", "proposal", "show", "--root", str(root),
                       "--proposal", proposal, "--limit", str(limit)]
            if kind.startswith("work-"):
                selection = output / (stem + ".selection.json")
                selected = dict(selected_base) if kind == "work-selection" else ({"references": references} if kind == "work-evidence" else {})
                if cursor is not None:
                    selected["cursor"] = cursor
                save(selection, selected)
                command = [str(launcher), "docs", "work", "read", "--root", str(root),
                           "--work", proposal, "--input", str(selection)]
            elif cursor is not None:
                command += ["--cursor", cursor]
            record = {"command": command, "cursor": cursor, "exitCode": None,
                      "stdout": stem + ".stdout", "stderr": stem + ".stderr"}
            save(output / (stem + ".command.json"), record)
            pages.append(record)
            start = time.monotonic()
            with (output / record["stdout"]).open("xb") as stdout, (output / record["stderr"]).open("xb") as stderr:
                try:
                    process = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=stdout,
                                             stderr=stderr, timeout=timeout, check=False)
                    record["exitCode"] = process.returncode
                except subprocess.TimeoutExpired:
                    record["failure"] = "PROCESS_TIMEOUT_NO_AUTOMATIC_RETRY"
                except OSError as error:
                    record["failure"] = "PROCESS_START_" + type(error).__name__
            record["seconds"] = time.monotonic() - start
            save(output / (stem + ".command.json"), record)
            if record.get("failure"):
                raise Incomplete(record["failure"])
            if record["exitCode"] != 0:
                raise Incomplete("NATIVE_PROCESS_FAILED")
            with (output / record["stdout"]).open("rb") as source:
                raw = source.read(max_page_bytes + 1)
            if len(raw) > max_page_bytes:
                raise Incomplete("HELPER_PAGE_BYTE_LIMIT")
            page = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_object,
                              parse_constant=reject_constant)
            observed = page_identity(page, proposal, kind)
            if identity is None:
                identity = observed
            elif observed != identity:
                raise Incomplete("PAGE_IDENTITY_CHANGED")
            for row in page["items"]:
                key = (row["kind"], row["id"])
                if key in seen_rows:
                    raise Incomplete("REPEATED_ITEM")
                seen_rows.add(key)
                items.append(row)
            omitted.extend(page["omitted"])
            cursor = page["nextCursor"]
            if cursor is None:
                terminal = True
                break
            if cursor in seen_cursors:
                raise Incomplete("REPEATED_CURSOR")
            seen_cursors.add(cursor)
            # An empty page with a new cursor still requires its continuation.
        if not terminal:
            raise Incomplete("HELPER_PAGE_LIMIT_BEFORE_TERMINAL")
        if omitted:
            raise Incomplete("OMITTED_RECORDS_REQUIRE_NATIVE_INSPECTION")
        if not tail and len(items) != identity["total"]:
            raise Incomplete("TOTAL_COUNT_MISMATCH")
    except Incomplete as error:
        reason = str(error)
    except (UnicodeError, json.JSONDecodeError, RecursionError) as error:
        reason = "INVALID_JSON_" + type(error).__name__
    except OSError as error:
        reason = "LOCAL_IO_" + type(error).__name__
    diagnostics = [row for row in items if row["kind"] == "DIAGNOSTIC"]
    codes = Counter(str(body.get("code", "UNSPECIFIED")) if isinstance(body, dict) else "UNSPECIFIED"
                    for row in diagnostics for body in (row.get("record", row),))
    save(output / "items.json", items)
    save(output / "diagnostics.json", diagnostics)
    save(output / "omitted.json", omitted)
    result = {"schema": ("codeclew-work-selection-collection/1.0" if kind == "work-selection" else "codeclew-work-evidence-collection/1.0" if kind == "work-evidence" else "codeclew-work-initial-collection/1.0") if kind.startswith("work-") else "codeclew-proposal-collection/1.0",
              "status": ("TAIL_RECORDED" if tail else "COMPLETE") if reason is None else "INCOMPLETE",
              "reason": reason, "identity": identity, "pages": len(pages),
              "nativeStatus": identity.get("status") if identity else None,
              "meaningReview": identity.get("meaningReview") if identity else None,
              "readScope": ("EXPLICIT_SELECTION_TAIL" if tail else "EXPLICIT_SELECTION") if kind == "work-selection" else ("EXACT_REFERENCES" if kind == "work-evidence" else "INITIAL_DEFAULT_ONLY") if kind.startswith("work-") else "SAVED_PROPOSAL",
              "selectionComplete": reason is None and not tail,
              "continuationSelection": {**selected_base, "cursor": cursor} if kind == "work-selection" and cursor is not None else None,
              "rows": len(items), "rowKinds": dict(sorted(Counter(row["kind"] for row in items).items())),
              "terminalCursorObserved": terminal, "nextCursor": cursor,
              "omittedRecords": len(omitted), "diagnosticRows": len(diagnostics),
              "diagnosticCodes": dict(sorted(codes.items())[:16]),
              "diagnosticCodesNotShown": max(0, len(codes) - 16),
              "artifacts": {"pages": pages, "items": "items.json", "diagnostics": "diagnostics.json", "omitted": "omitted.json"},
              "publication": "NOT_PERFORMED", "meaningValidation": "NOT_PERFORMED",
              "nextAction": ("Requested cursor tail recorded; earlier pages are not included and full selection delivery is not established." if tail else "Explicit selection delivered; archive presence does not prove delivery to or reading by a model." if kind == "work-selection" else ("Selected reference delivery complete; native candidate and independent meaning gates remain separate." if kind == "work-evidence" else "Initial selection delivered. Continue the requested thread through supported source/body reads; this does not complete its explanation.") if kind.startswith("work-") else "Inspect the saved native status and every diagnostic; complete delivery is not approval.") if reason is None else "Inspect saved raw responses and command results; resolve the named blocker through supported native commands and matching part reads. Partial rows are incomplete delivery."}
    save(output / "manifest.json", result)
    return result


def submit_and_collect(launcher, root, work, input_path, output, label="submit", *, max_pages=1000, timeout=120):
    """Submit exact archived input to the same Work, preserving failures first."""
    command = [str(launcher), "docs", "proposal", "submit", "--root", str(root),
               "--work", work, "--input", str(input_path)]
    record = {"command": command, "exitCode": None}
    save(output / (label + ".command.json"), record)
    with (output / (label + ".stdout")).open("xb") as stdout, (output / (label + ".stderr")).open("xb") as stderr:
        start = time.monotonic()
        try:
            process = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=stdout,
                                     stderr=stderr, timeout=timeout, check=False)
            record["exitCode"] = process.returncode
        except subprocess.TimeoutExpired:
            record["failure"] = "NATIVE_TIMEOUT_NO_AUTOMATIC_RETRY"
        except OSError as error:
            record["failure"] = "NATIVE_START_" + type(error).__name__
    record["seconds"] = time.monotonic() - start
    save(output / (label + ".command.json"), record)
    save(output / (label + ".exit.json"), {"exitCode": record["exitCode"]})
    if record.get("failure") or record["exitCode"] != 0:
        raise Incomplete(record.get("failure", "NATIVE_SUBMIT_FAILED"))
    with (output / (label + ".stdout")).open("rb") as source:
        raw = source.read(256 * 1024 + 1)
    if len(raw) > 256 * 1024:
        raise Incomplete("NATIVE_RESULT_BYTE_LIMIT")
    result = json.loads(raw, object_pairs_hook=unique_object, parse_constant=reject_constant)
    if not isinstance(result, dict) or not isinstance(result.get("proposal"), str):
        raise Incomplete("MISSING_NATIVE_PROPOSAL")
    if result.get("work") != work:
        raise Incomplete("NATIVE_SUBMIT_WORK_IDENTITY_CHANGED")
    delivery = collect(launcher, root, result["proposal"], output / (label + "-collection"),
                       max_pages=max_pages, timeout=timeout)
    if delivery["status"] != "COMPLETE" or delivery["identity"]["work"] != work:
        raise Incomplete("INCOMPLETE_OR_FOREIGN_NATIVE_PROPOSAL")
    return delivery


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--launcher", type=Path, required=True, help="Explicit admitted installed clew launcher")
    parser.add_argument("--root", type=Path, required=True)
    kind = globals().get("DEFAULT_COLLECTION_KIND", "proposal")
    parser.add_argument("--work" if kind.startswith("work-") else "--proposal", required=True)
    parser.add_argument("--output", type=Path, required=True, help="New private collection directory; existing outputs are never overwritten")
    if kind == "proposal":
        parser.add_argument("--limit", type=int, default=20)
    parser.add_argument("--max-pages", type=int, default=1000)
    parser.add_argument("--timeout", type=int, default=120, help="Per native command, seconds; a timeout is never retried")
    args = parser.parse_args()
    if not 1 <= getattr(args, "limit", 20) <= 100 or not 1 <= args.max_pages <= 10000 or not 1 <= args.timeout <= 900:
        parser.error("limit must be 1..100, max-pages 1..10000 and timeout 1..900")
    try:
        result = collect(args.launcher.absolute(), args.root.absolute(),
                         args.work if kind.startswith("work-") else args.proposal,
                         args.output.absolute(), limit=getattr(args, "limit", 20), max_pages=args.max_pages,
                         timeout=args.timeout, kind=kind)
    except OSError as error:
        result = {"status": "INCOMPLETE", "reason": "OUTPUT_" + type(error).__name__,
                  "publication": "NOT_PERFORMED", "meaningValidation": "NOT_PERFORMED"}
    # Private record text and full diagnostics remain in files, not this summary.
    print(json.dumps({k: v for k, v in result.items() if k not in ("artifacts", "identity")}, separators=(",", ":")))
    return 0 if result["status"] == "COMPLETE" else 2


if __name__ == "__main__":
    raise SystemExit(main())
