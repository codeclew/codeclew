#!/bin/sh
set -eu
umask 077

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

ci_usage() {
  printf '%s\n' 'Usage: scripts/ci-verify.sh [--csharp-only | --suite checks|retained-docs|native-pages|managed-docs|java|kotlin]' >&2
  exit 1
}

CLEW_CI_SUITE=all
case "${1-}" in
  '') test "$#" -eq 0 || ci_usage ;;
  --csharp-only) test "$#" -eq 1 || ci_usage; CLEW_CI_SUITE=csharp-only ;;
  --suite)
    test "$#" -eq 2 || ci_usage
    CLEW_CI_SUITE=$2
    case "$CLEW_CI_SUITE" in
      checks|retained-docs|native-pages|managed-docs|java|kotlin) ;;
      *) ci_usage ;;
    esac
    ;;
  *) ci_usage ;;
esac

export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 DOTNET_SKIP_FIRST_TIME_EXPERIENCE=1
export DOTNET_CLI_USE_MSBUILD_SERVER=0 MSBUILDDISABLENODEREUSE=1

# Full verification and every release architecture must exercise the C# preview.
if [ "$CLEW_CI_SUITE" = all ] || [ "$CLEW_CI_SUITE" = checks ] || [ "$CLEW_CI_SUITE" = csharp-only ]; then
  if ! command -v dotnet >/dev/null 2>&1; then
    printf '%s\n' 'C# verification requires an installed .NET SDK 10 or newer' >&2
    exit 1
  fi
  CSHARP_SDK_VERSION=$(cd workers/dotnet && dotnet --version)
  CSHARP_SDK_MAJOR=${CSHARP_SDK_VERSION%%.*}
  case "$CSHARP_SDK_MAJOR" in
    ''|*[!0-9]*) printf '%s\n' 'C# verification could not identify the .NET SDK major version' >&2; exit 1 ;;
  esac
  if [ "$CSHARP_SDK_MAJOR" -lt 10 ]; then
    printf 'C# verification requires .NET SDK 10 or newer; found %s\n' "$CSHARP_SDK_VERSION" >&2
    exit 1
  fi
  printf 'C# verification SDK: %s\n' "$CSHARP_SDK_VERSION"
fi

CI_TMP_BASE=$(python3 -I -S -c 'import os, pwd; print(pwd.getpwuid(os.geteuid()).pw_dir)')
TEST_TMP_ROOT=$(mktemp -d "$CI_TMP_BASE/.codeclew-ci.XXXXXX")
chmod 700 "$TEST_TMP_ROOT"
trap 'rm -rf -- "$TEST_TMP_ROOT"' EXIT HUP INT TERM
TMPDIR=$TEST_TMP_ROOT
export TMPDIR

qualify_csharp() {
  set -- dotnet publish workers/dotnet/src/Codeclew.CSharp.Analyzer.csproj --configuration Release \
    --output workers/dotnet/publish -p:UseAppHost=false -p:RestoreLockedMode=true \
    -nodeReuse:false -p:UseSharedCompilation=false
  if [ -n "${CODECLEW_DOTNET_PACKAGE_SOURCE-}" ]; then
    set -- "$@" --source "$CODECLEW_DOTNET_PACKAGE_SOURCE"
  fi
  "$@"
  test -f workers/dotnet/publish/Codeclew.CSharp.Analyzer.dll
  python3 -I -S scripts/build-trusted-worker-distributions.py --variant csharp --verify-only
  CODECLEW_TEST_CSHARP_WORKER="$ROOT/workers/dotnet/publish/Codeclew.CSharp.Analyzer.dll"
  export CODECLEW_TEST_CSHARP_WORKER
  CSHARP_INTEGRATION_TEST=csharp_project_model::tests::fixture_solution_yields_roslyn_facts_routes_and_restore_boundaries
  cargo test --locked -p clew --lib "$CSHARP_INTEGRATION_TEST" \
    -- --exact --ignored --list >"$TEST_TMP_ROOT/csharp-integration-tests.txt"
  if ! grep -Fx "$CSHARP_INTEGRATION_TEST: test" "$TEST_TMP_ROOT/csharp-integration-tests.txt" >/dev/null; then
    printf '%s\n' 'The real ignored C# integration test is missing; refusing a zero-test qualification' >&2
    exit 1
  fi
  cargo test --locked -p clew --lib "$CSHARP_INTEGRATION_TEST" \
    -- --exact --ignored --test-threads=1
}

if [ "${1-}" = --csharp-only ]; then
  qualify_csharp
  printf '%s\n' '{"schema":"codeclew-verification/1.0","status":"PASSED","scope":"CSHARP"}'
  exit 0
fi

if { [ "$CLEW_CI_SUITE" = all ] || [ "$CLEW_CI_SUITE" = kotlin ]; } &&
   { [ -z "${CODECLEW_TEST_JDK25_HOME-}" ] || [ ! -x "$CODECLEW_TEST_JDK25_HOME/bin/java" ]; }; then
  printf '%s\n' 'Full verification requires CODECLEW_TEST_JDK25_HOME pointing to JDK 25; keep the build/analyzer JDK at 21' >&2
  exit 1
fi

ci_run() {
  CLEW_CI_SECTION=$1
  shift
  if [ "$CLEW_CI_SUITE" = all ] || [ "$CLEW_CI_SUITE" = "$CLEW_CI_SECTION" ]; then
    CLEW_CI_STARTED=$(date +%s)
    printf '[ci] START %s: %s\n' "$CLEW_CI_SECTION" "$*"
    "$@"
    printf '[ci] COMPLETE %s: %ss\n' "$CLEW_CI_SECTION" "$(($(date +%s) - CLEW_CI_STARTED))"
  fi
}

ci_run checks python3 -I -S scripts/test_pilot_case_record.py
ci_run checks python3 -I -S scripts/test_ci_verify.py
ci_run checks python3 -I -S scripts/test_ci_reuse.py
ci_run checks python3 -I -S bootstrap/test_clew_bootstrap.py
ci_run checks python3 -I -S scripts/test_pilot_release_gate.py
ci_run checks python3 -I -S scripts/test_language_mutation_pilot.py
ci_run checks python3 -I -S scripts/test_check_repository_privacy.py
ci_run checks python3 -I -S scripts/test_check_english_content.py
ci_run checks python3 -I -S scripts/check_english_content.py
ci_run checks python3 -I -S scripts/test_documentation_selection_contract.py
ci_run checks python3 -I -S scripts/test_first_document.py
ci_run checks python3 -I -S scripts/build_cli_documentation.py --check
ci_run checks python3 -I -S scripts/test_agent_skill.py
ci_run checks python3 -I -S scripts/test_prepared_source.py
ci_run checks python3 -I -S scripts/test_pilot.py
ci_run checks python3 -I -S scripts/test_macos_distribution.py
ci_run checks python3 -I -S scripts/test_support_diagnostics.py
ci_run checks python3 -I -S scripts/test_build_macos_release.py
ci_run checks python3 -I -S scripts/test_build_local_release_candidate.py
ci_run checks python3 -I -S scripts/test_runtime_attach_canary.py
ci_run checks node --test scripts/test_documentation_visual_reader.cjs scripts/test_working_tree_report_reader.cjs scripts/test_data_model_reader.cjs scripts/test_documentation_packs.cjs
ci_run checks cargo fmt --all --check
ci_run checks cargo clippy --workspace --all-targets -- -D warnings
ci_run checks qualify_csharp
ci_run checks cargo test --locked -p clew --lib 'documentation::' -- --test-threads=1
ci_run checks cargo test --locked -p clew --lib 'compiler_failure::tests::' -- --test-threads=1
ci_run retained-docs cargo test --locked -p clew --test support_collect -- --test-threads=1
ci_run retained-docs cargo test --locked -p clew --test documentation_visuals --test documentation_language --test documentation_publication_conflict --test docs_work_source_parts --test docs_work_retained_parts --test documentation_account_recovery -- --test-threads=1
ci_run retained-docs cargo test --locked -p clew --test documentation_external_calls -- --test-threads=1
ci_run retained-docs cargo test --locked -p clew --test documentation_system docsys_t17_ -- --test-threads=1
ci_run retained-docs cargo test --locked -p clew --test documentation_system reviewed_ -- --test-threads=1
ci_run kotlin cargo test --locked -p clew --lib documentation::kotlin::tests::native_kotlin_19_maven_documentation -- --exact --ignored --test-threads=1
ci_run kotlin ./gradlew --no-daemon :workers:kotlin:test --tests dev.semanticthread.worker.KotlinDocumentationFlowTest --tests dev.semanticthread.worker.SpringAnnotationFactsTest --tests dev.semanticthread.worker.SpringCompilerPluginCompatibilityTest \
  :workers:kotlin21:test --tests dev.semanticthread.worker.KotlinDocumentationFlowTest --tests dev.semanticthread.worker.SpringAnnotationFactsTest
ci_run java cargo test --locked -p clew --lib documentation::check::tests::two_java_services_resolve_and_compose_declared_http_with_branches -- --exact --ignored --test-threads=1
ci_run managed-docs cargo test --locked -p clew --test managed_cli durable_documentation_cli_recovers_and_reports_route_fragments -- --exact --test-threads=1
ci_run managed-docs cargo test --locked -p clew --test managed_cli durable_source_documentation_without_build_tools_rebinds_and_preserves_publication -- --exact --test-threads=1
ci_run managed-docs cargo test --locked -p clew --test managed_cli durable_source_documentation_java_enrichment_recovers_on_the_same_source_roots -- --exact --ignored --test-threads=1
ci_run managed-docs cargo test --locked -p clew --test managed_cli flow_dsl_static_render_uses_original_supported_snapshot -- --exact --test-threads=1
ci_run native-pages cargo test --locked -p clew --test docs_dependency_targets -- --ignored --test-threads=1
ci_run native-pages cargo test --locked -p clew --test docs_static_pages -- --ignored --test-threads=1
ci_run native-pages cargo test --locked -p clew --test docs_static_linked_processes -- --ignored --test-threads=1
ci_run checks cargo test --locked -p clew --lib 'operations::tests::' -- --test-threads=1
ci_run java cargo test --locked -p clew --lib 'maven::tests::' -- --test-threads=1
ci_run checks cargo test --locked -p clew --lib 'project_config::tests::' -- --test-threads=1
ci_run java cargo test --locked -p clew --lib 'java_project_model::tests::' -- --test-threads=1
ci_run java cargo test --locked -p clew --lib java_project_model::tests::java_fixtures_extract_project_native_models -- --exact --ignored --test-threads=1
ci_run java cargo test --locked -p clew --lib java_project_model::tests::maven_effective_profile_matches_native_compile_properties -- --exact --ignored --test-threads=1
ci_run java cargo test --locked -p clew --lib java_project_model::tests::maven_large_logs_and_native_compile_failure_are_distinguished -- --exact --ignored --test-threads=1
ci_run checks cargo test --locked -p clew --lib 'worker_diagnostics::tests::' -- --test-threads=1
ci_run checks cargo test --locked -p clew --lib worker::tests::worker_frame_eof_keeps_exit_status_without_inline_stderr -- --exact --test-threads=1
ci_run java cargo test --locked -p clew --lib 'java_adapter_v2::tests::' -- --test-threads=1
ci_run java cargo test --locked -p clew --test java_dependency_targets -- --test-threads=1
ci_run java cargo test --locked -p clew --lib 'java_analysis_' -- --test-threads=1
ci_run java cargo test --locked -p clew --lib java_analysis_scratch::tests::native_java_child_retains_lease_after_parent_handles_close -- --exact --ignored --test-threads=1
ci_run java cargo test --locked -p clew --lib 'java_archive_inputs::tests::' -- --test-threads=1
ci_run checks cargo test --locked -p clew --lib 'generation_service::reuse_tests::' -- --test-threads=1
ci_run java cargo test --locked -p clew --lib \
  java_adapter_v2::tests::spring_entrypoints_use_resolved_annotations_on_java_17_and_21 \
  -- --exact --ignored --test-threads=1
ci_run checks cargo test --locked -p clew-framework-aspnetcore
ci_run checks cargo test --locked -p clew --lib 'csharp' -- --test-threads=1
ci_run checks cargo test --locked -p clew --lib 'context_v2::tests::' -- --test-threads=1
ci_run checks cargo test --locked -p clew --lib 'task_run_v2::tests::' -- --test-threads=1
ci_run checks cargo test --locked -p clew --lib 'session::tests::' -- --test-threads=1
ci_run checks cargo test --locked -p clew --lib 'cas::' -- --test-threads=1
ci_run kotlin cargo test --locked -p clew --lib 'kotlin_engine::tests::' -- --test-threads=1
ci_run kotlin cargo test --locked -p clew --lib worker::tests::kotlin_23_project_uses_core_once_and_preserves_original_compiler_identity -- --exact --test-threads=1
ci_run kotlin cargo test --locked -p clew --lib worker::tests::retired_kotlin23_engine_cannot_prepare_or_execute_a_worker -- --exact --test-threads=1
ci_run kotlin cargo test --locked -p clew --lib worker::tests::kotlin23_baseline_uses_core_and_preserves_option_boundaries -- --exact --test-threads=1
ci_run kotlin ./gradlew --no-daemon :workers:kotlin:test --tests dev.semanticthread.worker.KotlinEngineCompatibilityTest --tests dev.semanticthread.worker.Kotlin19OptionQualificationTest --tests dev.semanticthread.worker.KotlinJvm25QualificationTest
ci_run kotlin cargo test --locked -p clew --lib kotlin_adapter_v2::tests::cross_engine_normalization_preserves_nonempty_spring_evidence -- --exact --test-threads=1
ci_run kotlin cargo test --locked -p clew --lib kotlin_adapter_v2::tests::compiler_analysis_refusal_preserves_diagnostics_without_emitting_facts -- --exact --test-threads=1
ci_run kotlin cargo test --locked -p clew --lib kotlin_adapter_v2::tests::real_k2_failure_preserves_the_original_stage -- --exact --ignored --test-threads=1
ci_run checks cargo test --locked -p clew --lib 'working_tree' -- --test-threads=1
ci_run checks cargo test --locked -p clew --test managed_cli 'working_tree_' -- --test-threads=1
ci_run java cargo test --locked -p clew --test managed_cli \
  managed_java17_maven_local_config_returns_indexed_source_without_commits \
  -- --exact --test-threads=1
ci_run checks cargo test --locked -p clew --bin clew 'tests::' -- --test-threads=1
ci_run checks cargo test --locked -p clew --test managed_cli \
  managed_operational_commands_are_path_free_and_support_recovery -- --test-threads=1
ci_run checks cargo test --locked -p clew --test managed_cli \
  managed_python_context_rejects_missing_plan_without_project_processes -- --exact --test-threads=1
ci_run checks cargo test --locked -p clew --test managed_cli \
  managed_support_summary_requires_private_input_and_drops_private_material -- --test-threads=1
ci_run checks python3 -I -S scripts/check_repository_privacy.py --worktree
ci_run kotlin python3 -I -S scripts/usability-smoke.py

if [ "$CLEW_CI_SUITE" = all ]; then
  printf '%s\n' '{"schema":"codeclew-verification/1.0","status":"PASSED"}'
else
  printf '{"schema":"codeclew-verification/1.0","status":"PASSED","scope":"%s"}\n' "$CLEW_CI_SUITE"
fi
