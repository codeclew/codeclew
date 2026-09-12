#!/usr/bin/env python3
"""Bounded, vendor-neutral documentation jobs and configured GitLab qualification.

Only installation configuration selects commands, paths, credentials and audience.
Events, packages and model output are data. Python 3.11+, standard library only.
"""
from __future__ import annotations

import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

LIMIT = 2 * 1024 * 1024
ID = re.compile(r"[A-Za-z][A-Za-z0-9_-]{0,99}\Z")
SHA = re.compile(r"[0-9a-f]{40}\Z")
DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")


class Gap(Exception):
    """Stable public reason without command output or private configuration."""


def encode(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def digest(value):
    return "sha256:" + hashlib.sha256(encode(value)).hexdigest()


def decode(data):
    if len(data) > LIMIT:
        raise Gap("RECORD_TOO_LARGE")
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise Gap("DUPLICATE_JSON_FIELD")
            result[key] = value
        return result
    try:
        return json.loads(data, object_pairs_hook=unique,
                          parse_constant=lambda _: (_ for _ in ()).throw(Gap("NONFINITE_JSON_NUMBER")))
    except (ValueError, UnicodeError):
        raise Gap("MALFORMED_JSON") from None


def read(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise Gap("INPUT_MISSING_OR_SYMLINK")
    with path.open("rb") as stream:
        return decode(stream.read(LIMIT + 1))


def write(path, value):
    path = Path(path)
    if path.is_symlink():
        raise Gap("OUTPUT_SYMLINK")
    data = encode(value)
    if len(data) > LIMIT:
        raise Gap("RECORD_TOO_LARGE")
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".documentation-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def closed(value, required, optional=()):
    if not isinstance(value, dict) or not set(required) <= value.keys() or value.keys() - set(required) - set(optional):
        raise Gap("INVALID_RECORD_FIELDS")


def relative(root, name):
    if not isinstance(name, str) or not name or "\\" in name:
        raise Gap("INVALID_ARTIFACT_PATH")
    path = Path(name)
    if path.is_absolute() or any(p in (".", "..") for p in name.split("/")):
        raise Gap("INVALID_ARTIFACT_PATH")
    root = Path(root).resolve(strict=True)
    candidate = root
    for part in path.parts:
        candidate = candidate / part
        if candidate.is_symlink():
            raise Gap("ARTIFACT_SYMLINK")
    if not candidate.resolve().is_relative_to(root):
        raise Gap("ARTIFACT_PATH_ESCAPE")
    return candidate


def event(value):
    closed(value, ("schema", "id", "service", "repositoryId", "sourceRef", "revision", "sequence"), ("tag",))
    if value["schema"] != "codeclew-documentation-update-event/1.0":
        raise Gap("EVENT_SCHEMA_UNSUPPORTED")
    if any(not isinstance(value[k], str) or not ID.fullmatch(value[k]) for k in ("id", "service", "repositoryId")):
        raise Gap("EVENT_ID_INVALID")
    ref = value["sourceRef"]
    if not isinstance(ref, str) or not ref or len(ref) > 256 or ref.startswith("-") or any(c.isspace() or ord(c) < 32 for c in ref):
        raise Gap("EVENT_REF_INVALID")
    if not isinstance(value["revision"], str) or not SHA.fullmatch(value["revision"]):
        raise Gap("EXACT_REVISION_REQUIRED")
    if type(value["sequence"]) is not int or not 0 < value["sequence"] < 2**64:
        raise Gap("EVENT_SEQUENCE_INVALID")
    if value.get("tag") not in (None, ref):
        raise Gap("TAG_REF_MISMATCH")
    return value


def command(argv, payload=None, *, timeout=600, environment=None):
    """No shell; bounded pipes, process group cancellation, stable failure codes."""
    if not isinstance(argv, list) or not argv or len(argv) > 128 or any(not isinstance(a, str) or "\0" in a for a in argv):
        raise Gap("COMMAND_CONFIGURATION_INVALID")
    if not Path(argv[0]).is_absolute() or not Path(argv[0]).is_file():
        raise Gap("COMMAND_EXECUTABLE_UNAVAILABLE")
    if not 0 < timeout <= 600:
        raise Gap("COMMAND_TIMEOUT_INVALID")
    data = b"" if payload is None else encode(payload)
    if len(data) > LIMIT:
        raise Gap("RECORD_TOO_LARGE")
    with tempfile.TemporaryFile() as source:
        source.write(data)
        source.seek(0)
        process = subprocess.Popen(argv, stdin=source, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   env=environment, start_new_session=True)
        buffers = {"stdout": bytearray(), "stderr": bytearray()}
        try:
            with selectors.DefaultSelector() as selector:
                for name in buffers:
                    stream = getattr(process, name)
                    os.set_blocking(stream.fileno(), False)
                    selector.register(stream, selectors.EVENT_READ, name)
                deadline = time.monotonic() + timeout
                while selector.get_map():
                    if time.monotonic() >= deadline:
                        raise Gap("PROCESS_TIMEOUT")
                    for key, _ in selector.select(min(0.1, max(0, deadline-time.monotonic()))):
                        chunk = os.read(key.fileobj.fileno(), 65536)
                        if not chunk:
                            selector.unregister(key.fileobj)
                        else:
                            buffers[key.data].extend(chunk)
                            if len(buffers[key.data]) > LIMIT:
                                raise Gap("PROCESS_OUTPUT_LIMIT")
                try:
                    code = process.wait(timeout=max(0.001, deadline-time.monotonic()))
                except subprocess.TimeoutExpired:
                    raise Gap("PROCESS_TIMEOUT") from None
                if code:
                    raise Gap("PROCESS_FAILED")
                result = decode(buffers["stdout"])
                if not isinstance(result, dict):
                    raise Gap("PROCESS_RESULT_INVALID")
                return result
        finally:
            # Kill descendants even when the immediate driver exited first.
            with contextlib.suppress(ProcessLookupError):
                os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            process.stdout.close()
            process.stderr.close()


@contextlib.contextmanager
def coordinator_lock(root):
    path = relative(root, ".coordinator.lock")
    with path.open("a+b") as stream:
        try:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise Gap("COORDINATOR_BUSY_RETRY") from None
        try:
            yield
        finally:
            fcntl.flock(stream, fcntl.LOCK_UN)


def installation(config):
    closed(config, ("schema", "clewCommand", "docsRoot", "artifactRoot", "audience"),
           ("executionConfig", "maxWork", "timeoutSeconds", "publication"))
    if config["schema"] != "codeclew-documentation-ci-installation/1.0":
        raise Gap("INSTALLATION_SCHEMA_UNSUPPORTED")
    for key in ("docsRoot", "artifactRoot"):
        p = Path(config[key])
        if not p.is_absolute() or not p.is_dir() or p.is_symlink():
            raise Gap("INSTALLATION_ROOT_UNAVAILABLE")
    if not isinstance(config["audience"], str) or not config["audience"].strip():
        raise Gap("AUTHORIZED_AUDIENCE_REQUIRED")
    if type(config.get("maxWork", 8)) is not int or not 1 <= config.get("maxWork", 8) <= 64:
        raise Gap("WORK_BOUND_INVALID")
    return config


def run_job(config, job):
    installation(config)
    closed(job, ("schema", "id", "kind"), ("event", "artifact", "events"))
    if job["schema"] != "codeclew-documentation-ci-job/1.0" or not ID.fullmatch(job["id"]):
        raise Gap("JOB_ID_OR_SCHEMA_INVALID")
    root = Path(config["artifactRoot"]).resolve(strict=True)
    result_path = relative(root, f"results/{job['id']}.json")
    identity = digest(job)
    stages = []

    def clew(*args, rooted=True):
        argv = list(config["clewCommand"]) + ["docs", *map(str, args)]
        if rooted:
            argv += ["--root", config["docsRoot"]]
        value = command(argv, timeout=config.get("timeoutSeconds", 600))
        if value.get("ok") is False or "error" in value:
            raise Gap("CLEW_STAGE_FAILED")
        stages.append({"command": list(args[:2]), "status": value.get("status"), "resultDigest": digest(value)})
        return value

    with coordinator_lock(root):
        if result_path.exists():
            previous = read(result_path)
            if previous.get("jobDigest") != identity:
                raise Gap("JOB_ID_REUSED")
            if previous.get("status") == "COMPLETED":
                return {**previous, "replayed": True}
        result = {"schema": "codeclew-documentation-ci-result/1.0", "id": job["id"],
                  "jobDigest": identity, "kind": job["kind"], "status": "RUNNING", "stages": stages}
        write(result_path, result)
        try:
            kind = job["kind"]
            if kind in ("capture", "update"):
                target = event(job["event"])
                result["event"] = target
                target_path = relative(root, f"events/{target['id']}.json")
                if target_path.exists() and read(target_path) != target:
                    raise Gap("EVENT_ID_REUSED")
                write(target_path, target)
                if kind == "capture":
                    package = relative(root, f"packages/{job['id']}")
                    if not package.exists():
                        clew("evidence", "capture", "--service", target["service"], "--output", package)
                    observed = clew("evidence", "inspect", "--input", package, rooted=False)
                else:
                    artifact = job["artifact"]
                    closed(artifact, ("path", "manifestDigest"))
                    if not DIGEST.fullmatch(artifact["manifestDigest"]):
                        raise Gap("TRUSTED_DIGEST_REQUIRED")
                    # The event is accepted even if its evidence is missing or corrupt.
                    clew("update", "enqueue", "--input", target_path)
                    package = relative(root, artifact["path"])
                    observed = clew("evidence", "inspect", "--input", package, rooted=False)
                    if observed.get("manifestDigest") != artifact["manifestDigest"]:
                        raise Gap("ARTIFACT_DIGEST_MISMATCH")
                if any(observed.get(k) != target[k] for k in ("service", "repositoryId", "revision")):
                    raise Gap("ARTIFACT_TARGET_MISMATCH")
                if observed.get("compatible") is not True:
                    raise Gap("ARTIFACT_COMPATIBILITY_GAP")
                result["artifact"] = {"path": str(package.relative_to(root)), "manifestDigest": observed["manifestDigest"]}
                if kind == "update":
                    state = clew("update", "status")
                    expectation = {"schema": "codeclew-documentation-evidence-expectation/1.0",
                                   **{k: target[k] for k in ("service", "repositoryId", "revision", "sequence")},
                                   "serviceDigest": observed["serviceDigest"], "manifestDigest": observed["manifestDigest"]}
                    expected_path = relative(root, f"expectations/{job['id']}.json")
                    write(expected_path, expectation)
                    clew("evidence", "expect", "--input", expected_path, "--expected-input-digest", state["inputDigest"])
                    clew("evidence", "import", "--input", package)
            elif kind == "reconcile":
                events = job["events"]
                if not isinstance(events, list) or not 1 <= len(events) <= 64:
                    raise Gap("REVISION_SET_BOUND_INVALID")
                path = relative(root, f"reconciliations/{job['id']}.json")
                write(path, {"schema": "codeclew-documentation-revision-set/1.0", "events": [event(e) for e in events]})
                clew("update", "reconcile", "--input", path)
            elif kind != "status":
                raise Gap("JOB_KIND_UNSUPPORTED")
            if kind != "capture":
                args = ["update", "run", "--max-work", str(config.get("maxWork", 8))]
                if config.get("executionConfig"):
                    args += ["--config", config["executionConfig"]]
                refreshed = clew(*args)
                result["refresh"] = {k: refreshed.get(k) for k in ("status", "pendingCount", "attempted", "remainingAtStart")}
                result["targets"] = clew("update", "status").get("targets")
            result["status"] = "COMPLETED"
        except (Gap, KeyError, TypeError, OSError) as error:
            result.update(status="INTEGRATION_GAP", reason=str(error) if isinstance(error, Gap) else "INVALID_OR_UNAVAILABLE_INPUT")
        write(result_path, result)
        return result


def publish(config, result):
    installation(config)
    policy = config.get("publication")
    if not policy:
        raise Gap("PUBLICATION_CONFIGURATION_REQUIRED")
    closed(policy, ("command", "credentialEnvironment", "audience"))
    if policy["audience"] != config["audience"]:
        raise Gap("PUBLICATION_AUDIENCE_MISMATCH")
    names = policy["credentialEnvironment"]
    if not names or any(not re.fullmatch(r"[A-Z][A-Z0-9_]*", n) or not os.environ.get(n) for n in names):
        raise Gap("PUBLICATION_CREDENTIALS_UNAVAILABLE")
    if result.get("schema") != "codeclew-documentation-ci-result/1.0" or result.get("status") != "COMPLETED" or result.get("kind") == "capture":
        raise Gap("PUBLICATION_RESULT_NOT_READY")
    with coordinator_lock(config["artifactRoot"]):
        retained = read(relative(config["artifactRoot"], f"results/{result['id']}.json"))
        if retained != result:
            raise Gap("PUBLICATION_RESULT_NOT_RETAINED")
        current = command(list(config["clewCommand"]) + ["docs", "update", "status", "--root", config["docsRoot"]])
        if current.get("targets") != result.get("targets"):
            raise Gap("PUBLICATION_TARGET_SUPERSEDED")
        # Publisher must use result targets and its remote expected revision as CAS.
        published = command(policy["command"], {"schema": "codeclew-documentation-publish-request/1.0",
                       "result": result, "docsRoot": config["docsRoot"], "audience": config["audience"]},
                       environment={n: os.environ[n] for n in names})
        closed(published, ("schema", "jobDigest", "status"))
        if published["schema"] != "codeclew-documentation-publish-result/1.0" or published["jobDigest"] != result["jobDigest"] or published["status"] not in ("PUBLISHED", "UNCHANGED", "CONFLICT", "PUBLICATION_GAP"):
            raise Gap("PUBLICATION_RESULT_PROTOCOL_INVALID")
        return published


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise Gap("TRANSPORT_REDIRECT_DENIED")


def http_json(url, token, payload=None, timeout=60):
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.fragment:
        raise Gap("HTTPS_TRANSPORT_REQUIRED")
    request = urllib.request.Request(url, data=None if payload is None else encode(payload),
                                    headers={"PRIVATE-TOKEN": token, "Content-Type": "application/json"})
    try:
        with urllib.request.build_opener(NoRedirect).open(request, timeout=timeout) as response:
            return decode(response.read(LIMIT+1))
    except (urllib.error.URLError, TimeoutError):
        raise Gap("TRANSPORT_UNAVAILABLE") from None


def agent(config, request, transport=http_json):
    closed(config, ("schema", "roles"))
    if config["schema"] != "codeclew-documentation-agent-command/1.0" or request.get("schema") != "codeclew-documentation-agent-job/1.0":
        raise Gap("AGENT_PROTOCOL_UNSUPPORTED")
    role = config["roles"].get(request.get("role"))
    if request.get("role") not in ("author", "reviewer", "fallback") or not role:
        raise Gap("AGENT_ROLE_UNCONFIGURED")
    closed(role, ("model", "endpoint", "credentialEnvironment"))
    if request.get("model") != role["model"]:
        raise Gap("AGENT_MODEL_MISMATCH")
    token = os.environ.get(role["credentialEnvironment"])
    if not token:
        raise Gap("AGENT_CREDENTIAL_UNAVAILABLE")
    cap = request["cap"]
    if not 0 < cap["timeoutMs"] <= 600000 or not 0 < cap["outputBytes"] <= LIMIT:
        raise Gap("AGENT_CAP_INVALID")
    # Trusted gateway enforces the supplied finite token/cost caps before dispatch.
    reply = transport(role["endpoint"], token, request, timeout=cap["timeoutMs"]/1000)
    closed(reply, ("schema", "invocation", "role", "model", "result"), ("usage",))
    if reply["schema"] != "codeclew-documentation-agent-result/1.0" or any(reply[k] != request[k] for k in ("invocation", "role", "model")):
        raise Gap("AGENT_DISPATCH_MISMATCH")
    if len(encode(reply)) > cap["outputBytes"] or not isinstance(reply["result"], dict):
        raise Gap("AGENT_RESULT_INVALID")
    usage = reply.get("usage")
    if usage is not None:
        closed(usage, (), ("inputTokens", "outputTokens", "costUnits"))
        for key, value in usage.items():
            if value is not None and (type(value) is not int or value < 0):
                raise Gap("AGENT_USAGE_INVALID")
    # Keep absent usage absent. The core charges reserved maxima and enforces bounds.
    return reply


def qualify_gitlab(config, output, transport=http_json, invoke=command):
    closed(config, ("schema", "apiUrl", "projectId", "ref", "credentialEnvironment", "localCommand", "cases", "pollSeconds", "timeoutSeconds"))
    if config["schema"] != "codeclew-documentation-gitlab-qualification/1.0":
        raise Gap("QUALIFICATION_SCHEMA_UNSUPPORTED")
    if type(config["projectId"]) is not int or config["projectId"] <= 0 or not 1 <= config["pollSeconds"] <= 60 or not 1 <= config["timeoutSeconds"] <= 3600:
        raise Gap("QUALIFICATION_BOUNDS_INVALID")
    cases = config["cases"]
    if not isinstance(cases, list) or not 1 <= len(cases) <= 16:
        raise Gap("QUALIFICATION_CASE_BOUND_INVALID")
    token = os.environ.get(config["credentialEnvironment"])
    if not token:
        raise Gap("GITLAB_CREDENTIAL_UNAVAILABLE")
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    api = config["apiUrl"].rstrip("/") + f"/projects/{config['projectId']}"
    report = {"schema": "codeclew-documentation-gitlab-qualification-result/1.0", "status": "RUNNING",
              "configurationDigest": digest(config), "cases": []}
    write(output / "results.json", report)
    for case in cases:
        closed(case, ("id", "event", "jobName", "artifactPath", "expected"))
        target = event(case["event"])
        if not ID.fullmatch(case["id"]):
            raise Gap("QUALIFICATION_CASE_ID_INVALID")
        row = {"id": case["id"], "revision": target["revision"], "status": "RUNNING"}
        report["cases"].append(row)
        try:
            local = invoke(config["localCommand"], {"schema": "codeclew-documentation-platform-case/1.0", "id": case["id"], "event": target})
            pipeline = transport(api + "/pipeline", token, {"ref": config["ref"], "variables": [
                {"key": "DOCUMENTATION_CASE_ID", "value": case["id"]},
                {"key": "DOCUMENTATION_EVENT_JSON", "value": encode(target).decode()}]})
            pid = pipeline.get("id")
            if type(pid) is not int or pid <= 0:
                raise Gap("GITLAB_PIPELINE_ID_INVALID")
            row["pipelineId"] = pid
            deadline = time.monotonic() + config["timeoutSeconds"]
            while True:
                state = transport(api + f"/pipelines/{pid}", token)
                if state.get("status") in ("success", "failed", "canceled", "skipped"):
                    break
                if time.monotonic() >= deadline:
                    raise Gap("GITLAB_PIPELINE_TIMEOUT")
                time.sleep(min(config["pollSeconds"], max(0, deadline-time.monotonic())))
            jobs = transport(api + f"/pipelines/{pid}/jobs?per_page=100", token)
            matches = [j for j in jobs if j.get("name") == case["jobName"] and j.get("status") == "success"]
            if len(matches) != 1:
                raise Gap("GITLAB_RESULT_JOB_UNAVAILABLE")
            jid = matches[0]["id"]
            if type(jid) is not int or jid <= 0:
                raise Gap("GITLAB_JOB_ID_INVALID")
            row["jobId"] = jid
            artifact = case["artifactPath"]
            if not isinstance(artifact, str) or artifact.startswith("/") or any(x in ("", ".", "..") for x in artifact.split("/")):
                raise Gap("QUALIFICATION_ARTIFACT_PATH_INVALID")
            remote = transport(api + f"/jobs/{jid}/artifacts/" + urllib.parse.quote(artifact, safe="/"), token)
            for result in (local, remote):
                closed(result, ("schema", "id", "revision", "outcomes"))
                if result.get("schema") != "codeclew-documentation-platform-result/1.0" or result.get("id") != case["id"] or result.get("revision") != target["revision"]:
                    raise Gap("PLATFORM_RESULT_IDENTITY_MISMATCH")
                if not isinstance(result["outcomes"], dict) or len(result["outcomes"]) > 64 or any(
                    not ID.fullmatch(k) or not isinstance(v, (bool, int)) for k, v in result["outcomes"].items()
                ):
                    raise Gap("PLATFORM_OUTCOME_INVALID")
            row.update(local=local, gitlab=remote, pipelineStatus=state.get("status"))
            row["status"] = "MATCHED" if local.get("outcomes") == remote.get("outcomes") == case["expected"] else "MISMATCH"
        except (Gap, OSError, KeyError, TypeError) as error:
            row.update(status="QUALIFICATION_GAP", reason=str(error) if isinstance(error, Gap) else "INVALID_OR_UNAVAILABLE_PLATFORM_RESULT")
        write(output / "results.json", report)
    report["status"] = "MATCHED_CONFIGURED_CASES" if all(r["status"] == "MATCHED" for r in report["cases"]) else "QUALIFICATION_INCOMPLETE"
    write(output / "results.json", report)
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("run-job", "publish", "agent", "qualify-gitlab"):
        sub = commands.add_parser(name)
        sub.add_argument("--config", required=True)
        if name in ("run-job", "publish"):
            sub.add_argument("--input", required=True)
        if name != "agent":
            sub.add_argument("--output", required=True)
    build = commands.add_parser("build-job")
    build.add_argument("--event", required=True)
    build.add_argument("--id", required=True)
    build.add_argument("--kind", choices=("capture", "update"), required=True)
    build.add_argument("--capture-result")
    build.add_argument("--output", required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "build-job":
            target = event(read(args.event))
            if not ID.fullmatch(args.id):
                raise Gap("JOB_ID_INVALID")
            result = {"schema": "codeclew-documentation-ci-job/1.0", "id": args.id, "kind": args.kind, "event": target}
            if args.kind == "update":
                captured = read(args.capture_result)
                if captured.get("status") != "COMPLETED" or captured.get("kind") != "capture" or captured.get("event") != target:
                    raise Gap("CAPTURE_RESULT_TARGET_MISMATCH")
                result["artifact"] = captured["artifact"]
            write(args.output, result)
            print(json.dumps(result))
            return 0
        config = read(args.config)
        if args.command == "run-job":
            result = run_job(config, read(args.input))
        elif args.command == "publish":
            result = publish(config, read(args.input))
        elif args.command == "agent":
            result = agent(config, decode(sys.stdin.buffer.read(LIMIT+1)))
        else:
            result = qualify_gitlab(config, args.output)
        if args.command not in ("agent", "qualify-gitlab"):
            write(args.output, result)
        print(json.dumps(result))
        return 2 if result.get("status") in ("INTEGRATION_GAP", "QUALIFICATION_INCOMPLETE", "CONFLICT", "PUBLICATION_GAP") else 0
    except (Gap, KeyError, TypeError, OSError, ValueError) as error:
        print(json.dumps({"schema": "codeclew-documentation-ci-gap/1.0", "status": "INTEGRATION_GAP",
                          "reason": str(error) if isinstance(error, Gap) else "INVALID_OR_UNAVAILABLE_CONFIGURATION"}))
        return 2


if __name__ == "__main__":
    def interrupted(_signal, _frame):
        raise Gap("CANCELLED")
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    raise SystemExit(main())
