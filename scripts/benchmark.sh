#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
mkdir -p benchmarks/reports
cargo build --quiet --release --bin sthread
cargo build --quiet --release --example stage_benchmark
./gradlew :workers:kotlin:installDist --no-daemon --quiet
STAGES=$($ROOT/target/release/examples/stage_benchmark)
GRADLE_COMPILE_SECONDS=$(/usr/bin/time -p fixtures/kotlin-basic/gradlew -p fixtures/kotlin-basic compileKotlin --no-daemon --quiet 2>&1 >/dev/null | awk '/^real / {print $2}')
GRADLE_TEST_SECONDS=$(/usr/bin/time -p fixtures/kotlin-basic/gradlew -p fixtures/kotlin-basic test --no-daemon --quiet 2>&1 >/dev/null | awk '/^real / {print $2}')
jq -n --argjson stages "$STAGES" --arg compile "$GRADLE_COMPILE_SECONDS" --arg tests "$GRADLE_TEST_SECONDS" '{schema:"semantic-benchmark/0.3",scope:"isolated-clean-fixture-real-semantic-p95",milliseconds:$stages,seconds:{gradleCompile:($compile|tonumber),gradleTests:($tests|tonumber)}}' > benchmarks/reports/latest.json
jq -e '.milliseconds.sloPassed | all' benchmarks/reports/latest.json >/dev/null
jq -e '[.milliseconds.workerStartup,.milliseconds.ipcMicrosP95,.milliseconds.protocolSerializationMicrosP95,.milliseconds.psiParseMicrosP95,.milliseconds.k2SemanticAnalysisColdMicros,.milliseconds.k2ChangedFileAnalysisMicrosP95,.milliseconds.firCfgExtractionMicrosP95,.milliseconds.rustGraphConstructionMicrosP95,.milliseconds.ssaAndControlMicrosP95,.milliseconds.boundedSliceP95,.milliseconds.editPreviewP95,.seconds.gradleCompile,.seconds.gradleTests] | all(. > 0)' benchmarks/reports/latest.json >/dev/null
cat benchmarks/reports/latest.json
