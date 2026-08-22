#!/usr/bin/env python3
"""Bounded, independently attempted cleanup for qualification gates."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import signal
import subprocess
import sys


def terminate(process: subprocess.Popen[bytes]) -> None:
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=5)
        return
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        return
    process.wait()


def run_bounded(command: list[str], timeout: int) -> bool:
    try:
        process = subprocess.Popen(
            command,
            env=os.environ,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
    except OSError:
        return False
    try:
        return process.wait(timeout=timeout) == 0
    except subprocess.TimeoutExpired:
        terminate(process)
        return False
    except BaseException:
        terminate(process)
        raise


def cleanup_session(clew: str, session: str, timeout: int) -> bool:
    results = []
    # A launch failure, timeout, or nonzero close must never suppress GC.
    for action in ("close", "gc"):
        results.append(
            run_bounded(
                [clew, "session", action, "--json", "--session", session], timeout
            )
        )
    return all(results)


TREE_PROGRAM = r"""
import os
import pathlib
import shutil
import stat
import sys

root = pathlib.Path(sys.argv[1])
if not root.is_absolute() or '..' in root.parts:
    raise SystemExit('cleanup root must be normalized and absolute')
if root.exists():
    metadata = root.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
        raise SystemExit('cleanup root is not a physical directory')
    for current, directories, _files in os.walk(root, topdown=False, followlinks=False):
        for name in directories:
            path = pathlib.Path(current, name)
            child = path.lstat()
            if stat.S_ISDIR(child.st_mode) and not stat.S_ISLNK(child.st_mode):
                path.chmod(0o700)
        pathlib.Path(current).chmod(0o700)
    shutil.rmtree(root)
"""


def cleanup_tree(path: str, timeout: int) -> bool:
    root = Path(path)
    if not root.is_absolute() or ".." in root.parts:
        return False
    return run_bounded(
        [sys.executable, "-I", "-S", "-c", TREE_PROGRAM, str(root)], timeout
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout-seconds", type=int, default=30)
    subparsers = parser.add_subparsers(dest="operation", required=True)
    session_parser = subparsers.add_parser("session")
    session_parser.add_argument("--clew", required=True)
    session_parser.add_argument("--session", required=True)
    tree_parser = subparsers.add_parser("tree")
    tree_parser.add_argument("--path", required=True)
    arguments = parser.parse_args()
    if not 1 <= arguments.timeout_seconds <= 300:
        return 2
    if arguments.operation == "session":
        success = cleanup_session(
            arguments.clew, arguments.session, arguments.timeout_seconds
        )
    else:
        success = cleanup_tree(arguments.path, arguments.timeout_seconds)
    return 0 if success else 1


if __name__ == "__main__":
    raise SystemExit(main())
