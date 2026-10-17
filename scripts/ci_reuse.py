#!/usr/bin/env python3
"""Reuse a completed full CI run only for the identical promoted commit."""

from datetime import datetime
import json
import os
from pathlib import Path
import subprocess


def reusable_run(payload, revision, repository, repository_id, current_run):
    if not isinstance(payload, dict) or not isinstance(payload.get("workflow_runs"), list):
        return None
    candidates = []
    for run in payload["workflow_runs"]:
        if not isinstance(run, dict):
            return None
        if run.get("id") == current_run:
            continue
        if not isinstance(run.get("head_sha"), str):
            return None
        if run["head_sha"] != revision:
            continue
        try:
            created = datetime.fromisoformat(run["created_at"].replace("Z", "+00:00"))
            if created.tzinfo is None or type(run["id"]) is not int or run["id"] <= 0:
                return None
        except (KeyError, TypeError, ValueError, AttributeError):
            return None
        candidates.append((created, run["id"], run))
    if not candidates:
        return None
    latest = max(candidates, key=lambda item: item[:2])[2]
    head = latest.get("head_repository")
    # PR and dispatch events always run the full matrix. Push events may reuse
    # a result, so they must never become the authority for another reuse.
    if (
        latest.get("path") != ".github/workflows/ci.yml"
        or not isinstance(head, dict)
        or head.get("id") != repository_id
        or head.get("full_name") != repository
        or latest.get("event") not in ("pull_request", "workflow_dispatch")
        or latest.get("status") != "completed"
        or latest.get("conclusion") != "success"
    ):
        return None
    return latest


def main():
    selected = None
    if os.environ["GITHUB_EVENT_NAME"] == "push":
        revision = os.environ["GITHUB_SHA"]
        repository = os.environ["GITHUB_REPOSITORY"]
        try:
            result = subprocess.run(
                [
                    "gh", "api",
                    f"repos/{repository}/actions/workflows/ci.yml/runs?head_sha={revision}&per_page=100",
                ],
                check=True, capture_output=True, text=True, timeout=30,
            )
            selected = reusable_run(
                json.loads(result.stdout), revision, repository,
                int(os.environ["CLEW_CI_REPOSITORY_ID"]), int(os.environ["GITHUB_RUN_ID"]),
            )
        except (subprocess.SubprocessError, OSError, ValueError):
            print("CI reuse lookup unavailable; running the full verification matrix.")
    reuse = "true" if selected else "false"
    with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
        output.write(f"reuse_full_ci={reuse}\n")
        output.write(f"source_run={selected['id'] if selected else ''}\n")
    if selected:
        server = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
        url = f"{server}/{os.environ['GITHUB_REPOSITORY']}/actions/runs/{selected['id']}"
        message = f"Full CI already passed for exact commit `{os.environ['GITHUB_SHA']}`: [run {selected['id']}]({url}).\n"
        print(message)
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as summary:
                summary.write(message)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
