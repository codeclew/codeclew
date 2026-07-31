#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
command -v jq >/dev/null 2>&1 || { echo 'jq is required for the demo' >&2; exit 2; }
cd "$ROOT"
cargo build --quiet --bin sthread
./gradlew :workers:kotlin:installDist --no-daemon --quiet
DEMO_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/semantic-thread-demo.XXXXXX")
trap 'rm -rf "$DEMO_ROOT"' EXIT INT TERM
cp -R fixtures/kotlin-basic "$DEMO_ROOT/repo"
cd "$DEMO_ROOT/repo"
git init -q -b main
git add .
git -c user.name=Demo -c user.email=demo@localhost commit -qm baseline
BASE=$(git rev-parse HEAD)
cd "$ROOT"
BIN="$ROOT/target/debug/sthread"
"$BIN" slice --repo "$DEMO_ROOT/repo" --symbol com.acme.total --direction both --output "$DEMO_ROOT/thread.json" >/dev/null
jq --arg base "$BASE" '{schema:"semantic-edit/0.1",threadId:.threadId,baseRevision:$base,operations:[{opId:"op:1",kind:"REPLACE_EXPRESSION",target:(.nodes[]|select(.origin.sourceText=="value *= 2")|.origin),preconditions:{},replacement:{kotlin:"value = value + value"},postconditions:{}}]}' "$DEMO_ROOT/thread.json" > "$DEMO_ROOT/edit.json"
jq '.operations[0].replacement.kotlin="value ="' "$DEMO_ROOT/edit.json" > "$DEMO_ROOT/invalid-edit.json"
set +e
"$BIN" edit preview --repo "$DEMO_ROOT/repo" --thread "$DEMO_ROOT/thread.json" --operations "$DEMO_ROOT/invalid-edit.json" > "$DEMO_ROOT/invalid.json"
INVALID_EXIT=$?
set -e
[ "$INVALID_EXIT" -eq 6 ]
[ "$BASE" = "$(git -C "$DEMO_ROOT/repo" rev-parse refs/heads/main)" ]
"$BIN" edit preview --repo "$DEMO_ROOT/repo" --thread "$DEMO_ROOT/thread.json" --operations "$DEMO_ROOT/edit.json" --output "$DEMO_ROOT/preview.json" >/dev/null
jq -n --slurpfile thread "$DEMO_ROOT/thread.json" --slurpfile edit "$DEMO_ROOT/edit.json" --arg base "$BASE" '{schema:"semantic-transaction/0.1",txId:"tx:demo-valid",actorId:"agent:demo",intent:"rewrite premium multiplication equivalently",baseRevision:$base,projectModelHash:$thread[0].snapshot.projectModelHash,status:"CREATED",thread:$thread[0],edit:$edit[0],testTasks:["test"]}' > "$DEMO_ROOT/tx.json"
"$BIN" tx commit --repo "$DEMO_ROOT/repo" --transaction "$DEMO_ROOT/tx.json" --target-ref refs/heads/main > "$DEMO_ROOT/commit.json"
FINAL=$(git -C "$DEMO_ROOT/repo" rev-parse refs/heads/main)
git -C "$DEMO_ROOT/repo" show -s --format=%B "$FINAL" | grep -q 'Semantic-Transaction-Id: tx:demo-valid'
set +e
"$BIN" tx commit --repo "$DEMO_ROOT/repo" --transaction "$DEMO_ROOT/tx.json" --target-ref refs/heads/main > "$DEMO_ROOT/conflict.json"
CONFLICT_EXIT=$?
set -e
[ "$CONFLICT_EXIT" -eq 5 ]
[ "$FINAL" = "$(git -C "$DEMO_ROOT/repo" rev-parse refs/heads/main)" ]
jq -n --slurpfile invalid "$DEMO_ROOT/invalid.json" --slurpfile commit "$DEMO_ROOT/commit.json" --slurpfile conflict "$DEMO_ROOT/conflict.json" '{schema:"semantic-demo/0.1",status:"PASSED",invalidReplacement:$invalid[0],commit:$commit[0],conflict:$conflict[0],branchUnchangedAfterFailure:true,branchUnchangedAfterConflict:true}'
