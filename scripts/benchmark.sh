#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
mkdir -p benchmarks/reports
cargo build --quiet --release --bin sthread
cargo build --quiet --release --example stage_benchmark
./gradlew :workers:kotlin:installDist --no-daemon --quiet
STAGES=$($ROOT/target/release/examples/stage_benchmark)
GRADLE_SECONDS=$(/usr/bin/time -p fixtures/kotlin-basic/gradlew -p fixtures/kotlin-basic compileKotlin --no-daemon --quiet 2>&1 >/dev/null | awk '/^real / {print $2}')
jq -n --argjson stages "$STAGES" --arg gradle "$GRADLE_SECONDS" '{schema:"semantic-benchmark/0.3",scope:"isolated-clean-fixture-real-semantic-p95",milliseconds:$stages,seconds:{gradleValidation:($gradle|tonumber)}}' > benchmarks/reports/latest.json
jq -e '.milliseconds.sloPassed | all' benchmarks/reports/latest.json >/dev/null
cat benchmarks/reports/latest.json
