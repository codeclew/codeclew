#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
./gradlew :workers:kotlin:test :workers:kotlin:installDist --no-daemon --quiet
fixtures/kotlin-basic/gradlew -p fixtures/kotlin-basic test --no-daemon --quiet
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --quiet --bin sthread -- doctor >/dev/null
FIRST=$(cargo run --quiet --bin sthread -- project inspect --repo fixtures/kotlin-basic)
SECOND=$(cargo run --quiet --bin sthread -- project inspect --repo fixtures/kotlin-basic)
[ "$FIRST" = "$SECOND" ]
cargo run --quiet --bin sthread -- index --repo fixtures/kotlin-basic >/dev/null
HASH1=$(cargo run --quiet --bin sthread -- index --repo fixtures/kotlin-basic)
HASH2=$(cargo run --quiet --bin sthread -- index --repo fixtures/kotlin-basic)
[ "$HASH1" = "$HASH2" ]
./scripts/demo.sh >/dev/null
echo '{"schema":"semantic-verification/0.1","status":"PASSED"}'
