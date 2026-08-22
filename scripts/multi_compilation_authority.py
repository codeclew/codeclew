#!/usr/bin/env python3
"""Small executable authority checks shared by Q2 gate and unit tests."""

from __future__ import annotations

from pathlib import Path
import re


class WorkspaceAuthorityError(RuntimeError):
    pass


def require_session_authority(profile: object, expected: str) -> None:
    if (
        not isinstance(profile, dict)
        or re.fullmatch(r"sha256:[0-9a-f]{64}", expected) is None
        or profile.get("sessionAuthorityDigest") != expected
    ):
        raise WorkspaceAuthorityError("workspace profile is bound to another session")


def refuse_copied_runtime(state: Path) -> None:
    runtime_root = state / "v2" / "runtimes"
    try:
        entries = list(runtime_root.iterdir())
    except FileNotFoundError:
        return
    for path in entries:
        if (
            path.is_dir()
            and re.fullmatch(r"[0-9a-f]{64}", path.name) is not None
        ):
            raise WorkspaceAuthorityError("sealed runtime was copied into trial state")
