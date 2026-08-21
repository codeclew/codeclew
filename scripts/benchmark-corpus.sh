#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
CORPUS_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/semantic-thread-corpus.XXXXXX")
trap 'rm -rf "$CORPUS_ROOT"' EXIT INT TERM
mkdir "$CORPUS_ROOT/repo"
git archive HEAD fixtures/kotlin-basic | tar -x -C "$CORPUS_ROOT/repo" --strip-components=2
CORPUS_FILE="$CORPUS_ROOT/repo/src/main/kotlin/com/acme/GeneratedCorpus.kt"
awk 'BEGIN { print "package com.acme"; print ""; for (i=0; i<25000; i++) { print "fun generated" i "(input: Int): Int {"; print "    val doubled = input * 2"; print "    return doubled + " i; print "}" } }' > "$CORPUS_FILE"
LINES=$(wc -l < "$CORPUS_FILE" | tr -d ' ')
ELAPSED_SECONDS=$(/usr/bin/time -p "$ROOT/clew" index --repo "$CORPUS_ROOT/repo" --compilation :/main --syntax-only 2>&1 >/dev/null | awk '/^real / {print $2}')
MILLISECONDS=$(awk -v seconds="$ELAPSED_SECONDS" 'BEGIN { printf "%.0f", seconds * 1000 }')
mkdir -p benchmarks/reports
jq -n --argjson lines "$LINES" --argjson milliseconds "$MILLISECONDS" '{schema:"semantic-corpus-benchmark/0.1",mode:"cold-syntax-declaration-index",kotlinLoc:$lines,milliseconds:$milliseconds,sloMilliseconds:20000,passed:($milliseconds <= 20000)}' > benchmarks/reports/corpus-100k.json
[ "$(jq -r '.passed' benchmarks/reports/corpus-100k.json)" = true ]
cat benchmarks/reports/corpus-100k.json
