#!/usr/bin/env python3
"""Create a runner-local version tag for an unpublished, exact-commit build."""

import argparse
import re
import subprocess


def prepare(version: str, commit: str) -> None:
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ValueError("candidate version must be vMAJOR.MINOR.PATCH")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("candidate commit must be a full Git SHA")
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if head != commit:
        raise ValueError("candidate checkout does not match the workflow commit")
    existing = subprocess.run(
        ["git", "show-ref", "--verify", "--quiet", "refs/tags/" + version],
        check=False,
    )
    if existing.returncode == 0:
        raise ValueError("candidate version is already tagged; never replace a release tag")
    if existing.returncode != 1:
        raise ValueError("could not verify candidate tag availability")
    if subprocess.check_output(["git", "status", "--porcelain"], text=True):
        raise ValueError("candidate checkout contains uncommitted changes")
    subprocess.run(["git", "tag", version, commit], check=True)
    print(f"Runner-local candidate {version} -> {commit}; no tag or release published")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    args = parser.parse_args()
    try:
        prepare(args.version, args.commit)
    except ValueError as error:
        parser.exit(1, str(error) + "\n")
