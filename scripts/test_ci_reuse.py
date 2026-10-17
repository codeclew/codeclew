#!/usr/bin/env python3
"""Check exact-commit CI reuse and conservative fallback."""

import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location("ci_reuse", Path(__file__).with_name("ci_reuse.py"))
CI = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CI)
REVISION = "a" * 40
REPOSITORY = "example/project"


def run_record(**changes):
    run = {
        "id": 10, "head_sha": REVISION, "path": ".github/workflows/ci.yml",
        "created_at": "2026-10-17T20:00:00Z",
        "head_repository": {"id": 7, "full_name": REPOSITORY},
        "event": "pull_request", "status": "completed", "conclusion": "success",
    }
    run.update(changes)
    return run


def select(*runs):
    return CI.reusable_run({"workflow_runs": list(runs)}, REVISION, REPOSITORY, 7, 99)


class ReusePolicyTests(unittest.TestCase):
    def test_completed_pr_and_manual_full_runs_are_eligible(self):
        for event in ("pull_request", "workflow_dispatch"):
            self.assertEqual(select(run_record(event=event))["id"], 10)

    def test_other_revision_or_repository_cannot_authorize_reuse(self):
        for change in (
            {"head_sha": "b" * 40, "tree_sha": "same-tree"},
            {"head_repository": {"id": 8, "full_name": REPOSITORY}},
            {"head_repository": {"id": 7, "full_name": "fork/project"}},
            {"path": ".github/workflows/partial.yml"},
        ):
            with self.subTest(change=change):
                self.assertIsNone(select(run_record(**change)))

    def test_latest_failed_cancelled_or_running_attempt_blocks_old_success(self):
        for status, conclusion in (("completed", "failure"), ("completed", "cancelled"), ("in_progress", None)):
            self.assertIsNone(select(
                run_record(),
                run_record(id=11, created_at="2026-10-17T21:00:00Z", status=status, conclusion=conclusion),
            ))

    def test_push_results_cannot_form_a_reuse_chain(self):
        self.assertIsNone(select(run_record(event="push")))
        self.assertIsNone(select(
            run_record(), run_record(id=11, event="push", created_at="2026-10-17T21:00:00Z"),
        ))

    def test_current_run_is_excluded(self):
        self.assertEqual(select(run_record(), run_record(id=99, status="in_progress"))["id"], 10)

    def test_newest_unproven_binding_does_not_fall_back_to_old_success(self):
        self.assertIsNone(select(
            run_record(), run_record(id=11, created_at="2026-10-17T21:00:00Z", head_repository=None),
        ))

    def test_invalid_metadata_does_not_authorize_reuse(self):
        for payload in (None, [], {}, {"workflow_runs": None}, {"workflow_runs": [None]}):
            self.assertIsNone(CI.reusable_run(payload, REVISION, REPOSITORY, 7, 99))
        for change in ({"created_at": "invalid"}, {"created_at": "2026-10-17T20:00:00"}, {"id": True}):
            self.assertIsNone(select(run_record(**change)))


class ReuseCommandTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="clew-ci-reuse-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.environment = {
            "GITHUB_EVENT_NAME": "push", "GITHUB_SHA": REVISION,
            "GITHUB_REPOSITORY": REPOSITORY, "CLEW_CI_REPOSITORY_ID": "7",
            "GITHUB_RUN_ID": "99", "GITHUB_OUTPUT": str(self.root / "output"),
            "GITHUB_STEP_SUMMARY": str(self.root / "summary"),
            "GITHUB_SERVER_URL": "https://github.com",
        }

    def test_pr_always_runs_full_verification_without_lookup(self):
        environment = copy.copy(self.environment)
        environment["GITHUB_EVENT_NAME"] = "pull_request"
        with mock.patch.dict(os.environ, environment), mock.patch.object(CI.subprocess, "run") as lookup:
            self.assertEqual(CI.main(), 0)
        lookup.assert_not_called()
        self.assertIn("reuse_full_ci=false", (self.root / "output").read_text())

    def test_api_failure_keeps_full_verification(self):
        with mock.patch.dict(os.environ, self.environment), mock.patch.object(
            CI.subprocess, "run", side_effect=subprocess.CalledProcessError(1, ["gh"]),
        ):
            self.assertEqual(CI.main(), 0)
        self.assertIn("reuse_full_ci=false", (self.root / "output").read_text())

    def test_success_records_its_exact_commit_and_source_run(self):
        response = subprocess.CompletedProcess([], 0, json.dumps({"workflow_runs": [run_record()]}))
        with mock.patch.dict(os.environ, self.environment), mock.patch.object(
            CI.subprocess, "run", return_value=response,
        ) as lookup:
            self.assertEqual(CI.main(), 0)
        query = lookup.call_args.args[0][-1]
        self.assertIn("head_sha=" + REVISION, query)
        self.assertNotIn("status=success", query)
        self.assertIn("reuse_full_ci=true", (self.root / "output").read_text())
        self.assertIn("source_run=10", (self.root / "output").read_text())
        self.assertIn(REVISION, (self.root / "summary").read_text())


if __name__ == "__main__":
    unittest.main()
