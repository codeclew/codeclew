#!/usr/bin/env python3
"""Exercise the packaged documentation runner against neutral retained repositories."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from tempfile import TemporaryDirectory


ROOT = Path(__file__).resolve().parents[1]


def run(argv, cwd, env, allowed=(0,)):
    result = subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                            capture_output=True, text=True, timeout=240)
    if result.returncode not in allowed:
        raise RuntimeError(f"{argv[0]} {argv[1:5]} exited {result.returncode}\n{result.stdout}\n{result.stderr}")
    return result.stdout


def verify(options, base, service):
    env = {**os.environ, "CODECLEW_HOME": str(base / "state")}
    repository, docs, output = base / "repository", base / "docs", base / "run"
    shutil.copytree(ROOT / "fixtures/documentation-background-processes" / service, repository)

    def git(*args):
        return run(["git", "-c", "core.hooksPath=/dev/null", "-c", "user.name=Fixture",
                    "-c", "user.email=fixture@example.invalid", *args], repository, env)

    def docs_cli(*args, allowed=(0,)):
        return json.loads(run([options.clew, "docs", *args, "--root", str(docs)], base, env, allowed))

    def save(name, data):
        path = base / name
        path.write_text(json.dumps(data))
        return str(path)

    remote = f"https://example.invalid/fixtures/{service}.git"
    git("init", "-q", "-b", "main")
    git("remote", "add", "origin", remote)
    git("add", ".")
    git("commit", "-qm", "Retain background process sources")
    spec = {"schema": "codeclew-documentation-service/1.0", "id": service, "title": service.title(),
            "repositoryId": service, "repository": remote, "targetRef": "main"}
    if service == "host":
        run(["dotnet", "restore", "Host.csproj", "--ignore-failed-sources", "--nologo"], repository, env)
        spec.update(language="csharp", profile="csharp-dotnet-msbuild-read-only",
                    compilations=["csproj:Host.csproj@net10.0"])
    else:
        spec.update(language="typescript", profile="source-syntax", source={"roots": ["."], "dialect": "declared"})
    docs_cli("init")
    docs_cli("service", "add", "--input", save("service.json", spec),
             "--expected-input-digest", docs_cli("service", "list")["inputDigest"])
    docs_cli("bind", "--service", service, "--repo", str(repository))
    snapshot = docs_cli("check", allowed=(0, 3))
    assert snapshot["status"] == "CHECKED" and not snapshot.get("unresolved"), snapshot
    job = {
        "schema": "codeclew-docs-run-job/1.0", "clew": options.clew, "root": str(docs),
        "snapshot": snapshot["snapshot"], "services": [service], "request": "Document this service.",
        "language": "en", "renderer": str(options.skill / "examples/markdown-renderer"),
        "node": shutil.which("node"), "author": {"executable": sys.executable, "model": "fixture-author", "reasoningEffort": "low"},
        "budget": {"wallSeconds": 30, "maxRepairs": 0},
    }

    def prepare(destination, mode):
        job["evidenceMode"] = mode
        run([sys.executable, "-I", "-S", str(options.skill / "scripts/docs_run.py"), "prepare",
             "--job", save("job.json", job), "--run-dir", str(destination)], base, env)
        return json.loads((destination / "inputs/inventory.json").read_text())

    retained = prepare(base / "source-only", "RETAINED_SOURCE")
    normal = prepare(output, "PREPARED_FACTS")
    ordinary = {row["id"] for row in retained["requirements"]}
    detected = {row["entryKind"]: row for row in normal["requirements"]
                if row["kind"] == "ENTRY" and row["id"] not in ordinary}
    expected = {"HOSTED_BACKGROUND_WORKER"} if service == "host" else {"SCHEDULED_JOB", "KAFKA_LISTENER"}
    assert set(detected) == expected, detected
    home = output / "inputs/author-source" / service
    rows = json.loads((home / "prepared-facts.json").read_text())["entries"]
    ready = json.loads((home / "requirements.json").read_text())
    pages = json.loads((output / "inputs/starter/original.json").read_text())["pages"]
    for kind, requirement in detected.items():
        entry = rows[requirement["id"]]
        assert entry["status"] == "PREPARED", entry
        visible = "\n".join((home / view).read_text() for view in ready[requirement["id"]]["views"])
        assert f"Facts for Page {requirement['page']}" in visible, visible
        assert any(page["page"] == requirement["page"] and "clew-sequence" in str(page) for page in pages), pages
        assert entry["propositions"] and all(row["sources"] for row in entry["propositions"]), entry
        lower = visible.lower()
        facts = {row["kind"] for row in entry["propositions"]}
        assert "background-registration" in facts, entry
        if kind == "HOSTED_BACKGROUND_WORKER":
            assert "worker-poll" in facts and "tryreserveasync" in lower, visible
            assert "parcelsweep.cs" in lower and "program.cs" in lower, visible
            assert requirement["entrySymbol"] == "ParcelSweep.ExecuteAsync", requirement
        elif kind == "KAFKA_LISTENER":
            assert {"guarded_state_write", "await_bypass", "early_exit", "broker_offset_call"} <= facts, entry
            assert "parcels.ready" in lower and "processparcel" in lower and "commitoffsets" in lower, visible
        elif kind == "SCHEDULED_JOB":
            assert "5000" in lower and "updatehealth" in lower and "shadowed callback" not in lower, visible
    print(f"{service}: {', '.join(sorted(detected))}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clew", default=shutil.which("clew"))
    parser.add_argument("--skill", type=Path, default=ROOT / "skills/codeclew")
    parser.add_argument("--service", choices=("host", "subscriber"), action="append")
    options = parser.parse_args()
    options.skill = options.skill.resolve()
    for service in options.service or ("host", "subscriber"):
        with TemporaryDirectory(prefix="clew-background-process-") as directory:
            verify(options, Path(directory).resolve(), service)


if __name__ == "__main__":
    main()
