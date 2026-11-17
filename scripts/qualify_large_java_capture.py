#!/usr/bin/env python3
"""Owned bounded Java capture fixture and supported-launcher qualification.

Generation alone never claims producer success. No private state injection,
source parsing shortcut, model author or external repository is used.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
import signal
from pathlib import Path
import shutil
import subprocess
import sys
import time


def save(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def run(argv, cwd, env=None):
    if argv[0]=="git":
        supplied=env or os.environ
        env={k:v for k,v in supplied.items() if not k.startswith("GIT_")}
        for key in ("GIT_AUTHOR_NAME","GIT_COMMITTER_NAME","GIT_AUTHOR_EMAIL","GIT_COMMITTER_EMAIL","GIT_AUTHOR_DATE","GIT_COMMITTER_DATE"):
            if key in supplied: env[key]=supplied[key]
    return subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                          capture_output=True, text=True, check=True).stdout


def fixture_git_environment():
    # Environment identity cannot override the owned deterministic commit.
    return dict(os.environ, GIT_AUTHOR_NAME="Codeclew owned fixture",
                GIT_COMMITTER_NAME="Codeclew owned fixture",
                GIT_AUTHOR_EMAIL="fixture@example.invalid", GIT_COMMITTER_EMAIL="fixture@example.invalid",
                GIT_AUTHOR_DATE="2026-11-17T00:00:00Z", GIT_COMMITTER_DATE="2026-11-17T00:00:00Z")


def file_sha256(path):
    digest=hashlib.sha256()
    with path.open("rb") as reader:
        for block in iter(lambda:reader.read(1024*1024),b""): digest.update(block)
    return digest.hexdigest()


def runner_resources(root):
    report={"platform":sys.platform,"freeDiskBytes":shutil.disk_usage(root).free,
            "memoryTotalBytes":None,"swapTotalBytes":None,"swapUsedBytes":None}
    meminfo=Path("/proc/meminfo")
    if meminfo.is_file():
        values={k:int(v.split()[0])*1024 for k,v in
                (line.split(":",1) for line in meminfo.read_text().splitlines())}
        report.update(memoryTotalBytes=values.get("MemTotal"),swapTotalBytes=values.get("SwapTotal"),
                      swapUsedBytes=values.get("SwapTotal",0)-values.get("SwapFree",0))
    return report


def progress_inventory(events, selected):
    completed=[e.get("counts",{}) for e in events if e.get("phase")=="COMPILATION_GENERATION" and e.get("event")=="COMPLETED"]
    ordinals=[c.get("compilationOrdinal") for c in completed]
    totals=[c.get("compilationsTotal") for c in completed]
    return {"selectedCompilations":selected,"completedCompilationOrdinals":ordinals,
            "completedCompilationTotals":totals,
            "allSelectedGenerationsCompleted":ordinals==list(range(1,len(selected)+1)) and totals==[len(selected)]*len(selected)}


def generate(root, methods, steps, scopes):
    if root.exists():
        raise ValueError("fixture output must be new; previous trials remain intact")
    if not 1 <= methods <= 2200 or not 1 <= steps <= 200 or not 1 <= scopes <= 3:
        raise ValueError("bounded fixture dimensions required")
    root.mkdir(parents=True)
    repo = root / "repository"; repo.mkdir()
    modules = [f"scope-{n+1}" for n in range(scopes)]
    namespace = 'xmlns="http://maven.apache.org/POM/4.0.0"'
    parent = f'''<project {namespace}><modelVersion>4.0.0</modelVersion>
<groupId>dev.codeclew.owned</groupId><artifactId>fact-density</artifactId><version>1</version><packaging>pom</packaging>
<properties><maven.compiler.release>17</maven.compiler.release><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding></properties>
<modules>{''.join('<module>'+m+'</module>' for m in modules)}</modules>
<build><pluginManagement><plugins>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version></plugin>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-dependency-plugin</artifactId><version>3.8.1</version></plugin>
</plugins></pluginManagement></build></project>'''
    (repo / "pom.xml").write_text(parent)
    for module in modules:
        directory = repo / module; directory.mkdir()
        (directory / "pom.xml").write_text(f'''<project {namespace}><modelVersion>4.0.0</modelVersion>
<parent><groupId>dev.codeclew.owned</groupId><artifactId>fact-density</artifactId><version>1</version></parent>
<artifactId>{module}</artifactId></project>''')
    source = "package owned; public class Dense {\n" + "".join(
        f"public static int method{n}(int input) {{ int value = input;\n" + "value++;\n" * steps + "return value; }\n"
        for n in range(methods)) + "}\n"
    source_files = []
    for module in modules:
        path = repo / module / "src/main/java/owned/Dense.java"
        path.parent.mkdir(parents=True); path.write_text(source)
        source_files.append(str(path.relative_to(repo)))
    (repo / ".gitignore").write_text("target/\n**/target/\n")
    run(["git", "init", "-q", "-b", "main", "--object-format=sha1"], repo)
    run(["git", "config", "user.name", "Codeclew owned fixture"], repo)
    run(["git", "config", "user.email", "fixture@example.invalid"], repo)
    run(["git", "config", "commit.gpgsign", "false"], repo)
    run(["git", "config", "core.autocrlf", "false"], repo)
    run(["git", "remote", "add", "origin", "https://example.invalid/owned-fact-density"], repo)
    run(["git", "add", "."], repo)
    run(["git", "-c", "core.hooksPath=/dev/null", "commit", "-qm", "Freeze owned conventional compilation scopes"], repo, fixture_git_environment())
    revision = run(["git", "rev-parse", "HEAD"], repo).strip()
    service = {"schema": "codeclew-documentation-service/1.0", "id": "density", "title": "Owned fact density",
               "repositoryId": "owned-fact-density", "repository": "https://example.invalid/owned-fact-density",
               "language": "java", "profile": "java-17plus-maven-read-only", "targetRef": "main",
               "compilations": [f":{module}/main" for module in modules]}
    save(root / "service.json", service)
    files = {str(p.relative_to(repo)): hashlib.sha256(p.read_bytes()).hexdigest()
             for p in repo.rglob("*") if p.is_file() and ".git" not in p.relative_to(repo).parts}
    report = {"schema": "codeclew-owned-large-java-fixture/1.0", "status": "GENERATED_NOT_CAPTURED",
              "methods": methods, "steps": steps, "scopes": scopes, "sourceBytes": len(source.encode()),
              "revision": revision, "files": files, "inputBytes": sum((repo / p).stat().st_size for p in files),
              "expectedSourceFiles": source_files, "sameSourceBytesAcrossDistinctFiles": True, "expectedMethodsPerScope": methods,
              "recordCountPredictionFromMeasuredDensity": (methods * (steps + 5) + 2) * scopes,
              "predictionIsNotProducerMeasurement": True}
    save(root / "fixture.json", report)
    return report


def generate_semantic(root):
    if root.exists(): raise ValueError("semantic fixture output must be new")
    root.mkdir(parents=True); repo=root/"repository"; repo.mkdir()
    modules=("common","api","other","unselected")
    namespace='xmlns="http://maven.apache.org/POM/4.0.0"'
    (repo/"pom.xml").write_text(f'''<project {namespace}><modelVersion>4.0.0</modelVersion><groupId>dev.codeclew.owned</groupId><artifactId>semantic-parent</artifactId><version>1</version><packaging>pom</packaging>
<properties><maven.compiler.release>17</maven.compiler.release><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding></properties>
<modules>{''.join('<module>'+m+'</module>' for m in modules)}</modules><build><pluginManagement><plugins>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version></plugin>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-dependency-plugin</artifactId><version>3.8.1</version></plugin>
</plugins></pluginManagement></build></project>''')
    sources={
        "common/src/main/java/owned/CommonStore.java": """package owned;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
public final class CommonStore {
    public static int persist(int value) throws IOException {
        Files.writeString(Path.of("owned-output.txt"), Integer.toString(value));
        return value;
    }
}
""",
        "api/src/main/java/owned/Entry.java": """package owned;
import java.io.IOException;
public final class Entry {
    @Deprecated
    public int submit(int quantity) throws IOException {
        if (quantity < 0) throw new IllegalArgumentException("negative");
        int normalized = normalize(quantity);
        return CommonStore.persist(normalized);
    }
    private int normalize(int quantity) {
        return quantity == 0 ? 1 : quantity;
    }
}
""",
        "api/src/main/java/owned/Duplicate.java": """package owned;
public final class Duplicate {
    public static int flavor(int value) { return value + 10; }
}
""",
        "other/src/main/java/owned/OtherEntry.java": """package owned;
public final class OtherEntry {
    @Deprecated
    public int noise(int value) { return Duplicate.flavor(value); }
}
""",
        "other/src/main/java/owned/Duplicate.java": """package owned;
public final class Duplicate {
    public static int flavor(int value) { return value - 10; }
}
""",
        "unselected/src/main/java/owned/NotSelected.java": """package owned;
public final class NotSelected { deliberately invalid Java outside selected build closure }
""",
    }
    for module in modules:
        directory=repo/module; directory.mkdir()
        dependency=''
        if module in ("api","other"):
            dependency='<dependencies><dependency><groupId>dev.codeclew.owned</groupId><artifactId>common</artifactId><version>1</version></dependency></dependencies>'
        (directory/"pom.xml").write_text(f'<project {namespace}><modelVersion>4.0.0</modelVersion><parent><groupId>dev.codeclew.owned</groupId><artifactId>semantic-parent</artifactId><version>1</version></parent><artifactId>{module}</artifactId>{dependency}</project>')
    for name,source in sources.items():
        path=repo/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(source)
    (repo/".gitignore").write_text("target/\n**/target/\n")
    run(["git","init","-q","-b","main","--object-format=sha1"],repo)
    for key,value in (("user.name","Codeclew owned fixture"),("user.email","fixture@example.invalid"),("commit.gpgsign","false"),("core.autocrlf","false")):
        run(["git","config",key,value],repo)
    run(["git","remote","add","origin","https://example.invalid/owned-compiler-semantic"],repo)
    run(["git","add","."],repo)
    run(["git","-c","core.hooksPath=/dev/null","commit","-qm","Freeze owned compiler closure and scope ambiguity"],repo,fixture_git_environment())
    revision=run(["git","rev-parse","HEAD"],repo).strip()
    service={"schema":"codeclew-documentation-service/1.0","id":"semantic","title":"Owned compiler closure",
             "repositoryId":"owned-compiler-semantic","repository":"https://example.invalid/owned-compiler-semantic",
             "language":"java","profile":"java-17plus-maven-read-only","targetRef":"main",
             "compilations":[":api/main",":common/main",":other/main"]}
    save(root/"service.json",service)
    files={str(p.relative_to(repo)):hashlib.sha256(p.read_bytes()).hexdigest() for p in repo.rglob("*") if p.is_file() and ".git" not in p.relative_to(repo).parts}
    fragments={name:{"sha256":files[name],"lines":len(text.splitlines()),"source":text} for name,text in sources.items()}
    expected={"schema":"codeclew-owned-compiler-semantic-expectations/1.0","revision":revision,
              "selectedCompilations":service["compilations"],"excludedCompilations":[":unselected/main"],
              "entrySelector":{"language":"java","owner":"owned.Entry","name":"submit","parameterTypes":["int"]},
              "requiredCalls":[["owned.Entry.submit","owned.Entry.normalize"],["owned.Entry.submit","owned.CommonStore.persist"],["owned.CommonStore.persist","java.nio.file.Files.writeString"]],
              "branchConsequences":["negative quantity throws IllegalArgumentException before normalize or storage", "zero normalizes to one; other nonnegative values remain unchanged", "storage writes the normalized value then returns it; IOException propagates"],
              "duplicateIdentity":{"identity":"class:owned.Duplicate","scopes":[":api/main",":other/main"],"bodiesMustRemainDistinct":["return value + 10","return value - 10"]},
              "sharedAnnotationIdentity":{"identity":"java.lang.Deprecated","declarations":["owned.Entry.submit","owned.OtherEntry.noise"],"scopes":[":api/main",":other/main"]},
              "unrelatedToEntryThread":["owned.OtherEntry.noise","owned.Duplicate.flavor"],
              "files":fragments,"runtimeExecutionClaimed":False,"generatedNotYetQualified":True}
    save(root/"expected.json",expected)
    report={"schema":"codeclew-owned-large-java-fixture/1.0","kind":"SEMANTIC","status":"GENERATED_NOT_CAPTURED",
            "revision":revision,"files":files,"inputBytes":sum((repo/p).stat().st_size for p in files),
            "expectedSourceFiles":list(sources),"expectations":"expected.json","selectedCompilations":service["compilations"]}
    save(root/"fixture.json",report);return report


def disk_bytes(root):
    # Count logical bytes and actual allocated blocks, not a guessed payload sum.
    logical = allocated = 0
    for path in root.rglob("*"):
        if path.is_file() and not path.is_symlink():
            stat = path.stat(); logical += stat.st_size; allocated += getattr(stat, "st_blocks", 0) * 512
    return {"logicalBytes": logical, "allocatedBytes": allocated}


def capture(root, launcher, require_old_threshold, expected_commit=None, archive=None):
    root = root.resolve(); launcher = launcher.resolve(strict=True)
    fixture = json.loads((root / "fixture.json").read_text())
    repo = root / "repository"
    service = json.loads((root / "service.json").read_text()); service_id = service["id"]
    if run(["git", "rev-parse", "HEAD"], repo).strip() != fixture["revision"] or run(["git", "status", "--porcelain"], repo).strip():
        raise ValueError("frozen owned fixture revision or worktree changed")
    for name, expected in fixture["files"].items():
        relative = Path(name)
        if relative.is_absolute() or ".." in relative.parts or ".git" in relative.parts or (repo/relative).is_symlink():
            raise ValueError("invalid frozen fixture path")
        if hashlib.sha256((repo/relative).read_bytes()).hexdigest() != expected:
            raise ValueError("frozen owned fixture bytes changed")
    logs = root / "capture"; logs.mkdir()
    docs = root / "documentation"; docs.mkdir()
    env = {k: v for k, v in os.environ.items() if not k.startswith("CODECLEW_")}
    env["CODECLEW_HOME"] = str(root / "state")
    # Ordinary toolchain settings cannot change evidence scope. The admitted
    # official launcher never compiles Codeclew itself.
    for key in ("CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "JAVA_HOME"):
        if key in os.environ: env[key] = os.environ[key]
    env["TMPDIR"] = str(root / "temporary"); Path(env["TMPDIR"]).mkdir()
    commands = [
        [str(launcher), "docs", "init", "--root", str(docs)],
        [str(launcher), "docs", "service", "list", "--root", str(docs)],
    ]
    receipts = []
    def command(name, argv):
        started = time.monotonic()
        p = subprocess.run(argv, env=env, cwd=root, stdin=subprocess.DEVNULL, capture_output=True)
        (logs / (name + ".stdout")).write_bytes(p.stdout); (logs / (name + ".stderr")).write_bytes(p.stderr)
        receipts.append({"name": name, "argv": argv, "exitCode": p.returncode, "seconds": time.monotonic()-started})
        save(logs / "commands.json", receipts)
        if p.returncode: raise RuntimeError(f"{name} failed; retained stdout/stderr contain the cause")
        return json.loads(p.stdout)
    version = run([str(launcher), "--version"], root, env).strip()
    capabilities = command("capabilities", [str(launcher), "capabilities"])
    if capabilities.get("runtimeMode") != "RELEASE":
        raise ValueError("qualification requires the supported official packaged launcher")
    source_path = launcher.parent.parent / "source/release-source.json"
    source = json.loads(source_path.read_bytes())
    admission = {"schema":"codeclew-owned-capture-package-admission/1.0", "version":version,
                 "sourceRevision":source["sourceRevision"], "sourceTree":source["sourceTree"],
                 "launcherSha256":hashlib.sha256(launcher.read_bytes()).hexdigest(),
                 "sourceManifestSha256":hashlib.sha256(source_path.read_bytes()).hexdigest(),
                 "runtimeKey":capabilities.get("runtimeKey"), "agentContract":capabilities.get("agentContract"),
                 "minimumFreeBytes":512*1024*1024, "wallLimitSeconds":2700}
    if expected_commit is not None and source["sourceRevision"]!=expected_commit:
        raise ValueError("official package source differs from expected exact commit")
    if require_old_threshold and (not expected_commit or archive is None):
        raise ValueError("large qualification requires exact commit and sealed archive provenance")
    if archive is not None:
        admission["sealedArchiveSha256"]=file_sha256(archive)
        admission["sealedArchiveBytes"]=archive.stat().st_size
    admission.update(maximumSwapUsedBytes=8*1024**3,maximumTemporaryDiskBytes=8*1024**3,
                     minimumLargeLaunchDiskBytes=8*1024**3,resourceEscalationOrRetryAllowed=False)
    preflight=runner_resources(root)
    save(logs / "admitted-package.json", admission)
    save(logs / "runner-preflight.json",preflight)
    if require_old_threshold and (preflight["freeDiskBytes"]<admission["minimumLargeLaunchDiskBytes"]
            or preflight["memoryTotalBytes"] is None or preflight["memoryTotalBytes"]<12*1024**3
            or preflight["swapTotalBytes"] is None or preflight["swapTotalBytes"]>admission["maximumSwapUsedBytes"]):
        raise ValueError("runner outside predeclared large-capture resource envelope; no retry or escalation")
    command("init", commands[0]); current = command("services", commands[1])
    command("add", [str(launcher), "docs", "service", "add", "--root", str(docs), "--input", str(root/"service.json"), "--expected-input-digest", current["inputDigest"]])
    command("bind", [str(launcher), "docs", "bind", "--root", str(docs), "--service", service_id, "--repo", str(root/"repository")])
    initial_disk = disk_bytes(root)
    started = time.monotonic(); before = shutil.disk_usage(root).free
    stderr_path = logs / "check.stderr"; stdout_path = logs / "check.stdout"
    argv = [str(launcher), "docs", "check", "--root", str(docs), "--service", service_id]
    resources=[];live_receipts=[]
    events = []; first_live = None; minimum_free = before; temporary_peak = {"logicalBytes": 0, "allocatedBytes": 0}; peak_sampled_main_rss = 0; stop_reason = None; last_sample = 0.0
    # Read the growing file while the producer runs. This proves event delivery
    # before completion without a pipe reader changing child process behavior.
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        process = subprocess.Popen(argv, env=env, cwd=root, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr, start_new_session=True)
        offset = 0; pending = b""
        while process.poll() is None:
            with stderr_path.open("rb") as reader:
                reader.seek(offset); raw = reader.read(); offset = reader.tell()
            pending += raw; lines = pending.split(b"\n"); pending = lines.pop()
            for line in lines:
                try: event = json.loads(line)
                except (ValueError, UnicodeError): continue
                if event.get("schema") == "codeclew-documentation-progress/1.0":
                    events.append(event)
                    live_receipts.append({"spanId":event.get("spanId"),"phase":event.get("phase"),"event":event.get("event"),
                                          "observedAtUnixMs":int(time.time()*1000),"observedAfterSeconds":time.monotonic()-started})
                    if first_live is None: first_live = time.monotonic()-started
            free = shutil.disk_usage(root).free
            minimum_free = min(minimum_free, free)
            if time.monotonic()-last_sample >= 2:
                resource=runner_resources(root);resource["afterSeconds"]=time.monotonic()-started;resources.append(resource)
                measured = disk_bytes(Path(env["TMPDIR"]))
                temporary_peak = {k: max(temporary_peak[k], measured[k]) for k in temporary_peak}
                if shutil.which("ps"):
                    rss = subprocess.run(["ps", "-o", "rss=", "-p", str(process.pid)], capture_output=True, text=True).stdout.strip()
                    if rss.isdecimal(): peak_sampled_main_rss = max(peak_sampled_main_rss, int(rss)*1024)
                last_sample = time.monotonic()
            resource_exceeded=resources and (resources[-1].get("swapUsedBytes") or 0)>admission["maximumSwapUsedBytes"]
            temporary_exceeded=temporary_peak["allocatedBytes"]>admission["maximumTemporaryDiskBytes"]
            if free < 512*1024*1024 or time.monotonic()-started > 2700 or resource_exceeded or temporary_exceeded:
                stop_reason = ("HARNESS_MINIMUM_FREE_SPACE" if free < 512*1024*1024 else
                               "HARNESS_SWAP_LIMIT" if resource_exceeded else
                               "HARNESS_TEMPORARY_DISK_LIMIT" if temporary_exceeded else "HARNESS_WALL_LIMIT")
                os.killpg(process.pid, signal.SIGTERM)
                try: process.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL); process.wait()
                break
            time.sleep(0.2)
    # Parse the full final stream as well; incremental receipts remain distinct.
    complete_events = []
    for line in stderr_path.read_bytes().splitlines():
        try: event = json.loads(line)
        except (ValueError, UnicodeError): continue
        if event.get("schema") == "codeclew-documentation-progress/1.0": complete_events.append(event)
    try: result = json.loads(stdout_path.read_bytes()); stdout_valid = True
    except (ValueError, UnicodeError): result = {}; stdout_valid = False
    measured = [e for e in complete_events if e.get("phase") == "HYDRATE_FACT_PAYLOADS" and e.get("event") == "COMPLETED"]
    totals = measured[-1].get("counts", {}) if measured else {}
    count, size = totals.get("facts"), totals.get("payloadBytes")
    missing_baseline = result.get("freshness", {}).get("reasons") == ["MISSING_BASELINE"]
    capture_exit = process.returncode == 0 or (process.returncode == 3 and missing_baseline)
    frozen_after = run(["git", "rev-parse", "HEAD"], repo).strip() == fixture["revision"] and not run(["git", "status", "--porcelain"], repo).strip()
    revision_matches = result.get("services", {}).get(service_id, {}).get("revision") == fixture["revision"]
    success = stop_reason is None and stdout_valid and capture_exit and frozen_after and revision_matches and result.get("status") == "CHECKED" and result.get("unresolved") == {}
    inventory=progress_inventory(complete_events,service["compilations"])
    measured_complete=(type(count) is int and type(size) is int and totals.get("plannedFacts")==count
                       and totals.get("plannedPayloadBytes")==size)
    completed_by_span={e.get("spanId"):e for e in complete_events if e.get("event")=="COMPLETED"}
    live_before_completion=[r for r in live_receipts if r["event"]=="STARTED" and r["spanId"] in completed_by_span
                            and r["observedAtUnixMs"]<completed_by_span[r["spanId"]].get("timestampUnixMs",0)]
    native_live=any(r["phase"] in ("NATIVE_COMPILATION","COMPILER_ANALYSIS","JAVA_MODEL_EXTRACTION") for r in live_before_completion)
    native_heartbeat=any(e.get("event")=="HEARTBEAT" and e.get("phase") in ("COMPILER_ANALYSIS","NATIVE_COMPILATION","JAVA_MODEL_EXTRACTION") for e in events)
    if require_old_threshold:
        success = success and measured_complete and count > 524288 and size > 268435456 and inventory["allSelectedGenerationsCompleted"] and native_live and native_heartbeat
    report = {"schema": "codeclew-owned-large-java-capture/1.0", "status": "PASS" if success else "FAIL",
              "launcher": str(launcher), "version": version, "fixtureRevision": fixture["revision"], "argv": argv,
              "exitCode": process.returncode, "checkStatus": result.get("status"), "snapshot": result.get("snapshot"),
              "elapsedSeconds": time.monotonic()-started, "measuredFacts": count, "measuredPayloadBytes": size,
              "scopeInventory":inventory,"hydrationPlannedEqualsMeasured":measured_complete,
              "nativeStageStartObservedBeforeCompletion":native_live,"nativeHeartbeatObservedWhileRunning":native_heartbeat,
              "peakSampledSystemSwapUsedBytes":max((r.get("swapUsedBytes") or 0 for r in resources),default=0),
              "requiredFormerThresholdCrossing": require_old_threshold, "frozenSourceUnchanged":frozen_after,
              "captureRevisionMatchesFixture":revision_matches, "admittedPackage":"admitted-package.json", "firstProgressWhileRunningSeconds": first_live,
              "eventsObservedWhileRunning": len(events), "allProgressEvents": len(complete_events),
              "peakSampledMainProcessResidentBytes": peak_sampled_main_rss,
              "peakTemporaryDisk": temporary_peak, "finalTaskDisk": disk_bytes(root), "freeBeforeBytes": before,
              "minimumVolumeFreeBytes": minimum_free, "maximumVolumeConsumptionSinceStartBytes": before-minimum_free,
              "harnessStopReason": stop_reason, "stdoutValidFinalJSON": stdout_valid,
              "freshnessBaselineAbsent": missing_baseline, "initialTaskDisk": initial_disk,
              "freeAfterBytes": shutil.disk_usage(root).free,
              "limits": ["RSS is sampled for the launcher/main producer PID, not summed across concurrent compiler children; short peaks may be missed.",
                         "Temporary index disk is sampled separately; volume consumption can include unrelated writes; final task bytes are a full stat scan.",
                         "This fixture does not qualify the inaccessible owner repository or authored documentation."]}
    save(logs / "progress.json", complete_events);save(logs / "live-progress.json",live_receipts)
    save(logs / "resource-samples.json",resources);save(logs / "result.json", report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--methods", type=int, default=20)
    parser.add_argument("--steps", type=int, default=100)
    parser.add_argument("--scopes", type=int, default=3)
    parser.add_argument("--clew", type=Path)
    parser.add_argument("--expected-commit")
    parser.add_argument("--archive",type=Path)
    parser.add_argument("--capture-existing", action="store_true")
    parser.add_argument("--semantic", action="store_true")
    parser.add_argument("--require-old-threshold", action="store_true")
    args = parser.parse_args()
    if not args.capture_existing:
        if args.semantic: generate_semantic(args.output)
        else: generate(args.output, args.methods, args.steps, args.scopes)
    report = capture(args.output, args.clew, args.require_old_threshold,args.expected_commit,args.archive) if args.clew else json.loads((args.output/"fixture.json").read_text())
    print(json.dumps(report, indent=2))
    return 0 if report["status"] in ("PASS", "GENERATED_NOT_CAPTURED") else 1


if __name__ == "__main__":
    raise SystemExit(main())
