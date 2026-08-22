#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
mkdir -p benchmarks/reports
umask 077

PHYSICAL_CORES=$(python3 -I -S - <<'PY'
import subprocess


def macos_physical_cores():
    try:
        value = subprocess.run(
            ["sysctl", "-n", "hw.physicalcpu"],
            check=True,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        ).stdout.strip()
        return int(value)
    except (FileNotFoundError, subprocess.SubprocessError, ValueError):
        return None


def linux_physical_cores():
    try:
        pairs = set()
        physical = None
        core = None
        with open("/proc/cpuinfo", encoding="utf-8") as stream:
            for line in stream:
                if not line.strip():
                    if physical is not None and core is not None:
                        pairs.add((physical, core))
                    physical = core = None
                elif line.startswith("physical id"):
                    physical = line.split(":", 1)[1].strip()
                elif line.startswith("core id"):
                    core = line.split(":", 1)[1].strip()
        if physical is not None and core is not None:
            pairs.add((physical, core))
        return len(pairs) or None
    except OSError:
        return None


print(macos_physical_cores() or linux_physical_cores() or 0)
PY
)

REPORT=benchmarks/reports/cold-multicore-latest.json
TEMPORARY="$REPORT.tmp.$$"
trap 'rm -f -- "$TEMPORARY"' EXIT INT TERM
cargo test --quiet --locked --example cold_multicore_gate
cargo run --quiet --locked --example cold_multicore_gate -- "$ROOT" "$PHYSICAL_CORES" >"$TEMPORARY"
python3 -I -S - "$TEMPORARY" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
value = json.loads(path.read_text(encoding="utf-8"))
if value.get("schema") != "codeclew-cold-multicore-gate/2.0":
    raise SystemExit("cold multicore gate returned an unexpected schema")
status = value.get("status")
if value.get("accepted") is not True or status not in {
    "PASSED",
    "SKIPPED_UNQUALIFIED_HOST",
}:
    raise SystemExit("cold multicore gate failed")
if status == "SKIPPED_UNQUALIFIED_HOST" and value.get("releaseGatePassed") is not False:
    raise SystemExit("unqualified multicore host falsely passed the release gate")
PY
mv -f -- "$TEMPORARY" "$REPORT"
chmod 600 "$REPORT"
trap - EXIT INT TERM
cat "$REPORT"
