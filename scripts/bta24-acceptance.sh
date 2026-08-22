#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

./gradlew :workers:kotlin:test \
  --tests 'dev.semanticthread.worker.BtaIncrementalBackend21Test' \
  --tests 'dev.semanticthread.worker.K2FactGenerationStore21Test' \
  --no-daemon \
  --quiet

cargo test -p clew \
  kotlin_adapter_v2::tests::k24_real_bta_acceptance_matrix \
  -- --ignored --exact --nocapture

cargo test -p clew \
  generation_service::tests::delta_plan_executes_full_until_subset_protocol_exists \
  -- --exact

cargo test -p clew \
  generation_service::tests::corrupt_incremental_head_forces_invalid_receipt_full_plan \
  -- --exact

cargo test -p clew \
  generation_service::tests::compiler_index_receipt_is_per_file_cross_boundary_and_corruption_refuses \
  -- --exact

cargo test -p clew \
  incremental_v2::tests::unchanged_and_reverse_dependency_delta_are_exact \
  -- --exact

printf '%s\n' '{"schema":"codeclew-bta24-acceptance/1.0","status":"PASSED"}'
