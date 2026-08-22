#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$ROOT"
umask 077

CONTROL_HOME=${CODECLEW_CONTROL_HOME:-"$HOME/.cache/codeclew-control"}
EVIDENCE_PARENT="$CONTROL_HOME/qualification/generation-reuse"
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
  printf '%s\n' "generation-reuse failed: $stage" >&2
  return 1
}

# These are deterministic component contracts. Real worker/build-tool and
# end-to-end performance checks remain deferred to Q1/Q2.
run_stage incremental-planning \
  cargo test --locked -p clew --lib 'incremental_v2::tests::' -- --test-threads=1
run_stage stable-compiler-store \
  cargo test --locked -p clew --lib 'generation_service::tests::compiler_store_' -- --test-threads=1
run_stage corrupt-head-refusal \
  cargo test --locked -p clew --lib \
    'generation_service::tests::corrupt_incremental_head_forces_invalid_receipt_full_plan' \
    -- --exact --test-threads=1
run_stage immutable-generation-key \
  cargo test --locked -p clew --lib \
    'generation_service::tests::final_generation_key_is_fixed_before_output_completeness' \
    -- --exact --test-threads=1
run_stage deterministic-query-index \
  cargo test --locked -p clew --lib \
    'query_v2::tests::index_is_deterministic_and_query_reads_only_term_buckets' \
    -- --exact --test-threads=1
run_stage cross-index-refusal \
  cargo test --locked -p clew --lib \
    'query_v2::tests::tampered_or_cross_index_expansion_fails_closed' \
    -- --exact --test-threads=1
run_stage tracked-model-policy \
  cargo test --locked -p clew --lib \
    'session::tests::tracked_model_cache_requires_a_canonical_head_bound_manifest' \
    -- --exact --test-threads=1
run_stage model-mode-refusal \
  cargo test --locked -p clew --lib \
    'session::tests::model_cache_modes_reject_mixed_or_non_release_authority' \
    -- --exact --test-threads=1

python3 -I -S - "$EVIDENCE_ROOT" <<'PY'
import hashlib
import json
import os
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
qualification = {
    "contracts": [
        "STABLE_COMPILER_STORE_KEY",
        "CONFIG_OR_UNSURE_REFUSES_REUSE",
        "EXACT_UNCHANGED_GENERATION_REUSE",
        "CORRUPT_HEAD_FORCES_FULL",
        "IMMUTABLE_GENERATION_KEY",
        "DETERMINISTIC_QUERY_INDEX_CAS",
        "CROSS_INDEX_TAMPER_REFUSAL",
        "TRACKED_OR_SEALED_MODEL_POLICY_ONLY",
    ],
    "buildToolsStarted": [],
    "logDigests": {
        path.name: "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(root.iterdir(), key=lambda value: value.name)
        if path.is_file() and path.suffix in {".stdout", ".stderr"}
    },
    "schema": "codeclew-generation-reuse-qualification/1.0",
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
