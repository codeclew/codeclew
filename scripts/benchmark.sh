#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
mkdir -p benchmarks/reports

BENCH_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/codeclew-benchmark.XXXXXX")
BENCH_ROOT=$(CDPATH= cd -- "$BENCH_ROOT" && pwd -P)
trap 'rm -rf "$BENCH_ROOT"' EXIT INT TERM
mkdir "$BENCH_ROOT/repository"
git archive HEAD fixtures/kotlin-basic |
  tar -x -C "$BENCH_ROOT/repository" --strip-components=2
git init -q -b main "$BENCH_ROOT/repository"
git -C "$BENCH_ROOT/repository" add .
git -C "$BENCH_ROOT/repository" \
  -c user.name='Codeclew Benchmark' \
  -c user.email='benchmark@codeclew.invalid' \
  commit -q -m baseline

CODECLEW_HOME="$BENCH_ROOT/state" \
python3 -I -S - "$ROOT" "$BENCH_ROOT/repository" <<'PY'
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import time

root = Path(sys.argv[1])
repository = Path(sys.argv[2])
clew = root / "clew"
environment = dict(os.environ)
environment["CODECLEW_HOME"] = str(repository.parent / "state")


def invoke(arguments, *, parse_json=True):
    started = time.perf_counter_ns()
    completed = subprocess.run(
        [str(clew), *arguments],
        cwd=root,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
    if completed.returncode != 0:
        raise RuntimeError(
            f"clew command failed ({completed.returncode}): {' '.join(arguments)}\n"
            + completed.stdout.decode(errors="replace")
            + completed.stderr.decode(errors="replace")
        )
    value = json.loads(completed.stdout) if parse_json else None
    return value, elapsed_ms, completed.stderr.decode(errors="replace")


def p95(values):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * 0.95) - 1)]


_, cold_bootstrap, cold_bootstrap_events = invoke(["--help"], parse_json=False)
session, cold_session, _ = invoke([
    "session", "open",
    "--repo", str(repository),
    "--target-ref", "main",
    "--compilation", ":/main",
])
session_id = session["session"]["sessionId"]
context_arguments = [
    "context", "create",
    "--session", session_id,
    "--intent", "inspect total and its tests",
    "--term", "total",
    "--term", "SamplesTest",
]
context, cold_context, _ = invoke(context_arguments)

bootstrap_audits = []
bootstrap_audit_times = []
warm_events = []
for _ in range(3):
    value, elapsed, events = invoke(["--bootstrap-warm-audit"])
    bootstrap_audits.append(value)
    bootstrap_audit_times.append(elapsed)
    warm_events.append(events)

launcher = []
for _ in range(20):
    _, elapsed, events = invoke(["--help"], parse_json=False)
    launcher.append(elapsed)
    warm_events.append(events)

sessions = []
for _ in range(20):
    _, elapsed, events = invoke([
        "session", "open",
        "--repo", str(repository),
        "--target-ref", "main",
        "--compilation", ":/main",
    ])
    sessions.append(elapsed)
    warm_events.append(events)

contexts = []
context_ids = set()
for _ in range(20):
    value, elapsed, events = invoke(context_arguments)
    contexts.append(elapsed)
    context_ids.add(value["contextId"])
    warm_events.append(events)

forbidden_markers = (
    '"event":"STAGE_STARTED"',
    "Compiling ",
    "Finished release profile",
    "Gradle build daemon",
)
forbidden_observed = sorted({
    marker
    for events in warm_events
    for marker in forbidden_markers
    if marker in events
})
measurements = {
    "coldBootstrap": cold_bootstrap,
    "coldSessionOpen": cold_session,
    "coldContextCreate": cold_context,
    "launcherOverheadP95": p95(launcher),
    "sessionOpenP95": p95(sessions),
    "contextCreateP95": p95(contexts),
    "bootstrapMetadataAuditP95": p95(bootstrap_audit_times),
}
metadata_only_bootstrap = all(
    audit.get("status") == "PASSED"
    and audit.get("counters", {}).get("processRuns") == 0
    and audit.get("counters", {}).get("digestFileCalls") == 0
    and audit.get("counters", {}).get("checkpointHits", 0) >= 1
    for audit in bootstrap_audits
)
slo = {
    "launcherOverhead": measurements["launcherOverheadP95"] <= 1000,
    "sessionOpen": measurements["sessionOpenP95"] <= 2000,
    "contextCreate": measurements["contextCreateP95"] <= 30000,
    "stableContextIdentity": len(context_ids) == 1,
    "noWarmBuildEvents": not forbidden_observed,
    "metadataOnlyBootstrap": metadata_only_bootstrap,
}
report = {
    "schema": "codeclew-managed-warm-benchmark/2.0",
    "scope": "fresh-kotlin24-managed-session",
    "samples": 20,
    "measurementsMs": measurements,
    "coldBootstrapObserved": '"event":"STAGE_STARTED"' in cold_bootstrap_events,
    "contextCompleteness": context["completeness"],
    "warmForbiddenBuildMarkers": forbidden_observed,
    "bootstrapWarmAudits": bootstrap_audits,
    "sloPassed": slo,
    "passed": all(slo.values()),
}
path = root / "benchmarks/reports/latest.json"
path.write_text(
    json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
print(json.dumps(report, sort_keys=True, separators=(",", ":")))
if not report["passed"]:
    raise SystemExit(1)
PY
