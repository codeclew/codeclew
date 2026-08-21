#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
GRADLE_SEED="$ROOT/fixtures/kotlin-basic/.gradle"
./gradlew :workers:kotlin:test :workers:kotlin:installDist :workers:kotlin21:installDist :workers:kotlin23:installDist --no-daemon --quiet
fixtures/kotlin-basic/gradlew -p fixtures/kotlin-basic --gradle-user-home "$GRADLE_SEED" test --no-daemon --quiet
fixtures/kotlin-2-1/gradlew -p fixtures/kotlin-2-1 --gradle-user-home "$GRADLE_SEED" test --no-daemon --quiet
fixtures/kotlin-control-flow/gradlew -p fixtures/kotlin-control-flow --gradle-user-home "$GRADLE_SEED" compileKotlin --no-daemon --quiet
fixtures/kotlin-calls/gradlew -p fixtures/kotlin-calls --gradle-user-home "$GRADLE_SEED" compileKotlin --no-daemon --quiet
fixtures/kotlin-concurrency/gradlew -p fixtures/kotlin-concurrency --gradle-user-home "$GRADLE_SEED" compileKotlin --no-daemon --quiet
MAVEN_SEED="$ROOT/fixtures/kotlin-maven/.semantic-thread/maven-repository"
fixtures/kotlin-maven/mvnw -q -f fixtures/kotlin-maven/pom.xml -Dmaven.repo.local="$MAVEN_SEED" dependency:go-offline test
fixtures/kotlin-maven/mvnw -q -f fixtures/kotlin-maven/pom.xml -Dmaven.repo.local="$MAVEN_SEED" -DskipTests -Doutput="$ROOT/fixtures/kotlin-maven/.semantic-thread/effective-pom.xml" -Dmdep.outputFile="$ROOT/fixtures/kotlin-maven/.semantic-thread/classpath.txt" -Dmdep.includeScope=compile help:effective-pom dependency:build-classpath
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --quiet --bin clew -- doctor >/dev/null
FIRST=$(cargo run --quiet --bin clew -- project inspect --repo fixtures/kotlin-basic)
SECOND=$(cargo run --quiet --bin clew -- project inspect --repo fixtures/kotlin-basic)
[ "$FIRST" = "$SECOND" ]
cargo run --quiet --bin clew -- index --repo fixtures/kotlin-basic >/dev/null
normalize_index_output() {
  python3 -c '
import json, sys
value = json.load(sys.stdin)
for field in ("timing", "workerProfile", "projectModelCache"):
    value.pop(field, None)
json.dump(value, sys.stdout, sort_keys=True, separators=(",", ":"))
'
}
HASH1=$(cargo run --quiet --bin clew -- index --repo fixtures/kotlin-basic | normalize_index_output)
HASH2=$(cargo run --quiet --bin clew -- index --repo fixtures/kotlin-basic | normalize_index_output)
[ "$HASH1" = "$HASH2" ]
./scripts/demo.sh >/dev/null
./scripts/benchmark-corpus.sh >/dev/null
./scripts/benchmark.sh >/dev/null
echo '{"schema":"semantic-verification/0.1","status":"PASSED"}'
