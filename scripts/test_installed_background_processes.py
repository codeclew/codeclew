#!/usr/bin/env python3
"""Exercise the packaged documentation runner against neutral retained repositories."""
import argparse
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
from tempfile import TemporaryDirectory


ROOT = Path(__file__).resolve().parents[1]
FACT_CONTEXT = runpy.run_path(str(ROOT / "skills/codeclew/scripts/docs_run_fact_context.py"))


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
    elif service == "java-dispatch":
        spec.update(language="java", profile="source-syntax", source={"roots": ["src"], "dialect": "17"})
    else:
        language = "kotlin" if service == "producer" else "typescript"
        spec.update(language=language, profile="source-syntax", source={"roots": ["."], "dialect": "declared"})
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

    if service in {"producer", "java-dispatch"}:
        job["authorMode"] = "ISOLATED_PAGES"
        prepare(output, "PREPARED_FACTS")
        plan = json.loads((output / "inputs/author-plan.json").read_text())
        slot, = (slot for slot in plan["slots"] if slot["kind"] == "OPERATION")
        prompt = (output / "inputs" / slot["prompt"]).read_text()
        packet = FACT_CONTEXT["expand_fact_context"](json.loads(prompt.split("\n\n")[-1]))
        group = packet["facts"][slot["requirement"]]
        kinds = {row["kind"] for key in ("facts", "boundaries") for row in group.get(key, [])}
        if service == "java-dispatch":
            assert packet["requirement"]["entryKind"] == "JAVA_TYPED_DISPATCH", packet["requirement"]
            assert packet["requirement"]["entrySymbol"] == "neutral.assembly.ReceiptHandler.accept", packet["requirement"]
            assert {"background-registration", "java-guarded-outcome"} <= kinds, kinds
            registration, = (row for row in group["facts"] if row["kind"] == "background-registration")
            assert all(text in registration["text"] for text in ("receipts.pending", "neutral.assembly.BatchArrived")), registration
            assert "transport" in registration["uncertainty"], registration
            regions = {row["file"].rsplit("/", 1)[-1] for row in packet["sourceRanges"].values()}
            assert {"DispatchWiring.java", "ReceiptHandler.java"} <= regions and "UnusedTable.java" not in regions, regions
            assert any("IllegalArgumentException" in row["text"] for row in group["facts"]), group
            print(f"{service}: {len(prompt.encode())} isolated packet bytes for the wired callback")
            return
        assert packet["requirement"]["entryKind"] == "SCHEDULED_JOB", packet["requirement"]
        assert {"POSSIBLE_REPEAT_AFTER_MARK_FAILURE", "WAIT_TRANSACTION_BOUNDARY"} <= kinds, kinds
        repeat, = (row for row in group["facts"] if row["kind"] == "POSSIBLE_REPEAT_AFTER_MARK_FAILURE")
        assert "sent_at IS NULL" in repeat["text"] and "fails to commit" in repeat["text"], repeat
        assert len(repeat["sources"]) >= 2 and set(repeat["sources"]) <= packet["citations"].keys(), repeat
        assert "Broker acceptance and consumer completion remain open" in repeat["text"], repeat
        print(f"{service}: {len(prompt.encode())} isolated packet bytes for wait and conditional re-publication")
        return

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
            assert {"worker-poll", "guarded_state_write", "await_bypass", "early_exit"} <= facts, visible
            guarded = next(row for row in entry["propositions"] if row["kind"] == "guarded_state_write")
            assert "TryReserveAsync" in guarded["text"] and "processed += 1" in guarded["text"], guarded
            early = next(row for row in entry["propositions"] if row["kind"] == "early_exit")
            assert "InvalidOperationException" in early["text"] and "return" in early["text"], early
            assert "parcelsweep.cs" in lower and "program.cs" in lower, visible
            assert requirement["entrySymbol"] == "ParcelSweep.ExecuteAsync", requirement
        elif kind == "KAFKA_LISTENER":
            assert {"guarded_state_write", "await_bypass", "early_exit", "broker_offset_call"} <= facts, entry
            assert "parcels.ready" in lower and "processparcel" in lower and "commitoffsets" in lower, visible
        elif kind == "SCHEDULED_JOB":
            assert "5000" in lower and "updatehealth" in lower and "shadowed callback" not in lower, visible
            bindings = [row["text"] for row in entry["propositions"] if row["kind"] == "iteration_helper_binding"]
            assert len(bindings) == 1 and "sample.id through key in parameter key" in bindings[0], bindings
    print(f"{service}: {', '.join(sorted(detected))}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clew", default=shutil.which("clew"))
    parser.add_argument("--skill", type=Path, default=ROOT / "skills/codeclew")
    parser.add_argument("--service", choices=("host", "subscriber", "producer", "java-dispatch"), action="append")
    options = parser.parse_args()
    options.skill = options.skill.resolve()
    for service in options.service or ("host", "subscriber", "producer"):
        with TemporaryDirectory(prefix="clew-background-process-") as directory:
            verify(options, Path(directory).resolve(), service)


if __name__ == "__main__":
    main()
