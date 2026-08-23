#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import hashlib
import hmac
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


MODULE_PATH = Path(__file__).resolve().with_name("pilot_case_record.py")
SPEC = importlib.util.spec_from_file_location("codeclew_pilot_case_record", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
recorder = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = recorder
SPEC.loader.exec_module(recorder)

BASE = "a" * 40
FINAL = "b" * 40
SESSION = "session:test"
RUN = "run:test"
AUTHORITY = "sha256:" + "c" * 64
REPOSITORY_KEY = "c" * 64


def run(status: str, run_id: str = RUN, final: str | None = None) -> dict[str, object]:
    value: dict[str, object] = {
        "contextId": "context:test",
        "ledgerHead": "sha256:" + "1" * 64,
        "planId": "plan:test",
        "requestDigest": "sha256:" + "d" * 64,
        "runId": run_id,
        "sessionId": SESSION,
        "sequence": 1,
        "status": status,
        "transactionId": "tx:test",
    }
    if final is not None:
        value["finalCommit"] = final
    return value


def evidence() -> dict[str, object]:
    return {
        "opened": {
            "context": {
                "context": {"compilerVersions": {"main": "2.4.10"}},
                "contextId": "context:test",
                "schema": "codeclew-context-result/2.0",
                "sessionId": SESSION,
            },
            "schema": "codeclew-change-open/1.0",
            "session": {
                "baseRevision": BASE,
                "compilations": ["main"],
                "modelCachePolicy": "NON_CACHEABLE",
                "runtimeMode": "RELEASE",
                "repositoryKey": REPOSITORY_KEY,
                "sessionId": SESSION,
                "targetOid": BASE,
                "targetRef": "refs/heads/main",
            },
        },
        "prepared_first": {
            "contextId": "context:test",
            "planId": "plan:test",
            "schema": "codeclew-change-prepare/1.0",
            "sessionId": SESSION,
            "run": run("CREATED"),
        },
        "prepared_retry": {
            "contextId": "context:test",
            "planId": "plan:test",
            "schema": "codeclew-change-prepare/1.0",
            "sessionId": SESSION,
            "run": run("PREPARING"),
        },
        "terminal": {
            "candidate": {
                "candidateCommit": FINAL,
                "preparedAuthorityDigest": "sha256:" + "e" * 64,
                "validationEvidence": [{"launcher": "GRADLE", "success": True}],
            },
            "run": {
                **run("READY_TO_PUBLISH"),
                "candidateCommit": FINAL,
                "preparedAuthorityDigest": "sha256:" + "e" * 64,
            },
            "schema": "codeclew-change-status/1.0",
        },
        "published_first": {
            "run": {
                **run("PUBLISHED", final=FINAL),
                "candidateCommit": FINAL,
                "preparedAuthorityDigest": "sha256:" + "e" * 64,
            },
            "schema": "codeclew-change-publish/1.0",
        },
        "published_retry": {
            "run": {
                **run("PUBLISHED", final=FINAL),
                "candidateCommit": FINAL,
                "preparedAuthorityDigest": "sha256:" + "e" * 64,
            },
            "schema": "codeclew-change-publish/1.0",
        },
        "durations": {"open": 1, "prepareToReady": 2, "publish": 1, "total": 4},
        "prepublish_snapshot": {
            "baseRevision": BASE,
            "buildSystem": "GRADLE_WRAPPER",
            "candidateCommit": FINAL,
            "clean": True,
            "contextId": "context:test",
            "head": BASE,
            "ledgerHead": "sha256:" + "1" * 64,
            "planId": "plan:test",
            "preparedAuthorityDigest": "sha256:" + "e" * 64,
            "repoAuthority": AUTHORITY,
            "repositoryKey": REPOSITORY_KEY,
            "physicalRepositoryKey": REPOSITORY_KEY,
            "requestDigest": "sha256:" + "d" * 64,
            "runId": RUN,
            "runSequence": 1,
            "schema": "codeclew-pilot-source-snapshot/1.0",
            "sessionId": SESSION,
            "targetOid": BASE,
            "targetRef": "refs/heads/main",
            "transactionId": "tx:test",
        },
        "repository_facts": {
            "buildSystem": "GRADLE_WRAPPER",
            "clean": True,
            "commitCount": 1,
            "head": FINAL,
            "repoAuthority": AUTHORITY,
            "repositoryKey": REPOSITORY_KEY,
            "targetOid": FINAL,
            "targetRef": "refs/heads/main",
        },
    }


def derive(**changes: object) -> dict[str, object]:
    values = evidence()
    values.update(changes)
    return recorder.derive_case(
        case_id="case-01",
        manual_cleanup_used=False,
        private_data_leak=False,
        **values,
    )


class PilotCaseRecordTest(unittest.TestCase):
    def test_published_case_is_derived_from_bound_authority(self) -> None:
        case = derive()
        self.assertEqual(case["outcome"], "PUBLISHED")
        self.assertTrue(case["idempotentRetry"])
        self.assertTrue(case["preparedWithoutManualCleanup"])
        self.assertTrue(case["sourcePreservedBeforePublish"])
        self.assertTrue(case["validationPassed"])

    def test_retry_identity_mismatch_is_measured(self) -> None:
        values = evidence()
        values["prepared_retry"] = {
            "contextId": "context:test",
            "planId": "plan:test",
            "schema": "codeclew-change-prepare/1.0",
            "sessionId": SESSION,
            "run": run("PREPARING", "run:different"),
        }
        self.assertFalse(derive(**values)["idempotentRetry"])

    def test_prepublish_source_mutation_is_not_reconstructed_after_publish(self) -> None:
        values = evidence()
        snapshot = dict(values["prepublish_snapshot"])
        snapshot["clean"] = False
        values["prepublish_snapshot"] = snapshot
        self.assertFalse(derive(**values)["sourcePreservedBeforePublish"])

    def test_post_publish_state_must_match_managed_outcome(self) -> None:
        values = evidence()
        facts = dict(values["repository_facts"])
        facts["targetOid"] = BASE
        values["repository_facts"] = facts
        with self.assertRaises(recorder.RecordError):
            derive(**values)

    def test_forged_repository_or_context_authority_is_invalid(self) -> None:
        values = evidence()
        opened = dict(values["opened"])
        session = dict(opened["session"])
        session["repositoryKey"] = "f" * 64
        opened["session"] = session
        values["opened"] = opened
        with self.assertRaises(recorder.RecordError):
            derive(**values)
        values = evidence()
        context = dict(values["opened"]["context"])
        context["contextId"] = "context:other"
        opened = dict(values["opened"])
        opened["context"] = context
        values["opened"] = opened
        with self.assertRaises(recorder.RecordError):
            derive(**values)

    def test_typed_failure_can_be_recorded_without_candidate_authority(self) -> None:
        values = evidence()
        failed_run = run("FAILED")
        failed_run["failure"] = {"code": "COMPILE_FAILED"}
        values["terminal"] = {
            "candidate": None,
            "run": failed_run,
            "schema": "codeclew-change-status/1.0",
        }
        snapshot = dict(values["prepublish_snapshot"])
        snapshot["candidateCommit"] = None
        snapshot["preparedAuthorityDigest"] = None
        values["prepublish_snapshot"] = snapshot
        values["published_first"] = None
        values["published_retry"] = None
        values["repository_facts"] = {
            **values["repository_facts"],
            "commitCount": 0,
            "head": BASE,
            "targetOid": BASE,
        }
        values["durations"] = {
            "open": 1, "prepareToReady": 2, "publish": 0, "total": 3,
        }
        case = derive(**values)
        self.assertEqual(case["outcome"], "FAILED")
        self.assertEqual(case["errorCode"], "COMPILE_FAILED")

    def test_private_files_are_new_external_and_secret_scan_is_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / "case.json"
            digest = recorder.write_private_record(path, {"ok": True})
            self.assertTrue(digest.startswith("sha256:"))
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(recorder.read_private_json(path)[0], {"ok": True})
            with self.assertRaises(recorder.RecordError):
                recorder.write_private_record(path, {"ok": True})
            key_path = Path(directory).resolve() / "pilot.key"
            recorder.write_private_record(key_path, {
                "keyHex": (b"k" * 32).hex(),
                "schema": "codeclew-pilot-attestation-key/1.0",
            })
            self.assertEqual(recorder.read_private_key(key_path), b"k" * 32)
        self.assertTrue(
            recorder.artifact_leaks(
                [b'authorization: Bearer abcdefghijklmnop'], Path("/tmp/repo")
            )
        )
        for private_path in [b'"/tmp/pilot/file"', b'"/private/var/folders/x"', b'"/workspace/team/repo"']:
            self.assertTrue(recorder.artifact_leaks([private_path], Path("/safe/repo")))

    def test_snapshot_uses_physical_git_authority_without_a_path(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repository = (Path(directory) / "repo").resolve()
            subprocess.run(["git", "init", "-q", str(repository)], check=True)
            wrapper = repository / "gradlew"
            wrapper.touch(mode=0o755)
            subprocess.run(["git", "add", "gradlew"], cwd=repository, check=True)
            subprocess.run(
                [
                    "git", "-c", "user.name=Codeclew Maintainers",
                    "-c", "user.email=maintainers@codeclew.invalid",
                    "commit", "--allow-empty", "-qm", "base",
                ],
                cwd=repository,
                check=True,
            )
            base = subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=repository, text=True
            ).strip()
            facts = recorder.repository_facts(repository, "HEAD", base)
            snapshot = recorder.source_snapshot(facts, base, {"runId": RUN})
            self.assertEqual(snapshot["head"], base)
            self.assertTrue(str(snapshot["targetRef"]).startswith("refs/heads/"))
            self.assertNotIn(str(repository), str(snapshot))

    def test_attestation_binds_case_and_evidence(self) -> None:
        key = b"k" * 32
        case = recorder.attest_case(derive(), key, "sha256:" + "f" * 64)
        unsigned = dict(case)
        signature = str(unsigned.pop("attestation"))
        self.assertEqual(case["pilotId"], recorder.pilot_id(key))
        self.assertTrue(
            hmac.compare_digest(
                signature,
                "hmac-sha256:" + hmac.new(
                    key, recorder.canonical(unsigned), hashlib.sha256
                ).hexdigest(),
            )
        )


if __name__ == "__main__":
    unittest.main()
