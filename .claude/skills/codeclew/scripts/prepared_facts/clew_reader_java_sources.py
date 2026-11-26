"""Verified operation-scoped Java source files and reader citations."""

import hashlib
from hashlib import sha256
import os
from pathlib import PurePosixPath
import re
import subprocess
import time


MAX_FILE_BYTES = 128 * 1024
MAX_FILES = 12
MAX_LOOKUPS = 36
MAX_CITATIONS = 28
MAX_PROTOCOL_CITATIONS = MAX_CITATIONS + 16
MAX_CITATION_BYTES = 3 * 1024
MAX_TOTAL_CITATION_BYTES = 20 * 1024
MAX_TREE_BYTES = 2 * 1024 * 1024
MAX_TREE_PATHS = 30_000


def java_path(file):
    if not isinstance(file, str) or not file.endswith(".java") or len(file) > 512:
        return False
    path = PurePosixPath(file)
    return (not path.is_absolute() and ".." not in path.parts and path.as_posix() == file
            and not any(character in file for character in ("\\", ":"))
            and not any(ord(character) < 32 for character in file))


def retained_bounds(source):
    literal = source["text"].encode()
    for low, high in (("startByte", "endByte"), ("fragmentStartByte", "fragmentEndByte")):
        first, last = source.get(low), source.get(high)
        if type(first) is type(last) is int and 0 <= first < last and last - first == len(literal):
            return first, last
    return None


def locate(raw, literal, line, offset=None):
    if not raw or not literal:
        return None
    if offset is None:
        # Native ranges can be addressable by line and literal without an absolute byte offset.
        first = 0
        for _ in range(line - 1):
            first = raw.find(b"\n", first) + 1
            if not first:
                return None
        limit = raw.find(b"\n", first)
        limit = len(raw) if limit < 0 else limit
        offset = raw.find(literal, first)
        repeated = raw.find(literal, offset + 1) if offset >= 0 else -1
        if offset < first or offset > limit or first <= repeated <= limit:
            return None
    if raw[offset:offset + len(literal)] != literal or raw[:offset].count(b"\n") + 1 != line:
        return None
    return offset


def work_blob(row, file, revision, snapshot):
    """Check the reconstructed Work file against its Git blob identity."""
    origin = row.get("occurrence") or {}
    blob = origin.get("blob")
    text = row.get("text")
    if (not snapshot or row.get("file") != file or row.get("revision") != revision or origin.get("snapshot") != snapshot
            or origin.get("generatedOrigin") or not isinstance(blob, str)
            or not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", blob) or not isinstance(text, str)):
        return None
    raw = text.encode()
    if (not raw or len(raw) > MAX_FILE_BYTES or (origin.get("startByte"), origin.get("endByte")) != (0, len(raw))
            or row.get("textDigest") != "sha256:" + sha256(raw).hexdigest()):
        return None
    git = hashlib.new("sha1" if len(blob) == 40 else "sha256")
    git.update(f"blob {len(raw)}\0".encode())
    git.update(raw)
    return raw if git.hexdigest() == blob else None


class Snapshot:
    def __init__(self, process, occurrences, repository, *, revision, work_files, citation_sources,
                 source_members=None, allow_legacy_pinned=False, deadline=None):
        self.operation = process["id"]
        self.repository = repository
        self.source_members = (None if source_members is None
                               else frozenset(file for file in source_members if java_path(file)))
        self.work_files = {file: row for file, row in work_files.items() if self.admits(file)}
        self.deadline = deadline
        self.coverage = {}
        self.legacy_coverage = {}
        self.missing_authority = False
        self.ranges = {}
        self.total_bytes = 0
        self.citation_count = 0
        self.files = {}
        self.protocol_paths = set()
        self.owners = {}
        self.tree_paths = None
        self.pinned = False
        self.complete_tree = self.source_members is not None
        retained = {ref: row for ref, row in occurrences.items()
                    if self.admits(row.get("file")) and row.get("authority") == "EXACT_SNAPSHOT_TEXT"
                    and not (row.get("peerProcess") or row.get("peerService"))
                    and row.get("revision") == revision
                    and isinstance(row.get("text"), str) and row.get("text")
                    and isinstance(row.get("revision"), str) and row["revision"]
                    and type(row.get("startLine")) is int and 1 <= row["startLine"] <= 200_000
                    and "sha256:" + sha256(row["text"].encode()).hexdigest() == row.get("textDigest")
                    and all(row.get(key) == process.get("sources", {}).get(ref, {}).get(key) for key in
                            ("file", "revision", "startLine", "endLine", "textDigest", "authority"))}
        self.retained = retained
        roots = [retained[ref] for fact in process.get("facts", ())
                 if fact.get("declaration") == process.get("root") and fact.get("kind") in {"BODY", "BODY_SOURCE"}
                 for ref in fact.get("sources", ()) if ref in retained]
        revisions = {row["revision"] for row in roots}
        self.revision = next(iter(revisions)) if len(revisions) == 1 else None
        snapshots = {(row.get("occurrence") or {}).get("snapshot") for row in roots}
        self.snapshot = next(iter(snapshots)) if len(snapshots) == 1 else None
        # Earlier compiler observations can be exact line-addressed excerpts.
        # The runner attests that the capture did not select a working tree.
        legacy = (not self.snapshot and allow_legacy_pinned and bool(roots)
                  and all(row.get("occurrence") is None for row in roots))
        if not self.revision or (not self.snapshot and not legacy):
            return
        self.retained = {ref: row for ref, row in retained.items()
                         if (row.get("occurrence") or {}).get("snapshot") == self.snapshot}
        for row in citation_sources.values():
            origin = row.get("occurrence") or {}
            if (self.admits(row.get("file")) and row.get("revision") == self.revision
                    and origin.get("snapshot") == self.snapshot and not origin.get("generatedOrigin")
                    and row.get("authority") == "EXACT_SNAPSHOT_TEXT" and isinstance(row.get("text"), str)
                    and row.get("textDigest") == "sha256:" + sha256(row["text"].encode()).hexdigest()):
                if bounds := retained_bounds(row):
                    self.coverage.setdefault(row["file"], []).append((bounds, row["text"].encode()))
                elif legacy and row.get("occurrence") is None:
                    self.legacy_coverage.setdefault(row["file"], []).append(row)
        for row in self.retained.values():
            self.owners.setdefault(row["file"], []).append(row)
        for row in roots:
            raw = work_blob(self.work_files.get(row["file"], {}), row["file"], self.revision, self.snapshot)
            if self.match(raw, row) is not None:
                self.files[row["file"]] = raw
        if repository and re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", self.revision):
            if self.git("cat-file", "-t", self.revision) == b"commit\n":
                for row in roots:
                    raw = self.pinned_blob(row["file"])
                    if self.match(raw, row) is None:
                        continue
                    if not legacy:
                        object_id = self.git("rev-parse", "--verify", self.revision + ":" + row["file"])
                        origin = row["occurrence"]
                        if (origin.get("generatedOrigin") or not object_id
                                or object_id.strip().decode("ascii", errors="replace") != origin.get("blob")):
                            continue
                    self.files[row["file"]] = raw
                    self.pinned = True
                    break

    def remaining(self, limit):
        return limit if self.deadline is None else max(0, min(limit, self.deadline - time.monotonic()))

    def admits(self, file):
        return java_path(file) and (self.source_members is None or file in self.source_members)

    def git(self, *args):
        if (timeout := self.remaining(3)) <= 0:
            return None
        try:
            env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
            env.update(GIT_NO_LAZY_FETCH="1", GIT_TERMINAL_PROMPT="0", GIT_ALLOW_PROTOCOL="")
            result = subprocess.run(["git", "--no-replace-objects", "-C", str(self.repository), *args],
                                    capture_output=True, stdin=subprocess.DEVNULL, timeout=timeout, check=True, env=env)
            return result.stdout
        except (OSError, subprocess.SubprocessError):
            return None

    def pinned_blob(self, file):
        address = self.revision + ":" + file
        size = self.git("cat-file", "-s", address)
        if not size or not size.strip().isdigit() or int(size) > MAX_FILE_BYTES:
            return None
        raw = self.git("cat-file", "blob", address)
        if raw is None or len(raw) != int(size):
            return None
        try:
            raw.decode("utf-8")
        except UnicodeError:
            return None
        return raw

    def paths(self):
        return tuple(dict.fromkeys((*self.owners, *self.work_files)))

    def paths_containing(self, *markers):
        """Locate candidates in admitted source before spending full-unit reads."""
        return (file for file, spans in self.coverage.items()
                if all(any(marker in raw for _, raw in spans) for marker in markers))

    def existing_paths(self, candidates):
        paths = list(dict.fromkeys(file for file in candidates if self.admits(file)))[:32]
        if not paths:
            return ()
        if not self.pinned:
            return tuple(file for file in paths if file in self.work_files)
        raw = self.git("--literal-pathspecs", "ls-tree", "-r", "--name-only", "-z", self.revision, "--", *paths)
        if raw is None:
            return ()
        return tuple(file for file in paths if file.encode() in raw.split(b"\0"))

    def type_paths(self, qualified):
        """Find exact package/file suffixes among the operation's admissible sources."""
        if not all(part.isidentifier() for part in qualified.split(".")):
            return ()
        suffix = qualified.replace(".", "/") + ".java"
        if self.source_members is not None:
            # Work's complete file inventory belongs to this specific service,
            # even when several services share the same pinned Git tree.
            return tuple(sorted(path for path in self.source_members if path == suffix or path.endswith("/" + suffix)))
        if not self.pinned:
            return tuple(path for path in self.work_files if path == suffix or path.endswith("/" + suffix))
        if self.tree_paths is None:
            self.tree_paths = {}
            raw = self.git("ls-tree", "-r", "--name-only", "-z", self.revision)
            self.complete_tree = raw is not None and len(raw) <= MAX_TREE_BYTES and raw.count(b"\0") <= MAX_TREE_PATHS
            if self.complete_tree:
                for entry in raw.split(b"\0"):
                    if entry.endswith(b".java"):
                        file = entry.decode("utf-8", errors="replace")
                        if java_path(file) and file.encode() == entry:
                            self.tree_paths.setdefault(PurePosixPath(file).name, []).append(file)
        return tuple(path for path in self.tree_paths.get(PurePosixPath(suffix).name, ())
                     if path == suffix or path.endswith("/" + suffix))

    def read(self, file, *, protocol=False):
        if not self.revision or not (self.snapshot or self.pinned) or not self.admits(file):
            return None
        if file not in self.files:
            # A protocol lookup may discover assembly/type files absent from
            # the operation's own body walk. Cached bytes remain shared.
            counted = [raw for path, raw in self.files.items() if (path in self.protocol_paths) == protocol]
            if len(counted) >= MAX_LOOKUPS or sum(map(bool, counted)) >= MAX_FILES:
                return None
            raw = work_blob(self.work_files.get(file, {}), file, self.revision, self.snapshot)
            self.files[file] = raw if raw is not None else self.pinned_blob(file) if self.pinned else None
            if protocol:
                self.protocol_paths.add(file)
        return self.files.get(file)

    def match(self, raw, source):
        bounds = retained_bounds(source)
        literal = source["text"].encode()
        first = source["startLine"]
        if type(first) is not int or source.get("endLine") != first + literal.removesuffix(b"\n").count(b"\n"):
            return None
        return locate(raw, literal, first, bounds[0] if bounds else None)

    def file_for(self, source):
        if source.get("revision") != self.revision:
            return None
        raw = self.read(source["file"])
        return raw if self.match(raw, source) is not None else None

    def covered(self, file, start, end):
        raw = self.read(file)
        if not raw or not 0 <= start < end <= len(raw):
            return False
        payload = raw[start:end]
        if self.pinned:
            for row in self.legacy_coverage.pop(file, ()):
                if (first := self.match(raw, row)) is not None:
                    text = row["text"].encode()
                    self.coverage.setdefault(file, []).append(((first, first + len(text)), text))
        return any(first <= start < end <= last and covered[start - first:end - first] == payload
                   for (first, last), covered in self.coverage.get(file, ()))

    def cite(self, file, start, end, *, declaration=False, protocol=False):
        raw = self.read(file)
        if not raw or not 0 <= start < end <= len(raw) or end - start > (24 * 1024 if declaration else MAX_CITATION_BYTES):
            return None
        if not self.covered(file, start, end):
            self.missing_authority = True
            return None
        payload = raw[start:end]
        try:
            literal = payload.decode("utf-8")
        except UnicodeError:
            return None
        digest = sha256(payload).hexdigest()
        identity = "\0".join((self.operation, self.revision, file, str(start), str(end), digest))
        identifier = "J" + sha256(identity.encode()).hexdigest()[:20]
        if identifier not in self.ranges:
            if not declaration:
                maximum = MAX_PROTOCOL_CITATIONS if protocol else MAX_CITATIONS
                if self.citation_count >= maximum or self.total_bytes + len(payload) > MAX_TOTAL_CITATION_BYTES:
                    return None
                self.total_bytes += len(payload)
                self.citation_count += 1
            self.ranges[identifier] = {
                "file": file, "revision": self.revision, "operation": self.operation,
                "startLine": 1 + raw[:start].count(b"\n"),
                "endLine": 1 + raw[:end].removesuffix(b"\n").count(b"\n"),
                "startByte": start, "endByte": end, "authority": "EXACT_SNAPSHOT_TEXT",
                "textDigest": "sha256:" + digest, "text": literal, "references": [],
            }
        return identifier

    def cite_many(self, spans, *, protocol=False):
        """Reserve a witness's ranges together so an omitted witness uses no capacity."""
        before = set(self.ranges)
        used, count = self.total_bytes, self.citation_count
        refs = []
        for file, start, end in spans:
            identifier = self.cite(file, start, end, protocol=protocol)
            if identifier is None:
                for ref in self.ranges.keys() - before:
                    del self.ranges[ref]
                self.total_bytes, self.citation_count = used, count
                return None
            refs.append(identifier)
        return refs

    def compact_witnesses(self, spans):
        """Keep the leading occurrence exact and coalesce adjacent supporting witnesses."""
        first, *rest = spans
        existing = {}
        for row in self.ranges.values():
            if row["endByte"] - row["startByte"] <= MAX_CITATION_BYTES:
                existing.setdefault(row["file"], []).append((row["startByte"], row["endByte"]))
        existing.setdefault(first[0], []).append(first[1:])
        retained, compact = [first], []
        for file, start, end in sorted(rest):
            coverings = [(lower, upper) for lower, upper in existing.get(file, ()) if lower <= start and end <= upper]
            if coverings:
                lower, upper = min(coverings, key=lambda pair: pair[1] - pair[0])
                retained.append((file, lower, upper))
                continue
            if compact and compact[-1][0] == file:
                _, lower, upper = compact[-1]
                merged_end = max(upper, end)
                if (start - upper <= 96 and merged_end - lower <= 700
                        and self.covered(file, lower, merged_end)):
                    compact[-1] = file, lower, merged_end
                    continue
            compact.append((file, start, end))
        return tuple(dict.fromkeys((*retained, *compact)))
