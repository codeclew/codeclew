#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
python3 -I -S "$ROOT/scripts/stabilization_control.py" guard --gate final-verify >/dev/null

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -- --test-threads=1
python3 -I -S bootstrap/test_clew_bootstrap.py
python3 -I -S scripts/test_gate_safety.py
python3 -I -S scripts/build-trusted-worker-distributions.py --verify-only
python3 -I -S scripts/check_repository_privacy.py
fixtures/kotlin-basic/gradlew -p fixtures/kotlin-basic test --no-daemon --quiet
fixtures/kotlin-2-1/gradlew -p fixtures/kotlin-2-1 test --no-daemon --quiet
fixtures/kotlin-control-flow/gradlew -p fixtures/kotlin-control-flow compileKotlin --no-daemon --quiet
fixtures/kotlin-calls/gradlew -p fixtures/kotlin-calls compileKotlin --no-daemon --quiet
fixtures/kotlin-concurrency/gradlew -p fixtures/kotlin-concurrency compileKotlin --no-daemon --quiet
fixtures/kotlin-maven/mvnw -q -f fixtures/kotlin-maven/pom.xml test

./scripts/demo.sh >/dev/null

printf '%s\n' '{"schema":"codeclew-verification/1.0","status":"PASSED"}'
