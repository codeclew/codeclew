#!/usr/bin/env python3
"""Deterministic CI/transport contract checks; no external messages or models."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("documentation_ci", Path(__file__).with_name("documentation_ci.py"))
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)


def event():
    return {"schema": "codeclew-documentation-update-event/1.0", "id": "orders-two", "service": "orders",
            "repositoryId": "orders", "revision": "a"*40, "sourceRef": "refs/heads/main", "sequence": 2}


class Contract(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.config = {"schema": "codeclew-documentation-ci-installation/1.0", "clewCommand": [sys.executable],
                       "docsRoot": str(self.root), "artifactRoot": str(self.root), "audience": "fixture maintainers"}
        self.calls = []

    def job(self, kind="update"):
        job = {"schema": "codeclew-documentation-ci-job/1.0", "id": "fixture", "kind": kind}
        if kind in ("capture", "update"):
            job["event"] = event()
        if kind == "update":
            job["artifact"] = {"path": "packages/fixture", "manifestDigest": "sha256:"+"b"*64}
        if kind == "reconcile":
            job["events"] = [event()]
        return job

    def fake_clew(self, argv, *args, **kwargs):
        self.calls.append(argv)
        command = argv[2:4]
        if command == ["evidence", "inspect"]:
            return {"service": "orders", "repositoryId": "orders", "revision": "a"*40,
                    "manifestDigest": "sha256:"+"b"*64, "serviceDigest": "sha256:"+"c"*64, "compatible": True}
        if command == ["update", "status"]:
            return {"inputDigest": "sha256:"+"d"*64, "targets": {"orders": event()}}
        if command == ["update", "run"]:
            return {"status": "AGENT_CONFIGURATION_REQUIRED", "pendingCount": 1, "attempted": 0}
        return {"status": "RECORDED"}

    def test_local_and_gitlab_shaped_jobs_share_contract_and_replay(self):
        with patch.object(ci, "command", side_effect=self.fake_clew):
            first = ci.run_job(self.config, self.job())
            self.assertEqual(first["status"], "COMPLETED")
            count = len(self.calls)
            again = ci.run_job(self.config, self.job())
            self.assertTrue(again["replayed"])
            self.assertEqual(count, len(self.calls))
            self.assertEqual(first["targets"]["orders"], event())
            self.assertEqual(first["refresh"]["status"], "AGENT_CONFIGURATION_REQUIRED")
            commands = [v[2:4] for v in self.calls]
            self.assertLess(commands.index(["update", "enqueue"]), commands.index(["evidence", "inspect"]))
            bad = self.job(); bad["event"]["sequence"] = 3
            with self.assertRaisesRegex(ci.Gap, "JOB_ID_REUSED"):
                ci.run_job(self.config, bad)

    def test_capture_checks_exact_commit_and_reconciliation_uses_same_events(self):
        with patch.object(ci, "command", side_effect=self.fake_clew):
            capture = ci.run_job(self.config, self.job("capture"))
            self.assertEqual(capture["artifact"]["manifestDigest"], "sha256:"+"b"*64)
            job = self.job("reconcile"); job["id"] = "reconciled"
            self.assertEqual(ci.run_job(self.config, job)["status"], "COMPLETED")
            revision_set = ci.read(self.root / "reconciliations/reconciled.json")
            self.assertEqual(revision_set["events"], [event()])
            wrong = self.job("capture"); wrong["id"] = "wrong"; wrong["event"]["id"] = "wrong"; wrong["event"]["revision"] = "f"*40
            self.assertEqual(ci.run_job(self.config, wrong)["reason"], "ARTIFACT_TARGET_MISMATCH")

    def test_expired_artifact_preserves_accepted_target_and_retries(self):
        def failed(argv, *args, **kwargs):
            if argv[2:4] == ["evidence", "inspect"]:
                raise ci.Gap("PROCESS_FAILED")
            return self.fake_clew(argv)
        with patch.object(ci, "command", side_effect=failed):
            result = ci.run_job(self.config, self.job())
        self.assertEqual(result["status"], "INTEGRATION_GAP")
        self.assertIn(["update", "enqueue"], [c[2:4] for c in self.calls])
        with patch.object(ci, "command", side_effect=self.fake_clew):
            self.assertEqual(ci.run_job(self.config, self.job())["status"], "COMPLETED")

    def test_paths_source_instructions_and_forged_trust_never_select_commands(self):
        for name in ("../escape", "/absolute", "packages/../escape", "packages\\escape"):
            with self.subTest(name=name), self.assertRaises(ci.Gap):
                ci.relative(self.root, name)
        (self.root / "link").symlink_to(self.root)
        with self.assertRaisesRegex(ci.Gap, "SYMLINK"):
            ci.relative(self.root, "link/result.json")
        job = self.job(); job["command"] = ["untrusted-source-command"]
        with self.assertRaises(ci.Gap):
            ci.run_job(self.config, job)
        job = self.job(); job["artifact"]["manifestDigest"] = "sha256:"+"e"*64
        with patch.object(ci, "command", side_effect=self.fake_clew):
            self.assertEqual(ci.run_job(self.config, job)["reason"], "ARTIFACT_DIGEST_MISMATCH")

    def test_simultaneous_coordinator_requires_retry(self):
        with ci.coordinator_lock(self.root):
            with self.assertRaisesRegex(ci.Gap, "COORDINATOR_BUSY_RETRY"):
                ci.run_job(self.config, self.job())

    def test_publication_requires_credentials_audience_and_retained_result(self):
        with self.assertRaisesRegex(ci.Gap, "CONFIGURATION_REQUIRED"):
            ci.publish(self.config, {})
        self.config["publication"] = {"command": [sys.executable], "credentialEnvironment": ["DOCS_TEST_PUBLISH"], "audience": "fixture maintainers"}
        with patch.dict(os.environ, {}, clear=True), self.assertRaisesRegex(ci.Gap, "CREDENTIALS_UNAVAILABLE"):
            ci.publish(self.config, {})
        self.config["publication"]["audience"] = "public"
        with self.assertRaisesRegex(ci.Gap, "AUDIENCE_MISMATCH"):
            ci.publish(self.config, {})

    def test_process_failure_malformed_output_timeout_and_byte_cap(self):
        for code, expected in [("raise SystemExit(1)", "PROCESS_FAILED"), ("print('malformed')", "MALFORMED_JSON"),
                               ("import time;time.sleep(5)", "PROCESS_TIMEOUT"),
                               ("print('x'*3000000)", "PROCESS_OUTPUT_LIMIT")]:
            with self.subTest(expected=expected), self.assertRaisesRegex(ci.Gap, expected):
                ci.command([sys.executable, "-I", "-S", "-c", code], timeout=0.5)
        self.assertEqual(ci.command([sys.executable, "-c", "import json,sys;print(json.dumps(json.load(sys.stdin)))"], {"literal":"$(no-command)"}), {"literal":"$(no-command)"})

    def test_publication_checks_new_targets_before_dispatch_and_bounds_envelope(self):
        with patch.object(ci, "command", side_effect=self.fake_clew):
            result = ci.run_job(self.config, self.job())
        self.config["publication"] = {"command":[sys.executable], "credentialEnvironment":["DOCS_TEST_PUBLISH"], "audience":self.config["audience"]}
        with patch.dict(os.environ,{"DOCS_TEST_PUBLISH":"fixture"}):
            with patch.object(ci,"command",return_value={"targets":{}}) as dispatch:
                with self.assertRaisesRegex(ci.Gap,"TARGET_SUPERSEDED"):
                    ci.publish(self.config,result)
                self.assertEqual(dispatch.call_count,1)
            reply={"schema":"codeclew-documentation-publish-result/1.0","jobDigest":result["jobDigest"],"status":"PUBLISHED"}
            with patch.object(ci,"command",side_effect=[{"targets":result["targets"]},reply]):
                self.assertEqual(ci.publish(self.config,result),reply)

    def test_job_builder_validates_capture_target_and_rejects_duplicate_json(self):
        ci.write(self.root/"event.json",event())
        with patch.object(ci,"command",side_effect=self.fake_clew):
            captured=ci.run_job(self.config,self.job("capture"))
        ci.write(self.root/"capture.json",captured)
        with patch("sys.stdout"):
            code=ci.main(["build-job","--event",str(self.root/"event.json"),"--kind","update","--id","update-fixture","--capture-result",str(self.root/"capture.json"),"--output",str(self.root/"update.json")])
        self.assertEqual(code,0)
        self.assertEqual(ci.read(self.root/"update.json")["artifact"],captured["artifact"])
        with self.assertRaisesRegex(ci.Gap,"DUPLICATE_JSON_FIELD"):
            ci.decode(b'{"schema":1,"schema":2}')

    def agent_request(self, role="author"):
        return {"schema":"codeclew-documentation-agent-job/1.0", "invocation":"invocation", "role":role,
                "model":"configured-model", "work":"work", "payload":{"source":"untrusted instructions"},
                "cap":{"timeoutMs":1000,"outputBytes":4096,"maximum":{"inputTokens":5000,"outputTokens":500,"costUnits":10}}}

    def test_author_reviewer_fallback_dispatch_and_missing_usage_remains_unknown(self):
        config={"schema":"codeclew-documentation-agent-command/1.0", "roles":{r:{"model":"configured-model", "endpoint":"https://gateway.invalid/roles", "credentialEnvironment":"DOCS_TEST_TRANSPORT"} for r in ("author","reviewer","fallback")}}
        for role in config["roles"]:
            request=self.agent_request(role)
            reply={"schema":"codeclew-documentation-agent-result/1.0", **{k:request[k] for k in ("invocation","role","model")}, "result":{"action":"review" if role=="reviewer" else "proposal"}}
            with patch.dict(os.environ, {"DOCS_TEST_TRANSPORT":"fixture"}):
                self.assertNotIn("usage", ci.agent(config,request,lambda *a,**kw:reply))
                wrong={**reply,"role":"other"}
                with self.assertRaisesRegex(ci.Gap,"DISPATCH_MISMATCH"):
                    ci.agent(config,request,lambda *a,**kw:wrong)
                with self.assertRaisesRegex(ci.Gap,"TRANSPORT_UNAVAILABLE"):
                    ci.agent(config,request,lambda *a,**kw:(_ for _ in ()).throw(ci.Gap("TRANSPORT_UNAVAILABLE")))

    def test_invalid_events_and_redirect_are_rejected(self):
        for field,value in [("revision","HEAD"),("sequence",True),("sourceRef","main\ncommand"),("tag","different")]:
            bad=event();bad[field]=value
            with self.assertRaises(ci.Gap):ci.event(bad)
        with self.assertRaisesRegex(ci.Gap,"REDIRECT_DENIED"):
            ci.NoRedirect().redirect_request(None,None,302,"",{},"https://untrusted.invalid")

    def qualification(self):
        return {"schema":"codeclew-documentation-gitlab-qualification/1.0", "apiUrl":"https://gitlab.example.invalid/api/v4", "projectId":17,
                "ref":"qualification", "credentialEnvironment":"DOCS_TEST_GITLAB", "localCommand":[sys.executable],
                "pollSeconds":1,"timeoutSeconds":1,"cases":[{"id":"fixture-case","event":event(),"jobName":"compare","artifactPath":"result.json","expected":{"retryIdempotent":True}}]}

    def test_configured_gitlab_adapter_compares_real_api_shapes_with_fakes(self):
        config=self.qualification();calls=[]
        result={"schema":"codeclew-documentation-platform-result/1.0","id":"fixture-case","revision":"a"*40,"outcomes":{"retryIdempotent":True}}
        def transport(url,token,payload=None,**kwargs):
            calls.append((url,payload))
            if url.endswith("/pipeline"):return {"id":12}
            if url.endswith("/pipelines/12"):return {"status":"success"}
            if "jobs?" in url:return [{"name":"compare","status":"success","id":18}]
            return result
        with patch.dict(os.environ,{"DOCS_TEST_GITLAB":"fixture"}):
            report=ci.qualify_gitlab(config,self.root,transport,lambda *a,**kw:result)
        self.assertEqual(report["status"],"MATCHED_CONFIGURED_CASES")
        self.assertEqual(report["cases"][0]["pipelineId"],12)
        self.assertEqual(report["cases"][0]["jobId"],18)
        self.assertEqual(json.loads(calls[0][1]["variables"][1]["value"]),event())
        self.assertNotIn("fixture",json.dumps({k:v for k,v in report.items() if k=="credential"}))

    def test_gitlab_failed_job_expired_artifact_and_mismatched_local_results_are_gaps(self):
        config=self.qualification()
        for mode in ("failed-job","expired","mismatch"):
            def transport(url,token,payload=None,**kwargs):
                if url.endswith("/pipeline"):return {"id":12}
                if url.endswith("/pipelines/12"):return {"status":"failed" if mode=="failed-job" else "success"}
                if "jobs?" in url:return [] if mode=="failed-job" else [{"name":"compare","status":"success","id":18}]
                if mode=="expired":raise ci.Gap("TRANSPORT_UNAVAILABLE")
                return {"schema":"codeclew-documentation-platform-result/1.0","id":"fixture-case","revision":"a"*40,"outcomes":{}}
            with patch.dict(os.environ,{"DOCS_TEST_GITLAB":"fixture"}):
                report=ci.qualify_gitlab(config,self.root,transport,lambda *a,**kw:{"schema":"codeclew-documentation-platform-result/1.0","id":"fixture-case","revision":"a"*40,"outcomes":{"retryIdempotent":True}})
            self.assertEqual(report["status"],"QUALIFICATION_INCOMPLETE")


if __name__ == "__main__":
    unittest.main()
