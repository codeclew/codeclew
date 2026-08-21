#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -- --test-threads=2
python3 -I -S scripts/build-trusted-worker-distributions.py --verify-only

./clew --bootstrap-self-test >/dev/null
./clew doctor >/dev/null
./clew --bootstrap-warm-audit | grep -q '"status":"PASSED"'

fixtures/kotlin-basic/gradlew -p fixtures/kotlin-basic test --no-daemon --quiet
fixtures/kotlin-2-1/gradlew -p fixtures/kotlin-2-1 test --no-daemon --quiet
fixtures/kotlin-control-flow/gradlew -p fixtures/kotlin-control-flow compileKotlin --no-daemon --quiet
fixtures/kotlin-calls/gradlew -p fixtures/kotlin-calls compileKotlin --no-daemon --quiet
fixtures/kotlin-concurrency/gradlew -p fixtures/kotlin-concurrency compileKotlin --no-daemon --quiet
fixtures/kotlin-maven/mvnw -q -f fixtures/kotlin-maven/pom.xml test

FIRST=$(./clew project inspect --repo fixtures/kotlin-basic)
SECOND=$(./clew project inspect --repo fixtures/kotlin-basic)
[ "$FIRST" = "$SECOND" ]
./clew index --repo fixtures/kotlin-basic >/dev/null

./scripts/demo.sh >/dev/null
./scripts/benchmark-corpus.sh >/dev/null
./scripts/benchmark.sh >/dev/null

printf '%s\n' '{"schema":"codeclew-verification/1.0","status":"PASSED"}'
