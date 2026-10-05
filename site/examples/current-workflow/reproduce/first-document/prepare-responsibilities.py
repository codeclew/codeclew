#!/usr/bin/env python3
"""Case-only preparation of the unchanged responsibilities section.

Uses public native CLI commands and pagination helpers from the shipped starter.
No model calls, source capture, proposal submission or publication.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True)
    parser.add_argument("--snapshot", required=True)
    parser.add_argument("--clew", default="clew")
    args = parser.parse_args()
    root = Path(args.root).resolve()
    spec = importlib.util.spec_from_file_location("shipped_starter", root / "examples/first-document.py")
    starter = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(starter)
    output = Path(tempfile.mkdtemp(prefix="responsibilities-", dir=root / "authoring"))
    command = lambda *a: starter.cli(args.clew, root, *a, progress=output / "cli-progress.jsonl")
    with tempfile.TemporaryDirectory() as temporary:
        input_path = Path(temporary) / "request.json"
        def with_input(*arguments, value):
            starter.save(input_path, value)
            return command(*arguments, "--input", str(input_path))
        request = {"schema": "codeclew-documentation-work-request/1.0",
                   "audience": "An engineer maintaining the public source walkthrough",
                   "entrypoint": "section-responsibilities", "maxItems": 100, "maxBytes": 40960}
        starter.save(output / "request.json", request)
        first = with_input("work", "prepare", "--subject", "service:cli-documentation",
                           "--snapshot", args.snapshot, value=request)
        work = first["work"]
        read = lambda selection: with_input("work", "read", "--work", work, value=selection)
        context = list(starter.pages(read, {}, first))
        if any(p.get("omitted") for p in context):
            raise ValueError("Responsibilities context is incomplete; inspect and complete omitted records")
        section = next(r for p in context for r in p["items"]
                       if r["kind"] == "SECTION" and r["id"] == "section-responsibilities")
        inventory = list(starter.pages(read, {"query": {"kind": "SOURCE"}}))
        rows = [r for p in inventory for r in p["items"] + p.get("omitted", []) if r.get("reference")]
        references = starter.select_sources(rows)
        if not references:
            raise ValueError("No source references were delivered")
        parts = []
        for reference in sorted(references):
            request = {"schema": "codeclew-documentation-source-part-request/1.0", "reference": reference}
            read_part = lambda selection: with_input("work", "read-part", "--work", work, value=selection)
            parts.extend(starter.pages(read_part, request))
        for prefix, responses in [("context", context), ("inventory", inventory), ("source", parts)]:
            for index, response in enumerate(responses):
                starter.save(output / f"{prefix}-{index:04}.json", response)
        starter.save(output / "result.json", {"work": work, "snapshot": args.snapshot,
                     "sectionReference": section["reference"], "sourceReferences": sorted(references),
                     "publication": "NOT_PUBLISHED"})
        packet = ("Read the complete native context and retained SOURCE parts below. Author only the "
                  "responsibilities section using references from this Work. Explain byte-binding "
                  "verification, presentation generation and the meaning/runtime limits. Preserve "
                  "the existing overview. This section will remain unchanged in the maintenance exercise.\n")
        for response in context + parts:
            packet += "\n```json\n" + json.dumps(response, indent=2, ensure_ascii=False) + "\n```\n"
        (output / "packet.md").write_text(packet)
        print(json.dumps({"output": str(output), "work": work, "sectionReference": section["reference"]}))


if __name__ == "__main__":
    main()
