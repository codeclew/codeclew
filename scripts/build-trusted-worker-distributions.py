#!/usr/bin/env python3
"""Build and record deterministic trusted Kotlin worker distributions."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import shutil
import argparse
from contextlib import nullcontext


ROOT = Path(__file__).resolve().parents[1]
VARIANTS = (
    ("kotlin21", ":workers:kotlin21:installDist", "workers/kotlin21/build/install/kotlin21"),
    ("kotlin23", ":workers:kotlin23:installDist", "workers/kotlin23/build/install/kotlin23"),
    ("kotlin24", ":workers:kotlin:installDist", "workers/kotlin/build/install/kotlin"),
)
MANIFEST_ROOT = ROOT / "workers" / "manifests"
INJECTION_ENV = {
    "GRADLE_OPTS",
    "GRADLE_USER_HOME",
    "JAVA_TOOL_OPTIONS",
    "JDK_JAVA_OPTIONS",
    "JAVA_OPTS",
    "_JAVA_OPTIONS",
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return "sha256:" + digest.hexdigest()


def distribution_files(root: Path) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    for path in sorted(root.rglob("*")):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not (
            stat.S_ISDIR(metadata.st_mode) or stat.S_ISREG(metadata.st_mode)
        ):
            raise RuntimeError(f"unsupported distribution entry: {path}")
        if stat.S_ISREG(metadata.st_mode):
            rows.append(
                {
                    "path": path.relative_to(root).as_posix(),
                    "size": metadata.st_size,
                    "sha256": sha256(path),
                }
            )
    return rows


def tree_hash(rows: list[dict[str, object]]) -> str:
    digest = hashlib.sha256()
    for row in rows:
        digest.update(str(row["path"]).encode())
        digest.update(b"\0")
        digest.update(str(row["size"]).encode())
        digest.update(b"\0")
        digest.update(str(row["sha256"]).encode())
        digest.update(b"\0")
    return "sha256:" + digest.hexdigest()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--variant",
        action="append",
        choices=[variant for variant, _, _ in VARIANTS],
        help="build only this worker variant; may be repeated",
    )
    parser.add_argument("--offline", action="store_true")
    parser.add_argument(
        "--gradle-user-home",
        type=Path,
        help="use an existing Gradle cache instead of a fresh temporary one",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    selected = tuple(
        row for row in VARIANTS if not args.variant or row[0] in set(args.variant)
    )
    MANIFEST_ROOT.parent.mkdir(parents=True, exist_ok=True)
    gradle_home_context = (
        nullcontext(args.gradle_user_home.resolve())
        if args.gradle_user_home is not None
        else tempfile.TemporaryDirectory(prefix="codeclew-authority-gradle-")
    )
    with tempfile.TemporaryDirectory(
        prefix=".trusted-worker-manifests-", dir=MANIFEST_ROOT.parent
    ) as staged_manifest_root, gradle_home_context as gradle_home:
        staged_manifest_root = Path(staged_manifest_root)
        gradle_home = Path(gradle_home)
        if args.variant and MANIFEST_ROOT.exists():
            for manifest in MANIFEST_ROOT.glob("*.json"):
                shutil.copy2(manifest, staged_manifest_root / manifest.name)
        environment = {
            key: value
            for key, value in os.environ.items()
            if key not in INJECTION_ENV and not key.startswith("ORG_GRADLE_PROJECT_")
        }
        environment["GRADLE_USER_HOME"] = str(gradle_home)
        for variant, task, relative_distribution in selected:
            command = [
                str(ROOT / "gradlew"),
                task,
                "--rerun-tasks",
                "--no-daemon",
                "--quiet",
            ]
            if args.offline:
                command.append("--offline")
            subprocess.run(
                command,
                cwd=ROOT,
                env=environment,
                check=True,
            )
            distribution = ROOT / relative_distribution
            rows = distribution_files(distribution)
            manifest = {
                "schema": "trusted-worker-distribution/0.1",
                "variant": variant,
                "installTask": task,
                "files": rows,
                "treeHash": tree_hash(rows),
            }
            encoded = json.dumps(
                manifest, ensure_ascii=True, separators=(",", ":"), sort_keys=True
            ) + "\n"
            (staged_manifest_root / f"{variant}.json").write_text(
                encoded, encoding="utf-8"
            )
        expected = {f"{variant}.json" for variant, _, _ in VARIANTS}
        if {path.name for path in staged_manifest_root.iterdir()} != expected:
            raise RuntimeError("staged trusted-worker manifest set is incomplete")
        backup = MANIFEST_ROOT.parent / ".trusted-worker-manifests-backup"
        if backup.exists():
            raise RuntimeError(f"stale trusted-worker manifest backup exists: {backup}")
        if MANIFEST_ROOT.exists():
            os.replace(MANIFEST_ROOT, backup)
        try:
            os.replace(staged_manifest_root, MANIFEST_ROOT)
        except BaseException:
            if backup.exists() and not MANIFEST_ROOT.exists():
                os.replace(backup, MANIFEST_ROOT)
            raise
        if backup.exists():
            shutil.rmtree(backup)


if __name__ == "__main__":
    main()
