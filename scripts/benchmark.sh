#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
mkdir -p benchmarks/reports
cargo build --quiet --release --bin sthread
cargo build --quiet --release --example stage_benchmark
./gradlew :workers:kotlin:installDist --no-daemon --quiet
BIN="$ROOT/target/release/sthread"
STAGES=$($ROOT/target/release/examples/stage_benchmark)
BENCH_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/semantic-thread-benchmark.XXXXXX")
trap 'rm -rf "$BENCH_ROOT"' EXIT INT TERM
cp -R fixtures/kotlin-basic "$BENCH_ROOT/repo"
rm -rf "$BENCH_ROOT/repo/.semantic-thread" "$BENCH_ROOT/repo/.gradle" "$BENCH_ROOT/repo/build"
git -C "$BENCH_ROOT/repo" init -q -b main
git -C "$BENCH_ROOT/repo" add .
git -C "$BENCH_ROOT/repo" -c user.name=Benchmark -c user.email=benchmark@localhost commit -qm baseline
BASE=$(git -C "$BENCH_ROOT/repo" rev-parse HEAD)
"$BIN" slice --repo "$BENCH_ROOT/repo" --symbol com.acme.total --output "$BENCH_ROOT/thread.json" >/dev/null
jq --arg base "$BASE" '{schema:"semantic-edit/0.1",threadId:.threadId,baseRevision:$base,operations:[{opId:"op:benchmark",kind:"REPLACE_EXPRESSION",target:first(.nodes[]|select(.origin.sourceText=="value *= 2")|.origin),preconditions:{},replacement:{kotlin:"value = value + value"},postconditions:{}}]}' "$BENCH_ROOT/thread.json" > "$BENCH_ROOT/edit.json"
measure() { NAME=$1; shift; RESULT=$(/usr/bin/time -p "$@" 2>&1 >/dev/null | awk '/^real / {print $2}'); printf '%s=%s\n' "$NAME" "$RESULT"; }
{
  measure projectModel "$BIN" project inspect --repo fixtures/kotlin-basic
  measure declarationIndex "$BIN" index --repo fixtures/kotlin-basic
  measure editPreview "$BIN" edit preview --repo "$BENCH_ROOT/repo" --thread "$BENCH_ROOT/thread.json" --operations "$BENCH_ROOT/edit.json"
  measure gradleValidation fixtures/kotlin-basic/gradlew -p fixtures/kotlin-basic compileKotlin --no-daemon --quiet
} | awk -F= -v stages="$STAGES" 'BEGIN{printf "{\"schema\":\"semantic-benchmark/0.2\",\"scope\":\"separately-instrumented-stages-and-e2e-validation\",\"milliseconds\":%s,\"seconds\":{",stages} {printf "%s\"%s\":%s",sep,$1,$2;sep=","} END{print "}}"}' > benchmarks/reports/latest.json
cat benchmarks/reports/latest.json
