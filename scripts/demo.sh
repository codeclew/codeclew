#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

json_get() {
  python3 -I -S -c '
import json, sys
value = json.load(sys.stdin)
for part in sys.argv[1].split("."):
    value = value[part]
print(value)
' "$1"
}

DEMO_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/codeclew-demo.XXXXXX")
trap 'rm -rf "$DEMO_ROOT"' EXIT INT TERM
git archive HEAD fixtures/kotlin-basic | tar -x -C "$DEMO_ROOT" --strip-components=2
git init -q -b main "$DEMO_ROOT"
git -C "$DEMO_ROOT" add .
git -C "$DEMO_ROOT" -c user.name=Demo -c user.email=demo@localhost commit -q -m baseline
BASE=$(git -C "$DEMO_ROOT" rev-parse HEAD)

SESSION_JSON=$(./clew session open --repo "$DEMO_ROOT" --target-ref main)
SESSION_ID=$(printf '%s' "$SESSION_JSON" | json_get session.sessionId)

CONTEXT_JSON=$(./clew context create \
  --session "$SESSION_ID" \
  --intent 'add one compile-checked marker beside total and preserve existing tests' \
  --term com.acme.total \
  --term SamplesTest)
CONTEXT_ID=$(printf '%s' "$CONTEXT_JSON" | json_get contextId)
[ "$(printf '%s' "$CONTEXT_JSON" | json_get completeness.status)" = COMPLETE_TASK ]

PLAN_JSON=$(./clew plan validate \
  --session "$SESSION_ID" \
  --context "$CONTEXT_ID" \
  --plan fixtures/session/create-demo-marker-plan.json)
PLAN_ID=$(printf '%s' "$PLAN_JSON" | json_get planId)

START_JSON=$(./clew task-run start \
  --session "$SESSION_ID" \
  --context "$CONTEXT_ID" \
  --plan "$PLAN_ID")
RUN_ID=$(printf '%s' "$START_JSON" | json_get run.runId)

attempt=0
while [ "$attempt" -lt 300 ]; do
  STATUS_JSON=$(./clew task-run status --run "$RUN_ID")
  STATUS=$(printf '%s' "$STATUS_JSON" | json_get run.status)
  case "$STATUS" in
    CREATED|PREPARING)
      sleep 0.2
      ;;
    *)
      break
      ;;
  esac
  attempt=$((attempt + 1))
done
[ "$STATUS" = READY_TO_PUBLISH ]

PUBLISH_JSON=$(./clew session publish --session "$SESSION_ID" --run "$RUN_ID")
[ "$(printf '%s' "$PUBLISH_JSON" | json_get run.status)" = PUBLISHED ]
[ -f "$DEMO_ROOT/src/main/kotlin/com/acme/CodeclewDemoMarker.kt" ]
[ "$BASE" != "$(git -C "$DEMO_ROOT" rev-parse HEAD)" ]

printf '%s\n' '{"schema":"codeclew-demo/1.0","status":"PASSED"}'
