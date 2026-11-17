#!/usr/bin/env python3
"""Create a runner-local version tag for an unpublished, exact-commit build."""

import argparse
import re
import subprocess


def prepare(version: str, commit: str, expected_existing_tag_commit: str | None = None) -> None:
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ValueError("candidate version must be vMAJOR.MINOR.PATCH")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("candidate commit must be a full Git SHA")
    if expected_existing_tag_commit is not None and not re.fullmatch(
        r"[0-9a-f]{40}", expected_existing_tag_commit
    ):
        raise ValueError("expected existing tag commit must be a full Git SHA")
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if head != commit:
        raise ValueError("candidate checkout does not match the workflow commit")
    if subprocess.check_output(["git", "status", "--porcelain"], text=True):
        raise ValueError("candidate checkout contains uncommitted changes")
    existing = subprocess.run(
        ["git", "show-ref", "--verify", "--quiet", "refs/tags/" + version],
        check=False,
    )
    if existing.returncode == 0:
        if expected_existing_tag_commit is None:
            raise ValueError("candidate version is already tagged; never replace a release tag")
        local = subprocess.check_output(
            ["git", "rev-parse", "refs/tags/" + version + "^{}"], text=True
        ).strip()
        remote = subprocess.check_output(
            ["git", "ls-remote", "--tags", "origin", "refs/tags/" + version,
             "refs/tags/" + version + "^{}"], text=True
        )
        refs = dict(line.split()[::-1] for line in remote.splitlines())
        remote_commit = refs.get("refs/tags/" + version + "^{}", refs.get("refs/tags/" + version))
        if local != expected_existing_tag_commit or remote_commit != expected_existing_tag_commit:
            raise ValueError("existing local/origin tag does not match the expected correction base")
        # This only changes the disposable build checkout. Build jobs have
        # contents:read and this helper never pushes or publishes anything.
        subprocess.run(["git", "tag", "--force", version, commit], check=True)
    elif existing.returncode != 1:
        raise ValueError("could not verify candidate tag availability")
    elif expected_existing_tag_commit is not None:
        raise ValueError("expected correction tag is missing from the candidate checkout")
    else:
        subprocess.run(["git", "tag", version, commit], check=True)
    print(f"Runner-local candidate {version} -> {commit}; no tag or release published")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--expected-existing-tag-commit")
    args = parser.parse_args()
    try:
        prepare(args.version, args.commit, args.expected_existing_tag_commit)
    except ValueError as error:
        parser.exit(1, str(error) + "\n")
