#!/usr/bin/env python3
"""Exact committed-source mirror, separate from immutable native output.

Emits only the case's one Python file, with native source-range anchors. Requires
the changed capture packet directory, local exercise revision and caller clone.
Output belongs beneath reproduce/exercise-source/{revision}/scripts/.
"""
import argparse
import hashlib
import html
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clone", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--release-revision", required=True)
    parser.add_argument("--packet-dir", required=True)
    parser.add_argument("--output-root", required=True)
    args = parser.parse_args()
    file = "scripts/build_cli_documentation.py"
    revision = subprocess.check_output(
        ["git", "-C", args.clone, "rev-parse", "--verify", args.revision + "^{commit}"], text=True
    ).strip()
    if revision != args.revision:
        raise ValueError("Use the exact exercise commit, not a moving ref")
    source_bytes = subprocess.check_output(["git", "-C", args.clone, "show", f"{revision}:{file}"])
    source = source_bytes.decode()
    blob = subprocess.check_output(
        ["git", "-C", args.clone, "rev-parse", f"{revision}:{file}"], text=True
    ).strip()
    ranges = set()
    for path in Path(args.packet_dir).glob("inventory-*.json"):
        for row in json.loads(path.read_text())["items"]:
            record = row.get("record", {})
            if row.get("kind") == "SOURCE" and record.get("file") == file:
                if record.get("revision") != revision:
                    raise ValueError("Native source inventory belongs to another revision")
                occurrence = record.get("occurrence")
                if not occurrence or occurrence.get("blob") != blob:
                    raise ValueError("Native occurrence does not bind this committed blob")
                start_byte, end_byte = occurrence["startByte"], occurrence["endByte"]
                exact = source_bytes[start_byte:end_byte]
                if exact.decode() != record.get("text") or (
                    "sha256:" + hashlib.sha256(exact).hexdigest() != record.get("textDigest")
                ):
                    raise ValueError("Native retained text/digest differs from the committed source bytes")
                start_line = source_bytes[:start_byte].count(b"\n") + 1
                if record["startLine"] != start_line or (
                    record["endLine"] != start_line + len(exact.decode().splitlines()) - 1
                ):
                    raise ValueError("Native line range differs from its committed byte occurrence")
                ranges.add((record["startLine"], record["endLine"]))
    if not ranges:
        raise ValueError("No native source ranges are available")
    lines = source.splitlines()
    if any(start < 1 or end < start or end > len(lines) for start, end in ranges):
        raise ValueError("Native source range lies outside the committed file")
    content = ("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\">"
               "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">"
               "<title>Local maintenance exercise source</title>"
               "<style>body{font:16px system-ui;margin:2rem;max-width:80rem}"
               "pre{overflow:auto;line-height:1.5}.line{display:block}.line:target{background:#fff4bd}"
               ".range{scroll-margin-top:2rem}</style><h1>Local maintenance exercise source</h1>"
               "<p>This caller-owned exercise adds SVG-root validation to the saved diagram check. "
               "It is separate from released Codeclew source.</p>"
               f"<p>Public base revision: <code>{html.escape(args.release_revision)}</code><br>"
               f"Local exercise commit: <code>{revision}</code><br>File: <code>{file}</code><br>"
               f"Exact file SHA-256: <code>{hashlib.sha256(source_bytes).hexdigest()}</code><br>"
               '<a href="build_cli_documentation.py">Download exact committed source bytes</a></p><pre><code>')
    for index, line in enumerate(lines, 1):
        for start, end in sorted(ranges):
            if start == index:
                content += f'<span class="range" id="L{start}-L{end}"></span>'
        content += f'<span class="line" id="L{index}">{index:3} {html.escape(line)}</span>'
    content += "</code></pre></html>\n"
    output = Path(args.output_root) / revision / (file + ".html")
    output.parent.mkdir(parents=True, exist_ok=True)
    raw_output = output.with_suffix("")
    if output.exists() or raw_output.exists():
        raise ValueError("Preserve the previous mirror; choose a new output root")
    output.write_text(content)
    raw_output.write_bytes(source_bytes)
    print(json.dumps({"sourceMirror": str(output), "exerciseRevision": revision,
                      "nativeSourceRanges": len(ranges), "sourceFileSha256": hashlib.sha256(source_bytes).hexdigest(),
                      "rawCommittedSource": str(raw_output), "nativePublicationModified": False}))


if __name__ == "__main__":
    main()
