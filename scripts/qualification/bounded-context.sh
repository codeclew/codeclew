#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$ROOT"
umask 077

CONTROL_HOME=${CODECLEW_CONTROL_HOME:-"$HOME/.cache/codeclew-control"}
EVIDENCE_PARENT="$CONTROL_HOME/qualification/bounded-context"
EVIDENCE_ROOT="$EVIDENCE_PARENT/$(git rev-parse --verify HEAD)"
RUST_TARGET="$CONTROL_HOME/build-targets/rust"
mkdir -p "$EVIDENCE_PARENT" "$RUST_TARGET"
mkdir "$EVIDENCE_ROOT"
chmod 700 "$EVIDENCE_PARENT" "$EVIDENCE_ROOT" "$RUST_TARGET"
export CARGO_TARGET_DIR="$RUST_TARGET"

run_stage() {
  stage=$1
  shift
  if "$@" >"$EVIDENCE_ROOT/$stage.stdout" 2>"$EVIDENCE_ROOT/$stage.stderr"; then
    return 0
  fi
  printf '%s\n' "bounded-context failed: $stage" >&2
  return 1
}

run_stage context-projection-contracts \
  cargo test --locked -p clew --lib 'context_v2::tests::' -- --test-threads=1
run_stage bounded-request-contract \
  cargo test --locked -p clew --lib \
    'session::tests::context_request_is_bounded_and_nfc_before_analysis' \
    -- --exact --test-threads=1
run_stage immutable-query-expansion \
  cargo test --locked -p clew --lib \
    'query_v2::tests::tampered_or_cross_index_expansion_fails_closed' \
    -- --exact --test-threads=1
run_stage schema-cutover python3 -I -S - <<'PY'
from pathlib import Path

source = Path("crates/clew/src/context_v2.rs").read_text(encoding="utf-8")
for schema in (
    "codeclew-bounded-context/4.0",
    "codeclew-bounded-context-evidence/4.0",
    "codeclew-bounded-context-projection/4.0",
):
    if schema not in source:
        raise SystemExit("bounded context v4 schema cutover is incomplete")
if "codeclew-bounded-context/3.0" in source or '"windows":projected_windows' not in source:
    raise SystemExit("legacy or single-window context projection remains")
PY

python3 -I -S - "$EVIDENCE_ROOT" <<'PY'
import hashlib
import json
import os
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
qualification = {
    "contracts": [
        "STDOUT_ENVELOPE_AT_MOST_64_KIB",
        "PROJECTION_TARGET_AT_MOST_54_KIB",
        "IMMUTABLE_FULL_EVIDENCE_CAS",
        "ONE_CONTENT_AUTHORITY_PER_FILE",
        "BOUNDED_MULTI_WINDOW_SOURCE_PROJECTION",
        "EXACT_COMPILER_RANGE_BEATS_TEXT_IMPORT",
        "PARENT_AND_QUERY_INDEX_TAMPER_REFUSAL",
        "COMPILATION_AND_COMPLETENESS_PROVENANCE",
    ],
    "buildToolsStarted": [],
    "logDigests": {
        path.name: "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(root.iterdir(), key=lambda value: value.name)
        if path.is_file() and path.suffix in {".stdout", ".stderr"}
    },
    "schema": "codeclew-bounded-context-qualification/1.0",
    "status": "PASS",
}
temporary = root / ".qualification.json.tmp"
temporary.write_text(
    json.dumps(qualification, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
os.chmod(temporary, 0o400)
os.replace(temporary, root / "qualification.json")
print(json.dumps(qualification, sort_keys=True, separators=(",", ":")))
PY
