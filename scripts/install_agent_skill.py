#!/usr/bin/env python3
"""Install the bundled Codeclew Agent Skill atomically."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import stat
import sys
import uuid


SCHEMA = "codeclew-skill-install/1.0"
SKILL_NAME = "codeclew"
MAX_FILES = 64
MAX_TOTAL_BYTES = 1024 * 1024


class InstallError(RuntimeError):
    pass


def canonical(value: object) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)


def package_files(root: Path) -> list[tuple[str, bytes, int]]:
    if not root.is_dir() or root.is_symlink():
        raise InstallError("bundled Codeclew skill is unavailable")
    rows: list[tuple[str, bytes, int]] = []
    total = 0
    for path in sorted(root.rglob("*"), key=lambda value: value.as_posix()):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise InstallError("bundled Codeclew skill contains a symlink")
        if stat.S_ISDIR(metadata.st_mode):
            continue
        if not stat.S_ISREG(metadata.st_mode):
            raise InstallError("bundled Codeclew skill contains an unsupported entry")
        relative = path.relative_to(root).as_posix()
        content = path.read_bytes()
        total += len(content)
        rows.append((relative, content, 0o755 if metadata.st_mode & 0o111 else 0o644))
    if not rows or len(rows) > MAX_FILES or total > MAX_TOTAL_BYTES:
        raise InstallError("bundled Codeclew skill has an invalid size")
    if "SKILL.md" not in {relative for relative, _, _ in rows}:
        raise InstallError("bundled Codeclew skill has no SKILL.md")
    return rows


def digest(rows: list[tuple[str, bytes, int]]) -> str:
    value = hashlib.sha256()
    for relative, content, mode in rows:
        value.update(relative.encode("utf-8"))
        value.update(b"\0")
        value.update(str(mode).encode("ascii"))
        value.update(b"\0")
        value.update(content)
        value.update(b"\0")
    return "sha256:" + value.hexdigest()


def existing_files(root: Path) -> list[tuple[str, bytes, int]] | None:
    if not root.exists():
        return None
    if not root.is_dir() or root.is_symlink():
        raise InstallError(f"skill destination is not a safe directory: {root}")
    return package_files(root)


def write_stage(root: Path, rows: list[tuple[str, bytes, int]]) -> Path:
    stage = root / f".{SKILL_NAME}.install-{uuid.uuid4().hex}"
    stage.mkdir(mode=0o700)
    try:
        for relative, content, mode in rows:
            destination = stage / relative
            destination.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
            with destination.open("xb") as stream:
                stream.write(content)
            destination.chmod(mode)
        return stage
    except BaseException:
        shutil.rmtree(stage, ignore_errors=True)
        raise


def install_one(
    destination_root: Path, rows: list[tuple[str, bytes, int]], force: bool
) -> dict[str, str]:
    destination_root.mkdir(mode=0o755, parents=True, exist_ok=True)
    destination_root = destination_root.resolve(strict=True)
    if not destination_root.is_dir():
        raise InstallError(f"skill root is not a directory: {destination_root}")
    destination = destination_root / SKILL_NAME
    current = existing_files(destination)
    expected_digest = digest(rows)
    if current is not None and digest(current) == expected_digest:
        return {"path": str(destination), "status": "CURRENT"}
    if current is not None and not force:
        raise InstallError(
            f"a different Codeclew skill already exists at {destination}; use --force to replace it"
        )

    stage = write_stage(destination_root, rows)
    backup: Path | None = None
    try:
        if current is not None:
            backup = destination_root / f".{SKILL_NAME}.backup-{uuid.uuid4().hex}"
            destination.rename(backup)
        stage.rename(destination)
    except BaseException:
        if backup is not None and backup.exists() and not destination.exists():
            backup.rename(destination)
        shutil.rmtree(stage, ignore_errors=True)
        raise
    if backup is not None:
        shutil.rmtree(backup)
    return {
        "path": str(destination),
        "status": "REPLACED" if current is not None else "INSTALLED",
    }


def destinations(arguments: argparse.Namespace) -> list[tuple[str, Path]]:
    if arguments.destination is not None:
        if arguments.project is not None or arguments.agent != "all":
            raise InstallError("--destination cannot be combined with --project or --agent")
        return [("agent-skills", arguments.destination.expanduser())]

    if arguments.project is None:
        base = Path.home()
        roots = {
            "codex": base / ".agents" / "skills",
            "claude": base / ".claude" / "skills",
        }
    else:
        project = arguments.project.expanduser().resolve(strict=True)
        if not project.is_dir():
            raise InstallError("--project must name an existing directory")
        roots = {
            "codex": project / ".agents" / "skills",
            "claude": project / ".claude" / "skills",
        }
    selected = ["codex", "claude"] if arguments.agent == "all" else [arguments.agent]
    return [(agent, roots[agent]) for agent in selected]


def parse_arguments(arguments: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(prog="clew skill")
    commands = parser.add_subparsers(dest="command", required=True)
    install = commands.add_parser("install", help="install the bundled Codeclew skill")
    install.add_argument(
        "--agent", choices=("all", "codex", "claude"), default="all"
    )
    install.add_argument(
        "--project", type=Path, help="install into an existing project instead of user scope"
    )
    install.add_argument(
        "--destination",
        type=Path,
        help="install into an Agent Skills root used by another compatible agent",
    )
    install.add_argument(
        "--force", action="store_true", help="replace a different existing Codeclew skill"
    )
    return parser.parse_args(arguments)


def main(arguments: list[str]) -> int:
    try:
        parsed = parse_arguments(arguments)
        source = Path(__file__).resolve().parent.parent / "skills" / SKILL_NAME
        rows = package_files(source)
        installed = [
            {"agent": agent, **install_one(root, rows, parsed.force)}
            for agent, root in destinations(parsed)
        ]
        print(
            canonical(
                {
                    "digest": digest(rows),
                    "installations": installed,
                    "schema": SCHEMA,
                    "status": "INSTALLED",
                }
            )
        )
        return 0
    except (InstallError, OSError) as error:
        print(f"clew skill: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
