#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
./gradlew :workers:kotlin:test :workers:kotlin:installDist :workers:kotlin21:installDist --no-daemon --quiet
fixtures/kotlin-basic/gradlew -p fixtures/kotlin-basic test --no-daemon --quiet
fixtures/kotlin-2-1/gradlew -p fixtures/kotlin-2-1 compileKotlin --no-daemon --quiet
fixtures/kotlin-control-flow/gradlew -p fixtures/kotlin-control-flow compileKotlin --no-daemon --quiet
fixtures/kotlin-calls/gradlew -p fixtures/kotlin-calls compileKotlin --no-daemon --quiet
fixtures/kotlin-concurrency/gradlew -p fixtures/kotlin-concurrency compileKotlin --no-daemon --quiet
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --quiet --bin clew -- doctor >/dev/null
FIRST=$(cargo run --quiet --bin clew -- project inspect --repo fixtures/kotlin-basic)
SECOND=$(cargo run --quiet --bin clew -- project inspect --repo fixtures/kotlin-basic)
[ "$FIRST" = "$SECOND" ]
cargo run --quiet --bin clew -- index --repo fixtures/kotlin-basic >/dev/null
HASH1=$(cargo run --quiet --bin clew -- index --repo fixtures/kotlin-basic)
HASH2=$(cargo run --quiet --bin clew -- index --repo fixtures/kotlin-basic)
[ "$HASH1" = "$HASH2" ]
./scripts/demo.sh >/dev/null
./scripts/benchmark-corpus.sh >/dev/null
./scripts/benchmark.sh >/dev/null
echo '{"schema":"semantic-verification/0.1","status":"PASSED"}'
