#!/usr/bin/env python3
"""Keep repository prose English while allowing explicit Russian product localization."""

from pathlib import Path
from html.parser import HTMLParser
import hashlib
import json
import os
import re
import stat
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent
CYRILLIC = re.compile(r"[\u0400-\u04ff]")

# Russian product messages and their executable examples are intentional.
# This closed list does not exempt README, runbooks, plans or release notes.
RUSSIAN_LOCALIZATION_FILES = {
    "crates/clew/assets/documentation/analysis.js",
    "crates/clew/assets/documentation/app.js",
    "crates/clew/assets/documentation/limits.js",
    "crates/clew/assets/documentation/reader.js",
    "crates/clew/src/documentation/history.rs",
    "crates/clew/src/documentation/language.rs",
    "crates/clew/src/documentation/proposals.rs",
    "crates/clew/src/documentation/reader.rs",
    "crates/clew/src/documentation/render.rs",
    "crates/clew/tests/documentation_language.rs",
    "crates/clew/tests/documentation_system.rs",
    "scripts/test_documentation_visual_reader.cjs",
}

# This source contains reader-facing localized strings alongside English code
# and comments. Only complete Rust string literals in this one file are exempt.
RUSSIAN_LOCALIZATION_STRING_FILES = {
    "crates/clew/src/documentation/operation_answer.rs",
}


def _rust_character_literal_end(content: str, start: int) -> int | None:
    """Return the end of a Rust character literal, leaving lifetimes alone."""
    if start + 1 >= len(content) or content[start] != "'":
        return None

    cursor = start + 1
    if content[cursor] == "\\":
        cursor += 1
        if cursor >= len(content):
            return None
        escape = content[cursor]
        if escape == "u" and cursor + 1 < len(content) and content[cursor + 1] == "{":
            close = content.find("}", cursor + 2)
            if close < 0 or "\n" in content[cursor:close]:
                return None
            digits = content[cursor + 2 : close].replace("_", "")
            if not digits or any(char not in "0123456789abcdefABCDEF" for char in digits):
                return None
            cursor = close + 1
        elif escape == "x":
            digits = content[cursor + 1 : cursor + 3]
            if len(digits) != 2 or any(char not in "0123456789abcdefABCDEF" for char in digits):
                return None
            cursor += 3
        elif escape in "nrt0\\'\"":
            cursor += 1
        else:
            return None
    else:
        if content[cursor] in "'\r\n":
            return None
        cursor += 1

    if cursor < len(content) and content[cursor] == "'":
        return cursor + 1
    return None


def _rust_string_literal_end(content: str, start: int) -> int | None:
    """Find a complete ordinary, raw, byte, or C-style Rust string token."""
    if start >= len(content):
        return None

    quote = start
    raw = False
    prefixed = False
    if content[start] == '"':
        pass
    elif content[start] in "brc":
        if start and (content[start - 1].isalnum() or content[start - 1] == "_"):
            return None
        prefixed = True
        marker = content[start]
        quote = start + 1
        if marker == "b" and quote < len(content) and content[quote] == "r":
            raw = True
            quote += 1
        elif marker == "r":
            raw = True
        elif quote < len(content) and content[quote] == '"':
            pass
        else:
            return None

        if raw:
            hashes = 0
            while quote < len(content) and content[quote] == "#":
                hashes += 1
                quote += 1
            if quote >= len(content) or content[quote] != '"':
                return None
            closing = '"' + "#" * hashes
            cursor = quote + 1
            while True:
                end_quote = content.find('"', cursor)
                if end_quote < 0:
                    return -1
                if content.startswith(closing, end_quote):
                    return end_quote + len(closing)
                cursor = end_quote + 1

    if not prefixed:
        quote = start
    if content[quote] != '"':
        return None

    cursor = quote + 1
    while cursor < len(content):
        char = content[cursor]
        if char == "\\":
            if cursor + 1 >= len(content):
                return -1
            cursor += 2
        elif char == '"':
            return cursor + 1
        else:
            cursor += 1
    return -1


def _skip_rust_trivia(content: str, start: int) -> int:
    """Skip whitespace and Rust comments between attribute tokens."""
    cursor = start
    while cursor < len(content):
        if content[cursor].isspace():
            cursor += 1
        elif content.startswith("//", cursor):
            newline = min(
                (index for index in (content.find("\n", cursor + 2), content.find("\r", cursor + 2)) if index >= 0),
                default=-1,
            )
            cursor = len(content) if newline < 0 else newline + 1
        elif content.startswith("/*", cursor):
            depth = 1
            cursor += 2
            while cursor < len(content) and depth:
                if content.startswith("/*", cursor):
                    depth += 1
                    cursor += 2
                elif content.startswith("*/", cursor):
                    depth -= 1
                    cursor += 2
                else:
                    cursor += 1
        else:
            break
    return cursor


def _rust_attribute_end(content: str, start: int) -> int | None:
    """Return the end of any Rust attribute, or -1 if its brackets are incomplete."""
    if content[start] != "#" or (
        start > 0 and (content[start - 1].isalnum() or content[start - 1] == "_")
    ):
        return None

    cursor = _skip_rust_trivia(content, start + 1)
    if cursor < len(content) and content[cursor] == "!":
        cursor = _skip_rust_trivia(content, cursor + 1)
    if cursor >= len(content) or content[cursor] != "[":
        return None

    opening = cursor
    depth = 1
    cursor = opening + 1
    while cursor < len(content):
        if content.startswith("//", cursor):
            newline = min(
                (index for index in (content.find("\n", cursor + 2), content.find("\r", cursor + 2)) if index >= 0),
                default=-1,
            )
            cursor = len(content) if newline < 0 else newline + 1
            continue
        if content.startswith("/*", cursor):
            cursor = _skip_rust_trivia(content, cursor)
            continue
        if content[cursor] == "'":
            character_end = _rust_character_literal_end(content, cursor)
            if character_end is not None:
                cursor = character_end
                continue
        string_end = _rust_string_literal_end(content, cursor)
        if string_end == -1:
            return -1
        if string_end is not None:
            cursor = string_end
            continue
        if content[cursor] == "[":
            depth += 1
        elif content[cursor] == "]":
            depth -= 1
            if depth == 0:
                return cursor + 1
        cursor += 1
    return -1


def _mask_rust_string_literals(content: str) -> str:
    masked = list(content)
    cursor = 0
    while cursor < len(content):
        if content[cursor] == "#":
            attribute_end = _rust_attribute_end(content, cursor)
            if attribute_end == -1:
                break
            if attribute_end is not None:
                cursor = attribute_end
                continue
        if content.startswith("//", cursor):
            newline = content.find("\n", cursor + 2)
            cursor = len(content) if newline < 0 else newline
            continue
        if content.startswith("/*", cursor):
            depth = 1
            cursor += 2
            while cursor < len(content) and depth:
                if content.startswith("/*", cursor):
                    depth += 1
                    cursor += 2
                elif content.startswith("*/", cursor):
                    depth -= 1
                    cursor += 2
                else:
                    cursor += 1
            continue

        if content[cursor] == "'":
            character_end = _rust_character_literal_end(content, cursor)
            if character_end is not None:
                cursor = character_end
                continue

        string_end = _rust_string_literal_end(content, cursor)
        if string_end == -1:
            # An incomplete literal cannot hide any later text.
            break
        if string_end is not None:
            for index in range(cursor, string_end):
                if content[index] != "\n":
                    masked[index] = " "
            cursor = string_end
            continue
        cursor += 1
    return "".join(masked)



# Only the copied native publication has embedded localization and retained code.
# Narratives, source metadata, arbitrary JSON fields, and ordinary site pages
# remain English. These are the producer versions in the published example.
GENERATED_DOCS_PREFIXES = (
    "site/examples/codeclew-source/docs/",
    "site/examples/current-workflow/docs/",
)
GENERATED_SERVICE = re.compile(r"generated/[0-9a-f]{64}/services/[^/]+\.(html|json)\Z")
GENERATED_BINDINGS = re.compile(r"generated/[0-9a-f]{64}/bindings\.json\Z")
JSON_STRING = re.compile(r'"(?:[^"\\]|\\.)*"', re.DOTALL)
SCRIPT = re.compile(r"(<script\b[^>]*>)(.*?)(</script\s*>)", re.DOTALL | re.IGNORECASE)
LOCALIZATION_ASSETS = ("app.js", "analysis.js", "reader.js", "limits.js")

# Exact executable script bodies from the immutable v0.13.7 public readers.
# Verified against source revision fd82a763 before admitting these digests.
# New reader assets must not invalidate already published frozen snapshots.
FROZEN_LOCALIZATION_SCRIPT_DIGESTS = {
    "7e772ad1b4c368ba5d3a49f0a1e3dd306f7b51e2bbba552f9b600e9a49d28faa",
    "81169526b9b54c0c78da0eeadfb6035e955aee15444772bd1158a81dc651252e",
}


def _blank(content: str) -> str:
    return re.sub(r"[^\r\n]", " ", content)


def _unique_object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def _retained_source_text(source: object, source_id: str) -> bool:
    if not isinstance(source, dict) or source.get("id") != source_id:
        return False
    text = source.get("text")
    occurrence = source.get("occurrence")
    if not isinstance(text, str) or not isinstance(occurrence, dict):
        return False
    start, end = occurrence.get("startByte"), occurrence.get("endByte")
    if type(start) is not int or type(end) is not int or not 0 <= start < end:
        return False
    try:
        raw = text.encode("utf-8")
    except UnicodeEncodeError:
        return False
    return (
        source.get("authority") == "EXACT_SNAPSHOT_TEXT"
        and isinstance(source.get("service"), str) and bool(source["service"])
        and isinstance(source.get("file"), str) and bool(source["file"])
        and isinstance(source.get("revision"), str) and bool(source["revision"])
        and isinstance(source.get("evidenceDigest"), str)
        and re.fullmatch(r"sha256:[0-9a-f]{64}", source["evidenceDigest"]) is not None
        and isinstance(occurrence.get("snapshot"), str)
        and re.fullmatch(r"sha256:[0-9a-f]{64}", occurrence["snapshot"]) is not None
        and end - start == len(raw)
        and source.get("textDigest") == "sha256:" + hashlib.sha256(raw).hexdigest()
    )


def _mask_generated_json(content: str, kind: str | None) -> str:
    try:
        value = json.loads(content, object_pairs_hook=_unique_object)
    except (ValueError, RecursionError):
        return content
    allowed = set()

    def sources(group: object, path: tuple) -> None:
        if isinstance(group, dict):
            for source_id, source in group.items():
                if _retained_source_text(source, source_id):
                    allowed.add(path + (source_id, "text"))

    if isinstance(value, dict):
        if kind == "service" and value.get("renderer") == "codeclew-documentation-html/1.16":
            sources(value.get("sources"), ("sources",))
            groups = value.get("operationSources")
            if isinstance(groups, dict):
                for operation, group in groups.items():
                    sources(group, ("operationSources", operation))
        elif kind == "bindings" and value.get("schema") == "codeclew-documentation-bindings/1.4":
            sources(value.get("retainedSources"), ("retainedSources",))

    # Walk parsed keys/values and original string tokens together. Replacement
    # is by exact JSON path, never by shared text (a narrative may quote code).
    tokens = iter(JSON_STRING.finditer(content))
    edits = []

    def token(text: str, path: tuple | None) -> None:
        match = next(tokens)
        if path in allowed:
            edits.append((match.start(), match.end(), _blank(match[0])))
        elif CYRILLIC.search(text) and not CYRILLIC.search(match[0]):
            # Escaped Unicode narratives must not bypass the prose check.
            edits.append((match.start(), match.start() + 1, "\u0400"))

    def walk(item: object, path: tuple = ()) -> None:
        if isinstance(item, dict):
            for key, child in item.items():
                token(key, None)
                walk(child, path + (key,))
        elif isinstance(item, list):
            for index, child in enumerate(item):
                walk(child, path + (index,))
        elif isinstance(item, str):
            token(item, path)

    walk(value)
    parts = []
    cursor = 0
    for start, end, replacement in edits:
        parts.extend((content[cursor:start], replacement))
        cursor = end
    parts.append(content[cursor:])
    return "".join(parts)


class _ScriptAttributes(HTMLParser):
    attributes: dict | None = None

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag == "script" and len(dict(attrs)) == len(attrs):
            self.attributes = dict(attrs)


def _mask_generated_publication(relative_path: str, content: str) -> str:
    prefix = next((value for value in GENERATED_DOCS_PREFIXES
                   if relative_path.startswith(value)), None)
    if prefix is None:
        return content
    path = relative_path[len(prefix):]
    service = GENERATED_SERVICE.fullmatch(path)
    if path.endswith(".json"):
        kind = "bindings" if GENERATED_BINDINGS.fullmatch(path) else "service" if service else None
        return _mask_generated_json(content, kind)
    if not path.endswith(".html"):
        return content
    assets = [(ROOT / "crates/clew/assets/documentation" / name).read_text(encoding="utf-8")
              for name in LOCALIZATION_ASSETS]

    def script(match: re.Match) -> str:
        parser = _ScriptAttributes()
        parser.feed(match[1])
        attrs = parser.attributes
        body = match[2]
        if attrs is not None and attrs.get("type") == "application/json":
            kind = "service" if service and attrs.get("id") == "document-data" else None
            body = _mask_generated_json(body, kind)
        elif attrs is not None and attrs.get("type", "") in ("", "text/javascript"):
            if hashlib.sha256(body.encode("utf-8")).hexdigest() in FROZEN_LOCALIZATION_SCRIPT_DIGESTS:
                return match[1] + _blank(body) + match[3]
            for asset in assets:
                # Exact known executable asset bytes only; added prose remains.
                body = body.replace(asset, _blank(asset))
        return match[1] + body + match[3]

    return SCRIPT.sub(script, content)


def rejected_cyrillic_line_numbers(relative_path: str, content: str) -> list[int]:
    if relative_path in RUSSIAN_LOCALIZATION_STRING_FILES:
        content = _mask_rust_string_literals(content)
    content = _mask_generated_publication(relative_path, content)
    return [
        line_number
        for line_number, line in enumerate(content.splitlines(), start=1)
        if CYRILLIC.search(line)
    ]


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
        if path.relative_to(ROOT).as_posix() in RUSSIAN_LOCALIZATION_FILES:
            continue
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
        relative_path = path.relative_to(ROOT).as_posix()
        for line_number in rejected_cyrillic_line_numbers(relative_path, content):
            findings.append(f"{relative_path}:{line_number}")

    if findings:
        print("Cyrillic text outside explicit product localization is not allowed; repository prose must be English:", file=sys.stderr)
        for finding in findings:
            print(f"  {finding}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
