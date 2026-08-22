#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$ROOT"
python3 -B -I -S "$ROOT/scripts/stabilization_control.py" guard --gate provider-prime >/dev/null
umask 077
GRADLE_OPTS="${GRADLE_OPTS:+$GRADLE_OPTS }-Dorg.gradle.daemon=false"
export GRADLE_OPTS

python3 -B -I -S - "$ROOT" <<'PY'
import hashlib
import json
import os
import pathlib
import re
import selectors
import signal
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import xml.etree.ElementTree as ElementTree

repository = pathlib.Path(sys.argv[1])
sys.path.insert(0, str(repository / "scripts"))
from bounded_gate_cleanup import cleanup_tree  # noqa: E402


def run(command, cwd, timeout):
    process = subprocess.Popen(
        [str(value) for value in command],
        cwd=cwd,
        env=os.environ,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    try:
        code = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        raise SystemExit("provider dependency prime timed out")
    if code != 0:
        raise SystemExit("provider dependency prime failed")


def run_output(command, cwd, timeout, stdout_limit=64 * 1024, stderr_limit=1024 * 1024):
    process = subprocess.Popen(
        [str(value) for value in command],
        cwd=cwd,
        env=os.environ,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    streams = selectors.DefaultSelector()
    assert process.stdout is not None and process.stderr is not None
    stdout_value = bytearray()
    stderr_value = bytearray()
    streams.register(process.stdout, selectors.EVENT_READ, (stdout_value, stdout_limit))
    streams.register(process.stderr, selectors.EVENT_READ, (stderr_value, stderr_limit))
    deadline = time.monotonic() + timeout
    try:
        while streams.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError
            for key, _events in streams.select(min(remaining, 1.0)):
                chunk = os.read(key.fileobj.fileno(), 64 * 1024)
                if not chunk:
                    streams.unregister(key.fileobj)
                    continue
                output, limit = key.data
                output.extend(chunk)
                if len(output) > limit:
                    raise OverflowError
        code = process.wait(timeout=max(0.1, deadline - time.monotonic()))
    except (OverflowError, subprocess.TimeoutExpired, TimeoutError):
        try:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=5)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        raise SystemExit("provider readiness output was unbounded or timed out")
    finally:
        streams.close()
    if code != 0:
        raise SystemExit("provider readiness command failed")
    return bytes(stdout_value)


def read_bounded_regular(path, limit, label):
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise SystemExit(f"{label} is unavailable") from error
    try:
        before = os.fstat(descriptor)
        if (
            not stat.S_ISREG(before.st_mode)
            or before.st_uid != os.geteuid()
            or before.st_size <= 0
            or before.st_size > limit
        ):
            raise SystemExit(f"{label} is empty, oversized, or unsafe")
        chunks = bytearray()
        while len(chunks) <= limit:
            block = os.read(descriptor, min(64 * 1024, limit + 1 - len(chunks)))
            if not block:
                break
            chunks.extend(block)
        after = os.fstat(descriptor)
        identity_before = (
            before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns
        )
        identity_after = (
            after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns
        )
        if (
            len(chunks) != before.st_size
            or len(chunks) > limit
            or identity_before != identity_after
        ):
            raise SystemExit(f"{label} changed while it was read")
        return bytes(chunks)
    finally:
        os.close(descriptor)


control_home = pathlib.Path(
    os.environ.get("CODECLEW_CONTROL_HOME", str(pathlib.Path.home() / ".cache" / "codeclew-control"))
)
temporary_parent = control_home / "tmp"
temporary_parent.mkdir(mode=0o700, parents=True, exist_ok=True)
os.chmod(temporary_parent, 0o700)
work = pathlib.Path(tempfile.mkdtemp(prefix="provider-prime.", dir=temporary_parent))
archive = work / "fixtures.tar"
try:
    with archive.open("wb") as stream:
        completed = subprocess.run(
            [
                "git", "archive", "--format=tar", "HEAD",
                "fixtures/kotlin-basic", "fixtures/kotlin-maven",
            ],
            cwd=repository,
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=stream,
            stderr=subprocess.DEVNULL,
        )
    if completed.returncode != 0 or archive.stat().st_size > 256 * 1024 * 1024:
        raise SystemExit("provider fixture archive is invalid")
    materialized = work / "source"
    materialized.mkdir(mode=0o700)
    with tarfile.open(archive, mode="r:") as source:
        members = source.getmembers()
        if len(members) > 20_000:
            raise SystemExit("provider fixture archive is oversized")
        for member in members:
            relative = pathlib.PurePosixPath(member.name)
            if relative.is_absolute() or ".." in relative.parts or member.issym() or member.islnk():
                raise SystemExit("provider fixture archive contains an unsafe entry")
            destination = materialized.joinpath(*relative.parts)
            if member.isdir():
                destination.mkdir(mode=0o700, parents=True, exist_ok=True)
            elif member.isfile():
                destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                stream = source.extractfile(member)
                if stream is None:
                    raise SystemExit("provider fixture archive file is unavailable")
                data = stream.read(64 * 1024 * 1024 + 1)
                if len(data) > 64 * 1024 * 1024:
                    raise SystemExit("provider fixture file is oversized")
                destination.write_bytes(data)
                os.chmod(destination, 0o700 if member.mode & 0o111 else 0o600)
            else:
                raise SystemExit("provider fixture archive contains a special entry")
    gradle = materialized / "fixtures" / "kotlin-basic"
    maven = materialized / "fixtures" / "kotlin-maven"
    run([gradle / "gradlew", "--no-daemon", "--quiet", "compileKotlin"], gradle, 1200)
    run([maven / "mvnw", "-q", "-DskipTests", "compile"], maven, 1200)
    model_output = work / "effective-pom.xml"
    classpath_output = work / "classpath.txt"
    run(
        [
            maven / "mvnw", "-q", "-DskipTests",
            f"-Doutput={model_output}",
            f"-Dmdep.outputFile={classpath_output}",
            "-Dmdep.includeScope=compile",
            "help:effective-pom", "dependency:build-classpath",
        ],
        maven,
        1200,
    )
    repository_output = run_output(
        [
            maven / "mvnw", "-q", "help:evaluate",
            "-Dexpression=settings.localRepository", "-DforceStdout",
        ],
        maven,
        1200,
    )
    run([maven / "mvnw", "-version"], maven, 120)
    if not model_output.is_file() or not classpath_output.is_file():
        raise SystemExit("provider Maven model goals did not produce readiness outputs")
    model_bytes = read_bounded_regular(
        model_output, 16 * 1024 * 1024, "provider Maven effective model"
    )
    try:
        ElementTree.fromstring(model_bytes)
    except ElementTree.ParseError as error:
        raise SystemExit("provider Maven effective model is not parseable XML") from error
    classpath_bytes = read_bounded_regular(
        classpath_output, 1024 * 1024, "provider Maven classpath"
    )
    try:
        classpath_entries = [
            pathlib.Path(value)
            for value in classpath_bytes.decode("utf-8").strip().split(os.pathsep)
            if value
        ]
    except UnicodeDecodeError as error:
        raise SystemExit("provider Maven classpath is not UTF-8") from error
    if not classpath_entries or any(
        not value.is_absolute() or ".." in value.parts or not value.is_file()
        for value in classpath_entries
    ):
        raise SystemExit("provider Maven classpath is not an absolute materialized classpath")
    repository_text = re.sub(
        rb"\x1b\[[0-9;]*[A-Za-z]", b"", repository_output
    ).decode("utf-8", errors="strict")
    repository_candidates = [
        pathlib.Path(line.strip())
        for line in repository_text.splitlines()
        if line.strip() and pathlib.Path(line.strip()).is_absolute()
    ]
    if len(repository_candidates) != 1:
        raise SystemExit("provider Maven repository result is not one absolute path")
    repository_result = repository_candidates[0]
    if ".." in repository_result.parts or not repository_result.is_dir():
        raise SystemExit("provider Maven repository result is not a materialized directory")
    result = {
        "fixtureAuthority": "sha256:" + hashlib.sha256(archive.read_bytes()).hexdigest(),
        "providers": ["GRADLE_PROJECT_NATIVE", "MAVEN_PROJECT_NATIVE"],
        "schema": "codeclew-provider-cache-prime/1.0",
        "status": "PASS",
    }
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
finally:
    if not cleanup_tree(str(work), 30):
        raise SystemExit("provider prime cleanup failed")
PY
