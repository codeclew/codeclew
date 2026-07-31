#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
mkdir -p benchmarks/reports
cargo build --quiet --release --bin sthread
./gradlew :workers:kotlin:installDist --no-daemon --quiet
BIN="$ROOT/target/release/sthread"
measure() { NAME=$1; shift; RESULT=$(/usr/bin/time -p "$@" 2>&1 >/dev/null | awk '/^real / {print $2}'); printf '%s=%s\n' "$NAME" "$RESULT"; }
{
  measure doctor "$BIN" doctor
  measure inspect "$BIN" project inspect --repo fixtures/kotlin-basic
  measure index "$BIN" index --repo fixtures/kotlin-basic
  measure cfg "$BIN" cfg --repo fixtures/kotlin-basic --symbol com.acme.total
} | awk -F= 'BEGIN{print "{\"schema\":\"semantic-benchmark/0.1\",\"seconds\":{"} {printf "%s\"%s\":%s",sep,$1,$2;sep=","} END{print "}}"}' > benchmarks/reports/latest.json
cat benchmarks/reports/latest.json

