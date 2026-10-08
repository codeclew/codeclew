#!/usr/bin/env python3
"""Opt-in one-invocation diagnostics, including failures before the core starts.

No upload, environment dump, source copy, output reparse as shareable evidence,
or additional product command. Only explicitly private output tails are saved.
"""
from __future__ import annotations

import hashlib
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import selectors
import signal
import stat
import subprocess
import sys
import time

FILE_LIMIT = 1024 * 1024
TOTAL_LIMIT = 8 * FILE_LIMIT
LINE_LIMIT = 64 * 1024
REPORT_SCHEMA = "codeclew-support-report/1.0"
BUNDLE_SCHEMA = "codeclew-support-bundle/1.0"
MESSAGE = b"Codeclew diagnostics: bundle finalized; review sharing/privacy in manifest.json.\n"
UNAVAILABLE = b"Codeclew diagnostics: collection unavailable; original command result is preserved.\n"
README = b"""# Codeclew invocation diagnostics

The original command ran once. stdout, stderr and its exit status are preserved;
diagnostic notifications are appended to stderr. stdin stays inherited. The child
sees pipes for stdout/stderr in this opt-in mode, so terminal detection, colors
and progress rendering can differ. No additional doctor/capture/model is run.

report.json is allowlisted metadata. Content digests and hashed service identities
can correlate a repository; review before sharing. Original terminal output is
NOT assumed shareable and is NOT stored by default. PRIVATE_* files and private/
contain raw output only after --include-private-logs; review them before sharing.
No upload is performed. Files are 0600, directories 0700, and tails are bounded.

Inspection is MANIFEST_METADATA only; heavy source/fact objects are NOT_VERIFIED.
Only the exact Check saved by this invocation is inspected, never an older latest.
Missing metadata and active SQLite WAL/journals produce PARTIAL. Runtime bootstrap
failures, help/version and parse failures can have no core metadata or Check.
A cold source launcher can build its runtime as part of the original invocation.
manifest.json is finalized last. SIGINT/TERM/HUP are forwarded to the child group.
"""

# Literal CLI names only; no unknown token or argument value can enter a report.
COMMANDS = {"docs": {"check", "context", "init", "render", "work", "snapshot", "endpoint", "list", "status", "bind", "service", "process", "proposal", "answer", "dataflow", "field"},
            "context": {"open", "read", "slice"}, "session": {"open", "inspect", "close", "abort", "publish", "recover"},
            "support": {"collect", "summarize"}, "doctor": {"task"}, "capabilities": set(),
            "upgrade": set(), "skill": {"install"}, "pack": {"list", "install", "remove"},
            "change": {"inspect", "open", "status"}, "task-run": {"status", "execute", "cancel"},
            "nav": {"query"}, "thread": {"open", "read"}, "workspace": {"open", "publish", "inspect"},
            "--help": set(), "--version": set()}


def command_name(arguments: list[str]) -> str:
    first = arguments[0] if arguments else ""
    if first not in COMMANDS:
        return "UNKNOWN"
    second = arguments[1] if len(arguments) > 1 else ""
    return first + "/" + second if second in COMMANDS[first] else first


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def encoded(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode("ascii")


def write_new(root: Path, name: str, data: bytes) -> dict[str, object]:
    if len(data) > FILE_LIMIT or Path(name).is_absolute() or ".." in Path(name).parts:
        raise ValueError("artifact budget or name")
    fd = os.open(root / name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    return {"path": name, "bytes": len(data), "digest": "sha256:" + hashlib.sha256(data).hexdigest()}


def prepare(output: str, arguments: list[str]) -> Path:
    target = Path(output).absolute()
    parent = target.parent.resolve(strict=True)
    target = parent / target.name
    # Collection must not itself alter the inspected docs root.
    if arguments[:2] == ["docs", "check"]:
        for index, arg in enumerate(arguments):
            selected = arguments[index + 1] if arg == "--root" and index + 1 < len(arguments) else arg.removeprefix("--root=") if arg.startswith("--root=") else None
            if selected is not None:
                docs = Path(selected).resolve()
                if target == docs or docs in target.parents:
                    raise ValueError("output inside docs")
    os.mkdir(target, 0o700)  # Existing files/directories/symlinks are refused.
    metadata = target.lstat()
    if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
        raise ValueError("output permissions")
    return target


class Stream:
    def __init__(self, destination: int, private: bool) -> None:
        self.destination = destination
        self.private = private
        self.observed = 0
        self.tail = bytearray()
        self.line = bytearray()
        self.line_overflow = False
        self.failure_stage: str | None = None

    def append(self, data: bytes) -> None:
        self.observed += len(data)
        if self.private:
            self.tail.extend(data)
            if len(self.tail) > FILE_LIMIT:
                del self.tail[:len(self.tail) - FILE_LIMIT]
        # Only known schema identity is projected; every other field/text is discarded.
        for fragment in data.splitlines(keepends=True):
            if not self.line_overflow:
                if len(self.line) + len(fragment) <= LINE_LIMIT:
                    self.line.extend(fragment)
                else:
                    self.line.clear()
                    self.line_overflow = True
            if fragment.endswith(b"\n"):
                if not self.line_overflow:
                    try:
                        value = json.loads(self.line)
                        schema = value.get("schema") if isinstance(value, dict) else None
                        if schema in ("codeclew-bootstrap-error/1.0", "codeclew-bootstrap-error/2.0"):
                            self.failure_stage = "BOOTSTRAP"
                        elif schema == "codeclew-installation-error/1.0":
                            self.failure_stage = "INSTALLATION"
                    except (ValueError, UnicodeError):
                        pass
                self.line.clear()
                self.line_overflow = False
        view = memoryview(data)
        while view:
            try:
                written = os.write(self.destination, view)
                view = view[written:]
            except BrokenPipeError:
                break

    def metadata(self) -> dict[str, object]:
        return {"observedBytes": self.observed, "retainedBytes": len(self.tail),
                "droppedBytes": self.observed - len(self.tail),
                "capture": "PRIVATE_BOUNDED_TAIL" if self.private else "NOT_STORED"}


def run_once(launcher: str, arguments: list[str], root: Path | None, private: bool) -> tuple[int, dict[str, Stream], int | None]:
    environment = os.environ.copy()
    for name in ("CLEW_DIAGNOSTIC_REPORT_DIR", "CLEW_DIAGNOSTIC_PRIVATE", "CLEW_DIAGNOSTIC_DEBUG_DIR"):
        environment.pop(name, None)
    if root is not None:
        environment["CLEW_DIAGNOSTIC_REPORT_DIR"] = str(root)
        if private:
            environment["CLEW_DIAGNOSTIC_PRIVATE"] = "1"
            debug = root / "private"
            os.mkdir(debug, 0o700)
            environment["CLEW_DIAGNOSTIC_DEBUG_DIR"] = str(debug)
            if arguments[:2] == ["docs", "check"] and not any(arg == "--debug-output" or arg.startswith("--debug-output=") for arg in arguments):
                arguments = [*arguments, "--debug-output", str(debug)]
    child = subprocess.Popen([launcher, *arguments], stdin=None, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, env=environment, start_new_session=True)
    interrupted: int | None = None
    interrupted_at = 0.0

    def forward(signum: int, _frame: object) -> None:
        nonlocal interrupted, interrupted_at
        if interrupted is None:
            interrupted = signum
            interrupted_at = time.monotonic()
        try:
            os.killpg(child.pid, signum)
        except ProcessLookupError:
            pass

    previous = {signum: signal.signal(signum, forward) for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)}
    streams = {"stdout": Stream(1, private), "stderr": Stream(2, private)}
    try:
        with selectors.DefaultSelector() as selector:
            assert child.stdout is not None and child.stderr is not None
            selector.register(child.stdout, selectors.EVENT_READ, streams["stdout"])
            selector.register(child.stderr, selectors.EVENT_READ, streams["stderr"])
            while selector.get_map() or child.poll() is None:
                if interrupted is not None:
                    elapsed = time.monotonic() - interrupted_at
                    if elapsed > 1:
                        try:
                            os.killpg(child.pid, signal.SIGKILL if elapsed > 2 else signal.SIGTERM)
                        except ProcessLookupError:
                            pass
                for key, _ in selector.select(0.2):
                    data = os.read(key.fileobj.fileno(), 64 * 1024)
                    if data:
                        key.data.append(data)
                    else:
                        selector.unregister(key.fileobj)
                        key.fileobj.close()
        return child.wait(), streams, interrupted
    finally:
        if child.poll() is None:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
        for signum, handler in previous.items():
            signal.signal(signum, handler)


def finish(root: Path, code: int, streams: dict[str, Stream], private: bool, interrupted: int | None, timing: dict[str, object]) -> None:
    report_file = root / "core-report.json"
    if report_file.exists():
        metadata = report_file.lstat()
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > FILE_LIMIT:
            raise ValueError("core report invalid")
        fd = os.open(report_file, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(fd, "rb") as stream:
            report = json.loads(stream.read(FILE_LIMIT + 1))
        if report.get("schema") != REPORT_SCHEMA:
            raise ValueError("core report schema")
    else:
        stage = streams["stderr"].failure_stage or streams["stdout"].failure_stage or "BEFORE_CORE_REPORT"
        report = {"schema": REPORT_SCHEMA, "status": "PARTIAL", "sourceStage": stage,
                  "inspectionScope": "MANIFEST_METADATA", "heavyObjects": "NOT_VERIFIED", "snapshot": None,
                  "runtime": {"version": None, "mode": "NOT_RECORDED", "platform": platform.system(), "architecture": platform.machine()},
                  "issues": [{"code": stage + "_METADATA_UNAVAILABLE"}],
                  "remediationId": "REVIEW_PRIVATE_RUN_LOGS" if private else "RERUN_WITH_PRIVATE_LOGS_IF_NEEDED"}
    report["invocation"] = {"exitCode": code if code >= 0 else 128 - code, "signal": -code if code < 0 else interrupted,
                            "executionCount": 0 if streams["stderr"].failure_stage == "COMMAND_START" else 1,
                            **timing, "stdout": streams["stdout"].metadata(), "stderr": streams["stderr"].metadata()}
    artifacts = [write_new(root, "report.json", encoded(report)), write_new(root, "README.md", README)]
    if private:
        for name, stream in streams.items():
            artifacts.append(write_new(root, "PRIVATE_" + name + ".tail", bytes(stream.tail)))
    # Inspect only this wrapper-owned private directory, never paths in evidence.
    allowed = ["core-report.json"]
    if private:
        allowed.append("PRIVATE_SAVED_DETAILS.json")
    if private and (root / "private").is_dir():
        allowed += [str(path.relative_to(root)) for path in sorted((root / "private").iterdir())]
    total = sum(int(artifact["bytes"]) for artifact in artifacts)
    for name in allowed:
        path = root / name
        if not path.exists():
            continue
        metadata = path.lstat()
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > FILE_LIMIT or stat.S_IMODE(metadata.st_mode) != 0o600:
            raise ValueError("private artifact invalid")
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(fd, "rb") as stream:
            data = stream.read(FILE_LIMIT + 1)
        total += len(data)
        if total > TOTAL_LIMIT - FILE_LIMIT:
            raise ValueError("total artifact budget")
        artifacts.append({"path": name, "bytes": len(data), "digest": "sha256:" + hashlib.sha256(data).hexdigest()})
    manifest = {"schema": BUNDLE_SCHEMA, "status": report["status"], "sharing": "PRIVATE_REVIEW_REQUIRED" if private else "ALLOWLISTED_METADATA",
                "artifacts": artifacts, "limits": {"fileBytes": FILE_LIMIT, "totalBytes": TOTAL_LIMIT}, "invocation": report["invocation"]}
    write_new(root, "manifest.pending", encoded(manifest))
    os.rename(root / "manifest.pending", root / "manifest.json")
    fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def main(arguments: list[str]) -> int:
    # Internal prefix is supplied by the source/installed shell entrypoint.
    if len(arguments) < 5 or arguments[0] != "--launcher" or arguments[2] != "--diagnostics":
        os.write(2, b"Usage: clew --diagnostics NEWDIR [--include-private-logs] COMMAND ...\n")
        return 2
    launcher, output = arguments[1], arguments[3]
    command = arguments[4:]
    private = command[:1] == ["--include-private-logs"]
    if private:
        command = command[1:]
    if not command:
        os.write(2, b"Codeclew diagnostics: a command is required.\n")
        return 2
    try:
        root = prepare(output, command)
    except (OSError, ValueError):
        root = None
    started_utc, started = utc_now(), time.monotonic()
    try:
        code, streams, interrupted = run_once(launcher, command, root, private and root is not None)
    except OSError:
        # No raw OS exception/path is added by diagnostics. A failed spawn has
        # no original child result; use the normal unavailable-command status.
        code, interrupted = 127, None
        streams = {"stdout": Stream(1, False), "stderr": Stream(2, False)}
        streams["stderr"].failure_stage = "COMMAND_START"
    try:
        if root is None:
            raise ValueError("output unavailable")
        timing = {"command": command_name(command), "startedAt": started_utc, "endedAt": utc_now(), "durationMs": int((time.monotonic() - started) * 1000)}
        finish(root, code, streams, private, interrupted, timing)
        os.write(2, MESSAGE)
    except (OSError, ValueError, TypeError):
        os.write(2, UNAVAILABLE)
    return code if code >= 0 else 128 - code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
