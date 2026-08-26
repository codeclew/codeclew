#!/usr/bin/env python3
"""Reject Cyrillic text from tracked and pending repository content."""

from pathlib import Path
import os
import re
import stat
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent
CYRILLIC = re.compile(r"[\u0400-\u04ff]")


def repository_files() -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return [ROOT / os.fsdecode(name) for name in result.stdout.split(b"\0") if name]


def main() -> int:
    findings: list[str] = []
    for path in repository_files():
        try:
            metadata = path.lstat()
        except FileNotFoundError:
            continue
        if not stat.S_ISREG(metadata.st_mode):
            continue
        try:
            content = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        for line_number, line in enumerate(content.splitlines(), start=1):
            if CYRILLIC.search(line):
                findings.append(f"{path.relative_to(ROOT)}:{line_number}")

    if findings:
        print("Cyrillic text is not allowed; public repository content must be English:", file=sys.stderr)
        for finding in findings:
            print(f"  {finding}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
