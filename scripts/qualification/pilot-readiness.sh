#!/bin/sh
set -eu
umask 077

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"

QUALIFICATION_HOME=$(python3 -I -S -c 'import os, pwd; print(pwd.getpwuid(os.geteuid()).pw_dir)')
QUALIFICATION_ROOT=$(mktemp -d "$QUALIFICATION_HOME/.codeclew-pilot-qualification.XXXXXX")
chmod 700 "$QUALIFICATION_ROOT"
PRESERVE_STATE=0
cleanup() {
  if [ "$PRESERVE_STATE" -eq 0 ]; then
    # Verified runtime capsules are deliberately owner-read-only. Restore only
    # the owner's write bit inside this private qualification root before the
    # temporary state is removed.
    chmod -R u+w -- "$QUALIFICATION_ROOT"
    rm -rf -- "$QUALIFICATION_ROOT"
  fi
}
trap cleanup EXIT HUP INT TERM
CODECLEW_HOME=$QUALIFICATION_ROOT/state
export CODECLEW_HOME

./clew --version >/dev/null
./clew --bootstrap-warm-audit | python3 -I -S -c '
import json, sys
value = json.load(sys.stdin)
counters = value.get("counters", {})
if (
    value.get("schema") != "codeclew-bootstrap-warm-audit/2.0"
    or value.get("status") != "PASSED"
    or value.get("coldToolchainInvoked") is not False
    or value.get("capsuleBuildInvoked") is not False
    or counters.get("processRuns") != 0
    or counters.get("digestFileCalls") != 0
):
    raise SystemExit("warm runtime audit failed")
'
if python3 -I -S scripts/pilot.py --reuse-primed-runtime; then
  result=0
else
  result=$?
fi
if [ "$result" -eq 2 ]; then
  PRESERVE_STATE=1
fi
exit "$result"
