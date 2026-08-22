#!/usr/bin/env python3
"""Bounded, independently attempted cleanup for qualification gates."""

from __future__ import annotations

import argparse
import contextlib
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time


class CleanupInterrupted(BaseException):
    def __init__(self, signum: int):
        self.signum = signum
        super().__init__(f"cleanup interrupted by signal {signum}")


def process_group_exists(process_group: int) -> bool:
    try:
        os.killpg(process_group, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def terminate(process: subprocess.Popen[bytes]) -> None:
    process_group = process.pid
    try:
        os.killpg(process_group, signal.SIGTERM)
    except ProcessLookupError:
        return
    deadline = time.monotonic() + 5
    while process_group_exists(process_group) and time.monotonic() < deadline:
        process.poll()
        time.sleep(0.02)
    try:
        os.killpg(process_group, signal.SIGKILL)
    except ProcessLookupError:
        pass
    deadline = time.monotonic() + 5
    while process_group_exists(process_group) and time.monotonic() < deadline:
        process.poll()
        time.sleep(0.02)
    try:
        process.wait(timeout=1)
    except subprocess.TimeoutExpired:
        pass


@contextlib.contextmanager
def forward_cancellation_to(
    process: subprocess.Popen[bytes], previous_mask: set[signal.Signals] | None
):
    if threading.current_thread() is not threading.main_thread():
        yield
        return
    previous = {}

    def interrupted(signum: int, _frame: object) -> None:
        raise CleanupInterrupted(signum)

    mask_restored = previous_mask is None
    try:
        for signum in (signal.SIGINT, signal.SIGTERM):
            previous[signum] = signal.getsignal(signum)
            signal.signal(signum, interrupted)
        if previous_mask is not None:
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
            mask_restored = True
        yield
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)
        if previous_mask is not None and not mask_restored:
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)


def run_bounded(command: list[str], timeout: int) -> bool:
    previous_mask = None
    if (
        threading.current_thread() is threading.main_thread()
        and hasattr(signal, "pthread_sigmask")
    ):
        previous_mask = signal.pthread_sigmask(
            signal.SIG_BLOCK, {signal.SIGINT, signal.SIGTERM}
        )
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
        if previous_mask is not None:
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
        return False
    except BaseException:
        if previous_mask is not None:
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
        raise
    try:
        with forward_cancellation_to(process, previous_mask):
            return_code = process.wait(timeout=timeout)
            if process_group_exists(process.pid):
                terminate(process)
                return False
            return return_code == 0
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
import stat
import sys

root = pathlib.Path(sys.argv[1])
expected_identity = None
if len(sys.argv) == 4:
    try:
        expected_identity = (int(sys.argv[2]), int(sys.argv[3]))
    except ValueError:
        raise SystemExit('cleanup identity is invalid')
elif len(sys.argv) != 2:
    raise SystemExit('cleanup identity arguments are incomplete')
if (
    not root.is_absolute()
    or '..' in root.parts
    or pathlib.Path(os.path.normpath(root)) != root
    or root == pathlib.Path(root.anchor)
):
    raise SystemExit('cleanup root must be normalized and absolute')

flags = os.O_RDONLY | os.O_DIRECTORY | getattr(os, 'O_NOFOLLOW', 0)


def open_absolute_directory(path):
    descriptor = os.open(path.anchor, flags)
    try:
        for component in path.parts[1:]:
            child = os.open(component, flags, dir_fd=descriptor)
            metadata = os.fstat(child)
            mode = stat.S_IMODE(metadata.st_mode)
            unsafe_writable_ancestor = mode & 0o022 and not (
                metadata.st_uid == 0 and mode & stat.S_ISVTX
            )
            if (
                not stat.S_ISDIR(metadata.st_mode)
                or metadata.st_uid not in {0, os.geteuid()}
                or unsafe_writable_ancestor
            ):
                os.close(child)
                raise SystemExit('cleanup path contains an unsafe ancestor')
            os.close(descriptor)
            descriptor = child
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def remove_children(descriptor):
    os.fchmod(descriptor, 0o700)
    with os.scandir(descriptor) as entries:
        names = sorted(entry.name for entry in entries)
    for name in names:
        try:
            metadata = os.stat(name, dir_fd=descriptor, follow_symlinks=False)
        except FileNotFoundError:
            continue
        if stat.S_ISDIR(metadata.st_mode):
            child = os.open(name, flags, dir_fd=descriptor)
            try:
                observed = os.fstat(child)
                if (
                    not stat.S_ISDIR(observed.st_mode)
                    or (observed.st_dev, observed.st_ino)
                    != (metadata.st_dev, metadata.st_ino)
                ):
                    raise SystemExit('cleanup directory identity changed')
                remove_children(child)
            finally:
                os.close(child)
            current = os.stat(name, dir_fd=descriptor, follow_symlinks=False)
            if (current.st_dev, current.st_ino) != (metadata.st_dev, metadata.st_ino):
                raise SystemExit('cleanup directory binding changed')
            os.rmdir(name, dir_fd=descriptor)
        else:
            current = os.stat(name, dir_fd=descriptor, follow_symlinks=False)
            if (
                (current.st_dev, current.st_ino, current.st_mode)
                != (metadata.st_dev, metadata.st_ino, metadata.st_mode)
            ):
                raise SystemExit('cleanup entry binding changed')
            os.unlink(name, dir_fd=descriptor)


parent = open_absolute_directory(root.parent)
try:
    try:
        root_descriptor = os.open(root.name, flags, dir_fd=parent)
    except FileNotFoundError:
        raise SystemExit(0)
    try:
        metadata = os.fstat(root_descriptor)
        if (
            not stat.S_ISDIR(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or (
                expected_identity is not None
                and (metadata.st_dev, metadata.st_ino) != expected_identity
            )
        ):
            raise SystemExit('cleanup root is not an owned physical directory')
        remove_children(root_descriptor)
    finally:
        os.close(root_descriptor)
    current = os.stat(root.name, dir_fd=parent, follow_symlinks=False)
    if (current.st_dev, current.st_ino) != (metadata.st_dev, metadata.st_ino):
        raise SystemExit('cleanup root binding changed')
    os.rmdir(root.name, dir_fd=parent)
finally:
    os.close(parent)
"""


def cleanup_tree(
    path: str, timeout: int, expected_identity: tuple[int, int] | None = None
) -> bool:
    root = Path(path)
    if not root.is_absolute() or ".." in root.parts:
        return False
    command = [sys.executable, "-I", "-S", "-c", TREE_PROGRAM, str(root)]
    if expected_identity is not None:
        device, inode = expected_identity
        if type(device) is not int or type(inode) is not int or device < 0 or inode <= 0:
            return False
        command.extend([str(device), str(inode)])
    return run_bounded(command, timeout)


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
    try:
        raise SystemExit(main())
    except CleanupInterrupted as error:
        raise SystemExit(128 + error.signum)
