#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
mkdir -p benchmarks/reports

python3 -I -S - "$ROOT" <<'PY'
import json
from pathlib import Path
import statistics
import subprocess
import sys
import time

root = Path(sys.argv[1])
clew = root / "clew"
fixture = root / "fixtures/kotlin-basic"

def invoke(*arguments):
    started = time.perf_counter_ns()
    completed = subprocess.run(
        [str(clew), *arguments],
        cwd=root,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=True,
    )
    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
    return json.loads(completed.stdout), elapsed_ms

def p95(values):
    ordered = sorted(values)
    return ordered[max(0, (len(ordered) * 95 + 99) // 100 - 1)]

invoke("doctor")
launcher = [invoke("--bootstrap-warm-audit")[1] for _ in range(20)]
sessions = [
    invoke("session", "open", "--repo", str(root), "--target-ref", "main")[1]
    for _ in range(20)
]

session, _ = invoke("session", "open", "--repo", str(root), "--target-ref", "main")
session_id = session["session"]["sessionId"]
contexts = []
for _ in range(5):
    _, elapsed = invoke(
        "context", "create",
        "--session", session_id,
        "--intent", "inspect total and its tests",
        "--term", "com.acme.total",
    )
    contexts.append(elapsed)

invoke("index", "--repo", str(fixture))
indexes = []
compiler = []
for _ in range(20):
    value, elapsed = invoke("index", "--repo", str(fixture))
    indexes.append(elapsed)
    profile = value.get("compilerIndex")
    if profile:
        compiler.append(profile.get("compilerMicros", 0) / 1000)
preflight = {
    "code": "READY" if compiler else "BACKEND_UNAVAILABLE",
    "configured": True,
    "backendAvailable": bool(compiler),
    "compilerVersion": "2.4.10",
}

measurements = {
    "launcherOverheadP95": p95(launcher),
    "sessionOpenP95": p95(sessions),
    "contextCreateP95": p95(contexts),
    "k24CompilerIndexInternalP95": p95(compiler) if compiler else None,
    "k24IndexEndToEndP95": p95(indexes),
}
slo = {
    "launcherOverhead": measurements["launcherOverheadP95"] <= 1000,
    "sessionOpen": measurements["sessionOpenP95"] <= 2000,
    "contextCreate": measurements["contextCreateP95"] <= 30000,
    "k24CompilerIndexInternal": measurements["k24CompilerIndexInternalP95"] is not None
        and measurements["k24CompilerIndexInternalP95"] <= 300,
    "k24IndexEndToEnd": measurements["k24IndexEndToEndP95"] <= 2000,
}
report = {
    "schema": "codeclew-warm-benchmark/1.0",
    "scope": "warm-single-host-kotlin24",
    "measurementsMs": measurements,
    "compilerIndexPreflight": preflight,
    "sloPassed": slo,
    "passed": all(slo.values()),
}
path = root / "benchmarks/reports/latest.json"
path.write_text(json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n")
print(json.dumps(report, sort_keys=True, separators=(",", ":")))
if not report["passed"]:
    raise SystemExit(1)
PY
